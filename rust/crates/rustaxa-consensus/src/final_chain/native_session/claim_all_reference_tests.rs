//! Focused binding and scope rejection for the finite positive claim-all path.
//! Synthetic semantic fixtures exercise rejection; public integration owns Go
//! measurement parity and full physical-reader fault coverage.
use super::*;
use rustaxa_storage::{Config, Storage};
use rustaxa_types::{GenesisValidator, GenesisValidatorMetadata};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
const DELEGATOR: [u8; 20] = [0xd1; 20];
const OWNER: [u8; 20] = [0xa1; 20];
const VALIDATOR: [u8; 20] = [0x31; 20];
const SECOND: [u8; 20] = [0x32; 20];
type RawRows = BTreeMap<([u8; 20], [u8; 32]), ConcreteRead<Vec<u8>>>;

struct ReferenceState {
    rows: RefCell<RawRows>,
    accounts: RefCell<BTreeMap<[u8; 20], FinalChainNativeAccount>>,
}

/// Renamed semantic corpus for the measured preloaded underfunded profile.
/// Public integration supplies the independent authenticated canonical P1.
fn underfunded_session(chain: &FinalChain, historical: bool) -> FinalChainNativeSession<'_> {
    let mut s = if historical {
        historical_session(chain)
    } else {
        session(chain)
    };
    s.dpos_state.aspen_supply_state = AspenSupplyState::Unmigrated {
        minted_tokens: StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(800)),
    };
    s
}

struct CountOnlyState<'a> {
    inner: &'a ReferenceState,
    count_key: ConcreteStorageKey,
    reads: RefCell<Vec<ConcreteStorageKey>>,
}
impl FinalChainNativeStateRead for CountOnlyState<'_> {
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("underfunded scalar account read")
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        assert_eq!(*key, self.count_key);
        assert!(
            self.reads.borrow().is_empty(),
            "underfunded invocation count re-read"
        );
        self.reads.borrow_mut().push(*key);
        self.inner.raw_storage(address, key)
    }
}

#[test]
fn underfunded_claim_all_binds_count_and_consumes_without_scalar_reads() {
    with_claim_all_chain("underfunded-binding", |chain| {
        for historical in [false, true] {
            for gas in [0, 1, 68728, 89999] {
                let mut s = underfunded_session(chain, historical);
                let original = s.dpos_state.clone();
                let inner = ReferenceState::from_snapshot(&original);
                let state = CountOnlyState {
                    inner: &inner,
                    count_key: super::custody::iterable_count_key(
                        &super::custody::delegator_validators_prefix(DELEGATOR),
                    ),
                    reads: RefCell::new(Vec::new()),
                };
                let mut r = request();
                r.period = s.pending_period;
                r.supplied_gas = gas.into();
                let q = s.prepare(&r, &state).unwrap();
                assert_eq!(q.required_gas.as_u64(), 90000);
                let mut wrong = r.clone();
                wrong.supplied_gas = (gas + 1).into();
                assert_eq!(
                    s.invoke(&wrong, q, &state),
                    Err(FinalChainNativeSessionError::QuoteMismatch)
                );
                let mut wrong_q = q;
                wrong_q.required_gas = 89999.into();
                assert_eq!(
                    s.invoke(&r, wrong_q, &state),
                    Err(FinalChainNativeSessionError::QuoteMismatch)
                );
                assert_eq!(
                    s.invoke(&r, q, &state),
                    Ok(FinalChainNativeInvocationResult::InsufficientGas {
                        required_gas: 90000.into()
                    })
                );
                assert_eq!(*state.reads.borrow(), vec![state.count_key]);
                assert_eq!(s.dpos_state, original);
                assert_eq!(s.next_sequence, 1);
                assert!(s.prepared.is_none() && !s.aborted);
                assert_eq!(
                    s.invoke(&r, q, &state),
                    Err(FinalChainNativeSessionError::NotPrepared)
                );
                r.id.sequence = 1;
                assert_eq!(
                    s.prepare(&r, &state),
                    Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
                );
                assert!(s.aborted);
            }
        }
    });
}

#[test]
fn underfunded_claim_all_rejects_excluded_preloaded_profiles_and_count_integrity() {
    with_claim_all_chain("underfunded-profile", |chain| {
        for historical in [false, true] {
            for kind in 0..23 {
                let mut s = underfunded_session(chain, historical);
                let mut r = request();
                r.period = s.pending_period;
                r.supplied_gas = 89999.into();
                match kind {
                    0 => s.dpos_state.total_vote_count += 1,
                    1 => {
                        s.dpos_state.aspen_supply_state = AspenSupplyState::Unmigrated {
                            minted_tokens: StoredDposTokenAmount::default(),
                        }
                    }
                    2 => {
                        s.dpos_state.aspen_supply_state = AspenSupplyState::Migrated {
                            total_supply: StoredDposTokenAmount::canonical_u256_after_mutation(
                                U256::from(800),
                            ),
                            current_yield: AspenYield(0),
                        }
                    }
                    3 => r.caller = OWNER,
                    4 => s.dpos_state.validator_order.swap(0, 1),
                    5 => s.dpos_state.delegation_ledger_history_complete = false,
                    6 => {
                        s.dpos_state
                            .delegations
                            .get_mut(&VALIDATOR)
                            .unwrap()
                            .remove(&DELEGATOR);
                    }
                    7 => {
                        s.dpos_state.delegator_rewards.insert(
                            SECOND,
                            StoredDposTokenAmount::canonical_u256_after_mutation(U256::one()),
                        );
                    }

                    8 => s.dpos_state.reward_reference_graph = DposRewardGraph::incomplete(),
                    9 => {
                        s.dpos_state
                            .validator_reward_per_stake
                            .insert(
                                VALIDATOR,
                                StoredDposRewardIndex::canonical_after_mutation(
                                    DposRewardIndex::zero(),
                                ),
                            )
;
                    }
                    10 => r.input.push(0),
                    11 => r.is_static = true,
                    12 => r.depth = 1,
                    13 => r.value = FinalChainNativeValue::new(BigUint::from(1u8)),
                    14 => {
                        s.next_sequence = 1;
                        r.id.sequence = 1;
                    }
                    15 => {
                        s.origin = if historical {
                            NativeSessionOrigin::PendingPeriod
                        } else {
                            NativeSessionOrigin::HistoricalSimulation
                        }
                    }
                    16 => {
                        s.dpos_state.total_stakes.remove(&SECOND);
                    }
                    17 => {
                        s.dpos_state.commission_rewards.insert(
                            VALIDATOR,
                            StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(9)),
                        );
                    }

                    18 => s.dpos_state.redelegate_same_validator_history_complete = false,
                    _ => {}
                }
                let before = s.dpos_state.clone();
                let state = ReferenceState::from_snapshot(
                    &underfunded_session(chain, historical).dpos_state,
                );
                let count = super::custody::iterable_count_key(
                    &super::custody::delegator_validators_prefix(DELEGATOR),
                );
                if kind >= 19 {
                    state.rows.borrow_mut().insert(
                        (DPOS_CONTRACT_ADDRESS, count.0),
                        match kind {
                            19 => ConcreteRead::Absent,
                            20 => ConcreteRead::Present(vec![0xff]),
                            21 => ConcreteRead::Present(1u32.to_le_bytes().to_vec()),
                            _ => ConcreteRead::Present(3u32.to_le_bytes().to_vec()),
                        },
                    );
                }
                let raw = state.rows.borrow().clone();
                assert!(
                    s.prepare(&r, &state).is_err(),
                    "profile{kind} historical{historical}"
                );
                assert!(s.aborted && s.prepared.is_none());
                assert_eq!(s.dpos_state, before);
                assert_eq!(*state.rows.borrow(), raw);
            }
        }
    });
}

