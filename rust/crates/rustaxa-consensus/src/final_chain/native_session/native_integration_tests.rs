//! Native regressions over the authenticated current-profile Go corpus.
use super::*;
use k256::sha2::{Digest, Sha256};
use serde_json::{Value, json};
use std::{cell::RefCell, io::Read, path::PathBuf, sync::Arc};

fn decode(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .unwrap()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn corpus() -> Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_integration");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["references"]["public"],
        "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b"
    );
    assert_eq!(
        manifest["references"]["local"],
        "bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418"
    );
    let pin = &manifest["streams"]["local.observer.traced"];
    let stored = std::fs::read(root.join(pin["file"].as_str().unwrap())).unwrap();
    let mut body = Vec::new();
    if pin["encoding"] == "gzip" {
        flate2::read::GzDecoder::new(stored.as_slice())
            .read_to_end(&mut body)
            .unwrap();
    } else {
        body = stored;
    }
    assert_eq!(body.len() as u64, pin["bytes"].as_u64().unwrap());
    assert_eq!(hex(&Sha256::digest(&body)), pin["sha256"]);
    serde_json::from_slice(&body).unwrap()
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        Self(crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!(
                "rustaxa-native-integration-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            true,
        ))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.exists() {
            crate::closeout_test_scratch::dispose(&self.0).unwrap();
        }
    }
}

