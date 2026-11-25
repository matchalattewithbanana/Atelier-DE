#!/usr/bin/env python3
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-cursors.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class ArchiveTests(unittest.TestCase):
    def extract(self, members, compression="gz"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "asset"
            with tarfile.open(archive, "w:" + compression) as tar:
                for path, value in members:
                    entry = tarfile.TarInfo(path)
                    if isinstance(value, bytes):
                        entry.size = len(value)
                        tar.addfile(entry, io.BytesIO(value))
                    else:
                        entry.type = tarfile.SYMTYPE
                        entry.linkname = value
                        tar.addfile(entry)
            stage = root / "stage"
            stage.mkdir()
            installer.extract_theme(archive, compression, "Example", stage)
            return {path.name: path.read_bytes() for path in (stage / "cursors").iterdir()}

    def test_theme_and_alias_chain(self):
        for compression in ("gz", "xz"):
            self.assertEqual(self.extract([
                ("Example/cursors/left_ptr", b"Xcurpixels"),
                ("Example/cursors/default", "alias"),
                ("Example/cursors/alias", "left_ptr"),
                ("Example/index.theme", b"[Icon Theme]\nName=Example\n"),
            ], compression), {"left_ptr": b"Xcurpixels", "default": b"Xcurpixels", "alias": b"Xcurpixels"})

    def test_reject_unsafe_members(self):
        for member in [
            ("../escape", b"Xcur"),
            ("Example/cursors/default", "../../escape"),
            ("Example/cursors/default", "missing"),
            ("Example/cursors/default", "default"),
            ("Example/cursors/default", b"ELF!"),
            ("Example/executable", b"script"),
        ]:
            with self.subTest(member=member), self.assertRaises(ValueError):
                self.extract([("Example/cursors/left_ptr", b"Xcur"), member])

    def test_duplicate_and_missing_default(self):
        with self.assertRaises(ValueError):
            self.extract([("cursors/left_ptr", b"Xcur"), ("cursors/left_ptr", b"Xcur")])
        with self.assertRaises(ValueError):
            self.extract([("cursors/text", b"Xcur")])

    def test_catalog(self):
        self.assertEqual(len(installer.ASSETS), 16)
        self.assertIn("Bibata-Modern-Ice", installer.ASSETS)
        self.assertIn("GoogleDot-Blue", installer.ASSETS)


if __name__ == "__main__":
    unittest.main()
