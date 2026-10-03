"""Exercise quota freshness and threshold decisions using disposable logs."""

from datetime import datetime, timedelta, timezone
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('codex_quota', Path(__file__).parents[1] / 'codex_quota.py')
QUOTA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QUOTA)


class QuotaTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime(2026, 10, 1, 18, 0, tzinfo=timezone.utc)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'rollout.jsonl'

    def evaluate(self, used=19, age=1, reset=None, window=10080, extra='', identity='current',
                 min_remaining_percent=QUOTA.MIN_REMAINING_PERCENT):
        rows = [
            {'type': 'session_meta', 'payload': {'id': identity}},
            {'type': 'turn_context', 'payload': {'model': 'gpt-6.1-sol', 'effort': 'medium'}},
            {'type': 'event_msg', 'timestamp': (self.now - timedelta(seconds=age)).isoformat(),
             'payload': {'type': 'token_count', 'rate_limits': {'primary': {
                 'used_percent': used, 'window_minutes': window,
                 'resets_at': reset if reset is not None else (self.now + timedelta(days=1)).timestamp()}}}},
        ]
        self.path.write_text(''.join(json.dumps(r) + '\n' for r in rows) + extra)
        return QUOTA.inspect_log(self.path, now=self.now, expected_session='current',
                                 min_remaining_percent=min_remaining_percent)

    def test_default_80_percent_remaining_threshold_and_observed_model(self):
        for used, code in ((0, 0), (19, 0), (19.5, 0), (20, 2), (20.5, 2), (50, 2), (100, 2)):
            with self.subTest(used=used):
                result, actual = self.evaluate(used)
                self.assertEqual(actual, code)
                self.assertEqual(result['remaining_percent'], 100 - used)
                self.assertEqual(result['model'], 'gpt-6.1-sol')
                self.assertEqual(result['floor_remaining_percent'], 80)
        self.assertIn('More than 80% remains', self.evaluate(used=19)[0]['reason'])
        self.assertIn('At most 80% remains', self.evaluate(used=20)[0]['reason'])

    def test_explicit_20_percent_remaining_override(self):
        for used, expected in ((50, 0), (79.5, 0), (80, 2), (85, 2)):
            with self.subTest(used=used):
                result, code = self.evaluate(used=used, min_remaining_percent=20)
                self.assertEqual(code, expected)
                self.assertEqual(result['floor_remaining_percent'], 20)
                self.assertIn('20% remains', result['reason'])

    def test_invalid_floor_values_are_rejected_at_function_boundary(self):
        for floor in (True, False, -1, 101, '80', float('nan'), float('inf'), float('-inf')):
            with self.subTest(floor=floor):
                result, code = self.evaluate(min_remaining_percent=floor)
                self.assertEqual(code, 3)
                self.assertEqual(result['status'], 'unknown')
                self.assertIsNone(result['floor_remaining_percent'])
                self.assertIn('finite number from 0 to 100', result['reason'])

    def test_stale_future_expired_and_wrong_window_are_unknown(self):
        for params in ({'age': 301}, {'age': -3}, {'reset': self.now.timestamp()}, {'window': 300}):
            with self.subTest(params=params):
                self.assertEqual(self.evaluate(**params)[1], 3)

    def test_invalid_percentage_and_identity_are_unknown(self):
        for value in (True, -1, 101, '70', float('nan')):
            with self.subTest(value=value):
                self.assertEqual(self.evaluate(value)[1], 3)
        self.assertEqual(self.evaluate(identity='other')[1], 3)

    def test_partial_record_missing_snapshot_and_missing_file(self):
        self.assertEqual(self.evaluate(extra='{"unfinished":')[1], 3)
        self.path.write_text(json.dumps({'type': 'session_meta', 'payload': {'id': 'current'}}) + '\n')
        self.assertEqual(QUOTA.inspect_log(self.path, now=self.now)[1], 3)
        self.path.unlink()
        self.assertEqual(QUOTA.inspect_log(self.path, now=self.now)[1], 3)

    def test_uses_latest_snapshot_and_refuses_stale_completed_session(self):
        self.evaluate(used=19)
        with self.path.open('a') as stream:
            stream.write(json.dumps({'type': 'event_msg', 'timestamp': self.now.isoformat(),
                                    'payload': {'type': 'token_count', 'rate_limits': {'primary': {
                                        'used_percent': 20, 'window_minutes': 10080,
                                        'resets_at': (self.now + timedelta(days=1)).timestamp()}}}}) + '\n')
        self.assertEqual(QUOTA.inspect_log(self.path, now=self.now)[1], 2)
        self.assertEqual(QUOTA.inspect_log(self.path, now=self.now + timedelta(minutes=6))[1], 3)

    def test_cli_refuses_session_override_of_current_thread(self):
        with patch.dict('os.environ', {'CODEX_THREAD_ID': '00000000-0000-0000-0000-000000000002'}), \
                patch('sys.argv', ['codex_quota.py', '--session', '00000000-0000-0000-0000-000000000001']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 3)
        result = json.loads(output.call_args.args[0])
        self.assertEqual(result['status'], 'unknown')
        self.assertEqual(result['reason'], 'Requested session does not match CODEX_THREAD_ID.')
        self.assertEqual(result['floor_remaining_percent'], 80.0)

    def test_cli_accepts_minimum_remaining_override(self):
        self.evaluate(used=50)
        rows = [json.loads(line) for line in self.path.read_text().splitlines()]
        current = datetime.now(timezone.utc)
        rows[-1]['timestamp'] = current.isoformat()
        rows[-1]['payload']['rate_limits']['primary']['resets_at'] = (
            current + timedelta(days=1)).timestamp()
        self.path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'current'}), \
                patch('sys.argv', ['codex_quota.py', '--log', str(self.path),
                                   '--min-remaining-percent', '20']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 0)
        result = json.loads(output.call_args.args[0])
        self.assertEqual(result['status'], 'allow_start')
        self.assertEqual(result['floor_remaining_percent'], 20.0)

    def test_cli_rejects_invalid_floor(self):
        with patch.dict('os.environ', {}), \
                patch('sys.argv', ['codex_quota.py', '--min-remaining-percent', 'nan']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 3)
        result = json.loads(output.call_args.args[0])
        self.assertEqual(result['status'], 'unknown')
        self.assertIsNone(result['floor_remaining_percent'])


if __name__ == '__main__':
    unittest.main()
