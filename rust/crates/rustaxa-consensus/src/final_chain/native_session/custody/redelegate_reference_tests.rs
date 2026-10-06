//! Actual Go parity prerequisites and authenticated staged session checks.
use super::*;
use rustaxa_storage::Config;
use rustaxa_types::GenesisValidatorMetadata;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::time::{SystemTime, UNIX_EPOCH};

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

fn address(last: u8) -> [u8; 20] {
    let mut result = [0; 20];
    result[19] = last;
    result
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct RawOnly(BTreeMap<ConcreteStorageKey, Vec<u8>>);
impl FinalChainNativeStateRead for RawOnly {
    fn raw_storage(
        &self,
        contract: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        assert_eq!(contract, DPOS_CONTRACT_ADDRESS);
        Ok(self
            .0
            .get(key)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("zero-reward redelegation must not read or mutate accounts")
    }
}

#[test]
fn redelegate_kernel_and_composed_serializers_match_actual_go_repeat() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    let case = &public["cases"][0];
    assert_eq!(case["name"], "partial_and_repeat");
    let mut raw = RawOnly(BTreeMap::new());
    for read in case["attempts"][0]["ordered_reads"].as_array().unwrap() {
        if read["present"].as_bool().unwrap() {
            raw.0.insert(
                ConcreteStorageKey(unhex(read["key"].as_str().unwrap()).try_into().unwrap()),
                unhex(read["value"].as_str().unwrap()),
            );
        }
    }
    let (chain, storage, path) = kernel_chain(1_000_000);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    for attempt in case["attempts"].as_array().unwrap() {
        let before = session.dpos_state.clone();
        let mut after = before.clone();
        let mut accounts = StagedDposAccountPort::from_state(&raw);
        let outcome = chain
            .apply_dpos_redelegate(
                &mut after,
                &mut accounts,
                address(0xd1),
                address(0x31),
                address(0x32),
                vec![1, 0x2c],
                1.into(),
            )
            .unwrap();
        assert_eq!(outcome.status_code, 1);
        assert!(outcome.contract_error.is_none());
        assert_eq!(
            after
                .total_stakes
                .values()
                .fold(U256::zero(), |total, stake| total + stake.as_u256()),
            U256::from(2000)
        );
        assert_eq!(after.total_vote_count, before.total_vote_count);
        assert_eq!(after.validator_order, before.validator_order);
        assert_eq!(after.delegator_validators, before.delegator_validators);
        assert_eq!(
            outcome.code_retval,
            unhex(attempt["output"].as_str().unwrap())
        );
        assert!(accounts.into_mutations().is_empty());
        let logs = outcome.logs.iter().map(|log| json!({"address": hex(&log.address), "topics": log.topics.iter().map(|topic| hex(topic)).collect::<Vec<_>>(), "data": hex(&log.data)})).collect::<Vec<_>>();
        assert_eq!(logs, *attempt["logs"].as_array().unwrap());
        let mut trace = FinalChainNativeRawTrace::new(&raw);
        session
            .serialize_undelegate_principal(
                address(0xd1),
                address(0x31),
                &before,
                &after,
                &mut trace,
            )
            .unwrap();
        session
            .serialize_delegate(address(0xd1), address(0x32), &before, &after, &mut trace)
            .unwrap();
        let mutations = trace.finish();
        let actual = mutations.iter().map(|mutation| {
            let value = match &mutation.operation { FinalChainNativeRawOperation::Put(value) => value.as_bytes(), FinalChainNativeRawOperation::Delete => &[] };
            json!({"address": hex(&mutation.address), "key": hex(&mutation.key.0), "value": hex(value)})
        }).collect::<Vec<_>>();
        assert_eq!(actual, *attempt["ordered_raw_writes"].as_array().unwrap());
        for mutation in mutations {
            let current = raw
                .0
                .get(&mutation.key)
                .cloned()
                .map_or(ConcreteRead::Absent, ConcreteRead::Present);
            assert_eq!(current, mutation.expected);
            match mutation.operation {
                FinalChainNativeRawOperation::Put(value) => {
                    raw.0.insert(mutation.key, value.into_bytes());
                }
                FinalChainNativeRawOperation::Delete => {
                    raw.0.insert(mutation.key, Vec::new());
                }
            }
        }
        session.dpos_state = after;
    }
    for (validator, field) in [(0x31, "source_stake"), (0x32, "destination_stake")] {
        assert_eq!(
            session.dpos_state.total_stakes[&address(validator)]
                .as_u256()
                .to_string(),
            case[field].as_str().unwrap()
        );
    }
    drop(session);
    assert_eq!(
        chain.dpos_total_amount_delegated(0.into()).unwrap(),
        vec![7, 0xd0]
    );
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

fn kernel_chain(maximum: u64) -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
    kernel_chain_destination(maximum, address(0xd1))
}

fn kernel_chain_destination(
    maximum: u64,
    destination_delegator: [u8; 20],
) -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
    kernel_chain_profile(maximum, destination_delegator, None)
}

