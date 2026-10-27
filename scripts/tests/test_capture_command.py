"""Regression tests for actual process evidence and bounded failure capture."""

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location(
    'capture_command', Path(__file__).resolve().parents[1] / 'capture_command.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.output = Path(self.temp.name) / 'capture'

    def run_python(self, source, **kwargs):
        return runner.capture(self.output, [sys.executable, '-c', source],
                              timeout_seconds=kwargs.get('timeout', 3),
                              max_output_bytes=kwargs.get('limit', 65536))

    def test_nonzero_preserves_distinct_binary_streams_and_actual_exit(self):
        code, status = self.run_python(
            "import os;os.write(1,b'out\\x00');os.write(2,b'err\\xff');raise SystemExit(7)")
        self.assertEqual(code, 7)
        self.assertEqual((self.output / 'stdout').read_bytes(), b'out\0')
        self.assertEqual((self.output / 'stderr').read_bytes(), b'err\xff')
        self.assertEqual((self.output / 'exit').read_text(), '7\n')
        self.assertTrue(status['capture_complete'])
        self.assertEqual(json.loads((self.output / 'status.json').read_text()), status)

    def test_spawn_failure_never_claims_child_exit_or_complete_capture(self):
        code, status = runner.capture(self.output, ['/nonexistent/capture-command'],
                                      timeout_seconds=3, max_output_bytes=1000)
        self.assertEqual(code, 127)
        self.assertIsNone(status['child_exit'])
        self.assertFalse(status['capture_complete'])
        self.assertFalse((self.output / 'exit').exists())
        self.assertIn(b'FileNotFoundError', (self.output / 'runner.stderr').read_bytes())

    def test_timeout_retains_output_and_signal_exit(self):
        code, status = self.run_python(
            "import os,time;os.write(2,b'before timeout');time.sleep(5)", timeout=0.2)
        self.assertEqual(code, 124)
        self.assertEqual(status['outcome'], 'timeout')
        self.assertLess(status['child_exit'], 0)
        self.assertFalse(status['capture_complete'])
        self.assertEqual((self.output / 'stderr').read_bytes(), b'before timeout')

    def test_output_overflow_retains_bounded_prefix_never_success(self):
        code, status = self.run_python("import os;os.write(2,b'x'*4096)", limit=100)
        self.assertEqual(code, 125)
        self.assertEqual(status['outcome'], 'output_limit')
        self.assertFalse(status['capture_complete'])
        self.assertEqual((self.output / 'stderr').read_bytes(), b'x' * 100)

    def test_exact_output_limit_is_complete(self):
        code, status = self.run_python("import os;os.write(1,b'x'*100)", limit=100)
        self.assertEqual(code, 0)
        self.assertTrue(status['capture_complete'])

    def test_reuse_refused_before_child_side_effect(self):
        self.run_python("print('original')")
        marker = Path(self.temp.name) / 'must-not-exist'
        with self.assertRaises(FileExistsError):
            self.run_python(f"from pathlib import Path;Path({str(marker)!r}).touch()")
        self.assertFalse(marker.exists())
        self.assertEqual((self.output / 'stdout').read_bytes(), b'original\n')

    def test_closed_streams_do_not_hide_running_child(self):
        code, status = self.run_python(
            "import os,time;os.close(1);os.close(2);time.sleep(5)", timeout=0.2)
        self.assertEqual(code, 124)
        self.assertFalse(status['capture_complete'])

    def test_descendant_inheriting_streams_cannot_escape_deadline(self):
        code, status = self.run_python(
            "import os,time;pid=os.fork();time.sleep(5) if pid==0 else None", timeout=0.2)
        self.assertEqual(code, 124)
        self.assertFalse(status['capture_complete'])

    def test_wait_return_after_deadline_never_claims_complete(self):
        real_clock = runner.time.monotonic
        real_wait = runner.subprocess.Popen.wait
        waited = False

        def clock():
            return real_clock() + (100 if waited else 0)

        def late_wait(process, *args, **kwargs):
            nonlocal waited
            code = real_wait(process, *args, **kwargs)
            waited = True
            return code

        with mock.patch.object(runner.time, 'monotonic', side_effect=clock), \
                mock.patch.object(runner.subprocess.Popen, 'wait', new=late_wait):
            code, status = self.run_python("print('done')")
        self.assertEqual(code, 124)
        self.assertEqual(status['child_exit'], 0)
        self.assertFalse(status['capture_complete'])

    def test_explicit_retained_stdin_is_delivered_without_shell_expansion(self):
        source = Path(self.temp.name) / 'input'
        raw = b"literal `command` $value\x00\xff\n"
        source.write_bytes(raw)
        code, status = runner.capture(
            self.output, [sys.executable, '-c',
                          'import sys;sys.stdout.buffer.write(sys.stdin.buffer.read())'],
            timeout_seconds=3, max_output_bytes=1000, stdin_file=source)
        self.assertEqual(code, 0)
        self.assertEqual((self.output / 'stdout').read_bytes(), raw)
        self.assertTrue(status['capture_complete'])
        self.assertEqual(status['stdin_file'], str(source))

    def test_missing_stdin_file_fails_before_child_and_records_real_error(self):
        code, status = runner.capture(
            self.output, [sys.executable, '-c', "print('must not run')"],
            timeout_seconds=3, max_output_bytes=1000,
            stdin_file=Path(self.temp.name) / 'missing')
        self.assertEqual(code, 125)
        self.assertFalse(status['launched'])
        self.assertFalse(status['capture_complete'])
        self.assertFalse((self.output / 'exit').exists())
        self.assertIn(b'FileNotFoundError', (self.output / 'runner.stderr').read_bytes())


if __name__ == '__main__':
    unittest.main()
