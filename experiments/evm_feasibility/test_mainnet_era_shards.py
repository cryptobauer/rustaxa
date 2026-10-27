"""Adversarial coverage checks for the finite era shard assembler."""
import copy
import json
import unittest

import mainnet_era_reference as era


def fixture():
    scenario = {'name': 'unit', 'head': 10, 'first_retained': 9, 'periods': [11, 12, 13]}
    root = lambda p: f'{p:064x}'
    continuity = [{'period': p, 'prior_root': root(p - 1), 'root': root(p), 'transaction_hashes': []} for p in range(10, 14)]
    cases = []
    for name in era.api_names(scenario):
        head, request_name = name.split('/')
        callback = lambda stage: {'stage': stage, 'before': {}, 'after': {}, 'result': {}, 'ordered_raw_writes': []}
        cases.append({'name': name, 'head': int(head), 'request': {'name': request_name}, 'trace_period': int(head),
                      'call': {'gas': 1}, 'repeat_call': {'gas': 1}, 'estimates': [{'probes': [1]}],
                      'trace': {'result': [], 'calls': [callback('target')]},
                      'trace_sequence': {'result': [], 'calls': [callback('prefix'), callback('target')]}})
    identities = [era.digest(json.dumps({k: row[k] for k in ['head', 'request', 'trace_period']}, sort_keys=True, separators=(',', ':')).encode()) for row in cases]
    code_binding = era.code_objects_binding({})
    binding = {'schema': 3, 'reference': 'a' * 40, 'scenario': 'unit', 'starting_head': 10, **code_binding,
               'stop_before_rewards': 0, 'seed_identity': {'root': root(9)}, 'continuity': continuity,
               'api_case_names': era.api_names(scenario), 'api_case_identities': identities,
               'input_sha256': 'b' * 64, 'execution_input_sha256': 'c' * 64,
               'configuration_sha256': 'd' * 64, 'history_sha256': era.digest(b'{}'), 'observer': True}
    shards = []
    for selector in era.selectors(scenario):
        payload = {'history_rows': {}, 'code_objects': {}, **code_binding} if selector['kind'] == 'base' else (
            {'periods': [{'number': p, 'prior_root': root(p - 1), 'transactions': [], 'final': {'root': root(p)}}
                         for p in range(selector['first'], selector['end'])]} if selector['kind'] == 'periods' else
            {'api_cases': cases[selector['first']:selector['end']]})
        shards.append({**copy.deepcopy(binding), 'selector': {'kind': selector['kind'], 'first': selector.get('first', -1), 'end': selector.get('end', -1)}, 'payload': payload})
    return scenario, shards


class ShardCoverage(unittest.TestCase):
    def test_exact_complete_assembly(self):
        scenario, shards = fixture()
        result = era.assemble(shards, 'a' * 40, scenario)
        self.assertEqual([p['number'] for p in result['periods']], [10, 11, 12, 13])
        self.assertEqual(len(result['api_cases']), 21)

    def test_rejects_missing_duplicate_overlap_and_extra(self):
        scenario, shards = fixture()
        for bad in [shards[:-1], shards + [shards[0]], [shards[0], shards[1], shards[2], shards[2], shards[4]]]:
            with self.assertRaises(ValueError): era.assemble(bad, 'a' * 40, scenario)
        bad = copy.deepcopy(shards); bad[1]['selector']['first'] += 1
        with self.assertRaises(ValueError): era.assemble(bad, 'a' * 40, scenario)

    def test_rejects_identity_and_continuity_changes(self):
        scenario, shards = fixture()
        for field in ['reference', 'scenario', 'starting_head', 'input_sha256', 'execution_input_sha256', 'configuration_sha256', 'history_sha256', 'observer', 'stop_before_rewards', 'seed_identity', 'continuity', 'api_case_names', 'api_case_identities', 'code_objects_schema', 'code_objects_count', 'code_objects_sha256']:
            bad = copy.deepcopy(shards); bad[1][field] = 'changed'
            with self.subTest(field=field), self.assertRaises(ValueError): era.assemble(bad, 'a' * 40, scenario)

    def test_rejects_period_gap_duplicate_root_and_transaction_change(self):
        scenario, shards = fixture()
        for mutate in [lambda rows: rows.pop(), lambda rows: rows.__setitem__(1, rows[0]), lambda rows: rows[0]['final'].__setitem__('root', 'bad'), lambda rows: rows[0]['transactions'].append({'hash': 'unexpected'})]:
            bad = copy.deepcopy(shards); mutate(bad[1]['payload']['periods'])
            with self.assertRaises(ValueError): era.assemble(bad, 'a' * 40, scenario)

    def test_rejects_truncated_api_case_and_callbacks(self):
        scenario, shards = fixture()
        for mutate in [lambda row: row.__setitem__('name', 'wrong'), lambda row: row.__setitem__('request', {}), lambda row: row.__setitem__('repeat_call', {}), lambda row: row.__setitem__('estimates', []), lambda row: row['trace'].__setitem__('calls', []), lambda row: row['trace_sequence']['calls'].reverse(), lambda row: row['trace']['calls'][0].pop('after')]:
            bad = copy.deepcopy(shards); mutate(bad[2]['payload']['api_cases'][0])
            with self.assertRaises(ValueError): era.assemble(bad, 'a' * 40, scenario)

    def test_rejects_selector_grammar(self):
        scenario, shards = fixture()
        for selector in [{'kind': 'unknown'}, {'kind': 'base', 'first': 0}, {'kind': 'fork'}, {'kind': 'api', 'first': False, 'end': 7}, {'kind': 'api', 'first': 0.0, 'end': 7}, {'kind': 'api', 'first': -1, 'end': 7}, {'kind': 'api', 'first': 8, 'end': 7}, {'kind': 'api', 'first': 0, 'end': 8}, {'kind': 'periods', 'first': 10, 'end': 50}]:
            with self.subTest(selector=selector), self.assertRaises(ValueError): era.validate_shard(shards[0], 'a' * 40, scenario, selector, 0)

    def test_rejects_unbound_code_reference(self):
        scenario, shards = fixture()
        shards[1]['payload']['periods'][0]['final']['accounts'] = [{'code_object':'0'*64,'code_hash':'0'*64,'code_size':1}]
        with self.assertRaises(ValueError): era.assemble(shards,'a'*40,scenario)


if __name__ == '__main__':
    unittest.main()
