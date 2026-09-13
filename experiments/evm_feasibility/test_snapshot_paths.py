"""Bounded preservation checks; no supplied snapshot data is opened."""

import pathlib
import subprocess
import sys
import tempfile
import unittest

from snapshot_manifest import manifest, validate_outputs


class SnapshotPaths(unittest.TestCase):
    def test_outputs_preserve_both_inputs_and_aliases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source, copy = root / "source", root / "copy"
            source.mkdir()
            copy.mkdir()
            current = source / "CURRENT"
            current.write_bytes(b"unchanged")
            (copy / "CURRENT").write_bytes(b"unchanged")
            alias = root / "alias"
            alias.symlink_to(source, target_is_directory=True)
            hardlink = root / "hardlink"
            hardlink.hardlink_to(current)
            for output in [current, copy / "new", alias / "new", hardlink]:
                with self.assertRaises((ValueError, FileExistsError)):
                    validate_outputs([source, copy], [output])
            report = root / "report"
            with self.assertRaises(ValueError):
                validate_outputs([source, copy], [report, report])
            first = manifest(source, report)
            second = manifest(copy, root / "report2")
            self.assertEqual(first, second)
            with self.assertRaises(FileExistsError):
                manifest(source, report)
            self.assertEqual(current.read_bytes(), b"unchanged")

    def test_missing_paired_argument_writes_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            report = root / "report"
            result = subprocess.run(
                [sys.executable, str(pathlib.Path(__file__).with_name("snapshot_manifest.py")),
                 str(root), str(report), "--compare", str(root)],
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(report.exists())


if __name__ == "__main__":
    unittest.main()
