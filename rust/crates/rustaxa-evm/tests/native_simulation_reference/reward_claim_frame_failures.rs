//! Finite low-funding/nonpayable claim frames; no raw read is permitted.
use super::reward_claim_frames::CodeView;
use super::*;
use rustaxa_evm::{
    contracts::{
        ConsensusNativeDisposition, NativeCallKind, NativeExecutionPort, NativeGasQuote,
        NativeInvocation, NativeInvocationResult, NativeJournalAccount, NativeJournalRead,
        NativeJournalReadError, NativePortError, NativeStatus,
    },
    driver::{ExecutionDriverError, PeriodConsensusSequence, execute_top_level_call_with_native},
    journal::ExecutionJournal,
    native::NativeAdapterError,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
fn addr(v: &Value) -> [u8; 20] {
    bytes(v).try_into().unwrap()
}

/// Raw access fails even on a clean attempt, so a future guard regression cannot
/// silently consume P1. Account reads still use the live ordinary value lane.
struct NoRaw<'a> {
    inner: &'a dyn NativeJournalRead,
    reads: Rc<Cell<usize>>,
}
impl NativeJournalRead for NoRaw<'_> {
    fn account(&self, a: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.inner.account(a)
    }
    fn raw_storage(
        &self,
        _: [u8; 20],
        _: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        self.reads.set(self.reads.get() + 1);
        Err(NativeJournalReadError::Invariant(
            "claim guard consumed raw state".into(),
        ))
    }
}
/// Each attempt owns a complete port, journal and sequence. Faults before either
/// phase require immediate clean retry; no failed phase may publish an outcome.
struct Port<'a> {
    inner: mixed_native::MixedNativeExecutionPort<'a>,
    seen: Rc<RefCell<Vec<(NativeInvocation, u64)>>>,
    outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
    reads: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
    fail: u8,
}
impl Drop for Port<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1)
    }
}
impl NativeExecutionPort for Port<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        if self.fail == 1 {
            return Err(NativePortError::Infrastructure(
                "claim frame prepare fault".into(),
            ));
        }
        let quote = self.inner.prepare(
            i,
            &NoRaw {
                inner: j,
                reads: self.reads.clone(),
            },
        )?;
        self.seen
            .borrow_mut()
            .push((i.clone(), quote.required_gas.as_u64()));
        Ok(quote)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        if self.fail == 2 {
            return Err(NativePortError::Infrastructure(
                "claim frame invoke fault".into(),
            ));
        }
        let result = self.inner.invoke(
            i,
            q,
            &NoRaw {
                inner: j,
                reads: self.reads.clone(),
            },
        )?;
        self.outcomes.borrow_mut().push(result.clone());
        Ok(result)
    }
}
#[test]
fn claim_frame_guards_match_go_and_retry_each_phase_failure() {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_frame_failures/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_frame_failures/local.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_after"], public["state_before"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 4);
    let concrete = FixturePath::new("claim-frame-guards-concrete");
    let identity = materialize(&public, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-frame-guards-owner");
    let mut attempts = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        for row in public["cases"].as_array().unwrap() {
            let transaction = ExecutionTransaction {
                position: 0.into(),
                hash: [0x11; 32],
                sender: addr(&row["from"]),
                receiver: Some(addr(&row["to"])),
                nonce: FinalChainNonce::from_u64(0),
                gas_price: ExecutionGasPrice::default(),
                gas_limit: 200000.into(),
                value: ExecutionValue::new(row["value"].as_u64().unwrap().into()),
                input: bytes(&row["input"]),
                canonical_rlp: None,
                kind: ExecutionTransactionKind::Call,
            };
            let original_tx = transaction.clone();
            let block = ExecutionBlockContext {
                period: 2.into(),
                author: address(0x31),
                timestamp: 1700000002,
                gas_limit: 1_000_000.into(),
                chain_id: 666,
                difficulty: BigUint::default(),
            };
            // Two baselines and an immediate clean retry after each phase fault.
            for fail in [0, 0, 1, 0, 2, 0] {
                attempts += 1;
                let seen = Rc::new(RefCell::new(Vec::new()));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let reads = Rc::new(Cell::new(0));
                let drops = Rc::new(Cell::new(0));
                let view = CodeView {
                    base: &reader,
                    wrapper: addr(&row["to"]),
                    code: bytes(&row["code"]),
                };
                let mut journal = ExecutionJournal::new(view);
                let mut sequence = PeriodConsensusSequence::new(2.into());
                let mut port = Port {
                    inner: mixed_native::MixedNativeExecutionPort::new(
                        chain
                            .begin_native_session_bound([0x91; 32], 2.into(), 1.into())
                            .unwrap(),
                    ),
                    seen: seen.clone(),
                    outcomes: outcomes.clone(),
                    reads: reads.clone(),
                    drops: drops.clone(),
                    fail,
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
                assert_eq!(reads.get(), 0);
                if fail != 0 {
                    let Err(ExecutionDriverError::Native(NativeAdapterError::Port(
                        NativePortError::Infrastructure(message),
                    ))) = result
                    else {
                        panic!("phase fault: {result:?}")
                    };
                    assert_eq!(
                        message,
                        if fail == 1 {
                            "claim frame prepare fault"
                        } else {
                            "claim frame invoke fault"
                        }
                    );
                    assert!(outcomes.borrow().is_empty());
                    assert_eq!(seen.borrow().len(), usize::from(fail == 2));
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
                    assert!(execution.logs.is_empty());
                    assert_eq!(sequence.next_sequence(), 1);
                    let call = &row["calls"][0];
                    assert_eq!(seen.borrow().len(), 1);
                    let (i, quote) = seen.borrow()[0].clone();
                    assert_eq!(quote, call["required_gas"].as_u64().unwrap());
                    assert_eq!(i.id.sequence, 0);
                    assert_eq!(i.id.transaction, 0.into());
                    assert_eq!(i.period, 2.into());
                    assert_eq!(i.depth, 1);
                    assert_eq!(i.caller, addr(&call["caller"]));
                    assert_eq!(i.input, bytes(&call["input"]));
                    assert_eq!(i.value.value().to_string(), call["value"].as_str().unwrap());
                    assert_eq!(
                        i.supplied_gas.as_u64(),
                        call["supplied_native_gas"].as_u64().unwrap()
                    );
                    assert_eq!(i.kind, NativeCallKind::Call);
                    assert!(!i.is_static);
                    assert_eq!(outcomes.borrow().len(), 1);
                    let nonpayable = row["value"] == 1;
                    match &outcomes.borrow()[0] {
                        NativeInvocationResult::InsufficientGas { required_gas } => {
                            assert!(!nonpayable);
                            assert_eq!(required_gas.as_u64(), quote);
                            assert_eq!(call["native_called"], false)
                        }
                        NativeInvocationResult::Completed(out) => {
                            assert!(nonpayable);
                            let NativeStatus::ContractFailure(error) = &out.status else {
                                panic!("nonpayable")
                            };
                            assert_eq!(error.error, call["native_error"].as_str().unwrap());
                            assert_eq!(out.gas_used.as_u64(), 0);
                            assert!(
                                out.output.is_empty()
                                    && out.raw_mutations.is_empty()
                                    && out.account_mutations.is_empty()
                                    && out.logs.is_empty()
                            );
                            assert_eq!(call["native_called"], true)
                        }
                    }
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
                            FinalChainNonce::from_u64(
                                v["nonce"].as_str().unwrap().parse().unwrap()
                            )
                            .to_bytes()
                        );
                    }
                    assert_eq!(row["prior_raw"], row["after_raw"]);
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
                    assert!(settled.logs.is_empty() && settled.writes.raw_storage.is_empty());
                    assert_eq!(settled.native_invocations.len(), 1);
                    let fact = &settled.native_invocations[0];
                    assert_eq!(
                        fact.disposition,
                        ConsensusNativeDisposition::OwnFrameReverted
                    );
                    assert_eq!(fact.required_gas.as_u64(), quote);
                    assert_eq!(fact.gas_used.as_u64(), 0);
                    assert!(fact.logs.is_empty() && fact.output.is_empty());
                    if nonpayable {
                        assert!(
                            matches!(&fact.status,CodeExecutionStatus::Failure(CodeExecutionError::Native(error))if error.error=="Method is not payable")
                        )
                    } else {
                        assert_eq!(
                            fact.status,
                            CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                        )
                    }
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
    }
    assert_eq!(attempts, 48);
}
