//! One direct claim-all compares ordered traversal and both scalar effects.
//! Physical reader and public FinalChain owner stay fixed after every attempt.
use super::*;
use rustaxa_evm::{
    contracts::{
        ConsensusNativeDisposition, ExecutionLog, NativeCallKind, NativeExecutionPort,
        NativeGasQuote, NativeInvocation, NativeInvocationResult, NativeJournalAccount,
        NativeJournalRead, NativeJournalReadError, NativeOrdinaryAccountMutation, NativePortError,
        NativeRawOperation, NativeStatus,
    },
    driver::PeriodConsensusSequence,
    driver::execute_top_level_call_with_native,
    journal::ExecutionJournal,
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
fn tx(row: &Value) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: addr(&row["from"]),
        receiver: Some(addr(&row["to"])),
        nonce: FinalChainNonce::from_u64(0),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: 200000.into(),
        value: ExecutionValue::new(0u8.into()),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
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

/// Every attempt is disposable; failures in count preparation, traversal or
/// either scalar serializer expose no outcome, and a clean retry follows.
#[test]
fn positive_claim_all_matches_go_and_retries_every_raw_fault() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all/local.json"
    ))
    .unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_after"], public["state_before"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    let row = &public["cases"][0];
    let concrete = FixturePath::new("claim-all-concrete");
    let identity = materialize(&public, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-owner");
    let mut attempts = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        let transaction = tx(row);
        let original_tx = transaction.clone();
        let mut plan = vec![(None, None), (None, None)];
        let mut at = 0;
        let mut keys: Option<Vec<ConcreteStorageKey>> = None;
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
            let mut journal =
                ExecutionJournal::new(CompleteSeedReader::open(&concrete, &public, identity));
            let mut sequence = PeriodConsensusSequence::new(2.into());
            for (k, v) in row["prior_raw"].as_object().unwrap() {
                let got = reader
                    .storage(
                        address(0xfe),
                        ConcreteStorageKey(hex::decode(k).unwrap().try_into().unwrap()),
                    )
                    .unwrap();
                assert_eq!(
                    match got {
                        ConcreteRead::Present(v) => v,
                        _ => vec![],
                    },
                    bytes(v)
                );
            }
            let mut port = Port {
                inner: mixed_native::MixedNativeExecutionPort::new(
                    chain
                        .begin_native_session_bound([0x91; 32], 2.into(), 1.into())
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
            };
            let block = ExecutionBlockContext {
                period: 2.into(),
                author: address(0x31),
                timestamp: 1700000002,
                gas_limit: 1_000_000.into(),
                chain_id: 666,
                difficulty: BigUint::default(),
            };
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &Dpos,
                &Dpos,
                &mut port,
                &mut sequence,
                &block,
                &transaction,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            );
            if fail_read.is_some() || fail_prepare.is_some() {
                let Err(rustaxa_evm::driver::ExecutionDriverError::Native(
                    rustaxa_evm::native::NativeAdapterError::Port(error),
                )) = result
                else {
                    panic!("native failure {result:?}")
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
                    ) => {
                        // The mixed port binds repeated count reads before the
                        // session can compare them; either layer must abort.
                        assert!(m.starts_with("native raw read changed during one phase:"));
                    }
                    (Some((_, 2 | 3)), None, NativePortError::Domain(m)) => {
                        assert!(m.contains("RawIntegrity"), "{m}")
                    }
                    (_, _, e) => panic!("fault {e:?}"),
                }
                assert!(outcomes.borrow().is_empty());
                let count = fail_read.map(|(i, _)| i + 1).unwrap_or(0);
                assert_eq!(&*reads.borrow(), &keys.as_ref().unwrap()[..count]);
                assert_eq!(
                    &*boundaries.borrow(),
                    if count <= 1 { &[][..] } else { &[1][..] }
                );
            } else {
                let TransactionExecutionResult::Executed(execution) = result.unwrap() else {
                    panic!("admitted")
                };
                assert_eq!(execution.status, CodeExecutionStatus::Success);
                assert_eq!(
                    execution.gas_used.as_u64(),
                    row["transaction_gas_used"].as_u64().unwrap()
                );
                assert_eq!(execution.output, bytes(&row["parent_output"]));
                assert_eq!(execution.logs, logs(&row["logs"]));
                assert_eq!(sequence.next_sequence(), 1);
                assert_eq!(seen.borrow().len(), 1);
                assert_eq!(*boundaries.borrow(), vec![1]);
                let call = &row["calls"][0];
                let i = seen.borrow()[0].clone();
                assert_eq!(i.id.sequence, 0);
                assert_eq!(i.id.transaction, 0.into());
                assert_eq!(i.period, 2.into());
                assert_eq!(i.depth, 0);
                assert_eq!(i.kind, NativeCallKind::Call);
                assert!(!i.is_static);
                assert_eq!(i.caller, addr(&call["caller"]));
                assert_eq!(i.input, bytes(&call["input"]));
                assert_eq!(i.value.value(), &BigUint::default());
                assert_eq!(
                    i.supplied_gas.as_u64(),
                    call["supplied_native_gas"].as_u64().unwrap()
                );
                let out = outcomes.borrow()[0].clone();
                let NativeInvocationResult::Completed(ref completed) = out else {
                    panic!("funded")
                };
                assert_eq!(completed.status, NativeStatus::Success);
                assert!(completed.output.is_empty());
                assert_eq!(completed.gas_used.as_u64(), 90000);
                assert_eq!(completed.logs, logs(&call["logs"]));
                assert_eq!(raw(&out), row["ordered_raw_writes"]);
                assert_eq!(balances(&out), row["ordered_balance_effects"]);
                assert_eq!(completed.raw_mutations.len(), 12);
                assert_eq!(completed.account_mutations.len(), 2);
                assert_eq!(completed.logs.len(), 1);
                for (a, v) in row["accounts"].as_object().unwrap() {
                    let a = hex::decode(a).unwrap().try_into().unwrap();
                    let got = journal.account(a).unwrap();
                    assert!(got.exists);
                    assert_eq!(
                        got.balance.value().to_string(),
                        v["balance"].as_str().unwrap()
                    );
                    assert_eq!(
                        journal.account_metadata(a).unwrap().nonce.to_bytes(),
                        FinalChainNonce::from_u64(v["nonce"].as_str().unwrap().parse().unwrap())
                            .to_bytes()
                    );
                }
                for (k, v) in row["after_raw"].as_object().unwrap() {
                    let got = NativeJournalRead::raw_storage(
                        &journal,
                        address(0xfe),
                        &ConcreteStorageKey(hex::decode(k).unwrap().try_into().unwrap()),
                    )
                    .unwrap();
                    assert_eq!(
                        match got {
                            ConcreteRead::Present(v) => v,
                            _ => vec![],
                        },
                        bytes(v)
                    );
                }
                let settled = journal.settle_transaction().unwrap();
                assert_eq!(settled.logs, execution.logs);
                assert_eq!(settled.native_invocations.len(), 1);
                let fact = &settled.native_invocations[0];
                assert_eq!(fact.disposition, ConsensusNativeDisposition::Normal);
                assert_eq!(fact.logs, completed.logs);
                assert_eq!(fact.gas_used, completed.gas_used);
                assert_eq!(fact.status, CodeExecutionStatus::Success);
                for (k, v) in row["after_raw"].as_object().unwrap() {
                    if v != &row["prior_raw"][k] {
                        assert!(settled.writes.raw_storage.iter().any(|w|hex::encode(w.key.0)==*k&&matches!(&w.operation,NativeRawOperation::Put(x)if x.as_bytes()==bytes(v))));
                    }
                }
                let actual = reads.borrow().clone();
                assert_eq!(actual.len(), 14);
                assert_eq!(actual.iter().collect::<BTreeSet<_>>().len(), 13);
                // Prepare count, invoke count, then the two observed item keys.
                assert_eq!(hex::encode(actual[0].0), row["index_before"]["count_key"]);
                assert_eq!(actual[0], actual[1]);
                for index in 0..2 {
                    assert_eq!(
                        hex::encode(actual[index + 2].0),
                        row["index_before"]["items"][index]["key"]
                    );
                }
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
            drop(port);
            drop(journal);
            assert_eq!(drops.get(), 1);
            assert_eq!(transaction, original_tx);
            assert_eq!(reader.identity(), identity);
            assert_eq!(concrete_rows(&concrete), original);
            assert_eq!(
                super::redelegate_rewarded_partial::committed(&chain, &seed),
                owner
            );
        }
    }
    assert_eq!(attempts, 168);
}
