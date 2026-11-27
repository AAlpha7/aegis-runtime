#!/usr/bin/env python3
"""Real local supervisor + encrypted harness + deployment helper lifecycle.
Only the Tailscale daemon is simulated; all processes stay on loopback.
"""
import json
import os
from pathlib import Path
import secrets
import shlex
import socket
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
SUPERVISOR = ROOT / "scripts/aegis-runtime-local.sh"
DEPLOY = ROOT / "scripts/remote-access.py"

FAKE_TAILSCALE = '''#!/usr/bin/env python3
import json, os, pathlib, sys
p = pathlib.Path(os.environ['TEST_TAILSCALE_STATE'])
c = json.loads(p.read_text()) if p.exists() else {}
a = sys.argv[1:]
if a == ['status', '--json']:
    print(json.dumps({'BackendState':'Running','Self':{'DNSName':'computer.test.ts.net.'}}))
elif a == ['serve', 'status', '--json']:
    print(json.dumps(c))
elif a == ['serve', 'reset']:
    p.write_text('{}')
elif a[:3] in (['serve','--bg','--https=443'], ['funnel','--bg','--https=443']):
    p.write_text(json.dumps({'TCP':{'443':{'HTTPS':True}},'Web':{'computer.test.ts.net:443':{'Handlers':{'/':{'Proxy':a[3]}}}},'AllowFunnel':{'computer.test.ts.net:443':a[0]=='funnel'}}))
else:
    sys.exit(2)
'''


def main():
    with tempfile.TemporaryDirectory(prefix="aegis-lifecycle-") as directory:
        root = Path(directory)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        db = root / "memory.db"
        (root / ".env").write_text(f"MEMORY_DB_PATH={db}\nAXUM_PORT={port}\nOAUTH_CALLBACK_PORT={port}\n")
        fake = root / "tailscale"
        fake.write_text(FAKE_TAILSCALE)
        fake.chmod(0o700)
        launcher = root / "runtime"
        launcher.write_text("#!/bin/sh\nexec " + " ".join(map(shlex.quote, [str(TARGET / "debug/examples/remote_access_harness"), str(db), str(port)])) + "\n")
        launcher.chmod(0o700)
        env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                   AEGIS_RUNTIME_ROOT=str(root), AEGIS_RUNTIME_BIN=str(launcher),
                   AEGIS_RUNTIME_ACCESS_BIN=str(TARGET / "debug/aegis_runtime_access"),
                   AEGIS_RUNTIME_STATE_DIR=str(root / "state"),
                   AEGIS_RUNTIME_DB_KEY_HEX=secrets.token_hex(32),
                   TEST_TAILSCALE_STATE=str(root / "tailscale.json"))
        for key in ("AEGIS_ACCESS_MODE", "AEGIS_PUBLIC_URL", "OAUTH_CALLBACK_BIND", "AEGIS_PUBLIC_RISK_ACCEPTED", "AXUM_PORT", "OAUTH_CALLBACK_PORT", "MEMORY_DB_PATH"):
            env.pop(key, None)

        def run(args, expected=0):
            # A WSL task can inherit a terminal. This noninteractive fixture
            # must reject unacknowledged public mode, never wait for user input.
            result = subprocess.run(args, env=env, cwd=ROOT, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=180)
            # Never dump command output: successful pairing prints a short-lived secret.
            assert result.returncode == expected, f"command failed: {args[0:2]}, exit={result.returncode}"

        try:
            run(["bash", str(SUPERVISOR), "start"])
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health") as response:
                assert response.status == 200
            run(["python3", str(DEPLOY), "--root", str(root), "--mode", "public"], expected=1)
            assert not (root / "tailscale.json").exists()
            for mode in ("private", "public", "private", "local"):
                run(["python3", str(DEPLOY), "--root", str(root), "--mode", mode] + (["--accept-public-risk"] if mode == "public" else []))
                run(["bash", str(SUPERVISOR), "status"])
                config = json.loads((root / "tailscale.json").read_text())
                assert any(config.get("AllowFunnel", {}).values()) == (mode == "public")
                assert json.loads((root / ".aegis-access.json").read_text())["mode"] == mode
                print(f"PASS real supervisor restart and {mode} deployment transition")
            assert db.read_bytes()[:16] != b"SQLite format 3\x00"
            print("PASS encrypted state persisted across all transitions; no real accounts used")
        finally:
            subprocess.run(["bash", str(SUPERVISOR), "stop"], env=env, cwd=ROOT,
                           stdin=subprocess.DEVNULL, capture_output=True, timeout=60)


if __name__ == "__main__":
    main()
