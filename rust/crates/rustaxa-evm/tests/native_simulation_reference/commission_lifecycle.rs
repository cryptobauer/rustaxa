//! Same qualified lifecycle seeds through call, estimation and default trace APIs.
use super::*;
use rustaxa_consensus as lifecycle_consensus;
#[path = "../../../rustaxa-consensus/src/final_chain/native_session/tests/commission_lifecycle_fault_support.rs"]
mod faults;
#[path = "../../../rustaxa-consensus/src/final_chain/native_session/tests/commission_lifecycle_support.rs"]
mod support;
use rustaxa_evm::{
    contracts::{
        NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
        NativeJournalAccount, NativeJournalRead, NativeJournalReadError,
        NativeOrdinaryAccountMutation, NativeOutcome, NativePortError, NativeRawOperation,
        NativeStatus,
    },
    estimate::{EstimateError, EstimateProbe, estimate_gas},
    trace_runner::{
        StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
    },
};
use rustaxa_evm::{
    driver::ExecutionDriverError, native::NativeAdapterError, simulation::SimulationError,
};
use std::{cell::RefCell, rc::Rc};
fn lifecycle_keccak(data: &[u8]) -> [u8; 32] {
    keccak256(data).0
}
#[derive(Default)]
struct Seen {
    reads: Vec<(u64, ConcreteStorageKey)>,
    outcomes: Vec<NativeInvocationResult>,
    drops: usize,
}
type SeenRef = Rc<RefCell<Seen>>;
type CallResult = Result<rustaxa_evm::simulation::SimulationResult, SimulationError>;
type TraceResult =
    Result<rustaxa_evm::trace_runner::StructuredTraceRun, StructuredTraceRunnerError>;
enum Inner<'a> {
    Call(Box<mixed_native::SimulationNativeExecutionPort<'a>>),
    Trace(Box<mixed_native::MixedNativeExecutionPort<'a>>),
}
struct Port<'a> {
    inner: Inner<'a>,
    seen: SeenRef,
    fault: Option<(usize, bool)>,
    identity: ConcreteStateIdentity,
}
impl Drop for Port<'_> {
    fn drop(&mut self) {
        self.seen.borrow_mut().drops += 1;
    }
}
struct Journal<'a> {
    inner: &'a dyn NativeJournalRead,
    seen: SeenRef,
    fault: Option<(usize, bool)>,
    sequence: u64,
    identity: ConcreteStateIdentity,
}
impl NativeJournalRead for Journal<'_> {
    fn account(&self, a: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.inner.account(a)
    }
    fn raw_storage(
        &self,
        a: [u8; 20],
        k: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        let index = self.seen.borrow().reads.len();
        self.seen.borrow_mut().reads.push((self.sequence, *k));
        if let Some((at, malformed)) = self.fault
            && at == index
        {
            return if malformed {
                Ok(ConcreteRead::Present(vec![0xff]))
            } else {
                Err(NativeJournalReadError::State(
                    ConcreteReadError::HistoryUnavailable(self.identity),
                ))
            };
        }
        self.inner.raw_storage(a, k)
    }
}
impl NativeExecutionPort for Port<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        match &mut self.inner {
            Inner::Call(p) => p.prepare(i, j),
            Inner::Trace(p) => p.prepare(i, j),
        }
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        let proxy = Journal {
            inner: j,
            seen: self.seen.clone(),
            fault: self.fault,
            sequence: i.id.sequence,
            identity: self.identity,
        };
        let out = match &mut self.inner {
            Inner::Call(p) => p.invoke(i, q, &proxy),
            Inner::Trace(p) => p.invoke(i, q, &proxy),
        }?;
        self.seen.borrow_mut().outcomes.push(out.clone());
        Ok(out)
    }
}
fn block(period: u64) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: period.into(),
        author: address(0x31),
        timestamp: 1700000000 + period,
        gas_limit: 1000000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}
