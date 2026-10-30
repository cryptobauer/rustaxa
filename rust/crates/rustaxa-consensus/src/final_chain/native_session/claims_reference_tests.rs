//! Direct Rust-session comparison with the pinned live Go reward-claim corpus.
//!
//! The tests execute Rust reward distribution before the claims, compare every
//! claim status, log, and ordered irreversible raw write, and apply the staged
//! ordinary-account effects to an exact full-width journal model. They do not
//! grant publication/reopen authority. Retained zero-stake commission has its
//! separate Go-backed corpus; inconsistent live delegation references remain
//! an integrity failure.

use super::*;
use ethereum_types::{H160, U256};
use num_bigint::{BigInt, BigUint};
use rustaxa_storage::{Config, Storage};
use rustaxa_types::transaction::intrinsic_gas;
use rustaxa_types::{GenesisValidator, GenesisValidatorMetadata};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../experiments/evm_feasibility/fixtures/native_claims/public.json"
));
const DELEGATOR: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xd1,
];
const OWNER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa1,
];
const VALIDATOR: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31,
];
const MISSING_VALIDATOR: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x99,
];

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

    fn apply_outcome(&self, outcome: &FinalChainNativeOutcome) {
        self.apply_raw(&outcome.raw_mutations);
        self.apply_accounts(&outcome.account_mutations);
    }

    fn apply_rewards(&self, outcome: &FinalChainNativeRewardsOutcome) {
        self.apply_raw(&outcome.raw_mutations);
        self.apply_accounts(&outcome.account_mutations);
    }

    fn apply_raw(&self, mutations: &[FinalChainNativeRawMutation]) {
        for mutation in mutations {
            let value = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => {
                    ConcreteRead::Present(value.as_bytes().to_vec())
                }
                FinalChainNativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
            };
            self.rows
                .borrow_mut()
                .insert((mutation.address, mutation.key.0), value);
        }
    }

    fn apply_accounts(&self, mutations: &[FinalChainNativeOrdinaryMutation]) {
        let mut accounts = self.accounts.borrow_mut();
        for mutation in mutations {
            match mutation {
                FinalChainNativeOrdinaryMutation::NonceReplace { .. } => {
                    panic!("reward claims must not change account nonces")
                }
                FinalChainNativeOrdinaryMutation::EnsureExists {
                    address,
                    expected_exists,
                }
                | FinalChainNativeOrdinaryMutation::Touch {
                    address,
                    expected_exists,
                } => {
                    let current = accounts.entry(*address).or_insert(FinalChainNativeAccount {
                        exists: false,
                        nonce: FinalChainNonce::zero(),
                        balance: BigInt::default(),
                    });
                    assert_eq!(current.exists, *expected_exists);
                    current.exists = true;
                }
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address,
                    expected_exists,
                    expected,
                    replacement,
                } => {
                    let current = accounts.entry(*address).or_insert(FinalChainNativeAccount {
                        exists: false,
                        nonce: FinalChainNonce::zero(),
                        balance: BigInt::default(),
                    });
                    assert_eq!(current.exists, *expected_exists);
                    assert_eq!(&current.balance, expected);
                    current.exists = true;
                    current.balance = replacement.clone();
                }
            }
        }
    }

    fn balance(&self, address: [u8; 20]) -> BigInt {
        self.accounts.borrow()[&address].balance.clone()
    }

    fn replace_balance(&self, address: [u8; 20], balance: BigInt) {
        self.accounts
            .borrow_mut()
            .get_mut(&address)
            .unwrap()
            .balance = balance;
    }

    fn remove_account(&self, address: [u8; 20]) {
        self.accounts.borrow_mut().remove(&address);
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

struct RejectAccountReads<'a>(&'a ReferenceState);

impl FinalChainNativeStateRead for RejectAccountReads<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        Err(FinalChainNativeStateReadError::Invariant(format!(
            "unexpected zero-claim account read: {address:?}"
        )))
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        self.0.raw_storage(address, key)
    }
}

fn fixture() -> Value {
    serde_json::from_str(ORACLE).expect("pinned Go claims fixture is valid JSON")
}

fn scenario<'a>(fixture: &'a Value, name: &str) -> &'a Value {
    fixture["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|scenario| scenario["name"] == name)
        .unwrap_or_else(|| panic!("claims fixture scenario is absent: {name}"))
}

fn temp_db_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    crate::closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!(
            "rustaxa-consensus-claims-corpus-{test_name}-{}-{nanos}",
            std::process::id()
        )),
        true,
    )
}

fn with_reference_chain(test_name: &str, test: impl FnOnce(&FinalChain)) {
    with_reference_chain_at_magnolia(test_name, FinalChainBlockNumber::GENESIS, test)
}

fn with_reference_chain_at_magnolia(
    test_name: &str,
    magnolia: FinalChainBlockNumber,
    test: impl FnOnce(&FinalChain),
) {
    let path = temp_db_path(test_name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let rewards = FinalChainRewardsConfig {
        magnolia_period: magnolia,
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
        crate::FinalChainGenesisAllocation {
            accounts: Vec::new(),
            validators: vec![GenesisValidator {
                address: VALIDATOR,
                vrf_key: [0x44; 32],
                total_stake: U256::from(1_000_u64).to_big_endian().to_vec(),
                delegations: vec![(DELEGATOR, U256::from(1_000_u64).to_big_endian().to_vec())],
                metadata: GenesisValidatorMetadata {
                    owner: OWNER,
                    commission: 100,
                    ..Default::default()
                },
            }],
        },
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
    let _ = crate::closeout_test_scratch::dispose(path);
}

fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_decode(value: &str) -> Vec<u8> {
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "fixture hex has an odd length");
    pairs
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                .expect("fixture contains hexadecimal bytes")
        })
        .collect()
}

