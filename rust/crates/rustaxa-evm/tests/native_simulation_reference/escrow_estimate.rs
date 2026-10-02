//! Actual escrow execution probes composed with the legacy gas search.
//! The C++ reference consumes the same independently executed Go transcript.

use super::*;
use rustaxa_evm::estimate::{EstimateError, EstimateProbe, estimate_gas};

#[test]
fn persisted_escrow_estimates_match_cpp_search_and_actual_go_probes() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_estimate/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_estimate/local.json"
    ))
    .unwrap();
    let cpp: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_estimate/cpp.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["cases"].as_array().unwrap().len(), 5);
    assert_eq!(cpp["cases"].as_array().unwrap().len(), 5);
    assert_eq!(public["state_before"], public["state_after"]);
    let concrete = FixturePath::new("escrow-estimate-concrete");
    let app = FixturePath::new("escrow-estimate-app");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let chain = native_history_with_phalaenopsis(&app, 0.into());
    for _ in 0..2 {
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        for (case, expected) in public["cases"]
            .as_array()
            .unwrap()
            .iter()
            .zip(cpp["cases"].as_array().unwrap())
        {
            assert_eq!(case["name"], expected["name"]);
            let probes = case["probes"].as_array().unwrap();
            for _ in 0..2 {
                let mut index = 0;
                let estimate = estimate_gas(
                    case["cap"].as_u64().unwrap(),
                    |gas| -> Result<EstimateProbe, String> {
                        let probe = probes
                            .get(index)
                            .expect("C++/Go probe transcript has this call");
                        index += 1;
                        assert_eq!(
                            gas,
                            probe["gas"].as_u64().unwrap(),
                            "{} probe {}",
                            case["name"],
                            index
                        );
                        let nonce = FinalChainNonce::from_bytes(
                            &number(&probe["supplied_nonce"]).to_bytes_be(),
                        )
                        .unwrap();
                        let request = ExecutionTransaction {
                            position: 0.into(),
                            hash: [0; 32],
                            sender: address(0xaa),
                            receiver: Some(bytes(&probe["to"]).try_into().unwrap()),
                            nonce: nonce.clone(),
                            gas_price: ExecutionGasPrice::new(number(&probe["gas_price"])),
                            gas_limit: gas.into(),
                            value: ExecutionValue::new(number(&probe["value"])),
                            input: bytes(&probe["input"]),
                            canonical_rlp: None,
                            kind: ExecutionTransactionKind::Call,
                        };
                        let ConcreteRead::Present(sender) = reader.account(request.sender).unwrap()
                        else {
                            panic!("missing sender")
                        };
                        assert_eq!(
                            BigUint::from_bytes_be(&sender.account.nonce.next().to_bytes()),
                            number(&probe["output"]["effective_nonce"])
                        );
                        let result = simulate_with_native(
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
                        .map_err(|error| format!("{error:?}"))?;
                        assert_eq!(result.state, identity);
                        assert_eq!(request.nonce, nonce);
                        match result.execution {
                            TransactionExecutionResult::ConsensusFailure(result) => {
                                assert_eq!(
                                    result.error,
                                    rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
                                );
                                assert_eq!(
                                    probe["output"]["consensus_error"],
                                    "intrinsic gas too low"
                                );
                                assert_eq!(
                                    result.gas_used.as_u64(),
                                    probe["output"]["gas_used"].as_u64().unwrap()
                                );
                                assert_eq!(result.output, bytes(&probe["output"]["return"]));
                                assert_eq!(probe["output"]["execution_error"], "");
                                assert!(probe["output"]["logs"].as_array().unwrap().is_empty());
                                Ok(EstimateProbe::ConsensusFailure {
                                    error: "intrinsic gas too low".into(),
                                })
                            }
                            TransactionExecutionResult::Executed(result) => {
                                assert_eq!(probe["output"]["consensus_error"], "");
                                let error = match &result.status {
                                    CodeExecutionStatus::Success => "",
                                    CodeExecutionStatus::Failure(CodeExecutionError::Native(
                                        error,
                                    )) => error.error.as_str(),
                                    CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => {
                                        "out of gas"
                                    }
                                    status => panic!("unexpected {status:?}"),
                                };
                                assert_eq!(
                                    error,
                                    probe["output"]["execution_error"].as_str().unwrap()
                                );
                                assert_eq!(
                                    result.gas_used.as_u64(),
                                    probe["output"]["gas_used"].as_u64().unwrap()
                                );
                                assert_eq!(result.output, bytes(&probe["output"]["return"]));
                                let logs = result.logs.iter().map(|log| json!({"address": hex::encode(log.address), "topics": log.topics.iter().map(hex::encode).collect::<Vec<_>>(), "data": hex::encode(&log.data)})).collect::<Vec<_>>();
                                assert_eq!(logs, *probe["output"]["logs"].as_array().unwrap());
                                if error.is_empty() {
                                    Ok(EstimateProbe::Success {
                                        gas_used: result.gas_used.as_u64(),
                                    })
                                } else {
                                    Ok(EstimateProbe::CodeFailure {
                                        error: error.into(),
                                    })
                                }
                            }
                        }
                    },
                );
                assert_eq!(index, probes.len());
                assert_eq!(index as u64, expected["consumed"].as_u64().unwrap());
                match estimate {
                    Ok(value) => assert_eq!(value, expected["result"].as_u64().unwrap()),
                    Err(
                        EstimateError::CodeFailure { error }
                        | EstimateError::ConsensusFailure { error },
                    ) => assert_eq!(error, expected["error"].as_str().unwrap()),
                    Err(error) => panic!("unexpected estimator error {error:?}"),
                }
            }
        }
        drop(reader);
        assert_eq!(concrete_rows(&concrete), before);
    }
}