fn request(row: &Value, index: usize) -> ExecutionTransaction {
    ExecutionTransaction {
        position: (index as u32).into(),
        hash: [0; 32],
        sender: bytes(&row["sender"]).try_into().unwrap(),
        receiver: Some(bytes(&row["to"]).try_into().unwrap()),
        nonce: FinalChainNonce::from_bytes(
            &number(row.get("supplied_nonce").unwrap_or(&row["nonce"])).to_bytes_be(),
        )
        .unwrap(),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: row["gas"].as_u64().unwrap().into(),
        value: ExecutionValue::default(),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}
fn port_fault(
    row: &Value,
    key: &ConcreteStorageKey,
    malformed: bool,
    identity: ConcreteStateIdentity,
) -> NativePortError {
    if malformed {
        NativePortError::Domain(
            rustaxa_consensus::native_session::FinalChainNativeSessionError::RawIntegrity(
                faults::malformed_raw_error(row, &key.0),
            )
            .to_string(),
        )
    } else {
        NativePortError::Journal(NativeJournalReadError::State(
            ConcreteReadError::HistoryUnavailable(identity),
        ))
    }
}
fn call_fault(
    row: &Value,
    key: &ConcreteStorageKey,
    malformed: bool,
    identity: ConcreteStateIdentity,
) -> SimulationError {
    SimulationError::Execution(ExecutionDriverError::Native(NativeAdapterError::Port(
        port_fault(row, key, malformed, identity),
    )))
}
fn compare_execution(result: &TransactionExecutionResult, row: &Value) {
    let TransactionExecutionResult::Executed(out) = result else {
        panic!("executed lifecycle")
    };
    let err = match &out.status {
        CodeExecutionStatus::Success => "",
        CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) => e.error.as_str(),
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas",
        s => panic!("{s:?}"),
    };
    assert_eq!(err, row["execution_error"].as_str().unwrap());
    assert_eq!(row["consensus_error"], "");
    assert_eq!(out.gas_used.as_u64(), row["gas_used"].as_u64().unwrap());
    assert_eq!(out.output, bytes(&row["output"]));
    let logs=out.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
    assert_eq!(logs, *row["logs"].as_array().unwrap());
}
fn compare_native(out: &NativeOutcome, row: &Value, prior_logs: usize) {
    let err = match &out.status {
        NativeStatus::Success => "",
        NativeStatus::ContractFailure(e) => e.error.as_str(),
    };
    assert_eq!(err, row["execution_error"].as_str().unwrap());
    assert_eq!(out.output, bytes(&row["output"]));
    let raw=out.raw_mutations.iter().map(|w|json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
    assert_eq!(raw, *row["ordered_raw_writes"].as_array().unwrap());
    let logs=out.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
    assert_eq!(logs, row["logs"].as_array().unwrap()[prior_logs..]);
    let effects=out.account_mutations.iter().filter_map(|m| match m {
      NativeOrdinaryAccountMutation::Balance {address,expected_exists,expected,replacement} => {assert!(*expected_exists);let delta=replacement.value()-expected.value();assert_ne!(delta,num_bigint::BigInt::default());Some(json!({"address":hex::encode(address),"operation":if delta<num_bigint::BigInt::default(){"sub"}else{"add"},"amount":if delta<num_bigint::BigInt::default(){(-delta).to_string()}else{delta.to_string()},"balance":replacement.value().to_string()}))},
      NativeOrdinaryAccountMutation::EnsureExists{expected_exists,..}|NativeOrdinaryAccountMutation::Touch{expected_exists,..}=>{assert!(*expected_exists);None},
 NativeOrdinaryAccountMutation::Nonce{..}=>panic!("native lifecycle must not emit an account nonce mutation")
    }).collect::<Vec<_>>();
    assert_eq!(effects, *row["ordered_balance_effects"].as_array().unwrap());
}
fn committed(chain: &FinalChain, period: u64, snapshot: &Value) -> Value {
    let accounts = snapshot["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let a: [u8; 20] = bytes(&row["address"]).try_into().unwrap();
            (
                row["address"].clone(),
                format!("{:?}", chain.account(a).unwrap()),
            )
        })
        .collect::<Vec<_>>();
    json!({"header":hex::encode(chain.block_header(period.into()).unwrap().unwrap()),"accounts":accounts,"validators":format!("{:?}",chain.dpos_validators_total_stakes((period+1).into()).unwrap())})
}
#[derive(Clone, Copy)]
struct Attempt<'a> {
    chain: &'a FinalChain,
    path: &'a FixturePath,
    fixture: &'a Value,
    identity: ConcreteStateIdentity,
    selection: Option<ConcreteStateIdentity>,
    storage: &'a Storage,
    owner_rows: &'a Value,
    concrete_rows: &'a Vec<(String, Vec<u8>, Vec<u8>)>,
}
fn owner_rows(storage: &Storage) -> Value {
    Value::Array(
        Column::all()
            .iter()
            .map(|col| {
                let rows = storage
                    .iter(*col)
                    .map(|r| {
                        let (k, v) = r.unwrap();
                        json!([hex::encode(k), hex::encode(v)])
                    })
                    .collect::<Vec<_>>();
                json!({"column":col.name(),"rows":rows})
            })
            .collect(),
    )
}