fn fixture_request(
    period: u64,
    position: u32,
    sequence: u64,
    expected: &Value,
) -> FinalChainNativeRequest {
    let selector = hex_decode(expected["selector"].as_str().unwrap());
    let mut input = selector.clone();
    input.extend_from_slice(&[0; 12]);
    let name = expected["name"].as_str().unwrap();
    input.extend_from_slice(if name.contains("missing") {
        &MISSING_VALIDATOR
    } else {
        &VALIDATOR
    });
    let caller = if name.starts_with("claim_rewards") || name == "claim_commission_wrong_owner" {
        DELEGATOR
    } else {
        OWNER
    };
    let supplied_gas = if selector == DPOS_CLAIM_REWARDS_SELECTOR {
        DPOS_CLAIM_REWARDS_GAS
    } else {
        DPOS_CLAIM_COMMISSION_REWARDS_GAS
    };
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: FinalChainTransactionPosition::new(position),
            sequence,
        },
        period: period.into(),
        depth: 1,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller,
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(BigUint::default()),
        input,
        supplied_gas: supplied_gas.into(),
    }
}

fn compare_raw_mutations(actual: &[FinalChainNativeRawMutation], groups: &[&Value]) {
    let expected = groups
        .iter()
        .flat_map(|group| group.as_array().map(Vec::as_slice).unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        actual.len(),
        expected.len(),
        "actual ordered writes: {actual:?}; expected ordered writes: {expected:?}"
    );
    for (actual, expected) in actual.iter().zip(expected) {
        let value = match &actual.operation {
            FinalChainNativeRawOperation::Put(value) => value.as_bytes(),
            FinalChainNativeRawOperation::Delete => &[],
        };
        assert_eq!(hex_bytes(actual.address), expected["address"]);
        assert_eq!(hex_bytes(actual.key.0), expected["key"]);
        assert_eq!(hex_bytes(value), expected["value"]);
    }
}

fn invoke_and_compare(
    session: &mut FinalChainNativeSession<'_>,
    state: &ReferenceState,
    request: &FinalChainNativeRequest,
    expected: &Value,
) -> FinalChainNativeOutcome {
    assert_eq!(expected["consensus_error"], "");
    assert_eq!(hex_bytes(&request.input[..4]), expected["selector"]);
    let quote = session.prepare(request, state).unwrap();
    let action_gas = if request.input[..4] == DPOS_CLAIM_REWARDS_SELECTOR {
        DPOS_CLAIM_REWARDS_GAS
    } else {
        DPOS_CLAIM_COMMISSION_REWARDS_GAS
    };
    assert_eq!(quote.required_gas.as_u64(), action_gas);
    assert_eq!(quote.invocation, request.id);
    assert_eq!(
        intrinsic_gas(&request.input, false).unwrap() + action_gas,
        expected["gas_used"].as_u64().unwrap()
    );
    let outcome = match session.invoke(request, quote, state).unwrap() {
        FinalChainNativeInvocationResult::Completed(outcome) => outcome,
        result => panic!("claims corpus invocation did not complete: {result:?}"),
    };
    assert_eq!(outcome.gas_used.as_u64(), action_gas);
    assert_eq!(hex_bytes(&outcome.output), expected["output"]);
    let expected_error = expected["execution_error"].as_str().unwrap();
    assert_eq!(
        outcome.status,
        if expected_error.is_empty() {
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: expected_error.to_owned(),
            }
        }
    );
    let expected_logs = expected["logs"].as_array().unwrap();
    assert_eq!(outcome.logs.len(), expected_logs.len());
    for (actual, expected) in outcome.logs.iter().zip(expected_logs) {
        assert_eq!(hex_bytes(actual.address), expected["address"]);
        assert_eq!(
            actual.topics.iter().map(hex_bytes).collect::<Vec<_>>(),
            expected["topics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|topic| topic.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(hex_bytes(&actual.data), expected["data"]);
    }
    compare_raw_mutations(&outcome.raw_mutations, &[&expected["ordered_raw_writes"]]);
    outcome
}

fn reward_plan(
    chain: &FinalChain,
    request_id: [u8; 32],
) -> FinalChainPreparedExternalEvmRewardsStatsPlan {
    chain
        .plan_external_evm_rewards_stats(
            request_id,
            FinalizedRewardsPeriodFact {
                period: 1,
                block_author: H160::from(VALIDATOR),
                blocks_per_year: 1,
                dpos_eligible_total_vote_count: 0,
                transactions: Vec::new(),
                dag_blocks: Vec::new(),
                cert_votes: vec![RewardCertVoteFact {
                    voter: H160::from(VALIDATOR),
                    weight: 1,
                    period: 1,
                }],
            },
        )
        .unwrap()
}

fn advance_unpublished_semantic_session<'a>(
    chain: &'a FinalChain,
    mut session: FinalChainNativeSession<'a>,
) -> FinalChainNativeSession<'a> {
    chain
        .advance_reward_reference_graph_block(&mut session.dpos_state, 2.into())
        .unwrap();
    session.pending_period = 2.into();
    session.period_start_total_vote_count = session.dpos_state.total_vote_count;
    session.period_start_amount_delegated = total_staked_amount(&session.dpos_state).unwrap();
    session.eligibility_period = 1.into();
    session.eligibility_state = session.dpos_state.clone();
    session.request_id = None;
    session.next_sequence = 0;
    session.prepared = None;
    session.aborted = false;
    session.finished_rewards = false;
    session
}

