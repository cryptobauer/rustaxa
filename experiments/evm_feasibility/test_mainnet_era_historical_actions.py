"""Pure coordinate checks for bounded historical donors; no Go execution."""
from copy import deepcopy
import hashlib
import json
import unittest

from mainnet_era_historical_actions import validate_constant_pending_domain
from mainnet_era_profile import profile, scenarios


class CornusMaturityCoordinates(unittest.TestCase):
    def setUp(self):
        self.policy = profile()
        self.rows = {row['name']: row for row in scenarios()}
        self.boundary = self.policy['hardforks']['cornus_hf']['block_num']
        self.lock = self.policy['hardforks']['cornus_hf']['delegation_locking_period']

    def test_real_v2_lock_and_relative_action_api_coordinates(self):
        self.assertEqual(self.lock, 163459)
        created = self.boundary+110
        donor = self.rows['cornus_live_v2_donor']
        self.assertEqual(donor['head'], created-1)
        self.assertEqual(donor['first_retained'], created-5)
        self.assertEqual(donor['periods'], [created, created+1])
        self.assertEqual([row['period'] for row in donor['actions']], [created, created+1])
        self.assertEqual(donor['actions'][0]['method'], 'undelegateV2')
        self.assertEqual(donor['actions'][0]['amount'], str(10**21))
        mature = created+self.lock
        target = self.rows['cornus_v2_maturity']
        self.assertEqual(target['donor'], donor)
        self.assertEqual(target['head'], mature-2)
        self.assertEqual(target['first_retained'], mature-6)
        self.assertEqual(target['periods'], list(range(mature-1, mature+6)))
        self.assertEqual(target['api_heads'], [mature-1, mature, mature+1, mature+5])
        self.assertEqual([row['period'] for row in target['actions']], [mature-1, mature, mature+1])
        self.assertEqual([row['name'] for row in target['actions']], [
            'real_v2_still_locked', 'real_v2_mature_confirmation', 'real_v2_duplicate_confirmation'])
        self.assertTrue(all(row['id'] == 1 and row['method'] == 'confirmV2' for row in target['actions']))

    def test_former_b_plus_ten_domain_rejected_before_reference_launch(self):
        old = deepcopy(self.rows['cornus_live_v2_donor'])
        for name in ('head', 'first_retained'):
            old[name] -= 100
        for name in ('periods', 'pending_periods'):
            old[name] = [period-100 for period in old[name]]
        for action in old['actions']:
            action['period'] -= 100
        retained_pending = [p for p in old['pending_periods'] if p <= old['first_retained']]
        self.assertEqual(retained_pending[0]-1-5, self.boundary-5)
        with self.assertRaisesRegex(ValueError, 'crosses a rule activation'):
            validate_constant_pending_domain(old, self.policy)
        self.assertEqual(validate_constant_pending_domain(self.rows['cornus_live_v2_donor'], self.policy),
                         (self.boundary+95, self.boundary+104))

    def test_unfinished_batch_donor_must_precede_earliest_logical_reader(self):
        row = deepcopy(self.rows['cornus_v2_maturity'])
        low, _ = validate_constant_pending_domain(row, self.policy)
        row['donor']['periods'][-1] = low
        with self.assertRaisesRegex(ValueError, 'donor does not predate earliest pending reader'):
            validate_constant_pending_domain(row, self.policy)

    def test_every_other_profile_matches_pre_correction_inputs(self):
        # Compact portable pins from the checked pre-correction input objects.
        # These preserve all fields, including nested donors and signed actions.
        expected = {
            'mainnet_second_distribution_followthrough':'13614edd7b0c0e1cc3bfd46a77be8ca241eebd38e4f894b318ee2cac8f0e69b6', 'aspen_one':'f51e47d542d314656efcbffb30532db85804a929173eb75c8a63b40a199b3556',
            'aspen_two':'0d83d5ca15c9c2329ca62109d3b0d50c4005fed3f7650e0c534dc497c8f095e6',
            'cacti':'193f87d04b7a316e4c7c4cc54bcf88d16599cbe70ae159189f57d767095c5a5c',
            'cacti_custody_donor':'00d66b4a3f48315ff5d52327a45aa9fc330583a92f31de309a49e0061b8d8949',
            'claim_all_correction':'260742ef3af31a11c13d20f3e747082bc3fbc4af7980230eedba88c07df4dd9b',
            'claim_all_membership_donor':'7397449daebae2ca3274a9cb613f2782dfab20fe999e3e87caba437bcffb60c3',
            'cornus':'79b36c02c7df49563075b84e8b7ba6da09534bc33e1ce33d2281aad078a6e4e7',
            'cornus_old_v1_donor':'baa26bce91c815b209e6e561bb59fdde5b2e0d814abb0db02893be1a19781521',
            'ficus':'1678bb3bb3fabe06ee72efe6219da9b92914463ad512441afdf0854251b43b4d',
            'ficus_custody_donor':'7d18525ed6aedda657be471e4c44cd7fb9db45eeace80ddc0a93d3032a1d87c0',
            'ficus_reward_donor':'76f078ff899dd5e6727abbc960cf8aee827c1ad0cff390a35449a50cb31f2c4c',
            'magnolia':'5f24cad8f4b470748ded3992abd284a9cd27e5c8479cc0e99f8a5821b15013c7',
            'mainnet_106':'be845f75ec5bcfd9a5ca1b1532d21d4b03969d8476fa7a7065dbe8e056c897fc',
            'phalaenopsis':'6306d1cf0b6a298b70ce8bbb1520182084f2a79004b7df9aead0c7aaf8a06a45',
            'phalaenopsis_commission_donor':'f95089962de40baf69d34f33a0aa3a0671fa4ba300d7d8731a1218ab3af7e376',
            'redelegation_correction':'30ecc515f30046a247a346b815ac686e47b68c5e39ac2db1bce2262fbbe09510',
            'soleirolia':'7974e98446934db0e272877e4737ae835da81ed4fc5b5e442db8dda3822afab0',
        }
        self.assertEqual(set(self.rows)-set(expected), {'cornus_live_v2_donor','cornus_v2_maturity','mainnet_second_distribution_reopen','mainnet_second_distribution_followthrough_recovery','claim_all_legacy_batch_correction'})
        for name, expected_hash in expected.items():
            raw = json.dumps(self.rows[name], sort_keys=True, separators=(',', ':')).encode()
            with self.subTest(scenario=name):
                self.assertEqual(hashlib.sha256(raw).hexdigest(), expected_hash)



