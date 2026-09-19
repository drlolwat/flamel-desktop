#!/usr/bin/env python3
"""Create a lockfile inventory and retain available dependency license texts.

This deliberately includes build/dev and optional dependencies. Missing package
sources are reported, never silently treated as license clearance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import shutil
import tarfile
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version', '1'], cwd=ROOT))
    lock = json.loads((ROOT / 'package-lock.json').read_text())
    components, texts, gaps = {}, {}, []

    def record(ecosystem, name, version, license_name, directory, integrity=None):
        ref = f'pkg:{ecosystem}/{quote(name, safe="/")}@{version}'
        if ref in components:
            return
        component = {'type': 'library', 'bom-ref': ref, 'name': name,
                     'version': version, 'purl': ref}
        if license_name:
            # License expressions are retained verbatim; this is not a legal
            # compatibility decision or an assertion of SPDX validity.
            component['licenses'] = [{'license': {'name': license_name}}]
        if integrity:
            component['properties'] = [{'name': 'flamel:lock-integrity', 'value': integrity}]
        components[ref] = component
        if not directory.is_dir():
            archive = ROOT / 'target/dependency-source-cache/npm' / f'{name.replace("/", "-")}-{version}.tgz'
            retained = False
            if archive.is_file():
                with tarfile.open(archive, 'r:gz') as source:
                    for member in source.getmembers():
                        if member.isfile() and member.size < 2 * 1024 * 1024 and re.match(r'^(licen[cs]e|copying|notice|copyright)([._-]|$)', Path(member.name).name, re.I):
                            content = source.extractfile(member).read()
                            digest = hashlib.sha256(content).hexdigest()
                            texts.setdefault(digest, {'text': content.decode('utf-8', errors='replace'), 'packages': []})['packages'].append(ref)
                            retained = True
            if not retained:
                gaps.append({'package': ref, 'reason': 'optional platform source has no locally available separate notice; see dependency source archive'})
            return
        found = []
        for candidate in sorted(directory.iterdir()):
            if candidate.is_file() and re.match(r'^(licen[cs]e|copying|notice|copyright)([._-]|$)', candidate.name, re.I):
                found.append(candidate)
            elif candidate.is_dir() and candidate.name.lower() in {'licenses', 'licences'}:
                found.extend(p for p in sorted(candidate.rglob('*')) if p.is_file())
        if not found:
            # Some crates use an explicitly declared nonstandard license path.
            manifest = directory / 'Cargo.toml'
            if ecosystem == 'cargo' and manifest.exists():
                import tomllib
                custom = tomllib.loads(manifest.read_text()).get('package', {}).get('license-file')
                if custom:
                    candidate = (directory / custom).resolve()
                    if candidate.is_relative_to(directory.resolve()) and candidate.is_file():
                        found.append(candidate)
        if not found:
            gaps.append({'package': ref, 'reason': 'no separate license text in installed source', 'declaredLicense': license_name})
            # Preserve upstream attribution and license statements rather than
            # inventing copyright holders or silently omitting the package.
            found = [p for p in directory.iterdir() if p.is_file() and p.name.lower().startswith('readme')]
        for candidate in found:
            content = candidate.read_bytes()
            digest = hashlib.sha256(content).hexdigest()
            texts.setdefault(digest, {'text': content.decode('utf-8', errors='replace'), 'packages': []})['packages'].append(ref)

    for package in metadata['packages']:
        if package['id'] in metadata['workspace_members']:
            continue
        record('cargo', package['name'], package['version'], package.get('license'),
               Path(package['manifest_path']).parent)
    for path, package in lock['packages'].items():
        if 'node_modules/' not in path or package.get('link'):
            continue
        name = package.get('name') or path.rsplit('node_modules/', 1)[1]
        record('npm', name, package['version'], package.get('license'), ROOT / path, package.get('integrity'))
    bom = {'bomFormat': 'CycloneDX', 'specVersion': '1.5', 'version': 1,
           'metadata': {'component': {'type': 'application', 'name': 'flamel-desktop', 'version': '0.1.0'},
                        'properties': [{'name': 'flamel:scope', 'value': 'All resolved Rust and npm dependencies, including build/dev/optional; not a binary reachability claim'}]},
           'components': [components[key] for key in sorted(components)]}
    (args.output / 'dependencies.cdx.json').write_text(json.dumps(bom, indent=2) + '\n')
    (args.output / 'license-review.json').write_text(json.dumps({'components': len(components), 'licenseTexts': len(texts), 'gaps': gaps}, indent=2) + '\n')
    for name in ['LICENSE.md', 'NOTICE', 'COMMERCIAL.md', 'THIRD_PARTY_NOTICES.md']:
        shutil.copyfile(ROOT / name, args.output / name)
    with (args.output / 'DEPENDENCY_LICENSES.txt').open('w') as output:
        output.write('Third-party license texts from the locked dependency sources.\nThird-party terms are not replaced by the Flamel Community License.\n\n')
        for digest, item in sorted(texts.items()):
            output.write('\n' + '=' * 72 + '\n' + '\n'.join(sorted(set(item['packages']))) + '\nSHA-256: ' + digest + '\n\n' + item['text'] + '\n')
    print(f'Inventoried {len(components)} dependencies; retained {len(texts)} distinct notice texts; {len(gaps)} review entries.')


if __name__ == '__main__':
    main()