fn rewarded_claim_session<'a>(
    chain: &'a FinalChain,
    state: &ReferenceState,
    expected: &Value,
) -> FinalChainNativeSession<'a> {
    let request_id = [0x93; 32];
    let mut session = chain
        .begin_native_session_bound(request_id, 1.into(), FinalChainBlockNumber::GENESIS)
        .unwrap();
    let rewards = session
        .finish_rewards(&reward_plan(chain, request_id), state)
        .unwrap();
    assert_eq!(
        rewards.total_reward.as_u256().to_string(),
        expected["reward_minted"]
    );
    compare_raw_mutations(
        &rewards.raw_mutations,
        &[
            &expected["reward_ordered_raw_writes"],
            &expected["reward_end_block_ordered_raw_writes"],
        ],
    );
    state.apply_rewards(&rewards);
    advance_unpublished_semantic_session(chain, session)
}

fn assert_fixture_balances(state: &ReferenceState, expected: &Value) {
    assert_eq!(
        state.balance(DELEGATOR).to_string(),
        expected["delegator_balance"]
    );
    assert_eq!(state.balance(OWNER).to_string(), expected["owner_balance"]);
    assert_eq!(
        state.balance(DPOS_CONTRACT_ADDRESS).to_string(),
        expected["contract_balance"]
    );
}

#[test]
fn accrued_claim_sessions_match_pinned_go_outcomes_raw_traces_and_balances() {
    let fixture = fixture();
    assert_eq!(fixture["schema"], 1);
    assert_eq!(
        fixture["selectors"]["claim_rewards"],
        hex_bytes(DPOS_CLAIM_REWARDS_SELECTOR)
    );
    assert_eq!(
        fixture["selectors"]["claim_commission_rewards"],
        hex_bytes(DPOS_CLAIM_COMMISSION_REWARDS_SELECTOR)
    );
    assert_eq!(
        fixture["action_gas"]["claim_rewards"],
        DPOS_CLAIM_REWARDS_GAS
    );
    assert_eq!(
        fixture["action_gas"]["claim_commission_rewards"],
        DPOS_CLAIM_COMMISSION_REWARDS_GAS
    );

    for name in ["delegator_accrued", "commission_accrued"] {
        let expected = scenario(&fixture, name);
        with_reference_chain(name, |chain| {
            let initial = chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let state = ReferenceState::from_snapshot(&initial.dpos_state);
            assert_fixture_balances(&state, &expected["before"]);
            let mut session = rewarded_claim_session(chain, &state, expected);
            assert_fixture_balances(&state, &expected["after_reward"]);
            for (position, transaction) in expected["transactions"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let request = fixture_request(2, position as u32, position as u64, transaction);
                let outcome = invoke_and_compare(&mut session, &state, &request, transaction);
                if name == "delegator_accrued" && position == 1 {
                    assert!(outcome.account_mutations.is_empty());
                }
                if name == "commission_accrued" && position == 1 {
                    assert_eq!(
                        outcome.account_mutations,
                        vec![
                            FinalChainNativeOrdinaryMutation::EnsureExists {
                                address: DPOS_CONTRACT_ADDRESS,
                                expected_exists: true,
                            },
                            FinalChainNativeOrdinaryMutation::Touch {
                                address: OWNER,
                                expected_exists: true,
                            },
                        ]
                    );
                }
                state.apply_outcome(&outcome);
            }
            assert_fixture_balances(&state, &expected["after_claims"]);
        });
    }
}

fn compare_pre_magnolia_write_evidence(actual: &[FinalChainNativeRawMutation], tx: &Value) {
    let before = tx["legacy_dpos_storage_before"].as_array().unwrap();
    let after = tx["legacy_dpos_storage_after"].as_array().unwrap();
    assert_eq!(actual.len(), before.len());
    assert_eq!(actual.len(), after.len());
    for ((mutation, prior), following) in actual.iter().zip(before).zip(after) {
        assert_eq!(hex_bytes(mutation.address), prior["address"]);
        assert_eq!(hex_bytes(mutation.key.0), prior["key"]);
        assert_eq!(prior["address"], following["address"]);
        assert_eq!(prior["key"], following["key"]);
        let prior_bytes = hex_decode(prior["value"].as_str().unwrap());
        if prior["callback_observed"] == true {
            assert_eq!(mutation.expected, ConcreteRead::Present(prior_bytes));
        } else if prior["callback_observed"] == false {
            assert!(prior_bytes.is_empty());
            if let ConcreteRead::Present(bytes) = &mutation.expected {
                assert!(bytes.is_empty());
            }
        } else if prior["evidence"].as_str().unwrap().starts_with("replayed_") {
            match &mutation.expected {
                ConcreteRead::Present(bytes) => assert_eq!(bytes, &prior_bytes),
                ConcreteRead::Absent | ConcreteRead::Tombstone => assert!(prior_bytes.is_empty()),
            }
        }
        let written = match &mutation.operation {
            FinalChainNativeRawOperation::Put(value) => value.as_bytes(),
            FinalChainNativeRawOperation::Delete => &[],
        };
        assert_eq!(hex_bytes(written), following["value"]);
    }
}

