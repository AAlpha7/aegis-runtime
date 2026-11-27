#!/usr/bin/env python3
"""Exercise the real runtime's Heirloom opt-in with disposable encrypted state.

No accounts or external model calls: provider URLs point at loopback port 9.
This checks startup and storage isolation, not a real provider chat round-trip.
"""
import hashlib
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
BINARY = TARGET / "debug/aegis_runtime_core"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def snapshot(directory):
    return {p.name: hashlib.sha256(p.read_bytes()).digest()
            for p in directory.iterdir() if p.is_file()}


def main():
    assert BINARY.is_file(), "Build aegis_runtime_core before running this test"
    with tempfile.TemporaryDirectory(prefix="aegis-heirloom-mode-") as directory:
        root = Path(directory)
        heirloom_dir = root / "experimental"
        heirloom = heirloom_dir / "heirloom.db"
        env = {
            "PATH": "/usr/bin:/bin", "HOME": str(root), "USER_TIMEZONE": "UTC",
            "MEMORY_DB_PATH": str(root / "memory.db"),
            "IDENTITY_KEY_PATH": str(root / "identity.key"),
            "CREDENTIAL_DB_PATH": str(root / "credentials.db"),
            "HEIRLOOM_EVENT_LOG_DB": str(heirloom),
            "AEGIS_RUNTIME_DB_KEY_HEX": secrets.token_hex(32),
            "OPENAI_API_KEY": "local-test-fixture",
            "OPENAI_API_BASE": "http://127.0.0.1:9/v1",
            "EMBEDDING_API_KEY": "local-test-fixture",
            "EMBEDDING_API_BASE": "http://127.0.0.1:9/v1",
            "ADMIN_CHAT_ID": "1", "AEGIS_LOCAL_LOGIN_TOKEN": "off",
            "AEGIS_WEBAPP_DIR": str(ROOT / "webapp/dist"),
            "RUST_LOG": "error",
            # A stale read preference alone must never enable Heirloom.
            "AEGIS_RUNTIME_PROFILE_READ_SOURCE": "heirloom_preferred",
        }

        def run(mode, *, fail_preflight=False, setup=False, path=None, key=None):
            child_env = dict(env, AXUM_PORT=str(free_port()),
                             OAUTH_CALLBACK_PORT=str(free_port()))
            if mode is not None:
                child_env["AEGIS_RUNTIME_HEIRLOOM_BRIDGE"] = mode
            if path is not None:
                child_env["HEIRLOOM_EVENT_LOG_DB"] = str(path)
            if key is not None:
                child_env["AEGIS_RUNTIME_DB_KEY_HEX"] = key
            if setup:
                child_env.pop("OPENAI_API_KEY", None)
            args = [str(BINARY)] + (["setup-auth", "status"] if setup else [])
            # Output can contain local authentication details; never print it.
            with tempfile.TemporaryFile() as output:
                process = subprocess.Popen(args, cwd=root, env=child_env,
                                           stdout=output, stderr=output)
                try:
                    if fail_preflight or setup:
                        result = process.wait(timeout=30)
                        output.seek(0)
                        logs = output.read().decode(errors="replace")
                        if fail_preflight:
                            marker = "verifying active encrypted databases" in logs
                            assert result == 1 and marker, (
                                f"preflight rejection mismatch: exit={result}, "
                                f"expected_marker={marker}"
                            )
                        else:
                            marker = "NOT READY" in logs
                            # Keep raw process output private. These bounded flags
                            # distinguish an unexpected verdict, crash and storage
                            # error without disclosing keys or login/pairing links.
                            diagnostics = {
                                "exit": result,
                                "not_ready": marker,
                                "ready": "Verdict        : READY" in logs,
                                "database_locked": "database is locked" in logs,
                                "storage_open_error": "opening memory database" in logs
                                    or "opening credential store" in logs,
                                "panic": "panicked at" in logs,
                            }
                            assert result == 2 and marker, (
                                f"setup-auth status mismatch: {diagnostics}"
                            )
                    else:
                        deadline = time.monotonic() + 45
                        while time.monotonic() < deadline:
                            assert process.poll() is None, "runtime exited before readiness"
                            try:
                                with urllib.request.urlopen(
                                    f"http://127.0.0.1:{child_env['AXUM_PORT']}/health", timeout=1
                                ) as response:
                                    if response.status == 200:
                                        break
                            except OSError:
                                time.sleep(0.1)
                        else:
                            raise AssertionError("runtime did not become ready")
                        output.seek(0)
                        logs = output.read().decode(errors="replace")
                        expected = "Experimental dual-write enabled" if mode == "on" else "[Heirloom] Disabled"
                        assert expected in logs, "unexpected Heirloom startup state"
                finally:
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait(timeout=10)

        for mode in (None, "off", "true", "ON", "typo"):
            run(mode)
            assert not heirloom_dir.exists(), "disabled mode created Heirloom state"
        assert not (root / "memory.db").read_bytes().startswith(b"SQLite format 3")
        print("PASS default/off/unrecognized modes boot with encrypted memory and no Heirloom directory")

        heirloom_dir.mkdir()
        with sqlite3.connect(heirloom) as legacy:
            legacy.execute("CREATE TABLE retained(value TEXT)")
            legacy.execute("INSERT INTO retained VALUES ('inactive-heirloom-fixture')")
        before = snapshot(heirloom_dir)
        run(None)
        # A status command may print the expected verdict before crashing during
        # SQLCipher/OpenSSL teardown. Check every subprocess's real exit status,
        # repeatedly against existing encrypted memory; never retry a failure.
        for _ in range(32):
            run("off", setup=True)
        assert snapshot(heirloom_dir) == before
        run("on", fail_preflight=True)
        assert snapshot(heirloom_dir) == before
        print("PASS inactive old file preserved; 32 setup-auth processes exited cleanly; opt-in rejects invalid storage")

        # Only remove this synthetic fixture inside the temporary test directory.
        heirloom.unlink()
        run("on")
        assert heirloom.exists()
        assert not heirloom.read_bytes().startswith(b"SQLite format 3")
        before = snapshot(heirloom_dir)
        run("off")
        assert snapshot(heirloom_dir) == before
        run("on")
        print("PASS explicit opt-in creates encrypted Heirloom; off preserves it; re-enable boots")

        run("off", path=root / "memory.db")
        run("off", fail_preflight=True, key=secrets.token_hex(32))
        print("PASS disabled path ignored; memory still rejects a wrong key")


if __name__ == "__main__":
    main()
