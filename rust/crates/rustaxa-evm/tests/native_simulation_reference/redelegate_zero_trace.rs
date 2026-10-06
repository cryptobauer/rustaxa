//! Actual direct redelegate default structured traces and private value-frame disposal.
use super::metadata_trace::{TrackingPort, block, requests};
use super::*;
use rustaxa_evm::contracts::{
    NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
    NativeJournalRead, NativeOutcome, NativePortError, NativeRawOperation, NativeStatus,
};
use rustaxa_evm::trace_runner::{
    StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Confirms the mutating prefix actually completed before the injected target
/// failure. The inner tracking port owns the single-drop witness.
struct ProbePort<'a> {
    inner: TrackingPort<'a>,
    successful: Rc<Cell<usize>>,
    outcomes: Rc<RefCell<Vec<NativeOutcome>>>,
}
impl NativeExecutionPort for ProbePort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        // Effect expectations name the committed base; the journal names the
        // current live overlay. Check caller pairs before the next invocation,
        // including before an injected target failure.
        if let Some(prior) = self.outcomes.borrow().last() {
            for last in [0x31, 0x32] {
                let validator = address(last);
                let caller = address(0xaa);
                let key =
                    ConcreteStorageKey(keccak256([&[2, 0][..], &validator, &caller].concat()).0);
                let mutation = prior.raw_mutations.iter().find(|m| m.key == key).unwrap();
                let NativeRawOperation::Put(value) = &mutation.operation else {
                    panic!("existing caller pair must be retained")
                };
                assert_eq!(
                    journal.raw_storage(address(0xfe), &key).unwrap(),
                    ConcreteRead::Present(value.as_bytes().to_vec())
                );
            }
        }
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        let result = self.inner.invoke(invocation, quote, journal)?;
        if matches!(&result, NativeInvocationResult::Completed(outcome) if outcome.status == NativeStatus::Success)
        {
            self.successful.set(self.successful.get() + 1);
        }
        if let NativeInvocationResult::Completed(outcome) = &result {
            self.outcomes.borrow_mut().push(outcome.clone());
        }
        Ok(result)
    }
}

