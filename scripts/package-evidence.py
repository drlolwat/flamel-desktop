#!/usr/bin/env python3
"""Gather only known Desktop installers plus source provenance and notices."""
from pathlib import Path
import argparse
import hashlib
import json
import platform
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
parser.add_argument('--target', help='Cargo target triple for a cross-built installer')
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=True)
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=root))
target = Path(metadata['target_directory'])
if args.target:
    target /= args.target
target = target / 'release' / 'bundle'
package_platform = 'Windows' if args.target and 'windows' in args.target else platform.system()
installers = sorted(target.glob('deb/*.deb')) if package_platform == 'Linux' else sorted(target.glob('nsis/*.exe'))
if len(installers) != 1:
    raise SystemExit('Expected exactly one current platform installer; clean stale bundles before packaging')
for installer in installers:
    shutil.copyfile(installer, output / installer.name)
for name in ['LICENSE.md', 'NOTICE', 'COMMERCIAL.md', 'THIRD_PARTY_NOTICES.md', 'DEPENDENCY_REVIEW.md', 'RELEASE_GATES.md']:
    shutil.copyfile(root / name, output / name)
for source in (root / 'desktop/resources/notices').glob('*'):
    if source.is_file():
        shutil.copyfile(source, output / source.name)
allowlist = json.loads((root / 'scripts/source-allowlist.json').read_text())['files']
source_hashes = {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in sorted(allowlist)}
manifest = {'schema': 1, 'product': 'flamel-desktop', 'signed': False,
            'publicationApproved': False, 'platform': package_platform,
            'buildHost': platform.system(), 'target': args.target,
            'sourceFiles': source_hashes, 'tools': {},
            'artifacts': {}}
for command in [['rustc', '--version'], ['cargo', '--version'], ['node', '--version'], ['npm', '--version']]:
    manifest['tools'][command[0]] = subprocess.check_output(command, cwd=root, text=True, shell=platform.system() == 'Windows' and command[0] == 'npm').strip()
for path in sorted(output.iterdir()):
    if path.is_file() and path.name not in {'provenance.json', 'SHA256SUMS'}:
        manifest['artifacts'][path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
(output / 'provenance.json').write_text(json.dumps(manifest, indent=2) + '\n')
with (output / 'SHA256SUMS').open('w') as checksums:
    for path in sorted(output.iterdir()):
        if path.is_file() and path.name != 'SHA256SUMS':
            checksums.write(hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + path.name + '\n')
print('Packaged unsigned Desktop candidate with source hashes, notices and checksums.')
