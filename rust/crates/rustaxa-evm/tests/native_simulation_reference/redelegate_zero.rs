//! Exact redelegate DryRunner outputs over reopened, persisted Go seed rows.
//! Each Rust probe owns a fresh real historical native session. This fixture
//! grants absence authority only for its complete synthetic seed.

use super::*;

#[test]
fn persisted_zero_redelegate_simulations_match_actual_go_dry_runner_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_zero_simulation/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_zero_simulation/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["repeat_identical"], true);
    assert_eq!(public["committed_state_unchanged"], true);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    assert_eq!(public["cases"][0]["name"], "zero_before_aspen_two");
    assert_eq!(
        number(&public["cases"][0]["supplied_nonce"]),
        BigUint::from(1_u8) << 512
    );
    assert_eq!(public["cases"][0]["value"], "0");
    assert_eq!(public["cases"][0]["gas"], 200_000);
    assert_eq!(public["cases"][0]["gas_price"], "1");
    let input = bytes(&public["cases"][0]["input"]);
    assert_eq!(
        &input[..4],
        &keccak256(b"reDelegate(address,address,uint256)")[..4]
    );
    assert_eq!(&input[16..36], &address(0x31));
    assert_eq!(&input[48..68], &address(0x32));
    assert_eq!(&input[68..], &[0; 32]);
    assert_eq!(
        public["cases"][0]["output"]["logs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        bytes(&public["cases"][0]["output"]["logs"][0]["data"]),
        [0; 32]
    );
    let concrete = FixturePath::new("zero-redelegate-concrete");
    let app = FixturePath::new("zero-redelegate-app");
    let identity = materialize(&public, &concrete);
    assert_eq!(identity.period, 1.into());
    let before = concrete_rows(&concrete);
    let mut sessions = 0;
    for owner_restart in 0..2 {
        let chain = super::redelegate::history(&app, owner_restart == 0);
        super::redelegate::assert_committed(&chain);
        for _ in 0..2 {
            // Reopen the physical reader independently of semantic-owner reconstruction.
            let reader = CompleteSeedReader::open(&concrete, &public, identity);
            assert_eq!(reader.identity(), identity);
            for case in public["cases"].as_array().unwrap() {
                for _ in 0..2 {
                    let ConcreteRead::Present(sender) = reader.account(address(0xaa)).unwrap()
                    else {
                        panic!("missing sender")
                    };
                    assert_eq!(
                        BigUint::from_bytes_be(&sender.account.nonce.next().to_bytes()),
                        number(&case["output"]["effective_nonce"])
                    );
                    let supplied_nonce =
                        FinalChainNonce::from_bytes(&number(&case["supplied_nonce"]).to_bytes_be())
                            .unwrap();
                    let request = ExecutionTransaction {
                        position: 0.into(),
                        hash: [0; 32],
                        sender: address(0xaa),
                        receiver: Some(bytes(&case["to"]).try_into().unwrap()),
                        nonce: supplied_nonce.clone(),
                        gas_price: ExecutionGasPrice::new(number(&case["gas_price"])),
                        gas_limit: case["gas"].as_u64().unwrap().into(),
                        value: ExecutionValue::new(number(&case["value"])),
                        input: bytes(&case["input"]),
                        canonical_rlp: None,
                        kind: ExecutionTransactionKind::Call,
                    };
                    sessions += 1;
                    let simulated = simulate_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected| {
                            assert_eq!(selected, identity);
                            Ok(mixed_native::SimulationNativeExecutionPort::new(
                                chain.begin_native_simulation(selected.period).unwrap(),
                            ))
                        },
                        &ExecutionBlockContext {
                            period: identity.period,
                            author: address(0x31),
                            timestamp: 1_700_000_001,
                            gas_limit: 500_000.into(),
                            chain_id: 666,
                            difficulty: BigUint::default(),
                        },
                        &request,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    )
                    .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                    assert_eq!(simulated.state, identity);
                    assert_eq!(request.nonce, supplied_nonce);
                    assert_eq!(case["output"]["consensus_error"], "");
                    let TransactionExecutionResult::Executed(result) = simulated.execution else {
                        panic!("Go admitted {}", case["name"])
                    };
                    assert_eq!(result.status, CodeExecutionStatus::Success);
                    assert_eq!(case["output"]["execution_error"], "");
                    assert_eq!(
                        result.gas_used.as_u64(),
                        case["output"]["gas_used"].as_u64().unwrap(),
                        "{}",
                        case["name"]
                    );
                    assert_eq!(
                        result.output,
                        bytes(&case["output"]["return"]),
                        "{}",
                        case["name"]
                    );
                    let logs = result.logs.iter().map(|log| json!({
                    "address": hex::encode(log.address), "topics": log.topics.iter().map(hex::encode).collect::<Vec<_>>(), "data": hex::encode(&log.data),
                })).collect::<Vec<_>>();
                    assert_eq!(
                        logs,
                        *case["output"]["logs"].as_array().unwrap(),
                        "{}",
                        case["name"]
                    );
                }
            }
            assert_eq!(
                chain.dpos_total_amount_delegated(1.into()).unwrap(),
                vec![7, 0xd0]
            );
            super::redelegate::assert_committed(&chain);
            assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            // Reading exact physical rows after disposing both readers is below.
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
        drop(chain);
    }
    assert_eq!(sessions, 8);
}
