//! Actual direct escrow default structured traces and private value-frame disposal.
use super::metadata_trace::{TrackingPort, block, requests};
use super::*;
use rustaxa_evm::trace_runner::{
    StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn persisted_escrow_traces_match_actual_go_live_sequence_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_trace/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_trace/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 7);
    let concrete = FixturePath::new("escrow-trace-concrete");
    let app = FixturePath::new("escrow-trace-app");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let chain = native_history_with_phalaenopsis(&app, 0.into());
    for _ in 0..2 {
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        for case in public["cases"].as_array().unwrap() {
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            let original_nonces = prefix
                .iter()
                .chain(&targets)
                .map(|tx| tx.nonce.clone())
                .collect::<Vec<_>>();
            for _ in 0..2 {
                let seen = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let result = run_structured_trace_with_native(
                    &reader,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |selected, execution| {
                        assert_eq!(selected, identity);
                        assert_eq!(execution, 2.into());
                        Ok(TrackingPort {
                            inner: mixed_native::MixedNativeExecutionPort::new(
                                chain
                                    .begin_native_session(execution, selected.period)
                                    .unwrap(),
                            ),
                            seen: seen.clone(),
                            drops: drops.clone(),
                            fail_at: None,
                        })
                    },
                    &block(2),
                    &prefix,
                    &targets,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                )
                .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                assert_eq!(drops.get(), 1);
                assert_eq!(result.state, identity);
                let actual: Value = serde_json::from_slice(&result.output).unwrap();
                assert_eq!(actual, case["result"], "{}", case["name"]);
                assert!(
                    actual
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|row| row["structLogs"].as_array().unwrap().is_empty())
                );
                let seen = seen.borrow();
                // The stale nonce case fails consensus admission before native
                // preparation. All other case nonces increase across the sequence.
                let expected_calls = if case["name"] == "stale_nonce_preserved" {
                    0
                } else {
                    prefix.len() + targets.len()
                };
                assert_eq!(seen.len(), expected_calls, "{}", case["name"]);
                for (index, (invocation, transaction)) in
                    seen.iter().zip(prefix.iter().chain(&targets)).enumerate()
                {
                    assert_eq!(invocation.id.sequence, index as u64);
                    assert_eq!(invocation.id.transaction, transaction.position);
                    assert_eq!(invocation.period, 2.into());
                    assert_eq!(invocation.depth, 0);
                    assert_eq!(invocation.caller, transaction.sender);
                    assert_eq!(invocation.input, transaction.input);
                    assert_eq!(invocation.value.value(), transaction.value.value());
                }
                assert_eq!(
                    prefix
                        .iter()
                        .chain(&targets)
                        .map(|tx| tx.nonce.clone())
                        .collect::<Vec<_>>(),
                    original_nonces
                );
            }
        }
        // Failure after a mutating prefix must dispose its semantic port and
        // journal, retain precise stage/index, and return no partial trace.
        let case = &public["cases"][1];
        let prefix = requests(&case["prefix"], 0);
        let targets = requests(&case["targets"], prefix.len());
        let drops = Rc::new(Cell::new(0));
        let result = run_structured_trace_with_native(
            &reader,
            &NoHistory,
            &Dpos,
            &Dpos,
            |selected, execution| {
                Ok(TrackingPort {
                    inner: mixed_native::MixedNativeExecutionPort::new(
                        chain
                            .begin_native_session(execution, selected.period)
                            .unwrap(),
                    ),
                    seen: Rc::new(RefCell::new(Vec::new())),
                    drops: drops.clone(),
                    fail_at: Some(1),
                })
            },
            &block(2),
            &prefix,
            &targets,
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(TaraxaPhase::Ficus),
        );
        assert!(matches!(
            result,
            Err(StructuredTraceRunnerError::Execution {
                stage: TraceSequenceStage::Target,
                index: 0,
                ..
            })
        ));
        assert_eq!(drops.get(), 1);
        drop(reader);
        assert_eq!(concrete_rows(&concrete), before);
    }
}
