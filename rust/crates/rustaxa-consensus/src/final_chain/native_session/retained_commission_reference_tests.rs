//! Retained zero-stake commission parity, full-width effects and local rollback.

use super::account::StagedDposAccountPort;
use super::*;
use ethereum_types::{H160, U256};
use num_bigint::{BigInt, BigUint};
use rustaxa_storage::{Config, Storage};
use rustaxa_types::concrete_state::{ConcreteReadError, ConcreteStateIdentity};
use rustaxa_types::transaction::intrinsic_gas;
use rustaxa_types::{GenesisValidator, GenesisValidatorMetadata};
use serde_json::Value;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const DELEGATOR: [u8; 20] = [
    119, 149, 44, 232, 60, 163, 202, 217, 247, 173, 207, 171, 237, 168, 91, 210, 241, 245, 32, 8,
];
const OWNER: [u8; 20] = [
    93, 92, 153, 237, 245, 41, 51, 81, 96, 255, 24, 15, 161, 65, 221, 73, 103, 252, 0, 210,
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
                    panic!("commission claims must not change account nonces")
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

fn temp_db_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    crate::closeout_test_scratch::new_path(
        std::env::var_os("RUN_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "rustaxa-consensus-claims-corpus-{test_name}-{}-{nanos}",
                std::process::id()
            )),
        true,
    )
}

fn with_reference_chain(test_name: &str, test: impl FnOnce(&FinalChain)) {
    with_fork_chain(test_name, FinalChainBlockNumber::GENESIS, test);
}

fn with_fork_chain(
    test_name: &str,
    phalaenopsis_period: FinalChainBlockNumber,
    test: impl FnOnce(&FinalChain),
) {
    with_counter_fork_chain(
        test_name,
        phalaenopsis_period,
        FinalChainBlockNumber::GENESIS,
        test,
    );
}

fn with_counter_fork_chain(
    test_name: &str,
    phalaenopsis_period: FinalChainBlockNumber,
    ficus_period: FinalChainBlockNumber,
    test: impl FnOnce(&FinalChain),
) {
    let path = temp_db_path(test_name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let rewards = FinalChainRewardsConfig {
        magnolia_period: FinalChainBlockNumber::GENESIS,
        phalaenopsis_period,
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
        ficus_period,
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

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_retained_commission/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_retained_commission/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    public
}

fn seed<'a>(
    chain: &'a FinalChain,
    public: &Value,
) -> (FinalChainNativeSession<'a>, ReferenceState) {
    let request_id = [0x93; 32];
    let mut session = chain
        .begin_native_session_bound(request_id, 1.into(), 0.into())
        .unwrap();
    let state = ReferenceState::from_snapshot(&session.dpos_state);
    let rewards = session
        .finish_rewards(&reward_plan(chain, request_id), &state)
        .unwrap();
    assert_eq!(rewards.total_reward.as_u256().to_string(), "200");
    state.apply_rewards(&rewards);
    let mut session = advance_unpublished_semantic_session(chain, session);
    let mut accounts = StagedDposAccountPort::from_state(&state);
    let outcome = chain
        .apply_dpos_undelegate(
            &mut session.dpos_state,
            &mut accounts,
            DELEGATOR,
            VALIDATOR,
            U256::from(1000).to_big_endian().to_vec(),
            2.into(),
        )
        .unwrap();
    assert_eq!(outcome.status_code, 1);
    state.apply_accounts(&accounts.into_mutations());
    assert_eq!(
        checked_undelegations_count(&session.dpos_state, VALIDATOR).unwrap(),
        1
    );
    assert_eq!(session.dpos_state.undelegations[&DELEGATOR][0].block, 5);
    assert_retained_rows(&session.dpos_state, &public["state_before"]);
    // Go raw bytes are independent read authority, not Rust-generated expectations.
    *state.rows.borrow_mut() = public["state_before"]["native_raw"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                (DPOS_CONTRACT_ADDRESS, hex_decode(k).try_into().unwrap()),
                ConcreteRead::Present(hex_decode(v.as_str().unwrap())),
            )
        })
        .collect();
    for row in public["state_before"]["accounts"].as_array().unwrap() {
        let a: [u8; 20] = hex_decode(row["address"].as_str().unwrap())
            .try_into()
            .unwrap();
        assert_eq!(state.balance(a).to_string(), row["balance"]);
    }
    chain
        .advance_reward_reference_graph_block(&mut session.dpos_state, 3.into())
        .unwrap();
    session.pending_period = 3.into();
    (session, state)
}

