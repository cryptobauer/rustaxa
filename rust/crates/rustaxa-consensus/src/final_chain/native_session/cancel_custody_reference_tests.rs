//! Direct Rust-session comparison with pinned same-period and accrued-reward
//! Go cancellation custody corpora.

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
    "/../../../experiments/evm_feasibility/fixtures/native_cancel_custody/public.json"
));
const REWARD_ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../experiments/evm_feasibility/fixtures/native_cancel_reward/public.json"
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

type RawRows = BTreeMap<([u8; 20], [u8; 32]), ConcreteRead<Vec<u8>>>;

struct ReferenceState {
    rows: RefCell<RawRows>,
    accounts: RefCell<BTreeMap<[u8; 20], FinalChainNativeAccount>>,
}

impl ReferenceState {
    fn from_snapshot(snapshot: &DposSnapshot) -> Self {
        let rows = canonical_concrete_precompile_storage(snapshot, true)
            .expect("reference snapshot has canonical raw rows")
            .into_iter()
            .map(|(identity, candidates)| {
                (
                    identity,
                    ConcreteRead::Present(
                        candidates
                            .into_iter()
                            .next()
                            .expect("canonical raw row has at least one encoding"),
                    ),
                )
            })
            .collect();
        Self {
            rows: RefCell::new(rows),
            accounts: RefCell::new(BTreeMap::from([
                (
                    DPOS_CONTRACT_ADDRESS,
                    FinalChainNativeAccount {
                        exists: true,
                        nonce: FinalChainNonce::zero(),
                        balance: BigInt::from(1_000_u64),
                    },
                ),
                (
                    DELEGATOR,
                    FinalChainNativeAccount {
                        exists: true,
                        nonce: FinalChainNonce::zero(),
                        balance: BigInt::from(1_000_u64),
                    },
                ),
            ])),
        }
    }

    fn apply_raw(&self, outcome: &FinalChainNativeOutcome) {
        for mutation in &outcome.raw_mutations {
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
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address,
                    expected_exists,
                    expected,
                    replacement,
                } => {
                    let account = accounts.get_mut(address).unwrap_or_else(|| {
                        panic!("cancellation corpus account is absent: {address:?}")
                    });
                    assert_eq!(account.exists, *expected_exists);
                    assert_eq!(&account.balance, expected);
                    account.exists = true;
                    account.balance = replacement.clone();
                }
                effect => panic!("unexpected cancellation corpus account effect: {effect:?}"),
            }
        }
    }

    fn apply_rewards(&self, outcome: &FinalChainNativeRewardsOutcome) {
        for mutation in &outcome.raw_mutations {
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
        self.apply_accounts(&outcome.account_mutations);
    }

    fn balance(&self, address: [u8; 20]) -> BigInt {
        self.accounts
            .borrow()
            .get(&address)
            .unwrap_or_else(|| panic!("cancellation corpus account is absent: {address:?}"))
            .balance
            .clone()
    }

    fn remove_account(&self, address: [u8; 20]) {
        self.accounts.borrow_mut().remove(&address);
    }

    fn replace_balance(&self, address: [u8; 20], balance: BigInt) {
        self.accounts
            .borrow_mut()
            .get_mut(&address)
            .unwrap_or_else(|| panic!("cancellation corpus account is absent: {address:?}"))
            .balance = balance;
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
                    "cancellation corpus account is unavailable: {address:?}"
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

fn fixture() -> Value {
    serde_json::from_str(ORACLE).expect("pinned Go cancellation custody fixture is valid JSON")
}

fn reward_fixture() -> Value {
    serde_json::from_str(REWARD_ORACLE)
        .expect("pinned Go cancellation reward fixture is valid JSON")
}

fn scenario<'a>(fixture: &'a Value, name: &str) -> &'a Value {
    fixture["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|scenario| scenario["name"] == name)
        .unwrap_or_else(|| panic!("cancellation custody fixture scenario is absent: {name}"))
}

fn transaction<'a>(scenario: &'a Value, name: &str) -> &'a Value {
    scenario["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|transaction| transaction["name"] == name)
        .unwrap_or_else(|| panic!("cancellation custody fixture transaction is absent: {name}"))
}

fn temp_db_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustaxa-consensus-cancel-corpus-{test_name}-{}-{nanos}",
        std::process::id()
    ))
}

