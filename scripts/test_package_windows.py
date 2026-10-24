import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile
from package_windows import package


class PackageTests(unittest.TestCase):
    def test_allowlist_and_checksums(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            exe = root / "app.exe"
            exe.write_bytes(b"MZfixture-not-a-real-executable")
            (root / "session.json").write_text("must-not-ship")
            archive = root / "test.zip"
            package(exe, archive)
            with zipfile.ZipFile(archive) as z:
                self.assertEqual(set(z.namelist()), {"NekoSportsWorldTool.exe", "config.example.json", "更新说明.txt", "SHA256SUMS.txt"})
                self.assertEqual(json.loads(z.read("config.example.json"))["route_mode"], "road")
                for line in z.read("SHA256SUMS.txt").decode().splitlines():
                    digest, name = line.split("  ", 1)
                    self.assertEqual(hashlib.sha256(z.read(name)).hexdigest(), digest)
            self.assertEqual(archive.with_suffix(".sha256").read_text().split()[0], hashlib.sha256(archive.read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
