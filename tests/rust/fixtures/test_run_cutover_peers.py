"""Check the peer runner's output path before it opens any fixture inputs."""
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest


class OutputPathTest(unittest.TestCase):
    def test_rejects_retained_output_before_creating_files(self):
        runner = Path(__file__).with_name("run_cutover_peers.py")
        with TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = root / "evidence"
            first_db = evidence / "resume39-source111-schedule-standard-repair01-a/scratch/data/db"
            first_db.mkdir(parents=True)
            second_db = root / "second-db"
            second_db.mkdir()
            second_seed = evidence / "resume39-source116-app-offline01-evidence/scratch/native-publication-9/data"
            second_seed.mkdir(parents=True)
            (second_seed / "db").symlink_to(second_db, target_is_directory=True)
            alias = root / "alias"
            alias.symlink_to(first_db, target_is_directory=True)

            for parent in (evidence, first_db, second_db, alias):
                with self.subTest(parent=parent):
                    output = parent / "new-parent" / "output"
                    result = subprocess.run(
                        [sys.executable, str(runner), sys.executable, str(evidence), str(output)],
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn("output must be outside retained evidence and stores", result.stderr)
                    self.assertFalse(output.parent.exists())


if __name__ == "__main__":
    unittest.main()
