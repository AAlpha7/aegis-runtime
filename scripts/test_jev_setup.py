#!/usr/bin/env python3
"""Exercise the real first-run API-key wizard with synthetic keys and a PTY.

No runtime, login, network or paid API is used. The auth binary is a local stub.
"""
import errno
import os
from pathlib import Path
import pty
import select
import subprocess
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().with_name("first-run.sh")
PROMPT = b"Enable optional Jev safety assessment? [y/N]: "
CHAT_KEY = "synthetic-openai-key-for-setup-tests"
OR_KEY = "sk-or-synthetic-key-for-setup-tests"


class JevSetupTests(unittest.TestCase):
    def run_wizard(self, answer=None, *, tty=True, assume_yes=False,
                   chat_key=OR_KEY, extra_env=None, initial="", expected_exit=0):
        with tempfile.TemporaryDirectory(prefix="aegis-jev-setup-") as temp:
            root = Path(temp)
            (root / ".env.example").write_text("ADMIN_CHAT_ID=1\n")
            if initial:
                (root / ".env").write_text(initial)
            stub = root / "auth-stub"
            stub.write_text('#!/bin/sh\nif [ "$2" = verify-jev ]; then exit "${TEST_JEV_PROBE_EXIT:-0}"; fi\nexit "${TEST_PROVIDER_PROBE_EXIT:-0}"\n')
            stub.chmod(0o700)
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith(("AEGIS_", "OPENAI_", "OPENROUTER_", "EMBEDDING_", "LLM_"))}
            env.update(AEGIS_RUNTIME_ROOT=str(root), AEGIS_RUNTIME_BIN=str(stub),
                       AEGIS_RUNTIME_STATE_DIR=str(root / "state"),
                       AEGIS_RUNTIME_SKIP_LOCAL_EMBEDDING="1", OPENAI_API_KEY=chat_key)
            env.update(extra_env or {})
            args = ["bash", str(SCRIPT), "--provider", "api-key", "--auth-only", "--skip-build"]
            if assume_yes:
                args.append("--yes")
            output = b""
            if tty:
                master, slave = pty.openpty()
                proc = subprocess.Popen(args, env=env, stdin=slave, stdout=slave, stderr=slave)
                os.close(slave)
                sent = False
                deadline = time.monotonic() + 20
                try:
                    while time.monotonic() < deadline:
                        if select.select([master], [], [], 0.1)[0]:
                            try:
                                chunk = os.read(master, 65536)
                            except OSError as err:
                                if err.errno == errno.EIO:
                                    break
                                raise
                            if not chunk:
                                break
                            output += chunk
                            if PROMPT in output and not sent and answer is not None:
                                os.write(master, (answer + "\n").encode())
                                sent = True
                    else:
                        self.fail("setup did not finish")
                    proc.wait(timeout=3)
                finally:
                    if proc.poll() is None:
                        proc.kill()
                        proc.wait()
                    os.close(master)
            else:
                proc = subprocess.run(args, env=env, input=b"yes\n", stdout=subprocess.PIPE,
                                      stderr=subprocess.STDOUT, timeout=20)
                output = proc.stdout
            text = output.decode(errors="replace")
            self.assertEqual(proc.returncode, expected_exit, text)
            self.assertNotIn(chat_key, text)
            for name in ("AEGIS_JEV_API_KEY", "OPENROUTER_API_KEY"):
                if extra_env and extra_env.get(name):
                    self.assertNotIn(extra_env[name], text)
            return text, (root / ".env").read_text()

    def test_yes_with_openrouter_key_enables(self):
        text, config = self.run_wizard("yes")
        self.assertIn("AEGIS_JEV_ENABLED=1", config)
        self.assertIn("proposed outbound content/action context", text)
        self.assertIn("separate API charges", text)

    def test_openrouter_recommends_deepseek_without_enabling_optional_services(self):
        for base in ("https://openrouter.ai/api/v1", "https://openrouter.ai/api/v1/"):
            with self.subTest(base=base):
                text, config = self.run_wizard(tty=False, extra_env={"OPENAI_API_BASE": base})
                self.assertIn("LLM_MODEL=deepseek/deepseek-v4.1-flash", config)
                self.assertIn("will verify it before accepting", text)
                self.assertNotIn("AEGIS_JEV_ENABLED=1", config)
                self.assertNotIn("fallback", config)

    def test_model_recommendation_preserves_explicit_choices_and_other_endpoints(self):
        cases = [
            ({"OPENAI_API_BASE": "https://openrouter.ai/api/v1"}, "LLM_MODEL=vendor/existing\n", "vendor/existing"),
            ({"OPENAI_API_BASE": "https://openrouter.ai/api/v1", "LLM_MODEL": "vendor/explicit"}, "", "vendor/explicit"),
            ({"OPENAI_API_BASE": "http://localhost:11434/v1"}, "", None),
            ({"OPENAI_API_BASE": "https://openrouter.ai.example/api/v1"}, "", None),
        ]
        for extra, initial, model in cases:
            with self.subTest(extra=extra, initial=initial):
                _, config = self.run_wizard(tty=False, extra_env=extra, initial=initial)
                if model:
                    self.assertIn(f"LLM_MODEL={model}", config)
                self.assertNotIn("deepseek/deepseek-v4.1-flash", config)

    def test_recommended_model_failure_restores_original_configuration(self):
        initial = "ADMIN_CHAT_ID=7\nOPENAI_API_BASE=https://openrouter.ai/api/v1\n"
        text, config = self.run_wizard(tty=False, initial=initial, expected_exit=1,
                                       extra_env={"TEST_PROVIDER_PROBE_EXIT": "2"})
        self.assertIn("Recommended API model: DeepSeek V4.1 Flash", text)
        self.assertEqual(config, initial)

    def test_decline_blank_and_invalid_keep_off(self):
        for answer in ("n", "", "maybe"):
            with self.subTest(answer=answer):
                _, config = self.run_wizard(answer)
                self.assertIn("AEGIS_JEV_ENABLED=0", config)

    def test_generic_openai_key_cannot_enable_jev(self):
        text, config = self.run_wizard("y", chat_key=CHAT_KEY)
        self.assertIn("no usable OpenRouter credential", text)
        self.assertNotIn("AEGIS_JEV_ENABLED=1", config)
        self.assertIn(f"OPENAI_API_KEY={CHAT_KEY}", config)

    def test_dedicated_credential_can_enable_without_copying_it(self):
        for name in ("AEGIS_JEV_API_KEY", "OPENROUTER_API_KEY"):
            with self.subTest(name=name):
                _, config = self.run_wizard("Y", chat_key=CHAT_KEY, extra_env={name: OR_KEY})
                self.assertIn("AEGIS_JEV_ENABLED=1", config)
                self.assertNotIn(OR_KEY, config)

    def test_placeholder_cannot_enable_jev(self):
        text, config = self.run_wizard("y", chat_key=CHAT_KEY,
                                       extra_env={"AEGIS_JEV_API_KEY": "sk-or-xxxxxxxx-placeholder-for-test"})
        self.assertIn("no usable OpenRouter credential", text)
        self.assertNotIn("AEGIS_JEV_ENABLED=1", config)

    def test_live_probe_failure_does_not_save_opt_in(self):
        text, config = self.run_wizard("yes", extra_env={"TEST_JEV_PROBE_EXIT": "2"})
        self.assertIn("validation failed", text)
        self.assertNotIn("AEGIS_JEV_ENABLED=1", config)

    def test_provider_failure_restores_prior_file_exactly(self):
        original = "ADMIN_CHAT_ID=7\nOPENAI_API_KEY=previous-private-key\nAEGIS_JEV_ENABLED=0\n"
        text, config = self.run_wizard(tty=False, initial=original, expected_exit=1,
                                       extra_env={"TEST_PROVIDER_PROBE_EXIT": "2"})
        self.assertEqual(config, original)
        self.assertIn("previous .env configuration was restored", text)

    def test_yes_flag_and_piped_input_are_not_consent(self):
        for tty, assume_yes in ((True, True), (False, False)):
            with self.subTest(tty=tty):
                text, config = self.run_wizard(tty=tty, assume_yes=assume_yes)
                self.assertNotIn(PROMPT.decode(), text)
                self.assertNotIn("AEGIS_JEV_ENABLED=1", config)

    def test_existing_choices_are_preserved_without_prompt(self):
        for choice in ("0", "1"):
            with self.subTest(choice=choice):
                text, config = self.run_wizard(initial=f"AEGIS_JEV_ENABLED={choice}\n")
                self.assertNotIn(PROMPT.decode(), text)
                self.assertIn(f"AEGIS_JEV_ENABLED={choice}", config)

    def test_environment_opt_out_overrides_saved_opt_in(self):
        text, config = self.run_wizard(initial="AEGIS_JEV_ENABLED=1\n",
                                       extra_env={"AEGIS_JEV_ENABLED": "0"})
        self.assertNotIn(PROMPT.decode(), text)
        self.assertIn("existing choice preserved", text)
        self.assertIn("AEGIS_JEV_ENABLED=1", config)

    def test_empty_exported_credential_does_not_reuse_shadowed_file_key(self):
        text, config = self.run_wizard("y", chat_key=CHAT_KEY,
                                       initial=f"OPENROUTER_API_KEY={OR_KEY}\n",
                                       extra_env={"OPENROUTER_API_KEY": ""})
        self.assertIn("no usable OpenRouter credential", text)
        self.assertNotIn("AEGIS_JEV_ENABLED=1", config)


if __name__ == "__main__":
    unittest.main()
