//! Direct Rust-session comparison with the pinned Go V1 custody corpus.

use super::*;
use ethereum_types::U256;
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
    "/../../../experiments/evm_feasibility/fixtures/native_v1_custody/public.json"
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
    accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
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
            accounts: BTreeMap::from([
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
            ]),
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
}

impl FinalChainNativeStateRead for ReferenceState {
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.accounts.get(&address).cloned().ok_or_else(|| {
            FinalChainNativeStateReadError::Invariant(format!(
                "V1 corpus account is unavailable: {address:?}"
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
    serde_json::from_str(ORACLE).expect("pinned Go V1 custody fixture is valid JSON")
}

fn scenario<'a>(fixture: &'a Value, name: &str) -> &'a Value {
    fixture["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|scenario| scenario["name"] == name)
        .unwrap_or_else(|| panic!("V1 custody fixture scenario is absent: {name}"))
}

fn transaction<'a>(scenario: &'a Value, name: &str) -> &'a Value {
    scenario["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|transaction| transaction["name"] == name)
        .unwrap_or_else(|| panic!("V1 custody fixture transaction is absent: {name}"))
}

fn temp_db_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustaxa-consensus-v1-corpus-{test_name}-{}-{nanos}",
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
    let rewards = FinalChainRewardsConfig {
        magnolia_period: enabled_at_genesis(scenario["magnolia"].as_bool().unwrap()),
        fix_claim_all_block_num: FinalChainBlockNumber::GENESIS,
        fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
        dpos_blocks_per_year: 1,
        dpos_delegation_locking_period: 2,
        cornus_period: FinalChainBlockNumber::GENESIS,
        cornus_delegation_locking_period: scenario["cornus_lock"].as_u64().unwrap(),
        cacti_period: enabled_at_genesis(scenario["cacti"].as_bool().unwrap()),
        cacti_delegation_locking_period: scenario["cacti_lock"].as_u64().unwrap(),
        rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
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
        enabled_at_genesis(scenario["ficus"].as_bool().unwrap()),
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
    let action_gas = if selector == DPOS_UNDELEGATE_SELECTOR {
        let amount = if expected["name"] == "duplicate_v1" {
            U256::one()
        } else {
            U256::from_dec_str(scenario["amount"].as_str().unwrap()).unwrap()
        };
        input.extend_from_slice(&amount.to_big_endian());
        DPOS_UNDELEGATE_GAS
    } else if selector == DPOS_CONFIRM_UNDELEGATE_SELECTOR {
        DPOS_DEFAULT_METHOD_GAS
    } else {
        panic!("unexpected V1 corpus selector: {selector:?}")
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
        caller: DELEGATOR,
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(BigUint::default()),
        input,
        supplied_gas: action_gas.into(),
    }
}

fn invoke_and_compare(
    session: &mut FinalChainNativeSession<'_>,
    state: &ReferenceState,
    request: &FinalChainNativeRequest,
    expected: &Value,
    action_gas: u64,
) -> FinalChainNativeOutcome {
    assert_eq!(expected["consensus_error"], "");
    assert_eq!(hex_bytes(&request.input[..4]), expected["selector"]);

    let quote = session.prepare(request, state).unwrap();
    assert_eq!(quote.required_gas.as_u64(), action_gas);
    assert_eq!(quote.invocation, request.id);
    assert_eq!(
        intrinsic_gas(&request.input, false).unwrap() + action_gas,
        expected["gas_used"].as_u64().unwrap()
    );
    let outcome = match session.invoke(request, quote, state).unwrap() {
        FinalChainNativeInvocationResult::Completed(outcome) => outcome,
        result => panic!("V1 corpus invocation did not complete: {result:?}"),
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

fn assert_confirm_accounts(outcome: &FinalChainNativeOutcome, scenario: &Value) {
    let before = &scenario["before"];
    let after = &scenario["after_confirm_block"];
    assert_eq!(outcome.account_mutations.len(), 2);
    for (actual, address, expected_before, expected_after) in [
        (
            &outcome.account_mutations[0],
            DPOS_CONTRACT_ADDRESS,
            before["contract_balance"].as_str().unwrap(),
            after["contract_balance"].as_str().unwrap(),
        ),
        (
            &outcome.account_mutations[1],
            DELEGATOR,
            before["delegator_balance"].as_str().unwrap(),
            after["delegator_balance"].as_str().unwrap(),
        ),
    ] {
        assert_eq!(
            actual,
            &FinalChainNativeOrdinaryMutation::BalanceReplace {
                address,
                expected_exists: true,
                expected: expected_before.parse::<BigInt>().unwrap(),
                replacement: expected_after.parse::<BigInt>().unwrap(),
            }
        );
    }
}

fn begin_later_fixture_session<'a>(
    chain: &'a FinalChain,
    mut prior: FinalChainNativeSession<'a>,
    period: u64,
) -> FinalChainNativeSession<'a> {
    // This bounded comparison advances private test state across the fixture's
    // empty blocks and resets the block-local call sequence. It does not stand
    // in for FinalChain commit, reopen, or finalization coverage.
    for block in 2..=period {
        chain
            .advance_reward_reference_graph_block(&mut prior.dpos_state, block.into())
            .unwrap();
    }
    let period = FinalChainBlockNumber::new(period);
    prior.pending_period = period;
    prior.period_start_total_vote_count = prior.dpos_state.total_vote_count;
    prior.period_start_amount_delegated = total_staked_amount(&prior.dpos_state).unwrap();
    // The intervening Go blocks are empty, so their delayed eligibility view
    // has the same custody state. Queries are not exercised by this corpus.
    prior.eligibility_state = prior.dpos_state.clone();
    prior.eligibility_period = FinalChainBlockNumber::new(period.as_u64() - 1);
    prior.next_sequence = 0;
    prior.prepared = None;
    prior.aborted = false;
    prior.finished_rewards = false;
    prior
}

#[test]
fn supported_v1_sessions_match_pinned_go_outcomes_and_raw_traces() {
    let fixture = fixture();
    assert_eq!(fixture["schema"], 1);
    assert_eq!(
        fixture["selectors"]["undelegate"],
        hex_bytes(DPOS_UNDELEGATE_SELECTOR)
    );
    assert_eq!(
        fixture["selectors"]["confirm_undelegate"],
        hex_bytes(DPOS_CONFIRM_UNDELEGATE_SELECTOR)
    );

    for name in [
        "magnolia_ficus_partial_cornus_lock",
        "magnolia_ficus_terminal",
        "magnolia_pre_ficus_partial",
        "cacti_lock_priority",
    ] {
        let scenario = scenario(&fixture, name);
        with_reference_chain(scenario, |chain| {
            let mut session = chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let state = ReferenceState::from_snapshot(&session.dpos_state);
            assert_snapshot(&session.dpos_state, &scenario["before"]);
            assert_eq!(scenario["before"]["delegator_balance"], "1000");
            assert_eq!(scenario["before"]["contract_balance"], "1000");

            let undelegate = transaction(scenario, "undelegate");
            let request = fixture_request(1, 0, 0, scenario, undelegate);
            let outcome = invoke_and_compare(
                &mut session,
                &state,
                &request,
                undelegate,
                fixture["action_gas"]["undelegate"].as_u64().unwrap(),
            );
            assert!(outcome.account_mutations.is_empty());
            assert_snapshot(&session.dpos_state, &scenario["after_undelegate_block"]);
            state.apply_raw(&outcome);

            let duplicate = transaction(scenario, "duplicate_v1");
            let request = fixture_request(1, 1, 1, scenario, duplicate);
            let outcome = invoke_and_compare(
                &mut session,
                &state,
                &request,
                duplicate,
                fixture["action_gas"]["undelegate"].as_u64().unwrap(),
            );
            assert!(outcome.account_mutations.is_empty());

            let early = transaction(scenario, "early_confirm");
            let request = fixture_request(1, 2, 2, scenario, early);
            let outcome = invoke_and_compare(
                &mut session,
                &state,
                &request,
                early,
                fixture["action_gas"]["confirm_undelegate"]
                    .as_u64()
                    .unwrap(),
            );
            assert!(outcome.account_mutations.is_empty());

            let unlock = scenario["unlock_period"].as_u64().unwrap();
            session = begin_later_fixture_session(chain, session, unlock);
            let mature = transaction(scenario, "mature_confirm");
            let request = fixture_request(unlock, 0, 0, scenario, mature);
            let outcome = invoke_and_compare(
                &mut session,
                &state,
                &request,
                mature,
                fixture["action_gas"]["confirm_undelegate"]
                    .as_u64()
                    .unwrap(),
            );
            assert_confirm_accounts(&outcome, scenario);
            assert_snapshot(&session.dpos_state, &scenario["after_confirm_block"]);
            state.apply_raw(&outcome);

            let missing = transaction(scenario, "missing_confirm");
            let request = fixture_request(unlock, 1, 1, scenario, missing);
            let outcome = invoke_and_compare(
                &mut session,
                &state,
                &request,
                missing,
                fixture["action_gas"]["confirm_undelegate"]
                    .as_u64()
                    .unwrap(),
            );
            assert!(outcome.account_mutations.is_empty());
        });
    }
}

#[test]
fn pre_magnolia_fixture_remains_explicitly_outside_staged_custody() {
    let fixture = fixture();
    let scenario = scenario(&fixture, "pre_magnolia_terminal");
    assert_eq!(scenario["magnolia"], false);
    with_reference_chain(scenario, |chain| {
        let mut session = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let state = ReferenceState::from_snapshot(&session.dpos_state);
        let expected = transaction(scenario, "undelegate");
        let request = fixture_request(1, 0, 0, scenario, expected);
        let quote = session.prepare(&request, &state).unwrap();
        assert_eq!(
            session.invoke(&request, quote, &state),
            Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
        );
    });
}
