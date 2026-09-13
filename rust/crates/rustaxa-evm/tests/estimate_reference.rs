use rustaxa_evm::estimate::{EstimateError, EstimateProbe, estimate_gas};

#[test]
fn matches_extracted_cpp_search_results_and_probe_order() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/estimate_reference.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let scenario = case["scenario"].as_u64().unwrap();
        let cap = match scenario {
            1 => 0,
            2 => 1,
            3 => u64::MAX,
            _ => 100,
        };
        let mut probes = Vec::new();
        let result = estimate_gas(cap, |gas| {
            probes.push(gas);
            Ok::<_, ()>(match scenario {
                0 if gas < 61 => EstimateProbe::CodeFailure {
                    error: "execution reverted".into(),
                },
                0 => EstimateProbe::Success { gas_used: 61 },
                1 | 2 => EstimateProbe::Success { gas_used: 0 },
                3 => EstimateProbe::Success {
                    gas_used: u64::MAX - 1,
                },
                4 => EstimateProbe::ConsensusFailure {
                    error: "nonce too low".into(),
                },
                5 => EstimateProbe::CodeFailure {
                    error: "execution reverted: denied".into(),
                },
                6 if gas == 70 => EstimateProbe::ConsensusFailure {
                    error: "future block".into(),
                },
                6 => EstimateProbe::Success { gas_used: 40 },
                7 if gas < 60 => EstimateProbe::CodeFailure {
                    error: "out of gas".into(),
                },
                7 => EstimateProbe::Success { gas_used: 1 },
                _ => unreachable!(),
            })
        });
        assert_eq!(serde_json::json!(probes), case["probes"], "case {scenario}");
        if let Some(expected) = case["result"].as_u64() {
            assert_eq!(result, Ok(expected), "case {scenario}");
        } else {
            let error = match result.unwrap_err() {
                EstimateError::ConsensusFailure { error }
                | EstimateError::CodeFailure { error } => error,
                error => panic!("unexpected error {error:?}"),
            };
            assert_eq!(error, case["error"].as_str().unwrap(), "case {scenario}");
        }
    }
}

fn threshold_probe(
    threshold: u64,
    transcript: &mut Vec<u64>,
) -> impl FnMut(u64) -> Result<EstimateProbe, ()> + '_ {
    move |gas| {
        transcript.push(gas);
        Ok(if gas >= threshold {
            EstimateProbe::Success {
                gas_used: threshold,
            }
        } else {
            EstimateProbe::CodeFailure {
                error: "execution reverted".into(),
            }
        })
    }
}

#[test]
fn searches_with_legacy_five_percent_stop_and_fresh_probe_transcript() {
    let mut transcript = Vec::new();
    let result = estimate_gas(100, threshold_probe(61, &mut transcript)).unwrap();
    assert_eq!(result, 63);
    assert_eq!(transcript, vec![100, 80, 70, 65, 63]);
}

#[test]
fn handles_zero_and_one_unit_caps_without_nonprogress() {
    let mut zero = Vec::new();
    assert_eq!(estimate_gas(0, threshold_probe(0, &mut zero)), Ok(0));
    assert_eq!(zero, vec![0]);

    let mut one_success = Vec::new();
    assert_eq!(estimate_gas(1, threshold_probe(0, &mut one_success)), Ok(0));
    assert_eq!(one_success, vec![1, 0]);

    let mut one_failure = Vec::new();
    assert_eq!(
        estimate_gas(1, |gas| {
            one_failure.push(gas);
            Ok::<EstimateProbe, ()>(if gas == 1 {
                EstimateProbe::Success { gas_used: 0 }
            } else {
                EstimateProbe::CodeFailure {
                    error: "execution reverted".into(),
                }
            })
        }),
        Err(EstimateError::NonProgress { low: 0, high: 1 })
    );
    assert_eq!(one_failure, vec![1, 0]);
}

#[test]
fn supports_maximum_cap_without_arithmetic_overflow() {
    let mut transcript = Vec::new();
    let result = estimate_gas(u64::MAX, threshold_probe(u64::MAX - 1, &mut transcript)).unwrap();
    assert_eq!(result, u64::MAX);
    assert_eq!(transcript, vec![u64::MAX]);
}

#[test]
fn initial_failures_preserve_typed_kind_and_exact_message() {
    let consensus = estimate_gas(100, |_| {
        Ok::<EstimateProbe, ()>(EstimateProbe::ConsensusFailure {
            error: "nonce too low".into(),
        })
    });
    assert_eq!(
        consensus,
        Err(EstimateError::ConsensusFailure {
            error: "nonce too low".into()
        })
    );

    let code = estimate_gas(100, |_| {
        Ok::<EstimateProbe, ()>(EstimateProbe::CodeFailure {
            error: "execution reverted: denied".into(),
        })
    });
    assert_eq!(
        code,
        Err(EstimateError::CodeFailure {
            error: "execution reverted: denied".into()
        })
    );
}

#[test]
fn initial_callback_error_and_gas_overrun_are_terminal() {
    assert_eq!(
        estimate_gas(100, |_| Err::<EstimateProbe, _>("probe unavailable")),
        Err(EstimateError::Callback("probe unavailable"))
    );
    assert_eq!(
        estimate_gas(100, |_| Ok::<EstimateProbe, ()>(EstimateProbe::Success {
            gas_used: 101
        })),
        Err(EstimateError::OutOfGas)
    );
}

#[test]
fn midpoint_consensus_failure_is_terminal_but_code_failure_lowers_bound() {
    let mut consensus_transcript = Vec::new();
    let consensus = estimate_gas(100, |gas| {
        consensus_transcript.push(gas);
        Ok::<EstimateProbe, ()>(if gas == 70 {
            EstimateProbe::ConsensusFailure {
                error: "future block".into(),
            }
        } else {
            EstimateProbe::Success { gas_used: 40 }
        })
    });
    assert_eq!(
        consensus,
        Err(EstimateError::ConsensusFailure {
            error: "future block".into()
        })
    );
    assert_eq!(consensus_transcript, vec![100, 70]);

    let mut code_transcript = Vec::new();
    let code = estimate_gas(100, |gas| {
        code_transcript.push(gas);
        Ok::<EstimateProbe, ()>(if gas < 60 {
            EstimateProbe::CodeFailure {
                error: "out of gas".into(),
            }
        } else {
            EstimateProbe::Success { gas_used: 1 }
        })
    })
    .unwrap();
    assert_eq!(code, 62);
    assert_eq!(code_transcript, vec![100, 50, 75, 62, 56, 59]);
}

#[test]
fn midpoint_callback_error_is_propagated() {
    let mut transcript = Vec::new();
    let result = estimate_gas(100, |gas| {
        transcript.push(gas);
        if gas == 50 {
            Err("interrupted")
        } else {
            Ok(EstimateProbe::Success { gas_used: 1 })
        }
    });
    assert_eq!(result, Err(EstimateError::Callback("interrupted")));
    assert_eq!(transcript, vec![100, 50]);
}