fn chain(
    path: &Scratch,
    fixture: &Value,
    activation: FinalChainBlockNumber,
    generated: u64,
) -> FinalChain {
    chain_with_threshold(path, fixture, activation, generated, 100)
}
fn chain_with_threshold(
    path: &Scratch,
    fixture: &Value,
    activation: FinalChainBlockNumber,
    generated: u64,
    threshold: u64,
) -> FinalChain {
    chain_with_economics(
        path,
        fixture,
        activation,
        generated,
        threshold,
        U256::from(10_000_000_000u64),
    )
}
fn chain_with_economics(
    path: &Scratch,
    fixture: &Value,
    activation: FinalChainBlockNumber,
    generated: u64,
    threshold: u64,
    max_supply: U256,
) -> FinalChain {
    let storage = Arc::new(Storage::new(rustaxa_storage::Config::new(path.0.clone())).unwrap());
    let accounts = fixture["inputs"]["genesis_allocations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| rustaxa_types::GenesisAccount {
            address: decode(&row["address"]).try_into().unwrap(),
            balance: rustaxa_types::FinalChainAccountBalance::new_account(
                U256::from_dec_str(row["balance"].as_str().unwrap()).unwrap(),
            ),
        })
        .collect();
    FinalChain::new_with_rewards_config_and_ficus_activation(
        storage,
        10_000_000.into(),
        0,
        crate::FinalChainGenesisAllocation {
            accounts,
            validators: Vec::new(),
        },
        rustaxa_types::GenesisDposConfig {
            eligibility_balance_threshold: U256::from(threshold).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::one().into(),
            commission_change_delta: 100,
            commission_change_frequency: 2,
            delegation_delay: 1,
            ..Default::default()
        },
        rustaxa_types::FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            phalaenopsis_period: 0.into(),
            cornus_period: 0.into(),
            fix_claim_all_block_num: 0.into(),
            fix_redelegate_block_num: 0.into(),
            cacti_period: 0.into(),
            cacti_jail_time: 2,
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: activation,
            genesis_balance_sum: Some(
                fixture["inputs"]["genesis_allocations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .fold(U256::zero(), |total, row| {
                        total + U256::from_dec_str(row["balance"].as_str().unwrap()).unwrap()
                    })
                    .into(),
            ),
            aspen_max_supply: max_supply.into(),
            aspen_generated_rewards: U256::from(generated).into(),
            dpos_blocks_per_year: 100,
            yield_percentage: 1,
            committee_size: 1,
            rewards_distribution_frequency: vec![(0.into(), 1)],
            ..Default::default()
        },
        0.into(),
    )
    .unwrap()
}
#[derive(Clone)]
struct State {
    rows: BTreeMap<([u8; 20], ConcreteStorageKey), ConcreteRead<Vec<u8>>>,
    reads: RefCell<Vec<([u8; 20], ConcreteStorageKey)>>,
    fault: Option<(usize, bool)>,
}
impl State {
    fn genesis(fixture: &Value) -> Self {
        Self {
            rows: fixture["genesis"]["native_catalog"]["slots"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    (
                        (
                            decode(&row["address"]).try_into().unwrap(),
                            ConcreteStorageKey(decode(&row["key"]).try_into().unwrap()),
                        ),
                        if row["present"] == true {
                            ConcreteRead::Present(decode(&row["value"]))
                        } else {
                            ConcreteRead::Absent
                        },
                    )
                })
                .collect(),
            reads: RefCell::new(Vec::new()),
            fault: None,
        }
    }
    fn apply(&mut self, outcome: &FinalChainNativeOutcome) {
        for mutation in &outcome.raw_mutations {
            let key = (mutation.address, mutation.key);
            assert_eq!(
                self.rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent),
                mutation.expected
            );
            self.rows.insert(
                key,
                ConcreteRead::Present(match &mutation.operation {
                    FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
                    FinalChainNativeRawOperation::Delete => Vec::new(),
                }),
            );
        }
    }
}
impl FinalChainNativeStateRead for State {
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        let index = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, *key));
        if let Some((at, malformed)) = self.fault
            && index == at
        {
            return if malformed {
                Ok(ConcreteRead::Present(vec![0xff]))
            } else {
                Err(FinalChainNativeStateReadError::Invariant(
                    "native integration consumed read".into(),
                ))
            };
        }
        Ok(self
            .rows
            .get(&(address, *key))
            .cloned()
            .unwrap_or(ConcreteRead::Absent))
    }
}
fn request(row: &Value, sequence: u64) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: (sequence as u32).into(),
            sequence,
        },
        period: 1.into(),
        depth: 0,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: decode(&row["sender"]).try_into().unwrap(),
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(
            BigUint::parse_bytes(row["value"].as_str().unwrap().as_bytes(), 10).unwrap(),
        ),
        input: decode(&row["input"]),
        supplied_gas: 500_000.into(),
    }
}
fn completed(result: FinalChainNativeInvocationResult) -> FinalChainNativeOutcome {
    let FinalChainNativeInvocationResult::Completed(outcome) = result else {
        panic!("funded reference invocation")
    };
    outcome
}
fn compare(outcome: &FinalChainNativeOutcome, row: &Value) {
    assert_eq!(outcome.status, FinalChainNativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    assert_eq!(
        outcome.gas_used.as_u64()
            + rustaxa_types::transaction::intrinsic_gas(&decode(&row["input"]), false).unwrap(),
        row["gas_used"].as_u64().unwrap()
    );
    let raw = outcome.raw_mutations.iter().map(|mutation| json!({"address":hex(&mutation.address), "key":hex(&mutation.key.0), "value":hex(match &mutation.operation {
        FinalChainNativeRawOperation::Put(value) => value.as_bytes(), FinalChainNativeRawOperation::Delete => &[],
    })})).collect::<Vec<_>>();
    assert_eq!(raw, row["ordered_raw_writes"].as_array().unwrap().clone());
    let logs = outcome.logs.iter().map(|log| json!({"address":hex(&log.address), "topics":log.topics.iter().map(|topic|hex(topic)).collect::<Vec<_>>(), "data":hex(&log.data)})).collect::<Vec<_>>();
    assert_eq!(logs, row["logs"].as_array().unwrap().clone());
    assert_eq!(outcome.output, decode(&row["output"]));
}
#[test]
fn native_integration_registration_matches_go_and_consumed_failures_abort() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain(&path, &fixture, 0.into(), 0);
    let rows = fixture["periods"][0]["transactions"].as_array().unwrap();
    let mut state = State::genesis(&fixture);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    for (index, row) in rows[..3].iter().enumerate() {
        let request = request(row, index as u64);
        let quote = session.prepare(&request, &state).unwrap();
        let outcome = completed(session.invoke(&request, quote, &state).unwrap());
        compare(&outcome, row);
        let changes = session.input_mutations(request.id).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(request.input[changes[0].offset], changes[0].expected);
        assert_eq!(changes[0].replacement, changes[0].expected.wrapping_sub(27));
        assert!(session.input_mutations(request.id).is_err());
        assert_eq!(request.input, decode(&row["input"]));
        state.apply(&outcome);
    }
    let request = request(&rows[0], 0);
    let pristine = State::genesis(&fixture);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let quote = session.prepare(&request, &pristine).unwrap();
    let expected = completed(session.invoke(&request, quote, &pristine).unwrap());
    for at in 0..pristine.reads.borrow().len() {
        for malformed in [false, true] {
            let mut state = State::genesis(&fixture);
            state.fault = Some((at, malformed));
            let before = state.rows.clone();
            let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
            let semantic_before = session.dpos_state.clone();
            let quote = session.prepare(&request, &state).unwrap();
            assert!(session.invoke(&request, quote, &state).is_err());
            assert_eq!(session.dpos_state, semantic_before);
            assert_eq!(state.rows, before);
            assert_eq!(
                session.prepare(&request, &state),
                Err(FinalChainNativeSessionError::Aborted)
            );
            let state = State::genesis(&fixture);
            let mut retry = chain.begin_native_session(1.into(), 0.into()).unwrap();
            let quote = retry.prepare(&request, &state).unwrap();
            assert_eq!(
                completed(retry.invoke(&request, quote, &state).unwrap()),
                expected
            );
        }
    }
}
#[test]
fn native_integration_aspen_genesis_is_initialized_without_minting() {
    let fixture = corpus();
    for activation in [
        FinalChainBlockNumber::GENESIS,
        2.into(),
        FinalChainBlockNumber::MAX,
    ] {
        let path = Scratch::new();
        let chain = chain(&path, &fixture, activation, 7);
        let genesis = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
        assert!(genesis.delegator_rewards.is_empty() && genesis.commission_rewards.is_empty());
        assert!(genesis.validator_order.is_empty());
        if activation == FinalChainBlockNumber::GENESIS {
            let supply = DposTokenAmount::from(U256::from(5_000_000_007_u64));
            assert_eq!(
                genesis.aspen_supply_state.total_supply().unwrap().amount(),
                supply
            );
            assert_eq!(
                genesis.aspen_supply_state.current_yield(),
                Some(chain.aspen_current_yield(supply).unwrap())
            );
        } else {
            assert!(matches!(
                genesis.aspen_supply_state,
                AspenSupplyState::Unmigrated { .. }
            ));
        }
    }
}
#[test]
fn native_integration_slashing_marker_matches_literal_go_key_and_reversal() {
    let fixture = corpus();
    let row = fixture["periods"][3]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "slash_a")
        .unwrap();
    let mut input = decode(&row["input"]);
    let proof = verify_legacy_double_voting_proof_call_data(&input).unwrap();
    assert_eq!(
        hex(proof.proof_key.as_bytes()),
        "dc35efc475e49bffe215eea886063ea790bde40e358685c728d52129f16ae9db"
    );
    let left = input[4..36].to_vec();
    let right = input[36..68].to_vec();
    input[4..36].copy_from_slice(&right);
    input[36..68].copy_from_slice(&left);
    assert_eq!(
        verify_legacy_double_voting_proof_call_data(&input)
            .unwrap()
            .proof_key,
        proof.proof_key
    );
}

