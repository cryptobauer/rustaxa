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
    let path = temp_db_path(test_name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let rewards = FinalChainRewardsConfig {
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
        FinalChainBlockNumber::GENESIS,
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
