//! Default structured tracing over complete retained P2, with a live native prefix.
//! Go observations are separate from JSON. Rust outcomes are internal witnesses.
use super::*;
use retained_commission::{committed, corpus, history};
use rustaxa_evm::{
    contracts::{
        NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
        NativeJournalAccount, NativeJournalRead, NativeJournalReadError,
        NativeOrdinaryAccountMutation, NativeOutcome, NativePortError, NativeRawOperation,
        NativeStatus,
    },
    driver::ExecutionDriverError,
    native::NativeAdapterError,
    trace_runner::{
        StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
    },
};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Observations {
    reads: Vec<ConcreteStorageKey>,
    seen: Vec<NativeInvocation>,
    outcomes: Vec<NativeInvocationResult>,
    drops: usize,
}

struct Port<'a> {
    inner: mixed_native::MixedNativeExecutionPort<'a>,
    observations: Rc<RefCell<Observations>>,
    calls: &'a [Value],
    identity: ConcreteStateIdentity,
    fault: u8,
}

impl Drop for Port<'_> {
    fn drop(&mut self) {
        self.observations.borrow_mut().drops += 1;
    }
}

struct FaultRead<'a> {
    inner: &'a dyn NativeJournalRead,
    observations: Rc<RefCell<Observations>>,
    fail: bool,
    identity: ConcreteStateIdentity,
}

impl NativeJournalRead for FaultRead<'_> {
    fn account(&self, a: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.inner.account(a)
    }
    fn raw_storage(
        &self,
        a: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        self.observations.borrow_mut().reads.push(*key);
        if self.fail {
            return Err(NativeJournalReadError::State(
                ConcreteReadError::HistoryUnavailable(self.identity),
            ));
        }
        self.inner.raw_storage(a, key)
    }
}

