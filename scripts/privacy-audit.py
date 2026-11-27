#!/usr/bin/env python3
"""High-signal tracked-file privacy check. Reports locations, never matched values.

This supplements review; a clean result is not proof that all secrets are absent.
Generated files and local credentials are excluded by scanning Git's file list.
"""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RULES = {
    "private-key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"),
    "github-token": re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{60,})\b"),
    "provider-token": re.compile(r"\b(?:crsr_[a-f0-9]{40,}|sk-(?:proj-|or-v1-)?[A-Za-z0-9_-]{40,})\b"),
    "personal-workspace": re.compile(r"[A-Z]:[\\/]Users[\\/](?!Public\b|USER\b|user\b)[^\s/\\]+", re.I),
    "private-email": re.compile(r"\b[A-Za-z0-9._%+-]+@(?:gmail|hotmail|outlook|icloud|yahoo|protonmail)\.[A-Za-z]+\b"),
}

# Exact reviewed synthetic test lines, not blanket exclusions for test files or
# generic allow-comments. A changed fixture must be reviewed again.
FIXTURES = {
    ("core/src/admin_tests/part3_web_hardening.rs", "private-key"):
        r'let pem = "-----BEGIN ' r'EC PRIVATE KEY-----\nabc\n-----END EC PRIVATE KEY-----";',
    ("core/src/orchestrator/tests/mod.rs", "github-token"):
        'let args = "ghp_' '1234567890abcdef1234567890abcdefABCD"; // gitleaks:allow',
}


def main():
    names = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT).decode().split("\0")
    found = []
    for name in filter(None, names):
        path = ROOT / name
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeError, OSError):
            continue
        for number, line in enumerate(text.splitlines(), 1):
            for rule, pattern in RULES.items():
                if pattern.search(line):
                    if line.strip() == FIXTURES.get((name, rule)):
                        continue
                    found.append(f"{name}:{number}: {rule}")
    for finding in found:
        print(finding)
    print(f"Privacy scan: {len(found)} findings in tracked files; manual review still required.")
    return bool(found)


if __name__ == "__main__":
    sys.exit(main())