class MagnoliaPositiveFeeBoundary(unittest.TestCase):
    def test_three_real_signed_ordinary_transfers_keep_original_geometry_and_api_sender(self):
        row = next(row for row in scenarios() if row['name'] == 'magnolia')
        boundary = profile()['hardforks']['magnolia_hf']['block_num']
        self.assertEqual(row['head'], boundary-2)
        self.assertEqual(row['first_retained'], boundary-6)
        self.assertEqual(row['periods'], [boundary-1,boundary,boundary+1])
        self.assertNotIn('donor', row)
        self.assertFalse(row.get('no_warmup_delegation', False))
        self.assertEqual([a['period'] for a in row['actions']], [boundary-1,boundary,boundary+1])
        for action in row['actions']:
            self.assertEqual((action['who'],action['method'],action['input'],action['value'],action['gas'],action['gas_price']), (4,'raw','','1',21000,1000000000))
            self.assertEqual(action['to'], 'c48b812bb43401392c037381aca934f4069c0517')
        self.assertNotIn('api_ordinary_nonce', row)

    def test_all_other_current_registered_instances_remain_byte_exact(self):
        expected = {'redelegation_correction': '30ecc515f30046a247a346b815ac686e47b68c5e39ac2db1bce2262fbbe09510', 'phalaenopsis': '6306d1cf0b6a298b70ce8bbb1520182084f2a79004b7df9aead0c7aaf8a06a45', 'claim_all_correction': '260742ef3af31a11c13d20f3e747082bc3fbc4af7980230eedba88c07df4dd9b', 'mainnet_second_distribution_followthrough':'13614edd7b0c0e1cc3bfd46a77be8ca241eebd38e4f894b318ee2cac8f0e69b6', 'aspen_one': 'f51e47d542d314656efcbffb30532db85804a929173eb75c8a63b40a199b3556', 'aspen_two': '0d83d5ca15c9c2329ca62109d3b0d50c4005fed3f7650e0c534dc497c8f095e6', 'ficus': '1678bb3bb3fabe06ee72efe6219da9b92914463ad512441afdf0854251b43b4d', 'cornus': '79b36c02c7df49563075b84e8b7ba6da09534bc33e1ce33d2281aad078a6e4e7', 'soleirolia': '7974e98446934db0e272877e4737ae835da81ed4fc5b5e442db8dda3822afab0', 'cacti': '193f87d04b7a316e4c7c4cc54bcf88d16599cbe70ae159189f57d767095c5a5c', 'cacti_custody_donor': '00d66b4a3f48315ff5d52327a45aa9fc330583a92f31de309a49e0061b8d8949', 'mainnet_106': 'be845f75ec5bcfd9a5ca1b1532d21d4b03969d8476fa7a7065dbe8e056c897fc', 'phalaenopsis_commission_donor': 'f95089962de40baf69d34f33a0aa3a0671fa4ba300d7d8731a1218ab3af7e376', 'claim_all_membership_donor': '7397449daebae2ca3274a9cb613f2782dfab20fe999e3e87caba437bcffb60c3', 'ficus_reward_donor': '76f078ff899dd5e6727abbc960cf8aee827c1ad0cff390a35449a50cb31f2c4c', 'ficus_custody_donor': '7d18525ed6aedda657be471e4c44cd7fb9db45eeace80ddc0a93d3032a1d87c0', 'cornus_old_v1_donor': 'baa26bce91c815b209e6e561bb59fdde5b2e0d814abb0db02893be1a19781521', 'cornus_live_v2_donor': '9599eef70fff129effdac3d4fac5382e1e9d4d6a9a764b0bea0cbb69bd2a6d41', 'cornus_v2_maturity': 'b50e8e65410a318b5688ea1dbe70b33cbe43631bf40c930660ade931908d8ba5'}
        rows = {row['name']:row for row in scenarios()}
        self.assertEqual(set(rows)-{'magnolia','mainnet_second_distribution_reopen','mainnet_second_distribution_followthrough_recovery','claim_all_legacy_batch_correction'},set(expected))
        for name, wanted in expected.items():
            with self.subTest(scenario=name):
                self.assertEqual(hashlib.sha256(json.dumps(rows[name],sort_keys=True,separators=(',',':')).encode()).hexdigest(),wanted)


