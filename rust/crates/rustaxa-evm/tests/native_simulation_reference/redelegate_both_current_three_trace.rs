//! Actual one direct H2 trace over complete unchanged H1 with real partial300
//! prefix and full400 target in one public session/journal. Internal write witnesses
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
    prefix_fail: Option<usize>,
    prefix_corrupt: bool,
    prefix_reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    witness: Value,
    target_fail: Option<usize>,
    target_corrupt: bool,
    target_reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
}
impl NativeExecutionPort for ProbePort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let view = if invocation.id.sequence == 0 {
            &self.witness["before"]
        } else {
            &self.witness["steps"][0]["after"]
        };
        assert_live_raw(journal, &view["raw"]);
        // Envelope nonce update precedes native preparation; both wide values
        // match the separately observed actual live Go account state.
        let caller = invocation.caller;
        let expected = &self.witness["steps"][invocation.id.sequence as usize]["after"]["accounts"]
            [hex::encode(caller)];
        assert_eq!(
            BigUint::from_bytes_be(&journal.account(caller).unwrap().nonce.to_bytes()),
            number(&expected["nonce"])
        );
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
                corrupt: self.target_corrupt,
                reads: self.target_reads.clone(),
            };
            self.inner.invoke(invocation, quote, &target)?
        } else {
            let prefix = TargetJournal {
                inner: journal,
                fail_at: self.prefix_fail,
                corrupt: self.prefix_corrupt,
                reads: self.prefix_reads.clone(),
            };
            self.inner.invoke(invocation, quote, &prefix)?
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
        if let NativeInvocationResult::Completed(outcome) = &result {
            assert_live_after(
                journal,
                outcome,
                &self.witness["steps"][invocation.id.sequence as usize]["after"]["raw"],
            );
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
    corrupt: bool,
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
            if self.corrupt {
                return Ok(ConcreteRead::Present(vec![0xff]));
            }
            return Err(NativeJournalReadError::Invariant(
                "injected trace authentication read".into(),
            ));
        }
        self.inner.raw_storage(address, key)
    }
}
fn hash(key: &str) -> [u8; 32] {
    hex::decode(key).unwrap().try_into().unwrap()
}
fn expected_raw(row: &Value) -> Option<Vec<u8>> {
    (row["present"] == true).then(|| bytes(&row["value"]))
}
fn assert_live_raw(journal: &dyn NativeJournalRead, raw: &Value) {
    for (key, row) in raw.as_object().unwrap() {
        assert_eq!(
            logical(
                journal
                    .raw_storage(address(0xfe), &ConcreteStorageKey(hash(key)))
                    .unwrap()
            ),
            expected_raw(row),
            "live raw {key}"
        );
    }
}
fn assert_live_after(journal: &dyn NativeJournalRead, outcome: &NativeOutcome, raw: &Value) {
    for (key, row) in raw.as_object().unwrap() {
        let key = ConcreteStorageKey(hash(key));
        let actual = outcome
            .raw_mutations
            .iter()
            .rev()
            .find(|m| m.key == key)
            .map(|m| raw_value(&m.operation))
            .unwrap_or_else(|| logical(journal.raw_storage(address(0xfe), &key).unwrap()));
        assert_eq!(actual, expected_raw(row), "live after {key:?}");
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
fn assert_existing_current(outcome: &NativeOutcome) {
    assert_eq!(outcome.status, NativeStatus::Success);
    assert!(outcome.account_mutations.is_empty());
    assert_eq!(outcome.raw_mutations.len(), 15);
    let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
    let caller = super::redelegate_both_current_three::caller(
        &super::redelegate_both_current_three::corpus(),
    );
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
        vec![None]
    );
    assert_eq!(
        operations(key(&[&prefix, &[1]])),
        vec![Some(1_u32.to_le_bytes().to_vec())]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &address(0x31)])),
        vec![None]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &address(0x32)])),
        vec![Some(1_u32.to_le_bytes().to_vec())]
    );
    assert_eq!(
        operations(key(&[&prefix, &[2], &1_u32.to_le_bytes()])),
        vec![Some(address(0x32).to_vec())]
    );
    assert_eq!(
        operations(key(&[&[2, 0], &address(0x31), &caller])),
        vec![None]
    );
    let destination = operations(key(&[&[2, 0], &address(0x32), &caller]));
    assert_eq!(destination.len(), 1);
    let pair = rlp::Rlp::new(destination[0].as_ref().unwrap());
    assert_eq!(pair.val_at::<u64>(0).unwrap(), 2000);
    assert_eq!(pair.val_at::<u64>(1).unwrap(), 2);
    for (last, stake, final_node) in [(0x31, 900_u64, "c28002"), (0x32, 3100, "c28003")] {
        let validator = address(last);
        let values = operations(key(&[&[0, 0], &validator]));
        let state = rlp::Rlp::new(values.last().unwrap().as_ref().unwrap());
        assert_eq!(state.at(0).unwrap().val_at::<u64>(0).unwrap(), stake);
        assert_eq!(
            operations(key(&[&[1], &validator, &[2]])),
            vec![
                Some(hex::decode("c28001").unwrap()),
                Some(hex::decode(final_node).unwrap())
            ]
        );
        assert!(operations(key(&[&[1], &validator, &[]])).is_empty());
        assert!(
            operations(key(&[
                &[2, 0],
                &validator,
                &super::redelegate_both_current_three::owner(
                    &super::redelegate_both_current_three::corpus()
                )
            ]))
            .is_empty()
        );
    }
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(outcome.logs[0].data, U256::from(400).to_big_endian());
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
    for validator in [address(0x31), address(0x32)] {
        for preserved in [
            key(&[&[1], &validator, &[]]),
            key(&[
                &[2, 0],
                &validator,
                &super::redelegate_both_current_three::owner(
                    &super::redelegate_both_current_three::corpus(),
                ),
            ]),
        ] {
            assert!(outcome.raw_mutations.iter().all(|m| m.key != preserved));
        }
        let encoded = final_write(key(&[&[0, 0], &validator])).unwrap();
        assert_eq!(
            rlp::Rlp::new(&encoded)
                .at(0)
                .unwrap()
                .val_at::<u64>(0)
                .unwrap(),
            if validator == address(0x31) {
                1300
            } else {
                2700
            }
        );
        assert_eq!(
            final_write(key(&[&[1], &validator, &[2]])),
            Some(hex::decode("c28002").unwrap())
        );
        let old_operations = outcome
            .raw_mutations
            .iter()
            .filter(|m| m.key == key(&[&[1], &validator, &[1]]))
            .map(|m| raw_value(&m.operation))
            .collect::<Vec<_>>();
        assert_eq!(
            old_operations,
            vec![
                Some(hex::decode("c28002").unwrap()),
                Some(hex::decode("c28001").unwrap())
            ]
        );
        assert_eq!(
            final_write(key(&[&[1], &validator, &[1]])),
            Some(hex::decode("c28001").unwrap())
        );
        let pair = final_write(key(&[
            &[2, 0],
            &validator,
            &super::redelegate_both_current_three::caller(
                &super::redelegate_both_current_three::corpus(),
            ),
        ]))
        .unwrap();
        let rlp = rlp::Rlp::new(&pair);
        assert_eq!(
            rlp.val_at::<u64>(0).unwrap(),
            if validator == address(0x31) {
                400
            } else {
                1600
            }
        );
        assert_eq!(rlp.val_at::<u64>(1).unwrap(), 2);
    }
}
fn assert_sequence(outcomes: &[NativeOutcome]) {
    assert_eq!(outcomes.len(), 2);
    assert_prefix(&outcomes[0]);
    assert_existing_current(&outcomes[1]);
}
/// Persisted receipts and complete public validator metadata remain immutable.
fn public_identity(chain: &FinalChain, public: &Value) -> Vec<Vec<u8>> {
    let mut result = (0..2_u32)
        .map(|position| {
            chain
                .transaction_receipt_rlp(1.into(), position.into())
                .unwrap()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for last in [0x31, 0x32] {
        let mut input = keccak256(b"getValidator(address)")[..4].to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&address(last));
        let row = chain
            .call(rustaxa_types::FinalChainCallRequest {
                block_number: 1.into(),
                sender: super::redelegate_both_current_three::caller(public),
                receiver: Some(address(0xfe)),
                value: U256::zero().into(),
                gas_price: U256::zero().into(),
                gas_limit: 1000000.into(),
                input,
            })
            .unwrap();
        assert!(row.code_retval.len() >= 64);
        result.push(row.code_retval);
    }
    result
}
#[test]
fn persisted_both_current_three_trace_matches_real_prefix_and_full_target() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_three_trace/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_three_trace/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    let simulation: Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_three_simulation/public.json")).unwrap();
    assert_eq!(public["state_before"], simulation["state_before"]);
    assert_eq!(public["period"], 2);
    let concrete = FixturePath::new("both-current-three-trace-concrete");
    let app = FixturePath::new("both-current-three-trace-app");
    let identity = materialize(&public, &concrete);
    assert_eq!(identity.period, 1.into());
    assert_eq!(
        public["state_before"]["seed_rows"]
            .as_array()
            .unwrap()
            .len(),
        108
    );
    let before = concrete_rows(&concrete);
    let mut authenticated_keys: Option<Vec<ConcreteStorageKey>> = None;
    let mut prefix_keys: Option<Vec<ConcreteStorageKey>> = None;
    let mut failures = 0;
    let mut sessions = 0;
    for owner_restart in 0..2 {
        let chain =
            super::redelegate_both_current_three::history(&app, owner_restart == 0, &simulation);
        super::redelegate_both_current_three::assert_committed(&chain, &simulation);
        let owner_header = chain.block_header(1.into()).unwrap().unwrap();
        let owner_identity = public_identity(&chain, &simulation);
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
                    let prefix_reads = Rc::new(RefCell::new(Vec::new()));
                    sessions += 1;
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
                                prefix_fail: None,
                                prefix_corrupt: false,
                                prefix_reads: prefix_reads.clone(),
                                witness: public["live_witness"].clone(),
                                target_fail: None,
                                target_corrupt: false,
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
                    let measured_prefix = prefix_reads.borrow().clone();
                    assert_eq!(
                        measured_prefix
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len(),
                        measured_prefix.len()
                    );
                    for last in [0x31, 0x32] {
                        let key = ConcreteStorageKey(hash(
                            public["live_witness"]["node_keys"][format!("{last:02x}01")]
                                .as_str()
                                .unwrap(),
                        ));
                        assert!(measured_prefix.contains(&key));
                    }
                    println!(
                        "count3-H1 H2 prefix authentication keys: {}",
                        measured_prefix.len()
                    );
                    if let Some(expected) = &prefix_keys {
                        assert_eq!(&measured_prefix, expected);
                    } else {
                        prefix_keys = Some(measured_prefix);
                    }
                    let measured = observed_reads.borrow().clone();
                    println!(
                        "both-current-three H2: {} target authentication reads",
                        measured.len()
                    );
                    assert_eq!(measured.len(), 15);
                    assert_eq!(
                        measured
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len(),
                        measured.len()
                    );
                    let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
                    for validator in [address(0x31), address(0x32)] {
                        assert!(measured.contains(&key(&[&[1], &validator, &[2]])));
                        assert!(!measured.contains(&key(&[&[1], &validator, &[1]])));
                        let owner = super::redelegate_both_current_three::owner(&simulation);
                        assert!(!measured.contains(&key(&[&[2, 0], &validator, &owner])));
                        assert!(!measured.contains(&key(&[&[0, 3], &validator])));
                    }
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
                        assert_eq!(
                            invocation.caller,
                            super::redelegate_both_current_three::caller(
                                &super::redelegate_both_current_three::corpus()
                            )
                        );
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
                    super::redelegate_both_current_three::assert_committed(&chain, &simulation);
                    assert_eq!(chain.block_header(1.into()).unwrap().unwrap(), owner_header);
                    assert_eq!(public_identity(&chain, &simulation), owner_identity);
                    assert_eq!(concrete_rows(&concrete), before);
                }
            }
            // Hard failures at the prefix or after its successful mutation dispose
            // the whole live port/journal. A fresh retry uses the committed H=1 seed.
            let case = public["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["name"] == "partial_prefix_then_full_both_current_three")
                .unwrap();
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            let mut plan = vec![(0_u64, None, false), (1, None, false)];
            for (sequence, keys) in [
                (0, prefix_keys.as_ref().unwrap()),
                (1, authenticated_keys.as_ref().unwrap()),
            ] {
                for index in 0..keys.len() {
                    for corrupt in [true, false] {
                        plan.push((sequence, Some(index), corrupt));
                    }
                }
            }
            for (failed_sequence, fault_index, corrupt) in plan {
                let target_reads = Rc::new(RefCell::new(Vec::new()));
                let prefix_reads = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let successful = Rc::new(Cell::new(0));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let run = |injected: bool| {
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
                                prefix_fail: if injected && failed_sequence == 0 {
                                    fault_index
                                } else {
                                    None
                                },
                                prefix_corrupt: corrupt,
                                prefix_reads: prefix_reads.clone(),
                                witness: public["live_witness"].clone(),
                                target_fail: if injected && failed_sequence == 1 {
                                    fault_index
                                } else {
                                    None
                                },
                                target_corrupt: corrupt,
                                target_reads: target_reads.clone(),
                                inner: TrackingPort {
                                    inner: mixed_native::MixedNativeExecutionPort::new(
                                        chain
                                            .begin_native_session(execution, selected.period)
                                            .unwrap(),
                                    ),
                                    seen: Rc::new(RefCell::new(Vec::new())),
                                    drops: drops.clone(),
                                    fail_at: if injected && fault_index.is_none() {
                                        Some(failed_sequence)
                                    } else {
                                        None
                                    },
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
                sessions += 1;
                let result = run(true);
                let stage = if failed_sequence == 0 {
                    TraceSequenceStage::Prefix
                } else {
                    TraceSequenceStage::Target
                };
                assert!(
                    matches!(&result, Err(StructuredTraceRunnerError::Execution{stage:actual,index:0,..}) if *actual==stage)
                );
                assert_eq!(drops.get(), 1);
                assert_eq!(successful.get(), failed_sequence as usize);
                assert_eq!(outcomes.borrow().len(), failed_sequence as usize);
                if failed_sequence == 1 {
                    assert_prefix(&outcomes.borrow()[0]);
                }
                if let Some(index) = fault_index {
                    assert!(format!("{result:?}").contains(if corrupt {
                        "RawIntegrity"
                    } else {
                        "injected trace authentication read"
                    }));
                    let (actual, expected) = if failed_sequence == 0 {
                        (&prefix_reads, prefix_keys.as_ref().unwrap())
                    } else {
                        (&target_reads, authenticated_keys.as_ref().unwrap())
                    };
                    assert_eq!(&*actual.borrow(), &expected[..=index]);
                } else {
                    assert!(format!("{result:?}").contains("injected after prefix"));
                }
                prefix_reads.borrow_mut().clear();
                target_reads.borrow_mut().clear();
                outcomes.borrow_mut().clear();
                super::redelegate_both_current_three::assert_committed(&chain, &simulation);
                assert_eq!(chain.block_header(1.into()).unwrap().unwrap(), owner_header);
                assert_eq!(public_identity(&chain, &simulation), owner_identity);
                assert_eq!(concrete_rows(&concrete), before);
                sessions += 1;
                let retry = run(false).unwrap();
                assert_eq!(retry.state, identity);
                assert_eq!(
                    serde_json::from_slice::<Value>(&retry.output).unwrap(),
                    case["result"]
                );
                assert_eq!(drops.get(), 2);
                assert_eq!(successful.get(), failed_sequence as usize + 2);
                assert_sequence(&outcomes.borrow());
                assert_eq!(&*prefix_reads.borrow(), prefix_keys.as_ref().unwrap());
                assert_eq!(
                    &*target_reads.borrow(),
                    authenticated_keys.as_ref().unwrap()
                );
                super::redelegate_both_current_three::assert_committed(&chain, &simulation);
                assert_eq!(chain.block_header(1.into()).unwrap().unwrap(), owner_header);
                assert_eq!(public_identity(&chain, &simulation), owner_identity);
                assert_eq!(concrete_rows(&concrete), before);
                failures += 1;
            }
            super::redelegate_both_current_three::assert_committed(&chain, &simulation);
            assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
    }
    println!("count3-H1 H2 {sessions} sessions/{failures} faults and fresh retries");
}