#[test]
fn native_integration_slashing_writes_and_duplicate_match_go() {
    struct SlashingState(
        State,
        std::cell::Cell<usize>,
        std::cell::RefCell<Vec<(ConcreteStorageKey, bool)>>,
    );
    impl FinalChainNativeStateRead for SlashingState {
        fn account(
            &self,
            address: [u8; 20],
        ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
            assert_eq!(address, SLASHING_CONTRACT_ADDRESS);
            self.1.set(self.1.get() + 1);
            Ok(FinalChainNativeAccount {
                exists: false,
                nonce: FinalChainNonce::zero(),
                balance: BigInt::from(0),
            })
        }
        fn raw_storage(
            &self,
            address: [u8; 20],
            key: &ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            self.0.raw_storage(address, key)
        }
        fn observe_stage(&self, event: FinalChainNativeStageEvent<'_>) {
            match event {
                FinalChainNativeStageEvent::RawRead {
                    address,
                    key,
                    semantic,
                    ..
                } if address == SLASHING_CONTRACT_ADDRESS => {
                    self.2.borrow_mut().push((key, semantic))
                }
                #[cfg(feature = "synthetic-fixtures")]
                FinalChainNativeStageEvent::SlashingCurrentGet { begin, purpose, .. } => {
                    self.2
                        .borrow_mut()
                        .push((ConcreteStorageKey([0xf0 + purpose; 32]), begin));
                }
                #[cfg(feature = "synthetic-fixtures")]
                FinalChainNativeStageEvent::SlashingDelayedList {
                    selected_period,
                    validators,
                    ..
                } => {
                    self.2.borrow_mut().push((
                        ConcreteStorageKey([0xff; 32]),
                        selected_period.is_some() && validators.is_empty(),
                    ));
                }
                _ => {}
            }
        }
    }
    let fixture = corpus();
    let rows = fixture["periods"][3]["transactions"].as_array().unwrap();
    let slash = rows.iter().find(|row| row["name"] == "slash_a").unwrap();
    let duplicate = rows
        .iter()
        .find(|row| row["name"] == "slash_duplicate")
        .unwrap();
    let path = Scratch::new();
    let chain = chain_with_threshold(&path, &fixture, 0.into(), 0, 0);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    // Use the fixture's execution period with an independent, empty delayed list.
    session.pending_period = 4.into();
    let mut state = SlashingState(
        State::genesis(&fixture),
        std::cell::Cell::new(0),
        std::cell::RefCell::new(Vec::new()),
    );
    let mut call = request(slash, 0);
    call.period = 4.into();
    call.contract = SLASHING_CONTRACT_ADDRESS;
    call.state_address = SLASHING_CONTRACT_ADDRESS;
    for (at, name) in [
        "double-voting proof",
        "validator jail block",
        "slashing jailed membership",
    ]
    .into_iter()
    .enumerate()
    {
        for malformed in [false, true] {
            let mut unavailable = SlashingState(
                State::genesis(&fixture),
                std::cell::Cell::new(0),
                std::cell::RefCell::new(Vec::new()),
            );
            unavailable.0.fault = Some((at, malformed));
            let mut failed_session = chain.begin_native_session(1.into(), 0.into()).unwrap();
            failed_session.pending_period = 4.into();
            let before = failed_session.dpos_state.clone();
            let rows_before = unavailable.0.rows.clone();
            let quote = failed_session.prepare(&call, &unavailable).unwrap();
            let expected = if malformed {
                FinalChainNativeSessionError::RawIntegrity(name.into())
            } else {
                FinalChainNativeSessionError::StateRead(FinalChainNativeStateReadError::Invariant(
                    "native integration consumed read".into(),
                ))
            };
            assert_eq!(
                failed_session.invoke(&call, quote, &unavailable),
                Err(expected)
            );
            assert_eq!(failed_session.dpos_state, before);
            assert_eq!(unavailable.0.rows, rows_before);
            assert_eq!(unavailable.1.get(), 0);
            assert_eq!(
                failed_session.prepare(&call, &unavailable),
                Err(FinalChainNativeSessionError::Aborted)
            );
        }
    }
    let quote = session.prepare(&call, &state).unwrap();
    let outcome = completed(session.invoke(&call, quote, &state).unwrap());
    let mut list_key = [0_u8; 32];
    list_key[31] = 2;
    let list_reads = state.2.borrow();
    #[cfg(feature = "synthetic-fixtures")]
    for purpose in [1_u8, 2] {
        let marker = ConcreteStorageKey([0xf0 + purpose; 32]);
        let phases: Vec<_> = list_reads
            .iter()
            .filter(|(key, _)| *key == marker)
            .map(|(_, begin)| *begin)
            .collect();
        assert_eq!(phases, [true, false], "actual slashing getter markers");
    }
    assert!(
        list_reads
            .iter()
            .filter(|(key, semantic)| key.0 == list_key && !semantic)
            .count()
            >= 2
    );
    assert!(
        !list_reads
            .iter()
            .any(|(key, semantic)| key.0 == list_key && *semantic)
    );
    #[cfg(feature = "synthetic-fixtures")]
    assert!(
        list_reads
            .iter()
            .any(|(key, valid)| key.0 == [0xff; 32] && *valid),
        "actual delayed list observation"
    );
    drop(list_reads);
    assert_eq!(
        outcome.account_mutations,
        vec![FinalChainNativeOrdinaryMutation::NonceReplace {
            address: SLASHING_CONTRACT_ADDRESS,
            expected_exists: false,
            expected: FinalChainNonce::zero(),
            replacement: FinalChainNonce::from_u64(1),
        }]
    );
    let mut raw_outcome = outcome.clone();
    raw_outcome.account_mutations.clear();
    compare(&raw_outcome, slash);
    state.0.apply(&outcome);
    let before = session.dpos_state.clone();
    let mut call = request(duplicate, 1);
    call.period = 4.into();
    call.contract = SLASHING_CONTRACT_ADDRESS;
    call.state_address = SLASHING_CONTRACT_ADDRESS;
    let quote = session.prepare(&call, &state).unwrap();
    let outcome = completed(session.invoke(&call, quote, &state).unwrap());
    assert_eq!(
        outcome.gas_used.as_u64()
            + rustaxa_types::transaction::intrinsic_gas(&decode(&duplicate["input"]), false)
                .unwrap(),
        duplicate["gas_used"].as_u64().unwrap()
    );
    assert_eq!(
        outcome.status,
        FinalChainNativeStatus::ContractFailure {
            error: duplicate["execution_error"].as_str().unwrap().into()
        }
    );
    assert!(outcome.raw_mutations.is_empty());
    assert!(outcome.account_mutations.is_empty());
    assert!(outcome.logs.is_empty());
    assert_eq!(outcome.output, decode(&duplicate["output"]));
    assert_eq!(session.dpos_state, before);
}