#[test]
fn underfunded_claim_all_retains_selected_period_fork_guards() {
    for historical in [false, true] {
        for kind in 0..5 {
            let excluded = if historical { 2 } else { 3 };
            with_claim_all_chain_config(
                "underfunded-fork",
                |rewards, ficus| match kind {
                    0 => rewards.fix_claim_all_block_num = excluded.into(),
                    1 => rewards.aspen_part_one_period = excluded.into(),
                    2 => rewards.magnolia_period = excluded.into(),
                    3 => *ficus = excluded.into(),
                    4 => {
                        rewards.aspen_part_two_period = 1.into();
                        rewards.aspen_max_supply = U256::from(8000).into();
                    }
                    _ => unreachable!(),
                },
                |chain| {
                    let mut s = underfunded_session(chain, historical);
                    let state = ReferenceState::from_snapshot(&s.dpos_state);
                    let before = s.dpos_state.clone();
                    let mut r = request();
                    r.period = s.pending_period;
                    r.supplied_gas = 89999.into();
                    assert!(s.prepare(&r, &state).is_err());
                    assert!(s.aborted && s.prepared.is_none());
                    assert_eq!(s.dpos_state, before);
                },
            );
        }
    }
}

impl ReferenceState {
    fn from_snapshot(snapshot: &DposSnapshot) -> Self {
        let rows = canonical_concrete_precompile_storage(snapshot, true)
            .expect("claims snapshot has canonical raw rows")
            .into_iter()
            .map(|(identity, candidates)| {
                (
                    identity,
                    ConcreteRead::Present(
                        candidates
                            .into_iter()
                            .next()
                            .expect("canonical raw row has an encoding"),
                    ),
                )
            })
            .collect();
        let account = |balance| FinalChainNativeAccount {
            exists: true,
            nonce: FinalChainNonce::zero(),
            balance: BigInt::from(balance),
        };
        Self {
            rows: RefCell::new(rows),
            accounts: RefCell::new(BTreeMap::from([
                (DPOS_CONTRACT_ADDRESS, account(1_000_u64)),
                (DELEGATOR, account(1_000_u64)),
                (OWNER, account(1_000_u64)),
            ])),
        }
    }
}
impl FinalChainNativeStateRead for ReferenceState {
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.accounts
            .borrow()
            .get(&address)
            .cloned()
            .ok_or_else(|| {
                FinalChainNativeStateReadError::Invariant(format!(
                    "claims corpus account is unavailable: {address:?}"
                ))
            })
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        Ok(self
            .rows
            .borrow()
            .get(&(address, key.0))
            .cloned()
            .unwrap_or(ConcreteRead::Absent))
    }
}

fn temp_db_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustaxa-consensus-claims-corpus-{test_name}-{}-{nanos}",
        std::process::id()
    ))
}

fn with_claim_all_chain(test_name: &str, test: impl FnOnce(&FinalChain)) {
    with_claim_all_chain_config(test_name, |_, _| {}, test);
}

fn with_claim_all_chain_config(
    test_name: &str,
    configure: impl FnOnce(&mut FinalChainRewardsConfig, &mut FinalChainBlockNumber),
    test: impl FnOnce(&FinalChain),
) {
    let path = temp_db_path(test_name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let mut rewards = FinalChainRewardsConfig {
        magnolia_period: FinalChainBlockNumber::GENESIS,
        fix_claim_all_block_num: FinalChainBlockNumber::GENESIS,
        fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
        dpos_blocks_per_year: 1,
        dpos_delegation_locking_period: 2,
        cornus_period: FinalChainBlockNumber::GENESIS,
        cornus_delegation_locking_period: 3,
        cacti_period: FinalChainBlockNumber::MAX,
        cacti_delegation_locking_period: 2,
        aspen_part_one_period: FinalChainBlockNumber::GENESIS,
        aspen_part_two_period: FinalChainBlockNumber::MAX,
        committee_size: 1,
        rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
        yield_percentage: 20,
        max_block_author_reward_percent: 0,
        dag_proposers_reward_percent: 0,
        ..Default::default()
    };
    let mut ficus = FinalChainBlockNumber::GENESIS;
    configure(&mut rewards, &mut ficus);
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        Vec::new(),
        [VALIDATOR, SECOND]
            .into_iter()
            .map(|validator| GenesisValidator {
                address: validator,
                vrf_key: [0x44; 32],
                total_stake: U256::from(2_000_u64).to_big_endian().to_vec(),
                delegations: vec![
                    (OWNER, U256::from(1000).to_big_endian().to_vec()),
                    (DELEGATOR, U256::from(1000).to_big_endian().to_vec()),
                ],
                metadata: GenesisValidatorMetadata {
                    owner: OWNER,
                    commission: 100,
                    ..Default::default()
                },
            })
            .collect(),
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100_u64).into(),
            vote_eligibility_balance_step: U256::from(10_u64).into(),
            validator_maximum_stake: U256::from(1_000_000_u64).into(),
            minimum_deposit: U256::one().into(),
            delegation_delay: 1,
            ..Default::default()
        },
        rewards,
        ficus,
    )
    .unwrap();
    test(&chain);
    drop(chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
}