fn assert_retained_rows(snapshot: &DposSnapshot, expected: &Value) {
    assert_eq!(snapshot.total_stakes.len(), 1);
    assert!(snapshot.total_stakes[&VALIDATOR].is_zero());
    assert_eq!(snapshot.validator_metadata[&VALIDATOR].owner, OWNER);
    assert_eq!(checked_undelegations_count(snapshot, VALIDATOR).unwrap(), 1);
    let canonical = canonical_concrete_precompile_storage(snapshot, true).unwrap();
    for (key, value) in expected["native_raw"].as_object().unwrap() {
        let key: [u8; 32] = hex_decode(key).try_into().unwrap();
        let bytes = hex_decode(value.as_str().unwrap());
        assert!(
            canonical
                .get(&(DPOS_CONTRACT_ADDRESS, key))
                .map_or(bytes == vec![0; 4], |values| values.contains(&bytes)),
            "retained row {}",
            hex_bytes(key)
        );
    }
}

fn request(sequence: u64, case: &Value) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: 0.into(),
            sequence,
        },
        period: 3.into(),
        depth: 1,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: hex_decode(case["sender"].as_str().unwrap())
            .try_into()
            .unwrap(),
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(BigUint::default()),
        input: hex_decode(case["input"].as_str().unwrap()),
        supplied_gas: 20000.into(),
    }
}

