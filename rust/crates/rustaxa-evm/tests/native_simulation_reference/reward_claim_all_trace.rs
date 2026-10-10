//! Direct claim-all API trace preserves ordinary-prefix and wide nonce context.
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
    inner: mixed_native::MixedNativeExecutionPort<'a>,
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
        self.inner.account(address)
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
            Some((at, 3)) if at == index => Ok(ConcreteRead::Absent),
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
        assert_eq!(native.balance.value().to_string(), "4800");
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

/// Empty and ordinary-prefix traces use one disposable port. Every target
/// authentication fault aborts Target0, including after the ordinary prefix;
/// retries always start from the original physical and semantic P1 owners.
#[test]
fn claim_all_api_trace_matches_go_and_preserves_original_nonce() {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_trace/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_trace/local.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_after"], public["state_before"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 2);
    let concrete = FixturePath::new("claim-all-trace-concrete");
    let identity = materialize(&public, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-trace-owner");
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
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        for case in public["cases"].as_array().unwrap() {
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            assert!(prefix.len() <= 1);
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
            let mut keys: Option<Vec<ConcreteStorageKey>> = None;
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
                        assert_eq!(execution, 2.into());
                        Ok(Port {
                            inner: mixed_native::MixedNativeExecutionPort::new(
                                chain
                                    .begin_native_session(execution, selected.period)
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
                    &block(2),
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
                if fail_read.is_some() || fail_prepare.is_some() {
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
                    assert_eq!(&*reads.borrow(), &keys.as_ref().unwrap()[..count]);
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
                assert_eq!(seen.borrow().len(), 1);
                assert_eq!(outcomes.borrow().len(), 1);
                assert_eq!(*boundaries.borrow(), vec![1]);
                let i = seen.borrow()[0].clone();
                let target = &targets[0];
                assert_eq!(i.id.sequence, 0);
                assert_eq!(i.id.transaction, target.position);
                assert_eq!(i.period, 2.into());
                assert_eq!(i.depth, 0);
                assert_eq!(i.kind, NativeCallKind::Call);
                assert!(!i.is_static);
                assert_eq!(i.caller, target.sender);
                assert_eq!(i.input, target.input);
                assert_eq!(i.value.value(), target.value.value());
                assert_eq!(i.supplied_gas.as_u64(), 178728);
                let out = outcomes.borrow()[0].clone();
                let NativeInvocationResult::Completed(ref completed) = out else {
                    panic!("funded")
                };
                assert_eq!(completed.status, NativeStatus::Success);
                assert_eq!(completed.gas_used.as_u64(), 90000);
                assert!(completed.output.is_empty());
                let w = &case["observation_witness"];
                assert_eq!(raw(&out), w["raw"]);
                assert_eq!(balances(&out), w["balances"]);
                assert_eq!(completed.logs, logs(&w["logs"]));
                assert_eq!(
                    (
                        completed.raw_mutations.len(),
                        completed.account_mutations.len(),
                        completed.logs.len()
                    ),
                    (12, 2, 1)
                );
                let actual = reads.borrow().clone();
                assert_eq!(actual.len(), 14);
                assert_eq!(actual.iter().collect::<BTreeSet<_>>().len(), 13);
                assert_eq!(actual[0], actual[1]);
                if keys.is_none() {
                    assert_eq!(present.borrow().iter().filter(|x| **x).count(), 12);
                    for (index, is_present) in present.borrow().iter().enumerate() {
                        for mode in 1..=if *is_present { 3 } else { 2 } {
                            plan.extend([(Some((index, mode)), None), (None, None)]);
                        }
                    }
                    plan.extend([(None, Some(0)), (None, None)]);
                    keys = Some(actual.clone());
                }
                assert_eq!(Some(actual), keys);
            }
        }
    }
    assert_eq!(sessions, 336);
}

/// Maps the actual Go count, membership and serializer key order to Rust's
/// authenticated read plan. Lazy bootstrap reads use qualified semantic state.
fn gas_boundary_reads(case: &Value) -> Vec<ConcreteStorageKey> {
    let count = case["ordered_reads"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|r| r["stage"] == "quote" && r["kind"] == "logical")
        .unwrap();
    let mut keys = vec![ConcreteStorageKey(bytes(&count["key"]).try_into().unwrap()); 2];
    let items = case["ordered_reads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["stage"] == "run" && r["kind"] == "logical")
        .skip(2)
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 2);
    keys.extend(
        items
            .iter()
            .map(|r| ConcreteStorageKey(bytes(&r["key"]).try_into().unwrap())),
    );
    for index in [0, 1, 2, 4, 5, 6, 7, 8, 10, 11] {
        keys.push(ConcreteStorageKey(
            bytes(&case["ordered_raw_writes"][index]["key"])
                .try_into()
                .unwrap(),
        ));
    }
    keys
}

/// Both P2/P1 prefix layouts match all measured caps. Faults cover every
/// reachable native raw read and preparation; retries use fresh disposable
/// sessions while every concrete, semantic and public owner row stays fixed.
#[test]
fn claim_all_api_gas_boundaries_match_go_and_retry_every_reachable_fault() {
    let public: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_underfunded_trace/public.json")).unwrap();
    let local: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_underfunded_trace/local.json")).unwrap();
    let seed: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["p1_before"], seed["state_before"]);
    assert_eq!(public["p1_after"], public["p1_before"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 14);
    let source = json!({"state_before":public["p1_before"]});
    let concrete = FixturePath::new("claim-all-p2-boundary-concrete");
    let identity = materialize(&source, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-p2-boundary-owner");
    let mut baseline = None;
    let mut attempts = 0;
    let mut faults = 0;
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
        for (index, case) in public["cases"].as_array().unwrap().iter().enumerate() {
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            assert_eq!(prefix.len(), index / 7);
            assert_eq!(targets.len(), 1);
            let original_prefix = prefix.clone();
            let original_targets = targets.clone();
            for (position, tx) in prefix.iter().chain(&targets).enumerate() {
                assert_eq!(
                    BigUint::from_bytes_be(&tx.nonce.to_bytes()),
                    (BigUint::from(1u8) << 512usize) + position
                );
            }
            let cap = targets[0].gas_limit.as_u64();
            assert_eq!(
                cap,
                [21271, 21272, 21273, 90000, 111271, 111272, 111273][index % 7]
            );
            let expected = if cap == 21271 {
                vec![]
            } else if cap < 111272 {
                gas_boundary_reads(&public["cases"][5])[..1].to_vec()
            } else {
                gas_boundary_reads(case)
            };
            let mut plan = vec![(None, None), (None, None)];
            let mut at = 0;
            while at < plan.len() {
                let (fail_read, fail_prepare) = plan[at];
                at += 1;
                attempts += 1;
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
                        assert_eq!(execution, 2.into());
                        Ok(Port {
                            inner: mixed_native::MixedNativeExecutionPort::new(
                                chain
                                    .begin_native_session(execution, selected.period)
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
                    &block(2),
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
                assert_eq!(owner_rows(&app), rows);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner
                );
                if fail_read.is_some() || fail_prepare.is_some() {
                    faults += 1;
                    let Err(StructuredTraceRunnerError::Execution {
                        stage: TraceSequenceStage::Target,
                        index: 0,
                        error:
                            rustaxa_evm::driver::ExecutionDriverError::Native(
                                rustaxa_evm::native::NativeAdapterError::Port(error),
                            ),
                    }) = result
                    else {
                        panic!("target fault {result:?}");
                    };
                    match (fail_read, fail_prepare, error) {
                        (None, Some(0), NativePortError::Infrastructure(m)) => {
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
                    assert_eq!(&*reads.borrow(), &expected[..count]);
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
                assert_eq!(*reads.borrow(), expected);
                if cap == 21271 {
                    assert!(
                        seen.borrow().is_empty()
                            && outcomes.borrow().is_empty()
                            && boundaries.borrow().is_empty()
                    );
                    continue;
                }
                assert_eq!(seen.borrow().len(), 1);
                assert_eq!(outcomes.borrow().len(), 1);
                assert_eq!(*boundaries.borrow(), vec![1]);
                let i = seen.borrow()[0].clone();
                let target = &targets[0];
                assert_eq!(i.id.sequence, 0);
                assert_eq!(i.id.transaction, target.position);
                assert_eq!(i.period, 2.into());
                assert_eq!(i.depth, 0);
                assert_eq!(i.kind, NativeCallKind::Call);
                assert!(!i.is_static);
                assert_eq!(i.caller, target.sender);
                assert_eq!(i.input, target.input);
                assert_eq!(i.value.value(), target.value.value());
                assert_eq!(i.supplied_gas.as_u64(), cap - 21272);
                let out = outcomes.borrow()[0].clone();
                if cap < 111272 {
                    assert_eq!(
                        out,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: 90000.into()
                        }
                    );
                } else {
                    let NativeInvocationResult::Completed(ref completed) = out else {
                        panic!("funded");
                    };
                    assert_eq!(completed.status, NativeStatus::Success);
                    assert_eq!(completed.gas_used.as_u64(), 90000);
                    assert!(completed.output.is_empty());
                    assert_eq!(raw(&out), case["ordered_raw_writes"]);
                    assert_eq!(balances(&out), case["ordered_balance_effects"]);
                    assert_eq!(completed.logs, logs(&case["post"]["logs"]));
                    assert_eq!(
                        (
                            completed.raw_mutations.len(),
                            completed.account_mutations.len(),
                            completed.logs.len()
                        ),
                        (12, 2, 1)
                    );
                    assert_eq!(expected.len(), 14);
                    assert_eq!(expected.iter().collect::<BTreeSet<_>>().len(), 13);
                }
                if at == 1 {
                    assert_eq!(present.borrow().len(), expected.len());
                    if cap >= 111272 {
                        assert_eq!(present.borrow().iter().filter(|x| **x).count(), 12);
                    }
                    for (index, is_present) in present.borrow().iter().enumerate() {
                        for mode in 1..=if *is_present { 3 } else { 2 } {
                            plan.extend([(Some((index, mode)), None), (None, None)]);
                        }
                    }
                    plan.extend([(None, Some(0)), (None, None)]);
                }
            }
        }
    }
    assert_eq!(attempts, 840);
    assert_eq!(faults, 392);
}

/// Keeps one journal, sequence and port across the actual ordinary prefix and
/// target. All measured gas/status/log, wide nonce, account and watched raw
/// post-state values match Go, while persisted owners stay unchanged on reopen.
#[test]
fn claim_all_api_gas_boundary_driver_matches_prefix_and_target_post_state() {
    use rustaxa_evm::contracts::{
        CodeExecutionError, CodeExecutionStatus, ConsensusNativeDisposition,
        TransactionExecutionResult,
    };
    use rustaxa_evm::{
        driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
        journal::ExecutionJournal,
    };
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_underfunded_trace/public.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let source = json!({"state_before":public["p1_before"]});
    let concrete = FixturePath::new("claim-all-p2-boundary-driver");
    let identity = materialize(&source, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-p2-boundary-driver-owner");
    let mut baseline = None;
    let mut attempts = 0;
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
            attempts += 1;
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            let originals = (prefix.clone(), targets.clone());
            let target = &targets[0];
            let cap = target.gas_limit.as_u64();
            let mut journal =
                ExecutionJournal::new(CompleteSeedReader::open(&concrete, &source, identity));
            let drops = Rc::new(Cell::new(0));
            let mut port = Port {
                inner: mixed_native::MixedNativeExecutionPort::new(
                    chain
                        .begin_native_session(2.into(), identity.period)
                        .unwrap(),
                ),
                reads: Rc::new(RefCell::new(Vec::new())),
                present: Rc::new(RefCell::new(Vec::new())),
                seen: Rc::new(RefCell::new(Vec::new())),
                outcomes: Rc::new(RefCell::new(Vec::new())),
                drops: drops.clone(),
                boundaries: Rc::new(RefCell::new(Vec::new())),
                fail_read: None,
                fail_prepare: None,
            };
            let mut sequence = PeriodConsensusSequence::new(2.into());
            for (index, p) in prefix.iter().enumerate() {
                let result = execute_top_level_call_with_native(
                    &mut journal,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    &mut port,
                    &mut sequence,
                    &block(2),
                    p,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                )
                .unwrap();
                let TransactionExecutionResult::Executed(out) = result else {
                    panic!("ordinary prefix");
                };
                let expected = &case["prefix_results"][index];
                assert_eq!(out.status, CodeExecutionStatus::Success);
                assert_eq!(out.gas_used.as_u64(), expected["gas"].as_u64().unwrap());
                assert!(out.output.is_empty() && out.logs.is_empty());
                assert_eq!(
                    journal
                        .account(p.sender)
                        .unwrap()
                        .balance
                        .value()
                        .to_string(),
                    expected["sender_balance"].as_str().unwrap()
                );
                assert_eq!(
                    BigUint::from_bytes_be(
                        &journal.account_metadata(p.sender).unwrap().nonce.to_bytes()
                    )
                    .to_string(),
                    expected["sender_nonce"].as_str().unwrap()
                );
                assert_eq!(
                    journal
                        .account(address(0xfe))
                        .unwrap()
                        .balance
                        .value()
                        .to_string(),
                    expected["native_balance"].as_str().unwrap()
                );
                assert!(port.seen.borrow().is_empty());
            }
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &Dpos,
                &Dpos,
                &mut port,
                &mut sequence,
                &block(2),
                target,
                EnvelopeRules { cornus: true },
                TaraxaProfile::for_phase(TaraxaPhase::Ficus),
            )
            .unwrap();
            let gas = match result {
                TransactionExecutionResult::ConsensusFailure(out) => {
                    assert_eq!(cap, 21271);
                    assert_eq!(
                        out.error,
                        rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
                    );
                    assert!(out.output.is_empty());
                    out.gas_used
                }
                TransactionExecutionResult::Executed(out) => {
                    assert_eq!(
                        out.status,
                        if cap >= 111272 {
                            CodeExecutionStatus::Success
                        } else {
                            CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                        }
                    );
                    assert!(out.output.is_empty());
                    assert_eq!(out.logs, logs(&case["post"]["logs"]));
                    out.gas_used
                }
            };
            assert_eq!(gas.as_u64(), case["post"]["gas_used"].as_u64().unwrap());
            assert_eq!((prefix.clone(), targets.clone()), originals);
            assert_eq!(
                BigUint::from_bytes_be(&target.nonce.to_bytes()).to_string(),
                case["post"]["supplied_nonce"].as_str().unwrap()
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
                        ConcreteRead::Present(v) => v,
                        _ => vec![],
                    },
                    bytes(expected)
                );
            }
            let settled = journal.settle_transaction().unwrap();
            assert_eq!(settled.native_invocations.len(), usize::from(cap >= 21272));
            assert_eq!(settled.logs, logs(&case["post"]["logs"]));
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
                assert!(fact.output.is_empty());
                assert_eq!(fact.logs, logs(&case["post"]["logs"]));
            }
            if cap < 111272 {
                assert!(settled.writes.raw_storage.is_empty());
            }
            drop(port);
            assert_eq!(drops.get(), 1);
            assert_eq!(concrete_rows(&concrete), original);
            assert_eq!(owner_rows(&app), rows);
            assert_eq!(
                super::redelegate_rewarded_partial::committed(&chain, &seed),
                owner
            );
        }
    }
    assert_eq!(attempts, 28);
}
