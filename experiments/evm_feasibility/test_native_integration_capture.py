"""Focused process-boundary regressions; all generated streams are retained."""
import gzip
import hashlib
import json
import os
from pathlib import Path
import sys
import unittest
import subprocess
import time

from native_integration_reference import capture_child, _capture_child_inner


class CaptureTests(unittest.TestCase):
    def capture(self, name, program, *, limit=8192, timeout=5):
        root = Path(os.environ['RUN_ARTIFACT_DIR']) / 'internal-capture-tests'
        root.mkdir(exist_ok=True)
        directory = root / name
        directory.mkdir()  # Never overwrite a previous occurrence.
        status = capture_child([sys.executable, '-c', program], directory, directory,
                               max_output_bytes=limit, timeout_seconds=timeout)
        self.assertEqual(status, json.loads((directory / 'capture.json').read_bytes()))
        raw = {}
        for name, stream in status['streams'].items():
            raw[name] = gzip.decompress((directory / stream['file']).read_bytes())
            self.assertEqual(len(raw[name]), stream['bytes'])
            self.assertEqual(hashlib.sha256(raw[name]).hexdigest(), stream['sha256'])
        self.assertLessEqual(sum(map(len, raw.values())), limit)
        return status, raw

    def test_exact_combined_boundary_lossless(self):
        status, raw = self.capture('boundary', 'import os;os.write(1,bytes(range(256))*16);os.write(2,b"e"*4096)')
        self.assertEqual(status['child_exit'], 0)
        self.assertTrue(status['capture_complete'])
        self.assertEqual(status['captured_bytes'], 8192)
        self.assertEqual(raw['stdout'], bytes(range(256))*16)
        self.assertEqual(raw['stderr'], b'e'*4096)

    def test_overflow_even_when_child_fails(self):
        status, _ = self.capture('overflow-failing', 'import os;os.write(1,b"a"*5000);os.write(2,b"b"*5000);raise SystemExit(7)')
        self.assertFalse(status['capture_complete'])
        self.assertEqual(status['outcome'], 'output_limit')
        self.assertGreater(status['observed_bytes'], 8192)
        self.assertEqual(status['captured_bytes'], 8192)
        self.assertIsNotNone(status['child_exit'])

    def test_live_overflow_cancels_and_reaps(self):
        status, _ = self.capture('overflow-live', 'import os,time;os.write(1,b"a"*9000);time.sleep(60)')
        self.assertEqual(status['outcome'], 'output_limit')
        self.assertEqual(status['child_exit'], -9)
        self.assertLess(status['elapsed_seconds'], 5)

    def test_nonzero_exit_complete_stderr(self):
        status, raw = self.capture('exit7', 'import os;os.write(2,b"actual stderr\\n");raise SystemExit(7)')
        self.assertTrue(status['capture_complete'])
        self.assertEqual(status['child_exit'], 7)
        self.assertEqual(raw['stderr'], b'actual stderr\n')

    def test_timeout_cancels_group_and_retains_prefix(self):
        status, raw = self.capture('timeout', 'import os,time;os.write(1,b"before timeout");time.sleep(60)', timeout=0.15)
        self.assertFalse(status['capture_complete'])
        self.assertEqual(status['outcome'], 'timeout')
        self.assertEqual(status['child_exit'], -9)
        self.assertEqual(raw['stdout'], b'before timeout')
        self.assertLess(status['elapsed_seconds'], 3)

    def test_timeout_after_child_closes_streams(self):
        status, _ = self.capture('closed-stream-timeout', 'import os,time;os.close(1);os.close(2);time.sleep(60)', timeout=0.15)
        self.assertFalse(status['capture_complete'])
        self.assertEqual(status['outcome'], 'timeout')
        self.assertEqual(status['child_exit'], -9)

    def test_prelaunch_closed_control_does_not_launch_child(self):
        directory = Path(os.environ['RUN_ARTIFACT_DIR']) / 'prelaunch'
        directory.mkdir()
        read, write = os.pipe()
        os.close(write)
        status = _capture_child_inner([sys.executable, '-c', 'raise SystemExit(99)'],
                                      directory, directory, timeout_seconds=1,
                                      max_output_bytes=1024, control_fd=read)
        self.assertFalse(status['launched'])
        self.assertFalse(status['capture_complete'])
        self.assertEqual(status['outcome'], 'launch_error')

    def test_successful_leader_cleans_closed_stream_descendant(self):
        root = Path(os.environ['RUN_ARTIFACT_DIR'])
        pid_file = root / 'descendant.pid'
        descendant = 'import os,time;os.close(1);os.close(2);time.sleep(60)'
        program = f'import subprocess,sys;child=subprocess.Popen([sys.executable,"-c",{descendant!r}]);open({str(pid_file)!r},"w").write(str(child.pid))'
        status, _ = self.capture('descendant', program)
        self.assertTrue(status['capture_complete'])
        self.assertEqual(status['child_exit'], 0)
        pid = int(pid_file.read_text())
        deadline = time.monotonic() + 3
        while Path('/proc', str(pid), 'stat').exists() and time.monotonic() < deadline:
            if Path('/proc', str(pid), 'stat').read_text().split()[2] == 'Z':
                break  # Orphan reaping belongs to the host init; it is no longer running.
            time.sleep(0.02)
        self.assertTrue(not Path('/proc', str(pid)).exists() or Path('/proc', str(pid), 'stat').read_text().split()[2] == 'Z')
        self.assertFalse(Path('/proc', str(status['supervisor_pid'])).exists())

    def outer_cancellation(self, name, output_limit):
        root = Path(os.environ['RUN_ARTIFACT_DIR']) / 'internal-capture-tests' / name
        root.mkdir(parents=True)
        inner = root / 'inner'
        inner.mkdir()
        pid_file = inner / 'go.pid'
        program = f'import os,time;open({str(pid_file)!r},"w").write(str(os.getpid()));os.write(1,b"started");time.sleep(60)'
        producer = f'import sys;sys.path.insert(0,{str(Path(__file__).parent.resolve())!r});from native_integration_reference import capture_child, _capture_child_inner;from pathlib import Path;'
        if output_limit:
            overflow_code = "def overflow():\n while not pid.exists():time.sleep(0.01)\n os.write(1,b'x'*9000)\n"
            producer += f'import threading,time,os;pid=Path({str(pid_file)!r});exec({overflow_code!r});threading.Thread(target=overflow).start();'
        producer += f'capture_child([sys.executable,"-c",{program!r}],Path({str(inner)!r}),Path({str(inner)!r}),timeout_seconds=20,max_output_bytes=8192)'
        command = [sys.executable, str(Path(__file__).resolve().parents[2] / 'scripts/capture_command.py'),
                   '--output-dir', str(root / 'outer'), '--timeout-seconds', '5' if output_limit else '2',
                   '--max-output-bytes', '1024', '--', sys.executable, '-c', producer]
        result = subprocess.run(command, capture_output=True, timeout=10)
        outer = json.loads((root / 'outer/status.json').read_bytes())
        self.assertFalse(outer['capture_complete'])
        self.assertEqual(outer['outcome'], 'output_limit' if output_limit else 'timeout')
        self.assertNotEqual(result.returncode, 0)
        deadline = time.monotonic() + 5
        while not (inner / 'capture.json').exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        status = json.loads((inner / 'capture.json').read_bytes())
        self.assertFalse(status['capture_complete'])
        self.assertEqual(status['outcome'], 'cancelled')
        self.assertEqual(status['child_exit'], -9)
        self.assertEqual(gzip.decompress((inner / 'stdout.gz').read_bytes()), b'started')
        self.assertFalse(Path('/proc', str(status['child_pid'])).exists(), 'direct Go child was reaped')
        while Path('/proc', str(status['supervisor_pid'])).exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        supervisor_stat = Path('/proc', str(status['supervisor_pid']), 'stat')
        self.assertTrue(not supervisor_stat.exists() or supervisor_stat.read_text().split()[2] == 'Z')

    def test_outer_sigkill_timeout_cancels_and_reaps_go(self):
        self.outer_cancellation('outer-timeout', False)

    def test_outer_sigkill_output_limit_cancels_and_reaps_go(self):
        self.outer_cancellation('outer-output', True)


if __name__ == '__main__':
    unittest.main()
