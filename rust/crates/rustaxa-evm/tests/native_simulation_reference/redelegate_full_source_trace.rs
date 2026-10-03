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

/// Internal branch witnesses; the Go structured JSON does not expose raw effects.
fn assert_full_removal(outcome: &NativeOutcome) {
    assert_eq!(outcome.status, NativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    let caller = address(0xaa);
    let source = address(0x31);
    let pair = ConcreteStorageKey(keccak256([&[2, 0][..], &source, &caller].concat()).0);
    let prefix = [&[2, 1][..], &caller].concat();
    let count = ConcreteStorageKey(keccak256([&prefix[..], &[1]].concat()).0);
    let position = ConcreteStorageKey(keccak256([&prefix[..], &[2], &source].concat()).0);
    let operation = |key| {
        &outcome
            .raw_mutations
            .iter()
            .find(|m| m.key == key)
            .unwrap()
            .operation
    };
    assert_eq!(operation(pair), &NativeRawOperation::Delete);
    assert_eq!(operation(position), &NativeRawOperation::Delete);
    let NativeRawOperation::Put(value) = operation(count) else {
        panic!("missing membership count")
    };
    assert_eq!(value.as_bytes(), &1_u32.to_le_bytes());
}

fn assert_reverse_recreation(outcome: &NativeOutcome) {
    assert_eq!(outcome.status, NativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    let caller = address(0xaa);
    let destination = address(0x31);
    let source = address(0x32);
    let pair = |validator: [u8; 20]| {
        ConcreteStorageKey(keccak256([&[2, 0][..], &validator, &caller].concat()).0)
    };
    let prefix = [&[2, 1][..], &caller].concat();
    let count = ConcreteStorageKey(keccak256([&prefix[..], &[1]].concat()).0);
    let item = ConcreteStorageKey(keccak256([&prefix[..], &[2], &2_u32.to_le_bytes()].concat()).0);
    let position = ConcreteStorageKey(keccak256([&prefix[..], &[2], &destination].concat()).0);
    let mutation = |key| outcome.raw_mutations.iter().find(|m| m.key == key).unwrap();
    let new_pair = mutation(pair(destination));
    assert!(
        matches!(&new_pair.expected, ConcreteRead::Absent)
            || matches!(&new_pair.expected, ConcreteRead::Present(value) if value.is_empty())
    );
    for (validator, principal) in [(destination, 300_u64), (source, 1700)] {
        let NativeRawOperation::Put(value) = &mutation(pair(validator)).operation else {
            panic!("missing delegation")
        };
        let row = rlp::Rlp::new(value.as_bytes());
        assert_eq!(row.val_at::<u64>(0).unwrap(), principal);
        assert_eq!(row.val_at::<u64>(1).unwrap(), 2);
    }
    for (key, bytes) in [
        (count, 2_u32.to_le_bytes().to_vec()),
        (position, 2_u32.to_le_bytes().to_vec()),
        (item, destination.to_vec()),
    ] {
        let NativeRawOperation::Put(value) = &mutation(key).operation else {
            panic!("missing recreated membership")
        };
        assert_eq!(value.as_bytes(), bytes);
    }
}

#[test]
fn persisted_full_source_traces_match_actual_go_live_sequence_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_full_source_trace/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_full_source_trace/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 9);
    let concrete = FixturePath::new("full-source-trace-concrete");
    let app = FixturePath::new("full-source-trace-app");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    for owner_restart in 0..2 {
        let chain = super::redelegate_full_source::history(&app, owner_restart == 0);
        super::redelegate_full_source::assert_committed(&chain);
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
                    assert_eq!(drops.get(), 1);
                    if case["name"] == "prefix_full_then_reverse_partial" {
                        let outcomes = outcomes.borrow();
                        assert_eq!(outcomes.len(), 2);
                        assert_full_removal(&outcomes[0]);
                        assert_reverse_recreation(&outcomes[1]);
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
                .find(|case| case["name"] == "prefix_full_then_reverse_partial")
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
                    assert_full_removal(&outcomes.borrow()[0]);
                }
                outcomes.borrow_mut().clear();
                super::redelegate_full_source::assert_committed(&chain);
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
                assert_full_removal(&outcomes.borrow()[0]);
                assert_reverse_recreation(&outcomes.borrow()[1]);
            }
            super::redelegate_full_source::assert_committed(&chain);
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
    }
}
