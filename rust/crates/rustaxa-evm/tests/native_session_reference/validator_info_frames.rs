//! Metadata composition over the actual Go EVM frame corpus. The historical
//! API uses the same execution observations, but this does not claim a separate
//! Go DryRunner oracle, persisted-reader coverage or RPC/trace parity.

use super::*;
use rustaxa_evm::simulation::simulate_with_native;

#[allow(dead_code)]
#[path = "../support/mixed_native.rs"]
mod mixed_native;

const OWNER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa1,
];

fn corpus() -> Vec<Value> {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_validator_info/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_validator_info/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            matches!(
                row["name"].as_str().unwrap(),
                "replace"
                    | "empty"
                    | "maximum"
                    | "both_too_long"
                    | "description_too_long"
                    | "missing_validator"
                    | "static"
                    | "parent_revert"
            )
        })
        .cloned()
        .collect()
}

fn key(prefix: &[u8]) -> ConcreteStorageKey {
    ConcreteStorageKey(keccak256([prefix, &VALIDATOR].concat()).0)
}

/// Reconstructs the explicitly synthetic input in the pinned exporter, including
/// its exact wrapper bytecode. This supplies no authority over historical data.
fn state(row: &Value) -> FixtureState {
    let mut code = vec![
        0x36, 0x60, 0, 0x60, 0, 0x37, 0x60, 0, 0x60, 0, 0x36, 0x60, 0,
    ];
    let is_static = row["static"].as_bool().unwrap();
    if !is_static {
        code.extend([0x60, 0]);
    }
    code.extend([0x60, 0xfe, 0x61, 0x4e, 0x20]);
    code.push(if is_static { 0xfa } else { 0xf1 });
    code.extend([0x60, 0, 0x52, 0x60, 32, 0x60, 0]);
    code.push(if row["parent_revert"].as_bool().unwrap() {
        0xfd
    } else {
        0xf3
    });
    let adapted = serde_json::json!({"code": hex::encode(code), "prior_raw": {}});
    let state = FixtureState::from_case(&adapted);
    {
        let mut rows = state.rows.borrow_mut();
        let mut wrapper = rows.accounts.remove(&TARGET).unwrap();
        wrapper.balance = ConcreteAccountBalance::new(1_000_000_u64.into());
        rows.accounts.insert(OWNER, wrapper);
        for (key, value) in [
            (key(&[0, 3]), OWNER.to_vec()),
            (key(&[0, 1]), vec![0xc2, 0x80, 0x80]),
            (key(&[0, 5, 2]), vec![1, 0, 0, 0]),
            (ConcreteStorageKey(keccak256([4]).0), vec![10]),
            (ConcreteStorageKey(keccak256([5]).0), vec![0x27, 0x10]),
        ] {
            rows.storage.insert((DPOS, key), value);
        }
    }
    state
}

fn chain(storage: Arc<Storage>) -> FinalChain {
    FinalChain::new_with_rewards_config(
        storage,
        1_000_000.into(),
        0,
        Vec::new(),
        vec![GenesisValidator {
            address: VALIDATOR,
            vrf_key: [0; 32],
            total_stake: ethereum_types::U256::from(10_000).to_big_endian().to_vec(),
            delegations: vec![(
                VALIDATOR,
                ethereum_types::U256::from(10_000).to_big_endian().to_vec(),
            )],
            metadata: GenesisValidatorMetadata {
                owner: OWNER,
                commission: 100,
                ..Default::default()
            },
        }],
        GenesisDposConfig {
            eligibility_balance_threshold: ethereum_types::U256::from(1_000).into(),
            vote_eligibility_balance_step: ethereum_types::U256::from(1_000).into(),
            validator_maximum_stake: ethereum_types::U256::from(30_000).into(),
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: FinalChainBlockNumber::GENESIS,
            cornus_period: FinalChainBlockNumber::GENESIS,
            fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            ..Default::default()
        },
    )
    .unwrap()
}

fn transaction(row: &Value) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(OWNER),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: 200_000.into(),
        value: ExecutionValue::default(),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}

fn block(period: FinalChainBlockNumber) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period,
        author: [0; 20],
        timestamp: 0,
        gas_limit: 1_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}

fn assert_execution(row: &Value, result: TransactionExecutionResult) {
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("{}: Go admitted this transaction", row["name"])
    };
    let expected = if row["parent_error"] == "" {
        CodeExecutionStatus::Success
    } else {
        assert_eq!(row["parent_error"], "execution reverted");
        CodeExecutionStatus::Failure(CodeExecutionError::Revert)
    };
    assert_eq!(result.status, expected, "{}", row["name"]);
    assert_eq!(
        result.gas_used.as_u64(),
        row["transaction_gas_used"].as_u64().unwrap(),
        "{}",
        row["name"]
    );
    assert_eq!(
        result.output,
        bytes(&row["parent_output"]),
        "{}",
        row["name"]
    );
    assert_eq!(result.logs, fixture_logs(&row["logs"]), "{}", row["name"]);
    assert_eq!(result.attempted_contract_address, None);
}

