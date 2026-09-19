"""Exercise the exported source boundary without a Git checkout."""
from pathlib import Path
import json
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class SourceBoundary(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='flamel-export-test-')
        self.root = Path(self.temporary.name)
        for name in json.loads((ROOT / 'scripts/source-allowlist.json').read_text())['files']:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)

    def tearDown(self):
        self.temporary.cleanup()

    def audit(self):
        return subprocess.run([sys.executable, str(self.root / 'scripts/audit-source.py')],
                              cwd=self.root, capture_output=True, text=True)

    def test_archive_without_git_passes(self):
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_extra_private_source_fails(self):
        (self.root / 'server').mkdir()
        (self.root / 'server/private.rs').write_text('// Must never enter the export\n')
        self.assertNotEqual(self.audit().returncode, 0)

    def test_source_path_escape_fails(self):
        with (self.root / 'engine/Cargo.toml').open('a') as output:
            output.write('\n[dependencies.private-source]\npath = "../../private"\n')
        self.assertNotEqual(self.audit().returncode, 0)

    def test_unlisted_private_flame_metadata_fails(self):
        (self.root / '.flamel/private.json').write_text('{}')
        self.assertNotEqual(self.audit().returncode, 0)

    def test_private_network_address_fails(self):
        address = '.'.join(['192', '168', '42', '23'])
        with (self.root / 'README.md').open('a') as output:
            output.write('\nInternal endpoint: http://' + address + ':8181\n')
        result = self.audit()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('private network address', result.stdout)


if __name__ == '__main__':
    unittest.main()