/// Complete zero-reward seed; optional second source delegator retains a
/// validator after full caller-row removal. Existing helpers retain their seeds.
fn kernel_chain_profile(
    maximum: u64,
    destination_delegator: [u8; 20],
    retained_source: Option<u8>,
) -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "redelegate-composition-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        Vec::new(),
        [0x31, 0x32]
            .into_iter()
            .map(|validator| GenesisValidator {
                address: address(validator),
                vrf_key: [if validator == 0x31 { 0x44 } else { 0x55 }; 32],
                total_stake: U256::from(if retained_source == Some(validator) {
                    2000
                } else {
                    1000
                })
                .to_big_endian()
                .to_vec(),
                delegations: vec![(
                    if validator == 0x31 {
                        address(0xd1)
                    } else {
                        destination_delegator
                    },
                    U256::from(1000).to_big_endian().to_vec(),
                )]
                .into_iter()
                .chain(
                    (retained_source == Some(validator))
                        .then_some((address(0xa1), U256::from(1000).to_big_endian().to_vec())),
                )
                .collect(),
                metadata: GenesisValidatorMetadata {
                    owner: address(0xa1),
                    commission: 100,
                    ..Default::default()
                },
            })
            .collect(),
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(maximum).into(),
            minimum_deposit: U256::from(100).into(),
            delegation_delay: 1,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            yield_percentage: 0,
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    (chain, storage, path)
}

#[test]
fn redelegate_kernel_normal_preflight_errors_match_go_without_advancement() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/public.json"
    )))
    .unwrap();
    let names = [
        "destination_cap_before_insufficient_source",
        "missing_source",
        "missing_destination",
        "missing_source_delegation",
        "insufficient_source",
        "remainder_below_minimum",
        "same_validator",
    ];
    let raw = RawOnly(BTreeMap::new());
    for name in names {
        let case = public["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let maximum = if name == "destination_cap_before_insufficient_source" {
            1500
        } else {
            1_000_000
        };
        let (chain, storage, path) = kernel_chain(maximum);
        let session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let committed_before = chain.dpos_snapshot(0.into()).unwrap();
        let before = session.dpos_state.clone();
        let mut after = before.clone();
        let input = unhex(case["input"].as_str().unwrap());
        let mut accounts = StagedDposAccountPort::from_state(&raw);
        let outcome = chain
            .apply_dpos_redelegate(
                &mut after,
                &mut accounts,
                unhex(case["caller"].as_str().unwrap()).try_into().unwrap(),
                input[16..36].try_into().unwrap(),
                input[48..68].try_into().unwrap(),
                input[68..100].to_vec(),
                1.into(),
            )
            .unwrap();
        let expected = &case["attempts"][0];
        assert_eq!(outcome.status_code, 0, "{name}");
        assert_eq!(
            outcome.contract_error.unwrap().legacy_message(),
            expected["execution_error"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            outcome.code_retval,
            unhex(expected["output"].as_str().unwrap()),
            "{name}"
        );
        assert!(outcome.logs.is_empty(), "{name}");
        assert!(expected["logs"].as_array().unwrap().is_empty());
        assert!(
            expected["ordered_raw_writes"].is_null()
                || expected["ordered_raw_writes"]
                    .as_array()
                    .unwrap()
                    .is_empty()
        );
        assert!(accounts.into_mutations().is_empty(), "{name}");
        assert_eq!(after, before, "{name}");
        assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed_before);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

fn oracle() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/public.json"
    )))
    .unwrap()
}

