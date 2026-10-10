//! Seven literal H1/P0 gas caps bind captured P0 and the supplied wide nonce.
//! Only the measured empty-prefix, one-target case is admitted.
//! Physical reader and public FinalChain owner stay fixed after every attempt.
use super::metadata_trace::{block, requests};
use super::owner_snapshot::owner_rows;
use super::*;
use rustaxa_evm::contracts::{
    ExecutionLog, NativeCallKind, NativeExecutionPort, NativeGasQuote, NativeInvocation,
    NativeInvocationResult, NativeJournalAccount, NativeJournalRead, NativeJournalReadError,
    NativeOrdinaryAccountMutation, NativePortError, NativeRawOperation, NativeStatus,
};
use rustaxa_evm::trace_runner::{
    StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn addr(v: &Value) -> [u8; 20] {
    bytes(v).try_into().unwrap()
}
fn logs(v: &Value) -> Vec<ExecutionLog> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| ExecutionLog {
            address: addr(&x["address"]),
            topics: x["topics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| bytes(x).try_into().unwrap())
                .collect(),
            data: bytes(&x["data"]),
        })
        .collect()
}
fn raw(out: &NativeInvocationResult) -> Value {
    let NativeInvocationResult::Completed(out) = out else {
        panic!("funded")
    };
    json!(out.raw_mutations.iter().map(|w|json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>())
}
fn balances(out: &NativeInvocationResult) -> Value {
    let NativeInvocationResult::Completed(out) = out else {
        panic!("funded")
    };
    json!(out.account_mutations.iter().map(|m|{let NativeOrdinaryAccountMutation::Balance{address,expected_exists:true,expected,replacement}=m else{panic!("two payout balances")};let delta=replacement.value()-expected.value();let(op,amount)=if delta.sign()==num_bigint::Sign::Minus{("sub",-delta)}else{("add",delta)};json!({"address":hex::encode(address),"operation":op,"amount":amount.to_string(),"balance":replacement.value().to_string()})}).collect::<Vec<_>>())
}
struct Port<'a> {
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    seen: Rc<RefCell<Vec<NativeInvocation>>>,
    outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
    drops: Rc<Cell<usize>>,
    present: Rc<RefCell<Vec<bool>>>,
    boundaries: Rc<RefCell<Vec<usize>>>,
    fail_read: Option<(usize, u8)>,
    fail_prepare: Option<usize>,
}
impl Drop for Port<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
struct Journal<'a> {
    inner: &'a dyn NativeJournalRead,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    present: Rc<RefCell<Vec<bool>>>,
    fail: Option<(usize, u8)>,
}
impl NativeJournalRead for Journal<'_> {
    fn account(&self, address: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        panic!("zero-payout native scalar account read: {address:?}")
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        let index = self.reads.borrow().len();
        self.reads.borrow_mut().push(*key);
        match self.fail {
            Some((at, 2)) if at == index => Ok(ConcreteRead::Present(vec![0xff])),
            Some((at, 3)) if at == index => {
                // Flip presence in both directions. An already absent current
                // node is not a fault merely because it is absent again.
                Ok(match self.inner.raw_storage(address, key)? {
                    ConcreteRead::Present(_) => ConcreteRead::Absent,
                    ConcreteRead::Absent | ConcreteRead::Tombstone => {
                        ConcreteRead::Present(vec![0xff])
                    }
                })
            }
            Some((at, _)) if at == index => {
                Err(NativeJournalReadError::Invariant("frame raw fault".into()))
            }
            _ => {
                let value = self.inner.raw_storage(address, key)?;
                self.present
                    .borrow_mut()
                    .push(matches!(&value,ConcreteRead::Present(v) if !v.is_empty()));
                Ok(value)
            }
        }
    }
}
impl NativeExecutionPort for Port<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let index = self.seen.borrow().len();
        self.seen.borrow_mut().push(i.clone());
        if self.fail_prepare == Some(index) {
            return Err(NativePortError::Infrastructure(
                "frame prepare fault".into(),
            ));
        }
        // The driver has advanced the target nonce without rewriting the
        // supplied transaction. An ordinary prefix leaves both balances fixed.
        let caller = j.account(i.caller).map_err(NativePortError::Journal)?;
        let native = j.account(address(0xfe)).map_err(NativePortError::Journal)?;
        assert!(caller.exists && native.exists);
        assert_eq!(caller.balance.value().to_string(), "3000");
        assert_eq!(native.balance.value().to_string(), "4000");
        assert_eq!(
            BigUint::from_bytes_be(&native.nonce.to_bytes()),
            BigUint::from(1u8)
        );
        assert_eq!(
            BigUint::from_bytes_be(&caller.nonce.to_bytes()),
            (BigUint::from(1u8) << 512usize)
                + if i.id.transaction == 0.into() {
                    1u8
                } else {
                    2u8
                }
        );
        let quote = self.inner.prepare(
            i,
            &Journal {
                inner: j,
                reads: self.reads.clone(),
                present: self.present.clone(),
                fail: self.fail_read,
            },
        )?;
        assert_eq!(quote.required_gas.as_u64(), 90000);
        Ok(quote)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.boundaries.borrow_mut().push(self.reads.borrow().len());
        let result = self.inner.invoke(
            i,
            q,
            &Journal {
                inner: j,
                reads: self.reads.clone(),
                fail: self.fail_read,
                present: self.present.clone(),
            },
        )?;
        self.outcomes.borrow_mut().push(result.clone());
        Ok(result)
    }
}