/// Prove the prefix changed each live caller pair, and zero retained those
/// principals. Structured trace JSON omits prefix effects.
fn assert_principals(outcome: &NativeOutcome, principals: [u64; 2]) {
    assert_eq!(outcome.status, NativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    for (last, principal) in [0x31, 0x32].into_iter().zip(principals) {
        let validator = address(last);
        let caller = address(0xaa);
        let key = ConcreteStorageKey(keccak256([&[2, 0][..], &validator, &caller].concat()).0);
        let mutation = outcome.raw_mutations.iter().find(|m| m.key == key).unwrap();
        let NativeRawOperation::Put(value) = &mutation.operation else {
            panic!("existing caller pair must be retained")
        };
        assert_eq!(
            rlp::Rlp::new(value.as_bytes()).val_at::<u64>(0).unwrap(),
            principal
        );
    }
}

fn assert_prefix_then_zero(outcomes: &[NativeOutcome]) {
    assert_eq!(outcomes.len(), 2);
    assert_principals(&outcomes[0], [700, 1300]);
    assert_principals(&outcomes[1], [700, 1300]);
    assert_eq!(outcomes[1].logs.len(), 1);
    assert_eq!(outcomes[1].logs[0].data, [0; 32]);
}

#[test]
fn persisted_zero_redelegate_traces_match_actual_go_live_sequence_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_zero_trace/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_zero_trace/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 4);
    let concrete = FixturePath::new("zero-redelegate-trace-concrete");
    let app = FixturePath::new("zero-redelegate-trace-app");
    let identity = materialize(&public, &concrete);
    assert_eq!(identity.period, 1.into());
    let before = concrete_rows(&concrete);
    for owner_restart in 0..2 {
        let chain = super::redelegate::history(&app, owner_restart == 0);
        super::redelegate::assert_committed(&chain);
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
                    let outcomes = Rc::new(RefCell::new(Vec::new()));
                    let result = run_structured_trace_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected, execution| {
                            assert_eq!(selected, identity);
                            assert_eq!(execution, 2.into());
                            Ok(ProbePort {
                                successful: Rc::new(Cell::new(0)),
                                outcomes: outcomes.clone(),
                                inner: TrackingPort {
                                    inner: mixed_native::MixedNativeExecutionPort::new(
                                        chain
                                            .begin_native_session(execution, selected.period)
                                            .unwrap(),
                                    ),
                                    seen: seen.clone(),
                                    drops: drops.clone(),
                                    fail_at: None,
                                },
                            })
                        },
                        &block(2),
                        &prefix,
                        &targets,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    )
                    .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
                    assert_eq!(drops.get(), 1);
                    if case["name"] == "single_zero" || case["name"] == "two_zero_targets" {
                        let outcomes = outcomes.borrow();
                        assert_eq!(outcomes.len(), targets.len());
                        for outcome in outcomes.iter() {
                            assert_principals(outcome, [1000, 1000]);
                            assert_eq!(outcome.logs.len(), 1);
                            assert_eq!(outcome.logs[0].data, [0; 32]);
                        }
                    }
                    if case["name"] == "prefix_partial_then_zero" {
                        let outcomes = outcomes.borrow();
                        assert_eq!(outcomes.len(), 2);
                        assert_prefix_then_zero(&outcomes);
                    }
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
            // Hard failures at the prefix or after its successful mutation dispose
            // the whole live port/journal. A fresh retry uses the committed H=1 seed.
            let case = public["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["name"] == "prefix_partial_then_zero")
                .unwrap();
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            for fail_at in [0, 1] {
                let drops = Rc::new(Cell::new(0));
                let successful = Rc::new(Cell::new(0));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let run = |injected| {
                    run_structured_trace_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected, execution| {
                            assert_eq!(selected, identity);
                            assert_eq!(execution, 2.into());
                            Ok(ProbePort {
                                successful: successful.clone(),
                                outcomes: outcomes.clone(),
                                inner: TrackingPort {
                                    inner: mixed_native::MixedNativeExecutionPort::new(
                                        chain
                                            .begin_native_session(execution, selected.period)
                                            .unwrap(),
                                    ),
                                    seen: Rc::new(RefCell::new(Vec::new())),
                                    drops: drops.clone(),
                                    fail_at: injected,
                                },
                            })
                        },
                        &block(2),
                        &prefix,
                        &targets,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    )
                };
                let result = run(Some(fail_at));
                let expected_stage = if fail_at == 0 {
                    TraceSequenceStage::Prefix
                } else {
                    TraceSequenceStage::Target
                };
                assert!(
                    matches!(result, Err(StructuredTraceRunnerError::Execution { stage, index: 0, .. }) if stage == expected_stage)
                );
                assert_eq!(drops.get(), 1);
                assert_eq!(successful.get(), fail_at as usize);
                assert_eq!(outcomes.borrow().len(), fail_at as usize);
                if fail_at == 1 {
                    assert_principals(&outcomes.borrow()[0], [700, 1300]);
                }
                outcomes.borrow_mut().clear();
                super::redelegate::assert_committed(&chain);
                assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
                assert_eq!(concrete_rows(&concrete), before);
                let retry = run(None).unwrap();
                assert_eq!(retry.state, identity);
                assert_eq!(
                    serde_json::from_slice::<Value>(&retry.output).unwrap(),
                    case["result"]
                );
                assert_eq!(drops.get(), 2);
                assert_eq!(successful.get(), fail_at as usize + 2);
                assert_eq!(outcomes.borrow().len(), 2);
                assert_prefix_then_zero(&outcomes.borrow());
                assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            }
            super::redelegate::assert_committed(&chain);
            assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
    }
}