struct AuthenticatedRaw {
    rows: BTreeMap<ConcreteStorageKey, Vec<u8>>,
    reads: RefCell<Vec<ConcreteStorageKey>>,
    fail_at: Option<usize>,
}
impl AuthenticatedRaw {
    fn from_attempt(attempt: &Value) -> Self {
        let mut rows = BTreeMap::new();
        for read in attempt["ordered_reads"].as_array().unwrap() {
            if read["present"].as_bool().unwrap() {
                rows.insert(
                    ConcreteStorageKey(unhex(read["key"].as_str().unwrap()).try_into().unwrap()),
                    unhex(read["value"].as_str().unwrap()),
                );
            }
        }
        Self {
            rows,
            reads: RefCell::new(Vec::new()),
            fail_at: None,
        }
    }
    fn apply(&mut self, mutations: &[FinalChainNativeRawMutation]) {
        for mutation in mutations {
            let current = self
                .rows
                .get(&mutation.key)
                .cloned()
                .map_or(ConcreteRead::Absent, ConcreteRead::Present);
            assert_eq!(current, mutation.expected);
            let value = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
                FinalChainNativeRawOperation::Delete => Vec::new(),
            };
            self.rows.insert(mutation.key, value);
        }
    }
}
impl FinalChainNativeStateRead for AuthenticatedRaw {
    fn raw_storage(
        &self,
        contract: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        assert_eq!(contract, DPOS_CONTRACT_ADDRESS);
        self.reads.borrow_mut().push(*key);
        if self.fail_at == Some(self.reads.borrow().len()) {
            return Err(FinalChainNativeStateReadError::Invariant(
                "injected raw reader failure".to_owned(),
            ));
        }
        Ok(self
            .rows
            .get(key)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("bounded redelegation must not access accounts")
    }
}

fn staged_request(case: &Value, sequence: u64) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: FinalChainTransactionPosition::new(sequence as u32),
            sequence,
        },
        period: 1.into(),
        depth: 1,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: unhex(case["caller"].as_str().unwrap()).try_into().unwrap(),
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::default(),
        input: unhex(case["input"].as_str().unwrap()),
        supplied_gas: 80_000.into(),
    }
}
fn completed(result: FinalChainNativeInvocationResult) -> FinalChainNativeOutcome {
    let FinalChainNativeInvocationResult::Completed(outcome) = result else {
        panic!("expected completed native action")
    };
    outcome
}
fn write_json(mutations: &[FinalChainNativeRawMutation]) -> Value {
    Value::Array(mutations.iter().map(|mutation| {
        let value = match &mutation.operation { FinalChainNativeRawOperation::Put(value) => value.as_bytes(), FinalChainNativeRawOperation::Delete => &[] };
        json!({"address": hex(&mutation.address), "key": hex(&mutation.key.0), "value": hex(value)})
    }).collect())
}
fn log_json(logs: &[FinalChainCallLog]) -> Value {
    Value::Array(logs.iter().map(|log| json!({"address": hex(&log.address), "topics": log.topics.iter().map(|topic| hex(topic)).collect::<Vec<_>>(), "data": hex(&log.data)})).collect())
}
fn install_historical(chain: &FinalChain) {
    let mut snapshot = chain.dpos_snapshot(0.into()).unwrap();
    chain
        .advance_reward_reference_graph_block(&mut snapshot, 1.into())
        .unwrap();
    chain.insert_dpos_snapshot(1.into(), snapshot).unwrap();
    chain
        .storage
        .final_chain()
        .write_block_header(
            1,
            ethereum_types::H256::from_low_u64_be(1),
            &[0xc0],
            &[0xc0],
        )
        .unwrap();
}

