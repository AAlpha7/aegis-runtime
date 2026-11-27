#!/usr/bin/env python3
"""Explicit foreground route for Linux containers; no service/key/login setup."""
import argparse
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys

SCRIPT = Path(__file__).resolve()
spec = importlib.util.spec_from_file_location("managed_runtime", SCRIPT.with_name("managed-runtime.py"))
managed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(managed)


def preflight(app, skip_build=False):
    app.check_tools(skip_build, managed=False)
    # Do not query systemd, or allow an existing managed install to be adopted.
    app.check_ports(reuse_address=True, allow_managed=False)
    print("Foreground preflight passed: tools and loopback ports only. No service, key or provider login created; chat and persistence are unverified.", flush=True)


def main(args=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    commands.add_parser("preflight").add_argument("--skip-build", action="store_true")
    commands.add_parser("build", help="build source without account login or runtime state")
    commands.add_parser("run", help="exec the prepared runtime; no background process or automatic login")
    args = parser.parse_args(args)
    app = managed.Installation()
    if args.action == "preflight":
        preflight(app, args.skip_build)
    elif args.action == "build":
        # Building does not need unused ports or a running service manager.
        app.check_tools(managed=False)
        env = dict(os.environ)
        env.setdefault("CARGO_BUILD_JOBS", "2")
        for command, directory in (
            (["npm", "ci", "--no-audit", "--no-fund"], app.root / "webapp"),
            (["npm", "run", "build"], app.root / "webapp"),
            (["cargo", "build", "--release", "-p", "aegis-runtime-core", "-p", "aegis-executor"], app.root),
        ):
            subprocess.run(command, cwd=directory, env=env, check=True)
        print("Build complete. No account login, database key or runtime state was created. See docs/INSTALLATION.md before foreground run.")
    else:
        preflight(app, skip_build=True)
        target = Path(os.environ.get("CARGO_TARGET_DIR", app.root / "target"))
        if not target.is_absolute():
            target = app.root / target
        binary = Path(os.environ.get("AEGIS_RUNTIME_BIN", target / "release/aegis_runtime_core"))
        if not binary.is_absolute():
            binary = app.root / binary
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError("Runtime binary missing; run foreground-runtime.py build first, or set AEGIS_RUNTIME_BIN")
        key = os.environ.get("AEGIS_RUNTIME_DB_KEY_HEX")
        if key is not None and not re.fullmatch(r"[0-9a-fA-F]{64}", key):
            raise RuntimeError("AEGIS_RUNTIME_DB_KEY_HEX must be exactly 64 hex characters from an external secret source; value withheld")
        if key is None and not sys.stdin.isatty():
            raise RuntimeError("Unattended foreground run needs AEGIS_RUNTIME_DB_KEY_HEX from an external secret source (never .env). An interactive terminal can use the runtime passphrase prompt. No key was generated")
        print("Starting in foreground; provider authentication is still required. Stop with Ctrl-C or the container's signal. No restart or durable-service guarantee.", flush=True)
        # Preserve the supervisor's duplicate-install guard and exec/signal behavior.
        env = dict(os.environ, AEGIS_RUNTIME_BIN=str(binary))
        os.chdir(app.root)
        os.execvpe("bash", ["bash", str(SCRIPT.with_name("aegis-runtime-local.sh")), "run"], env)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except subprocess.CalledProcessError as error:
        print(f"Aegis foreground: build command failed (exit {error.returncode}); nothing was started", file=sys.stderr)
        sys.exit(1)
    except (RuntimeError, ValueError, OSError) as error:
        message = str(error) if isinstance(error, RuntimeError) else "Operation failed; inspect local paths and permissions"
        print("Aegis foreground: " + message, file=sys.stderr)
        sys.exit(1)
