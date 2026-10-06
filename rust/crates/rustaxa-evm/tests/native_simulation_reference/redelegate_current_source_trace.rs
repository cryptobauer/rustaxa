//! Actual one direct H2 trace over complete unchanged H1 with real partial300
//! prefix and full700 target in one public session/journal. Internal write witnesses
//! use accepted staged/frame authority; default Go JSON omits native effects.
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
    collections::BTreeMap,
    rc::Rc,
};

/// Confirms the mutating prefix actually completed before the injected target
/// failure. The inner tracking port owns the single-drop witness.
struct ProbePort<'a> {
    inner: TrackingPort<'a>,
    successful: Rc<Cell<usize>>,
    outcomes: Rc<RefCell<Vec<NativeOutcome>>>,
    target_fail: Option<usize>,
    target_reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
}
impl NativeExecutionPort for ProbePort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let outcomes = self.outcomes.borrow();
        let reduced = outcomes
            .iter()
            .flat_map(|outcome| outcome.raw_mutations.iter())
            .map(|m| (m.key, &m.operation))
            .collect::<BTreeMap<_, _>>();
        for (key, operation) in reduced {
            assert_eq!(
                logical(journal.raw_storage(address(0xfe), &key).unwrap()),
                raw_value(operation)
            );
        }
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        assert_eq!(quote.required_gas.as_u64(), 80_000);
        let result = if invocation.id.sequence == 1 {
            let target = TargetJournal {
                inner: journal,
                fail_at: self.target_fail,
                reads: self.target_reads.clone(),
            };
            self.inner.invoke(invocation, quote, &target)?
        } else {
            self.inner.invoke(invocation, quote, journal)?
        };
        if let NativeInvocationResult::Completed(outcome) = &result {
            let mut prefix = BTreeMap::new();
            for mutation in &outcome.raw_mutations {
                let prior = prefix.entry(mutation.key).or_insert_with(|| {
                    logical(
                        journal
                            .raw_storage(mutation.address, &mutation.key)
                            .unwrap(),
                    )
                });
                assert_eq!(logical(mutation.expected.clone()), *prior);
                *prior = raw_value(&mutation.operation);
            }
        }
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

use rustaxa_evm::contracts::{NativeJournalAccount, NativeJournalReadError};

