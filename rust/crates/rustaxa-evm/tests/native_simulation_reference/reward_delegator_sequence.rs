//! Sequential delegator claims use one live native port and journal over public P1.
//! Exact Go default JSON, raw writes, payout and cumulative logs remain separate.
use super::metadata_trace::{block, requests};
use super::*;
use rustaxa_evm::contracts::{
    NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
    NativeJournalAccount, NativeJournalRead, NativeJournalReadError, NativeOrdinaryAccountMutation,
    NativePortError, NativeRawOperation,
};
use rustaxa_evm::trace_runner::{
    StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace_with_native,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Each attempt owns the whole unpublished session. Inject failures at either
/// claim's raw authentication or preparation, including after the first payout.
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
            Some((at, _)) if at == index => Err(NativeJournalReadError::Invariant(
                "sequence raw fault".into(),
            )),
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
                "sequence prepare fault".into(),
            ));
        }
        let quote = self.inner.prepare(i, j)?;
        assert_eq!(quote.required_gas.as_u64(), 40000);
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

/// Compare the whole retained sequence with copy-only Go observations. Native
/// calls with zero delegator payout return no ordinary account or log effects.
fn witness(outcomes: &[NativeInvocationResult], expected: &Value, caller: [u8; 20], prefix: usize) {
    assert_eq!(outcomes.len(), 2);
    let mut raw = Vec::new();
    let mut logs = Vec::new();
    let mut cumulative = Vec::new();
    let mut balances = Vec::new();
    for (index, result) in outcomes.iter().enumerate() {
        let NativeInvocationResult::Completed(out) = result else {
            panic!("funded")
        };
        assert_eq!(out.status, rustaxa_evm::contracts::NativeStatus::Success);
        assert_eq!(out.gas_used.as_u64(), 40000);
        assert!(out.output.is_empty());
        assert_eq!(out.raw_mutations.len(), if index == 0 { 6 } else { 3 });
        for w in &out.raw_mutations {
            raw.push(json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation { NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[] })}));
        }
        assert_eq!(out.logs.len(), if index == 0 { 1 } else { 0 });
        cumulative.extend(out.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})));
        if index >= prefix {
            logs.extend(cumulative.iter().cloned());
        }
        if index == 0 {
            assert_eq!(out.account_mutations.len(), 2);
            for (mutation, (a, before, after, op)) in out.account_mutations.iter().zip([
                (address(0xfe), 4800, 4404, "sub"),
                (caller, 3000, 3396, "add"),
            ]) {
                assert!(
                    matches!(mutation, NativeOrdinaryAccountMutation::Balance{address,expected_exists:true,expected,replacement} if *address==a && expected.value()==&num_bigint::BigInt::from(before) && replacement.value()==&num_bigint::BigInt::from(after))
                );
                balances.push(json!({"address":hex::encode(a),"operation":op,"amount":"396","balance":after.to_string()}));
            }
        } else {
            assert!(out.account_mutations.is_empty());
        }
    }
    assert_eq!(
        json!({"raw":raw,"balances":balances,"logs":logs}),
        *expected
    );
}

