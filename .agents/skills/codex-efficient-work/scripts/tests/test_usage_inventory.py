"""Use disposable logs to check schema coverage without touching live sessions."""

from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    'usage_inventory', Path(__file__).parents[1] / 'usage_inventory.py')
INVENTORY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INVENTORY)


class UsageInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'rollout.jsonl'

    def write(self, rows):
        self.path.write_text(''.join(json.dumps(row) + '\n' for row in rows))

    def response(self, *, thread='lead', response='r1', amount=100, nested=False):
        payload = {'thread_id': thread, 'response_id': response,
                   'usage': {'input_tokens': amount, 'output_tokens': 5}}
        if nested:
            payload['type'] = 'token_usage_record'
        return {'type': 'event_msg' if nested else 'token_usage_record', 'payload': payload}

    def test_top_level_response_is_found_beside_cumulative_events(self):
        snapshot = {'type': 'event_msg', 'payload': {'type': 'token_count', 'info': {
            'total_token_usage': {'input_tokens': 100},
            'last_token_usage': {'input_tokens': 100}}}}
        self.write([snapshot, snapshot, self.response()])
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['per_response_locations'], {'top_level': 1})
        self.assertEqual(result['unique_response_keys'], 1)
        self.assertEqual(result['cumulative_snapshots'], 2)
        self.assertEqual(result['last_usage_snapshots'], 2)
        self.assertEqual(result['recommendation'], 'inspect_per_response_records_and_resolve_gaps')
        self.assertNotIn('total_tokens', result)

    def test_nested_records_and_session_metadata_are_supported(self):
        record = self.response(nested=True)
        del record['payload']['thread_id']
        self.write([{'type': 'session_meta', 'payload': {'id': 'lead'}}, record,
                    self.response()])
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['per_response_records'], 2)
        self.assertEqual(result['unique_response_keys'], 1)
        self.assertEqual(result['duplicate_response_records'], 1)
        self.assertEqual(result['unidentified_response_records'], 0)

    def test_duplicate_conflicts_are_reported_without_being_silently_chosen(self):
        self.write([self.response(), self.response(), self.response(amount=101)])
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['duplicate_response_records'], 2)
        self.assertEqual(result['conflicting_duplicate_keys'], 1)
        with redirect_stdout(io.StringIO()):
            self.assertEqual(INVENTORY.main(['--log', str(self.path)]), 2)

    def test_same_response_id_in_different_threads_is_not_a_duplicate(self):
        self.write([self.response(), self.response(thread='child')])
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['unique_response_keys'], 2)
        self.assertEqual(result['duplicate_response_records'], 0)

    def test_unidentified_response_does_not_invite_cumulative_fallback(self):
        self.write([self.response(response=None)])
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['unidentified_response_records'], 1)
        self.assertEqual(result['unique_response_keys'], 0)
        self.assertEqual(result['recommendation'], 'inspect_per_response_records_and_resolve_gaps')

    def test_snapshot_only_log_is_explicitly_a_fallback_candidate(self):
        self.write([{'type': 'event_msg', 'payload': {'type': 'token_count', 'info': {
            'total_token_usage': {'input_tokens': 100}}}}])
        self.assertEqual(INVENTORY.inventory(self.path)['recommendation'],
                         'consider_labelled_cumulative_delta_fallback')

    def test_malformed_input_preserves_detectable_coverage_but_is_incomplete(self):
        self.write([self.response()])
        with self.path.open('a') as stream:
            stream.write('{unfinished\n[]\n')
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['unique_response_keys'], 1)
        self.assertEqual(result['malformed_records'], 2)
        self.assertFalse(result['scan_complete'])

    def test_cli_is_read_only_and_does_not_print_conversation_bodies(self):
        self.write([self.response(), {'type': 'response_item', 'payload': {
            'text': 'private-transcript-marker'}}])
        original = self.path.read_bytes()
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(INVENTORY.main(['--log', str(self.path)]), 0)
        self.assertEqual(self.path.read_bytes(), original)
        self.assertNotIn('private-transcript-marker', out.getvalue())
        self.assertEqual(json.loads(out.getvalue())['logs'][0]['unique_response_keys'], 1)

    def test_missing_log_reports_failure_without_creating_it(self):
        result = INVENTORY.inventory(self.path)
        self.assertEqual(result['read_error'], 'FileNotFoundError')
        self.assertFalse(result['scan_complete'])
        self.assertFalse(self.path.exists())


if __name__ == '__main__':
    unittest.main()