fn session(chain: &FinalChain) -> FinalChainNativeSession<'_> {
    let mut s = chain.begin_native_session(1.into(), 0.into()).unwrap();
    chain
        .advance_reward_reference_graph_block(&mut s.dpos_state, 2.into())
        .unwrap();
    s.dpos_state.delegator_rewards.insert(
        VALIDATOR,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(792)),
    );
    s.dpos_state.commission_rewards.insert(
        VALIDATOR,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(8)),
    );
    s.pending_period = 2.into();
    s
}
fn request() -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: FinalChainTransactionPosition::new(0),
            sequence: 0,
        },
        period: 2.into(),
        depth: 0,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: DELEGATOR,
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::default(),
        input: DPOS_CLAIM_ALL_REWARDS_SELECTOR.to_vec(),
        supplied_gas: 90000.into(),
    }
}
#[test]
fn claim_all_binding_and_scope_reject_without_partial_effects() {
    with_claim_all_chain("binding", |chain| {
        // A renamed structural seed succeeds. Mismatched invocation/quote
        // retains the valid preparation; the exact request alone can consume it.
        let mut positive = session(chain);
        let state = ReferenceState::from_snapshot(&positive.dpos_state);
        let r = request();
        let quote = positive.prepare(&r, &state).unwrap();
        let mut wrong = r.clone();
        wrong.input.push(0);
        assert_eq!(
            positive.invoke(&wrong, quote, &state),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        let mut wrong_quote = quote;
        wrong_quote.required_gas = 89999.into();
        assert_eq!(
            positive.invoke(&r, wrong_quote, &state),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        let FinalChainNativeInvocationResult::Completed(out) =
            positive.invoke(&r, quote, &state).unwrap()
        else {
            panic!("funded")
        };
        assert_eq!(out.status, FinalChainNativeStatus::Success);
        assert_eq!(
            (
                out.raw_mutations.len(),
                out.account_mutations.len(),
                out.logs.len()
            ),
            (12, 2, 1)
        );
        let mut repeat = r.clone();
        repeat.id.sequence = 1;
        assert_eq!(
            positive.prepare(&repeat, &state),
            Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
        );
        assert!(positive.aborted);
        for kind in 0..17 {
            let mut s = session(chain);
            let mut r = request();
            match kind {
                2 => {
                    s.dpos_state.validator_order.swap(0, 1);
                }
                3 => {
                    s.dpos_state
                        .delegator_validators
                        .get_mut(&DELEGATOR)
                        .unwrap()[1] = VALIDATOR;
                }
                4 => {
                    r.caller = OWNER;
                }
                5 => {
                    s.next_sequence = 1;
                    r.id.sequence = 1;
                }
                6 => {
                    s.dpos_state.delegation_ledger_history_complete = false;
                }
                7 => {
                    s.dpos_state.delegator_rewards.insert(
                        SECOND,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::one()),
                    );
                }
                8 => {
                    s.dpos_state.reward_reference_graph = DposRewardGraph::incomplete();
                }
                9 => {
                    r.input.push(0);
                }
                10 => {
                    r.depth = 1;
                }
                11 => {
                    r.is_static = true;
                }
                12 => {
                    r.value = FinalChainNativeValue::new(BigUint::from(1u8));
                }
                13 => {
                    r.supplied_gas = 89999.into();
                }
                14 => {
                    s.origin = NativeSessionOrigin::HistoricalSimulation;
                }
                15 => {
                    s.dpos_state.reward_reference_graph = DposRewardGraph::new();
                }
                16 => {
                    s.dpos_state.validator_reward_per_stake.insert(
                        VALIDATOR,
                        StoredDposRewardIndex::canonical_after_mutation(DposRewardIndex::zero()),
                    );
                }
                _ => {}
            }
            let state = ReferenceState::from_snapshot(&session(chain).dpos_state);
            let before = s.dpos_state.clone();
            let raw = state.rows.borrow().clone();
            let prepared = s.prepare(&r, &state);
            if let Ok(q) = prepared {
                assert_eq!(q.required_gas.as_u64(), 90000);
                if kind == 0 {
                    let key = super::custody::iterable_count_key(
                        &super::custody::delegator_validators_prefix(DELEGATOR),
                    );
                    state.rows.borrow_mut().insert(
                        (DPOS_CONTRACT_ADDRESS, key.0),
                        ConcreteRead::Present(1u32.to_le_bytes().to_vec()),
                    );
                }
                if kind == 1 {
                    let prefix = super::custody::delegator_validators_prefix(DELEGATOR);
                    let key = super::custody::iterable_item_key(&prefix, 1);
                    state.rows.borrow_mut().insert(
                        (DPOS_CONTRACT_ADDRESS, key.0),
                        ConcreteRead::Present(SECOND.to_vec()),
                    );
                }
                assert!(s.invoke(&r, q, &state).is_err(), "excluded {kind}");
            } else {
                assert!(s.aborted);
            }
            assert_eq!(s.dpos_state, before);
            assert_eq!(
                s.prepare(&r, &state),
                Err(FinalChainNativeSessionError::Aborted)
            );
            if kind > 1 {
                assert_eq!(*state.rows.borrow(), raw);
            }
        }
        // An unexpected scalar affordability failure also returns no outcome.
        let mut s = session(chain);
        let r = request();
        let state = ReferenceState::from_snapshot(&s.dpos_state);
        state
            .accounts
            .borrow_mut()
            .get_mut(&DPOS_CONTRACT_ADDRESS)
            .unwrap()
            .balance = BigInt::from(0);
        let before = s.dpos_state.clone();
        let q = s.prepare(&r, &state).unwrap();
        assert!(s.invoke(&r, q, &state).is_err());
        assert_eq!(s.dpos_state, before);
        assert!(s.aborted);
    });
}

/// Synthetic renamed H1 topology is constructed at graph1 without regressing a
/// graph2 view. Public integration separately owns exact finalized P1 authority.
fn historical_session(chain: &FinalChain) -> FinalChainNativeSession<'_> {
    let mut s = chain.begin_native_session(1.into(), 0.into()).unwrap();
    s.origin = NativeSessionOrigin::HistoricalSimulation;
    s.dpos_state.delegator_rewards.insert(
        VALIDATOR,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(792)),
    );
    s.dpos_state.commission_rewards.insert(
        VALIDATOR,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(8)),
    );
    s
}

