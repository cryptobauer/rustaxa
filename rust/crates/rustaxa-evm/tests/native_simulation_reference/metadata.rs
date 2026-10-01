//! Exact metadata DryRunner outputs over reopened, persisted Go seed rows.
//! Each Rust probe owns a fresh real historical native session. This fixture
//! grants absence authority only for its complete synthetic seed.

use super::*;

#[test]
fn persisted_metadata_simulations_match_actual_go_dry_runner_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_metadata_simulation/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_metadata_simulation/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["repeat_identical"], true);
    assert_eq!(public["committed_state_unchanged"], true);
    assert_eq!(public["cases"].as_array().unwrap().len(), 9);
    let concrete = FixturePath::new("metadata-concrete");
    let app = FixturePath::new("metadata-app");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let chain = native_history(&app);
    for _ in 0..2 {
        // Reopen the physical concrete reader over the same fixed semantic owner.
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        for case in public["cases"].as_array().unwrap() {
            for _ in 0..2 {
                let ConcreteRead::Present(sender) = reader.account(address(0xaa)).unwrap() else {
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
                if let TransactionExecutionResult::ConsensusFailure(result) = &simulated.execution {
                    assert_eq!(
                        result.error,
                        rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
                    );
                    assert_eq!(case["output"]["consensus_error"], "intrinsic gas too low");
                    assert_eq!(
                        result.gas_used.as_u64(),
                        case["output"]["gas_used"].as_u64().unwrap()
                    );
                    assert_eq!(result.output, bytes(&case["output"]["return"]));
                    assert_eq!(case["output"]["execution_error"], "");
                    assert!(case["output"]["logs"].as_array().unwrap().is_empty());
                    continue;
                }
                assert_eq!(case["output"]["consensus_error"], "");
                let TransactionExecutionResult::Executed(result) = simulated.execution else {
                    panic!("Go admitted {}", case["name"])
                };
                let error = match &result.status {
                    CodeExecutionStatus::Success => "",
                    CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) => {
                        error.error.as_str()
                    }
                    CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas",
                    status => panic!("{} unexpected {status:?}", case["name"]),
                };
                assert_eq!(
                    error,
                    case["output"]["execution_error"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
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
            vec![110]
        );
        // Reading exact physical rows after disposing both readers is below.
        drop(reader);
        assert_eq!(concrete_rows(&concrete), before);
    }
}
