#!/usr/bin/env python3
"""Foreground CLI behavior in isolated temporary roots; no provider or service.

Child executables record invocation/exit/signal behavior. They do not simulate
provider acceptance or real runtime storage; see the documented browser harness.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().with_name('foreground-runtime.py')


class ForegroundTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='aegis-foreground-')
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / 'checkout with spaces'
        self.root.mkdir()
        (self.root / 'webapp').mkdir()
        self.bin = self.base / 'bin'
        self.bin.mkdir()
        self.events = self.base / 'events'
        self.env = dict(PATH=str(self.bin), HOME=str(self.base / 'home'),
                        AEGIS_RUNTIME_ROOT=str(self.root),
                        XDG_CONFIG_HOME=str(self.base / 'config'),
                        AEGIS_RUNTIME_STATE_DIR=str(self.base / 'state'),
                        EVENTS=str(self.events))
        for command in ('bash', 'python3', 'dirname'):
            (self.bin / command).symlink_to(shutil.which(command))
        self.python = shutil.which('python3')
        for name in ('cargo', 'rustup', 'cc', 'pkg-config', 'node', 'npm', 'systemctl', 'loginctl', 'codex', 'grok'):
            script = self.bin / name
            script.write_text(f'''#!{self.python}
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
with open(os.environ['EVENTS'], 'a') as f:
 f.write(json.dumps([name, sys.argv[1:], os.getcwd(), os.environ.get('CARGO_TARGET_DIR'), os.environ.get('CARGO_BUILD_JOBS')])+'\\n')
if name in ('systemctl','loginctl','codex','grok'):
 sys.exit(99)
if name == 'node': print(os.environ.get('NODE_MAJOR', '22'))
if name == os.environ.get('FAIL_TOOL'): sys.exit(17)
''')
            script.chmod(0o700)
        ports = []
        with socket.socket() as a, socket.socket() as b:
            for s in (a, b):
                s.bind(('127.0.0.1', 0))
                ports.append(str(s.getsockname()[1]))
        self.env.update(AXUM_PORT=ports[0], OAUTH_CALLBACK_PORT=ports[1])

    def invoke(self, *args):
        return subprocess.run([self.python, str(SCRIPT), *args], env=self.env,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20)

    def calls(self):
        return [json.loads(line) for line in self.events.read_text().splitlines()] if self.events.exists() else []

    def untouched(self):
        for path in (self.base / 'config', self.base / 'state', self.root / '.env'):
            self.assertFalse(path.exists(), str(path))
        self.assertFalse({'systemctl', 'loginctl', 'codex', 'grok'} & {c[0] for c in self.calls()})

    def runtime(self, body='import sys\nsys.exit(23)\n'):
        runtime = self.root / 'target/release/aegis_runtime_core'
        runtime.parent.mkdir(parents=True)
        runtime.write_text(f'#!{self.python}\n'+body)
        runtime.chmod(0o700)
        return runtime

    def test_preflight_without_systemd_or_logins(self):
        for command in ('systemctl', 'loginctl'):
            (self.bin / command).unlink()
        result = self.invoke('preflight')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('chat and persistence are unverified', result.stdout)
        self.untouched()

    def test_missing_and_old_build_tools_fail(self):
        self.env['NODE_MAJOR'] = '20'
        result = self.invoke('preflight')
        self.assertEqual(result.returncode, 1)
        self.assertIn('Node.js 22+', result.stderr)
        (self.bin / 'cargo').unlink()
        result = self.invoke('build')
        self.assertEqual(result.returncode, 1)
        self.assertIn('cargo', result.stderr)
        self.assertNotIn('npm', {c[0] for c in self.calls()})
        self.untouched()

    def test_prepared_runtime_preflight_needs_no_build_tools(self):
        for command in ('cargo', 'rustup', 'cc', 'pkg-config', 'node', 'npm'):
            (self.bin / command).unlink()
        result = self.invoke('preflight', '--skip-build')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.calls())
        self.untouched()

    def test_occupied_port_refused_without_changing_listener(self):
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            self.env['AXUM_PORT'] = str(listener.getsockname()[1])
            result = self.invoke('preflight', '--skip-build')
            self.assertEqual(result.returncode, 1)
            self.assertIn('unavailable', result.stderr)
            self.assertGreater(listener.fileno(), 0)
        self.untouched()

    def test_stopped_listener_time_wait_does_not_prevent_restart(self):
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            port = listener.getsockname()[1]
            with socket.create_connection(('127.0.0.1', port)) as client:
                connection, _ = listener.accept()
                connection.close()
                self.assertEqual(client.recv(1), b'')
        self.env['AXUM_PORT'] = str(port)
        result = self.invoke('preflight', '--skip-build')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.untouched()

    def test_managed_manifest_never_adopted_even_if_corrupt(self):
        ident = hashlib.sha256(str(self.root.resolve()).encode()).hexdigest()[:16]
        manifest = self.base / 'config/aegis-runtime/installations' / ident / 'install.json'
        manifest.parent.mkdir(parents=True)
        manifest.write_text('not valid json')
        result = self.invoke('preflight', '--skip-build')
        self.assertEqual(result.returncode, 1)
        self.assertIn('Managed installation exists', result.stderr)
        self.assertEqual(manifest.read_text(), 'not valid json')
        self.assertFalse(self.calls())

    def test_build_runs_only_build_commands_with_target_and_jobs(self):
        self.env.update(CARGO_TARGET_DIR='custom-target', CARGO_BUILD_JOBS='1')
        result = self.invoke('build')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [c for c in self.calls() if c[0] in ('npm', 'cargo')]
        self.assertEqual([c[:2] for c in calls], [
            ['npm', ['ci', '--no-audit', '--no-fund']],
            ['npm', ['run', 'build']],
            ['cargo', ['build', '--release', '-p', 'aegis-runtime-core', '-p', 'aegis-executor']],
        ])
        self.assertEqual([c[2] for c in calls], [str(self.root/'webapp'), str(self.root/'webapp'), str(self.root)])
        self.assertTrue(all(c[3:] == ['custom-target', '1'] for c in calls))
        self.untouched()

    def test_failed_build_stops_before_next_step(self):
        self.env['FAIL_TOOL'] = 'npm'
        result = self.invoke('build')
        self.assertEqual(result.returncode, 1)
        self.assertIn('exit 17', result.stderr)
        self.assertEqual([c[0] for c in self.calls()].count('npm'), 1)
        self.assertNotIn('cargo', {c[0] for c in self.calls()})
        self.untouched()

    def test_build_only_succeeds_with_occupied_runtime_ports(self):
        with socket.socket() as admin, socket.socket() as console:
            for name, listener in (('AXUM_PORT', admin), ('OAUTH_CALLBACK_PORT', console)):
                listener.bind(('127.0.0.1', 0))
                listener.listen()
                self.env[name] = str(listener.getsockname()[1])
            result = self.invoke('build')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual([c[0] for c in self.calls() if c[0] in ('npm', 'cargo')],
                             ['npm', 'npm', 'cargo'])
            for args in (('preflight', '--skip-build'), ('run',)):
                result = self.invoke(*args)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn('unavailable', result.stderr)
            self.assertEqual(str(admin.getsockname()[1]), self.env['AXUM_PORT'])
            self.assertEqual(str(console.getsockname()[1]), self.env['OAUTH_CALLBACK_PORT'])
        self.untouched()

    def test_unattended_run_never_generates_key_or_uses_dotenv_key(self):
        self.runtime()
        result = self.invoke('run')
        self.assertEqual(result.returncode, 1)
        self.assertIn('No key was generated', result.stderr)
        self.untouched()
        (self.root / '.env').write_text('AEGIS_RUNTIME_DB_KEY_HEX='+'1'*64+'\n')
        result = self.invoke('run')
        self.assertEqual(result.returncode, 1)
        self.assertIn('external secret source', result.stderr)

    def test_invalid_key_withheld_and_runtime_not_executed(self):
        self.runtime()
        self.env['AEGIS_RUNTIME_DB_KEY_HEX'] = 'invalid-synthetic-secret'
        result = self.invoke('run')
        self.assertEqual(result.returncode, 1)
        self.assertIn('64 hex', result.stderr)
        self.assertNotIn(self.env['AEGIS_RUNTIME_DB_KEY_HEX'], result.stdout+result.stderr)
        self.untouched()

    def test_run_preserves_exit_status_with_relative_target(self):
        runtime = self.runtime()
        runtime.parent.parent.rename(self.root/'custom-target')
        self.env.update(CARGO_TARGET_DIR='custom-target', AEGIS_RUNTIME_DB_KEY_HEX='1'*64)
        result = self.invoke('run')
        self.assertEqual(result.returncode, 23, result.stderr)
        self.untouched()

    def test_foreground_exec_receives_termination_directly(self):
        marker = self.base / 'ready'
        self.runtime(f'import os, pathlib, signal\npathlib.Path({str(marker)!r}).write_text(str(os.getpid()))\nsignal.pause()\n')
        self.env['AEGIS_RUNTIME_DB_KEY_HEX'] = '1'*64
        with tempfile.TemporaryFile() as output:
            proc = subprocess.Popen([self.python, str(SCRIPT), 'run'], env=self.env,
                                    stdin=subprocess.DEVNULL, stdout=output, stderr=output)
            try:
                deadline = time.monotonic()+10
                while not marker.exists() and proc.poll() is None and time.monotonic()<deadline:
                    time.sleep(0.02)
                self.assertTrue(marker.exists())
                self.assertEqual(int(marker.read_text()), proc.pid)
                proc.send_signal(signal.SIGTERM)
                self.assertEqual(proc.wait(timeout=5), -signal.SIGTERM)
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()
        self.untouched()


if __name__ == '__main__':
    unittest.main()