struct LegacyFaultReader<'a> {
    state: &'a ReferenceState,
    fail_key: ConcreteStorageKey,
    reads_before_failure: std::cell::Cell<usize>,
}

impl FinalChainNativeStateRead for LegacyFaultReader<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.state.account(address)
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        if *key == self.fail_key {
            return Err(FinalChainNativeStateReadError::Invariant(
                "legacy late read fault".to_owned(),
            ));
        }
        self.reads_before_failure
            .set(self.reads_before_failure.get() + 1);
        self.state.raw_storage(address, key)
    }
}

#[test]
fn pre_magnolia_commission_guards_and_late_fault_are_atomic() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_claims_pre_magnolia/public.json"
    )))
    .unwrap();
    let expected = scenario(&fixture, "commission_accrued");
    for mode in [
        "owner",
        "validator",
        "incomplete",
        "nonlegacy",
        "zero_stake",
        "late_fault",
    ] {
        with_reference_chain_at_magnolia(mode, 3.into(), |chain| {
            let initial = chain.begin_native_session(1.into(), 0.into()).unwrap();
            let state = ReferenceState::from_snapshot(&initial.dpos_state);
            let mut session = rewarded_claim_session(chain, &state, expected);
            let request = fixture_request(2, 0, 0, &expected["transactions"][0]);
            if mode == "owner" {
                state.rows.borrow_mut().insert(
                    (
                        DPOS_CONTRACT_ADDRESS,
                        custody::validator_owner_key(VALIDATOR).0,
                    ),
                    ConcreteRead::Present(vec![0x77]),
                );
            } else if mode == "validator" {
                state.rows.borrow_mut().insert(
                    (DPOS_CONTRACT_ADDRESS, custody::validator_key(VALIDATOR).0),
                    ConcreteRead::Present(vec![1]),
                );
            } else if mode == "incomplete" {
                session.dpos_state.validator_row_authority =
                    ValidatorRowAuthority::CompatibilityOnly;
            } else if mode == "nonlegacy" {
                if let ValidatorRowAuthority::Complete(rows) =
                    &mut session.dpos_state.validator_row_authority
                {
                    rows.insert(VALIDATOR, ValidatorRowShape::Extended);
                } else {
                    panic!("expected complete fixture authority");
                }
            } else if mode == "zero_stake" {
                session
                    .dpos_state
                    .total_stakes
                    .insert(VALIDATOR, StoredDposTokenAmount::default());
            }
            let before = session.dpos_state.clone();
            let rows_before = state.rows.borrow().clone();
            let accounts_before = state.accounts.borrow().clone();
            let quote = session.prepare(&request, &state).unwrap();
            if mode == "late_fault" {
                let reader = LegacyFaultReader {
                    state: &state,
                    fail_key: custody::rewards_key(VALIDATOR),
                    reads_before_failure: std::cell::Cell::new(0),
                };
                assert!(matches!(session.invoke(&request, quote, &reader),
                    Err(FinalChainNativeSessionError::StateRead(FinalChainNativeStateReadError::Invariant(ref s))) if s == "legacy late read fault"));
                assert!(reader.reads_before_failure.get() >= 2);
            } else if mode == "owner" || mode == "validator" || mode == "zero_stake" {
                assert!(matches!(
                    session.invoke(&request, quote, &state),
                    Err(FinalChainNativeSessionError::RawIntegrity(_))
                ));
            } else {
                assert!(matches!(
                    session.invoke(&request, quote, &state),
                    Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
                ));
            }
            assert_eq!(session.dpos_state, before);
            assert_eq!(*state.rows.borrow(), rows_before);
            assert_eq!(*state.accounts.borrow(), accounts_before);
            assert!(matches!(
                session.invoke(&request, quote, &state),
                Err(FinalChainNativeSessionError::Aborted)
            ));
        });
    }
}

fn legacy_fixture_advance(
    chain: &FinalChain,
    session: &mut FinalChainNativeSession<'_>,
    next: u64,
) {
    chain
        .advance_reward_reference_graph_block(&mut session.dpos_state, next.into())
        .unwrap();
    session.pending_period = next.into();
    session.period_start_total_vote_count = session.dpos_state.total_vote_count;
    session.period_start_amount_delegated = total_staked_amount(&session.dpos_state).unwrap();
    session.eligibility_period = (next - 1).into();
    session.eligibility_state = session.dpos_state.clone();
    session.next_sequence = 0;
    session.prepared = None;
    session.finished_rewards = false;
}