#[test]
fn sequential_delegator_trace_matches_go_and_retries_each_call_fault() {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_delegator_sequence/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_delegator_sequence/local.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 2);
    let concrete = FixturePath::new("delegator-sequence-concrete");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let app = FixturePath::new("delegator-sequence-owner");
    let mut sessions = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner_before = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        for case in public["cases"].as_array().unwrap() {
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            let transactions = prefix.iter().chain(&targets).collect::<Vec<_>>();
            let nonces = transactions
                .iter()
                .map(|t| t.nonce.clone())
                .collect::<Vec<_>>();
            for (index, nonce) in nonces.iter().enumerate() {
                assert_eq!(
                    BigUint::from_bytes_be(&nonce.to_bytes()),
                    (BigUint::from(1u8) << 512usize) + index
                );
            }
            let mut plan = vec![(None, None), (None, None)];
            let mut expected_keys: Option<Vec<ConcreteStorageKey>> = None;
            let mut attempt = 0;
            while attempt < plan.len() {
                let (fail_read, fail_prepare) = plan[attempt];
                attempt += 1;
                sessions += 1;
                let reads = Rc::new(RefCell::new(Vec::new()));
                let seen = Rc::new(RefCell::new(Vec::new()));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let present = Rc::new(RefCell::new(Vec::new()));
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
                            seen: seen.clone(),
                            outcomes: outcomes.clone(),
                            drops: drops.clone(),
                            present: present.clone(),
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
                assert_eq!(
                    transactions
                        .iter()
                        .map(|t| t.nonce.clone())
                        .collect::<Vec<_>>(),
                    nonces
                );
                assert_eq!(concrete_rows(&concrete), before);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner_before
                );
                if fail_read.is_some() || fail_prepare.is_some() {
                    let at = fail_prepare
                        .unwrap_or_else(|| if fail_read.unwrap().0 < 5 { 0 } else { 1 });
                    let (expected_stage, expected_index) = if at < prefix.len() {
                        (TraceSequenceStage::Prefix, at)
                    } else {
                        (TraceSequenceStage::Target, at - prefix.len())
                    };
                    assert!(
                        matches!(result,Err(StructuredTraceRunnerError::Execution{stage,index,..}) if stage==expected_stage && index==expected_index),
                        "{result:?}"
                    );
                    assert_eq!(outcomes.borrow().len(), at);
                    let count = fail_read.map(|(index, _)| index + 1).unwrap_or(if at == 0 {
                        0
                    } else {
                        5
                    });
                    assert_eq!(&*reads.borrow(), &expected_keys.as_ref().unwrap()[..count]);
                    continue;
                }
                let result = result.unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                assert_eq!(result.state, identity);
                assert_eq!(
                    serde_json::from_slice::<Value>(&result.output).unwrap(),
                    case["result"]
                );
                let keys = reads.borrow().clone();
                assert_eq!(keys.len(), 7);
                assert_eq!(*boundaries.borrow(), vec![0, 5]);
                assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 5);
                if expected_keys.is_none() {
                    assert_eq!(present.borrow().iter().filter(|value| **value).count(), 6);
                    for (index, is_present) in present.borrow().iter().enumerate() {
                        for mode in 1..=if *is_present { 3 } else { 2 } {
                            plan.extend([(Some((index, mode)), None), (None, None)]);
                        }
                    }
                    for call in 0..2 {
                        plan.extend([(None, Some(call)), (None, None)]);
                    }
                    expected_keys = Some(keys.clone());
                }
                assert_eq!(Some(keys), expected_keys);
                for (index, (i, t)) in seen.borrow().iter().zip(&transactions).enumerate() {
                    assert_eq!(i.id.sequence, index as u64);
                    assert_eq!(i.id.transaction, t.position);
                    assert_eq!(i.period, 2.into());
                    assert_eq!(i.depth, 0);
                    assert_eq!(i.caller, t.sender);
                    assert_eq!(i.input, t.input);
                    assert_eq!(i.value.value(), t.value.value());
                    assert_eq!(i.supplied_gas.as_u64(), 178536);
                    assert_eq!(i.kind, rustaxa_evm::contracts::NativeCallKind::Call);
                    assert!(!i.is_static);
                }
                assert_eq!(seen.borrow().len(), 2);
                witness(
                    &outcomes.borrow(),
                    &case["observation_witness"],
                    transactions[0].sender,
                    prefix.len(),
                );
            }
        }
    }
    assert_eq!(sessions, 184);
    let reopened = super::redelegate_rewarded_partial::history(&app, false, &seed);
    super::redelegate_rewarded_partial::committed(&reopened, &seed);
    assert_eq!(concrete_rows(&concrete), before);
}
