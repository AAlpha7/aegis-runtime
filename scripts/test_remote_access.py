"""Behavioral deployment tests; no network account, daemon, or model key required."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("remote_access", Path(__file__).with_name("remote-access.py"))
access = importlib.util.module_from_spec(spec)
spec.loader.exec_module(access)


class DeploymentBehavior(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = self.root / ".env"
        self.env.write_text("# preserved comment\nUNRELATED_SETTING=keep\nMEMORY_DB_PATH=memory.db\n")
        self.helper = self.root / "aegis_runtime_access"
        self.helper.touch()
        self.config = {}
        self.calls = []
        self.fail_restart = False
        self.fail_reset = False
        self.wrong_route = False
        self.hostname = "computer.example.ts.net"
        self.env_patch = patch.dict(os.environ, {"AEGIS_RUNTIME_ACCESS_BIN": str(self.helper)}, clear=True)
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)

    def run_command(self, args, *, capture=True, extra_env=None):
        self.calls.append(args)
        if args[0] == "tailscale":
            if args[1:] == ["status", "--json"]:
                return json.dumps({"BackendState": "Running", "Self": {"DNSName": self.hostname + "."}})
            if args[1:] == ["serve", "status", "--json"]:
                return json.dumps(self.config)
            if args[1:] == ["serve", "reset"]:
                if self.fail_reset:
                    raise subprocess.CalledProcessError(1, args)
                self.config = {}
                return ""
            host = self.hostname + ":443"
            self.config = {"TCP": {"443": {"HTTPS": True}}, "Web": {host: {"Handlers": {"/": {"Proxy": args[-1]}}}}, "AllowFunnel": {host: args[1] == "funnel"}}
            if self.wrong_route:
                self.config["AllowFunnel"][host] = True
            return ""
        if args[0] == "bash" and self.fail_restart:
            raise subprocess.CalledProcessError(1, args)
        if args[0] == "bash":
            self.assertEqual(extra_env, {"AEGIS_RUNTIME_ROOT": str(self.root)})
        return "registered_passkeys=0\n"

    def deploy(self, mode, consent=False):
        args = ["--root", str(self.root), "--mode", mode]
        if consent:
            args.append("--accept-public-risk")
        with patch.object(access, "command", self.run_command), patch.object(access.shutil, "which", return_value="tailscale"), patch("sys.stdin.isatty", return_value=False), contextlib.redirect_stdout(io.StringIO()):
            access.main(args)

    def test_public_requires_explicit_consent_before_any_change(self):
        before = self.env.read_bytes()
        with self.assertRaisesRegex(RuntimeError, "acknowledgment"):
            self.deploy("public")
        self.assertEqual(self.calls, [])
        self.assertEqual(before, self.env.read_bytes())

    def test_private_public_private_local_switches_remove_public_exposure(self):
        self.deploy("private")
        self.assertFalse(any(self.config["AllowFunnel"].values()))
        self.deploy("public", True)
        self.assertTrue(any(self.config["AllowFunnel"].values()))
        self.deploy("private")
        self.assertFalse(any(self.config["AllowFunnel"].values()))
        self.deploy("local")
        self.assertEqual(self.config, {})
        settings = access.read_env(self.env)
        self.assertEqual(settings["AEGIS_PUBLIC_URL"], "")
        self.assertEqual(settings["UNRELATED_SETTING"], "keep")
        self.assertEqual(settings["AEGIS_PUBLIC_RISK_ACCEPTED"], "")
        self.assertIn("# preserved comment", self.env.read_text())

    def test_unmanaged_configuration_is_never_overwritten(self):
        self.config = {"TCP": {"8443": {"HTTPS": True}}}
        with self.assertRaisesRegex(RuntimeError, "unmanaged"):
            self.deploy("private")
        self.assertFalse(any("reset" in call for call in self.calls))

    def test_concurrent_external_edit_is_not_overwritten(self):
        self.deploy("private")
        self.config["OtherService"] = {"owner": "someone-else"}
        self.calls.clear()
        with self.assertRaisesRegex(RuntimeError, "unmanaged"):
            self.deploy("public", True)
        self.assertFalse(any("reset" in call for call in self.calls))

    def test_failed_shutdown_does_not_claim_private_or_change_mode(self):
        self.deploy("public", True)
        self.fail_reset = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy("private")
        self.assertEqual(access.read_env(self.env)["AEGIS_ACCESS_MODE"], "public")

    def test_failed_restart_leaves_tunnel_closed(self):
        self.deploy("public", True)
        self.fail_restart = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.deploy("private")
        self.assertEqual(self.config, {})
        self.assertEqual(json.loads((self.root / ".aegis-access.json").read_text())["mode"], "transition")

    def test_unexpected_public_activation_is_removed(self):
        self.wrong_route = True
        with self.assertRaisesRegex(RuntimeError, "exposure"):
            self.deploy("private")
        self.assertEqual(self.config, {})

    def test_environment_cannot_silently_override_selected_mode(self):
        with patch.dict(os.environ, {"AEGIS_PUBLIC_URL": "https://other.example"}):
            with self.assertRaisesRegex(RuntimeError, "Unset"):
                self.deploy("private")
        self.assertEqual(self.calls, [])

    def test_failure_cleanup_preserves_concurrent_unmanaged_service(self):
        original = self.run_command
        def concurrent_command(args, **kwargs):
            result = original(args, **kwargs)
            if "--bg" in args:
                self.config["OtherService"] = {"owner": "someone-else"}
            return result
        with patch.object(self, "run_command", concurrent_command), contextlib.redirect_stderr(io.StringIO()) as warning:
            with self.assertRaisesRegex(RuntimeError, "unmanaged"):
                self.deploy("private")
        self.assertIn("OtherService", self.config)
        self.assertFalse(any("reset" in call for call in self.calls))
        self.assertIn("MAY STILL BE ACTIVE", warning.getvalue())

    def test_legacy_remote_route_requires_explicit_migration(self):
        with self.env.open("a") as stream:
            stream.write("AEGIS_PUBLIC_URL=https://previous.example\n")
        with self.assertRaisesRegex(RuntimeError, "not managed"):
            self.deploy("local")
        self.assertEqual(self.calls, [])

    def test_empty_daemon_config_and_invalid_output(self):
        with patch.object(access, "command", return_value="null"):
            self.assertEqual(access.read_config("tailscale"), {})
        with patch.object(access, "command", return_value="[]"):
            with self.assertRaisesRegex(RuntimeError, "format"):
                access.read_config("tailscale")


if __name__ == "__main__":
    unittest.main()