#[test]
fn historical_claim_all_binds_exact_origin_period_graph_and_guards() {
    with_claim_all_chain("h1-binding", |chain| {
        let mut s = historical_session(chain);
        let state = ReferenceState::from_snapshot(&s.dpos_state);
        let mut r = request();
        r.period = 1.into();
        let q = s.prepare(&r, &state).unwrap();
        let mut wrong = q;
        wrong.required_gas = 89999.into();
        assert_eq!(
            s.invoke(&r, wrong, &state),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        let mut wrong_request = r.clone();
        wrong_request.input.push(0);
        assert_eq!(
            s.invoke(&wrong_request, q, &state),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        let FinalChainNativeInvocationResult::Completed(out) = s.invoke(&r, q, &state).unwrap()
        else {
            panic!("funded H1")
        };
        assert_eq!(out.status, FinalChainNativeStatus::Success);
        assert_eq!(
            (
                out.raw_mutations.len(),
                out.account_mutations.len(),
                out.logs.len()
            ),
            (12, 2, 1)
        );
        assert_eq!(
            s.dpos_state.reward_reference_graph.current_block().unwrap(),
            1
        );
        assert_eq!(
            s.dpos_state
                .reward_reference_graph
                .read_validator_head(&VALIDATOR)
                .unwrap(),
            1
        );
        assert_eq!(
            s.dpos_state
                .reward_reference_graph
                .read_validator_head(&SECOND)
                .unwrap(),
            1
        );
        let mut repeat = r.clone();
        repeat.id.sequence = 1;
        assert_eq!(
            s.prepare(&repeat, &state),
            Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
        );
        assert!(s.aborted);
        for kind in 0..21 {
            let mut s = historical_session(chain);
            let mut r = request();
            r.period = 1.into();
            match kind {
                0 => s.origin = NativeSessionOrigin::PendingPeriod,
                1 => {
                    s.pending_period = 2.into();
                    r.period = 2.into();
                }
                2 => {
                    s.pending_period = 3.into();
                    r.period = 3.into();
                }
                3 => chain
                    .advance_reward_reference_graph_block(&mut s.dpos_state, 2.into())
                    .unwrap(),
                4 => {
                    s.origin = NativeSessionOrigin::PendingPeriod;
                    s.pending_period = 2.into();
                    r.period = 2.into();
                }
                5 => r.is_static = true,
                6 => r.depth = 1,
                7 => r.kind = FinalChainNativeCallKind::DelegateCall,
                8 => r.value = FinalChainNativeValue::new(BigUint::from(1u8)),
                9 => r.input.push(0),
                10 => r.supplied_gas = 89999.into(),
                11 => {
                    s.next_sequence = 1;
                    r.id.sequence = 1;
                }
                12 => r.caller = OWNER,
                13 => s.dpos_state.validator_order.swap(0, 1),
                14 => {
                    s.dpos_state.delegation_ledger_history_complete = false;
                }
                15 => {
                    s.dpos_state.validator_reward_per_stake.insert(
                        VALIDATOR,
                        StoredDposRewardIndex::canonical_after_mutation(DposRewardIndex::zero()),
                    );
                }
                16 => {
                    s.dpos_state.reward_reference_graph = DposRewardGraph::incomplete();
                }
                20 => r.kind = FinalChainNativeCallKind::StaticCall,
                17 => {
                    s.dpos_state.delegator_rewards.insert(
                        SECOND,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::one()),
                    );
                }
                _ => {}
            }
            let state = ReferenceState::from_snapshot(&historical_session(chain).dpos_state);
            let before = s.dpos_state.clone();
            if kind == 18 {
                state
                    .accounts
                    .borrow_mut()
                    .get_mut(&DPOS_CONTRACT_ADDRESS)
                    .unwrap()
                    .balance = BigInt::from(0);
            }
            if kind == 19 {
                state
                    .accounts
                    .borrow_mut()
                    .get_mut(&DELEGATOR)
                    .unwrap()
                    .exists = false;
            }
            let raw = state.rows.borrow().clone();
            if kind == 7 {
                // The outer request validator rejects this shape before claim
                // preparation and leaves the session available for a valid call.
                assert_eq!(
                    s.prepare(&r, &state),
                    Err(FinalChainNativeSessionError::UnsupportedCallKind)
                );
                assert!(!s.aborted);
                assert!(s.prepared.is_none());
                assert_eq!(s.dpos_state, before);
                assert_eq!(*state.rows.borrow(), raw);
                r.kind = FinalChainNativeCallKind::Call;
                let q = s.prepare(&r, &state).unwrap();
                assert!(s.invoke(&r, q, &state).is_ok());
                continue;
            }
            if let Ok(q) = s.prepare(&r, &state) {
                assert!(s.invoke(&r, q, &state).is_err(), "H1 excluded {kind}");
            }
            assert!(s.aborted, "H1 excluded {kind}");
            assert_eq!(s.dpos_state, before);
            assert_eq!(*state.rows.borrow(), raw);
            assert_eq!(
                s.prepare(&r, &state),
                Err(FinalChainNativeSessionError::Aborted)
            );
        }
    });
    // Each required fork is evaluated at literal1, not the accepted pending2.
    for kind in 0..5 {
        with_claim_all_chain_config(
            "h1-fork",
            |rewards, ficus| match kind {
                0 => rewards.fix_claim_all_block_num = 2.into(),
                1 => rewards.aspen_part_one_period = 2.into(),
                2 => rewards.magnolia_period = 2.into(),
                3 => *ficus = 2.into(),
                4 => {
                    rewards.aspen_part_two_period = 1.into();
                    rewards.aspen_max_supply = U256::from(8000).into();
                }
                _ => unreachable!(),
            },
            |chain| {
                let mut s = historical_session(chain);
                let state = ReferenceState::from_snapshot(&s.dpos_state);
                let before = s.dpos_state.clone();
                let mut r = request();
                r.period = 1.into();
                assert!(s.prepare(&r, &state).is_err());
                assert!(s.aborted);
                assert_eq!(s.dpos_state, before);
            },
        );
    }
}

/// The trace constructor checks exact P0 before changing only its private context.
/// No ordinary account callback is permitted by the zero-reward scalar lane.
struct NoTraceAccounts<'a>(&'a ReferenceState);
impl FinalChainNativeStateRead for NoTraceAccounts<'_> {
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("H1/P0 native account read")
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        self.0.raw_storage(address, key)
    }
}

