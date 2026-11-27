#!/usr/bin/env python3
"""Behavior checks for stale-file exclusion and failed-package preservation."""
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package_release


class PackagingTests(unittest.TestCase):
    def setUp(self):
        # Fixtures are synthetic files, so the packaging logic is host
        # independent; pin the platform so these run on aarch64 dev hosts too.
        # The guard itself is covered by test_rejects_non_x86_64_host.
        for name, value in (('system', 'Linux'), ('machine', 'x86_64')):
            patcher = patch.object(package_release.platform, name, return_value=value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def test_rejects_non_x86_64_host(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(package_release.platform, 'machine', return_value='aarch64'):
            root = Path(directory)
            target, archive = self.fixture(root)
            with self.assertRaisesRegex(ValueError, 'Linux x86_64 only'):
                package_release.assemble(target, archive)
            self.assertFalse(archive.exists())

    def fixture(self, root):
        target = root / 'build/release'
        target.mkdir(parents=True)
        for name in ['aegis_runtime_core', 'aegis_runtime_access', 'aegis_runtime_backup_db',
                     'aegis_runtime_encrypt_db', 'aegis-executor']:
            (target / name).write_text('fixture executable')
        for name in ['README.md', 'SECURITY.md', 'AGENTS.md', 'LICENSE', 'THIRD_PARTY.md', '.env.example']:
            (root / name).write_text('fixture document')
        for directory in ['scripts', 'docs', 'skills', 'compiled_skills', 'webapp/dist', 'dist/aegis-runtime']:
            (root / directory).mkdir(parents=True, exist_ok=True)
        (root / 'scripts/first-run.sh').write_bytes(b'#!/bin/sh\r\nexit 0\r\n')
        (root / 'scripts/private-helper.py').write_text('untracked helper')
        (root / 'docs/INSTALLATION.md').write_text('install')
        (root / 'webapp/dist/index.html').write_text('console')
        (root / 'build/wasm32-wasip1/release').mkdir(parents=True)
        for name in ['web_reader.wasm', 'test_no_net.wasm', 'web_search.wasm']:
            (root / 'compiled_skills' / name).write_bytes(b'fixture wasm')
            (root / 'build/wasm32-wasip1/release' / name).write_bytes(b'fixture wasm')
        (root / 'dist/aegis-runtime/.env').write_text('OLD_PRIVATE_SETTING=fixture')
        return root / 'build', root / 'dist/result.tar.gz'

    def test_fresh_stage_excludes_stale_and_untracked_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target, archive = self.fixture(root)
            with patch.object(package_release, 'ROOT', root), \
                 patch.object(package_release.subprocess, 'check_output', return_value=b'scripts/first-run.sh\0docs/INSTALLATION.md\0'), \
                 patch.object(package_release, 'build_notices', side_effect=lambda stage: (stage / 'THIRD_PARTY_NOTICES.txt').write_text('fixture notices')):
                package_release.assemble(target, archive)
            with tarfile.open(archive) as package:
                names = package.getnames()
                self.assertIn('aegis-runtime/LICENSE', names)
                self.assertNotIn('aegis-runtime/.env', names)
                self.assertNotIn('aegis-runtime/scripts/private-helper.py', names)
                self.assertNotIn('aegis-runtime/compiled_skills/web_search.wasm', names)
                self.assertTrue(all(not member.uname and not member.gname and member.uid == member.gid == 0 for member in package.getmembers()))
                self.assertTrue(all(not member.mode & 0o022 for member in package.getmembers()))
                self.assertNotIn(b'\r', package.extractfile('aegis-runtime/scripts/first-run.sh').read())

    def test_missing_notices_preserves_previous_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target, archive = self.fixture(root)
            archive.write_bytes(b'previous known-good archive')
            with patch.object(package_release, 'ROOT', root), \
                 patch.object(package_release.subprocess, 'check_output', return_value=b''), \
                 patch.object(package_release, 'build_notices', side_effect=ValueError('missing notices')):
                with self.assertRaisesRegex(ValueError, 'missing notices'):
                    package_release.assemble(target, archive)
            self.assertEqual(archive.read_bytes(), b'previous known-good archive')

    def test_rejects_state_in_frontend_dist(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target, archive = self.fixture(root)
            (root / 'webapp/dist/credential.dpapi').write_bytes(b'fixture encrypted key')
            with patch.object(package_release, 'ROOT', root), \
                 patch.object(package_release.subprocess, 'check_output', return_value=b''), \
                 patch.object(package_release, 'build_notices'):
                with self.assertRaisesRegex(ValueError, 'Forbidden package entry'):
                    package_release.assemble(target, archive)
            self.assertFalse(archive.exists())

    def test_rejects_frontend_symlink_before_copying_its_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target, archive = self.fixture(root)
            private = root / 'private.txt'
            private.write_text('fixture private content')
            (root / 'webapp/dist/innocent.txt').symlink_to(private)
            with patch.object(package_release, 'ROOT', root), \
                 patch.object(package_release.subprocess, 'check_output', return_value=b''), \
                 patch.object(package_release, 'build_notices'):
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    package_release.assemble(target, archive)
            self.assertFalse(archive.exists())


if __name__ == '__main__':
    unittest.main()