#[test]
fn metadata_frames_match_go_through_real_pending_session_and_journal() {
    let cases = corpus();
    assert_eq!(cases.len(), 8);
    let success_logs = fixture_logs(&cases[0]["logs"]);
    for row in cases {
        let state = state(&row);
        let path = temp_db_path(row["name"].as_str().unwrap());
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = chain(storage.clone());
        {
            let mut port = SessionPort {
                session: chain.begin_native_session(1.into(), 0.into()).unwrap(),
                prepared: Vec::new(),
                invoked: Vec::new(),
            };
            let mut journal = ExecutionJournal::new(state.clone());
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &DposNative,
                &DposNative,
                &mut port,
                &mut sequence,
                &block(1.into()),
                &transaction(&row),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
            assert_execution(&row, result);
            assert_eq!(sequence.next_sequence(), 1);
            assert_eq!(port.prepared.len(), 1);
            assert_eq!(port.invoked.len(), 1);
            let (invocation, quote) = &port.prepared[0];
            assert_eq!(invocation, &port.invoked[0]);
            assert_eq!(invocation.caller, OWNER);
            assert_eq!(invocation.depth, row["depth"].as_u64().unwrap() as u16);
            assert_eq!(invocation.input, bytes(&row["input"]));
            assert_eq!(
                invocation.supplied_gas.as_u64(),
                row["supplied_native_gas"].as_u64().unwrap()
            );
            assert_eq!(quote.as_u64(), row["required_gas"].as_u64().unwrap());
            let settled = journal.settle_transaction().unwrap();
            assert_eq!(settled.native_invocations.len(), 1);
            let fact = &settled.native_invocations[0];
            assert_eq!(fact.invocation, *invocation);
            assert_eq!(fact.gas_used, *quote);
            assert_eq!(fact.output, bytes(&row["native_output"]));
            let disposition = if row["native_error"] != "" {
                let CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) = &fact.status
                else {
                    panic!("expected native failure")
                };
                assert_eq!(error.error, row["native_error"].as_str().unwrap());
                ConsensusNativeDisposition::OwnFrameReverted
            } else {
                assert_eq!(fact.status, CodeExecutionStatus::Success);
                if row["parent_revert"].as_bool().unwrap() {
                    ConsensusNativeDisposition::OuterFrameReverted
                } else {
                    ConsensusNativeDisposition::Normal
                }
            };
            assert_eq!(fact.disposition, disposition);
            let expected_native_logs = if row["native_error"] == "" {
                &success_logs[..]
            } else {
                &[]
            };
            assert_eq!(fact.logs, expected_native_logs);
            assert_eq!(settled.logs, fixture_logs(&row["logs"]));
            assert!(settled.writes.ordinary_storage.is_empty());
            let actual = settled.writes.raw_storage.iter().map(|write| {
                let NativeRawOperation::Put(value) = &write.operation else { panic!("unexpected delete") };
                serde_json::json!({"address": hex::encode(write.address), "key": hex::encode(write.key.0), "value": hex::encode(value.as_bytes())})
            }).collect::<Vec<_>>();
            assert_eq!(
                actual,
                row["ordered_raw_writes"].as_array().unwrap().clone()
            );
            state.apply(settled.writes);
            assert_eq!(
                state.storage(DPOS, key(&[0, 1])).unwrap(),
                ConcreteRead::Present(bytes(&row["after_info"]))
            );
        }
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn metadata_historical_simulation_matches_frame_results_and_discards_every_probe() {
    for row in corpus() {
        let reader = state(&row);
        let before_raw = reader.rows.borrow().storage.clone();
        let before_accounts = [SENDER, OWNER, DPOS].map(|address| reader.account(address).unwrap());
        let before_code = reader.rows.borrow().code.clone();
        let path = temp_db_path("metadata-simulation");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = chain(storage.clone());
        // Every probe gets a new historical session. The selected synthetic
        // genesis activates the same method rules as Go's Cornus test period.
        for _ in 0..2 {
            let mut transaction = transaction(&row);
            transaction.nonce = FinalChainNonce::from_u64(999);
            let result = simulate_with_native(
                &reader,
                &NoHistory,
                &DposNative,
                &DposNative,
                |identity| {
                    assert_eq!(identity, reader.identity());
                    Ok(mixed_native::SimulationNativeExecutionPort::new(
                        chain.begin_native_simulation(identity.period).unwrap(),
                    ))
                },
                &block(reader.identity().period),
                &transaction,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
            assert_eq!(result.state, reader.identity());
            assert_execution(&row, result.execution);
            assert_eq!(transaction.nonce, FinalChainNonce::from_u64(999));
            assert_eq!(reader.rows.borrow().storage, before_raw);
            assert_eq!(
                [SENDER, OWNER, DPOS].map(|address| reader.account(address).unwrap()),
                before_accounts
            );
            assert_eq!(reader.rows.borrow().code, before_code);
        }
        // A new real pending session still starts with committed empty metadata.
        // Its reader is the unchanged raw state; any leaked semantic change would
        // cause the real session's semantic/raw authentication to fail.
        let mut fresh = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let journal = ExecutionJournal::new(reader.clone());
        let invocation = NativeInvocation {
            id: NativeInvocationId {
                transaction: 0.into(),
                sequence: 0,
            },
            period: 1.into(),
            depth: 1,
            kind: NativeCallKind::Call,
            is_static: false,
            caller: OWNER,
            contract: DPOS,
            state_address: DPOS,
            value: ExecutionValue::default(),
            input: bytes(&row["input"]),
            supplied_gas: 20_000.into(),
        };
        let request = consensus_request(&invocation);
        let read = SessionRead(&journal);
        let quote = fresh.prepare(&request, &read).unwrap();
        fresh.invoke(&request, quote, &read).unwrap();
        drop(fresh);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