class AspenPositiveFeeBoundaries(unittest.TestCase):
    def test_signed_boundary_controls_keep_pending_geometry_and_use_captured_api_nonce(self):
        rows = {row['name']: row for row in scenarios()}
        for family, boundary in [('aspen_one', 8118000), ('aspen_two', 8572000)]:
            with self.subTest(family=family):
                row = rows[family]
                self.assertEqual((row['head'], row['first_retained']), (boundary-2, boundary-6))
                self.assertEqual(row['periods'], [boundary-1, boundary, boundary+1])
                self.assertEqual(row['pending_periods'], list(range(boundary-99, boundary-1)))
                self.assertEqual(len([p for p in row['pending_periods'] if p <= row['first_retained']]), 94)
                self.assertNotIn('donor', row)
                self.assertFalse(row.get('no_warmup_delegation', False))
                self.assertNotIn('api_ordinary_nonce', row)
                self.assertEqual([a['period'] for a in row['actions']], [boundary-1, boundary, boundary+1])
                self.assertEqual([a['name'] for a in row['actions']], [family+'_fee_before', family+'_fee_activation', family+'_fee_after'])
                for action in row['actions']:
                    self.assertEqual((action['who'], action['method'], action['input'], action['value'], action['gas'], action['gas_price']), (4, 'raw', '', '1', 21000, 1000000000))
                    self.assertEqual(action['to'], 'c48b812bb43401392c037381aca934f4069c0517')



class SoleiroliaValidExecutionBoundary(unittest.TestCase):
    def test_numeric_limits_are_valid_inputs_not_outer_admission_proof(self):
        row = next(row for row in scenarios() if row['name'] == 'soleirolia')
        b = 17380000
        self.assertEqual((row['head'], row['first_retained']), (b-2, b-6))
        self.assertEqual(row['periods'], [b-1, b, b+1])
        self.assertEqual(row['pending_periods'], list(range(b-99, b-1)))
        self.assertEqual(len([p for p in row['pending_periods'] if p <= row['first_retained']]), 94)
        self.assertNotIn('donor', row)
        self.assertNotIn('api_ordinary_nonce', row)
        self.assertFalse(row.get('no_warmup_delegation', False))
        self.assertEqual([a['period'] for a in row['actions']], [b-1, b, b+1])
        self.assertEqual([a['gas'] for a in row['actions']], [21000, 31500000, 21000])
        self.assertEqual([a['name'] for a in row['actions']], ['soleirolia_fee_before', 'soleirolia_fee_activation', 'soleirolia_fee_after'])
        for action in row['actions']:
            self.assertEqual((action['who'], action['method'], action['input'], action['value'], action['gas_price']), (4, 'raw', '', '1', 1000000000))
            self.assertEqual(action['to'], 'c48b812bb43401392c037381aca934f4069c0517')
        policy = profile()['hardforks']['soleirolia_hf']
        self.assertEqual(int(policy['trx_min_gas_price'], 0), 1000000000)
        self.assertEqual(int(policy['trx_max_gas_limit'], 0), 31500000)