#[test]
fn native_integration_existing_checkpoint_keeps_later_reward_pool() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain(&path, &fixture, 0.into(), 0);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let state = State::genesis(&fixture);
    let row = &fixture["periods"][0]["transactions"][0];
    let request = request(row, 0);
    let quote = session.prepare(&request, &state).unwrap();
    completed(session.invoke(&request, quote, &state).unwrap());
    let mut snapshot = session.dpos_state.clone();
    let validator = decode(&fixture["periods"][0]["transactions"][0]["input"])[16..36]
        .try_into()
        .unwrap();
    snapshot.delegator_rewards.insert(
        validator,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(11)),
    );
    let existing = chain
        .checkpoint_validator_reward_per_stake(&mut snapshot, validator)
        .unwrap();
    assert_eq!(
        snapshot.delegator_rewards[&validator].as_u256(),
        U256::from(11)
    );
    chain
        .advance_reward_reference_graph_block(&mut snapshot, 2.into())
        .unwrap();
    let next = chain
        .checkpoint_validator_reward_per_stake(&mut snapshot, validator)
        .unwrap();
    assert!(next > existing);
    assert!(snapshot.delegator_rewards[&validator].is_zero());
}

#[test]
fn native_integration_zero_threshold_accepts_absent_stake_as_go_does() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain_with_threshold(&path, &fixture, 0.into(), 0, 0);
    let row = fixture["periods"][3]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "slash_a")
        .unwrap();
    let proof = verify_legacy_double_voting_proof_call_data(&decode(&row["input"])).unwrap();
    let mut state = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
    assert!(state.total_stakes.is_empty());
    let result = chain
        .apply_slashing_double_voting_proof(&mut state, None, 1.into(), proof)
        .unwrap();
    assert_eq!(result.status_code, 1);
    assert_eq!(state.slashing_jail_blocks.len(), 1);
}