#[test]
fn trace_claim_all_binds_p0_context_and_zero_payout_without_account_reads() {
    with_claim_all_chain("trace-p0", |chain| {
        let source = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
        assert_eq!(source.reward_reference_graph.current_block().unwrap(), 0);
        let mut trace = chain
            .begin_native_trace_claim_all(1.into(), 0.into())
            .unwrap();
        assert_eq!(trace.session.origin, NativeSessionOrigin::HistoricalTrace);
        assert_eq!(
            trace
                .session
                .dpos_state
                .reward_reference_graph
                .current_block()
                .unwrap(),
            1
        );
        let state = ReferenceState::from_snapshot(&source);
        let port = NoTraceAccounts(&state);
        let mut r = request();
        r.period = 1.into();
        r.supplied_gas = 178728.into();
        let q = trace.prepare(&r, &port).unwrap();
        let mut wrong = r.clone();
        wrong.supplied_gas = 178727.into();
        assert_eq!(
            trace.invoke(&wrong, q, &port),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        let mut wrong_q = q;
        wrong_q.required_gas = 89999.into();
        assert_eq!(
            trace.invoke(&r, wrong_q, &port),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        let FinalChainNativeInvocationResult::Completed(out) = trace.invoke(&r, q, &port).unwrap()
        else {
            panic!("trace success")
        };
        assert_eq!(out.status, FinalChainNativeStatus::Success);
        assert_eq!(out.gas_used.as_u64(), 90000);
        assert!(out.output.is_empty() && out.logs.is_empty() && out.account_mutations.is_empty());
        assert_eq!(out.raw_mutations.len(), 12);
        for validator in [VALIDATOR, SECOND] {
            assert_eq!(
                trace
                    .session
                    .dpos_state
                    .reward_reference_graph
                    .read_validator_head(&validator)
                    .unwrap(),
                1
            );
            assert_eq!(
                trace
                    .session
                    .dpos_state
                    .reward_reference_graph
                    .read_cursor(&validator, &DELEGATOR)
                    .unwrap(),
                1
            );
            assert_eq!(
                trace
                    .session
                    .dpos_state
                    .reward_reference_graph
                    .read_cursor(&validator, &OWNER)
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            trace.invoke(&r, q, &port),
            Err(FinalChainNativeSessionError::NotPrepared)
        );
        r.id.sequence = 1;
        assert_eq!(
            trace.prepare(&r, &port),
            Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
        );
        drop(trace);
        assert_eq!(
            chain.dpos_snapshot_at_finalized_block(0.into()).unwrap(),
            source
        );
        for (execution, reader) in [(0, 0), (1, 1), (2, 0), (2, 1)] {
            assert!(matches!(
                chain.begin_native_trace_claim_all(execution.into(), reader.into()),
                Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
            ));
        }
    });
}

#[test]
fn trace_claim_all_rejects_unmeasured_requests_and_source_shapes() {
    with_claim_all_chain("trace-scope", |chain| {
        for kind in 0..28 {
            let mut trace = chain
                .begin_native_trace_claim_all(1.into(), 0.into())
                .unwrap();
            let mut r = request();
            r.period = 1.into();
            r.supplied_gas = 178728.into();
            match kind {
                0 => r.supplied_gas = 89999.into(),
                1 => r.supplied_gas = 90000.into(),
                2 => r.supplied_gas = 178727.into(),
                3 => r.supplied_gas = 178729.into(),
                4 => r.id.transaction = 1.into(),
                5 => r.input.push(0),
                6 => r.input = DPOS_REDELEGATE_SELECTOR.to_vec(),
                7 => r.input = vec![0; 4],
                8 => r.depth = 1,
                9 => r.is_static = true,
                10 => r.value = FinalChainNativeValue::new(BigUint::from(1u8)),
                11 => r.caller = OWNER,
                12 => trace.session.dpos_state.total_vote_count += 1,
                13 => {
                    trace.session.dpos_state.aspen_supply_state = AspenSupplyState::Unmigrated {
                        minted_tokens: StoredDposTokenAmount::canonical_u256_after_mutation(
                            U256::one(),
                        ),
                    }
                }
                14 => {
                    trace.session.dpos_state.delegator_rewards.insert(
                        VALIDATOR,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(792)),
                    );
                }
                15 => {
                    trace.session.dpos_state.commission_rewards.insert(
                        VALIDATOR,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(8)),
                    );
                }
                16 => trace.session.dpos_state.delegation_ledger_history_complete = false,
                17 => {
                    trace
                        .session
                        .dpos_state
                        .redelegate_same_validator_history_complete = false
                }
                18 => trace.session.dpos_state.validator_order.swap(0, 1),
                19 => {
                    trace.session.dpos_state.reward_reference_graph = DposRewardGraph::incomplete()
                }
                20 => {
                    trace
                        .session
                        .dpos_state
                        .delegation_reward_cursors
                        .get_mut(&VALIDATOR)
                        .unwrap()
                        .remove(&OWNER);
                }
                21 => {
                    trace
                        .session
                        .dpos_state
                        .delegator_rewards
                        .insert([0x99; 20], StoredDposTokenAmount::default());
                }
                22 => {
                    trace.session.dpos_state.aspen_supply_state = AspenSupplyState::Migrated {
                        total_supply: StoredDposTokenAmount::canonical_u256_after_mutation(
                            U256::from(8000),
                        ),
                        current_yield: AspenYield(0),
                    }
                }
                23 => {
                    trace.session.dpos_state.vote_counts.insert(VALIDATOR, 201);
                }
                24 => {
                    trace.session.dpos_state.vote_counts.insert([0x99; 20], 0);
                }
                25 => {
                    trace
                        .session
                        .dpos_state
                        .validator_metadata
                        .get_mut(&VALIDATOR)
                        .unwrap()
                        .commission = 101
                }
                26 => {
                    trace
                        .session
                        .dpos_state
                        .validator_metadata
                        .get_mut(&VALIDATOR)
                        .unwrap()
                        .last_commission_change = 1
                }
                27 => trace
                    .session
                    .dpos_state
                    .slashing_jailed_validators
                    .push(VALIDATOR),
                _ => unreachable!(),
            }
            let before = trace.session.dpos_state.clone();
            let raw = ReferenceState::from_snapshot(
                &chain.dpos_snapshot_at_finalized_block(0.into()).unwrap(),
            );
            assert!(
                trace.prepare(&r, &NoTraceAccounts(&raw)).is_err(),
                "trace excluded{kind}"
            );
            assert!(trace.session.aborted && trace.session.prepared.is_none());
            assert_eq!(trace.session.dpos_state, before);
        }
        // The original graph0 cannot contain extra rewards or fabricated graph1.
        let mut s = chain
            .native_session_from_state(
                None,
                1.into(),
                chain.dpos_snapshot_at_finalized_block(0.into()).unwrap(),
                NativeSessionOrigin::HistoricalTrace,
            )
            .unwrap();
        s.qualify_trace_claim_all_source().unwrap();
        chain
            .advance_reward_reference_graph_block(&mut s.dpos_state, 1.into())
            .unwrap();
        assert!(s.qualify_trace_claim_all_source().is_err());
    });
}

#[test]
fn trace_claim_all_retains_fork_guards() {
    for kind in 0..5 {
        with_claim_all_chain_config(
            "trace-fork",
            |rewards, ficus| match kind {
                0 => rewards.fix_claim_all_block_num = 2.into(),
                1 => rewards.aspen_part_one_period = 2.into(),
                2 => rewards.magnolia_period = 2.into(),
                3 => *ficus = 2.into(),
                4 => {
                    rewards.aspen_part_two_period = 1.into();
                    rewards.aspen_max_supply = U256::from(8000).into();
                }
                _ => unreachable!(),
            },
            |chain| {
                let mut trace = chain
                    .begin_native_trace_claim_all(1.into(), 0.into())
                    .unwrap();
                let before = trace.session.dpos_state.clone();
                let raw = ReferenceState::from_snapshot(&before);
                let mut r = request();
                r.period = 1.into();
                r.supplied_gas = 178728.into();
                assert!(trace.prepare(&r, &NoTraceAccounts(&raw)).is_err());
                assert!(trace.session.aborted);
                assert_eq!(trace.session.dpos_state, before);
            },
        );
    }
}

#[test]
fn trace_boundary_claim_all_consumes_exact_count_only_terminals() {
    with_claim_all_chain("trace-boundary-terminal", |chain| {
        let source = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
        for gas in [0, 1, 68728, 89999] {
            let mut trace = chain
                .begin_native_trace_claim_all_gas_boundaries(1.into(), 0.into())
                .unwrap();
            assert_eq!(
                trace.session.origin,
                NativeSessionOrigin::HistoricalTraceBoundary
            );
            let before = trace.session.dpos_state.clone();
            let inner = ReferenceState::from_snapshot(&source);
            let state = CountOnlyState {
                inner: &inner,
                count_key: super::custody::iterable_count_key(
                    &super::custody::delegator_validators_prefix(DELEGATOR),
                ),
                reads: RefCell::new(Vec::new()),
            };
            let mut r = request();
            r.period = 1.into();
            r.supplied_gas = gas.into();
            let q = trace.prepare(&r, &state).unwrap();
            assert_eq!(q.required_gas.as_u64(), 90000);
            let mut wrong = r.clone();
            wrong.supplied_gas = (gas + 2).into();
            assert_eq!(
                trace.invoke(&wrong, q, &state),
                Err(FinalChainNativeSessionError::QuoteMismatch)
            );
            let mut wrong_q = q;
            wrong_q.required_gas = 89999.into();
            assert_eq!(
                trace.invoke(&r, wrong_q, &state),
                Err(FinalChainNativeSessionError::QuoteMismatch)
            );
            assert_eq!(
                trace.invoke(&r, q, &state),
                Ok(FinalChainNativeInvocationResult::InsufficientGas {
                    required_gas: 90000.into()
                })
            );
            assert_eq!(*state.reads.borrow(), vec![state.count_key]);
            assert_eq!(trace.session.dpos_state, before);
            assert_eq!(trace.session.next_sequence, 1);
            assert_eq!(
                trace.invoke(&r, q, &state),
                Err(FinalChainNativeSessionError::NotPrepared)
            );
            r.id.sequence = 1;
            assert_eq!(
                trace.prepare(&r, &state),
                Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
            );
            assert!(trace.session.aborted);
        }
        assert_eq!(
            chain.dpos_snapshot_at_finalized_block(0.into()).unwrap(),
            source
        );
    });
}

#[test]
fn trace_boundary_claim_all_funded_caps_use_zero_payout_kernel() {
    with_claim_all_chain("trace-boundary-funded", |chain| {
        let source = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
        for gas in [90000, 90001] {
            let mut trace = chain
                .begin_native_trace_claim_all_gas_boundaries(1.into(), 0.into())
                .unwrap();
            let raw = ReferenceState::from_snapshot(&source);
            let mut r = request();
            r.period = 1.into();
            r.supplied_gas = gas.into();
            let q = trace.prepare(&r, &NoTraceAccounts(&raw)).unwrap();
            let FinalChainNativeInvocationResult::Completed(out) =
                trace.invoke(&r, q, &NoTraceAccounts(&raw)).unwrap()
            else {
                panic!("funded boundary")
            };
            assert_eq!(out.status, FinalChainNativeStatus::Success);
            assert_eq!(out.gas_used.as_u64(), 90000);
            assert_eq!(out.raw_mutations.len(), 12);
            assert!(
                out.account_mutations.is_empty() && out.logs.is_empty() && out.output.is_empty()
            );
        }
        assert_eq!(
            chain.dpos_snapshot_at_finalized_block(0.into()).unwrap(),
            source
        );
    });
}

#[test]
fn trace_boundary_claim_all_rejects_unmeasured_caps_and_nonzero_sources() {
    with_claim_all_chain("trace-boundary-exclusions", |chain| {
        for gas in [2, 68727, 68729, 89998, 90002, 178728] {
            let mut trace = chain
                .begin_native_trace_claim_all_gas_boundaries(1.into(), 0.into())
                .unwrap();
            let raw = ReferenceState::from_snapshot(&trace.session.dpos_state);
            let state = CountOnlyState {
                inner: &raw,
                count_key: super::custody::iterable_count_key(
                    &super::custody::delegator_validators_prefix(DELEGATOR),
                ),
                reads: RefCell::new(Vec::new()),
            };
            let mut r = request();
            r.period = 1.into();
            r.supplied_gas = gas.into();
            assert_eq!(
                trace.prepare(&r, &state),
                Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
            );
            assert!(state.reads.borrow().is_empty() && trace.session.aborted);
        }
        for kind in 0..8 {
            let mut trace = chain
                .begin_native_trace_claim_all_gas_boundaries(1.into(), 0.into())
                .unwrap();
            let raw = ReferenceState::from_snapshot(&trace.session.dpos_state);
            let state = CountOnlyState {
                inner: &raw,
                count_key: super::custody::iterable_count_key(
                    &super::custody::delegator_validators_prefix(DELEGATOR),
                ),
                reads: RefCell::new(Vec::new()),
            };
            let mut r = request();
            r.period = 1.into();
            r.supplied_gas = 0.into();
            match kind {
                0 => trace.session.dpos_state.total_vote_count = 401,
                1 => {
                    trace.session.dpos_state.aspen_supply_state = AspenSupplyState::Unmigrated {
                        minted_tokens: StoredDposTokenAmount::canonical_u256_after_mutation(
                            U256::from(800),
                        ),
                    }
                }
                2 => {
                    trace.session.dpos_state.delegator_rewards.insert(
                        VALIDATOR,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(792)),
                    );
                }
                3 => r.id.transaction = 1.into(),
                4 => r.is_static = true,
                5 => r.depth = 1,
                6 => r.value = FinalChainNativeValue::new(BigUint::from(1u8)),
                7 => r.input = vec![0; 4],
                _ => unreachable!(),
            }
            let before = trace.session.dpos_state.clone();
            assert!(trace.prepare(&r, &state).is_err(), "boundaryexcluded{kind}");
            assert!(state.reads.borrow().is_empty() && trace.session.aborted);
            assert_eq!(trace.session.dpos_state, before);
        }
    });
}