fn with_reference_chain(scenario: &Value, test: impl FnOnce(&FinalChain)) {
    let name = scenario["name"].as_str().unwrap();
    let path = temp_db_path(name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let enabled_at_genesis = |enabled| {
        if enabled {
            FinalChainBlockNumber::GENESIS
        } else {
            FinalChainBlockNumber::MAX
        }
    };
    let reward_case = scenario["timing"].is_string();
    let magnolia = scenario["magnolia"].as_bool().unwrap_or(true);
    let rewards = FinalChainRewardsConfig {
        magnolia_period: enabled_at_genesis(magnolia),
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
        yield_percentage: if reward_case { 20 } else { 0 },
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
    scenario: &Value,
    expected: &Value,
) -> FinalChainNativeRequest {
    let selector = hex_decode(expected["selector"].as_str().unwrap());
    let mut input = selector.clone();
    input.extend_from_slice(&[0; 12]);
    input.extend_from_slice(&VALIDATOR);
    let name = expected["name"].as_str().unwrap();
    if selector == DPOS_UNDELEGATE_SELECTOR {
        let amount = U256::from_dec_str(scenario["amount"].as_str().unwrap()).unwrap();
        input.extend_from_slice(&amount.to_big_endian());
    } else if selector == DPOS_CANCEL_UNDELEGATE_SELECTOR {
    } else if selector == DPOS_UNDELEGATE_V2_SELECTOR {
        let amount = match name {
            "undelegate_v2" => U256::from_dec_str(scenario["amount"].as_str().unwrap()).unwrap(),
            "undelegate_v2_id_1" => U256::from(200_u64),
            "undelegate_v2_id_2" => U256::from(300_u64),
            _ => panic!("unexpected V2 undelegation fixture transaction: {name}"),
        };
        input.extend_from_slice(&amount.to_big_endian());
    } else if selector == DPOS_CANCEL_UNDELEGATE_V2_SELECTOR {
        let id = match name {
            "cancel_v2" => 1_u64,
            "cancel_v2_id_1" => 1_u64,
            "cancel_v2_id_2" => 2_u64,
            "cancel_v2_missing" => 99_u64,
            _ => panic!("unexpected V2 cancellation fixture transaction: {name}"),
        };
        input.extend_from_slice(&U256::from(id).to_big_endian());
    } else {
        panic!("unexpected cancellation corpus selector: {selector:?}");
    }
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: FinalChainTransactionPosition::new(position),
            sequence,
        },
        period: period.into(),
        depth: 1,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: DELEGATOR,
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(BigUint::default()),
        input,
        supplied_gas: DPOS_UNDELEGATE_GAS.into(),
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
    assert_eq!(quote.required_gas.as_u64(), DPOS_UNDELEGATE_GAS);
    assert_eq!(quote.invocation, request.id);
    assert_eq!(
        intrinsic_gas(&request.input, false).unwrap() + DPOS_UNDELEGATE_GAS,
        expected["gas_used"].as_u64().unwrap()
    );
    let outcome = match session.invoke(request, quote, state).unwrap() {
        FinalChainNativeInvocationResult::Completed(outcome) => outcome,
        result => panic!("cancellation corpus invocation did not complete: {result:?}"),
    };
    assert_eq!(outcome.gas_used.as_u64(), DPOS_UNDELEGATE_GAS);
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

    let expected_writes = expected["ordered_raw_writes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    assert_eq!(outcome.raw_mutations.len(), expected_writes.len());
    for (actual, expected) in outcome.raw_mutations.iter().zip(expected_writes) {
        let value = match &actual.operation {
            FinalChainNativeRawOperation::Put(value) => value.as_bytes(),
            FinalChainNativeRawOperation::Delete => &[],
        };
        assert_eq!(hex_bytes(actual.address), expected["address"]);
        assert_eq!(hex_bytes(actual.key.0), expected["key"]);
        assert_eq!(hex_bytes(value), expected["value"]);
    }
    outcome
}

fn assert_snapshot(snapshot: &DposSnapshot, expected: &Value) {
    assert_eq!(
        snapshot
            .total_stakes
            .get(&VALIDATOR)
            .map(StoredDposTokenAmount::as_u256)
            .unwrap_or_default()
            .to_string(),
        expected["validator_stake"]
    );
    assert_eq!(
        total_staked_amount(snapshot).unwrap().to_string(),
        expected["total_delegated"]
    );
    assert_eq!(
        snapshot.total_stakes.contains_key(&VALIDATOR),
        expected["validator_exists"].as_bool().unwrap()
    );
}

fn assert_accounts(state: &ReferenceState, expected: &Value) {
    assert_eq!(
        state.balance(DELEGATOR).to_string(),
        expected["delegator_balance"]
    );
    assert_eq!(
        state.balance(DPOS_CONTRACT_ADDRESS).to_string(),
        expected["contract_balance"]
    );
}

fn accrued_reward_plan(
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

fn compare_raw_mutations(actual: &[FinalChainNativeRawMutation], expected_groups: &[&Value]) {
    let expected = expected_groups
        .iter()
        .flat_map(|group| {
            group
                .as_array()
                .unwrap_or_else(|| panic!("raw fixture group is not an array"))
        })
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

fn advance_unpublished_semantic_session<'a>(
    chain: &'a FinalChain,
    mut session: FinalChainNativeSession<'a>,
    period: FinalChainBlockNumber,
) -> FinalChainNativeSession<'a> {
    // This advances the complete session snapshot produced by the real custody
    // and reward kernels. It deliberately grants no scheduler, persistence, or
    // publication authority and therefore does not prove commit/reopen parity.
    chain
        .advance_reward_reference_graph_block(&mut session.dpos_state, period)
        .unwrap();
    session.pending_period = period;
    session.period_start_total_vote_count = session.dpos_state.total_vote_count;
    session.period_start_amount_delegated = total_staked_amount(&session.dpos_state).unwrap();
    session.eligibility_period = FinalChainBlockNumber::new(period.as_u64() - 1);
    session.eligibility_state = session.dpos_state.clone();
    session.request_id = None;
    session.next_sequence = 0;
    session.prepared = None;
    session.aborted = false;
    session.finished_rewards = false;
    session
}

fn run_fixture_transactions(
    session: &mut FinalChainNativeSession<'_>,
    state: &ReferenceState,
    scenario: &Value,
) {
    for (position, expected) in scenario["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let request = fixture_request(1, position as u32, position as u64, scenario, expected);
        let outcome = invoke_and_compare(session, state, &request, expected);
        assert!(outcome.account_mutations.is_empty());
        if outcome.status == FinalChainNativeStatus::Success {
            state.apply_raw(&outcome);
        }
    }
}

#[test]
fn supported_cancellation_sessions_match_pinned_go_outcomes_and_raw_traces() {
    let fixture = fixture();
    assert_eq!(fixture["schema"], 1);
    assert_eq!(
        fixture["selectors"]["cancel_v1"],
        hex_bytes(DPOS_CANCEL_UNDELEGATE_SELECTOR)
    );
    assert_eq!(
        fixture["selectors"]["cancel_v2"],
        hex_bytes(DPOS_CANCEL_UNDELEGATE_V2_SELECTOR)
    );
    assert_eq!(fixture["action_gas"]["cancel_v1"], DPOS_UNDELEGATE_GAS);
    assert_eq!(fixture["action_gas"]["cancel_v2"], DPOS_UNDELEGATE_GAS);

    for name in [
        "v1_partial_existing_delegation",
        "v1_full_recreated_delegation",
        "v1_missing_queue",
        "v2_non_last_then_last",
    ] {
        let scenario = scenario(&fixture, name);
        with_reference_chain(scenario, |chain| {
            let mut session = chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let state = ReferenceState::from_snapshot(&session.dpos_state);
            assert_snapshot(&session.dpos_state, &scenario["before"]);
            run_fixture_transactions(&mut session, &state, scenario);
            assert_snapshot(&session.dpos_state, &scenario["after_block"]);
        });
    }
}

#[test]
fn accrued_reward_cancellation_matches_two_period_go_session_composition() {
    let fixture = reward_fixture();
    assert_eq!(fixture["schema"], 1);
    for name in ["v1_same_period_reward", "v2_same_period_reward"] {
        let scenario = scenario(&fixture, name);
        assert_eq!(scenario["timing"], "undelegate_then_reward_same_period");
        with_reference_chain(scenario, |chain| {
            let request_id = [0x91; 32];
            let mut session = chain
                .begin_native_session_bound(request_id, 1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let state = ReferenceState::from_snapshot(&session.dpos_state);
            let transactions = scenario["transactions"].as_array().unwrap();
            assert_snapshot(&session.dpos_state, &scenario["before"]);
            assert_accounts(&state, &scenario["before"]);

            let undelegate_request = fixture_request(1, 0, 0, scenario, &transactions[0]);
            let undelegated =
                invoke_and_compare(&mut session, &state, &undelegate_request, &transactions[0]);
            assert!(undelegated.account_mutations.is_empty());
            state.apply_raw(&undelegated);

            let rewards = session
                .finish_rewards(&accrued_reward_plan(chain, request_id), &state)
                .unwrap();
            assert_eq!(rewards.total_reward.as_u256(), U256::from(140_u64));
            compare_raw_mutations(
                &rewards.raw_mutations,
                &[
                    &scenario["reward_ordered_raw_writes"],
                    &scenario["reward_end_block_ordered_raw_writes"],
                ],
            );
            assert_eq!(rewards.account_mutations.len(), 1);
            state.apply_rewards(&rewards);
            assert_snapshot(&session.dpos_state, &scenario["after_reward"]);
            assert_eq!(
                state.balance(DPOS_CONTRACT_ADDRESS),
                BigInt::from(1_140_u64)
            );

            session = advance_unpublished_semantic_session(chain, session, 2.into());
            let cancel_request = fixture_request(2, 0, 0, scenario, &transactions[1]);
            let canceled =
                invoke_and_compare(&mut session, &state, &cancel_request, &transactions[1]);
            assert_eq!(canceled.account_mutations.len(), 2);
            state.apply_accounts(&canceled.account_mutations);
            assert_eq!(
                state.balance(DPOS_CONTRACT_ADDRESS),
                BigInt::from(1_002_u64)
            );
            assert_eq!(state.balance(DELEGATOR), BigInt::from(1_138_u64));
            assert_snapshot(&session.dpos_state, &scenario["after_cancel"]);
        });
    }
}

#[test]
fn accrued_reward_cancellation_rejects_missing_or_underfunded_custody() {
    // A valid Go lifecycle funds custody with every minted reward, so these
    // deliberately inconsistent account views cover Rust infrastructure error
    // typing without presenting either state as a reachable Go contract result.
    let fixture = reward_fixture();
    for (name, missing) in [
        ("v1_same_period_reward", true),
        ("v2_same_period_reward", false),
    ] {
        let scenario = scenario(&fixture, name);
        with_reference_chain(scenario, |chain| {
            let request_id = [0x92; 32];
            let mut session = chain
                .begin_native_session_bound(request_id, 1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let state = ReferenceState::from_snapshot(&session.dpos_state);
            let transactions = scenario["transactions"].as_array().unwrap();
            assert_snapshot(&session.dpos_state, &scenario["before"]);
            assert_accounts(&state, &scenario["before"]);
            let undelegate_request = fixture_request(1, 0, 0, scenario, &transactions[0]);
            let undelegated =
                invoke_and_compare(&mut session, &state, &undelegate_request, &transactions[0]);
            state.apply_raw(&undelegated);
            let rewards = session
                .finish_rewards(&accrued_reward_plan(chain, request_id), &state)
                .unwrap();
            state.apply_rewards(&rewards);
            session = advance_unpublished_semantic_session(chain, session, 2.into());

            if missing {
                state.remove_account(DPOS_CONTRACT_ADDRESS);
            } else {
                state.replace_balance(DPOS_CONTRACT_ADDRESS, BigInt::from(137_u64));
            }
            let snapshot_before = session.dpos_state.clone();
            let rows_before = state.rows.borrow().clone();
            let cancel_request = fixture_request(2, 0, 0, scenario, &transactions[1]);
            let quote = session.prepare(&cancel_request, &state).unwrap();
            let error = session.invoke(&cancel_request, quote, &state).unwrap_err();
            if missing {
                assert_eq!(
                    error,
                    FinalChainNativeSessionError::StateRead(
                        FinalChainNativeStateReadError::Invariant(format!(
                            "cancellation corpus account is unavailable: {DPOS_CONTRACT_ADDRESS:?}"
                        ))
                    )
                );
            } else {
                assert_eq!(
                    error,
                    FinalChainNativeSessionError::Domain(
                        "Rust FinalChain::finalize DPoS contract balance insufficient for reward claim"
                            .to_owned()
                    )
                );
            }
            assert_eq!(session.dpos_state, snapshot_before);
            assert_eq!(*state.rows.borrow(), rows_before);
        });
    }
}

#[test]
fn pre_magnolia_fixture_remains_explicitly_outside_staged_custody() {
    let fixture = fixture();
    let scenario = scenario(&fixture, "pre_magnolia_v1_missing_validator");
    assert_eq!(scenario["magnolia"], false);
    assert_eq!(
        transaction(scenario, "cancel_v1")["execution_error"],
        "Validator does not exist"
    );
    with_reference_chain(scenario, |chain| {
        let mut session = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        let expected = transaction(scenario, "undelegate_v1");
        let request = fixture_request(1, 0, 0, scenario, expected);
        let quote = session.prepare(&request, &state).unwrap();
        assert_eq!(
            session.invoke(&request, quote, &state),
            Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
        );
    });
}
