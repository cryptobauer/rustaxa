//! Direct Rust-session comparison with the pinned Go V1 custody corpus.

use super::*;
use ethereum_types::U256;
use num_bigint::{BigInt, BigUint};
use rustaxa_storage::{Config, Storage};
use rustaxa_types::{GenesisValidator, GenesisValidatorMetadata};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

const ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/native_v1_custody/public.json"
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
    crate::test_paths::new_path(&format!("rustaxa-consensus-v1-corpus-{test_name}"))
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
        enabled_at_genesis(scenario["ficus"].as_bool().unwrap()),
    )
    .unwrap();
    test(&chain);
    drop(chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
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

#[test]
fn pre_magnolia_v1_confirmation_preserves_surviving_validator_and_account_failures() {
    let fixture = fixture();
    for ficus in [false, true] {
        let mut scenario = scenario(&fixture, "pre_magnolia_terminal").clone();
        scenario["ficus"] = Value::Bool(ficus);
        for offset in [0, 1] {
            for failure in ["none", "missing", "underfunded"] {
                with_reference_chain(&scenario, |chain| {
                    let mut session = chain
                        .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                        .unwrap();
                    let initial = ReferenceState::from_snapshot(&session.dpos_state);
                    let mut accounts = account::StagedDposAccountPort::from_state(&initial);
                    let setup = chain
                        .apply_dpos_undelegate(
                            &mut session.dpos_state,
                            &mut accounts,
                            DELEGATOR,
                            VALIDATOR,
                            U256::from(500_u64).to_big_endian().to_vec(),
                            1.into(),
                        )
                        .unwrap();
                    assert_eq!(setup.status_code, 1);
                    let mut state = ReferenceState::from_snapshot(&session.dpos_state);
                    match failure {
                        "missing" => {
                            state.accounts.remove(&DELEGATOR);
                        }
                        "underfunded" => {
                            state
                                .accounts
                                .get_mut(&DPOS_CONTRACT_ADDRESS)
                                .unwrap()
                                .balance = BigInt::from(0);
                        }
                        _ => {}
                    }
                    let period = scenario["unlock_period"].as_u64().unwrap() + offset;
                    session = begin_later_fixture_session(chain, session, period);
                    let before = session.dpos_state.clone();
                    let rows = state.rows.borrow().clone();
                    let request = fixture_request(
                        period,
                        0,
                        0,
                        &scenario,
                        transaction(&scenario, "mature_confirm"),
                    );
                    let quote = session.prepare(&request, &state).unwrap();
                    let result = session.invoke(&request, quote, &state);
                    if failure != "none" {
                        assert!(result.is_err());
                        assert_eq!(session.dpos_state, before);
                        assert_eq!(*state.rows.borrow(), rows);
                    } else {
                        let FinalChainNativeInvocationResult::Completed(outcome) = result.unwrap()
                        else {
                            panic!("confirmation result")
                        };
                        assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                        assert_eq!(outcome.account_mutations.len(), 2);
                        assert!(
                            outcome
                                .raw_mutations
                                .iter()
                                .all(|mutation| mutation.key != custody::validator_key(VALIDATOR))
                        );
                        assert_eq!(
                            session.dpos_state.total_stakes.get(&VALIDATOR),
                            before.total_stakes.get(&VALIDATOR)
                        );
                        assert!(
                            find_undelegation(&session.dpos_state, DELEGATOR, VALIDATOR).is_none()
                        );
                    }
                });
            }
        }
    }
}