fn compare_zero_setup_logs(actual: &[ReceiptLog], expected: &Value) {
    let logs = expected["logs"].as_array().unwrap();
    assert_eq!(actual.len(), logs.len());
    for (log, row) in actual.iter().zip(logs) {
        assert_eq!(hex_bytes(log.address), row["address"]);
        assert_eq!(
            log.topics.iter().map(hex_bytes).collect::<Vec<_>>(),
            row["topics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(hex_bytes(&log.data), row["data"]);
    }
}

fn apply_captured_legacy_setup(
    chain: &FinalChain,
    session: &mut FinalChainNativeSession<'_>,
    state: &ReferenceState,
    transaction: DposTransaction,
    expected: &Value,
    before_key: &str,
    after_key: &str,
) {
    let before = session.dpos_state.clone();
    let mut after = before.clone();
    let mut accounts = account::StagedDposAccountPort::from_state(state);
    let result = match transaction.clone() {
        DposTransaction::Undelegate {
            delegator,
            validator,
            amount,
        } => chain.apply_dpos_undelegate(
            &mut after,
            &mut accounts,
            delegator,
            validator,
            amount,
            session.pending_period,
        ),
        DposTransaction::ConfirmUndelegate {
            delegator,
            validator,
        } => chain.apply_dpos_confirm_undelegate(
            &mut after,
            &mut accounts,
            delegator,
            validator,
            session.pending_period,
        ),
        _ => panic!("unsupported synthetic setup"),
    }
    .unwrap();
    let tx = if matches!(transaction, DposTransaction::Undelegate { .. }) {
        &expected["undelegation"]
    } else {
        &expected["confirmation"]
    };
    let mut input = hex_decode(tx["selector"].as_str().unwrap());
    input.extend_from_slice(&[0; 12]);
    input.extend_from_slice(&VALIDATOR);
    let action_gas = if matches!(transaction, DposTransaction::Undelegate { .. }) {
        input.extend_from_slice(&U256::from(1000_u64).to_big_endian());
        DPOS_UNDELEGATE_GAS
    } else {
        DPOS_DEFAULT_METHOD_GAS
    };
    assert_eq!(
        intrinsic_gas(&input, false).unwrap() + action_gas,
        tx["gas_used"].as_u64().unwrap()
    );
    assert_eq!(result.status_code, 1);
    assert_eq!(tx["consensus_error"], "");
    assert_eq!(tx["execution_error"], "");
    assert_eq!(hex_bytes(&result.code_retval), tx["output"]);
    compare_zero_setup_logs(&result.logs, tx);
    let mut trace = raw::FinalChainNativeRawTrace::new(state);
    session
        .serialize_legacy_setup_for_test(&transaction, &before, &after, &mut trace)
        .unwrap();
    let writes = trace.finish();
    compare_raw_mutations(&writes, &[&tx["ordered_raw_writes"]]);
    let mut evidence = tx.clone();
    evidence["legacy_dpos_storage_before"] = expected[before_key].clone();
    evidence["legacy_dpos_storage_after"] = expected[after_key].clone();
    compare_pre_magnolia_write_evidence(&writes, &evidence);
    state.apply_raw(&writes);
    state.apply_accounts(&accounts.into_mutations());
    session.dpos_state = after;
}

fn assert_no_recorded_end_block_writes(value: &Value) {
    assert!(
        value.is_null() || value.as_array().is_some_and(Vec::is_empty),
        "skipped EndBlock must have no recorded raw effects"
    );
}

#[test]
fn pre_magnolia_zero_stake_and_cold_commission_match_paired_go() {
    let public: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_commission_zero_stake_pre_magnolia/public.json"))).unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_commission_zero_stake_pre_magnolia/local.json"))).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["configuration"]["magnolia"], 10);
    for name in ["pending_retained", "confirmed_cleared"] {
        let expected = scenario(&public, name);
        for mode in [
            "owner_mismatch",
            "validator_mismatch",
            "zero_pool",
            "late_fault",
            "clean",
        ] {
            with_reference_chain_at_magnolia(name, 10.into(), |chain| {
                let initial = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let state = ReferenceState::from_snapshot(&initial.dpos_state);
                let mut session = rewarded_claim_session(chain, &state, expected);
                let setup = DposTransaction::Undelegate {
                    delegator: DELEGATOR,
                    validator: VALIDATOR,
                    amount: U256::from(1000_u64).to_big_endian().to_vec(),
                };
                apply_captured_legacy_setup(
                    chain,
                    &mut session,
                    &state,
                    setup,
                    expected,
                    "undelegation_before_writes",
                    "undelegation_after_writes",
                );
                let mut trace = raw::FinalChainNativeRawTrace::new(&state);
                session
                    .serialize_deferred_end_block_for_test(&session.dpos_state, &mut trace)
                    .unwrap();
                let writes = trace.finish();
                compare_raw_mutations(
                    &writes,
                    &[&expected["undelegation_end_block_ordered_raw_writes"]],
                );
                state.apply_raw(&writes);
                assert!(session.dpos_state.total_stakes[&VALIDATOR].is_zero());
                assert!(find_undelegation(&session.dpos_state, DELEGATOR, VALIDATOR).is_some());
                if name == "confirmed_cleared" {
                    let empty = expected["empty_periods"].as_array().unwrap();
                    assert_eq!(empty.len(), 2);
                    for (entry, period) in empty.iter().zip([3u64, 4]) {
                        assert_eq!(entry["period"], period);
                        assert_no_recorded_end_block_writes(&entry["end_block_ordered_raw_writes"]);
                    }
                    for period in 3..=5 {
                        legacy_fixture_advance(chain, &mut session, period);
                    }
                    apply_captured_legacy_setup(
                        chain,
                        &mut session,
                        &state,
                        DposTransaction::ConfirmUndelegate {
                            delegator: DELEGATOR,
                            validator: VALIDATOR,
                        },
                        expected,
                        "confirmation_before_writes",
                        "confirmation_after_writes",
                    );
                    assert!(find_undelegation(&session.dpos_state, DELEGATOR, VALIDATOR).is_none());
                    assert_no_recorded_end_block_writes(
                        &expected["confirmation_end_block_ordered_raw_writes"],
                    );
                }
                let claim_period = expected["claim_period"].as_u64().unwrap();
                legacy_fixture_advance(chain, &mut session, claim_period);
                assert_eq!(
                    state.balance(OWNER).to_string(),
                    expected["before_claim"]["owner_balance"]
                );
                assert_eq!(
                    state.balance(DELEGATOR).to_string(),
                    expected["before_claim"]["custody"]["delegator_balance"]
                );
                assert_eq!(
                    state.balance(DPOS_CONTRACT_ADDRESS).to_string(),
                    expected["before_claim"]["custody"]["contract_balance"]
                );
                assert_no_recorded_end_block_writes(
                    &expected["claim_end_block_ordered_raw_writes"],
                );
                let request = fixture_request(claim_period, 0, 0, &expected["claim"]);
                if mode != "clean" {
                    if mode == "owner_mismatch" {
                        state.rows.borrow_mut().insert(
                            (
                                DPOS_CONTRACT_ADDRESS,
                                custody::validator_owner_key(VALIDATOR).0,
                            ),
                            ConcreteRead::Present(vec![0x77]),
                        );
                    } else if mode == "validator_mismatch" {
                        state.rows.borrow_mut().insert(
                            (DPOS_CONTRACT_ADDRESS, custody::validator_key(VALIDATOR).0),
                            ConcreteRead::Present(vec![1]),
                        );
                    } else if mode == "zero_pool" {
                        session
                            .dpos_state
                            .commission_rewards
                            .insert(VALIDATOR, StoredDposTokenAmount::default());
                        state.rows.borrow_mut().insert(
                            (DPOS_CONTRACT_ADDRESS, custody::rewards_key(VALIDATOR).0),
                            ConcreteRead::Present(custody::encode_rewards(
                                &session.dpos_state,
                                VALIDATOR,
                            )),
                        );
                    }
                    let before = session.dpos_state.clone();
                    let rows_before = state.rows.borrow().clone();
                    let accounts_before = state.accounts.borrow().clone();
                    let quote = session.prepare(&request, &state).unwrap();
                    if mode == "late_fault" {
                        let last = expected["claim"]["ordered_raw_writes"]
                            .as_array()
                            .unwrap()
                            .last()
                            .unwrap();
                        let key: [u8; 32] = hex_decode(last["key"].as_str().unwrap())
                            .try_into()
                            .unwrap();
                        let reader = LegacyFaultReader {
                            state: &state,
                            fail_key: ConcreteStorageKey(key),
                            reads_before_failure: std::cell::Cell::new(0),
                        };
                        assert!(matches!(session.invoke(&request, quote, &reader),
                        Err(FinalChainNativeSessionError::StateRead(FinalChainNativeStateReadError::Invariant(ref s))) if s == "legacy late read fault"));
                        assert!(
                            reader.reads_before_failure.get() >= 5,
                            "earlier serializer rows were reached"
                        );
                    } else if mode == "zero_pool" {
                        assert!(matches!(
                            session.invoke(&request, quote, &state),
                            Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
                        ));
                    } else {
                        assert!(matches!(
                            session.invoke(&request, quote, &state),
                            Err(FinalChainNativeSessionError::RawIntegrity(_))
                        ));
                    }
                    assert_eq!(session.dpos_state, before);
                    assert_eq!(*state.rows.borrow(), rows_before);
                    assert_eq!(*state.accounts.borrow(), accounts_before);
                    assert!(matches!(
                        session.invoke(&request, quote, &state),
                        Err(FinalChainNativeSessionError::Aborted)
                    ));
                    return;
                }
                let outcome =
                    invoke_and_compare(&mut session, &state, &request, &expected["claim"]);
                let mut evidence = expected["claim"].clone();
                evidence["legacy_dpos_storage_before"] = expected["claim_before_writes"].clone();
                evidence["legacy_dpos_storage_after"] = expected["claim_after_writes"].clone();
                compare_pre_magnolia_write_evidence(&outcome.raw_mutations, &evidence);
                state.apply_outcome(&outcome);
                assert!(!session.dpos_state.total_stakes.contains_key(&VALIDATOR));
                assert_eq!(
                    find_undelegation(&session.dpos_state, DELEGATOR, VALIDATOR).is_some(),
                    name == "pending_retained"
                );
                assert_eq!(
                    state.balance(OWNER).to_string(),
                    expected["after_claim"]["owner_balance"]
                );
                assert_eq!(
                    state.balance(DELEGATOR).to_string(),
                    expected["after_claim"]["custody"]["delegator_balance"]
                );
                assert_eq!(
                    state.balance(DPOS_CONTRACT_ADDRESS).to_string(),
                    expected["after_claim"]["custody"]["contract_balance"]
                );
            });
        }
    }
    let expected = scenario(&public, "cold_zero_positive_stake");
    assert_no_recorded_end_block_writes(&expected["claim_end_block_ordered_raw_writes"]);
    with_reference_chain_at_magnolia("cold-zero", 10.into(), |chain| {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        let request = fixture_request(1, 0, 0, &expected["claim"]);
        let outcome = invoke_and_compare(&mut session, &state, &request, &expected["claim"]);
        let mut evidence = expected["claim"].clone();
        evidence["legacy_dpos_storage_before"] = expected["claim_before_writes"].clone();
        evidence["legacy_dpos_storage_after"] = expected["claim_after_writes"].clone();
        compare_pre_magnolia_write_evidence(&outcome.raw_mutations, &evidence);
        state.apply_outcome(&outcome);
        assert_eq!(
            state.balance(OWNER).to_string(),
            expected["after_claim"]["owner_balance"]
        );
        assert_eq!(
            state.balance(DPOS_CONTRACT_ADDRESS).to_string(),
            expected["after_claim"]["custody"]["contract_balance"]
        );
    });
}

