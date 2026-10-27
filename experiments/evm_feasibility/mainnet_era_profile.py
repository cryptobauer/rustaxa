#!/usr/bin/env python3
"""Mainnet numeric policy and finite synthetic era coordinates.

This module does not construct real historical state. Allocations and seed facts
belong to the separately authenticated synthetic corpus.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GENESIS = ROOT / 'libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json'
REFERENCES = {'public': '6c7e5338b22d5e596cc2365a88d1f94840e1ee1b',
              'local': 'bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418'}
BOUNDARIES = [('redelegation_correction', 3091000), ('magnolia', 5730000),
              ('phalaenopsis', 6943000), ('claim_all_correction', 7600000),
              ('aspen_one', 8118000), ('aspen_two', 8572000),
              ('ficus', 11616000), ('cornus', 15610000),
              ('soleirolia', 17380000), ('cacti', 24350801)]
HEAD = 25706894
LAST = 25707000


def integer(value):
    return int(value, 0) if isinstance(value, str) else int(value)


def profile():
    raw = GENESIS.read_bytes()
    genesis = json.loads(raw)
    hf = genesis['hardforks']
    coordinates = [hf['fix_redelegate_block_num'], hf['magnolia_hf']['block_num'],
                   hf['phalaenopsis_hf_block_num'], hf['fix_claim_all_block_num'],
                   hf['aspen_hf']['block_num_part_one'], hf['aspen_hf']['block_num_part_two'],
                   hf['ficus_hf']['block_num'], hf['cornus_hf']['block_num'],
                   hf['soleirolia_hf']['block_num'], hf['cacti_hf']['block_num']]
    if coordinates != [b for _, b in BOUNDARIES]:
        raise ValueError('mainnet activation coordinates changed')
    dpos = {k: integer(v) for k, v in genesis['dpos'].items() if k != 'initial_validators'}
    committee = integer(genesis['pbft']['committee_size'])
    # genesis.cpp derives the original rate from 2*lambda + delay (700 ms).
    blocks_per_year = 365 * 24 * 60 * 60 * 1000 // (2 * integer(genesis['pbft']['lambda_ms']) + 700)
    return {'schema': 1, 'chain_id': genesis['chain_id'], 'committee_size': committee,
            'dpos': dpos, 'hardforks': hf, 'blocks_per_year_before_cacti': blocks_per_year,
            'genesis_sha256': hashlib.sha256(raw).hexdigest(),
            'pbft_gas_limit': integer(genesis['pbft']['gas_limit']),
            'dag_gas_limit': integer(genesis['dag']['gas_limit'])}


def frequency(policy, period):
    rules = policy['hardforks']['rewards_distribution_frequency']
    return next((integer(rules[str(start)]) for start in sorted(map(int, rules), reverse=True)
                 if start <= period), 1)


def pending_periods(policy, head):
    freq = frequency(policy, head)
    if freq == 1 or head % freq == 0:
        return []
    activation = max((int(p) for p in policy['hardforks']['rewards_distribution_frequency']
                      if int(p) <= head), default=0)
    return list(range(max(activation, head - head % freq + 1), head + 1))


def frame_matrix():
    """Bounded CALL/STATICCALL/DELEGATECALL BLS admission matrix.

    Input memory remains zero and disjoint from output flags. Each native has
    zero-gas, malformed-size and valid Ficus-shape controls. Actual Go decides
    outcomes before/after the remap; no expected output is synthesized here.
    """
    def push(value):
        raw = value.to_bytes(max(1, (value.bit_length()+7)//8), 'big')
        return bytes([0x5f+len(raw)]) + raw
    code = bytearray()
    at = 0
    cells = [*zip(range(11,20), [256,160,160,512,288,288,384,64,128], strict=True), (9,213), (0x100,160), (0xfa1c,0)]
    for opcode in [0xf1, 0xfa, 0xf4]:
        for address, length in cells:
            for gas, size in [(0, length), (100000, 1), (100000, length)]:
                code += push(0)+push(0)+push(size)+push(32768)
                if opcode == 0xf1: code += push(0)
                code += push(address)+push(gas)+bytes([opcode])+push(at)+bytes([0x52])
                code += bytes([0x3d])+push(at+32)+bytes([0x52])
                at += 64
    code += push(at)+push(0)+bytes([0xf3])
    return code.hex()


def donor_window(policy, name, first, last, actions, donor=None):
    result = {'name': name, 'head': first-1, 'first_retained': first-5,
              'periods': list(range(first,last+1)), 'pending_periods': pending_periods(policy,first-1),
              'seed_actions_only': True, 'no_warmup_delegation': True, 'actions': actions}
    if donor is not None: result['donor'] = donor
    return result


def scenarios():
    policy = profile()
    windows = [{'name': name, 'head': boundary - 2, 'first_retained': boundary - 6,
                'periods': [boundary - 1, boundary, boundary + 1],
                'pending_periods': pending_periods(policy, boundary - 2)}
               for name, boundary in BOUNDARIES]
    cacti = next(value for value in windows if value['name'] == 'cacti')
    boundary = dict(BOUNDARIES)['cacti']
    old_lock = int(policy['hardforks']['cornus_hf']['delegation_locking_period'])
    creation = boundary-1-old_lock
    amount = '1000000000000000000000'
    proof_period = boundary-1-int(policy['hardforks']['magnolia_hf']['jail_time'])
    donor_first, donor_last = min(creation-1,proof_period), max(creation+1,proof_period+2)
    if donor_last-donor_first > 7:
        raise ValueError('custody/jail donor requires a separately reviewed bounded composition')
    custody = donor_window(policy, 'cacti_custody_donor', donor_first, donor_last, [
        {'period':creation-1,'who':4,'name':'old_delegation','method':'delegate','validator':0,'value':str(2*int(amount))},
        {'period':creation,'who':0,'name':'old_v1_queue','method':'undelegate','validator':0,'amount':amount},
        {'period':creation+1,'who':0,'name':'old_v2_queue','method':'undelegateV2','validator':0,'amount':amount},
        {'period':proof_period,'who':4,'name':'old_jail_proof','method':'slash','validator':0,'vote_period':proof_period-5},
        {'period':proof_period+2,'who':4,'name':'old_cacti_subject_jail','method':'slash','validator':2,'vote_period':proof_period-3},
    ])
    last = ((boundary//100)+1)*100
    legacy = '3615600c576000356000b4005b6000b360005260206000f3'
    modern = '3615600c5760003560005d005b60005c60005260206000f3'
    from mainnet_era_compare import keccak, rlp_encode
    owner = bytes.fromhex('1a642f0e3c3af545e7acbd38b07251b3990914f1')
    modern_address = keccak(rlp_encode([owner,3]))[-20:]
    reverting_address = keccak(rlp_encode([owner,5]))[-20:]
    def push(value):
        raw = value.to_bytes(max(1,(value.bit_length()+7)//8),'big')
        return bytes([0x5f+len(raw)])+raw
    def call(address, gas, input_size=0, output_size=0):
        return push(output_size)+push(0)+push(input_size)+push(0)+push(0)+bytes([0x73])+address+push(gas)+bytes([0xf1,0x50])
    reverting = (push(9)+push(0)+bytes([0x52])+call(modern_address,100000,32)+push(0)+push(0)+bytes([0xfd])).hex()
    rollback = (call(reverting_address,200000)+call(modern_address,100000,0,32)+push(32)+push(0)+bytes([0xf3])).hex()
    actions = [
        {'period':boundary-1,'who':0,'name':'create_legacy_transient','method':'create','input':legacy,'created_nonce':2,'created_by_key':0},
        {'period':boundary-1,'who':0,'name':'create_cacti_transient','method':'create','input':modern,'created_nonce':3,'created_by_key':0},
        {'period':boundary-1,'who':0,'name':'create_registry_matrix','method':'create','input':frame_matrix(),'gas':2000000,'created_nonce':4,'created_by_key':0},
        {'period':boundary-1,'who':0,'name':'create_reverting_transient_child','method':'create','input':reverting,'created_nonce':5,'created_by_key':0},
        {'period':boundary-1,'who':0,'name':'create_transient_rollback_probe','method':'create','input':rollback,'created_nonce':6,'created_by_key':0},
        {'period':boundary-1,'who':0,'name':'mature_old_v1','method':'confirm','validator':0},
        {'period':boundary-1,'who':0,'name':'locked_old_v2','method':'confirmV2','validator':0,'id':1},
        {'period':boundary,'who':0,'name':'mature_old_v2','method':'confirmV2','validator':0,'id':1},
        {'period':boundary-1,'who':4,'name':'cornus_lock_creation','method':'undelegateV2','validator':0,'amount':amount},
        {'period':boundary,'who':4,'name':'cacti_lock_creation','method':'undelegateV2','validator':0,'amount':amount},
        {'period':boundary+1,'who':4,'name':'cacti_lock_not_mature','method':'confirmV2','validator':0,'id':2},
        {'period':boundary+1,'who':4,'name':'cancel_cacti_lock','method':'cancelV2','validator':0,'id':2},
        {'period':boundary+1,'who':4,'name':'cancel_old_lock','method':'cancelV2','validator':0,'id':1},
        {'period':boundary,'who':4,'name':'new_cacti_jail','method':'slash','validator':2,'vote_period':boundary-5},
        {'period':boundary,'who':4,'name':'duplicate_cacti_jail','method':'slash','validator':2,'vote_period':boundary-5},
    ]
    for p in [boundary-1,boundary,boundary+1]:
        for nonce,label in [(2,'legacy'),(3,'cacti')]:
            actions += [
                {'period':p,'who':4,'name':label+'_transient_write','method':'call','created_by_key':0,'created_nonce':nonce,'input':'00'*31+'09'},
                {'period':p,'who':4,'name':label+'_transient_reset','method':'call','created_by_key':0,'created_nonce':nonce},
            ]
        actions.append({'period':p,'who':4,'name':'registry_frame_matrix','method':'call','created_by_key':0,'created_nonce':4,'gas':10000000})
        actions.append({'period':p,'who':4,'name':'transient_parent_rollback','method':'call','created_by_key':0,'created_nonce':6,'gas':500000})
    cacti.update({'donor':custody,'no_warmup_delegation':True,'periods':list(range(boundary-1,last+1)),
                  'required_activation_periods':[boundary-1,boundary,boundary+1], 'reward_followthrough_last':last,
                  'api_heads':[boundary-1,boundary,boundary+1,boundary+4,boundary+5,last],'api_ordinary_nonce':3,'api_validator':2,
                  'ordinary_api_name':'ordinary_instruction_boundary','actions':actions})
    windows.append(custody)
    windows.append({'name': 'mainnet_106', 'head': HEAD, 'first_retained': HEAD - 5,
                    'periods': list(range(HEAD + 1, LAST + 1)),
                    'pending_periods': pending_periods(policy, HEAD)})
    from mainnet_era_historical_actions import apply_historical_actions
    return apply_historical_actions(windows, policy, frame_matrix)


def go_configuration():
    """Convert numeric policy to the actual two-pin Go ChainConfig wire shape."""
    p = profile()
    h = p['hardforks']
    names = {'delegation_delay': 'DelegationDelay', 'delegation_locking_period': 'DelegationLockingPeriod',
             'eligibility_balance_threshold': 'EligibilityBalanceThreshold',
             'vote_eligibility_balance_step': 'VoteEligibilityBalanceStep',
             'validator_maximum_stake': 'ValidatorMaximumStake', 'minimum_deposit': 'MinimumDeposit',
             'max_block_author_reward': 'MaxBlockAuthorReward', 'dag_proposers_reward': 'DagProposersReward',
             'commission_change_delta': 'CommissionChangeDelta', 'commission_change_frequency': 'CommissionChangeFrequency',
             'yield_percentage': 'YieldPercentage'}
    dpos = {names[k]: v for k, v in p['dpos'].items()}
    dpos['BlocksPerYear'] = p['blocks_per_year_before_cacti']
    # Native uint256 policy values remain JSON integers, never binary floats.
    return {'EVMChainConfig': {'ChainId': p['chain_id']}, 'GenesisBalances': {}, 'DPOS': dpos,
            'Hardforks': {
                'FixRedelegateBlockNum': h['fix_redelegate_block_num'],
                'Redelegations': [{'Validator': r['validator'], 'Delegator': r['delegator'],
                                  'Amount': integer(r['amount'])} for r in h['redelegations']],
                'RewardsDistributionFrequency': h['rewards_distribution_frequency'],
                'MagnoliaHf': {'BlockNum': h['magnolia_hf']['block_num'], 'JailTime': h['magnolia_hf']['jail_time']},
                'PhalaenopsisHfBlockNum': h['phalaenopsis_hf_block_num'],
                'FixClaimAllBlockNum': h['fix_claim_all_block_num'],
                'AspenHf': {'BlockNumPartOne': h['aspen_hf']['block_num_part_one'],
                            'BlockNumPartTwo': h['aspen_hf']['block_num_part_two'],
                            'MaxSupply': integer(h['aspen_hf']['max_supply']),
                            'GeneratedRewards': integer(h['aspen_hf']['generated_rewards'])},
                'FicusHf': {'BlockNum': h['ficus_hf']['block_num'],
                            'PillarBlocksInterval': h['ficus_hf']['pillar_blocks_interval'],
                            'BridgeContractAddress': h['ficus_hf']['bridge_contract_address']},
                'CornusHf': {'BlockNum': h['cornus_hf']['block_num'],
                             'DelegationLockingPeriod': h['cornus_hf']['delegation_locking_period'],
                             'DagGasLimit': integer(h['cornus_hf']['dag_gas_limit']),
                             'PbftGasLimit': integer(h['cornus_hf']['pbft_gas_limit'])},
                'SoleiroliaHf': {'BlockNum': h['soleirolia_hf']['block_num'],
                                 'TrxMinGasPrice': integer(h['soleirolia_hf']['trx_min_gas_price']),
                                 'TrxMaxGasLimit': integer(h['soleirolia_hf']['trx_max_gas_limit'])},
                'CactiHf': {''.join(w.title() for w in k.split('_')) if k != 'block_num' else 'BlockNum': v
                            for k, v in h['cacti_hf'].items()}}}


if __name__ == '__main__':
    print(json.dumps({'policy': profile(), 'scenarios': scenarios()}, indent=2))
