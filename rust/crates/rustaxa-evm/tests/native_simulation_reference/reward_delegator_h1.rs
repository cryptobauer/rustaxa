//! Positive delegator same-H1 claim trace over public finalized/reopened semantic P1.
//! The independent complete Go P1 reader supplies accounts/raw/root authority.
//! No production routing or Rust root-algorithm equality is claimed.
use super::*;
use rustaxa_evm::contracts::{
    NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
    NativeJournalAccount, NativeJournalRead, NativeJournalReadError, NativePortError,
};
use rustaxa_evm::contracts::{NativeOrdinaryAccountMutation, NativeRawOperation};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Observe actual authentication reads, then inject each failure without changing
/// the committed reader. Every attempt owns and drops a fresh semantic session.
struct AuthPort<'a> {
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    drops: Rc<Cell<usize>>,
    fail: Option<usize>,
    mode: u8,
    outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
    seen: Rc<RefCell<Vec<NativeInvocation>>>,
    fail_prepare: bool,
}
impl Drop for AuthPort<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
struct AuthJournal<'a> {
    inner: &'a dyn NativeJournalRead,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    fail: Option<usize>,
    mode: u8,
}
impl NativeJournalRead for AuthJournal<'_> {
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
        if self.fail == Some(index) {
            if self.mode == 2 {
                return Ok(ConcreteRead::Present(vec![0xff]));
            }
            if self.mode == 3 {
                return Ok(ConcreteRead::Absent);
            }
            return Err(NativeJournalReadError::Invariant(
                "delegator claim trace authentication failure".into(),
            ));
        }
        self.inner.raw_storage(address, key)
    }
}
impl NativeExecutionPort for AuthPort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.seen.borrow_mut().push(invocation.clone());
        if self.fail_prepare {
            return Err(NativePortError::Infrastructure(
                "injected trace prepare".into(),
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
        assert_eq!(invocation.period, 1.into());
        let result = self.inner.invoke(
            invocation,
            quote,
            &AuthJournal {
                inner: journal,
                reads: self.reads.clone(),
                fail: self.fail,
                mode: self.mode,
            },
        )?;
        self.outcomes.borrow_mut().push(result.clone());
        Ok(result)
    }
}

fn assert_witness(out: &rustaxa_evm::contracts::NativeOutcome, expected: &Value, caller: [u8; 20]) {
    assert_eq!(out.status, rustaxa_evm::contracts::NativeStatus::Success);
    assert_eq!(out.gas_used.as_u64(), 40000);
    assert!(out.output.is_empty());
    let raw=out.raw_mutations.iter().map(|w|json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
    assert_eq!(raw, *expected["raw"].as_array().unwrap());
    let logs=out.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
    assert_eq!(logs, *expected["logs"].as_array().unwrap());
    assert_eq!(out.account_mutations.len(), 2);
    for (m, (a, before, after)) in out
        .account_mutations
        .iter()
        .zip([(address(0xfe), 4800, 4404), (caller, 3000, 3396)])
    {
        assert!(
            matches!(m,NativeOrdinaryAccountMutation::Balance{address,expected_exists:true,expected,replacement} if *address==a&&expected.value()==&num_bigint::BigInt::from(before)&&replacement.value()==&num_bigint::BigInt::from(after))
        );
    }
}

use rustaxa_evm::simulation::SimulationError;
fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_delegator_h1/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_delegator_h1/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public
}
#[test]
fn delegator_claim_h1_matches_go_and_disposes_faulted_simulations() {
    let public = corpus();
    let simulation:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public["state_before"], simulation["state_before"]);
    let concrete = FixturePath::new("delegator-h1-concrete");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let app = FixturePath::new("delegator-h1-owner");
    let case = &public["cases"][0];
    let caller = bytes(&public["caller"]).try_into().unwrap();
    let original =
        FinalChainNonce::from_bytes(&(BigUint::from(1u8) << 512usize).to_bytes_be()).unwrap();
    let request = ExecutionTransaction {
        position: 0.into(),
        hash: [0; 32],
        sender: caller,
        receiver: Some(address(0xfe)),
        nonce: original.clone(),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: case["gas"].as_u64().unwrap().into(),
        value: ExecutionValue::new(number(&case["value"])),
        input: bytes(&case["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    };
    let block = ExecutionBlockContext {
        period: 1.into(),
        author: address(0x31),
        timestamp: 1700000001,
        gas_limit: 500000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    };
    let mut sessions = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &simulation);
        let owner_before = super::redelegate_rewarded_partial::committed(&chain, &simulation);
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        let mut plan = vec![(None, 0, false), (None, 0, false)];
        let mut attempt = 0;
        let mut expected_keys: Option<Vec<ConcreteStorageKey>> = None;
        while attempt < plan.len() {
            let (fail, mode, fail_prepare) = plan[attempt];
            attempt += 1;
            if mode == 4 || mode == 5 {
                if mode == 4 {
                    let mut wrong_block = block.clone();
                    wrong_block.period = 2.into();
                    let result = simulate_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |_| -> Result<AuthPort<'_>, NativePortError> {
                            panic!("factory after period mismatch")
                        },
                        &wrong_block,
                        &request,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    );
                    assert!(matches!(
                        result,
                        Err(SimulationError::StatePeriodMismatch { .. })
                    ));
                } else {
                    let mut wrong = identity;
                    wrong.state_root[0] ^= 1;
                    let result = simulate_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected| -> Result<AuthPort<'_>, NativePortError> {
                            assert_ne!(selected, wrong);
                            Err(NativePortError::Infrastructure(
                                "factory identity qualification".into(),
                            ))
                        },
                        &block,
                        &request,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    );
                    assert!(matches!(result, Err(SimulationError::NativeSession(_))));
                }
                assert_eq!(request.nonce, original);
                assert_eq!(concrete_rows(&concrete), before);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &simulation),
                    owner_before
                );
                continue;
            }
            sessions += 1;
            let reads = Rc::new(RefCell::new(Vec::new()));
            let drops = Rc::new(Cell::new(0));
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let seen = Rc::new(RefCell::new(Vec::new()));
            let result = simulate_with_native(
                &reader,
                &NoHistory,
                &Dpos,
                &Dpos,
                |selected| {
                    assert_eq!(selected, identity);
                    Ok(AuthPort {
                        inner: mixed_native::SimulationNativeExecutionPort::new(
                            chain.begin_native_simulation(selected.period).unwrap(),
                        ),
                        reads: reads.clone(),
                        drops: drops.clone(),
                        outcomes: outcomes.clone(),
                        seen: seen.clone(),
                        fail,
                        mode,
                        fail_prepare,
                    })
                },
                &block,
                &request,
                EnvelopeRules { cornus: true },
                TaraxaProfile::for_phase(TaraxaPhase::Ficus),
            );
            assert_eq!(drops.get(), 1);
            assert_eq!(request.nonce, original);
            assert_eq!(concrete_rows(&concrete), before);
            assert_eq!(
                super::redelegate_rewarded_partial::committed(&chain, &simulation),
                owner_before
            );
            if fail.is_some() || fail_prepare {
                assert!(
                    matches!(result, Err(SimulationError::Execution(_))),
                    "{result:?}"
                );
                assert!(outcomes.borrow().is_empty());
                if let Some(i) = fail {
                    assert_eq!(&*reads.borrow(), &expected_keys.as_ref().unwrap()[..=i]);
                } else {
                    assert!(reads.borrow().is_empty());
                }
                continue;
            }
            let result = result.unwrap();
            assert_eq!(result.state, identity);
            let TransactionExecutionResult::Executed(executed) = result.execution else {
                panic!("executed claim")
            };
            assert_eq!(executed.status, CodeExecutionStatus::Success);
            assert_eq!(
                executed.gas_used.as_u64(),
                case["output"]["gas_used"].as_u64().unwrap()
            );
            assert_eq!(executed.output, bytes(&case["output"]["return"]));
            let logs=executed.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
            assert_eq!(logs, *case["output"]["logs"].as_array().unwrap());
            assert_eq!(case["output"]["effective_nonce"], "1");
            let seen = seen.borrow();
            assert_eq!(seen.len(), 1);
            let i = &seen[0];
            assert_eq!(i.period, 1.into());
            assert_eq!(i.id.sequence, 0);
            assert_eq!(i.id.transaction, request.position);
            assert_eq!(i.depth, 0);
            assert_eq!(i.caller, request.sender);
            assert_eq!(i.input, request.input);
            assert_eq!(i.value.value(), request.value.value());
            assert_eq!(i.supplied_gas.as_u64(), 178536);
            assert_eq!(i.kind, rustaxa_evm::contracts::NativeCallKind::Call);
            assert!(!i.is_static);
            let keys = reads.borrow().clone();
            assert_eq!(keys.len(), 5);
            assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 5);
            if attempt == 1 {
                let mut present = 0;
                for (index, key) in keys.iter().enumerate() {
                    plan.extend([
                        (Some(index), 1, false),
                        (None, 0, false),
                        (Some(index), 2, false),
                        (None, 0, false),
                    ]);
                    if matches!(reader.storage(address(0xfe),*key).unwrap(),ConcreteRead::Present(ref v) if !v.is_empty())
                    {
                        present += 1;
                        plan.extend([(Some(index), 3, false), (None, 0, false)]);
                    }
                }
                assert_eq!(present, 4);
                plan.extend([
                    (None, 0, true),
                    (None, 0, false),
                    (None, 4, false),
                    (None, 0, false),
                    (None, 5, false),
                    (None, 0, false),
                ]);
                expected_keys = Some(keys.clone());
            }
            assert_eq!(&keys, expected_keys.as_ref().unwrap());
            let outcomes = outcomes.borrow();
            assert_eq!(outcomes.len(), 1);
            let NativeInvocationResult::Completed(out) = &outcomes[0] else {
                panic!("funded")
            };
            assert_witness(
                out,
                &json!({"raw":case["ordered_raw_writes"],"logs":case["output"]["logs"]}),
                caller,
            );
        }
    }
    assert_eq!(sessions, 68);
    let reopened = super::redelegate_rewarded_partial::history(&app, false, &simulation);
    super::redelegate_rewarded_partial::committed(&reopened, &simulation);
    assert_eq!(concrete_rows(&concrete), before);
}