fn expected_raw_reads(case: &Value) -> Vec<ConcreteStorageKey> {
    let quote_count = case["ordered_reads"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|row| row["stage"] == "quote" && row["kind"] == "logical")
        .unwrap();
    let run_items = case["ordered_reads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["stage"] == "run" && row["kind"] == "logical")
        .skip(2)
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(run_items.len(), 2);
    let mut expected_reads =
        vec![ConcreteStorageKey(bytes(&quote_count["key"]).try_into().unwrap()); 2];
    expected_reads.extend(
        run_items
            .iter()
            .map(|row| ConcreteStorageKey(bytes(&row["key"]).try_into().unwrap())),
    );
    for index in [0, 1, 2, 4, 5, 6, 7, 8, 10, 11] {
        expected_reads.push(ConcreteStorageKey(
            bytes(&case["ordered_raw_writes"][index]["key"])
                .try_into()
                .unwrap(),
        ));
    }
    expected_reads
}

/// Target authentication faults abort Target0. Every immediate clean retry
/// constructs a fresh port from the captured concrete P0 and unchanged semantic
/// history. Full Rust owner rows include its independently retained P1.
#[test]
fn claim_all_h1_p0_gas_boundaries_match_go_and_discard_every_attempt() {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_underfunded_trace/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_underfunded_trace/local.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["p1_before"], seed["state_before"]);
    assert_eq!(public["p1_after"], public["p1_before"]);
    assert_eq!(public["p0_source_after"], public["p0_source_before"]);
    assert_eq!(
        public["p0_source_before"]["seed_rows"]
            .as_array()
            .unwrap()
            .len(),
        82
    );
    assert_eq!(
        public["p1_before"]["seed_rows"].as_array().unwrap().len(),
        92
    );
    assert_eq!(public["cases"].as_array().unwrap().len(), 7);
    let source = json!({"state_before": public["p0_source_before"]});
    let concrete = FixturePath::new("claim-all-trace-concrete");
    let identity = materialize(&source, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-trace-owner");
    let canonical_read_order = expected_raw_reads(&public["cases"][5]);
    let mut sessions = 0;
    let mut original_owner_rows = None;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let owner_storage = owner_rows(&app);
        if let Some(before) = &original_owner_rows {
            assert_eq!(&owner_storage, before, "reopened owner rows");
        } else {
            original_owner_rows = Some(owner_storage.clone());
        }
        let reader = CompleteSeedReader::open(&concrete, &source, identity);
        for case in public["cases"].as_array().unwrap() {
            let cap = case["targets"][0]["gas"].as_u64().unwrap();
            let funded = cap >= 111272;
            let intrinsic = cap == 21271;
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            assert!(prefix.is_empty());
            assert_eq!(targets.len(), 1);
            let original_prefix = prefix.clone();
            let original_targets = targets.clone();
            for (index, tx) in prefix.iter().chain(&targets).enumerate() {
                assert_eq!(
                    BigUint::from_bytes_be(&tx.nonce.to_bytes()),
                    (BigUint::from(1u8) << 512usize) + index
                );
            }
            if let Some(p) = prefix.first() {
                assert_eq!(p.sender, p.receiver.unwrap());
                assert_ne!(p.receiver.unwrap(), address(0xfe));
                assert!(p.input.is_empty());
                assert_eq!(p.gas_limit.as_u64(), 21000);
                assert_eq!(p.value.value(), &BigUint::default());
            }
            let mut plan = vec![(None, None), (None, None)];
            for index in 0..14 {
                for mode in 1..=3 {
                    plan.extend([(Some((index, mode)), None), (None, None)]);
                }
            }
            plan.extend([(None, Some(0)), (None, None)]);
            let mut at = 0;
            while at < plan.len() {
                let (fail_read, fail_prepare) = plan[at];
                at += 1;
                sessions += 1;
                let reads = Rc::new(RefCell::new(Vec::new()));
                let present = Rc::new(RefCell::new(Vec::new()));
                let seen = Rc::new(RefCell::new(Vec::new()));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let boundaries = Rc::new(RefCell::new(Vec::new()));
                let result = run_structured_trace_with_native(
                    &reader,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |selected, execution| {
                        assert_eq!(selected, identity);
                        assert_eq!(execution, 1.into());
                        assert_eq!(selected.period, 0.into());
                        Ok(Port {
                            inner: mixed_native::SimulationNativeExecutionPort::new(
                                chain
                                    .begin_native_trace_claim_all_gas_boundaries(
                                        execution,
                                        selected.period,
                                    )
                                    .unwrap(),
                            ),
                            reads: reads.clone(),
                            present: present.clone(),
                            seen: seen.clone(),
                            outcomes: outcomes.clone(),
                            drops: drops.clone(),
                            boundaries: boundaries.clone(),
                            fail_read,
                            fail_prepare,
                        })
                    },
                    &block(1),
                    &prefix,
                    &targets,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                assert_eq!(drops.get(), 1);
                assert_eq!(prefix, original_prefix);
                assert_eq!(targets, original_targets);
                assert_eq!(reader.identity(), identity);
                assert_eq!(concrete_rows(&concrete), original);
                assert_eq!(owner_rows(&app), owner_storage);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner
                );
                let expected_fault = !intrinsic
                    && (fail_prepare.is_some()
                        || fail_read.is_some_and(|(index, _)| funded || index == 0));
                if expected_fault {
                    let Err(StructuredTraceRunnerError::Execution {
                        stage: TraceSequenceStage::Target,
                        index: 0,
                        error:
                            rustaxa_evm::driver::ExecutionDriverError::Native(
                                rustaxa_evm::native::NativeAdapterError::Port(error),
                            ),
                    }) = result
                    else {
                        panic!("target fault {result:?}")
                    };
                    match (fail_read, fail_prepare, error) {
                        (None, Some(_), NativePortError::Infrastructure(m)) => {
                            assert_eq!(m, "frame prepare fault")
                        }
                        (
                            Some((_, 1)),
                            None,
                            NativePortError::Journal(NativeJournalReadError::Invariant(m)),
                        ) => assert_eq!(m, "frame raw fault"),
                        (
                            Some((1, 2 | 3)),
                            None,
                            NativePortError::Journal(NativeJournalReadError::Invariant(m)),
                        ) => assert!(m.starts_with("native raw read changed during one phase:")),
                        (Some((_, 2 | 3)), None, NativePortError::Domain(m)) => {
                            assert!(m.contains("RawIntegrity"), "{m}")
                        }
                        (_, _, e) => panic!("typed fault {e:?}"),
                    }
                    assert!(outcomes.borrow().is_empty());
                    let count = fail_read.map(|(i, _)| i + 1).unwrap_or(0);
                    assert_eq!(&*reads.borrow(), &canonical_read_order[..count]);
                    assert_eq!(
                        &*boundaries.borrow(),
                        if count <= 1 { &[][..] } else { &[1][..] }
                    );
                    continue;
                }
                let result = result.unwrap();
                assert_eq!(result.state, identity);
                assert_eq!(
                    serde_json::from_slice::<Value>(&result.output).unwrap(),
                    case["result"]
                );
                if intrinsic {
                    assert!(seen.borrow().is_empty() && outcomes.borrow().is_empty());
                    assert!(boundaries.borrow().is_empty() && reads.borrow().is_empty());
                    continue;
                }
                assert_eq!(seen.borrow().len(), 1);
                assert_eq!(outcomes.borrow().len(), 1);
                assert_eq!(*boundaries.borrow(), vec![1]);
                let i = seen.borrow()[0].clone();
                let target = &targets[0];
                assert_eq!(i.id.sequence, 0);
                assert_eq!(i.id.transaction, target.position);
                assert_eq!(i.period, 1.into());
                assert_eq!(i.depth, 0);
                assert_eq!(i.kind, NativeCallKind::Call);
                assert!(!i.is_static);
                assert_eq!(i.caller, target.sender);
                assert_eq!(i.input, target.input);
                assert_eq!(i.value.value(), target.value.value());
                assert_eq!(i.supplied_gas.as_u64(), cap - 21272);
                let out = outcomes.borrow()[0].clone();
                if !funded {
                    assert_eq!(
                        out,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: 90000.into()
                        }
                    );
                    assert_eq!(*boundaries.borrow(), vec![1]);
                    assert_eq!(reads.borrow().len(), 1);
                    assert_eq!(
                        hex::encode(reads.borrow()[0].0),
                        case["ordered_reads"][8]["key"].as_str().unwrap()
                    );
                    continue;
                }
                let NativeInvocationResult::Completed(ref completed) = out else {
                    panic!("funded")
                };
                assert_eq!(completed.status, NativeStatus::Success);
                assert_eq!(completed.gas_used.as_u64(), 90000);
                assert!(completed.output.is_empty());
                let w = case;
                assert_eq!(raw(&out), w["ordered_raw_writes"]);
                let mut post_raw = case["prior_raw"].as_object().unwrap().clone();
                for write in raw(&out).as_array().unwrap() {
                    post_raw.insert(
                        write["key"].as_str().unwrap().into(),
                        write["value"].clone(),
                    );
                }
                assert_eq!(Value::Object(post_raw), case["post"]["after_raw"]);
                assert!(
                    case["ordered_account_scalar_reads"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(balances(&out), w["ordered_balance_effects"]);
                assert_eq!(completed.logs, logs(&w["post"]["logs"]));
                assert_eq!(
                    (
                        completed.raw_mutations.len(),
                        completed.account_mutations.len(),
                        completed.logs.len()
                    ),
                    (12, 0, 0)
                );
                let actual = reads.borrow().clone();
                assert_eq!(actual.len(), 14);
                assert_eq!(actual.iter().collect::<BTreeSet<_>>().len(), 13);
                assert_eq!(actual[0], actual[1]);
                assert_eq!(actual, canonical_read_order);
                assert_eq!(present.borrow().iter().filter(|x| **x).count(), 12);
            }
        }
    }
    assert_eq!(sessions, 1232);
}

/// These faults belong to the source/factory boundary, not native business
/// status. Each fault is followed by a fresh successful trace on the same owner.
struct SourceFault<'a> {
    reader: &'a CompleteSeedReader,
    mode: u8,
}
impl ConcreteStateRead for SourceFault<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        let mut identity = self.reader.identity();
        if self.mode == 1 {
            identity.period = 1.into();
        }
        if self.mode == 4 {
            identity.state_root[0] ^= 1;
        }
        identity
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if self.mode == 2 {
            return Err(ConcreteReadError::Io("trace sender source fault".into()));
        }
        self.reader.account(address)
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.reader.storage(address, key)
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.reader.code(hash)
    }
}
struct DropPort<'a> {
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    drops: Rc<Cell<usize>>,
}
impl Drop for DropPort<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
impl NativeExecutionPort for DropPort<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.inner.prepare(i, j)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.inner.invoke(i, q, j)
    }
}
#[test]
fn claim_all_h1_p0_gas_boundary_source_and_factory_errors_preserve_owners() {
    let public: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_underfunded_trace/public.json")).unwrap();
    let seed: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let source = json!({"state_before": public["p0_source_before"]});
    let concrete = FixturePath::new("claim-all-h1-p0-source-faults");
    let identity = materialize(&source, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-h1-p0-fault-owner");
    let case = &public["cases"][0];
    let targets = requests(&case["targets"], 0);
    let original_targets = targets.clone();
    let mut baseline = None;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows = owner_rows(&app);
        if let Some(before) = &baseline {
            assert_eq!(&rows, before);
        } else {
            baseline = Some(rows.clone());
        }
        let reader = CompleteSeedReader::open(&concrete, &source, identity);
        for fault in 1..=4 {
            for mode in [fault, 0] {
                let drops = Rc::new(Cell::new(0));
                let factories = Cell::new(0);
                let result = run_structured_trace_with_native(
                    &SourceFault {
                        reader: &reader,
                        mode,
                    },
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |selected, execution| {
                        factories.set(factories.get() + 1);
                        if selected != identity {
                            return Err(NativePortError::Infrastructure(
                                "trace identity binding".into(),
                            ));
                        }
                        assert_eq!(execution, 1.into());
                        if mode == 3 {
                            return Err(NativePortError::Infrastructure(
                                "trace factory fault".into(),
                            ));
                        }
                        Ok(DropPort {
                            inner: mixed_native::SimulationNativeExecutionPort::new(
                                chain
                                    .begin_native_trace_claim_all_gas_boundaries(
                                        execution,
                                        selected.period,
                                    )
                                    .unwrap(),
                            ),
                            drops: drops.clone(),
                        })
                    },
                    &block(1),
                    &[],
                    &targets,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                match mode {
                    0 => {
                        assert_eq!(
                            serde_json::from_slice::<Value>(&result.unwrap().output).unwrap(),
                            case["result"]
                        );
                        assert_eq!(drops.get(), 1);
                    }
                    1 => {
                        assert!(
                            matches!(result, Err(StructuredTraceRunnerError::StatePeriodMismatch { state, expected, execution }) if state.period == 1.into() && expected == 0.into() && execution == 1.into())
                        );
                        assert_eq!(factories.get(), 0);
                    }
                    2 => {
                        let Err(StructuredTraceRunnerError::Execution {
                            stage: TraceSequenceStage::Target,
                            index: 0,
                            error:
                                rustaxa_evm::driver::ExecutionDriverError::Envelope(
                                    rustaxa_evm::envelope::EnvelopeError::Journal(
                                        rustaxa_evm::journal::JournalError::State(
                                            ConcreteReadError::Io(message),
                                        ),
                                    ),
                                ),
                        }) = result
                        else {
                            panic!("source fault {result:?}")
                        };
                        assert_eq!(message, "trace sender source fault");
                        assert_eq!(drops.get(), 1);
                    }
                    3 | 4 => {
                        assert!(
                            matches!(result, Err(StructuredTraceRunnerError::NativeSession(NativePortError::Infrastructure(message))) if message == if mode == 3 { "trace factory fault" } else { "trace identity binding" })
                        );
                        assert_eq!(drops.get(), 0);
                    }
                    _ => unreachable!(),
                }
                assert_eq!(targets, original_targets);
                assert_eq!(reader.identity(), identity);
                assert_eq!(concrete_rows(&concrete), original);
                assert_eq!(owner_rows(&app), rows);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner
                );
            }
        }
    }
}

