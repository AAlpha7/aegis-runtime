#!/usr/bin/env python3
"""Durable single-user installation. No shell evaluation of config or secrets."""
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.request
from urllib.parse import urlsplit

SCRIPT = Path(__file__).resolve()
OWNER = "aegis-runtime-managed-v1"


class LocalSessionCookiePolicy(http.cookiejar.DefaultCookiePolicy):
    """Match browsers' Secure-cookie exception for this exact loopback origin.

    urllib otherwise drops Aegis's Secure owner cookie on local HTTP. Never
    extend this exception to another port, remote host or redirected origin.
    """
    def __init__(self, origin):
        super().__init__()
        self.origin = urlsplit(origin)
        if self.origin.scheme != "http" or self.origin.hostname != "127.0.0.1":
            raise RuntimeError("Local chat validation requires a loopback HTTP origin")

    def return_ok_secure(self, cookie, request):
        target = urlsplit(request.full_url)
        if target.scheme == self.origin.scheme and target.netloc == self.origin.netloc:
            return True
        return super().return_ok_secure(cookie, request)


def run(args, check=True):
    return subprocess.run(list(map(str, args)), text=True, capture_output=True,
                          timeout=120, check=check)


def dotenv(root):
    result = {}
    path = root / ".env"
    if path.exists():
        for line in path.read_text().splitlines():
            if re.match(r"^[A-Z][A-Z0-9_]*=", line):
                key, value = line.split("=", 1)
                result[key] = " ".join(shlex.split(value, comments=True))
    return result


