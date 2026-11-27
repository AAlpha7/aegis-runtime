#!/usr/bin/env python3
"""Installer behavior with real files/processes and a simulated service manager.

Does not claim real systemd, provider, WSL lifetime, or reboot acceptance.
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import http.server
import threading
import urllib.request
import socket
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("managed-runtime.py").resolve()
spec = importlib.util.spec_from_file_location("managed_runtime", SCRIPT)
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class InstallerTests(unittest.TestCase):
    def test_fresh_preflight_rejects_occupied_port_without_touching_listener(self):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            port = listener.getsockname()[1]
            (self.root / '.env').write_text(f'AXUM_PORT={port}\n')
            with self.assertRaisesRegex(RuntimeError, 'AXUM_PORT port .* is unavailable'):
                self.app.check_ports()
            self.assertEqual(listener.getsockname()[1], port)
            self.assertFalse(self.app.manifest.exists())

    def test_preflight_accepts_free_configured_ports_and_rejects_invalid_port(self):
        with socket.socket() as probe, socket.socket() as second:
            probe.bind(('127.0.0.1', 0))
            second.bind(('127.0.0.1', 0))
            port = probe.getsockname()[1]
            other = second.getsockname()[1]
        (self.root / '.env').write_text(f'AXUM_PORT={port}\nOAUTH_CALLBACK_PORT={other}\n')
        self.app.check_ports()
        (self.root / '.env').write_text(f'AXUM_PORT={port}\nOAUTH_CALLBACK_PORT={port}\n')
        with self.assertRaisesRegex(RuntimeError, 'different ports'):
            self.app.check_ports()
        (self.root / '.env').write_text('AXUM_PORT=70000\n')
        with self.assertRaisesRegex(RuntimeError, 'between 1 and 65535'):
            self.app.check_ports()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="aegis-install-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "checkout with spaces"
        self.root.mkdir()
        self.config = self.base / "config"
        self.state = self.base / "state"
        env = {k: v for k, v in os.environ.items() if not k.startswith(("AEGIS_", "XDG_", "CARGO_TARGET_DIR"))}
        env.update(AEGIS_RUNTIME_ROOT=str(self.root), XDG_CONFIG_HOME=str(self.config),
                   AEGIS_RUNTIME_STATE_DIR=str(self.state))
        self.patch = patch.dict(os.environ, env, clear=True)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.app = runtime.Installation()

    def key(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            path, value = self.app.key(create=True)
        self.assertNotIn(value, output.getvalue())
        return path, value

    def fake_run(self, args, check=True):
        args = list(map(str, args))
        self.commands.append(args)
        stdout = "yes\n" if args[0] == "loginctl" else "tester\n" if args[0] == "id" else ""
        return subprocess.CompletedProcess(args, 0, stdout, "")

    def install(self):
        self.key()
        binary = self.root / "target/release/aegis_runtime_core"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_text("#!/bin/sh\nexit 0\n")
        binary.chmod(0o700)
        bundle = self.root / "webapp/dist/index.html"
        bundle.parent.mkdir(parents=True, exist_ok=True)
        bundle.write_text("console")
        self.commands = []
        with patch.object(runtime, "run", self.fake_run), patch.object(runtime.shutil, "which", return_value="/usr/bin/tool"), patch.object(self.app, "check_ports"):
            self.app.install()
        return binary

    def test_key_created_private_reused_and_never_printed(self):
        path, value = self.key()
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(path.parent.stat().st_mode & 0o777, 0o700)
        self.assertEqual(self.key(), (path, value))
        self.assertFalse(list(self.root.rglob("*.hex")))

    def test_remote_launcher_uses_https_but_readiness_stays_loopback(self):
        self.install()
        self.assertEqual(self.app.browser_url(), "http://127.0.0.1:8765/ui/")
        (self.root / ".env").write_text("AEGIS_PUBLIC_URL=https://example.test/\nAEGIS_ACCESS_MODE=private\n")
        self.assertEqual(self.app.browser_url(), "https://example.test/access")
        self.assertEqual(self.app.login(), "https://example.test/access")
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.app.control("windows-info")
        info = json.loads(output.getvalue())
        self.assertTrue(info["remote"])
        self.assertEqual(info["browser_url"], "https://example.test/access")
        self.assertEqual(info["console"], "http://127.0.0.1:8765/ui/")

    def test_missing_key_with_existing_data_refuses_to_reset(self):
        self.state.mkdir()
        data = self.state / "aegis_runtime_memory.db"
        data.write_bytes(b"existing encrypted data")
        with self.assertRaisesRegex(RuntimeError, "Restore the original key"):
            self.key()
        self.assertEqual(data.read_bytes(), b"existing encrypted data")
        self.assertFalse(self.config.exists())

    def test_missing_key_with_custom_db_refuses_to_reset(self):
        data = self.base / "separate.db"
        data.write_bytes(b"existing")
        (self.root / ".env").write_text(f"MEMORY_DB_PATH={data}\n")
        with self.assertRaises(RuntimeError):
            self.key()

    def test_key_rejects_unsafe_mode_symlink_and_invalid_value(self):
        path, original = self.key()
        path.chmod(0o644)
        with self.assertRaisesRegex(RuntimeError, "0600"):
            self.app.key()
        path.chmod(0o600)
        path.write_text("invalid")
        with self.assertRaisesRegex(RuntimeError, "64 hex"):
            self.app.key()
        path.unlink()
        other = self.base / "other-key"
        other.write_text(original)
        other.chmod(0o600)
        path.symlink_to(other)
        with self.assertRaises(OSError):
            self.app.key()

    def test_key_cannot_be_inside_checkout_or_state(self):
        for parent in (self.root, self.state):
            with patch.dict(os.environ, AEGIS_RUNTIME_DB_KEY_FILE=str(parent / "key.hex")):
                with self.assertRaisesRegex(RuntimeError, "outside"):
                    self.key()

    def test_injected_key_not_silently_persisted_or_replaced(self):
        with patch.dict(os.environ, AEGIS_RUNTIME_DB_KEY_HEX="1" * 64):
            with self.assertRaisesRegex(RuntimeError, "not be copied"):
                self.key()
        path, value = self.key()
        with patch.dict(os.environ, AEGIS_RUNTIME_DB_KEY_HEX="2" * 64):
            with self.assertRaisesRegex(RuntimeError, "differ"):
                self.app.key()
        self.assertEqual(path.read_text().strip(), value)

    def test_install_and_rerun_never_embed_secret_in_service(self):
        self.install()
        path, value = self.key()
        unit = self.app.unit_dir / self.app.unit
        initial = unit.read_text()
        self.assertNotIn(value, initial)
        self.assertNotIn(value, self.app.manifest.read_text())
        self.assertIn('Restart=on-failure', initial)
        self.assertIn('Environment="XDG_CONFIG_HOME=', initial)
        self.install()
        self.assertEqual(unit.read_text(), initial)
        self.assertEqual(path.read_text().strip(), value)

    def test_existing_unowned_unit_is_preserved(self):
        self.install()
        unit = self.app.unit_dir / self.app.unit
        unit.write_text("unrelated service\n")
        with self.assertRaisesRegex(RuntimeError, "not owned"):
            self.install()
        self.assertEqual(unit.read_text(), "unrelated service\n")

    def test_another_checkout_cannot_adopt_same_state(self):
        self.install()
        other = self.app.directory.parent / "other" / "install.json"
        other.parent.mkdir()
        other.write_text(json.dumps({"state": str(self.state), "root": "/unrelated"}))
        with self.assertRaisesRegex(RuntimeError, "already owns"):
            self.install()

    def test_environment_only_credential_is_not_silently_lost_or_copied(self):
        self.install()
        before = self.app.manifest.read_text()
        with patch.dict(os.environ, EMBEDDING_API_KEY="synthetic-env-only-secret"):
            with self.assertRaisesRegex(RuntimeError, "persistent secret custody"):
                self.install()
        self.assertEqual(self.app.manifest.read_text(), before)

    def test_missing_key_on_reinstall_is_not_regenerated(self):
        self.install()
        path, _ = self.key()
        path.unlink()
        with self.assertRaisesRegex(RuntimeError, "Restore the original key"):
            self.key()
        self.assertFalse(path.exists())

    def test_remove_retains_data_key_and_unrelated_units(self):
        self.install()
        key, value = self.key()
        data = self.state / "aegis_runtime_memory.db"
        data.write_bytes(b"retain")
        other = self.app.unit_dir / "unrelated.service"
        other.write_text("do not touch")
        with patch.object(runtime, "run", self.fake_run):
            self.app.control("remove")
        self.assertEqual(key.read_text().strip(), value)
        self.assertEqual(data.read_bytes(), b"retain")
        self.assertEqual(other.read_text(), "do not touch")
        self.assertFalse(self.app.manifest.exists())

    def test_service_loads_same_key_from_file_after_installer_exits(self):
        binary = self.install()
        path, value = self.key()
        # The child writes a digest, never the key, to its private output.
        binary.write_text("#!/usr/bin/env python3\nimport os, hashlib\nprint(hashlib.sha256(os.environ['AEGIS_RUNTIME_DB_KEY_HEX'].encode()).hexdigest())\n")
        import hashlib
        expected = hashlib.sha256(value.encode()).hexdigest()
        for _ in range(2):
            result = subprocess.run(["python3", str(SCRIPT), "serve"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((self.state / "aegis-runtime.log").read_text().strip(), expected)
            self.assertNotIn(value, result.stdout + result.stderr)
        self.assertEqual(path.read_text().strip(), value)

    def test_bad_key_prevents_service_execution_without_resetting_data(self):
        binary = self.install()
        marker = self.base / "ran"
        binary.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
        path, _ = self.key()
        path.write_text("invalid")
        result = subprocess.run(["python3", str(SCRIPT), "serve"], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(marker.exists())

    def test_unreachable_manager_is_reported_without_portable_fallback(self):
        def denied(args, check=True):
            return subprocess.CompletedProcess(args, 1, "", "Permission denied")
        with patch.object(runtime, "run", denied), patch.object(runtime.shutil, "which", return_value="/usr/bin/tool"):
            with self.assertRaisesRegex(RuntimeError, "sandbox denial"):
                self.app.preflight(True)
        self.assertFalse(self.app.manifest.exists())

    def test_linger_is_not_automatically_granted(self):
        def no_linger(args, check=True):
            return subprocess.CompletedProcess(args, 0, "tester\n" if args[0] == "id" else "no\n", "")
        with patch.object(runtime, "run", no_linger), patch.object(runtime.shutil, "which", return_value="/usr/bin/tool"):
            with self.assertRaisesRegex(RuntimeError, "sudo loginctl enable-linger tester"):
                self.app.preflight(True)

    def test_unit_quoting_rejects_line_injection(self):
        self.assertEqual(runtime.unit_quote('/tmp/space %h/"file'), '"/tmp/space %%h/\\"file"')
        with self.assertRaises(RuntimeError):
            runtime.unit_quote("/tmp/path\nExecStart=evil")


class LocalSessionTests(unittest.TestCase):
    def test_secure_cookie_works_on_local_origin_but_not_another_port(self):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                if self.path == '/login':
                    self.send_header('Set-Cookie', '__Host-aegis_session=synthetic-owner; Secure; HttpOnly; Path=/')
                self.end_headers()
                self.wfile.write(json.dumps({'authenticated': self.headers.get('Cookie') == '__Host-aegis_session=synthetic-owner'}).encode())

            def log_message(self, *args):
                pass

        servers = [http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler) for _ in range(2)]
        for server in servers:
            threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            origins = [f'http://127.0.0.1:{s.server_port}' for s in servers]
            jar = runtime.http.cookiejar.CookieJar(policy=runtime.LocalSessionCookiePolicy(origins[0]))
            client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
            client.open(origins[0] + '/login', timeout=5).close()
            with client.open(origins[0] + '/api/session', timeout=5) as response:
                self.assertTrue(json.load(response)['authenticated'])
            with client.open(origins[1] + '/api/session', timeout=5) as response:
                self.assertFalse(json.load(response)['authenticated'])
            with self.assertRaises(RuntimeError):
                runtime.LocalSessionCookiePolicy('http://remote.invalid')
        finally:
            for server in servers:
                server.shutdown()
                server.server_close()


if __name__ == "__main__":
    unittest.main()