#[test]
fn pre_magnolia_claims_match_paired_go_reference_and_ordered_raw_effects() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_claims_pre_magnolia/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_claims_pre_magnolia/local.json"
    )))
    .unwrap();
    assert_eq!(local, public);
    assert_eq!(public["configuration"]["magnolia"], 3);
    assert_eq!(public["configuration"]["reward_period"], 1);
    assert_eq!(public["configuration"]["claims_period"], 2);
    for name in ["delegator_accrued", "commission_accrued"] {
        let expected = scenario(&public, name);
        with_reference_chain_at_magnolia(name, 3.into(), |chain| {
            let initial = chain.begin_native_session(1.into(), 0.into()).unwrap();
            assert!(initial.dpos_state.has_complete_validator_shapes());
            let state = ReferenceState::from_snapshot(&initial.dpos_state);
            assert_fixture_balances(&state, &expected["before"]);
            let mut session = rewarded_claim_session(chain, &state, expected);
            assert_fixture_balances(&state, &expected["after_reward"]);
            for (position, transaction) in expected["transactions"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let request = fixture_request(2, position as u32, position as u64, transaction);
                let outcome = invoke_and_compare(&mut session, &state, &request, transaction);
                compare_pre_magnolia_write_evidence(&outcome.raw_mutations, transaction);
                state.apply_outcome(&outcome);
            }
            assert_fixture_balances(&state, &expected["after_claims"]);
        });
    }
    with_reference_chain_at_magnolia("legacy-errors", 3.into(), |chain| {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        for (position, expected) in public["errors"].as_array().unwrap().iter().enumerate() {
            let request = fixture_request(1, position as u32, position as u64, expected);
            let outcome = invoke_and_compare(&mut session, &state, &request, expected);
            assert!(
                outcome.account_mutations.is_empty()
                    && outcome.raw_mutations.is_empty()
                    && outcome.logs.is_empty()
            );
        }
    });
}