/// Target-only forwarding read port; overlay reads are not hidden by backing caches.
struct TargetJournal<'a> {
    inner: &'a dyn NativeJournalRead,
    fail_at: Option<usize>,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
}
impl NativeJournalRead for TargetJournal<'_> {
    fn account(&self, address: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.inner.account(address)
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        assert_eq!(address, super::address(0xfe));
        let index = self.reads.borrow().len();
        self.reads.borrow_mut().push(*key);
        if self.fail_at == Some(index) {
            return Err(NativeJournalReadError::Invariant(
                "injected target authentication read".into(),
            ));
        }
        self.inner.raw_storage(address, key)
    }
}
fn logical(read: ConcreteRead<Vec<u8>>) -> Option<Vec<u8>> {
    match read {
        ConcreteRead::Present(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}
fn raw_value(operation: &NativeRawOperation) -> Option<Vec<u8>> {
    match operation {
        NativeRawOperation::Put(value) => Some(value.as_bytes().to_vec()),
        NativeRawOperation::Delete => None,
    }
}
/// Rust internal effect witness; Go structured target JSON omits prefix effects.
fn assert_current_source(outcome: &NativeOutcome) {
    assert_eq!(outcome.status, NativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    assert_eq!(outcome.raw_mutations.len(), 18);
    let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
    let caller = address(0xd1);
    let prefix = [&[2, 1][..], &caller].concat();
    let operations = |key| {
        outcome
            .raw_mutations
            .iter()
            .filter(|m| m.key == key)
            .map(|m| raw_value(&m.operation))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        operations(key(&[&prefix, &[2], &2_u32.to_le_bytes()])),
        vec![None, Some(address(0x32).to_vec())]
    );
    assert_eq!(
        operations(key(&[&prefix, &[1]])),
        vec![
            Some(1_u32.to_le_bytes().to_vec()),
            Some(2_u32.to_le_bytes().to_vec())
        ]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &address(0x31)])),
        vec![None]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &address(0x32)])),
        vec![Some(2_u32.to_le_bytes().to_vec())]
    );
    assert_eq!(
        operations(key(&[&[2, 0], &address(0x31), &caller])),
        vec![None]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &1_u32.to_le_bytes()])),
        vec![Some(address(0x33).to_vec())]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &address(0x33)])),
        vec![Some(1_u32.to_le_bytes().to_vec())]
    );
    let destination = operations(key(&[&[2, 0], &address(0x32), &caller]));
    let rlp = rlp::Rlp::new(destination[0].as_ref().unwrap());
    assert_eq!(rlp.val_at::<u64>(0).unwrap(), 700);
    assert_eq!(rlp.val_at::<u64>(1).unwrap(), 2);
    for (last, stake, current_count) in [(0x31, 1000_u64, 2_u64), (0x32, 1700, 2)] {
        let validator = address(last);
        let values = operations(key(&[&[0, 0], &validator]));
        assert_eq!(
            rlp::Rlp::new(values.last().unwrap().as_ref().unwrap())
                .at(0)
                .unwrap()
                .val_at::<u64>(0)
                .unwrap(),
            stake
        );
        let nodes = if last == 0x31 {
            vec![(&[2][..], current_count)]
        } else {
            vec![(&[][..], 1_u64), (&[2][..], current_count)]
        };
        for (block, count) in nodes {
            let values = operations(key(&[&[1], &validator, block]));
            let node = rlp::Rlp::new(values.last().unwrap().as_ref().unwrap());
            assert_eq!(node.val_at::<u64>(0).unwrap(), 0);
            assert_eq!(node.val_at::<u64>(1).unwrap(), count);
        }
    }
    assert_eq!(
        operations(key(&[&[1], &address(0x31), &[2]])),
        vec![
            Some(hex::decode("c28001").unwrap()),
            Some(hex::decode("c28002").unwrap())
        ]
    );
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(outcome.logs[0].data, U256::from(700).to_big_endian());
}
fn assert_prefix(outcome: &NativeOutcome) {
    assert_eq!(outcome.status, NativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    assert_eq!(outcome.raw_mutations.len(), 12);
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(outcome.logs[0].data, U256::from(300).to_big_endian());
    let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
    let final_write = |key| {
        outcome
            .raw_mutations
            .iter()
            .rev()
            .find(|m| m.key == key)
            .map(|m| raw_value(&m.operation))
            .unwrap()
    };
    for validator in [address(0x31), address(0x33)] {
        assert_eq!(
            final_write(key(&[&[1], &validator, &[2]])),
            Some(hex::decode("c28002").unwrap())
        );
        let pair = final_write(key(&[&[2, 0], &validator, &address(0xd1)])).unwrap();
        let rlp = rlp::Rlp::new(&pair);
        assert_eq!(
            rlp.val_at::<u64>(0).unwrap(),
            if validator == address(0x31) {
                700
            } else {
                1300
            }
        );
        assert_eq!(rlp.val_at::<u64>(1).unwrap(), 2);
    }
}
fn assert_sequence(outcomes: &[NativeOutcome]) {
    assert_eq!(outcomes.len(), 2);
    assert_prefix(&outcomes[0]);
    assert_current_source(&outcomes[1]);
}
#[test]
fn persisted_current_source_trace_matches_real_prefix_and_full_target() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_current_source_trace/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_current_source_trace/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    let simulation: Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_swap_append_simulation/public.json")).unwrap();
    assert_eq!(public["state_before"], simulation["state_before"]);
    assert_eq!(public["period"], 2);
    let concrete = FixturePath::new("current-source-trace-concrete");
    let app = FixturePath::new("current-source-trace-app");
    let identity = materialize(&public, &concrete);
    assert_eq!(identity.period, 1.into());
    let before = concrete_rows(&concrete);
    let mut authenticated_keys: Option<Vec<ConcreteStorageKey>> = None;
    for owner_restart in 0..2 {
        let chain = super::redelegate_swap_append::history(&app, owner_restart == 0);
        super::redelegate_swap_append::assert_committed(&chain);
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
                    let observed_reads = Rc::new(RefCell::new(Vec::new()));
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
                                target_fail: None,
                                target_reads: observed_reads.clone(),
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
                    assert_sequence(&outcomes.borrow());
                    let measured = observed_reads.borrow().clone();
                    assert_eq!(measured.len(), 18);
                    let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
                    let membership = [&[2, 1][..], &address(0xd1)].concat();
                    assert_eq!(
                        &measured[15..],
                        &[
                            key(&[&membership, &[2], &address(0x33)]),
                            key(&[&membership, &[2], &1_u32.to_le_bytes()]),
                            key(&[&membership, &[2], &2_u32.to_le_bytes()]),
                        ]
                    );
                    if let Some(expected) = &authenticated_keys {
                        assert_eq!(&measured, expected)
                    } else {
                        authenticated_keys = Some(measured)
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
                    let expected_calls = prefix.len() + targets.len();
                    assert_eq!(seen.len(), expected_calls, "{}", case["name"]);
                    for (index, (invocation, transaction)) in
                        seen.iter().zip(prefix.iter().chain(&targets)).enumerate()
                    {
                        assert_eq!(invocation.id.sequence, index as u64);
                        assert_eq!(invocation.id.transaction, transaction.position);
                        assert_eq!(invocation.period, 2.into());
                        assert_eq!(invocation.depth, 0);
                        assert_eq!(
                            invocation.kind,
                            rustaxa_evm::contracts::NativeCallKind::Call
                        );
                        assert!(!invocation.is_static);
                        assert_eq!(invocation.supplied_gas.as_u64(), 178088);
                        assert_eq!(invocation.caller, address(0xd1));
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
                .find(|case| case["name"] == "partial_prefix_then_full_current_source")
                .unwrap();
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            for mode in 0..(2 + authenticated_keys.as_ref().unwrap().len()) {
                let fail_at = if mode == 0 { 0 } else { 1 };
                let target_reads = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let successful = Rc::new(Cell::new(0));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let run = |injected: Option<u64>| {
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
                                target_fail: if injected.is_some() && mode >= 2 {
                                    Some(mode - 2)
                                } else {
                                    None
                                },
                                target_reads: target_reads.clone(),
                                inner: TrackingPort {
                                    inner: mixed_native::MixedNativeExecutionPort::new(
                                        chain
                                            .begin_native_session(execution, selected.period)
                                            .unwrap(),
                                    ),
                                    seen: Rc::new(RefCell::new(Vec::new())),
                                    drops: drops.clone(),
                                    fail_at: if mode < 2 { injected } else { None },
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
                    assert_prefix(&outcomes.borrow()[0]);
                }
                if mode < 2 {
                    assert!(format!("{result:?}").contains("injected after prefix"));
                }
                if mode >= 2 {
                    assert!(format!("{result:?}").contains("injected target authentication read"));
                    assert_eq!(
                        *target_reads.borrow(),
                        authenticated_keys.as_ref().unwrap()[..=mode - 2]
                    );
                }
                target_reads.borrow_mut().clear();
                outcomes.borrow_mut().clear();
                super::redelegate_swap_append::assert_committed(&chain);
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
                assert_sequence(&outcomes.borrow());
                assert_eq!(
                    &*target_reads.borrow(),
                    authenticated_keys.as_ref().unwrap()
                );
                assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            }
            super::redelegate_swap_append::assert_committed(&chain);
            assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
    }
}
