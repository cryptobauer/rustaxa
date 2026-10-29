"""Bounded historical action inputs; no expected state, roots or outcomes.

The caller supplies the checked mainnet policy, existing scenario list and the
existing frame_matrix function. This pure transformation does not read files,
launch references or change the numeric policy. Donor gaps are explicit fixture
rebases, never replay claims. See mainnet_era_reference.go:eraRebasedSeed and
FinalChainSyntheticDonor's same-era check.
"""
from copy import deepcopy


DPOS = '00' * 19 + 'fe'
PRINCIPAL = str(10**24)  # eraConfig's actual synthetic owner delegation.
DEPOSIT = str(10**21)  # Real mainnet minimum deposit; no shortened policy.


def validate_constant_pending_domain(row, policy):
    """Check the Go pending evaluator's rule and donor bounds before launch.

    This checks declared coordinates only. Actual jail expiry, reader contents
    and constant eligibility still require the unchanged Go runtime checks.
    """
    pending = [period for period in row['pending_periods']
               if period <= row['first_retained']]
    if not pending:
        return None
    delay = int(policy['dpos']['delegation_delay'])
    low = max(0, pending[0]-1-delay)
    high = pending[-1]-1
    hf = policy['hardforks']
    boundaries = [hf['fix_redelegate_block_num'], hf['magnolia_hf']['block_num'],
                  hf['phalaenopsis_hf_block_num'], hf['fix_claim_all_block_num'],
                  hf['aspen_hf']['block_num_part_one'], hf['aspen_hf']['block_num_part_two'],
                  hf['ficus_hf']['block_num'], hf['cornus_hf']['block_num'],
                  hf['cacti_hf']['block_num'],
                  *map(int, hf['rewards_distribution_frequency'])]
    if any(low < boundary <= high for boundary in boundaries):
        raise ValueError('constant synthetic pending domain crosses a rule activation')
    if row.get('donor') and row['donor']['periods'][-1] >= low:
        raise ValueError('donor does not predate earliest pending reader coordinate')
    return low, high