#[test]
fn claim_business_errors_match_go_precedence_without_account_or_raw_effects() {
    let fixture = fixture();
    with_reference_chain("errors", |chain| {
        let mut session = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        for (position, expected) in fixture["errors"].as_array().unwrap().iter().enumerate() {
            let request = fixture_request(1, position as u32, position as u64, expected);
            let outcome = invoke_and_compare(&mut session, &state, &request, expected);
            assert!(outcome.account_mutations.is_empty());
            assert!(outcome.raw_mutations.is_empty());
            assert!(outcome.logs.is_empty());
        }
    });
}

#[test]
fn claims_preserve_full_width_balances_and_type_account_read_failures() {
    let fixture = fixture();
    let delegator_expected = scenario(&fixture, "delegator_accrued");
    let commission_expected = scenario(&fixture, "commission_accrued");
    with_reference_chain("full-width", |chain| {
        let initial = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let state = ReferenceState::from_snapshot(&initial.dpos_state);
        let mut session = rewarded_claim_session(chain, &state, delegator_expected);
        let wide = (BigInt::from(1_u8) << 320_usize) + BigInt::from(1_200_u64);
        state.replace_balance(DPOS_CONTRACT_ADDRESS, wide.clone());
        state.replace_balance(DELEGATOR, BigInt::from(-17));
        let transaction = &delegator_expected["transactions"][0];
        let request = fixture_request(2, 0, 0, transaction);
        let quote = session.prepare(&request, &state).unwrap();
        let outcome = match session.invoke(&request, quote, &state).unwrap() {
            FinalChainNativeInvocationResult::Completed(outcome) => outcome,
            result => panic!("full-width claim did not complete: {result:?}"),
        };
        assert_eq!(
            outcome.account_mutations,
            vec![
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: DPOS_CONTRACT_ADDRESS,
                    expected_exists: true,
                    expected: wide.clone(),
                    replacement: &wide - BigInt::from(198_u64),
                },
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: DELEGATOR,
                    expected_exists: true,
                    expected: BigInt::from(-17),
                    replacement: BigInt::from(181_u64),
                },
            ]
        );

        let initial = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let missing_state = ReferenceState::from_snapshot(&initial.dpos_state);
        let mut missing_session = rewarded_claim_session(chain, &missing_state, delegator_expected);
        missing_state.remove_account(DPOS_CONTRACT_ADDRESS);
        let request = fixture_request(2, 0, 0, transaction);
        let quote = missing_session.prepare(&request, &missing_state).unwrap();
        assert_eq!(
            missing_session.invoke(&request, quote, &missing_state),
            Err(FinalChainNativeSessionError::StateRead(
                FinalChainNativeStateReadError::Invariant(format!(
                    "claims corpus account is unavailable: {DPOS_CONTRACT_ADDRESS:?}"
                ))
            ))
        );
        assert_eq!(
            missing_session.prepare(&request, &missing_state),
            Err(FinalChainNativeSessionError::Aborted)
        );

        let initial = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let commission_state = ReferenceState::from_snapshot(&initial.dpos_state);
        let mut commission_session =
            rewarded_claim_session(chain, &commission_state, commission_expected);
        let commission_wide = (BigInt::from(1_u8) << 384_usize) + BigInt::from(1_200_u64);
        commission_state.replace_balance(DPOS_CONTRACT_ADDRESS, commission_wide.clone());
        commission_state.replace_balance(OWNER, BigInt::from(-17));
        let commission_transaction = &commission_expected["transactions"][0];
        let commission_request = fixture_request(2, 0, 0, commission_transaction);
        let quote = commission_session
            .prepare(&commission_request, &commission_state)
            .unwrap();
        let outcome = match commission_session
            .invoke(&commission_request, quote, &commission_state)
            .unwrap()
        {
            FinalChainNativeInvocationResult::Completed(outcome) => outcome,
            result => panic!("full-width commission claim did not complete: {result:?}"),
        };
        assert_eq!(
            outcome.account_mutations,
            vec![
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: DPOS_CONTRACT_ADDRESS,
                    expected_exists: true,
                    expected: commission_wide.clone(),
                    replacement: &commission_wide - BigInt::from(2_u8),
                },
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: OWNER,
                    expected_exists: true,
                    expected: BigInt::from(-17),
                    replacement: BigInt::from(-15),
                },
            ]
        );

        let initial = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let missing_owner_state = ReferenceState::from_snapshot(&initial.dpos_state);
        let mut missing_owner_session =
            rewarded_claim_session(chain, &missing_owner_state, commission_expected);
        missing_owner_state.remove_account(OWNER);
        let snapshot_before = missing_owner_session.dpos_state.clone();
        let rows_before = missing_owner_state.rows.borrow().clone();
        let commission_request = fixture_request(2, 0, 0, commission_transaction);
        let quote = missing_owner_session
            .prepare(&commission_request, &missing_owner_state)
            .unwrap();
        assert_eq!(
            missing_owner_session.invoke(&commission_request, quote, &missing_owner_state),
            Err(FinalChainNativeSessionError::StateRead(
                FinalChainNativeStateReadError::Invariant(format!(
                    "claims corpus account is unavailable: {OWNER:?}"
                ))
            ))
        );
        assert_eq!(missing_owner_session.dpos_state, snapshot_before);
        assert_eq!(*missing_owner_state.rows.borrow(), rows_before);
    });
}

