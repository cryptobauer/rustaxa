//! Direct claim-all API trace preserves ordinary-prefix and wide nonce context.
//! Physical reader and public FinalChain owner stay fixed after every attempt.
use super::metadata_trace::{block, requests};
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

/// Copies every column name and ordered logical row from the public owner's
/// RocksDB, including receipts and metadata. Opening a fresh read-only view
/// replays its WAL without flushing or changing the live owner. Empty columns
/// remain part of the comparison; the Go concrete reader is checked separately.
fn owner_rows(path: &FixturePath) -> Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)> {
    let path = path.0.join("db");
    let options = rocksdb::Options::default();
    let mut columns = rocksdb::DB::list_cf(&options, &path).unwrap();
    columns.sort();
    let descriptors = columns.iter().map(|name| {
        rustaxa_storage::Column::from_name(name)
            .unwrap()
            .descriptor(&options)
    });
    let db =
        rocksdb::DB::open_cf_descriptors_read_only(&options, &path, descriptors, false).unwrap();
    columns
        .into_iter()
        .map(|column| {
            let rows = db
                .iterator_cf(
                    &db.cf_handle(&column).unwrap(),
                    rocksdb::IteratorMode::Start,
                )
                .map(|row| {
                    let (key, value) = row.unwrap();
                    (key.to_vec(), value.to_vec())
                })
                .collect();
            (column, rows)
        })
        .collect()
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
