use super::*;
use serde_json::Value;

const INFO_OWNER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa1,
];

fn with_info_chain(name: &str, test: impl FnOnce(&FinalChain)) {
    with_info_chain_profile(name, false, test);
}

fn with_info_chain_profile(name: &str, cacti: bool, test: impl FnOnce(&FinalChain)) {
    let path = temp_db_path(name);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        crate::FinalChainGenesisAllocation {
            accounts: Vec::new(),
            validators: vec![GenesisValidator {
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
        },
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
            cacti_period: if cacti {
                FinalChainBlockNumber::GENESIS
            } else {
                FinalChainBlockNumber::MAX
            },
            ..Default::default()
        },
        if cacti {
            FinalChainBlockNumber::GENESIS
        } else {
            FinalChainBlockNumber::MAX
        },
    )
    .unwrap();
    test(&chain);
    drop(chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
}

const INFO_ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/native_validator_info/public.json"
));

const ABI_ORACLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/native_validator_info_abi/public.json"
));

#[test]
fn validator_info_abi_failure_binds_exact_quote_and_leaves_next_invocation_available() {
    with_info_chain("validator-info-abi-binding", |chain| {
        let mut session = chain
            .begin_native_simulation(FinalChainBlockNumber::GENESIS)
            .unwrap();
        let state = RawState::from_snapshot(&session.session.dpos_state, BTreeMap::new());
        let request = simulation_request(
            FinalChainBlockNumber::GENESIS,
            0,
            DPOS_SET_VALIDATOR_INFO_SELECTOR.to_vec(),
            20_000,
        );
        let quote = session.prepare(&request, &state).unwrap();
        let mut mismatch = request.clone();
        mismatch.input.push(0);
        assert_eq!(
            session.invoke(&mismatch, quote, &state),
            Err(FinalChainNativeSessionError::QuoteMismatch)
        );
        assert_eq!(state.reads.get(), 0);
        let failure = completed(session.invoke(&request, quote, &state).unwrap());
        assert_eq!(
            failure.status,
            FinalChainNativeStatus::ContractFailure {
                error: "abi: cannot marshal in to go type: length insufficient 0 require 32"
                    .to_owned()
            }
        );
        assert!(failure.raw_mutations.is_empty());
        let oracle: Value = serde_json::from_str(ABI_ORACLE).unwrap();
        let mut valid = simulation_request(
            FinalChainBlockNumber::GENESIS,
            1,
            unhex(&oracle["cases"][0]["input"]),
            20_000,
        );
        valid.caller = INFO_OWNER;
        let quote = session.prepare(&valid, &state).unwrap();
        let success = completed(session.invoke(&valid, quote, &state).unwrap());
        assert_eq!(success.status, FinalChainNativeStatus::Success);
        assert_eq!(success.raw_mutations.len(), 1);
    });
}

fn unhex(value: &Value) -> Vec<u8> {
    let text = value.as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
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
