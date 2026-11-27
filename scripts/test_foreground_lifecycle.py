#!/usr/bin/env python3
"""Run the existing encrypted loopback harness through the foreground launcher.

Temporary fixture key/state only; no provider, systemd or real Tailscale calls.
This proves local foreground routing/restart behavior, not container persistence.
"""
import os
from pathlib import Path
import secrets
import shlex
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target')).resolve()
HARNESS = TARGET / 'debug/examples/remote_access_harness'


def main():
    assert HARNESS.is_file(), 'Build remote_access_harness before this test'
    with tempfile.TemporaryDirectory(prefix='aegis-foreground-lifecycle-') as directory:
        root = Path(directory)
        db = root / 'memory.db'
        with socket.socket() as a, socket.socket() as b:
            a.bind(('127.0.0.1', 0))
            b.bind(('127.0.0.1', 0))
            port, admin_port = a.getsockname()[1], b.getsockname()[1]
        launcher = root / 'harness'
        launcher.write_text('#!/bin/sh\nexec ' + ' '.join(map(shlex.quote, [str(HARNESS), str(db), str(port)])) + '\n')
        launcher.chmod(0o700)
        env = dict(PATH=os.environ['PATH'], HOME=str(root),
                   XDG_CONFIG_HOME=str(root/'config'),
                   AEGIS_RUNTIME_ROOT=str(root), AEGIS_RUNTIME_BIN=str(launcher),
                   AEGIS_RUNTIME_STATE_DIR=str(root/'state'),
                   AEGIS_RUNTIME_DB_KEY_HEX=secrets.token_hex(32),
                   AEGIS_WEBAPP_DIR=str(ROOT/'webapp/dist'),
                   AXUM_PORT=str(admin_port), OAUTH_CALLBACK_PORT=str(port))
        command = ['python3', str(ROOT/'scripts/foreground-runtime.py'), 'run']
        origin = f'http://127.0.0.1:{port}'
        for attempt in range(2):
            with tempfile.TemporaryFile() as logs:
                proc = subprocess.Popen(command, env=env, cwd=ROOT, stdin=subprocess.DEVNULL,
                                        stdout=logs, stderr=logs)
                try:
                    deadline = time.monotonic()+30
                    while time.monotonic()<deadline:
                        assert proc.poll() is None, 'harness exited before ready; raw logs withheld'
                        try:
                            with urllib.request.urlopen(origin+'/health', timeout=1) as response:
                                assert response.status == 200
                            break
                        except (urllib.error.URLError, TimeoutError):
                            time.sleep(0.1)
                    else:
                        raise AssertionError('foreground harness readiness timeout')
                    with urllib.request.urlopen(origin+'/ui/', timeout=5) as response:
                        assert response.status == 200
                        assert b'<html' in response.read().lower()
                    assert db.read_bytes()[:16] != b'SQLite format 3\0'
                    if attempt == 0:
                        inode = db.stat().st_ino
                    else:
                        assert db.stat().st_ino == inode, 'database was replaced across restart'
                    print(f'PASS foreground harness start {attempt+1}: HTTP/UI and encrypted database')
                finally:
                    if proc.poll() is None:
                        proc.send_signal(signal.SIGTERM)
                        try:
                            assert proc.wait(timeout=10) == -signal.SIGTERM
                        except subprocess.TimeoutExpired:
                            proc.kill()
                            proc.wait()
                            raise
        original = db.read_bytes()
        wrong = dict(env, AEGIS_RUNTIME_DB_KEY_HEX=secrets.token_hex(32))
        result = subprocess.run(command, env=wrong, cwd=ROOT, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=30)
        assert result.returncode == 1, 'wrong key must fail without a crash'
        assert db.read_bytes() == original, 'wrong-key attempt changed encrypted state'
        assert not (root/'config').exists() and not (root/'state').exists()
        assert not (root/'.env').exists()
        print('PASS foreground signal exit, encrypted reopen, wrong-key rejection; no service/key files created')


if __name__ == '__main__':
    main()
