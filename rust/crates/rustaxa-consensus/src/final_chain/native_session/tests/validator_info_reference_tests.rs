use super::*;
use serde_json::Value;

const INFO_OWNER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa1,
];

fn with_info_chain(name: &str, test: impl FnOnce(&FinalChain)) {
    let path = temp_db_path(name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        Vec::new(),
        vec![GenesisValidator {
            address: VALIDATOR,
            vrf_key: [0; 32],
            total_stake: U256::from(10_000).to_big_endian().to_vec(),
            delegations: vec![(VALIDATOR, U256::from(10_000).to_big_endian().to_vec())],
            metadata: GenesisValidatorMetadata {
                owner: INFO_OWNER,
                commission: 100,
                ..Default::default()
            },
        }],
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(1_000).into(),
            vote_eligibility_balance_step: U256::from(1_000).into(),
            validator_maximum_stake: U256::from(30_000).into(),
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
        FinalChainBlockNumber::MAX,
    )
    .unwrap();
    test(&chain);
    drop(chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
}

const INFO_ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../experiments/evm_feasibility/fixtures/native_validator_info/public.json"
));

fn unhex(value: &Value) -> Vec<u8> {
    let text = value.as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

trait InfoSession {
    fn prepare_info(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &RawState,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError>;
    fn invoke_info(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &RawState,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError>;
}
impl InfoSession for FinalChainNativeSession<'_> {
    fn prepare_info(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &RawState,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        self.prepare(request, state)
    }
    fn invoke_info(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &RawState,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        self.invoke(request, quote, state)
    }
}
impl InfoSession for FinalChainNativeSimulation<'_> {
    fn prepare_info(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &RawState,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        self.prepare(request, state)
    }
    fn invoke_info(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &RawState,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        self.invoke(request, quote, state)
    }
}

#[test]
fn validator_info_pending_and_historical_sessions_match_actual_go_frames() {
    let oracle: Value = serde_json::from_str(INFO_ORACLE).unwrap();
    assert_eq!(oracle["fix_redelegate"], 3_091_000);
    assert_eq!(oracle["cornus"], 15_610_000);
    let cases = oracle["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    let supported = cases
        .iter()
        .filter(|case| case["period"].as_u64().unwrap() >= 15_610_000)
        .collect::<Vec<_>>();
    assert_eq!(supported.len(), 11);
    // The Go corpus uses real mainnet activation numbers. Rust owner fixtures
    // rebase the same active rules to genesis without fabricating history.
    with_info_chain("validator-info-reference", |chain| {
        let finalized = chain
            .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
            .unwrap();
        for case in supported {
            for historical in [false, true] {
                let state = RawState::from_snapshot(&finalized, BTreeMap::new());
                let period = if historical {
                    FinalChainBlockNumber::GENESIS
                } else {
                    1.into()
                };
                let mut pending;
                let mut simulation;
                let session: &mut dyn InfoSession = if historical {
                    simulation = chain.begin_native_simulation(period).unwrap();
                    &mut simulation
                } else {
                    pending = chain
                        .begin_native_session(period, FinalChainBlockNumber::GENESIS)
                        .unwrap();
                    &mut pending
                };
                let mut request = simulation_request(
                    period,
                    0,
                    unhex(&case["input"]),
                    case["supplied_native_gas"].as_u64().unwrap(),
                );
                request.caller = unhex(&case["caller"]).try_into().unwrap();
                request.depth = case["depth"].as_u64().unwrap() as u16;
                request.value = FinalChainNativeValue::new(case["value"].as_u64().unwrap().into());
                if case["static"].as_bool().unwrap() {
                    request.kind = FinalChainNativeCallKind::StaticCall;
                    request.is_static = true;
                }
                let quote = session.prepare_info(&request, &state).unwrap();
                assert_eq!(
                    quote.required_gas.as_u64(),
                    case["required_gas"].as_u64().unwrap(),
                    "{}",
                    case["name"]
                );
                let result = session.invoke_info(&request, quote, &state).unwrap();
                if !case["native_called"].as_bool().unwrap() {
                    assert_eq!(
                        result,
                        FinalChainNativeInvocationResult::InsufficientGas {
                            required_gas: 20_000.into()
                        }
                    );
                    assert_eq!(state.reads.get(), 0);
                    continue;
                }
                let mut outcome = completed(result);
                let error = case["native_error"].as_str().unwrap();
                let status = if error.is_empty() {
                    FinalChainNativeStatus::Success
                } else {
                    FinalChainNativeStatus::ContractFailure {
                        error: error.to_owned(),
                    }
                };
                assert_eq!(outcome.status, status, "{}", case["name"]);
                assert_eq!(outcome.gas_used, quote.required_gas);
                assert_eq!(outcome.output, unhex(&case["native_output"]));
                assert!(outcome.account_mutations.is_empty());
                assert_eq!(state.account_reads.get(), 0);
                let expected_keys = case["ordered_reads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| ConcreteStorageKey(unhex(value).try_into().unwrap()))
                    .collect::<Vec<_>>();
                assert_eq!(*state.read_keys.borrow(), expected_keys, "{}", case["name"]);
                let writes = case["ordered_raw_writes"].as_array().unwrap();
                assert_eq!(outcome.raw_mutations.len(), writes.len());
                for (actual, expected) in outcome.raw_mutations.iter().zip(writes) {
                    assert_eq!(actual.address.as_slice(), unhex(&expected["address"]));
                    assert_eq!(actual.key.0.as_slice(), unhex(&expected["key"]));
                    assert_eq!(
                        actual.expected,
                        ConcreteRead::Present(vec![0xc2, 0x80, 0x80])
                    );
                    assert_eq!(
                        actual.operation,
                        FinalChainNativeRawOperation::Put(
                            FinalChainNativeRawValue::new(unhex(&expected["value"])).unwrap()
                        )
                    );
                    state.apply(actual);
                }
                if case["parent_revert"].as_bool().unwrap() {
                    // Actual Go outer REVERT removes its log and retains the
                    // irreversible raw write. The frame owner drops Rust logs.
                    assert_eq!(outcome.logs.len(), 1);
                    outcome.logs.clear();
                }
                let expected_logs = case["logs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|log| FinalChainCallLog {
                        address: unhex(&log["address"]).try_into().unwrap(),
                        topics: log["topics"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|topic| unhex(topic).try_into().unwrap())
                            .collect(),
                        data: unhex(&log["data"]),
                    })
                    .collect::<Vec<_>>();
                assert_eq!(outcome.logs, expected_logs);
                let key = concrete_storage_key(&[&[0, 1], &VALIDATOR]);
                assert_eq!(
                    state.rows.borrow().get(&(DPOS_CONTRACT_ADDRESS, key)),
                    Some(&ConcreteRead::Present(unhex(&case["after_info"])))
                );
            }
        }
        assert_eq!(
            chain
                .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
                .unwrap(),
            finalized
        );
        assert_eq!(
            chain.last_block_number_typed().unwrap(),
            FinalChainBlockNumber::GENESIS
        );
    });
}

#[test]
fn validator_info_raw_failures_poison_without_semantic_or_partial_effects() {
    with_info_chain("validator-info-integrity", |chain| {
        let mut request = simulation_request(
            1.into(),
            0,
            unhex(&serde_json::from_str::<Value>(INFO_ORACLE).unwrap()["cases"][0]["input"]),
            20_000,
        );
        request.depth = 1;
        request.caller = INFO_OWNER;
        for prefix in [&[0, 3][..], &[0, 1][..], &[0, 5, 2][..]] {
            for corruption in [
                ConcreteRead::Absent,
                ConcreteRead::Present(Vec::new()),
                ConcreteRead::Present(vec![0xff]),
            ] {
                let mut session = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let before = session.dpos_state.clone();
                let state = RawState::from_snapshot(&before, BTreeMap::new());
                state.set(
                    DPOS_CONTRACT_ADDRESS,
                    concrete_storage_key(&[prefix, &VALIDATOR]),
                    corruption,
                );
                let quote = session.prepare(&request, &state).unwrap();
                assert!(matches!(
                    session.invoke(&request, quote, &state),
                    Err(FinalChainNativeSessionError::RawIntegrity(_))
                ));
                assert_eq!(session.dpos_state, before);
                assert_eq!(
                    session.prepare(&request, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                );
            }
        }
        let mut session = chain
            .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
            .unwrap();
        let before = session.dpos_state.clone();
        let unavailable = RawState::erroring();
        let quote = session.prepare(&request, &unavailable).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &unavailable),
            Err(FinalChainNativeSessionError::StateRead(_))
        ));
        assert_eq!(session.dpos_state, before);
        assert_eq!(
            session.prepare(&request, &unavailable),
            Err(FinalChainNativeSessionError::Aborted)
        );
    });
}

