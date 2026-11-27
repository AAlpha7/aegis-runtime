#!/usr/bin/env python3
"""Build distributable host/Wasm binaries without builder home/workspace paths."""
import os
import json
import hashlib
import platform
from pathlib import Path
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        raise ValueError('Distribution builds currently support Linux x86_64 only')
    environment = dict(os.environ)
    # The offline notice collector resolves the complete lockfile before
    # selecting Linux/Wasm dependencies. A native build alone leaves other
    # platform manifests absent on a fresh Cargo home. Prepare that graph here
    # rather than relying on unrelated earlier builds to populate the cache.
    print('Preparing locked Cargo sources for offline notice collection.', flush=True)
    subprocess.run(['cargo', 'fetch', '--locked'], cwd=ROOT, env=environment, check=True)
    mappings = [(str(Path.home()), '/build/home'), (str(ROOT), '/build/aegis-runtime')]
    rust_maps = [f'--remap-path-prefix={source}={destination}' for source, destination in mappings]
    # CARGO_ENCODED_RUSTFLAGS takes precedence; preserve caller flags in either form.
    existing = environment.get('CARGO_ENCODED_RUSTFLAGS')
    flags = existing.split('\x1f') if existing is not None else shlex.split(environment.get('RUSTFLAGS', ''))
    environment['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags + rust_maps)
    environment['SKIP_SKILL_BUILD'] = '1'
    # OpenSSL embeds CFLAGS in its version string. Put the remapping arguments in
    # a PATH-resolved shim so the private source prefixes do not reappear there.
    with tempfile.TemporaryDirectory(prefix='aegis-release-compiler-', dir='/tmp') as directory:
        shim_source = (ROOT / 'scripts/release_compiler.py').read_bytes().replace(b'\r\n', b'\n')
        identity = json.dumps([mappings, environment.get('CC', 'cc'), environment.get('CXX', 'c++')]).encode()
        digest = hashlib.sha256(shim_source + identity).hexdigest()[:16]
        for name, original in [('cc', 'CC'), ('cxx', 'CXX')]:
            variable = 'AEGIS_RELEASE_' + original
            environment[variable] = environment.get(original, 'cc' if name == 'cc' else 'c++')
            shim = Path(directory) / ('aegis-release-' + digest + '-' + name)
            shim.write_bytes(shim_source)
            shim.chmod(0o700)
            environment[original] = shim.name
        environment['AEGIS_RELEASE_PATH_MAPS'] = json.dumps(mappings)
        environment['PATH'] = directory + os.pathsep + environment['PATH']
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'aegis-runtime-core', '-p', 'aegis-executor'],
                       cwd=ROOT, env=environment, check=True)
        # Never propagate a host linker flag into Wasm; keep just the path mapping.
        environment['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(rust_maps)
        subprocess.run(['cargo', 'build', '--locked', '--release', '--target', 'wasm32-wasip1',
                        '-p', 'web-reader', '-p', 'test-no-net'], cwd=ROOT, env=environment, check=True)


if __name__ == '__main__':
    main()