fn invoke(
    session: &mut FinalChainNativeSession<'_>,
    state: &ReferenceState,
    sequence: u64,
    case: &Value,
) -> FinalChainNativeOutcome {
    let req = request(sequence, case);
    let quote = session.prepare(&req, state).unwrap();
    let FinalChainNativeInvocationResult::Completed(out) =
        session.invoke(&req, quote, state).unwrap()
    else {
        panic!("completed claim")
    };
    assert_eq!(
        intrinsic_gas(&req.input, false).unwrap() + out.gas_used.as_u64(),
        case["gas_used"].as_u64().unwrap()
    );
    assert_eq!(out.output, hex_decode(case["output"].as_str().unwrap()));
    let error = case["execution_error"].as_str().unwrap();
    assert_eq!(
        out.status,
        if error.is_empty() {
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: error.into(),
            }
        }
    );
    let logs=out.logs.iter().map(|l|json!({"address":hex_bytes(l.address),"topics":l.topics.iter().map(hex_bytes).collect::<Vec<_>>(),"data":hex_bytes(&l.data)})).collect::<Vec<_>>();
    assert_eq!(logs, *case["logs"].as_array().unwrap());
    let raw=out.raw_mutations.iter().map(|w|json!({"address":hex_bytes(w.address),"key":hex_bytes(w.key.0),"value":hex_bytes(match &w.operation{FinalChainNativeRawOperation::Put(v)=>v.as_bytes(),FinalChainNativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
    assert_eq!(raw, *case["ordered_raw_writes"].as_array().unwrap());
    out
}

#[test]
fn retained_commission_matches_go_ordered_writes_rows_and_full_width_accounts() {
    let public = corpus();
    with_reference_chain("retained-commission", |chain| {
        let (mut session, state) = seed(chain, &public);
        for (sequence, case) in public["cases"].as_array().unwrap().iter().enumerate() {
            let out = invoke(&mut session, &state, sequence as u64, case);
            if case["name"] == "positive" {
                assert_eq!(out.account_mutations.len(), 2);
            } else if case["name"] == "wrong_owner" {
                assert!(out.account_mutations.is_empty());
            }
            state.apply_outcome(&out);
        }
        assert_retained_rows(&session.dpos_state, &public["state_after"]);
        let actual = state
            .rows
            .borrow()
            .iter()
            .map(|((address, key), value)| {
                assert_eq!(*address, DPOS_CONTRACT_ADDRESS);
                let ConcreteRead::Present(bytes) = value else {
                    panic!("present Go row")
                };
                (hex_bytes(key), json!(hex_bytes(bytes)))
            })
            .collect::<serde_json::Map<String, Value>>();
        assert_eq!(
            actual,
            *public["state_after"]["native_raw"].as_object().unwrap()
        );

        assert_eq!(state.balance(OWNER), BigInt::from(1002));
        assert_eq!(state.balance(DPOS_CONTRACT_ADDRESS), BigInt::from(1000));
        let (mut session, state) = seed(chain, &public);
        let wide = BigInt::from(1u8) << 600usize;
        state.replace_balance(DPOS_CONTRACT_ADDRESS, wide.clone());
        state.replace_balance(OWNER, BigInt::from(-17));
        let out = invoke(&mut session, &state, 0, &public["cases"][1]);
        state.apply_outcome(&out);
        assert_eq!(state.balance(DPOS_CONTRACT_ADDRESS), wide - 2);
        assert_eq!(state.balance(OWNER), BigInt::from(-15));
        let effects = &public["wide_effects"]["accounts_after"];
        assert_eq!(
            state.balance(OWNER).to_string(),
            effects[hex_bytes(OWNER)]["balance"]
        );
        assert_eq!(
            state.balance(DPOS_CONTRACT_ADDRESS).to_string(),
            effects[hex_bytes(DPOS_CONTRACT_ADDRESS)]["balance"]
        );

        let (mut session, state) = seed(chain, &public);
        state.remove_account(OWNER);
        let before = session.dpos_state.clone();
        let req = request(0, &public["cases"][1]);
        let quote = session.prepare(&req, &state).unwrap();
        assert!(matches!(
            session.invoke(&req, quote, &state),
            Err(FinalChainNativeSessionError::StateRead(_))
        ));
        assert_eq!(session.dpos_state, before);
    });
}

struct Fault<'a> {
    state: &'a ReferenceState,
    mode: u8,
    identity: ConcreteStateIdentity,
}

struct KeyFault<'a> {
    fault: Fault<'a>,
    key: ConcreteStorageKey,
}
impl FinalChainNativeStateRead for KeyFault<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.fault.state.account(address)
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        if address == DPOS_CONTRACT_ADDRESS && *key == self.key {
            self.fault.raw_storage(address, key)
        } else {
            self.fault.state.raw_storage(address, key)
        }
    }
}
impl FinalChainNativeStateRead for Fault<'_> {
    fn account(
        &self,
        a: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.state.account(a)
    }
    fn raw_storage(
        &self,
        _: [u8; 20],
        _: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        match self.mode {
            0 => Err(FinalChainNativeStateReadError::State(
                ConcreteReadError::HistoryUnavailable(self.identity),
            )),
            1 => Ok(ConcreteRead::Present(vec![0xff])),
            _ => Ok(ConcreteRead::Absent),
        }
    }
}

