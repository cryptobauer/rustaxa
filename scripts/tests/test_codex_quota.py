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
                 min_remaining_percent=QUOTA.MIN_REMAINING_PERCENT,
                 expected_session='current'):
        rows = [
            {'type': 'session_meta', 'payload': {'id': identity}},
            {'type': 'turn_context', 'payload': {'model': 'gpt-6.1-sol', 'effort': 'medium'}},
            {'type': 'event_msg', 'timestamp': (self.now - timedelta(seconds=age)).isoformat(),
             'payload': {'type': 'token_count', 'rate_limits': {'primary': {
                 'used_percent': used, 'window_minutes': window,
                 'resets_at': reset if reset is not None else (self.now + timedelta(days=1)).timestamp()}}}},
        ]
        self.path.write_text(''.join(json.dumps(r) + '\n' for r in rows) + extra)
        return QUOTA.inspect_log(self.path, now=self.now, expected_session=expected_session,
                                 min_remaining_percent=min_remaining_percent)

    def budget_observation(self, used, *, identity='current', reset=None, age=1):
        result, code = self.evaluate(used=used, identity=identity, reset=reset, age=age,
                                     min_remaining_percent=0, expected_session=identity)
        self.assertIn(code, (0, 2))
        return result

    def write_live_log(self, *, used, identity='lead', reset=None, age=1):
        now = datetime.now(timezone.utc)
        reset = reset if reset is not None else (now + timedelta(days=1)).timestamp()
        rows = [
            {'type': 'session_meta', 'payload': {'id': identity}},
            {'type': 'turn_context', 'payload': {'model': 'gpt-6.1-sol', 'effort': 'medium'}},
            {'type': 'event_msg', 'timestamp': (now - timedelta(seconds=age)).isoformat(),
             'payload': {'type': 'token_count', 'rate_limits': {'primary': {
                 'used_percent': used, 'window_minutes': 10080, 'resets_at': reset}}}},
        ]
        self.path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        return now, reset

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

    def test_budget_starts_at_80_and_stops_at_70(self):
        state_path = Path(self.temp.name) / 'nested' / 'quota-budget.json'
        baseline = self.budget_observation(used=20)
        result, code = QUOTA.initialize_budget(state_path, baseline, now=self.now)
        self.assertEqual(code, 0)
        self.assertTrue(result['budget_initialized'])
        self.assertEqual(result['baseline_remaining_percent'], 80)
        self.assertEqual(result['consumed_percent'], 0)
        self.assertEqual(result['floor_remaining_percent'], 70)
        self.assertIn('10 percentage points remain', result['reason'])
        self.assertEqual(json.loads(state_path.read_text())['consume_percent'], 10)

        almost, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=29), now=self.now)
        self.assertEqual(code, 0)
        self.assertIn('1 percentage point remains', almost['reason'])
        stopped, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=30), now=self.now)
        self.assertEqual(code, 2)
        self.assertEqual(stopped['consumed_percent'], 10)
        self.assertEqual(stopped['floor_remaining_percent'], 70)

    def test_budget_uses_starting_remaining_for_other_baseline(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        result, code = QUOTA.initialize_budget(
            state_path, self.budget_observation(used=43), now=self.now)
        self.assertEqual(code, 0)
        self.assertEqual(result['floor_remaining_percent'], 47)
        stopped, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=53), now=self.now)
        self.assertEqual(code, 2)
        self.assertEqual(stopped['remaining_percent'], 47)

    def test_log_regression_cannot_reopen_saved_budget(self):
        for peak in (39, 44):
            with self.subTest(peak=peak):
                state_path = Path(self.temp.name) / f'budget-{peak}.json'
                baseline = self.budget_observation(used=34)
                QUOTA.initialize_budget(state_path, baseline, now=self.now)
                original = state_path.read_bytes()
                self.evaluate(used=peak, min_remaining_percent=0)
                first = self.path.read_text()
                self.evaluate(used=peak - 1, min_remaining_percent=0)
                self.path.write_text(first + self.path.read_text())
                observation, code = QUOTA.inspect_log(
                    self.path, now=self.now, expected_session='current', min_remaining_percent=0)
                self.assertEqual(code, 3)
                self.assertIn('decreased', observation['reason'])
                self.assertEqual(QUOTA.inspect_budget(state_path, observation, now=self.now)[1], 3)
                self.assertEqual(state_path.read_bytes(), original)

    def test_log_usage_peak_is_separate_for_each_reset_window(self):
        self.evaluate(used=90, min_remaining_percent=0)
        first = self.path.read_text()
        reset = (self.now + timedelta(days=2)).timestamp()
        self.evaluate(used=10, reset=reset, min_remaining_percent=0)
        self.path.write_text(first + self.path.read_text())
        observation, code = QUOTA.inspect_log(
            self.path, now=self.now, expected_session='current', min_remaining_percent=0)
        self.assertEqual(code, 0)
        self.assertEqual(observation['used_percent'], 10)

    def test_initialization_can_set_a_valid_custom_budget(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        result, code = QUOTA.initialize_budget(
            state_path, self.budget_observation(used=43), consume_percent=7, now=self.now)
        self.assertEqual(code, 0)
        self.assertEqual(result['floor_remaining_percent'], 50)
        self.assertEqual(result['consume_percent'], 7)

    def test_resume_keeps_original_baseline_without_rebasing(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        QUOTA.initialize_budget(state_path, self.budget_observation(used=20), now=self.now)
        original = state_path.read_text()
        resumed, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=25), now=self.now)
        self.assertEqual(code, 0)
        self.assertEqual(resumed['baseline_used_percent'], 20)
        self.assertEqual(resumed['consumed_percent'], 5)
        self.assertEqual(resumed['floor_remaining_percent'], 70)
        self.assertEqual(state_path.read_text(), original)

    def test_budget_reports_stale_observation_as_unknown(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        QUOTA.initialize_budget(state_path, self.budget_observation(used=20), now=self.now)
        stale = self.budget_observation(used=21)
        stale['age_seconds'] = 301
        stale['observed_utc'] = (self.now - timedelta(seconds=301)).isoformat()
        result, code = QUOTA.inspect_budget(state_path, stale, now=self.now)
        self.assertEqual(code, 3)
        self.assertEqual(result['status'], 'unknown')
        self.assertEqual(result['floor_remaining_percent'], 70)
        self.assertIsNone(result['consumed_percentage_points'])

    def test_budget_rejects_missing_malformed_identity_reset_and_rollback(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        unknown, code = QUOTA.inspect_budget(state_path, self.budget_observation(used=20))
        self.assertEqual(code, 3)
        self.assertIn('missing', unknown['reason'])

        state_path.write_text('{bad json')
        self.assertEqual(QUOTA.inspect_budget(
            state_path, self.budget_observation(used=20))[1], 3)
        state_path.unlink()
        QUOTA.initialize_budget(state_path, self.budget_observation(used=20), now=self.now)

        other_session, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=21, identity='worker'), now=self.now)
        self.assertEqual(code, 3)
        self.assertIn('lead baseline session', other_session['reason'])
        changed_reset = (self.now + timedelta(days=2)).timestamp()
        reset_result, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=21, reset=changed_reset), now=self.now)
        self.assertEqual(code, 3)
        self.assertIn('different allowance reset', reset_result['reason'])
        rollback, code = QUOTA.inspect_budget(
            state_path, self.budget_observation(used=19), now=self.now)
        self.assertEqual(code, 3)
        self.assertIn('below the saved baseline', rollback['reason'])
        self.assertEqual(rollback['baseline_used_percent'], 20)

    def test_budget_rejects_contradictory_and_invalid_baseline_fields(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        QUOTA.initialize_budget(state_path, self.budget_observation(used=20), now=self.now)
        original = json.loads(state_path.read_text())
        mutations = (
            {'consume_percent': True}, {'consume_percent': 101},
            {'baseline_used_percent': 21}, {'floor_remaining_percent': 69},
            {'baseline_remaining_percent': 79}, {'resets_at': float('nan')},
            {'observed_utc': 'no timestamp'},
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                state_path.write_text(json.dumps({**original, **mutation}))
                result, code = QUOTA.inspect_budget(
                    state_path, self.budget_observation(used=21), now=self.now)
                self.assertEqual(code, 3)
                self.assertEqual(result['status'], 'unknown')
        state_path.write_text(json.dumps(original))

    def test_initialization_never_overwrites_existing_file(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        state_path.write_text('preserve this')
        result, code = QUOTA.initialize_budget(
            state_path, self.budget_observation(used=20), now=self.now)
        self.assertEqual(code, 3)
        self.assertIn('never overwrites', result['reason'])
        self.assertEqual(state_path.read_text(), 'preserve this')

    def test_invalid_consume_value_does_not_create_state(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        observation = self.budget_observation(used=20)
        for value in (True, -1, 101, float('nan'), float('inf')):
            with self.subTest(value=value):
                result, code = QUOTA.initialize_budget(
                    state_path, observation, consume_percent=value, now=self.now)
                self.assertEqual(code, 3)
                self.assertIn('Consume percentage', result['reason'])
                self.assertFalse(state_path.exists())

    def test_stale_initialization_does_not_create_budget_file(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        stale, code = QUOTA.inspect_log(self.path, now=self.now, expected_session='current',
                                        min_remaining_percent=0)
        self.assertEqual(code, 3)
        result, code = QUOTA.initialize_budget(state_path, stale)
        self.assertEqual(code, 3)
        self.assertFalse(state_path.exists())

    def test_cli_budget_flags_and_lead_identity(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        cases = (
            (['--init-budget'], 'requires --budget-file'),
            (['--consume-percent', '10'], 'only valid with --init-budget'),
            (['--budget-file', str(state_path), '--min-remaining-percent', '20'],
             'cannot be combined'),
        )
        for args, reason in cases:
            with self.subTest(args=args), patch.dict('os.environ', {'CODEX_THREAD_ID': 'lead'}), \
                    patch('sys.argv', ['codex_quota.py', *args]), \
                    patch('builtins.print') as output:
                self.assertEqual(QUOTA.main(), 3)
                self.assertIn(reason, json.loads(output.call_args.args[0])['reason'])

        with patch.dict('os.environ', {}, clear=True), \
                patch('sys.argv', ['codex_quota.py', '--log', str(self.path),
                                   '--budget-file', str(state_path), '--init-budget']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 3)
        self.assertIn('lead session identity', json.loads(output.call_args.args[0])['reason'])
        self.assertFalse(state_path.exists())

    def test_cli_initializes_then_resumes_budget(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        now, reset = self.write_live_log(used=20)
        common = ['--log', str(self.path), '--budget-file', str(state_path)]
        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'lead'}), \
                patch('sys.argv', ['codex_quota.py', *common, '--init-budget']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 0)
        initialized = json.loads(output.call_args.args[0])
        self.assertEqual(initialized['baseline_session_id'], 'lead')
        self.assertEqual(initialized['floor_remaining_percent'], 70)
        self.assertTrue(state_path.exists())

        self.write_live_log(used=25, identity='lead', reset=reset)
        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'lead'}), \
                patch('sys.argv', ['codex_quota.py', *common]), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 0)
        resumed = json.loads(output.call_args.args[0])
        self.assertEqual(resumed['baseline_used_percent'], 20)
        self.assertEqual(resumed['consumed_percent'], 5)
        self.assertEqual(resumed['consumed_percentage_points'], 5)
        self.assertEqual(resumed['floor_remaining_percent'], 70)

        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'worker'}), \
                patch('sys.argv', ['codex_quota.py', *common]), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 3)
        self.assertIn('does not match', json.loads(output.call_args.args[0])['reason'])

    def test_cli_accepts_custom_consume_only_during_initialization(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        self.write_live_log(used=20)
        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'lead'}), \
                patch('sys.argv', ['codex_quota.py', '--log', str(self.path),
                                   '--budget-file', str(state_path), '--init-budget',
                                   '--consume-percent', '12.5']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 0)
        result = json.loads(output.call_args.args[0])
        self.assertEqual(result['consume_percent'], 12.5)
        self.assertEqual(result['floor_remaining_percent'], 67.5)
        self.assertEqual(json.loads(state_path.read_text())['consume_percent'], 12.5)

    def test_cli_existing_initialization_preserves_file_and_stale_start_has_no_file(self):
        state_path = Path(self.temp.name) / 'quota-budget.json'
        state_path.write_text('preserve')
        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'current'}), \
                patch('sys.argv', ['codex_quota.py', '--log', str(self.path),
                                   '--budget-file', str(state_path), '--init-budget']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 3)
        self.assertEqual(state_path.read_text(), 'preserve')
        state_path.unlink()

        rows = [
            {'type': 'session_meta', 'payload': {'id': 'current'}},
            {'type': 'event_msg', 'timestamp': (self.now - timedelta(minutes=6)).isoformat(),
             'payload': {'type': 'token_count', 'rate_limits': {'primary': {
                 'used_percent': 20, 'window_minutes': 10080,
                 'resets_at': (self.now + timedelta(days=1)).timestamp()}}}},
        ]
        self.path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        with patch.dict('os.environ', {'CODEX_THREAD_ID': 'current'}), \
                patch('sys.argv', ['codex_quota.py', '--log', str(self.path),
                                   '--budget-file', str(state_path), '--init-budget']), \
                patch('builtins.print') as output:
            self.assertEqual(QUOTA.main(), 3)
        self.assertFalse(state_path.exists())


if __name__ == '__main__':
    unittest.main()