class CurrentSecondDistributionFollowthrough(unittest.TestCase):
    def test_new_case_preserves_original_current_workload_and_explicit_tail(self):
        rows = {row['name']: row for row in scenarios()}
        original = rows['mainnet_106']
        added = rows['mainnet_second_distribution_followthrough']
        expected = dict(original)
        expected.update(name='mainnet_second_distribution_followthrough',
                        periods=list(range(25706895, 25707006)),
                        api_heads=list(range(25707000, 25707006)))
        expected['actions'] = [dict(period=p, who=4, name='second_distribution_transfer_'+str(p), method='raw', to='c48b812bb43401392c037381aca934f4069c0517', input='', value='1', gas=21000, gas_price=1000000000) for p in range(25707000, 25707006)]
        self.assertEqual(added, expected)
        self.assertEqual(original['periods'][-1], 25707000)
        self.assertEqual(added['periods'][-1] - added['first_retained'], 116)
        self.assertEqual(len(added['pending_periods']), 94)
        self.assertNotIn('donor', added)

class CurrentSecondDistributionReopen(unittest.TestCase):
    def test_separate_topology_keeps_exact_existing_actions_and_other_fields(self):
        rows = {row['name']: row for row in scenarios()}
        old = rows['mainnet_second_distribution_followthrough']
        expected = dict(old)
        expected.update(name='mainnet_second_distribution_reopen',
                        reopen_after=list(range(25706999, 25707005)))
        self.assertEqual(rows['mainnet_second_distribution_reopen'], expected)
        self.assertEqual(len(rows), 23)
        raw = json.dumps(expected, sort_keys=True, separators=(',', ':')).encode()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), 'eb34ee6abd2bdf6b4d81a6f0732b661dfebb8b8aeea69585c54d6d7edf2dedc6')

class CurrentSecondDistributionRecovery(unittest.TestCase):
    def test_companion_adds_only_named_reopen_topology(self):
        rows = {row['name']: row for row in scenarios()}
        expected = dict(rows['mainnet_second_distribution_followthrough'])
        expected.update(name='mainnet_second_distribution_followthrough_recovery',
                        reopen_after=[25706905, *range(25706999, 25707005)])
        self.assertEqual(rows['mainnet_second_distribution_followthrough_recovery'], expected)
        self.assertEqual(len(rows), 23)

class ClaimAllLegacyBatchComponent(unittest.TestCase):
    def test_additive_component_keeps_original_and_exact_declared_actions(self):
        from copy import deepcopy
        rows = {row['name']:row for row in scenarios()}
        original = rows['claim_all_correction']
        component = rows['claim_all_legacy_batch_correction']
        expected = deepcopy(original)
        expected['name'] = 'claim_all_legacy_batch_correction'
        expected['actions'] = []
        for action in original['actions']:
            if action['name'] == 'full_claim_route':
                continue
            item = deepcopy(action)
            if item['name'] == 'full_claim_with_value':
                item.update(name='old_batch_zero_with_value', method='raw',
                            to='00'*19+'fe', input='09b72e00'+'00'*32)
            expected['actions'].append(item)
        self.assertEqual(component, expected)
        self.assertEqual(len(original['actions']), 19)
        self.assertEqual(len(component['actions']), 16)
        self.assertEqual(len(component['donor']['actions']), 4)
        for p in (7599999, 7600000, 7600001):
            items = [item for item in component['actions'] if item['period'] == p]
            if p == 7599999:
                self.assertEqual(items[0]['name'], 'create_nested_claim_parent_revert')
                items = items[1:]
            self.assertEqual([item['name'] for item in items], ['old_batch_zero', 'old_batch_one_empty_tail', 'old_batch_truncated', 'old_batch_zero_with_value', 'nested_old_claim_parent_revert'])
            self.assertEqual([item['method'] for item in items], ['raw','raw','raw','raw','call'])
            self.assertEqual(items[3]['value'], '1')
            self.assertEqual(items[3]['input'], '09b72e00'+'00'*32)
            self.assertEqual(items[4]['created_by_key'], 0)
            self.assertEqual(items[4]['created_nonce'], 0)
            self.assertEqual(items[4]['input'], '09b72e00'+'00'*32)
        self.assertEqual(hashlib.sha256(json.dumps(original,sort_keys=True,separators=(',',':')).encode()).hexdigest(), '260742ef3af31a11c13d20f3e747082bc3fbc4af7980230eedba88c07df4dd9b')

if __name__ == '__main__':
    unittest.main()