#[test]
fn claim_all_h1_p0_gas_boundary_driver_preserves_wide_nonce_and_exact_post_state() {
    use rustaxa_evm::contracts::{
        CodeExecutionError, CodeExecutionStatus, ConsensusNativeDisposition,
    };
    use rustaxa_evm::{
        driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
        journal::ExecutionJournal,
    };
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_underfunded_trace/public.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let source = json!({"state_before":public["p0_source_before"]});
    let concrete = FixturePath::new("claim-all-h1-p0-boundary-driver");
    let identity = materialize(&source, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-h1-p0-boundary-driver-owner");
    let mut baseline = None;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows = owner_rows(&app);
        if let Some(before) = &baseline {
            assert_eq!(&rows, before);
        } else {
            baseline = Some(rows.clone());
        }
        for case in public["cases"].as_array().unwrap() {
            let transactions = requests(&case["targets"], 0);
            let transaction = &transactions[0];
            let cap = transaction.gas_limit.as_u64();
            let mut journal =
                ExecutionJournal::new(CompleteSeedReader::open(&concrete, &source, identity));
            let drops = Rc::new(Cell::new(0));
            let mut port = DropPort {
                inner: mixed_native::SimulationNativeExecutionPort::new(
                    chain
                        .begin_native_trace_claim_all_gas_boundaries(1.into(), identity.period)
                        .unwrap(),
                ),
                drops: drops.clone(),
            };
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &Dpos,
                &Dpos,
                &mut port,
                &mut sequence,
                &block(1),
                transaction,
                EnvelopeRules { cornus: true },
                TaraxaProfile::for_phase(TaraxaPhase::Ficus),
            )
            .unwrap();
            let gas_used = match &result {
                rustaxa_evm::contracts::TransactionExecutionResult::ConsensusFailure(out) => {
                    assert_eq!(cap, 21271);
                    assert_eq!(
                        out.error,
                        rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
                    );
                    assert!(out.output.is_empty());
                    out.gas_used
                }
                rustaxa_evm::contracts::TransactionExecutionResult::Executed(out) => {
                    assert_eq!(
                        out.status,
                        if cap >= 111272 {
                            CodeExecutionStatus::Success
                        } else {
                            CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                        }
                    );
                    assert!(out.output.is_empty() && out.logs.is_empty());
                    out.gas_used
                }
            };
            assert_eq!(
                gas_used.as_u64(),
                case["post"]["gas_used"].as_u64().unwrap()
            );
            for (address, expected) in case["post"]["accounts"].as_object().unwrap() {
                let address: [u8; 20] = hex::decode(address).unwrap().try_into().unwrap();
                assert_eq!(
                    journal
                        .account(address)
                        .unwrap()
                        .balance
                        .value()
                        .to_string(),
                    expected["balance"].as_str().unwrap()
                );
                assert_eq!(
                    BigUint::from_bytes_be(
                        &journal.account_metadata(address).unwrap().nonce.to_bytes()
                    )
                    .to_string(),
                    expected["nonce"].as_str().unwrap()
                );
            }
            for (key, expected) in case["post"]["after_raw"].as_object().unwrap() {
                let key = ConcreteStorageKey(hex::decode(key).unwrap().try_into().unwrap());
                let got = NativeJournalRead::raw_storage(&journal, address(0xfe), &key).unwrap();
                assert_eq!(
                    match got {
                        ConcreteRead::Present(value) => value,
                        _ => vec![],
                    },
                    bytes(expected)
                );
            }
            let settled = journal.settle_transaction().unwrap();
            assert_eq!(settled.native_invocations.len(), usize::from(cap >= 21272));
            assert!(settled.logs.is_empty());
            if cap >= 21272 {
                let fact = &settled.native_invocations[0];
                assert_eq!(fact.required_gas.as_u64(), 90000);
                assert_eq!(
                    fact.gas_used.as_u64(),
                    if cap >= 111272 { 90000 } else { 0 }
                );
                assert_eq!(
                    fact.status,
                    if cap >= 111272 {
                        CodeExecutionStatus::Success
                    } else {
                        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                    }
                );
                assert_eq!(
                    fact.disposition,
                    if cap >= 111272 {
                        ConsensusNativeDisposition::Normal
                    } else {
                        ConsensusNativeDisposition::OwnFrameReverted
                    }
                );
                assert!(fact.logs.is_empty() && fact.output.is_empty());
            }
            if cap < 111272 {
                assert!(settled.writes.raw_storage.is_empty());
            }
            drop(port);
            drop(journal);
            assert_eq!(drops.get(), 1);
            assert_eq!(concrete_rows(&concrete), original);
            assert_eq!(owner_rows(&app), rows);
            assert_eq!(
                super::redelegate_rewarded_partial::committed(&chain, &seed),
                owner
            );
        }
    }
}