#[test]
fn retained_commission_faults_preserve_precedence_and_local_rollback() {
    let public = corpus();
    for activation in [0_u64, 10] {
        with_fork_chain("retained-commission-faults", activation.into(), |chain| {
            for mode in 0..3 {
                let (mut session, state) = seed(chain, &public);
                let before = session.dpos_state.clone();
                let rows = state.rows.borrow().clone();
                let accounts = state.accounts.borrow().clone();
                // Wrong owner wins before the consumed reward read.
                let req = request(0, &public["cases"][0]);
                let fault = Fault {
                    state: &state,
                    mode,
                    identity: ConcreteStateIdentity {
                        period: 2.into(),
                        state_root: hex_decode(public["state_before"]["root"].as_str().unwrap())
                            .try_into()
                            .unwrap(),
                    },
                };
                let quote = session.prepare(&req, &fault).unwrap();
                assert!(matches!(
                    session.invoke(&req, quote, &fault).unwrap(),
                    FinalChainNativeInvocationResult::Completed(FinalChainNativeOutcome {
                        status: FinalChainNativeStatus::ContractFailure { .. },
                        ..
                    })
                ));
                let req = request(1, &public["cases"][1]);
                let quote = session.prepare(&req, &fault).unwrap();
                let error = session.invoke(&req, quote, &fault).unwrap_err();
                if mode == 0 {
                    assert_eq!(
                        error,
                        FinalChainNativeSessionError::StateRead(
                            FinalChainNativeStateReadError::State(
                                ConcreteReadError::HistoryUnavailable(fault.identity)
                            )
                        )
                    );
                } else {
                    assert!(matches!(
                        error,
                        FinalChainNativeSessionError::RawIntegrity(_)
                    ));
                }
                assert_eq!(session.dpos_state, before);
                assert_eq!(*state.rows.borrow(), rows);
                assert_eq!(*state.accounts.borrow(), accounts);
                assert_eq!(
                    session.prepare(&req, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                );
                let (mut fresh, state) = seed(chain, &public);
                let mut case = public["cases"][1].clone();
                if activation > 3 {
                    case["ordered_raw_writes"] = json!([]);
                }
                invoke(&mut fresh, &state, 0, &case);
            }
        });
    }
}

#[test]
fn retained_commission_before_phalaenopsis_repeats_pool_until_activation() {
    let public = corpus();
    with_fork_chain("retained-pre-phalaenopsis", 4.into(), |chain| {
        let (mut session, state) = seed(chain, &public);
        let before = session.dpos_state.clone();
        let raw_before = state.rows.borrow().clone();
        let owner_before = state.balance(OWNER);
        let contract_before = state.balance(DPOS_CONTRACT_ADDRESS);
        let reward = BigInt::from(BigUint::from_bytes_be(&u256_to_big_endian(
            before.commission_rewards[&VALIDATOR].amount().as_u256(),
        )));
        assert!(reward > BigInt::default());
        let mut pre_case = public["cases"][1].clone();
        pre_case["ordered_raw_writes"] = json!([]);
        for sequence in 0..2 {
            let outcome = invoke(&mut session, &state, sequence, &pre_case);
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            assert!(outcome.raw_mutations.is_empty());
            state.apply_outcome(&outcome);
            assert_eq!(session.dpos_state, before);
            assert_eq!(*state.rows.borrow(), raw_before);
            assert_eq!(
                checked_undelegations_count(&session.dpos_state, VALIDATOR).unwrap(),
                1
            );
        }
        assert_eq!(state.balance(OWNER), owner_before + &reward * 2);
        assert_eq!(
            state.balance(DPOS_CONTRACT_ADDRESS),
            contract_before - &reward * 2
        );

        session.pending_period = 4.into();
        let mut req = request(2, &public["cases"][1]);
        req.period = 4.into();
        let quote = session.prepare(&req, &state).unwrap();
        let FinalChainNativeInvocationResult::Completed(outcome) =
            session.invoke(&req, quote, &state).unwrap()
        else {
            panic!("completed activation claim")
        };
        assert_eq!(outcome.status, FinalChainNativeStatus::Success);
        assert_eq!(outcome.raw_mutations.len(), 1);
        assert!(matches!(
            outcome.raw_mutations[0].operation,
            FinalChainNativeRawOperation::Put(_)
        ));
        assert_eq!(
            outcome.logs[0].data,
            hex_decode(public["cases"][1]["logs"][0]["data"].as_str().unwrap())
        );
        state.apply_outcome(&outcome);
        assert!(session.dpos_state.commission_rewards[&VALIDATOR].is_zero());
        assert_eq!(
            checked_undelegations_count(&session.dpos_state, VALIDATOR).unwrap(),
            1
        );

        session.pending_period = 5.into();
        req.period = 5.into();
        req.id.sequence = 3;
        let quote = session.prepare(&req, &state).unwrap();
        let FinalChainNativeInvocationResult::Completed(outcome) =
            session.invoke(&req, quote, &state).unwrap()
        else {
            panic!("completed post-activation claim")
        };
        assert_eq!(outcome.status, FinalChainNativeStatus::Success);
        assert_eq!(outcome.raw_mutations.len(), 1);
        assert_eq!(outcome.logs[0].data, vec![0; 32]);
        assert!(session.dpos_state.total_stakes.contains_key(&VALIDATOR));
    });
}

fn mature_confirmation_request(sequence: u64, public: &Value) -> FinalChainNativeRequest {
    let mut req = request(sequence, &public["cases"][1]);
    req.period = 5.into();
    req.caller = DELEGATOR;
    req.input = DPOS_CONFIRM_UNDELEGATE_SELECTOR.to_vec();
    req.input.extend_from_slice(&[0; 12]);
    req.input.extend_from_slice(&VALIDATOR);
    req
}

#[test]
fn pre_ficus_confirmation_retains_authenticated_counter_and_commission_liveness() {
    let public = corpus();
    with_counter_fork_chain(
        "pre-ficus-counter",
        0.into(),
        FinalChainBlockNumber::MAX,
        |chain| {
            let (mut session, state) = seed(chain, &public);
            session.pending_period = 5.into();
            let rows_before = state.rows.borrow().clone();
            let req = mature_confirmation_request(0, &public);
            let quote = session.prepare(&req, &state).unwrap();
            let FinalChainNativeInvocationResult::Completed(out) =
                session.invoke(&req, quote, &state).unwrap()
            else {
                panic!("completed confirmation")
            };
            assert_eq!(out.status, FinalChainNativeStatus::Success);
            assert!(
                session
                    .dpos_state
                    .undelegations
                    .get(&DELEGATOR)
                    .is_none_or(Vec::is_empty)
            );
            assert_eq!(
                checked_undelegations_count(&session.dpos_state, VALIDATOR).unwrap(),
                1
            );
            assert_eq!(
                session
                    .dpos_state
                    .retained_undelegation_counts
                    .get(&VALIDATOR),
                Some(&StoredUndelegationCounter(1))
            );
            let key = custody::validator_key(VALIDATOR);
            assert!(out.raw_mutations.iter().all(|m| m.key != key));
            state.apply_outcome(&out);
            assert_eq!(
                state.rows.borrow().get(&(DPOS_CONTRACT_ADDRESS, key.0)),
                rows_before.get(&(DPOS_CONTRACT_ADDRESS, key.0))
            );
            let canonical =
                canonical_concrete_precompile_storage(&session.dpos_state, true).unwrap();
            let ConcreteRead::Present(actual) =
                state.rows.borrow()[&(DPOS_CONTRACT_ADDRESS, key.0)].clone()
            else {
                panic!("retained validator row")
            };
            assert!(canonical[&(DPOS_CONTRACT_ADDRESS, key.0)].contains(&actual));
            let encoded = encode_dpos_snapshot_rlp(&session.dpos_state).unwrap();
            assert_eq!(rlp::Rlp::new(&encoded).item_count().unwrap(), 26);
            assert_eq!(
                rlp::Rlp::new(&encoded)
                    .at(24)
                    .unwrap()
                    .item_count()
                    .unwrap(),
                1
            );
            session.dpos_state = decode_dpos_snapshot_rlp(&encoded).unwrap();
            // The retained count, rather than the now-empty queue, controls deletion.
            let mut claim = request(1, &public["cases"][1]);
            claim.period = 5.into();
            let quote = session.prepare(&claim, &state).unwrap();
            let FinalChainNativeInvocationResult::Completed(out) =
                session.invoke(&claim, quote, &state).unwrap()
            else {
                panic!("completed commission claim")
            };
            assert_eq!(out.status, FinalChainNativeStatus::Success);
            assert_eq!(
                out.logs[0].data,
                hex_decode(public["cases"][1]["logs"][0]["data"].as_str().unwrap())
            );
            assert!(session.dpos_state.total_stakes.contains_key(&VALIDATOR));
            assert_eq!(
                checked_undelegations_count(&session.dpos_state, VALIDATOR).unwrap(),
                1
            );
        },
    );
}

#[test]
fn pre_ficus_confirmation_raw_faults_abort_without_queue_or_account_changes() {
    let public = corpus();
    with_counter_fork_chain(
        "pre-ficus-counter-faults",
        0.into(),
        FinalChainBlockNumber::MAX,
        |chain| {
            for (key, mode) in [
                custody::validator_key(VALIDATOR),
                custody::rewards_key(VALIDATOR),
            ]
            .into_iter()
            .flat_map(|key| (0..3).map(move |mode| (key, mode)))
            {
                let (mut session, state) = seed(chain, &public);
                session.pending_period = 5.into();
                let before = session.dpos_state.clone();
                let rows = state.rows.borrow().clone();
                let accounts = state.accounts.borrow().clone();
                let fault = KeyFault {
                    key,
                    fault: Fault {
                        state: &state,
                        mode,
                        identity: ConcreteStateIdentity {
                            period: 2.into(),
                            state_root: hex_decode(
                                public["state_before"]["root"].as_str().unwrap(),
                            )
                            .try_into()
                            .unwrap(),
                        },
                    },
                };
                let req = mature_confirmation_request(0, &public);
                let quote = session.prepare(&req, &fault).unwrap();
                let error = session.invoke(&req, quote, &fault).unwrap_err();
                if mode == 0 {
                    assert_eq!(
                        error,
                        FinalChainNativeSessionError::StateRead(
                            FinalChainNativeStateReadError::State(
                                ConcreteReadError::HistoryUnavailable(fault.fault.identity)
                            )
                        )
                    );
                } else {
                    assert!(matches!(
                        error,
                        FinalChainNativeSessionError::RawIntegrity(_)
                    ));
                }
                assert_eq!(session.dpos_state, before);
                assert_eq!(*state.rows.borrow(), rows);
                assert_eq!(*state.accounts.borrow(), accounts);
                assert_eq!(session.next_sequence, 0);
                assert_eq!(
                    session.prepare(&req, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                );
                drop(session);
                let (mut retry, retry_state) = seed(chain, &public);
                retry.pending_period = 5.into();
                let quote = retry.prepare(&req, &retry_state).unwrap();
                let FinalChainNativeInvocationResult::Completed(out) =
                    retry.invoke(&req, quote, &retry_state).unwrap()
                else {
                    panic!("fresh confirmation retry must complete")
                };
                assert_eq!(out.status, FinalChainNativeStatus::Success);
            }
        },
    );
}

#[test]
fn retained_counter_zero_survives_cancel_then_confirm_and_codec_roundtrips() {
    let public = corpus();
    with_reference_chain("zero-counter-cancel-confirm", |chain| {
        let (session, _) = seed(chain, &public);
        let mut snapshot = session.dpos_state;
        // A bounded arithmetic fixture represents an explicitly retained zero;
        // it is not evidence that the pinned Go lifecycle generated this state.
        let id = create_undelegation_v2(
            &mut snapshot,
            DELEGATOR,
            VALIDATOR,
            StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
            5,
        )
        .unwrap();
        set_persisted_undelegations_count(&mut snapshot, VALIDATOR, 0).unwrap();
        let state = ReferenceState::from_snapshot(&snapshot);
        let mut accounts = StagedDposAccountPort::from_state(&state);
        let result = chain
            .apply_dpos_cancel_undelegate_v2(&mut snapshot, &mut accounts, DELEGATOR, VALIDATOR, id)
            .unwrap();
        assert_eq!(result.status_code, 1);
        assert_eq!(queue_undelegations_count(&snapshot, VALIDATOR), 1);
        assert_eq!(
            checked_undelegations_count(&snapshot, VALIDATOR).unwrap(),
            0
        );
        snapshot = decode_dpos_snapshot_rlp(&encode_dpos_snapshot_rlp(&snapshot).unwrap()).unwrap();
        let result = chain
            .apply_dpos_confirm_undelegate(
                &mut snapshot,
                &mut accounts,
                DELEGATOR,
                VALIDATOR,
                5.into(),
            )
            .unwrap();
        assert_eq!(result.status_code, 1);
        assert_eq!(queue_undelegations_count(&snapshot, VALIDATOR), 0);
        assert_eq!(
            checked_undelegations_count(&snapshot, VALIDATOR).unwrap(),
            0
        );
        assert!(snapshot.total_stakes.contains_key(&VALIDATOR));
        assert_eq!(
            decode_dpos_snapshot_rlp(&encode_dpos_snapshot_rlp(&snapshot).unwrap()).unwrap(),
            snapshot
        );
    });
}
