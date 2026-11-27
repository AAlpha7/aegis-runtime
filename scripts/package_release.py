#!/usr/bin/env python3
"""Assemble a Linux release in a fresh stage, with notices and no stale files."""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile

from release_notices import build_notices

ROOT = Path(__file__).resolve().parents[1]


def assemble(target, destination):
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        raise ValueError('This release packager supports Linux x86_64 only')
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='aegis-package-', dir=destination.parent) as directory:
        stage = Path(directory) / 'aegis-runtime'
        stage.mkdir()
        for name in ['aegis_runtime_core', 'aegis_runtime_access', 'aegis_runtime_backup_db',
                     'aegis_runtime_encrypt_db', 'aegis-executor']:
            shutil.copy2(target / 'release' / name, stage / name)
        for name in ['README.md', 'SECURITY.md', 'AGENTS.md', 'LICENSE', 'THIRD_PARTY.md', '.env.example']:
            shutil.copy2(ROOT / name, stage / name)
        # Only tracked source/docs/scripts: local helper files cannot enter a package.
        names = subprocess.check_output(['git', 'ls-files', '-z', 'scripts', 'skills', 'docs'], cwd=ROOT).decode().split('\0')
        for name in filter(None, names):
            source = ROOT / name
            if source.is_symlink() or not source.is_file():
                raise ValueError(f'Unsafe tracked package path: {name}')
            output = stage / name
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, output)
            if source.suffix == '.sh':
                output.write_bytes(output.read_bytes().replace(b'\r\n', b'\n'))
        (stage / 'compiled_skills').mkdir()
        for name in ['web_reader.wasm', 'test_no_net.wasm']:
            shutil.copy2(target / 'wasm32-wasip1/release' / name, stage / 'compiled_skills' / name)
        private_prefixes = [str(Path.home()).encode(), str(ROOT).encode()]
        executables = [stage / name for name in ['aegis_runtime_core', 'aegis_runtime_access',
                       'aegis_runtime_backup_db', 'aegis_runtime_encrypt_db', 'aegis-executor']]
        for binary in [*executables, *sorted((stage / 'compiled_skills').glob('*.wasm'))]:
            if any(prefix in binary.read_bytes() for prefix in private_prefixes):
                raise ValueError(f'Builder path in {binary.name}; run the release build before packaging')
        frontend = ROOT / 'webapp/dist'
        if frontend.is_symlink() or any(path.is_symlink() for path in frontend.rglob('*')):
            raise ValueError('Frontend build contains a symlink')
        shutil.copytree(frontend, stage / 'webapp/dist')
        build_notices(stage)
        # Refuse state or symlinks even if accidentally added to the source list.
        for path in stage.rglob('*'):
            if (path.is_symlink() or path.name == '.env' or path.name.endswith('.env')
                    or path.name in {'auth.json', 'credentials.json'}
                    or path.suffix in {'.db', '.sqlite', '.sqlite3', '.key', '.hex', '.dpapi', '.pem', '.log'}):
                raise ValueError(f'Forbidden package entry: {path.relative_to(stage)}')
        temporary = Path(directory) / 'release.tar.gz'
        def public_metadata(info):
            info.uid = info.gid = 0
            info.uname = info.gname = ''
            info.mtime = 0
            executable = Path(info.name).name in {'aegis_runtime_core', 'aegis_runtime_access',
                         'aegis_runtime_backup_db', 'aegis_runtime_encrypt_db', 'aegis-executor'}
            info.mode = 0o755 if info.isdir() or executable or info.name.endswith('.sh') else 0o644
            return info

        with tarfile.open(temporary, 'w:gz') as archive:
            archive.add(stage, arcname='aegis-runtime', filter=public_metadata)
        os.replace(temporary, destination)
    print(f'Packaged: {destination}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    args = parser.parse_args()
    assemble(args.target.resolve(), args.archive.resolve())