fn assert_outcome(out: &NativeOutcome, call: &Value, index: usize) {
    let error = match &out.status {
        NativeStatus::Success => "",
        NativeStatus::ContractFailure(error) => &error.error,
    };
    assert_eq!(error, call["output"]["execution_error"].as_str().unwrap());
    assert_eq!(out.gas_used.as_u64(), 20000);
    assert_eq!(out.output, bytes(&call["output"]["return"]));
    let raw = out.raw_mutations.iter().map(|w| json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation {NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
    assert_eq!(raw, *call["raw"].as_array().unwrap());
    for mutation in &out.raw_mutations {
        assert_eq!(
            mutation.expected,
            ConcreteRead::Present(bytes(&call["before"]["raw"][hex::encode(mutation.key.0)]))
        );
    }
    let logs = out.logs.iter().map(|l| json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
    // Actual Go returns cumulative journal logs. The native outcome is per call.
    let cumulative = call["output"]["logs"].as_array().unwrap();
    assert_eq!(logs, cumulative[index..]);
    let effects = call["balances"].as_array().unwrap();
    // The accepted serializer may request zero balance operations, while Go's
    // Add/Sub observer is below its zero early return. Compare those separately.
    if !error.is_empty() {
        assert!(out.account_mutations.is_empty());
    } else {
        assert_eq!(out.account_mutations.len(), 2);
        if index == 1 {
            assert!(effects.is_empty());
            assert_eq!(
                out.account_mutations,
                vec![
                    NativeOrdinaryAccountMutation::EnsureExists {
                        address: address(0xfe),
                        expected_exists: true
                    },
                    NativeOrdinaryAccountMutation::Touch {
                        address: bytes(&call["request"]["sender"]).try_into().unwrap(),
                        expected_exists: true
                    },
                ]
            );
            for (key, before) in call["before"]["accounts"].as_object().unwrap() {
                assert_eq!(before["balance"], call["after"]["accounts"][key]["balance"]);
                assert_eq!(before["exists"], call["after"]["accounts"][key]["exists"]);
            }
            return;
        }
        let mut observed = Vec::new();
        for mutation in &out.account_mutations {
            let NativeOrdinaryAccountMutation::Balance {
                address,
                expected_exists,
                expected,
                replacement,
            } = mutation
            else {
                panic!("commission balance effect")
            };
            let key = hex::encode(address);
            assert_eq!(
                *expected_exists,
                call["before"]["accounts"][&key]["exists"]
                    .as_bool()
                    .unwrap()
            );
            assert_eq!(
                expected.value().to_string(),
                call["before"]["accounts"][&key]["balance"]
                    .as_str()
                    .unwrap()
            );
            assert_eq!(
                replacement.value().to_string(),
                call["after"]["accounts"][&key]["balance"].as_str().unwrap()
            );
            let delta = replacement.value() - expected.value();
            if delta != num_bigint::BigInt::from(0) {
                let (op, amount) = if delta < num_bigint::BigInt::from(0) {
                    ("sub", -delta)
                } else {
                    ("add", delta)
                };
                observed.push(json!({"address":key,"operation":op,"amount":amount.to_string(),"balance":replacement.value().to_string()}));
            }
        }
        assert_eq!(&observed, effects);
    }
}

impl NativeExecutionPort for Port<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let index = usize::try_from(invocation.id.sequence).unwrap();
        let call = &self.calls[index];
        let mut observations = self.observations.borrow_mut();
        assert_eq!(observations.outcomes.len(), index);
        // Before target execution, prove the prefix completed and applied its
        // actual positive claim to the shared journal (pool2 -> pool0).
        for (key, value) in call["before"]["raw"].as_object().unwrap() {
            let read = journal
                .raw_storage(
                    address(0xfe),
                    &ConcreteStorageKey(bytes(&json!(key)).try_into().unwrap()),
                )
                .unwrap();
            assert_eq!(read, ConcreteRead::Present(bytes(value)));
        }
        for (key, expected) in call["before"]["accounts"].as_object().unwrap() {
            let a = bytes(&json!(key)).try_into().unwrap();
            let actual = journal.account(a).unwrap();
            assert_eq!(actual.exists, expected["exists"].as_bool().unwrap());
            assert_eq!(
                actual.balance.value().to_string(),
                expected["balance"].as_str().unwrap()
            );
            // Both engines have incremented the sender nonce before prepare.
            let expected_nonce = if a == invocation.caller {
                &call["after"]["accounts"][key]["nonce"]
            } else {
                &expected["nonce"]
            };
            assert_eq!(
                BigUint::from_bytes_be(&actual.nonce.to_bytes()),
                number(expected_nonce)
            );
        }
        assert_eq!(invocation.period, 3.into());
        assert_eq!(invocation.depth, 0);
        assert_eq!(
            invocation.caller.as_slice(),
            bytes(&call["request"]["sender"])
        );
        assert_eq!(invocation.input, bytes(&call["request"]["input"]));
        assert_eq!(invocation.value.value(), &BigUint::default());
        assert_eq!(invocation.supplied_gas.as_u64(), 178536);
        assert_eq!(
            invocation.kind,
            rustaxa_evm::contracts::NativeCallKind::Call
        );
        assert!(!invocation.is_static);
        observations.seen.push(invocation.clone());
        drop(observations);
        if self.fault == 2 && index == 1 {
            return Err(NativePortError::Infrastructure(
                "retained trace target prepare".into(),
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
        let index = usize::try_from(invocation.id.sequence).unwrap();
        let result = self.inner.invoke(
            invocation,
            quote,
            &FaultRead {
                inner: journal,
                observations: self.observations.clone(),
                fail: self.fault == 1 && index == 1,
                identity: self.identity,
            },
        )?;
        let NativeInvocationResult::Completed(out) = &result else {
            panic!("completed native call")
        };
        assert_outcome(out, &self.calls[index], index);
        self.observations.borrow_mut().outcomes.push(result.clone());
        Ok(result)
    }
}

#[test]
fn retained_commission_trace_go_sequences_faults_retries_and_reopen() {
    let public: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_retained_commission_trace/public.json")).unwrap();
    let local: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_retained_commission_trace/local.json")).unwrap();
    assert_eq!(public, local);
    let seed = corpus();
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_before"], public["state_after"]);
    let concrete = FixturePath::new("retained-trace-concrete");
    let identity = materialize(&public, &concrete);
    let rows_before = concrete_rows(&concrete);
    let app = FixturePath::new("retained-trace-owner");
    let mut original = None;
    let (mut attempts, mut sessions, mut successes, mut failures, mut calls) = (0, 0, 0, 0, 0);
    for restart in 0..2 {
        let chain = history(&app, restart == 0, &seed);
        let owner_before = committed(&chain, &seed);
        if let Some(before) = &original {
            assert_eq!(&owner_before, before);
        } else {
            original = Some(owner_before.clone());
        }
        for case in public["cases"].as_array().unwrap() {
            let prefix = metadata_trace::requests(&case["prefix"], 0);
            let targets = metadata_trace::requests(&case["targets"], prefix.len());
            let nonces = prefix
                .iter()
                .chain(&targets)
                .map(|t| t.nonce.clone())
                .collect::<Vec<_>>();
            // Every injected failure has an immediate complete fresh retry.
            let roster = if prefix.is_empty() {
                vec![0, 0]
            } else {
                vec![0, 0, 1, 0, 2, 0, 3, 0, 4, 0]
            };
            for fault in roster {
                attempts += 1;
                let reader = CompleteSeedReader::open(&concrete, &public, identity);
                assert_eq!(reader.identity(), identity);
                let observations = Rc::new(RefCell::new(Observations::default()));
                let mut unavailable = identity;
                unavailable.period = 1.into();
                let mut mismatched = identity;
                mismatched.state_root[0] ^= 1;
                let result = run_structured_trace_with_native(
                    &reader,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |selected, execution| {
                        assert_eq!(selected, identity);
                        assert_eq!(execution, 3.into());
                        if fault == 3 || fault == 4 {
                            let requested = if fault == 3 { unavailable } else { mismatched };
                            let readers = ConcreteCheckpointReaders::open_read_only(
                                &concrete.0,
                                selected,
                                [selected],
                            )
                            .unwrap();
                            let error = ConcreteCheckpointNativeStateRead::new(&readers, requested)
                                .unwrap_err();
                            let rustaxa_consensus::native_session::FinalChainNativeStateReadError::State(error) = error else { panic!("historical reader error") };
                            assert_eq!(error, ConcreteReadError::HistoryUnavailable(requested));
                            return Err(NativePortError::Journal(NativeJournalReadError::State(
                                error,
                            )));
                        }
                        sessions += 1;
                        Ok(Port {
                            inner: mixed_native::MixedNativeExecutionPort::new(
                                chain
                                    .begin_native_session(execution, selected.period)
                                    .unwrap(),
                            ),
                            observations: observations.clone(),
                            calls: case["observation_witness"].as_array().unwrap(),
                            identity,
                            fault,
                        })
                    },
                    &ExecutionBlockContext {
                        period: 3.into(),
                        author: address(0x31),
                        timestamp: 1700000003,
                        gas_limit: 1000000.into(),
                        chain_id: 666,
                        difficulty: BigUint::default(),
                    },
                    &prefix,
                    &targets,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                assert_eq!(
                    prefix
                        .iter()
                        .chain(&targets)
                        .map(|t| t.nonce.clone())
                        .collect::<Vec<_>>(),
                    nonces
                );
                assert_eq!(committed(&chain, &seed), owner_before);
                drop(reader);
                assert_eq!(concrete_rows(&concrete), rows_before);
                let reopened = CompleteSeedReader::open(&concrete, &public, identity);
                assert_eq!(reopened.identity(), identity);
                let observed = observations.borrow();
                assert_eq!(observed.drops, usize::from(fault < 3));
                calls += observed.seen.len();
                if fault != 0 {
                    failures += 1;
                    let port_error = match fault {
                        1 => NativePortError::Journal(NativeJournalReadError::State(
                            ConcreteReadError::HistoryUnavailable(identity),
                        )),
                        2 => {
                            NativePortError::Infrastructure("retained trace target prepare".into())
                        }
                        3 | 4 => NativePortError::Journal(NativeJournalReadError::State(
                            ConcreteReadError::HistoryUnavailable(if fault == 3 {
                                unavailable
                            } else {
                                mismatched
                            }),
                        )),
                        _ => unreachable!(),
                    };
                    let expected = if fault < 3 {
                        assert_eq!(observed.outcomes.len(), 1);
                        assert_eq!(observed.seen.len(), 2);
                        assert_eq!(observed.reads.len(), if fault == 1 { 2 } else { 1 });
                        StructuredTraceRunnerError::Execution {
                            stage: TraceSequenceStage::Target,
                            index: 0,
                            error: ExecutionDriverError::Native(NativeAdapterError::Port(
                                port_error,
                            )),
                        }
                    } else {
                        assert!(
                            observed.outcomes.is_empty()
                                && observed.seen.is_empty()
                                && observed.reads.is_empty()
                        );
                        StructuredTraceRunnerError::NativeSession(port_error)
                    };
                    assert_eq!(result, Err(expected));
                } else {
                    successes += 1;
                    let result = result.unwrap();
                    assert_eq!(result.state, identity);
                    assert_eq!(
                        serde_json::from_slice::<Value>(&result.output).unwrap(),
                        case["result"]
                    );
                    assert_eq!(observed.outcomes.len(), prefix.len() + targets.len());
                    assert_eq!(observed.seen.len(), observed.outcomes.len());
                    for (index, (invocation, tx)) in observed
                        .seen
                        .iter()
                        .zip(prefix.iter().chain(&targets))
                        .enumerate()
                    {
                        assert_eq!(invocation.id.sequence, index as u64);
                        assert_eq!(invocation.id.transaction, tx.position);
                    }
                    assert_eq!(
                        observed.reads.len(),
                        if case["name"] == "wrong_owner" {
                            0
                        } else {
                            prefix.len() + 1
                        }
                    );
                }
                for key in &observed.reads {
                    assert_eq!(
                        hex::encode(key.0),
                        seed["cases"][1]["ordered_raw_writes"][0]["key"]
                    );
                }
            }
        }
    }
    let chain = history(&app, false, &seed);
    assert_eq!(committed(&chain, &seed), original.unwrap());
    assert_eq!(concrete_rows(&concrete), rows_before);
    assert!(attempts > 0 && sessions > 0 && successes > 0 && failures > 0 && calls > 0);
    assert_eq!(attempts, successes + failures);
    println!(
        "retained_commission_trace attempts={attempts} sessions={sessions} successes={successes} failures={failures} native_prepares={calls} semantic_owner_starts=2"
    );
}
