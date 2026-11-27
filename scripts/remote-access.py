#!/usr/bin/env python3
"""Manage local / Tailscale Serve / Funnel access for the portable installation.

Run in the same Linux/WSL environment as Aegis and tailscaled. Does not install
Tailscale, log into accounts, modify tailnet policy, or manage unrelated services.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

RISK_VERSION = "2026-09-27"
WARNING = """公网访问的风险高于私网 / Public access has greater exposure than private access.
Anyone on the internet can send requests to your Aegis login endpoint.
HTTPS and Passkeys do not eliminate software vulnerabilities or compromised devices.
Protect your account/devices, install security updates, keep recovery credentials safe,
and take responsibility for day-to-day security of your public deployment.
If unsure, choose private mode. This acknowledgment never bypasses authentication.
请保护账号和设备、及时更新、保管恢复凭据，并负责公网部署的日常安全管理。
"""


def command(args, *, capture=True, extra_env=None):
    return subprocess.run(args, check=True, text=True, capture_output=capture, timeout=120,
                          env=dict(os.environ, **(extra_env or {}))).stdout


def atomic_write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".aegis-access-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def read_env(path):
    result = {}
    for line in path.read_text().splitlines():
        if not re.match(r"^[A-Z][A-Z0-9_]*=", line):
            continue
        key, value = line.split("=", 1)
        parts = shlex.split(value, comments=True)
        result[key] = " ".join(parts)
    return result


def update_env(path, values):
    lines = [line for line in path.read_text().splitlines()
             if line.split("=", 1)[0] not in values]
    lines.extend(f"{key}={shlex.quote(value)}" for key, value in values.items())
    atomic_write(path, "\n".join(lines) + "\n")


def nonempty(config):
    return any(bool(value) for value in config.values())


def read_config(tailscale):
    config = json.loads(command([tailscale, "serve", "status", "--json"]))
    if config is None:
        return {}
    if not isinstance(config, dict):
        raise RuntimeError("Unexpected Tailscale status format")
    return config


def verify_route(config, hostname, target, public):
    if any(value for key, value in config.items() if key not in {"TCP", "Web", "AllowFunnel"}):
        raise RuntimeError("Unexpected unmanaged Tailscale configuration")
    expected_host = hostname + ":443"
    web = config.get("Web", {})
    tcp = config.get("TCP", {})
    funnel = config.get("AllowFunnel", {})
    if set(web) != {expected_host} or set(tcp) != {"443"}:
        raise RuntimeError("Unexpected Tailscale listeners; refusing to report success")
    if tcp["443"] != {"HTTPS": True} or web[expected_host].get("Handlers") != {"/": {"Proxy": target}}:
        raise RuntimeError("Tailscale is not forwarding only to the Aegis loopback listener")
    enabled = {host for host, value in funnel.items() if value}
    if enabled != ({expected_host} if public else set()):
        raise RuntimeError("Funnel exposure does not match requested mode")


def choose_mode():
    print("访问方式 / Access mode:\n  1. 仅本机 / Local only (default)\n  2. 私网 / Private, recommended for remote access; phone needs Tailscale VPN\n  3. 公网 / Public; browser only, greater exposure")
    choice = input("Choose [1/2/3, default 1]: ").strip()
    if choice not in ("", "1", "2", "3"):
        raise RuntimeError("Invalid access mode")
    return {"": "local", "1": "local", "2": "private", "3": "public"}[choice]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["local", "private", "public"])
    parser.add_argument("--accept-public-risk", action="store_true")
    parser.add_argument("--check-choice", action="store_true", help="validate mode/consent only; do not change configuration")
    parser.add_argument("--root", type=Path, default=Path(os.environ.get("AEGIS_RUNTIME_ROOT", Path(__file__).resolve().parent.parent)))
    args = parser.parse_args(argv)
    mode = args.mode or (choose_mode() if sys.stdin.isatty() else "local")
    if mode == "public":
        print(WARNING)
        accepted = args.accept_public_risk
        if not accepted and sys.stdin.isatty():
            accepted = input("我已了解风险并选择公网 / Type PUBLIC to confirm: ").strip() == "PUBLIC"
        if not accepted:
            raise RuntimeError("Public access not enabled: explicit acknowledgment required (--accept-public-risk)")
    if args.check_choice:
        return
    root = args.root.resolve()
    if not (root / ".env").is_file():
        raise RuntimeError("Complete local setup first: scripts/first-run.sh")
    with (root / ".aegis-access.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("Another access-mode change is in progress") from error
        configure(root, mode)


def configure(root, mode):
    env_path = root / ".env"
    state_path = root / ".aegis-access.json"
    if not env_path.is_file():
        raise RuntimeError("Complete local setup first: scripts/first-run.sh")
    env = read_env(env_path)
    for key in ("AEGIS_ACCESS_MODE", "AEGIS_PUBLIC_URL", "OAUTH_CALLBACK_BIND", "AEGIS_PUBLIC_RISK_ACCEPTED"):
        if key in os.environ:
            raise RuntimeError(f"Unset {key} in the shell before changing modes; .env must be authoritative")
    port = int(os.environ.get("OAUTH_CALLBACK_PORT", env.get("OAUTH_CALLBACK_PORT", "8765")))
    if not 1 <= port <= 65535:
        raise RuntimeError("Invalid console port")
    supervisor = Path(__file__).resolve().with_name("aegis-runtime-local.sh")
    previous = json.loads(state_path.read_text()) if state_path.exists() else None
    if not previous and env.get("AEGIS_PUBLIC_URL"):
        raise RuntimeError("Existing remote configuration is not managed here. Disable/review its old forwarding route before adopting managed access")
    tailscale = shutil.which("tailscale")
    config = {}
    hostname = None
    if mode != "local" or previous or tailscale:
        if not tailscale:
            raise RuntimeError("Install and log into Tailscale in the same Linux/WSL environment first: https://tailscale.com/download")
        config = read_config(tailscale)
        if nonempty(config) and (not previous or previous.get("config") != config):
            raise RuntimeError("Tailscale Serve/Funnel has unmanaged changes. Review them first; no configuration was overwritten")
        if mode != "local":
            status = json.loads(command([tailscale, "status", "--json"]))
            if status.get("BackendState") != "Running":
                raise RuntimeError("Log into Tailscale first (tailscale up), then rerun")
            hostname = status.get("Self", {}).get("DNSName", "").rstrip(".")
            if not re.fullmatch(r"[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+\.ts\.net", hostname):
                raise RuntimeError("A valid Tailscale DNS name is required")
    origin = f"https://{hostname}" if hostname else ""
    helper = Path(os.environ.get("AEGIS_RUNTIME_ACCESS_BIN", str(Path(os.environ.get("AEGIS_RUNTIME_BIN", root / "target/release/aegis_runtime_core")).with_name("aegis_runtime_access"))))
    memory = Path(os.environ.get("MEMORY_DB_PATH", env.get("MEMORY_DB_PATH", "aegis_runtime_memory.db")))
    if not memory.is_absolute():
        memory = root / memory
    if mode != "local":
        if not helper.is_file():
            raise RuntimeError("Build aegis_runtime_access before enabling remote access")
        # Validate database/key before touching a running deployment. Never print keys.
        command([str(helper), "status", str(memory), origin])
    # reset affects the whole Serve config, hence the strict ownership check above.
    if nonempty(config):
        if read_config(tailscale) != config:
            raise RuntimeError("Tailscale configuration changed during setup; no reset performed")
        command([tailscale, "serve", "reset"])
        if nonempty(read_config(tailscale)):
            raise RuntimeError("Remote listener removal failed; public access may still be active")
    atomic_write(state_path, json.dumps({"mode": "transition", "config": {}}) + "\n")
    update_env(env_path, {"AEGIS_ACCESS_MODE": mode, "AEGIS_PUBLIC_URL": origin,
                         "OAUTH_CALLBACK_BIND": "127.0.0.1",
                         "AEGIS_PUBLIC_RISK_ACCEPTED": RISK_VERSION if mode == "public" else ""})
    try:
        command(["bash", str(supervisor), "restart"], extra_env={"AEGIS_RUNTIME_ROOT": str(root)})
        if mode != "local":
            target = f"http://127.0.0.1:{port}"
            command([tailscale, "funnel" if mode == "public" else "serve", "--bg", "--https=443", target], capture=False)
            config = read_config(tailscale)
            verify_route(config, hostname, target, mode == "public")
        else:
            config = {}
        atomic_write(state_path, json.dumps({"mode": mode, "config": config,
                                           "risk_notice_version": RISK_VERSION if mode == "public" else None}) + "\n")
    except BaseException:
        if mode != "local":
            try:
                current = read_config(tailscale)
                if nonempty(current):
                    # An external actor may have changed the daemon meanwhile.
                    # Never erase another service during failure cleanup.
                    verify_route(current, hostname, f"http://127.0.0.1:{port}",
                                 any(current.get("AllowFunnel", {}).values()))
                    command([tailscale, "serve", "reset"])
                if nonempty(read_config(tailscale)):
                    raise RuntimeError("listener still present")
            except Exception:
                print("WARNING: could not verify shutdown; remote access MAY STILL BE ACTIVE. Inspect tailscale serve status.", file=sys.stderr)
        raise
    print(f"Access mode configured: {mode}. " + (f"Open {origin}/access" if origin else f"Open http://127.0.0.1:{port}/ui/"))
    if mode != "local":
        print("Host must remain awake and online. Tailscale account/device authorization can expire.")
        if mode == "private":
            print("Connect the phone's Tailscale VPN. Review tailnet access rules; default rules may allow all your tailnet devices.")
        command([str(helper), "pair", str(memory), origin], capture=False)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as error:
        # A subprocess can carry secrets in stdout, so never print its full repr/output.
        print(f"Remote access setup failed: {error if not isinstance(error, subprocess.SubprocessError) else 'command failed; inspect the service locally'}", file=sys.stderr)
        sys.exit(1)