#[test]
fn native_integration_failed_prepare_discards_old_input_metadata() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain(&path, &fixture, 0.into(), 0);
    for invalid in 0..3 {
        let state = State::genesis(&fixture);
        let original = request(&fixture["periods"][0]["transactions"][0], 0);
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let quote = session.prepare(&original, &state).unwrap();
        completed(session.invoke(&original, quote, &state).unwrap());
        assert_eq!(session.input_mutations.len(), 1);
        let before = session.dpos_state.clone();
        let mut bad = original.clone();
        bad.id.sequence = 1;
        match invalid {
            0 => bad.period = 2.into(),
            1 => {
                bad.contract = [7; 20];
                bad.state_address = [7; 20];
            }
            _ => bad.id.sequence = 3,
        }
        assert!(session.prepare(&bad, &state).is_err());
        assert_eq!(session.dpos_state, before);
        assert_eq!(
            session.input_mutations(original.id),
            Err(FinalChainNativeSessionError::NotPrepared)
        );
        assert!(session.input_mutations.is_empty());
    }
}

#[test]
fn native_integration_genesis_yield_zero_wide_and_wrapping_match_go_and_reopen() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_integration");
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(root.join("genesis-boundary-manifest.json")).unwrap(),
    )
    .unwrap();
    let pin = &manifest["stream"];
    let raw = std::fs::read(root.join(pin["file"].as_str().unwrap())).unwrap();
    assert_eq!(hex(&Sha256::digest(&raw)), pin["sha256"]);
    assert_eq!(raw.len() as u64, pin["bytes"].as_u64().unwrap());
    let oracle: Value = serde_json::from_slice(&raw).unwrap();
    for row in oracle["cases"].as_array().unwrap() {
        let mut fixture = corpus();
        let supply = U256::from_dec_str(row["supply"].as_str().unwrap()).unwrap();
        for allocation in fixture["inputs"]["genesis_allocations"]
            .as_array_mut()
            .unwrap()
        {
            allocation["balance"] = Value::String((supply / U256::from(5)).to_string());
        }
        let max = U256::from_dec_str(row["max_supply"].as_str().unwrap()).unwrap();
        let path = Scratch::new();
        let chain = chain_with_economics(&path, &fixture, 0.into(), 0, 100, max);
        let snapshot = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
        assert_eq!(
            snapshot
                .aspen_supply_state
                .total_supply()
                .unwrap()
                .as_u256(),
            supply
        );
        assert_eq!(
            snapshot
                .aspen_supply_state
                .current_yield()
                .unwrap()
                .as_u64(),
            row["stored_yield"].as_u64().unwrap()
        );
        assert_eq!(row["reward"], "0");
        assert!(snapshot.delegator_rewards.is_empty() && snapshot.commission_rewards.is_empty());
        let raw_rows = canonical_concrete_precompile_storage(&snapshot, true).unwrap();
        let encoded = supply.to_big_endian();
        let first = encoded
            .iter()
            .position(|b| *b != 0)
            .unwrap_or(encoded.len());
        assert!(
            raw_rows[&(DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[7]]))]
                .contains(&encoded[first..].to_vec())
        );
        assert!(
            raw_rows[&(DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[8]]))]
                .contains(&rlp::encode(&row["stored_yield"].as_u64().unwrap()).to_vec())
        );
        drop(chain);
        let reopened = chain_with_economics(&path, &fixture, 0.into(), 0, 100, max);
        assert_eq!(
            reopened
                .dpos_snapshot_at_finalized_block(0.into())
                .unwrap()
                .aspen_supply_state,
            snapshot.aspen_supply_state
        );
    }
}
