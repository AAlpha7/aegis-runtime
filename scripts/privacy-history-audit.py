#!/usr/bin/env python3
"""Read-only reachable Git-history scan. Reports locations, never secret values."""
import importlib.util
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('privacy_audit', ROOT / 'scripts/privacy-audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
RULES = dict(audit.RULES, **{
    'private-tailnet': re.compile(r'https?://[^\s/]+\.ts\.net'),
    'pairing-value': re.compile(r'#pair=[a-f0-9]{32,}'),
})
EXAMPLE_ORIGINS = {'https://computer.tailnet.ts.net', 'https://device.example.ts.net',
                   'https://other.example.ts.net'}


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)


def matches(text, name):
    for number, line in enumerate(text.splitlines(), 1):
        for rule, pattern in RULES.items():
            hits = pattern.findall(line)
            if not hits or line.strip() == audit.FIXTURES.get((name, rule)):
                continue
            if rule == 'private-tailnet' and all(hit in EXAMPLE_ORIGINS for hit in hits):
                continue
            yield number, rule


def main():
    entries = git('rev-list', '--objects', '--all').decode().splitlines()
    findings, blobs, binaries = 0, 0, set()
    process = subprocess.Popen(['git', 'cat-file', '--batch'], cwd=ROOT,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        for entry in entries:
            oid, _, name = entry.partition(' ')
            process.stdin.write((oid + '\n').encode())
            process.stdin.flush()
            header = process.stdout.readline().decode().split()
            data = process.stdout.read(int(header[2]))
            process.stdout.read(1)
            if header[1] == 'commit':
                # Author attribution is intentional Git metadata; scan the message.
                text = data.decode(errors='replace').partition('\n\n')[2]
                name = '(commit-message)'
            elif header[1] == 'blob':
                blobs += 1
                if Path(name).name == '.env' or name.endswith(('.db', '.sqlite', '.sqlite3', '.key', '.hex', '.dpapi', '.log')):
                    print(f'{oid[:12]} {name}: sensitive-filename')
                    findings += 1
                try:
                    text = data.decode('utf-8')
                except UnicodeDecodeError:
                    binaries.add(name)
                    continue
            else:
                continue
            for number, rule in matches(text, name):
                print(f'{oid[:12]} {name}:{number}: {rule}')
                findings += 1
    finally:
        process.stdin.close()
        process.wait()
    for name in sorted(binaries):
        print('Manual binary review: ' + name)
    commits = int(git('rev-list', '--count', '--all'))
    print(f'History scan: {findings} findings; {commits} commits, {blobs} unique blobs. '
          'Reachable refs only; manual review and author-metadata review still required.')
    return bool(findings)


if __name__ == '__main__':
    raise SystemExit(main())