#[test]
fn zero_stake_commission_rejects_inconsistent_live_delegation_references() {
    with_reference_chain("zero-stake", |chain| {
        let mut session = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        session
            .dpos_state
            .total_stakes
            .insert(VALIDATOR, StoredDposTokenAmount::default());
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        let before = session.dpos_state.clone();
        let rows_before = state.rows.borrow().clone();
        let accounts_before = state.accounts.borrow().clone();
        let fixture = fixture();
        let expected = &scenario(&fixture, "commission_accrued")["transactions"][1];
        let request = fixture_request(1, 0, 0, expected);
        let quote = session.prepare(&request, &state).unwrap();
        assert_eq!(
            session.invoke(&request, quote, &state),
            Err(FinalChainNativeSessionError::Domain(
                "DPoS validator deletion found live delegation references".to_owned()
            ))
        );
        assert_eq!(session.dpos_state, before);
        assert_eq!(*state.rows.borrow(), rows_before);
        assert_eq!(*state.accounts.borrow(), accounts_before);
        assert_eq!(
            session.prepare(&request, &state),
            Err(FinalChainNativeSessionError::Aborted)
        );
    });
}

#[test]
fn zero_delegator_repeat_skips_the_account_lane_entirely() {
    let fixture = fixture();
    let expected = scenario(&fixture, "delegator_accrued");
    with_reference_chain("zero-delegator-account-read", |chain| {
        let initial = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let state = ReferenceState::from_snapshot(&initial.dpos_state);
        let mut session = rewarded_claim_session(chain, &state, expected);
        let first = fixture_request(2, 0, 0, &expected["transactions"][0]);
        let first_outcome =
            invoke_and_compare(&mut session, &state, &first, &expected["transactions"][0]);
        state.apply_outcome(&first_outcome);

        let second = fixture_request(2, 1, 1, &expected["transactions"][1]);
        let rejecting = RejectAccountReads(&state);
        let quote = session.prepare(&second, &rejecting).unwrap();
        let outcome = match session.invoke(&second, quote, &rejecting).unwrap() {
            FinalChainNativeInvocationResult::Completed(outcome) => outcome,
            result => panic!("zero delegator claim did not complete: {result:?}"),
        };
        assert_eq!(outcome.status, FinalChainNativeStatus::Success);
        assert!(outcome.account_mutations.is_empty());
        assert!(outcome.logs.is_empty());
        compare_raw_mutations(
            &outcome.raw_mutations,
            &[&expected["transactions"][1]["ordered_raw_writes"]],
        );
    });
}
