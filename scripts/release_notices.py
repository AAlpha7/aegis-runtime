#!/usr/bin/env python3
"""Collect locked Linux/Wasm build notices offline; fail on missing evidence."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
LICENSE_NAME = re.compile(r'^(licen[cs]e|copying|notice|copyright|authors)(?:[._-]|$)', re.I)


def cargo(*args):
    return subprocess.check_output(['cargo', *args], cwd=ROOT, text=True)


def build_notices(output):
    metadata = json.loads(cargo('metadata', '--locked', '--offline', '--format-version', '1'))
    selected = set()
    for target, packages in [
        ('x86_64-unknown-linux-gnu', ['aegis-runtime-core', 'aegis-executor']),
        ('wasm32-wasip1', ['aegis-runtime-wasm-core', 'web-reader', 'test-no-net']),
    ]:
        args = ['tree', '--locked', '--offline', '--prefix', 'none', '--format', '{p}',
                '--edges', 'normal,build', '--target', target]
        for package in packages:
            args += ['-p', package]
        selected.update(re.findall(r'^(\S+) v(\S+)', cargo(*args), re.M))
    supplements = ROOT / 'licenses/supplemental'
    manifest = json.loads((supplements / 'manifest.json').read_text())
    inventory = []
    sections = ['Aegis Runtime third-party notices\n\n'
                'Conservative inventory of locked Linux and Wasm normal/build dependencies.\n'
                'Build dependencies may not be present in the executable. Platform-conditional\n'
                'dependencies for other targets and development-only tools are excluded.\n'
                'Components retain their own licenses; the project MIT license does not\n'
                'replace them. Native nested LICENSE/NOTICE files are included below.\n']
    source_dir = output / 'third-party-sources'
    source_dir.mkdir()
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    checksums = {(p['name'], p['version']): p.get('checksum') for p in lock['package']}
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        name, version = package['name'], package['version']
        if not package['source'] or (name, version) not in selected:
            continue
        if not package['source'].startswith('registry+'):
            raise ValueError(f'Unreviewed dependency source: {name}')
        base = Path(package['manifest_path']).parent
        files = sorted(p for p in base.rglob('*') if p.is_file() and LICENSE_NAME.match(p.name))
        key = f'{name}@{version}'
        supplement = manifest.get(key, {})
        if not files and not supplement:
            raise ValueError(f'Missing license text: {key}')
        source = f'https://crates.io/api/v1/crates/{name}/{version}/download'
        entry = {'ecosystem': 'cargo', 'name': name, 'version': version,
                 'license': package['license'], 'source': source,
                 'source_sha256': checksums[name, version], 'notice_files': []}
        sections.append(f'\n{"=" * 72}\n{key}\nDeclared license: {package["license"]}\n'
                        f'Source: {source}\nAuthors: {", ".join(package.get("authors", []))}\n')
        for file in files:
            relative = file.relative_to(base).as_posix()
            sections.append(f'\n--- {relative} ---\n' + file.read_text(errors='replace') + '\n')
            entry['notice_files'].append(relative)
        for item in supplement.get('files', []):
            file = supplements / item['file']
            assert file.parent == supplements and file.suffix == '.txt'
            data = file.read_bytes()
            assert hashlib.sha256(data).hexdigest() == item['sha256'], f'Changed supplement: {key}'
            sections.append(f'\n--- Supplement: {item["source"]} ---\n'
                            + data.decode('utf-8') + '\n')
            entry['notice_files'].append(item['source'])
        if supplement.get('note'):
            sections.append('\nProvenance: ' + supplement['note'] + '\n')
        # Make the exact original source conveniently available for MPL components,
        # the BSD-selected ITT crates, and standard-text supplements lacking notices.
        if 'MPL-' in (package['license'] or '') or name.startswith('ittapi') or 'Standard license text' in supplement.get('note', ''):
            archive = base.parent.parent.parent / 'cache' / base.parent.name / f'{name}-{version}.crate'
            assert archive.is_file(), f'Missing locked source archive: {key}'
            assert hashlib.sha256(archive.read_bytes()).hexdigest() == checksums[name, version]
            shutil.copyfile(archive, source_dir / archive.name)
            entry['bundled_source'] = 'third-party-sources/' + archive.name
        inventory.append(entry)
    lock = json.loads((ROOT / 'webapp/package-lock.json').read_text())
    for path, package in sorted(lock['packages'].items()):
        if not path or package.get('dev'):
            continue
        base = ROOT / 'webapp' / path
        installed = json.loads((base / 'package.json').read_text())
        assert installed['version'] == package['version'], 'npm ci required before packaging'
        files = sorted(p for p in base.iterdir() if p.is_file() and LICENSE_NAME.match(p.name))
        assert files, f'Missing npm notice: {path}'
        name = installed['name']
        sections.append(f'\n{"=" * 72}\n{name}@{package["version"]}\n'
                        f'Declared license: {package.get("license")}\nSource: {package["resolved"]}\n')
        for file in files:
            sections.append(f'\n--- {file.name} ---\n' + file.read_text() + '\n')
        inventory.append({'ecosystem': 'npm', 'name': name, 'version': package['version'],
                          'license': package.get('license'), 'source': package['resolved'],
                          'integrity': package['integrity'], 'notice_files': [p.name for p in files]})
    toolchain = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    toolchain_docs = output / 'rust-toolchain-notices'
    toolchain_docs.mkdir()
    for name in ['COPYRIGHT.html', 'COPYRIGHT-library.html']:
        shutil.copyfile(toolchain / 'share/doc/rust' / name, toolchain_docs / name)
    (toolchain_docs / 'version.txt').write_text(subprocess.check_output(['rustc', '--version'], text=True))
    (output / 'THIRD_PARTY_NOTICES.txt').write_text('\n'.join(sections))
    (output / 'dependency-inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
    print(f'Collected {len(inventory)} dependency notices; exact selected sources bundled.')


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    build_notices(args.output)
