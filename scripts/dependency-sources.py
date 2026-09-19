#!/usr/bin/env python3
"""Archive exact registry sources, verified against the public lockfiles.

No package scripts run. This preserves third-party source and notices alongside
binary distributions, including file-copyleft dependencies and optional targets.
"""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tomllib
from urllib.parse import urlparse
from urllib.request import urlopen

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
cache = root / 'target/dependency-source-cache'
cache.mkdir(parents=True, exist_ok=True)
packages = []
for package in tomllib.loads((root / 'Cargo.lock').read_text())['package']:
    if package.get('source', '').startswith('registry+'):
        name, version = package['name'], package['version']
        packages.append((f'cargo/{name}-{version}.crate', f'https://static.crates.io/crates/{name}/{name}-{version}.crate', 'sha256', package['checksum'], False))
for path, package in json.loads((root / 'package-lock.json').read_text())['packages'].items():
    if 'node_modules/' not in path or package.get('link'):
        continue
    url, integrity = package.get('resolved'), package.get('integrity')
    if not url or not integrity or urlparse(url).scheme != 'https' or urlparse(url).hostname != 'registry.npmjs.org':
        raise SystemExit(f'Non-registry dependency needs explicit review: {path}')
    algorithm, digest = integrity.split()[0].split('-', 1)
    name = package.get('name') or path.rsplit('node_modules/', 1)[1]
    packages.append((f'npm/{name.replace("/", "-")}-{package["version"]}.tgz', url, algorithm, digest, True))
packages = sorted(set(packages))


def obtain(package):
    name, url, algorithm, digest, encoded = package
    destination = cache / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    data = destination.read_bytes() if destination.exists() else None
    def valid(content):
        calculated = hashlib.new(algorithm, content).digest()
        return (base64.b64encode(calculated).decode() if encoded else calculated.hex()) == digest
    if data is None or not valid(data):
        with urlopen(url, timeout=60) as response:
            data = response.read(100 * 1024 * 1024 + 1)
        if len(data) > 100 * 1024 * 1024 or not valid(data):
            raise ValueError(f'Failed size/integrity check: {name}')
        destination.write_bytes(data)
    return name, hashlib.sha256(data).hexdigest()


with ThreadPoolExecutor(max_workers=6) as executor:
    hashes = dict(executor.map(obtain, packages))
manifest = {'scope': 'Exact public Cargo/npm registry sources, including build/dev and optional dependencies', 'files': hashes}
with tarfile.open(args.output / 'dependency-sources.tar', 'w') as archive:
    for name in sorted(hashes):
        info = archive.gettarinfo(str(cache / name), arcname=name)
        info.uid = info.gid = 0
        info.uname = info.gname = ''
        info.mtime = 0
        with (cache / name).open('rb') as source:
            archive.addfile(info, source)
    content = (json.dumps(manifest, indent=2) + '\n').encode()
    info = tarfile.TarInfo('manifest.json')
    info.size = len(content)
    archive.addfile(info, io.BytesIO(content))
(args.output / 'dependency-source-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(f'Archived and verified {len(hashes)} dependency source packages.')