/// Qualified guard terminals must not manufacture count/bootstrap/account reads.
struct NoNonpayableReads;
impl FinalChainNativeStateRead for NoNonpayableReads {
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("nonpayable native account callback")
    }
    fn raw_storage(
        &self,
        _: [u8; 20],
        _: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        panic!("nonpayable native raw callback")
    }
}
fn nonpayable_session(chain: &FinalChain, historical: bool) -> FinalChainNativeSession<'_> {
    let mut s = underfunded_session(chain, historical);
    s.origin = NativeSessionOrigin::ClaimAllNonPayable;
    s
}

#[test]
fn nonpayable_claim_all_binds_read_free_zero_gas_terminal() {
    with_claim_all_chain("nonpayable-binding", |chain| {
        for historical in [false, true] {
            for gas in [0, 1, 68728, 89999, 90000, 90001] {
                let mut s = nonpayable_session(chain, historical);
                let original = s.dpos_state.clone();
                let mut r = request();
                r.period = s.pending_period;
                r.value = FinalChainNativeValue::new(1u8.into());
                r.supplied_gas = gas.into();
                let q = s.prepare(&r, &NoNonpayableReads).unwrap();
                assert_eq!(q.required_gas.as_u64(), 0);
                let mut wrong = r.clone();
                wrong.value = FinalChainNativeValue::new(2u8.into());
                assert_eq!(
                    s.invoke(&wrong, q, &NoNonpayableReads),
                    Err(FinalChainNativeSessionError::QuoteMismatch)
                );
                let mut wrong_q = q;
                wrong_q.required_gas = 1.into();
                assert_eq!(
                    s.invoke(&r, wrong_q, &NoNonpayableReads),
                    Err(FinalChainNativeSessionError::QuoteMismatch)
                );
                let FinalChainNativeInvocationResult::Completed(out) =
                    s.invoke(&r, q, &NoNonpayableReads).unwrap()
                else {
                    panic!("nonpayable completed")
                };
                assert_eq!(
                    out.status,
                    FinalChainNativeStatus::ContractFailure {
                        error: "Method is not payable".into()
                    }
                );
                assert_eq!(out.gas_used.as_u64(), 0);
                assert!(
                    out.output.is_empty()
                        && out.raw_mutations.is_empty()
                        && out.account_mutations.is_empty()
                        && out.logs.is_empty()
                );
                assert_eq!(s.dpos_state, original);
                assert_eq!(s.next_sequence, 1);
                assert!(s.prepared.is_none() && !s.aborted);
                assert_eq!(
                    s.invoke(&r, q, &NoNonpayableReads),
                    Err(FinalChainNativeSessionError::NotPrepared)
                );
                r.id.sequence = 1;
                assert_eq!(
                    s.prepare(&r, &NoNonpayableReads),
                    Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
                );
                assert!(s.aborted);
            }
        }
        for (execution, reader) in [(0, 1), (3, 1), (1, 0), (2, 0), (1, 2), (2, 2)] {
            assert!(matches!(
                chain.begin_native_claim_all_nonpayable(execution.into(), reader.into()),
                Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
            ));
        }
        assert!(
            chain
                .begin_native_claim_all_nonpayable(1.into(), 1.into())
                .is_err(),
            "missing finalized P1"
        );
    });
}

