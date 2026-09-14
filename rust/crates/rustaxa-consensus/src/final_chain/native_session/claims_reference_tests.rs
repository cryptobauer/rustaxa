//! Direct Rust-session comparison with the pinned live Go reward-claim corpus.
//!
//! The tests execute Rust reward distribution before the claims, compare every
//! claim status, log, and ordered irreversible raw write, and apply the staged
//! ordinary-account effects to an exact full-width journal model. They do not
//! grant publication/reopen authority, and zero-stake commission remains an
//! explicitly rejected boundary until an actual Go witness is added.

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
    std::env::temp_dir().join(format!(
        "rustaxa-consensus-claims-corpus-{test_name}-{}-{nanos}",
        std::process::id()
    ))
}

fn with_reference_chain(test_name: &str, test: impl FnOnce(&FinalChain)) {
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
        vec![GenesisValidator {
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
    assert_eq!(actual.len(), expected.len());
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
fn zero_stake_commission_remains_explicitly_unsupported() {
    with_reference_chain("zero-stake", |chain| {
        let mut session = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        session
            .dpos_state
            .total_stakes
            .insert(VALIDATOR, StoredDposTokenAmount::default());
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        let fixture = fixture();
        let expected = &scenario(&fixture, "commission_accrued")["transactions"][1];
        let request = fixture_request(1, 0, 0, expected);
        let quote = session.prepare(&request, &state).unwrap();
        assert_eq!(
            session.invoke(&request, quote, &state),
            Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
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