#[test]
fn validator_info_successive_updates_keep_raw_and_query_views_consistent() {
    with_info_chain("validator-info-sequence", |chain| {
        let oracle: Value = serde_json::from_str(INFO_ORACLE).unwrap();
        let mut session = chain
            .begin_native_simulation(FinalChainBlockNumber::GENESIS)
            .unwrap();
        let finalized = session.session.dpos_state.clone();
        let state = RawState::from_snapshot(&finalized, BTreeMap::new());
        for (sequence, index) in [0, 7].into_iter().enumerate() {
            let mut request = simulation_request(
                FinalChainBlockNumber::GENESIS,
                sequence as u64,
                unhex(&oracle["cases"][index]["input"]),
                20_000,
            );
            request.is_static = index == 7;
            request.caller = INFO_OWNER;
            let quote = session.prepare(&request, &state).unwrap();
            let outcome = completed(session.invoke(&request, quote, &state).unwrap());
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            apply_raw_mutations(&state, &outcome);
        }
        let read = simulation_request(
            FinalChainBlockNumber::GENESIS,
            2,
            address_word_input(DPOS_GET_VALIDATOR_SELECTOR, VALIDATOR),
            DPOS_GET_METHOD_GAS,
        );
        let quote = session.prepare(&read, &state).unwrap();
        let result = completed(session.invoke(&read, quote, &state).unwrap());
        assert_eq!(result.status, FinalChainNativeStatus::Success);
        assert_eq!(
            session.session.dpos_state.validator_metadata[&VALIDATOR].description,
            b"static"
        );
        assert!(result.output.windows(6).any(|bytes| bytes == b"static"));
        assert_eq!(
            chain
                .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
                .unwrap(),
            finalized
        );
    });
}