#[test]
fn nonpayable_claim_all_rejects_unmeasured_shapes_and_sources() {
    with_claim_all_chain("nonpayable-scope", |chain| {
        for historical in [false, true] {
            for kind in 0..24 {
                let mut s = nonpayable_session(chain, historical);
                let mut r = request();
                r.period = s.pending_period;
                r.value = FinalChainNativeValue::new(1u8.into());
                match kind {
                    0 => r.value = FinalChainNativeValue::default(),
                    1 => r.value = FinalChainNativeValue::new(2u8.into()),
                    2 => r.supplied_gas = 89998.into(),
                    3 => r.supplied_gas = 178728.into(),
                    4 => r.input.push(0),
                    5 => r.input = DPOS_GET_TOTAL_DELEGATION_SELECTOR.to_vec(),
                    6 => r.depth = 1,
                    7 => r.is_static = true,
                    8 => r.kind = FinalChainNativeCallKind::DelegateCall,
                    9 => r.id.transaction = 1.into(),
                    10 => r.caller = OWNER,
                    11 => r.caller = VALIDATOR,
                    12 => s.dpos_state.total_vote_count = 401,
                    13 => s
                        .dpos_state
                        .vote_counts
                        .insert(SECOND, 201)
                        .map(|_| ())
                        .unwrap(),
                    14 => {
                        s.dpos_state.aspen_supply_state = AspenSupplyState::Unmigrated {
                            minted_tokens: StoredDposTokenAmount::default(),
                        }
                    }
                    15 => s.dpos_state.delegation_ledger_history_complete = false,
                    16 => s.dpos_state.validator_order.swap(0, 1),
                    17 => {
                        s.dpos_state
                            .validator_metadata
                            .get_mut(&VALIDATOR)
                            .unwrap()
                            .commission = 101
                    }
                    18 => s.dpos_state.reward_reference_graph = DposRewardGraph::incomplete(),
                    19 => s
                        .dpos_state
                        .delegator_rewards
                        .insert(
                            SECOND,
                            StoredDposTokenAmount::canonical_u256_after_mutation(U256::one()),
                        )
                        .map(|_| ())
                        .unwrap_or(()),
                    20 => s.dpos_state.slashing_jailed_validators.push(SECOND),
                    21 => {
                        let m = s.dpos_state.validator_metadata.remove(&SECOND).unwrap();
                        s.dpos_state
                            .validator_metadata
                            .insert(DPOS_CONTRACT_ADDRESS, m);
                        s.dpos_state.validator_order[1] = DPOS_CONTRACT_ADDRESS;
                    }
                    22 => s
                        .dpos_state
                        .total_stakes
                        .get_mut(&VALIDATOR)
                        .unwrap()
                        .clone_from(&StoredDposTokenAmount::canonical_u256_after_mutation(
                            U256::from(2001),
                        )),
                    23 => s.pending_period = 3.into(),
                    _ => unreachable!(),
                };
                let before = s.dpos_state.clone();
                assert!(
                    s.prepare(&r, &NoNonpayableReads).is_err(),
                    "nonpayable excluded{kind}"
                );
                assert_eq!(s.aborted, !matches!(kind, 8 | 23));
                assert!(s.prepared.is_none());
                assert_eq!(s.dpos_state, before);
            }
        }
    });
}

#[test]
fn nonpayable_claim_all_retains_selected_period_fork_guards() {
    for historical in [false, true] {
        for kind in 0..5 {
            let excluded = if historical { 2 } else { 3 };
            with_claim_all_chain_config(
                "nonpayable-fork",
                |rewards, ficus| match kind {
                    0 => rewards.fix_claim_all_block_num = excluded.into(),
                    1 => rewards.aspen_part_one_period = excluded.into(),
                    2 => rewards.magnolia_period = excluded.into(),
                    3 => *ficus = excluded.into(),
                    4 => {
                        rewards.aspen_part_two_period = 1.into();
                        rewards.aspen_max_supply = U256::from(8000).into();
                    }
                    _ => unreachable!(),
                },
                |chain| {
                    let mut s = nonpayable_session(chain, historical);
                    let before = s.dpos_state.clone();
                    let mut r = request();
                    r.period = s.pending_period;
                    r.value = FinalChainNativeValue::new(1u8.into());
                    assert!(s.prepare(&r, &NoNonpayableReads).is_err());
                    assert!(s.aborted && s.prepared.is_none());
                    assert_eq!(s.dpos_state, before);
                },
            );
        }
    }
}