def private_dir(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise RuntimeError("Configuration directory must be owned by this user, without symlinks")
    path.chmod(0o700)


def atomic_write(path, text, mode=0o600):
    private_dir(path.parent)
    if path.is_symlink():
        raise RuntimeError("Refusing to replace a symlink")
    fd, name = tempfile.mkstemp(dir=path.parent, prefix=".aegis-")
    try:
        with os.fdopen(fd, "w") as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def unit_quote(value):
    value = str(value)
    if any(ord(char) < 32 for char in value) or "$" in value:
        raise RuntimeError("Control characters and dollar substitutions are not allowed in service paths")
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"').replace('%', '%%') + '"'


class Installation:
    def __init__(self, root=None):
        if sys.platform == "linux":
            # WSL task launches are non-login shells. A missing shell variable
            # must not hide an existing manager, including start/hold commands.
            user_run = Path(f"/run/user/{os.getuid()}")
            if user_run.is_dir() and user_run.stat().st_uid == os.getuid() and (user_run / "bus").is_socket():
                os.environ.setdefault("XDG_RUNTIME_DIR", str(user_run))
                os.environ.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path={user_run}/bus")
        self.root = Path(root or os.environ.get("AEGIS_RUNTIME_ROOT", SCRIPT.parent.parent)).resolve()
        self.ident = hashlib.sha256(str(self.root).encode()).hexdigest()[:16]
        self.config_base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
        self.directory = self.config_base / "aegis-runtime" / "installations" / self.ident
        self.manifest = self.directory / "install.json"
        self.unit = f"aegis-runtime-{self.ident}.service"
        self.unit_dir = self.config_base / "systemd/user"
        self.state = Path(os.environ.get("AEGIS_RUNTIME_STATE_DIR",
                          str(Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state")) / "aegis-runtime"))).resolve()

    def load(self):
        value = json.loads(self.manifest.read_text())
        if value.get("owner") != OWNER or value.get("root") != str(self.root):
            raise RuntimeError("Installation belongs to another checkout; nothing changed")
        return value

    def key(self, create=False):
        config = dotenv(self.root)
        config.update({name: os.environ[name] for name in ("MEMORY_DB_PATH", "HEIRLOOM_EVENT_LOG_DB") if name in os.environ})
        recorded = self.load() if self.manifest.exists() else {}
        path = Path(os.environ.get("AEGIS_RUNTIME_DB_KEY_FILE", recorded.get("key_file",
                    str(self.config_base / "aegis-runtime/keys" / (self.ident + ".hex"))))).absolute()
        state = Path(recorded.get("state", self.state)).resolve()
        resolved = path.resolve()
        protected = [self.root, state]
        for name in ("MEMORY_DB_PATH", "HEIRLOOM_EVENT_LOG_DB"):
            if config.get(name):
                protected.append((self.root / config[name]).resolve().parent)
        if any(resolved == p or p in resolved.parents for p in protected):
            raise RuntimeError("Keep the database key outside the checkout and database directories")
        if not path.exists() and create:
            if os.environ.get("AEGIS_RUNTIME_DB_KEY_HEX"):
                raise RuntimeError("An external key is injected. Supply its persistent AEGIS_RUNTIME_DB_KEY_FILE; it will not be copied automatically")
            existing = [self.root / config.get("MEMORY_DB_PATH", "aegis_runtime_memory.db"),
                        state / "aegis_runtime_memory.db", self.root / "heirloom_event_log.db",
                        state / "heirloom_event_log.db"]
            if config.get("HEIRLOOM_EVENT_LOG_DB"):
                existing.append(self.root / config["HEIRLOOM_EVENT_LOG_DB"])
            if recorded or any(p.exists() for p in existing):
                raise RuntimeError("Existing state found but key missing. Restore the original key; no new key or database was created")
            private_dir(path.parent)
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "w") as stream:
                stream.write(secrets.token_hex(32) + "\n")
                stream.flush()
                os.fsync(stream.fileno())
            print("Created a private database key outside your data. Back it up separately; losing it makes your databases unreadable.", flush=True)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd) as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
                raise RuntimeError("Key must be a regular file owned by you with mode 0600")
            value = stream.read(256).strip()
        if not re.fullmatch(r"[0-9a-fA-F]{64}", value):
            raise RuntimeError("Database key must contain exactly 64 hex characters; value withheld")
        injected = os.environ.get("AEGIS_RUNTIME_DB_KEY_HEX")
        if injected and injected.lower() != value.lower():
            raise RuntimeError("Injected key and saved key differ; existing key was preserved")
        return path, value

    def check_tools(self, skip_build=False, managed=True):
        if sys.platform != "linux":
            raise RuntimeError("This installation route supports Linux / WSL2. On Windows, run inside WSL2")
        required = ("bash", "curl", "systemctl", "loginctl") if managed else ("bash",)
        missing = [name for name in required if not shutil.which(name)]
        if not skip_build:
            missing += [name for name in ("cargo", "rustup", "cc", "pkg-config", "node", "npm") if not shutil.which(name)]
            if shutil.which("pkg-config") and run(["pkg-config", "--exists", "openssl"], False).returncode:
                missing.append("OpenSSL development headers")
            if shutil.which("node"):
                version = run(["node", "-p", "process.versions.node.split('.')[0]"], False).stdout.strip()
                if not version.isdigit() or int(version) < 22:
                    missing.append("Node.js 22+")
        if missing:
            raise RuntimeError("Missing prerequisites: " + ", ".join(missing) +
                               ". See docs/INSTALLATION.md. Ubuntu/Debian system packages: sudo apt-get install build-essential pkg-config libssl-dev curl python3 git. Rust/Node are separate; do not run the whole installer as root")

    def preflight(self, skip_build=False):
        self.check_tools(skip_build)
        result = run(["systemctl", "--user", "show-environment"], False)
        if result.returncode:
            raise RuntimeError("Cannot contact your systemd user manager. A sandbox denial does not mean systemd is absent. Run setup in a normal host terminal; see docs/INSTALLATION.md. No temporary background fallback was selected")
        user = run(["id", "-un"]).stdout.strip()
        if run(["loginctl", "show-user", user, "-p", "Linger", "--value"], False).stdout.strip() != "yes":
            raise RuntimeError(f"Persistent user services need lingering. Ask the host administrator to run: sudo loginctl enable-linger {shlex.quote(user)} ; then rerun setup. This grants no sudo access")
        self.check_ports()
        print("Preflight passed: systemd user service, external key file, local access by default. Full developer tests are not part of installation.", flush=True)

    def check_ports(self, reuse_address=False, allow_managed=True):
        # A rerun may deliberately keep this installation's live service.
        # Fresh installs must not mistake another deployment's /health for theirs.
        active_owned = False
        if self.manifest.exists():
            if not allow_managed:
                raise RuntimeError("Managed installation exists for this checkout; use its start/stop/restart commands, not a second foreground process")
            self.load()
            active = run(["systemctl", "--user", "is-active", self.unit], False)
            active_owned = active.returncode == 0 and active.stdout.strip() == "active"
        configured = dotenv(self.root)
        seen = set()
        for name, default in (("AXUM_PORT", "3000"), ("OAUTH_CALLBACK_PORT", "8765")):
            raw = os.environ.get(name, configured.get(name, default))
            if not str(raw).isdigit() or not 1 <= int(raw) <= 65535:
                raise RuntimeError(f"{name} must be a port between 1 and 65535")
            if int(raw) in seen:
                raise RuntimeError("AXUM_PORT and OAUTH_CALLBACK_PORT must use different ports")
            seen.add(int(raw))
            if active_owned:
                continue
            with socket.socket() as probe:
                if reuse_address:
                    # Match a restarting TCP server: TIME_WAIT is not a live
                    # listener. SO_REUSEADDR still rejects an active listener.
                    probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                try:
                    probe.bind(("127.0.0.1", int(raw)))
                except OSError as error:
                    raise RuntimeError(f"{name} port {raw} is unavailable. Choose a free port in .env before setup; existing listeners were not changed") from error

    def install(self):
        self.preflight(skip_build=True)
        key_path, _ = self.key()
        target = Path(os.environ.get("CARGO_TARGET_DIR", self.root / "target"))
        binary = Path(os.environ.get("AEGIS_RUNTIME_BIN", target / "release/aegis_runtime_core")).resolve()
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError("Runtime binary is missing or not executable")
        if not (self.root / "webapp/dist/index.html").is_file():
            raise RuntimeError("Web console bundle missing; build webapp before installing")
        private_dir(self.directory)
        private_dir(self.state)
        old = self.load() if self.manifest.exists() else None
        for other in self.directory.parent.glob("*/install.json"):
            if other == self.manifest:
                continue
            previous = json.loads(other.read_text())
            if Path(previous.get("state", "/")).resolve() == self.state:
                raise RuntimeError("Another managed installation already owns this state directory; choose separate state or migrate explicitly")
        configured = dotenv(self.root)
        # Environment-only credentials vanish at logout. Never silently deploy
        # a different configuration, nor copy an injected secret into metadata.
        for name in ("OPENAI_API_KEY", "OPENROUTER_API_KEY", "AEGIS_JEV_API_KEY", "EMBEDDING_API_KEY",
                     "GOOGLE_LOGIN_CLIENT_SECRET", "SESSION_COOKIE_SECRET", "TELOXIDE_TOKEN"):
            if os.environ.get(name) and os.environ[name] != configured.get(name):
                raise RuntimeError(f"{name} is only injected into this shell. Configure persistent secret custody before managed installation; no credential was copied")
        runtime_env = {name: os.environ[name] for name in (
            "AXUM_PORT", "OAUTH_CALLBACK_PORT", "OAUTH_CALLBACK_BIND", "MEMORY_DB_PATH", "CREDENTIAL_DB_PATH",
            "IDENTITY_KEY_PATH", "HEIRLOOM_EVENT_LOG_DB", "AEGIS_RUNTIME_HEIRLOOM_BRIDGE", "AEGIS_WEBAPP_DIR",
            "OPENAI_API_BASE", "LLM_MODEL", "EMBEDDING_API_BASE", "EMBEDDING_MODEL", "EMBEDDING_DIMENSIONS",
            "GOOGLE_LOGIN_ENABLED", "AEGIS_JEV_ENABLED", "ADMIN_CHAT_ID", "USER_TIMEZONE"
        ) if name in os.environ}
        config = dict(owner=OWNER, root=str(self.root), state=str(self.state), key_file=str(key_path),
                      binary=str(binary), path=os.environ["PATH"], unit=self.unit,
                      runtime_env=runtime_env,
                      wsl="microsoft" in Path("/proc/sys/kernel/osrelease").read_text().lower())
        if old and (old["key_file"] != config["key_file"] or old["state"] != config["state"]):
            raise RuntimeError("Existing installation uses different state/key paths; migrate explicitly first")
        # Never adopt a live portable process or an unrelated unit.
        if not old and (self.state / "aegis-runtime.pid").exists():
            raise RuntimeError("Portable installation found. Stop it explicitly before installing managed services")
        unit_path = self.unit_dir / self.unit
        if unit_path.exists() and (not old or OWNER not in unit_path.read_text()):
            raise RuntimeError("Existing service is not owned by this installer; nothing overwritten")
        command = " ".join(unit_quote(v) for v in (sys.executable, SCRIPT, "--root", self.root, "serve"))
        embedding_unit = self.embedding_unit()
        dependency = f"Wants={embedding_unit}\nAfter={embedding_unit}\n" if (self.unit_dir / embedding_unit).exists() else ""
        unit = (f"# {OWNER}\n[Unit]\nDescription=Aegis Runtime ({self.ident})\nAfter=network.target\n"
                f"{dependency}[Service]\nType=simple\nExecStart={command}\n"
                f"Environment={unit_quote('XDG_CONFIG_HOME=' + str(self.config_base))}\nRestart=on-failure\nRestartSec=5\n"
                "TimeoutStopSec=30\nUMask=0077\n[Install]\nWantedBy=default.target\n")
        # Manifest is retained on failure so rerun can repair our own installation.
        atomic_write(self.manifest, json.dumps(config, indent=2) + "\n")
        atomic_write(unit_path, unit)
        run(["systemctl", "--user", "daemon-reload"])
        run(["systemctl", "--user", "enable", self.unit])
        print("Installed persistent user service. Data and key are preserved on rerun/removal.")

    def embedding_unit(self):
        return f"aegis-embedding-{self.ident}.service"

    def embedding_start(self):
        # Reuse an independently installed system service only when enabled and
        # active. Never stop/adopt an unmanaged listener or another installation.
        if (run(["systemctl", "is-active", "ollama.service"], False).returncode == 0
                and run(["systemctl", "is-enabled", "ollama.service"], False).returncode == 0):
            print("Using existing enabled Ollama system service (not owned or removed by Aegis).")
            return
        binary = shutil.which("ollama")
        if not binary:
            raise RuntimeError("Install Ollama first; no embedding service was created")
        unit = self.embedding_unit()
        path = self.unit_dir / unit
        if path.exists() and (path.is_symlink() or f"{OWNER}:{self.root}" not in path.read_text()):
            raise RuntimeError("Embedding service has another owner; nothing changed")
        if not path.exists():
            with socket.socket() as probe:
                if probe.connect_ex(("127.0.0.1", 11434)) == 0:
                    raise RuntimeError("Ollama port is already used without a recognized persistent service. Manage that installation explicitly; no temporary listener was adopted")
        content = (f"# {OWNER}:{self.root}\n[Unit]\nDescription=Aegis local embedding\n"
                   f"[Service]\nExecStart={unit_quote(binary)} serve\nEnvironment=OLLAMA_HOST=127.0.0.1:11434\n"
                   "Restart=on-failure\nRestartSec=5\nUMask=0077\n[Install]\nWantedBy=default.target\n")
        atomic_write(path, content)
        run(["systemctl", "--user", "daemon-reload"])
        run(["systemctl", "--user", "enable", "--now", unit])
        print("Local embedding uses a persistent user service on loopback.")

    def serve(self):
        config = self.load()
        _, key = self.key()
        env = dict(os.environ, **config.get("runtime_env", {}))
        env.update(PATH=config["path"], AEGIS_RUNTIME_DB_KEY_HEX=key,
                   AEGIS_RUNTIME_STATE_DIR=config["state"])
        state = Path(config["state"])
        private_dir(state)
        # Truncate per boot so login never returns a token from a previous process.
        fd = os.open(state / "aegis-runtime.log", os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
        os.fchmod(fd, 0o600)
        os.dup2(fd, 1)
        os.dup2(fd, 2)
        os.close(fd)
        os.chdir(self.root)
        os.execve(config["binary"], [config["binary"]], env)

    def urls(self):
        config = dotenv(self.root)
        if self.manifest.exists():
            config.update(self.load().get("runtime_env", {}))
        return (f"http://127.0.0.1:{config.get('AXUM_PORT', '3000')}/health",
                f"http://127.0.0.1:{config.get('OAUTH_CALLBACK_PORT', '8765')}")

    def wait_ready(self):
        timeout = int(os.environ.get("AEGIS_RUNTIME_START_TIMEOUT", "90"))
        deadline = time.monotonic() + timeout
        urls = self.urls()
        while time.monotonic() < deadline:
            try:
                if run(["systemctl", "--user", "is-active", self.unit], False).returncode == 0:
                    for url in (urls[0], urls[1] + "/ui/"):
                        with urllib.request.urlopen(url, timeout=3) as response:
                            if response.status != 200:
                                raise RuntimeError("HTTP not ready")
                    print("Service and console ready. Chat validation is a separate check.")
                    return
            except (OSError, RuntimeError):
                pass
            time.sleep(1)
        raise RuntimeError("Service/console not ready. Run the status command; inspect your private service log. No success claimed")

    def login(self):
        config = dotenv(self.root)
        if config.get("AEGIS_PUBLIC_URL"):
            return config["AEGIS_PUBLIC_URL"].rstrip("/") + "/access"
        text = (Path(self.load()["state"]) / "aegis-runtime.log").read_text(errors="replace")
        links = re.findall(r"http://127\.0\.0\.1:[0-9]+/auth/login/local\?token=[A-Za-z0-9_-]+", text)
        if not links:
            raise RuntimeError("No local login link: use configured authentication, or restart to create a fresh one")
        return links[-1]

    def browser_url(self):
        origin = dotenv(self.root).get("AEGIS_PUBLIC_URL", "").rstrip("/")
        return origin + "/access" if origin else self.urls()[1] + "/ui/"

    def smoke(self):
        self.wait_ready()
        if dotenv(self.root).get("AEGIS_PUBLIC_URL"):
            raise RuntimeError("Remote mode requires authenticated browser chat validation; not marked passed by a local probe")
        base = self.urls()[1]
        jar = http.cookiejar.CookieJar(policy=LocalSessionCookiePolicy(base))
        client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
        link = self.login()
        if urlsplit(link).netloc != urlsplit(base).netloc:
            raise RuntimeError("Sign-in link does not match this installation's local origin")
        client.open(link, timeout=10).close()
        session = json.load(client.open(base + "/api/session", timeout=10))
        csrf = session.get("csrf_token")
        if not csrf:
            raise RuntimeError("Owner login failed; restart for a fresh link, then retry")
        payload = dict(schema_version=1, input="Reply with exactly: aegis-chat-ok", mode="text",
                       intent_kind="chat", client_context="home")
        req = urllib.request.Request(base + "/webapp/vnext/command", json.dumps(payload).encode(),
                                     {"Content-Type": "application/json", "x-aegis-csrf": csrf})
        with client.open(req, timeout=120) as response:
            answer = json.load(response).get("assistant_response", {}).get("visible_answer", "")
        if "aegis-chat-ok" not in answer:
            raise RuntimeError("Chat returned no expected answer; provider not accepted")
        print("PASS: real authenticated owner chat returned the expected marker (uses your selected provider quota).")

    def control(self, action):
        config = self.load()
        if action == "status":
            active = run(["systemctl", "--user", "is-active", self.unit], False)
            print(f"Service: {active.stdout.strip() or 'unavailable'}. Console: {self.browser_url()}")
            print("WSL requires the Windows session task; sleep, logout and shutdown interrupt access." if config["wsl"] else
                  "User service enabled with lingering; computer must remain awake and online.")
            if active.returncode:
                raise RuntimeError("Service is not active; use start or inspect the private log")
            self.wait_ready()
        elif action in ("start", "restart", "stop"):
            run(["systemctl", "--user", action, self.unit])
            if action != "stop":
                self.wait_ready()
        elif action == "remove":
            path = self.unit_dir / self.unit
            if path.is_symlink() or (path.exists() and OWNER not in path.read_text()):
                raise RuntimeError("Service ownership changed; refusing removal")
            embedding = self.unit_dir / self.embedding_unit()
            if embedding.exists() and (embedding.is_symlink() or f"{OWNER}:{self.root}" not in embedding.read_text()):
                raise RuntimeError("Embedding service ownership changed; refusing removal")
            run(["systemctl", "--user", "disable", "--now", self.unit])
            path.unlink(missing_ok=True)
            if embedding.exists():
                run(["systemctl", "--user", "disable", "--now", self.embedding_unit()])
                embedding.unlink()
            run(["systemctl", "--user", "daemon-reload"])
            self.manifest.unlink()
            print("Removed managed service. Databases, key and config retained. Remove the Windows task separately if installed. Lingering was not changed.")
        elif action == "paths":
            print(f"root={self.root}\nbin={config['binary']}\nlog_file={config['state']}/aegis-runtime.log\nkey_file={config['key_file']}")
            print(f"health_url={self.urls()[0]}\nconsole_url={self.urls()[1]}/ui/")
        elif action == "login":
            print(self.login())
            print("Use your Passkey; private access requires your Tailscale connection." if dotenv(self.root).get("AEGIS_PUBLIC_URL") else
                  "One-time link; if already used, restart to obtain a fresh link.", file=sys.stderr)
        elif action == "hold":
            run(["systemctl", "--user", "start", self.unit])
            os.execv("/usr/bin/sleep", ["sleep", "infinity"])
        elif action == "windows-info":
            print(json.dumps(dict(id=self.ident, root=str(self.root), unit=self.unit,
                                  health=self.urls()[0], console=self.urls()[1] + "/ui/",
                                  browser_url=self.browser_url(),
                                  remote=bool(dotenv(self.root).get("AEGIS_PUBLIC_URL")))))


def main(args=None):
    args = list(sys.argv[1:] if args is None else args)
    root = None
    if args[:1] == ["--root"]:
        root = args[1]
        args = args[2:]
    app = Installation(root)
    action = args.pop(0) if args else "status"
    if action == "is-installed":
        if not app.manifest.exists():
            return 3
        app.load()
    elif action == "setup":
        app.preflight("--skip-build" in args)
        path, key = app.key(create=True)
        env = dict(os.environ, AEGIS_RUNTIME_MANAGED_SETUP="1", AEGIS_RUNTIME_DB_KEY_FILE=str(path),
                   AEGIS_RUNTIME_DB_KEY_HEX=key, AEGIS_RUNTIME_ROOT=str(app.root))
        os.execve("/bin/bash", ["bash", str(SCRIPT.with_name("first-run.sh")), *args], env)
    elif action == "preflight":
        app.preflight("--skip-build" in args)
    elif action == "install":
        app.install()
    elif action == "embedding-start":
        app.embedding_start()
    elif action == "serve":
        app.serve()
    elif action == "smoke":
        app.smoke()
    elif action == "with-key":
        _, key = app.key()
        if not args:
            raise RuntimeError("with-key requires a command")
        os.execvpe(args[0], args, dict(os.environ, AEGIS_RUNTIME_DB_KEY_HEX=key))
    elif action in ("start", "stop", "restart", "status", "remove", "paths", "login", "hold", "windows-info"):
        if action == "remove" and not app.manifest.exists():
            print("No managed installation at this checkout; nothing removed.")
        else:
            app.control(action)
    else:
        raise RuntimeError("Commands: preflight, setup, status, start, stop, restart, login, smoke, paths, remove, with-key")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as error:
        # HTTP/subprocess error strings can include login URLs or credential-bearing output.
        message = str(error) if isinstance(error, RuntimeError) else "Operation failed; check permissions, service status and private logs locally (sensitive details withheld)"
        print("Aegis: " + message, file=sys.stderr)
        sys.exit(1)