#[test]
fn redelegate_staged_pending_and_historical_match_actual_go_and_authenticate_every_call() {
    let oracle = oracle();
    let case = &oracle["cases"][0];
    for historical in [false, true] {
        let (chain, storage, path) = kernel_chain(1_000_000);
        if historical {
            install_historical(&chain);
        }
        let committed = chain
            .dpos_snapshot(if historical { 1.into() } else { 0.into() })
            .unwrap();
        let mut pending;
        let mut simulation;
        let session = if historical {
            simulation = chain.begin_native_simulation(1.into()).unwrap();
            &mut simulation.session
        } else {
            pending = chain.begin_native_session(1.into(), 0.into()).unwrap();
            &mut pending
        };
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        for (sequence, attempt) in case["attempts"].as_array().unwrap().iter().enumerate() {
            raw.reads.borrow_mut().clear();
            let request = staged_request(case, sequence as u64);
            let quote = session.prepare(&request, &raw).unwrap();
            assert_eq!(quote.required_gas, 80_000.into());
            assert!(raw.reads.borrow().is_empty());
            let outcome = completed(session.invoke(&request, quote, &raw).unwrap());
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            assert_eq!(
                write_json(&outcome.raw_mutations),
                attempt["ordered_raw_writes"]
            );
            assert_eq!(log_json(&outcome.logs), attempt["logs"]);
            assert_eq!(outcome.output, unhex(attempt["output"].as_str().unwrap()));
            assert!(outcome.account_mutations.is_empty());
            // Rust deliberately authenticates on both calls. Go's warm call
            // records no reads; this assertion makes no Go warm-cache claim.
            let reads = raw.reads.borrow();
            assert_eq!(reads.len(), if sequence == 0 { 14 } else { 12 });
            assert_eq!(
                reads
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                reads.len()
            );
            drop(reads);
            raw.apply(&outcome.raw_mutations);
            assert_eq!(
                chain
                    .dpos_snapshot(if historical { 1.into() } else { 0.into() })
                    .unwrap(),
                committed
            );
        }
        assert_eq!(
            session.dpos_state.total_stakes[&address(0x31)].as_u256(),
            U256::from(400)
        );
        assert_eq!(
            session.dpos_state.total_stakes[&address(0x32)].as_u256(),
            U256::from(1600)
        );
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn redelegate_staged_normal_failures_authenticate_actual_go_cold_prefix_before_scope() {
    let oracle = oracle();
    let names = [
        "destination_cap_before_insufficient_source",
        "missing_source",
        "missing_destination",
        "missing_source_delegation",
        "insufficient_source",
        "remainder_below_minimum",
        "same_validator",
    ];
    for name in names {
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let (chain, storage, path) =
            kernel_chain(if name == "destination_cap_before_insufficient_source" {
                1500
            } else {
                1_000_000
            });
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let before = session.dpos_state.clone();
        let attempt = &case["attempts"][0];
        let raw = AuthenticatedRaw::from_attempt(attempt);
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        let outcome = completed(session.invoke(&request, quote, &raw).unwrap());
        assert_eq!(
            outcome.status,
            FinalChainNativeStatus::ContractFailure {
                error: attempt["execution_error"].as_str().unwrap().to_owned()
            },
            "{name}"
        );
        assert!(
            outcome.account_mutations.is_empty()
                && outcome.raw_mutations.is_empty()
                && outcome.logs.is_empty()
        );
        assert_eq!(session.dpos_state, before);
        assert!(!session.aborted);
        let expected = attempt["ordered_reads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|read| {
                ConcreteStorageKey(unhex(read["key"].as_str().unwrap()).try_into().unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(*raw.reads.borrow(), expected, "{name}");
        // Authenticated semantic failure remains first even outside success scope.
        session.dpos_state.delegation_ledger_history_complete = false;
        raw.reads.borrow_mut().clear();
        let request = staged_request(case, 1);
        let quote = session.prepare(&request, &raw).unwrap();
        assert_eq!(
            completed(session.invoke(&request, quote, &raw).unwrap()).status,
            outcome.status
        );
        assert_eq!(*raw.reads.borrow(), expected);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn redelegate_staged_reader_and_all_authenticated_row_failures_abort_without_advancement() {
    let oracle = oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain(1_000_000);
    for read in 0..14 {
        for reader_failure in [false, true] {
            let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
            let before = session.dpos_state.clone();
            let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
            let key = ConcreteStorageKey(
                unhex(
                    case["attempts"][0]["ordered_reads"][read]["key"]
                        .as_str()
                        .unwrap(),
                )
                .try_into()
                .unwrap(),
            );
            if reader_failure {
                // Inject at each actual Rust read, independent of Go success order.
                raw.fail_at = Some(read + 1);
            } else {
                raw.rows.insert(key, vec![0xff]);
            }
            let request = staged_request(case, 0);
            let quote = session.prepare(&request, &raw).unwrap();
            let error = session.invoke(&request, quote, &raw).unwrap_err();
            assert!(
                if reader_failure {
                    matches!(error, FinalChainNativeSessionError::StateRead(_))
                } else {
                    matches!(error, FinalChainNativeSessionError::RawIntegrity(_))
                },
                "{read}: {error:?}"
            );
            assert_eq!(session.dpos_state, before);
            assert!(session.aborted && session.prepared.is_none());
            assert_eq!(session.next_sequence, 0);
        }
    }
    // Missing rows must also be authenticated before returning a normal error.
    for name in [
        "missing_source",
        "missing_destination",
        "missing_source_delegation",
    ] {
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        let absent = case["attempts"][0]["ordered_reads"]
            .as_array()
            .unwrap()
            .last()
            .unwrap();
        raw.rows.insert(
            ConcreteStorageKey(unhex(absent["key"].as_str().unwrap()).try_into().unwrap()),
            vec![0xff],
        );
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let before = session.dpos_state.clone();
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &raw),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_staged_excluded_successes_have_no_effects_or_advancement() {
    let oracle = oracle();
    let (chain, storage, path) = kernel_chain(1_000_000);
    for name in ["full_source"] {
        let case = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let before = session.dpos_state.clone();
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &raw),
            Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
        ));
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted);
    }
    for scope in 0..5 {
        let case = &oracle["cases"][0];
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        match scope {
            0 => {
                session
                    .dpos_state
                    .delegations
                    .get_mut(&address(0x32))
                    .unwrap()
                    .remove(&address(0xd1));
            }
            1 => {
                session.dpos_state.delegation_ledger_history_complete = false;
            }
            2 => {
                session
                    .dpos_state
                    .redelegate_same_validator_history_complete = false;
            }
            3 => {
                session.dpos_state.delegator_rewards.insert(
                    address(0x31),
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            4 => {
                session
                    .dpos_state
                    .validator_reward_per_stake
                    .insert(address(0x31), vec![1].into());
            }
            _ => unreachable!(),
        }
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        if scope == 3 {
            raw.rows.insert(
                rewards_key(address(0x31)),
                encode_rewards(&session.dpos_state, address(0x31)),
            );
        }
        let before = session.dpos_state.clone();
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        let result = session.invoke(&request, quote, &raw);
        if scope == 0 {
            // Principal deletion alone leaves corrupt aggregate/cursor/order.
            assert!(
                matches!(result, Err(FinalChainNativeSessionError::Domain(error))
                if error.contains("FINAL_CHAIN_DPOS_PRINCIPAL_AGGREGATE_MISMATCH"))
            );
        } else {
            assert!(
                matches!(
                    result,
                    Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
                ),
                "scope {scope}"
            );
        }
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_staged_repeat_rejects_stale_or_corrupted_overlay_without_advancement() {
    let oracle = oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain(1_000_000);
    // Every second invocation reads its current overlay again, including rows
    // that did not change (membership and rewards). Corrupt each warm key.
    for target in 0..12 {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        let first = staged_request(case, 0);
        let quote = session.prepare(&first, &raw).unwrap();
        let outcome = completed(session.invoke(&first, quote, &raw).unwrap());
        raw.apply(&outcome.raw_mutations);
        let before = session.dpos_state.clone();
        // Discover the bounded warm key order in a separate disposable session.
        let mut probe = chain.begin_native_session(1.into(), 0.into()).unwrap();
        probe.dpos_state = before.clone();
        probe.next_sequence = 1;
        raw.reads.borrow_mut().clear();
        let request = staged_request(case, 1);
        let quote = probe.prepare(&request, &raw).unwrap();
        probe.invoke(&request, quote, &raw).unwrap();
        let key = raw.reads.borrow()[target];
        raw.rows.insert(key, vec![0xff]);
        raw.reads.borrow_mut().clear();
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &raw),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted);
        assert_eq!(session.next_sequence, 1);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_staged_inactive_or_pre_fix_success_scope_is_explicit() {
    let oracle = oracle();
    let case = &oracle["cases"][0];
    for scope in 0..4 {
        let (mut chain, storage, path) = kernel_chain(1_000_000);
        match scope {
            0 => chain.ficus_activation_period = FinalChainBlockNumber::MAX,
            1 => chain.rewards_config.magnolia_period = FinalChainBlockNumber::MAX,
            2 => chain.rewards_config.fix_redelegate_block_num = 1.into(),
            3 => chain.rewards_config.fix_redelegate_block_num = 2.into(),
            _ => unreachable!(),
        }
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        for validator in [address(0x31), address(0x32)] {
            raw.rows.insert(
                validator_key(validator),
                session
                    .encode_validator_row(&session.dpos_state, validator)
                    .unwrap(),
            );
        }
        let before = session.dpos_state.clone();
        let mut request = staged_request(case, 0);
        request.depth = 0;
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(
            matches!(
                session.invoke(&request, quote, &raw),
                Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
            ),
            "scope {scope}"
        );
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn redelegate_selector_first_admission_and_abi_match_actual_go_frames() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_frames/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_frames/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    for case in public["cases"].as_array().unwrap() {
        let (mut chain, storage, path) = kernel_chain(1_000_000);
        chain.rewards_config.fix_redelegate_block_num = case["fix"].as_u64().unwrap().into();
        if case["aspen_zero"].as_bool().unwrap() {
            chain.rewards_config.aspen_part_two_period = 1.into();
        }
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let before = session.dpos_state.clone();
        let raw = AuthenticatedRaw {
            rows: case["prior_raw"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| {
                    (
                        ConcreteStorageKey(unhex(key).try_into().unwrap()),
                        unhex(value.as_str().unwrap()),
                    )
                })
                .collect(),
            reads: RefCell::new(Vec::new()),
            fail_at: None,
        };
        let mut request = staged_request(case, 0);
        request.depth = case["depth"].as_u64().unwrap() as u16;
        request.supplied_gas = case["supplied_native_gas"].as_u64().unwrap().into();
        request.value = FinalChainNativeValue::new(case["value"].as_u64().unwrap().into());
        let quote = session.prepare(&request, &raw).unwrap();
        assert_eq!(
            quote.required_gas.as_u64(),
            case["required_gas"].as_u64().unwrap(),
            "{}",
            case["name"]
        );
        assert!(raw.reads.borrow().is_empty());
        let result = session.invoke(&request, quote, &raw).unwrap();
        if !case["native_called"].as_bool().unwrap() {
            assert!(matches!(
                result,
                FinalChainNativeInvocationResult::InsufficientGas { .. }
            ));
        } else {
            let outcome = completed(result);
            let error = case["native_error"].as_str().unwrap();
            assert_eq!(
                outcome.status,
                if error.is_empty() {
                    FinalChainNativeStatus::Success
                } else {
                    FinalChainNativeStatus::ContractFailure {
                        error: error.to_owned(),
                    }
                },
                "{}",
                case["name"]
            );
            assert_eq!(
                write_json(&outcome.raw_mutations),
                case["ordered_raw_writes"]
            );
            assert_eq!(
                outcome.output,
                unhex(case["native_output"].as_str().unwrap())
            );
            assert!(outcome.account_mutations.is_empty());
            if !error.is_empty() {
                assert!(outcome.logs.is_empty());
            }
        }
        if !case["native_called"].as_bool().unwrap() || case["native_error"] != "" {
            assert_eq!(session.dpos_state, before);
        }
        assert!(!session.aborted);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[path = "redelegate_new_destination_tests.rs"]
mod new_destination;

#[path = "redelegate_full_source_tests.rs"]
mod full_source;

#[path = "redelegate_zero_existing_tests.rs"]
mod zero_existing;

#[path = "redelegate_full_new_tests.rs"]
mod full_new;

#[path = "redelegate_swap_append_tests.rs"]
mod redelegate_swap_append_tests;

#[path = "redelegate_current_source_tests.rs"]
mod redelegate_current_source_tests;
