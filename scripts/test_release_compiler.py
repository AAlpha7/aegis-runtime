#!/usr/bin/env python3
"""Compile a real C __FILE__ probe through the release-path shim."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import build_release


class CompilerPathTest(unittest.TestCase):
    def setUp(self):
        # cargo is mocked in the ordering tests, so they are host independent;
        # pin the platform so they also run on aarch64 dev hosts. The guard
        # itself is covered by test_rejects_non_x86_64_host.
        for name, value in (('system', 'Linux'), ('machine', 'x86_64')):
            patcher = patch.object(build_release.platform, name, return_value=value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def test_rejects_non_x86_64_host(self):
        with patch.object(build_release.platform, 'machine', return_value='aarch64'), \
             patch.object(build_release.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'Linux x86_64 only'):
                build_release.main()
            run.assert_not_called()

    def test_cold_registry_is_prepared_before_compilation(self):
        prepared = False
        builds = []

        def cargo_run(argv, **kwargs):
            nonlocal prepared
            self.assertTrue(kwargs['check'])
            if argv[:2] == ['cargo', 'fetch']:
                self.assertIn('--locked', argv)
                self.assertNotIn('--target', argv)  # metadata resolves all platforms
                prepared = True
            else:
                self.assertTrue(prepared, 'cold registry must be ready before compiling')
                builds.append(argv)

        with patch.object(build_release.subprocess, 'run', side_effect=cargo_run):
            build_release.main()
        self.assertTrue(any('--target' not in argv for argv in builds))
        self.assertTrue(any('wasm32-wasip1' in argv for argv in builds))

    def test_failed_dependency_preparation_stops_before_compiling(self):
        def fail_fetch(argv, **kwargs):
            if argv[:2] != ['cargo', 'fetch']:
                self.fail('compilation started despite unavailable locked sources')
            raise subprocess.CalledProcessError(101, argv)

        with patch.object(build_release.subprocess, 'run', side_effect=fail_fetch):
            with self.assertRaises(subprocess.CalledProcessError):
                build_release.main()

    def test_native_file_macro_uses_public_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'private-build-path.c'
            source.write_text('#include <stdio.h>\nint main(void) { puts(__FILE__); puts(ENGINESDIR); return 0; }\n')
            shim = root / 'aegis-release-cc'
            shim.write_bytes(Path(__file__).with_name('release_compiler.py').read_bytes().replace(b'\r\n', b'\n'))
            shim.chmod(0o700)
            environment = dict(os.environ, AEGIS_RELEASE_CC=shutil.which('cc') or 'cc',
                               AEGIS_RELEASE_PATH_MAPS=json.dumps([[str(root), '/build/project']]))
            output = root / 'probe'
            subprocess.run([str(shim), str(source), f'-DENGINESDIR="{root}/openssl/engines"', '-o', str(output)], env=environment, check=True, capture_output=True)
            value = subprocess.check_output([str(output)], text=True).strip()
            self.assertEqual(value, '/build/project/private-build-path.c\n/build/project/openssl/engines')
            self.assertNotIn(str(root).encode(), output.read_bytes())


if __name__ == '__main__':
    unittest.main()