def apply_historical_actions(windows, policy, frame_matrix):
    """Return scenarios with four source-backed signed workloads and donors.

    All donor nodes have distinct names. Each node occurs once in the returned
    list and may also occur as its consumer's nested donor. Count logical signed
    actions once per name, not once per serialization of a nested donor.
    """
    result = deepcopy(windows)
    by_name = {row['name']: row for row in result}
    required = ('phalaenopsis', 'claim_all_correction', 'ficus', 'cornus')
    if any(name not in by_name for name in required):
        raise ValueError('four existing historical scenarios are required')
    if any(by_name[name].get('actions') or by_name[name].get('donor') for name in required):
        raise ValueError('historical action plans must not replace existing work')
    delay = int(policy['dpos']['delegation_delay'])
    old_lock = int(policy['dpos']['delegation_locking_period'])
    v2_lock = int(policy['hardforks']['cornus_hf']['delegation_locking_period'])
    if delay != 5 or old_lock != 700540 or v2_lock != 163459:
        raise ValueError('mainnet delay/locking inputs changed')
    if int(policy['dpos']['minimum_deposit']) != int(DEPOSIT):
        raise ValueError('mainnet minimum deposit changed')

    def pending(head):
        rules = policy['hardforks']['rewards_distribution_frequency']
        start = max((int(p) for p in rules if int(p) <= head), default=0)
        frequency = int(rules.get(str(start), 1))
        return [] if frequency == 1 or head % frequency == 0 else list(
            range(max(start, head - head % frequency + 1), head + 1))

    def action(period, who, name, method, **fields):
        return dict(period=period, who=who, name=name, method=method, **fields)

    def raw(period, who, name, data, **fields):
        return action(period, who, name, 'raw', to=DPOS, input=data, **fields)

    def donor(name, first, last, actions, source=None):
        row = dict(name=name, head=first-1, first_retained=first-delay,
                   periods=list(range(first, last+1)), pending_periods=pending(first-1),
                   seed_actions_only=True, no_warmup_delegation=True, actions=actions)
        if source is not None:
            row['donor'] = source
        if name in by_name:
            raise ValueError('duplicate donor name')
        result.append(row)
        by_name[name] = row
        return row

    def configure(name, actions, source):
        row = by_name[name]
        boundary = row['head'] + 2
        row.update(donor=source, actions=actions, no_warmup_delegation=True,
                   periods=list(range(boundary-1, boundary+6)),
                   required_activation_periods=[boundary-1, boundary, boundary+1],
                   api_heads=[boundary-1, boundary, boundary+1, boundary+4, boundary+5])
        return boundary

    # Real signed ordinary transfers exercise each boundary's fee economics.
    # Signer4 also makes the API requests. Their effective nonce and balances
    # come from the independently captured state after these transactions.
    # Keep the original warmup delegation and all declared window coordinates.
    for family in ('magnolia', 'aspen_one', 'aspen_two', 'soleirolia'):
        if family not in by_name:
            continue
        row = by_name[family]
        if row.get('actions') or row.get('donor'):
            raise ValueError('fee boundary actions must not replace existing work')
        boundary = row['head'] + 2
        row['actions'] = [action(period, 4, f'{family}_fee_{phase}', 'raw',
                                 to='c48b812bb43401392c037381aca934f4069c0517',
                                 input='', value='1',
                                 gas=(31500000 if family == 'soleirolia' and period == boundary else 21000),
                                 gas_price=1000000000)
                          for period, phase in [(boundary-1, 'before'),
                                                (boundary, 'activation'),
                                                (boundary+1, 'after')]]

    # CALL forwards exact calldata and CALLVALUE to the real DPoS account; the
    # parent's revert tests ordinary frame rollback of value and native effects.
    # DPoS ContractAddress is 0xfe (dpos_contract.go), not slashing's 0xee.
    revert_dpos = '366000600037600060003660003460fe620493e0f15060006000fd'

    # dpos_contract.go:claimCommissionRewards deliberately omits the surviving
    # zero-stake reward write before Phalaenopsis. A real distribution creates
    # commission, and a real full withdrawal creates the pending V1/count/escrow.
    b = by_name['phalaenopsis']['head'] + 2
    # Donor must also precede the earliest p-6 reader for the next
    # unfinished reward batch; a donor inside that batch is insufficient.
    r = b-200
    source = donor('phalaenopsis_commission_donor', r-1, r+1, [
        action(r+1, 0, 'full_rewarded_owner_withdrawal', 'undelegate',
               validator=0, amount=PRINCIPAL)])
    actions = [action(b-1, 0, 'create_transfer_parent_revert', 'create', input=revert_dpos)]
    for p in (b-1, b, b+1):
        actions += [
            action(p, 0, 'zero_stake_owner_commission', 'commission', validator=0),
            raw(p, 4, 'exact_payable_transfer', '44df8e70', value='7'),
            raw(p, 4, 'transfer_extra_byte', '44df8e7000', value='7'),
            action(p, 4, 'transfer_parent_revert', 'call', created_by_key=0,
                   created_nonce=1, input='44df8e70', value='7'),
        ]
    configure('phalaenopsis', actions, source)
    by_name['phalaenopsis']['api_ordinary_nonce'] = 1

    # Old claim-all ABI has a uint32 batch; preserve literal selector and exact
    # argument word (dpos_hardforks.go:GetOldClaimAllRewardsABI). Delegations are
    # inserted in validator order, then a real full withdrawal removes the tail.
    b = by_name['claim_all_correction']['head'] + 2
    r = b-200
    source = donor('claim_all_membership_donor', r-6, r+1, [
        *[action(r-6, 4, 'ordered_delegate_'+str(v), 'delegate', validator=v,
                 value=DEPOSIT) for v in range(3)],
        action(r+1, 4, 'remove_tail_membership', 'undelegate', validator=2,
               amount=DEPOSIT),
    ])
    actions = [action(b-1, 0, 'create_nested_claim_parent_revert', 'create', input=revert_dpos)]
    for p in (b-1, b, b+1):
        old_zero = '09b72e00' + '00'*32
        actions += [
            raw(p, 4, 'old_batch_zero', old_zero),
            raw(p, 4, 'old_batch_one_empty_tail', '09b72e00' + (1).to_bytes(32, 'big').hex()),
            action(p, 4, 'full_claim_route', 'claimAll'),
            raw(p, 4, 'old_batch_truncated', '09b72e00'),
            action(p, 4, 'full_claim_with_value', 'claimAll', value='1'),
            action(p, 4, 'nested_old_claim_parent_revert', 'call', created_by_key=0,
                   created_nonce=0, input=old_zero),
        ]
    configure('claim_all_correction', actions, source)
    by_name['claim_all_correction']['api_ordinary_nonce'] = 0

    # Two genuine V1 entries belong to different delegators of one validator.
    # Their total withdrawals leave stake zero and commission nonzero. Keeping
    # the second entry makes pre-Ficus omitted count persistence observable.
    # dpos_contract.go:confirmUndelegate only writes a surviving validator at B.
    b = by_name['ficus']['head'] + 2
    creation = b-1-old_lock
    r = ((creation-10)//100)*100-100
    reward_source = donor('ficus_reward_donor', r-6, r, [
        action(r-6, 4, 'second_custody_delegator', 'delegate', validator=0, value=DEPOSIT)])
    source = donor('ficus_custody_donor', creation, creation+1, [
        action(creation, 0, 'full_owner_v1_queue', 'undelegate', validator=0, amount=PRINCIPAL),
        action(creation+1, 4, 'full_second_v1_queue', 'undelegate', validator=0, amount=DEPOSIT),
    ], reward_source)
    # MCOPY copies one byte from offset0 to32. Pre-B execution supplies its actual
    # invalid-instruction outcome. Registry code is the existing full frame matrix.
    bls_revert = '6000600061010060006000600b620186a0f15060006000fd'
    actions = [
        action(b-1, 0, 'create_ficus_registry_matrix', 'create', input=frame_matrix(), gas=2000000),
        action(b-1, 0, 'create_mcopy_probe', 'create', input='60016000536001600060205e60216000f3'),
        action(b-1, 0, 'create_bls_parent_revert', 'create', input=bls_revert),
        action(b-1, 0, 'first_mature_v1_decreases_surviving_count', 'confirm', validator=0),
        action(b-1, 4, 'second_v1_one_period_locked', 'confirm', validator=0),
        action(b, 4, 'second_mature_v1_after_ficus', 'confirm', validator=0),
        action(b-1, 1, 'pending_custody_for_redelegation', 'undelegate', validator=1, amount=DEPOSIT),
        action(b-1, 1, 'redelegate_with_pending_custody', 'reDelegate', validator=1,
               target_validator=2, amount=DEPOSIT),
        action(b+1, 0, 'nonexistent_v1_confirmation', 'confirm', validator=0),
        action(b+1, 4, 'redelegate_without_remaining_principal', 'reDelegate', validator=0,
               target_validator=1, amount=DEPOSIT),
    ]
    for p in (b-1, b, b+1):
        for nonce, name, gas in ((1, 'registry_frame_matrix', 10000000),
                                 (2, 'mcopy_activation', 500000),
                                 (3, 'bls_parent_revert', 500000)):
            actions.append(action(p, 4, name, 'call', created_by_key=0, created_nonce=nonce, gas=gas))
    configure('ficus', actions, source)
    by_name['ficus'].update(api_ordinary_nonce=2, ordinary_api_name='ordinary_instruction_boundary')

    # Mature V1 at B-1 and one-period-locked V1 mature at B were actually created
    # under the old700540 lock. No pre-Cornus V2 record is materialized.
    b = by_name['cornus']['head'] + 2
    creation = b-1-old_lock
    source = donor('cornus_old_v1_donor', creation, creation+1, [
        action(creation, 0, 'old_mature_v1_creation', 'undelegate', validator=0, amount=DEPOSIT),
        action(creation+1, 1, 'old_just_locked_v1_creation', 'undelegate', validator=1, amount=DEPOSIT),
    ])
    actions = [
        action(b-1, 0, 'create_cornus_ordinary_control', 'create', input='600160005560006000f3'),
        action(b-1, 0, 'old_lock_mature_v1', 'confirm', validator=0),
        action(b-1, 1, 'old_lock_v1_one_period_locked', 'confirm', validator=1),
        action(b, 1, 'old_lock_v1_matures_at_boundary', 'confirm', validator=1),
        action(b-1, 0, 'inactive_v2_creation', 'undelegateV2', validator=0, amount=DEPOSIT),
        action(b-1, 0, 'inactive_v2_cancellation', 'cancelV2', validator=0, id=1),
        action(b, 0, 'new_v2_id_one', 'undelegateV2', validator=0, amount=DEPOSIT),
        action(b, 0, 'new_v2_one_not_mature', 'confirmV2', validator=0, id=1),
        action(b, 0, 'cancel_new_v2_one', 'cancelV2', validator=0, id=1),
        action(b+1, 0, 'new_v2_id_two', 'undelegateV2', validator=0, amount=DEPOSIT),
        action(b+1, 0, 'new_v2_two_not_mature', 'confirmV2', validator=0, id=2),
        action(b+1, 0, 'cancel_new_v2_two', 'cancelV2', validator=0, id=2),
        action(b+1, 0, 'v2_nonpayable_cancellation', 'cancelV2', validator=0, id=2, value='1'),
    ]
    configure('cornus', actions, source)
    by_name['cornus']['api_ordinary_nonce'] = 1

    # Confirmation coverage needs a separate same-era bounded donor, because a
    # V2 created at B cannot mature inside B±1. The real lock stays163459.
    # Keep the first unfinished batch's p-1-delay reader after Cornus too.
    # B+10 would reuse a constant logical seed across the B activation.
    created = b+110
    source = donor('cornus_live_v2_donor', created, created+1, [
        action(created, 0, 'real_post_cornus_v2_creation', 'undelegateV2', validator=0, amount=DEPOSIT),
        action(created+1, 0, 'create_maturity_ordinary_control', 'create', input='600160005560006000f3')])
    mature = created+v2_lock
    name = 'cornus_v2_maturity'
    row = dict(name=name, head=mature-2, first_retained=mature-6,
               periods=list(range(mature-1, mature+6)), pending_periods=pending(mature-2),
               no_warmup_delegation=True, donor=source,
               api_ordinary_nonce=1,
               api_heads=[mature-1, mature, mature+1, mature+5], actions=[
                   action(mature-1, 0, 'real_v2_still_locked', 'confirmV2', validator=0, id=1),
                   action(mature, 0, 'real_v2_mature_confirmation', 'confirmV2', validator=0, id=1),
                   action(mature+1, 0, 'real_v2_duplicate_confirmation', 'confirmV2', validator=0, id=1)])
    if name in by_name:
        raise ValueError('duplicate V2 maturity scenario')
    result.append(row)

    # Pure input invariants, not execution acceptance. Do not let profile edits
    # silently introduce cross-era rebases, long donor loops or extra actions.
    hf = policy['hardforks']
    boundaries = [hf['fix_redelegate_block_num'], hf['magnolia_hf']['block_num'],
                  hf['phalaenopsis_hf_block_num'], hf['fix_claim_all_block_num'],
                  hf['aspen_hf']['block_num_part_one'], hf['aspen_hf']['block_num_part_two'],
                  hf['ficus_hf']['block_num'], hf['cornus_hf']['block_num'],
                  hf['soleirolia_hf']['block_num'], hf['cacti_hf']['block_num']]
    new_names = set(required) | {'cornus_v2_maturity', 'aspen_one', 'aspen_two', 'soleirolia'} | {
        'phalaenopsis_commission_donor', 'claim_all_membership_donor',
        'ficus_reward_donor', 'ficus_custody_donor', 'cornus_old_v1_donor', 'cornus_live_v2_donor'}
    for row in result:
        if row['name'] not in new_names:
            continue
        validate_constant_pending_domain(row, policy)
        for a in row['actions']:
            if a['period'] not in row['periods']:
                raise ValueError('action outside its actual execution periods')
        parent = row
        depth = 0
        while parent.get('donor'):
            child = parent['donor']
            depth += 1
            final = child['periods'][-1]
            target = parent['first_retained']
            if depth > 3 or final >= target or any((final >= p) != (target >= p) for p in boundaries):
                raise ValueError('donor must precede retained history within identical active eras')
            parent = child
    if sum(len(row['actions']) for row in result if row['name'] in new_names) > 100:
        raise ValueError('historical action helper exceeds100 new logical transactions')
    return result


def legacy_batch_correction_component(original):
    """Synthetic additive component; no inferred poststate or Go outcome."""
    if original['name'] != 'claim_all_correction':
        raise ValueError('expected original claim-all correction input')
    row = deepcopy(original)
    row['name'] = 'claim_all_legacy_batch_correction'
    actions = []
    for action in row['actions']:
        if action['name'] == 'full_claim_route':
            continue
        if action['name'] == 'full_claim_with_value':
            action = dict(action, name='old_batch_zero_with_value', method='raw',
                          to=DPOS, input='09b72e00' + '00'*32)
        actions.append(action)
    if len(actions) != 16 or len(row['donor']['actions']) != 4:
        raise ValueError('unexpected component action shape')
    row['actions'] = actions
    return row
