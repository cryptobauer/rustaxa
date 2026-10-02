//! Actual direct-native default structured TraceRunner composition and disposal.

use super::*;
use rustaxa_evm::{
    contracts::{
        NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
        NativeJournalRead, NativePortError,
    },
    trace_runner::{
        StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct TrackingPort<'a> {
    pub(super) inner: mixed_native::MixedNativeExecutionPort<'a>,
    pub(super) seen: Rc<RefCell<Vec<NativeInvocation>>>,
    pub(super) drops: Rc<Cell<usize>>,
    pub(super) fail_at: Option<u64>,
}

impl Drop for TrackingPort<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

impl NativeExecutionPort for TrackingPort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.seen.borrow_mut().push(invocation.clone());
        if Some(invocation.id.sequence) == self.fail_at {
            return Err(NativePortError::Infrastructure(
                "injected after prefix".into(),
            ));
        }
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.inner.invoke(invocation, quote, journal)
    }
}

pub(super) fn block(period: u64) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: period.into(),
        author: address(0x31),
        timestamp: 1_700_000_002,
        gas_limit: 500_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}

pub(super) fn requests(rows: &Value, offset: usize) -> Vec<ExecutionTransaction> {
    rows.as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, row)| ExecutionTransaction {
            position: u32::try_from(offset + index).unwrap().into(),
            hash: [0; 32],
            sender: bytes(&row["sender"]).try_into().unwrap(),
            receiver: Some(bytes(&row["to"]).try_into().unwrap()),
            nonce: if number(&row["nonce"]) == BigUint::default() {
                FinalChainNonce::zero()
            } else {
                FinalChainNonce::from_bytes(&number(&row["nonce"]).to_bytes_be()).unwrap()
            },
            gas_price: ExecutionGasPrice::new(number(&row["gas_price"])),
            gas_limit: row["gas"].as_u64().unwrap().into(),
            value: ExecutionValue::new(number(&row["value"])),
            input: bytes(&row["input"]),
            canonical_rlp: None,
            kind: ExecutionTransactionKind::Call,
        })
        .collect()
}

#[test]
fn persisted_metadata_traces_match_actual_go_live_sequence_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_metadata_trace/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_metadata_trace/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 7);
    let concrete = FixturePath::new("metadata-trace-concrete");
    let app = FixturePath::new("metadata-trace-app");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let chain = native_history(&app);
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

struct NoRead;
impl ConcreteStateRead for NoRead {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: 1.into(),
            state_root: [9; 32],
        }
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        panic!("no account access")
    }
    fn storage(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        panic!("no storage access")
    }
    fn code(&self, _: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        panic!("no code access")
    }
}

struct NeverPort;
impl NativeExecutionPort for NeverPort {
    fn prepare(
        &mut self,
        _: &NativeInvocation,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        panic!("no prepare")
    }
    fn invoke(
        &mut self,
        _: &NativeInvocation,
        _: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        panic!("no invoke")
    }
}

#[test]
fn native_trace_mismatch_precedes_factory_and_factory_failure_precedes_reads() {
    let mismatch = run_structured_trace_with_native(
        &NoRead,
        &NoHistory,
        &Dpos,
        &Dpos,
        |_, _| -> Result<NeverPort, NativePortError> { panic!("no factory on mismatch") },
        &block(3),
        &[],
        &[],
        EnvelopeRules { cornus: true },
        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
    );
    assert!(
        matches!(mismatch,Err(StructuredTraceRunnerError::StatePeriodMismatch {expected,..}) if expected==2.into())
    );
    let error = NativePortError::Infrastructure("factory unavailable".into());
    let failed = run_structured_trace_with_native(
        &NoRead,
        &NoHistory,
        &Dpos,
        &Dpos,
        |selected, execution| -> Result<NeverPort, NativePortError> {
            assert_eq!(selected, NoRead.identity());
            assert_eq!(execution, 2.into());
            Err(error.clone())
        },
        &block(2),
        &[],
        &[],
        EnvelopeRules { cornus: true },
        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
    );
    assert_eq!(
        failed,
        Err(StructuredTraceRunnerError::NativeSession(error))
    );
}