impl Attempt<'_> {
    fn search(
        &self,
        estimate: &Value,
        probes: &mut usize,
    ) -> (Result<u64, EstimateError<SimulationError>>, usize) {
        let rows = estimate["probes"].as_array().unwrap();
        let mut index = 0;
        let result = estimate_gas(estimate["cap"].as_u64().unwrap(), |gas| {
            let row = &rows[index];
            index += 1;
            *probes += 1;
            assert_eq!(gas, row["gas"].as_u64().unwrap());
            let (result, seen) = self.call(row, None);
            let result = result?;
            compare_execution(&result.execution, row);
            for out in &seen.borrow().outcomes {
                if let NativeInvocationResult::Completed(out) = out {
                    compare_native(out, row, 0)
                }
            }
            let TransactionExecutionResult::Executed(execution) = result.execution else {
                panic!("probe executed")
            };
            Ok::<_, SimulationError>(if execution.status == CodeExecutionStatus::Success {
                EstimateProbe::Success {
                    gas_used: execution.gas_used.as_u64(),
                }
            } else {
                EstimateProbe::CodeFailure {
                    error: row["execution_error"].as_str().unwrap().into(),
                }
            })
        });
        assert_eq!(index, rows.len());
        (result, index)
    }
    fn preserve(&self) {
        assert_eq!(&owner_rows(self.storage), self.owner_rows);
        assert_eq!(&concrete_rows(self.path), self.concrete_rows);
    }
    fn selection_error(&self, selected: ConcreteStateIdentity) -> Result<(), NativePortError> {
        if let Some(requested) = self.selection {
            let readers =
                ConcreteCheckpointReaders::open_read_only(&self.path.0, selected, [selected])
                    .unwrap();
            let error = ConcreteCheckpointNativeStateRead::new(&readers, requested).unwrap_err();
            let rustaxa_consensus::native_session::FinalChainNativeStateReadError::State(error) =
                error
            else {
                panic!("historical selection error")
            };
            assert_eq!(error, ConcreteReadError::HistoryUnavailable(requested));
            return Err(NativePortError::Journal(NativeJournalReadError::State(
                error,
            )));
        }
        Ok(())
    }

    fn call(&self, row: &Value, fault: Option<(usize, bool)>) -> (CallResult, SeenRef) {
        let reader = CompleteSeedReader::open(self.path, self.fixture, self.identity);
        let seen = Rc::new(RefCell::new(Seen::default()));
        let req = request(row, 0);
        let original = req.clone();
        let result = simulate_with_native(
            &reader,
            &NoHistory,
            &Dpos,
            &Dpos,
            |selected| {
                assert_eq!(selected, self.identity);
                self.selection_error(selected)?;
                Ok(Port {
                    inner: Inner::Call(Box::new(mixed_native::SimulationNativeExecutionPort::new(
                        self.chain.begin_native_simulation(selected.period).unwrap(),
                    ))),
                    seen: seen.clone(),
                    fault,
                    identity: selected,
                })
            },
            &block(self.identity.period.as_u64()),
            &req,
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(TaraxaPhase::Ficus),
        );
        assert_eq!(req, original);
        assert_eq!(seen.borrow().drops, usize::from(self.selection.is_none()));
        self.preserve();
        (result, seen)
    }
    fn trace(
        &self,
        trace: &Value,
        sequence: bool,
        fault: Option<(usize, bool)>,
    ) -> (TraceResult, SeenRef) {
        let rows = trace["requests"].as_array().unwrap();
        let requests = rows
            .iter()
            .enumerate()
            .map(|(i, r)| request(r, i))
            .collect::<Vec<_>>();
        let split = usize::from(sequence);
        let reader = CompleteSeedReader::open(self.path, self.fixture, self.identity);
        let seen = Rc::new(RefCell::new(Seen::default()));
        let result = run_structured_trace_with_native(
            &reader,
            &NoHistory,
            &Dpos,
            &Dpos,
            |selected, execution| {
                self.selection_error(selected)?;
                Ok(Port {
                    inner: Inner::Trace(Box::new(mixed_native::MixedNativeExecutionPort::new(
                        self.chain
                            .begin_native_session(execution, selected.period)
                            .unwrap(),
                    ))),
                    seen: seen.clone(),
                    fault,
                    identity: selected,
                })
            },
            &block(self.identity.period.as_u64() + 1),
            &requests[..split],
            &requests[split..],
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(TaraxaPhase::Ficus),
        );
        assert_eq!(seen.borrow().drops, usize::from(self.selection.is_none()));
        self.preserve();
        (result, seen)
    }
}
fn fixture_path(label: &str) -> FixturePath {
    let path = PathBuf::from(std::env::var_os("RUN_ARTIFACT_DIR").unwrap()).join(format!(
        "commission-lifecycle-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    crate::closeout_test_scratch::register(&path);
    std::fs::create_dir_all(&path).unwrap();
    crate::closeout_test_scratch::created(&path);
    FixturePath(path)
}
fn expected_raw(
    fixture: &Value,
    key: &str,
    identity: ConcreteStateIdentity,
) -> ConcreteRead<Vec<u8>> {
    if let Some(value) = fixture["state_before"]["native_raw"].get(key) {
        return ConcreteRead::Present(bytes(value));
    }
    let logical = ConcreteStorageKey(hex::decode(key).unwrap().try_into().unwrap());
    let prefix = rustaxa_storage::storage_version_prefix(address(0xfe), logical);
    let latest = fixture["state_before"]["seed_rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| {
            if row["column"] != 4 {
                return None;
            }
            let physical = bytes(&row["key"]);
            if physical.len() != 40 || physical[..32] != prefix {
                return None;
            }
            let period = u64::from_be_bytes(physical[32..].try_into().unwrap());
            if period > identity.period.as_u64() {
                return None;
            }
            Some((period, bytes(&row["value"])))
        })
        .max_by_key(|(period, _)| *period);
    if let Some((_, value)) = latest {
        assert!(
            value.is_empty(),
            "unobserved native row has nonempty physical bytes"
        );
        ConcreteRead::Tombstone
    } else {
        ConcreteRead::Absent
    }
}
#[test]
fn commission_lifecycle_calls_estimates_traces_faults_reopen_and_preservation() {
    let corpus = support::corpus();
    let cpp: Value = serde_json::from_slice(
        &std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../experiments/evm_feasibility/fixtures/native_commission_lifecycle/cpp.json",
        ))
        .unwrap(),
    )
    .unwrap();
    let mut cpp_index = 0;
    let (mut calls, mut probes, mut traces, mut faults) = (0, 0, 0, 0);
    for s in corpus["scenarios"].as_array().unwrap() {
        let app = fixture_path("owner");
        for row in s["cases"].as_array().unwrap() {
            let fixture = json!({"state_before":support::expand(&corpus,&row["state_before"])});
            let concrete = fixture_path("concrete");
            let identity = materialize(&fixture, &concrete);
            let baseline = concrete_rows(&concrete);
            let reader = CompleteSeedReader::open(&concrete, &fixture, identity);
            for key in corpus["raw_key_inventory"].as_array().unwrap() {
                let value = reader
                    .storage(
                        address(0xfe),
                        ConcreteStorageKey(bytes(key).try_into().unwrap()),
                    )
                    .unwrap();
                let expected = expected_raw(&fixture, key.as_str().unwrap(), identity);
                assert_eq!(value, expected);
            }
            drop(reader);

            let mut owner_original = None;
            for restart in 0..2 {
                let (chain, storage) =
                    support::history(&app.0, restart == 0, s, identity.period.as_u64());
                let owner_before =
                    committed(&chain, identity.period.as_u64(), &fixture["state_before"]);
                assert_eq!(
                    owner_original.get_or_insert(owner_before.clone()),
                    &owner_before
                );
                let raw_owner = owner_rows(&storage);
                let attempt = Attempt {
                    chain: &chain,
                    path: &concrete,
                    fixture: &fixture,
                    identity,
                    selection: None,
                    storage: &storage,
                    owner_rows: &raw_owner,
                    concrete_rows: &baseline,
                };
                let call = &row["consumers"]["call"];
                let (result, seen) = attempt.call(call, None);
                let result = result.unwrap();
                assert_eq!(result.state, identity);
                compare_execution(&result.execution, call);
                calls += 1;
                let consumed = seen.borrow().reads.clone();
                for out in &seen.borrow().outcomes {
                    if let NativeInvocationResult::Completed(out) = out {
                        compare_native(out, call, 0)
                    }
                }
                if restart == 0 {
                    for (index, (_, key)) in consumed.iter().enumerate() {
                        for malformed in [false, true] {
                            let (result, seen) = attempt.call(call, Some((index, malformed)));
                            let error = result.unwrap_err();
                            assert_eq!(error, call_fault(call, key, malformed, identity));
                            assert_eq!(seen.borrow().reads.len(), index + 1);
                            assert!(seen.borrow().outcomes.is_empty());
                            faults += 1;
                        }
                        let (result, _) = attempt.call(call, None);
                        compare_execution(&result.unwrap().execution, call);
                        calls += 1;
                    }
                }
                if restart == 0 {
                    for root_mismatch in [false, true] {
                        let mut requested = identity;
                        if root_mismatch {
                            requested.state_root[0] ^= 1
                        } else {
                            requested.period = (identity.period.as_u64() + 100).into()
                        };
                        let unavailable = Attempt {
                            selection: Some(requested),
                            ..attempt
                        };
                        let expected = NativePortError::Journal(NativeJournalReadError::State(
                            ConcreteReadError::HistoryUnavailable(requested),
                        ));
                        let (result, seen) = unavailable.call(call, None);
                        assert_eq!(
                            result,
                            Err(SimulationError::NativeSession(expected.clone()))
                        );
                        assert!(
                            seen.borrow().outcomes.is_empty() && seen.borrow().reads.is_empty()
                        );
                        faults += 1;
                        let mut count = 0;
                        let result = estimate_gas(200000, |_| {
                            count += 1;
                            let (result, _) = unavailable.call(call, None);
                            result.map(|_| EstimateProbe::Success { gas_used: 0 })
                        });
                        assert_eq!(count, 1);
                        assert_eq!(
                            result,
                            Err(EstimateError::Callback(SimulationError::NativeSession(
                                expected.clone()
                            )))
                        );
                        faults += 1;
                        let (result, seen) =
                            unavailable.trace(&row["consumers"]["trace_sequence"], true, None);
                        assert_eq!(
                            result,
                            Err(StructuredTraceRunnerError::NativeSession(expected))
                        );
                        assert!(
                            seen.borrow().outcomes.is_empty() && seen.borrow().reads.is_empty()
                        );
                        faults += 1;
                    }
                }
                for (offset, estimate) in row["consumers"]["estimates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    let expected = &cpp["cases"][cpp_index + offset];
                    let rows = estimate["probes"].as_array().unwrap();
                    let (result, index) = attempt.search(estimate, &mut probes);
                    let ordinary_result = result.clone();
                    if restart == 0 && offset == 0 {
                        for (read_index, (_, key)) in consumed.iter().enumerate() {
                            for malformed in [false, true] {
                                let mut count = 0;
                                let result =
                                    estimate_gas(estimate["cap"].as_u64().unwrap(), |_| {
                                        count += 1;
                                        let (result, _) =
                                            attempt.call(&rows[0], Some((read_index, malformed)));
                                        result.map(|_| EstimateProbe::Success { gas_used: 0 })
                                    });
                                assert_eq!(count, 1);
                                assert_eq!(
                                    result,
                                    Err(EstimateError::Callback(call_fault(
                                        &rows[0], key, malformed, identity
                                    )))
                                );
                                faults += 1;
                                let (retry, consumed) = attempt.search(estimate, &mut probes);
                                assert_eq!(retry, ordinary_result);
                                assert_eq!(consumed, rows.len());
                            }
                        }
                    }
                    assert_eq!(index, rows.len());
                    assert_eq!(index as u64, expected["consumed"].as_u64().unwrap());
                    if let Some(value) = expected["result"].as_u64() {
                        assert_eq!(result, Ok(value))
                    } else {
                        assert_eq!(
                            result,
                            Err(EstimateError::CodeFailure {
                                error: expected["error"].as_str().unwrap().into()
                            })
                        )
                    }
                }
                for sequence in [false, true] {
                    let trace =
                        &row["consumers"][if sequence { "trace_sequence" } else { "trace" }];
                    let (result, seen) = attempt.trace(trace, sequence, None);
                    let result = result.unwrap();
                    assert_eq!(result.state, identity);
                    assert_eq!(
                        serde_json::from_slice::<Value>(&result.output).unwrap(),
                        trace["result"]
                    );
                    let observations = trace["observation_witness"].as_array().unwrap();
                    let mut prior = 0;
                    for (actual, witness) in seen.borrow().outcomes.iter().zip(observations) {
                        let NativeInvocationResult::Completed(actual) = actual else {
                            panic!("complete trace child")
                        };
                        compare_native(actual, &witness["result"], prior);
                        prior = witness["result"]["logs"].as_array().unwrap().len();
                    }
                    assert_eq!(seen.borrow().outcomes.len(), observations.len());
                    traces += 1;
                    if restart == 0 {
                        let reads = seen.borrow().reads.clone();
                        for (index, (seq, _)) in reads.iter().enumerate() {
                            for malformed in [false, true] {
                                let (result, seen) =
                                    attempt.trace(trace, sequence, Some((index, malformed)));
                                let error = result.unwrap_err();
                                let (stage, target) = if sequence && *seq == 0 {
                                    (TraceSequenceStage::Prefix, 0)
                                } else {
                                    (TraceSequenceStage::Target, 0)
                                };
                                assert_eq!(
                                    error,
                                    StructuredTraceRunnerError::Execution {
                                        stage,
                                        index: target,
                                        error: ExecutionDriverError::Native(
                                            NativeAdapterError::Port(port_fault(
                                                &observations[*seq as usize]["result"],
                                                &reads[index].1,
                                                malformed,
                                                identity
                                            ))
                                        )
                                    }
                                );
                                assert_eq!(seen.borrow().outcomes.len(), *seq as usize);
                                assert_eq!(seen.borrow().reads.len(), index + 1);
                                faults += 1;
                            }
                            let (result, _) = attempt.trace(trace, sequence, None);
                            assert_eq!(
                                serde_json::from_slice::<Value>(&result.unwrap().output).unwrap(),
                                trace["result"]
                            );
                            traces += 1;
                        }
                    }
                }
                assert_eq!(
                    committed(&chain, identity.period.as_u64(), &fixture["state_before"]),
                    owner_before
                );
                assert_eq!(concrete_rows(&concrete), baseline);
            }
            cpp_index += row["consumers"]["estimates"].as_array().unwrap().len();
        }
    }
    assert_eq!(cpp_index, cpp["cases"].as_array().unwrap().len());
    assert!(calls > 0 && probes > 0 && traces > 0 && faults > 0);
    println!(
        "commission_lifecycle calls={calls} estimate_probes={probes} traces={traces} faults={faults}"
    );
}
