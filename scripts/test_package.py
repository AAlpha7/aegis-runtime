#!/usr/bin/env python3
"""Check a local Linux release archive without provider accounts or user data."""
import argparse
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import tarfile
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    parser.add_argument('--preflight', action='store_true',
                        help='also check this host has a systemd user manager and lingering')
    args = parser.parse_args()
    with tarfile.open(args.archive) as package, tempfile.TemporaryDirectory(prefix='aegis-package-check-') as directory:
        names = package.getnames()
        assert all(not member.uname and not member.gname and member.uid == member.gid == 0
                   for member in package.getmembers()), 'builder identity in archive metadata'
        assert all(not member.mode & 0o022 for member in package.getmembers()), 'group/world-writable package entry'
        required = ['aegis_runtime_core', 'aegis_runtime_access', 'webapp/dist/index.html',
                    'scripts/first-run.sh', 'scripts/managed-runtime.py', 'scripts/install-windows.ps1',
                    'compiled_skills/web_reader.wasm', 'docs/INSTALLATION.md', 'LICENSE',
                    'THIRD_PARTY.md', 'THIRD_PARTY_NOTICES.txt', 'dependency-inventory.json',
                    'docs/SUBSCRIPTIONS.md', 'docs/HEIRLOOM.md',
                    'rust-toolchain-notices/COPYRIGHT-library.html']
        assert all('aegis-runtime/' + name in names for name in required), 'missing package content'
        assert all(Path(name).name not in {'.env', 'auth.json', 'credentials.json'}
                   and not name.endswith(('.env', '.db', '.sqlite', '.sqlite3', '.hex', '.key', '.pem', '.dpapi', '.log'))
                   for name in names), 'runtime state in package'
        assert not any('/skills/web_search' in name or name.endswith('/web_search.wasm') for name in names), 'retired search in package'
        package.extractall(directory, filter='data')
        root = Path(directory) / 'aegis-runtime'
        private_prefixes = [str(Path.home()).encode(), str(Path(__file__).resolve().parents[1]).encode()]
        binaries = [root / name for name in ['aegis_runtime_core', 'aegis-executor',
                    'aegis_runtime_access', 'aegis_runtime_backup_db', 'aegis_runtime_encrypt_db']]
        for binary in [*binaries, *sorted((root / 'compiled_skills').glob('*.wasm'))]:
            content = binary.read_bytes()
            assert not any(prefix in content for prefix in private_prefixes), f'builder path in {binary.name}'
        assert (root / 'LICENSE').read_bytes() == (Path(__file__).resolve().parents[1] / 'LICENSE').read_bytes(), 'project license changed or omitted'
        inventory = json.loads((root / 'dependency-inventory.json').read_text())
        assert inventory and all(item.get('notice_files') for item in inventory), 'incomplete notices'
        assert any(item['name'] == 'libsqlite3-sys' and 'sqlcipher/LICENSE' in item['notice_files'] for item in inventory), 'SQLCipher notice omitted'
        assert {'react', 'react-dom', 'scheduler'} <= {item['name'] for item in inventory if item['ecosystem'] == 'npm'}, 'frontend notices omitted'
        for item in inventory:
            if 'MPL-' in (item.get('license') or ''):
                assert (root / item['bundled_source']).is_file(), 'MPL source archive omitted'
        for script in (root / 'scripts').glob('*.sh'):
            assert b'\r\n' not in script.read_bytes(), f'Windows line endings: {script.name}'
            subprocess.run(['bash', '-n', str(script)], check=True)
        subprocess.run(['bash', 'scripts/first-run.sh', '--help'], cwd=root, check=True, capture_output=True)
        if args.preflight:
            subprocess.run(['python3', 'scripts/managed-runtime.py', 'preflight', '--skip-build'], cwd=root, check=True)
        result = subprocess.run([str(root / 'aegis_runtime_core'), 'setup-auth', '--provider', 'invalid-fixture-provider'],
                                cwd=root, capture_output=True,
                                env=dict(os.environ, AEGIS_RUNTIME_DB_KEY_HEX=secrets.token_hex(32)))
        assert result.returncode == 1 and b'invalid-fixture-provider' in result.stderr, f'runtime argument validation: expected exit 1, got {result.returncode}'
        assert os.access(root / 'aegis_runtime_access', os.X_OK), 'access helper not executable'
        # Exercise the crypto implementation actually shipped after native-path
        # remapping, using disposable synthetic data and no provider account.
        plain = Path(directory) / 'fixture.db'
        encrypted = Path(directory) / 'encrypted.db'
        backup = Path(directory) / 'backup.db'
        with sqlite3.connect(plain) as connection:
            connection.execute('CREATE TABLE release_probe(value TEXT)')
            connection.execute('INSERT INTO release_probe VALUES (?)', ('synthetic-release-probe',))
        environment = dict(os.environ, AEGIS_RUNTIME_DB_KEY_HEX=secrets.token_hex(32))
        subprocess.run([str(root / 'aegis_runtime_encrypt_db'), str(plain), str(encrypted)],
                       env=environment, check=True, capture_output=True)
        subprocess.run([str(root / 'aegis_runtime_backup_db'), str(encrypted), str(backup)],
                       env=environment, check=True, capture_output=True)
        for database in [encrypted, backup]:
            content = database.read_bytes()
            assert not content.startswith(b'SQLite format 3') and b'synthetic-release-probe' not in content
        rejected_output = Path(directory) / 'wrong-key.db'
        rejected = subprocess.run([str(root / 'aegis_runtime_backup_db'), str(encrypted), str(rejected_output)],
                                  env=dict(environment, AEGIS_RUNTIME_DB_KEY_HEX=secrets.token_hex(32)), capture_output=True)
        assert rejected.returncode == 1 and not rejected_output.exists(), f'wrong-key rejection: expected exit 1 without output, got {rejected.returncode}'
        print('PASS: extracted Linux package, notices, permissions, private-path exclusion, startup and real encrypted migration/backup/wrong-key checks. No provider account or chat was used.')


if __name__ == '__main__':
    main()
