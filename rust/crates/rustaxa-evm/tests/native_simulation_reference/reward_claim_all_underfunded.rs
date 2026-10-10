//! Measured first-call gas terminals over authenticated canonical reward-only P1.
//! Go lazy setup reads map to the loaded semantic owner; its full quote trace is
//! retained in the oracle, not relabeled as the Rust per-call count read.
use super::owner_snapshot::owner_rows;
use super::*;
use rustaxa_evm::{
    contracts::{
        ConsensusFailure, ConsensusNativeDisposition, ExecutionLog, NativeExecutionPort,
        NativeGasQuote, NativeInvocation, NativeInvocationResult, NativeJournalAccount,
        NativeJournalRead, NativeJournalReadError, NativeOrdinaryAccountMutation, NativePortError,
        NativeRawOperation, NativeStatus,
    },
    driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
    journal::ExecutionJournal,
    simulation::SimulationError,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Copy, Default, Debug)]
struct Fault {
    raw: Option<(usize, u8)>,
    account: Option<usize>,
    prepare: bool,
    factory: bool,
}
#[derive(Default)]
struct Seen {
    raw: Vec<ConcreteStorageKey>,
    present: Vec<bool>,
    accounts: Vec<[u8; 20]>,
    calls: Vec<NativeInvocation>,
    quotes: Vec<NativeGasQuote>,
    results: Vec<NativeInvocationResult>,
    boundaries: Vec<usize>,
}
struct Journal<'a> {
    inner: &'a dyn NativeJournalRead,
    seen: Rc<RefCell<Seen>>,
    fault: Fault,
}
impl NativeJournalRead for Journal<'_> {
    fn account(&self, address: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        let index = self.seen.borrow().accounts.len();
        self.seen.borrow_mut().accounts.push(address);
        if self.fault.account == Some(index) {
            return Err(NativeJournalReadError::Invariant(
                "boundary account fault".into(),
            ));
        }
        self.inner.account(address)
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        let index = self.seen.borrow().raw.len();
        self.seen.borrow_mut().raw.push(*key);
        let value = match self.fault.raw {
            Some((at, 2)) if at == index => ConcreteRead::Present(vec![0xff]),
            Some((at, 3)) if at == index => ConcreteRead::Absent,
            Some((at, _)) if at == index => {
                return Err(NativeJournalReadError::Invariant(
                    "boundary raw fault".into(),
                ));
            }
            _ => self.inner.raw_storage(address, key)?,
        };
        self.seen
            .borrow_mut()
            .present
            .push(matches!(&value, ConcreteRead::Present(v) if !v.is_empty()));
        Ok(value)
    }
}
enum Inner<'a> {
    Historical(Box<mixed_native::SimulationNativeExecutionPort<'a>>),
    Pending(Box<mixed_native::MixedNativeExecutionPort<'a>>),
}
impl NativeExecutionPort for Inner<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        match self {
            Self::Historical(port) => port.prepare(i, j),
            Self::Pending(port) => port.prepare(i, j),
        }
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        match self {
            Self::Historical(port) => port.invoke(i, q, j),
            Self::Pending(port) => port.invoke(i, q, j),
        }
    }
}
struct Port<'a> {
    inner: Inner<'a>,
    seen: Rc<RefCell<Seen>>,
    drops: Rc<Cell<usize>>,
    fault: Fault,
}
impl Drop for Port<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
impl NativeExecutionPort for Port<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.seen.borrow_mut().calls.push(i.clone());
        if self.fault.prepare {
            return Err(NativePortError::Infrastructure(
                "boundary prepare fault".into(),
            ));
        }
        let q = self.inner.prepare(
            i,
            &Journal {
                inner: j,
                seen: self.seen.clone(),
                fault: self.fault,
            },
        )?;
        self.seen.borrow_mut().quotes.push(q);
        Ok(q)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        let boundary = self.seen.borrow().raw.len();
        self.seen.borrow_mut().boundaries.push(boundary);
        let out = self.inner.invoke(
            i,
            q,
            &Journal {
                inner: j,
                seen: self.seen.clone(),
                fault: self.fault,
            },
        )?;
        self.seen.borrow_mut().results.push(out.clone());
        Ok(out)
    }
}
fn addr(value: &Value) -> [u8; 20] {
    bytes(value).try_into().unwrap()
}
fn logs(value: &Value) -> Vec<ExecutionLog> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|log| ExecutionLog {
            address: addr(&log["address"]),
            topics: log["topics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|topic| bytes(topic).try_into().unwrap())
                .collect(),
            data: bytes(&log["data"]),
        })
        .collect()
}
fn tx(row: &Value, caller: [u8; 20], nonce: &Value) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: caller,
        receiver: Some(addr(&row["to"])),
        nonce: if number(nonce) == BigUint::default() {
            FinalChainNonce::zero()
        } else {
            FinalChainNonce::from_bytes(&number(nonce).to_bytes_be()).unwrap()
        },
        gas_price: ExecutionGasPrice::default(),
        gas_limit: row["gas"].as_u64().unwrap().into(),
        value: ExecutionValue::new(0u8.into()),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}
fn raw(out: &rustaxa_evm::contracts::NativeOutcome) -> Value {
    json!(out.raw_mutations.iter().map(|w| json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation { NativeRawOperation::Put(value) => value.as_bytes(), NativeRawOperation::Delete => &[] })})).collect::<Vec<_>>())
}
fn balances(out: &rustaxa_evm::contracts::NativeOutcome) -> Value {
    json!(out.account_mutations.iter().map(|m| {
        let NativeOrdinaryAccountMutation::Balance { address, expected_exists: true, expected, replacement } = m else { panic!("payout balance") };
        let delta = replacement.value() - expected.value();
        let (operation, amount) = if delta.sign() == num_bigint::Sign::Minus { ("sub", -delta) } else { ("add", delta) };
        json!({"address":hex::encode(address),"operation":operation,"amount":amount.to_string(),"balance":replacement.value().to_string()})
    }).collect::<Vec<_>>())
}
fn assert_result(result: &TransactionExecutionResult, row: &Value) {
    let cap = row["gas"].as_u64().unwrap();
    if cap < 21272 {
        let TransactionExecutionResult::ConsensusFailure(out) = result else {
            panic!("intrinsic {result:?}")
        };
        assert_eq!(out.error, ConsensusFailure::IntrinsicGas);
        assert_eq!(out.gas_used.as_u64(), row["gas_used"].as_u64().unwrap());
        assert!(out.output.is_empty());
    } else {
        let TransactionExecutionResult::Executed(out) = result else {
            panic!("admitted {result:?}")
        };
        assert_eq!(
            out.status,
            if cap < 111272 {
                CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
            } else {
                CodeExecutionStatus::Success
            }
        );
        assert_eq!(out.gas_used.as_u64(), row["gas_used"].as_u64().unwrap());
        assert_eq!(out.output, bytes(&row["return"]));
        assert_eq!(out.logs, logs(&row["logs"]));
    }
}

struct SourceFault<'a>(&'a CompleteSeedReader);
impl ConcreteStateRead for SourceFault<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.0.identity()
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        Err(ConcreteReadError::Io("boundary sender source fault".into()))
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.0.storage(address, key)
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.0.code(hash)
    }
}

/// Source-period and sender failures remain infrastructure before any native
/// factory, even below intrinsic or native gas. The adapter itself returns the
/// entire native gas on a valid terminal and records failed-frame disposition.
#[test]
fn claim_all_boundary_source_errors_and_native_adapter_gas_remain_distinct() {
    use rustaxa_evm::contracts::{NativeCallKind, NativeInvocationId};
    use rustaxa_evm::native::invoke_native;

    let public: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_underfunded/public.json")).unwrap();
    let seed: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let concrete = FixturePath::new("claim-all-source-concrete");
    let identity = materialize(&public, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-source-owner");
    let chain = super::redelegate_rewarded_partial::history(&app, true, &seed);
    let rows_before = owner_rows(&app);
    let reader = CompleteSeedReader::open(&concrete, &public, identity);
    for row in public["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["period"] == 1 && row["gas"].as_u64().unwrap() < 111272)
    {
        let transaction = tx(row, addr(&public["caller"]), &row["supplied_nonce"]);
        let mut block = ExecutionBlockContext {
            period: 2.into(),
            author: address(0x31),
            timestamp: 1700000001,
            gas_limit: 1_000_000.into(),
            chain_id: 666,
            difficulty: BigUint::default(),
        };
        let mismatch = simulate_with_native(
            &reader,
            &NoHistory,
            &Dpos,
            &Dpos,
            |_| -> Result<Port<'_>, NativePortError> { panic!("period mismatch reached factory") },
            &block,
            &transaction,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        );
        assert!(
            matches!(mismatch, Err(SimulationError::StatePeriodMismatch { state, requested }) if state == identity && requested == 2.into())
        );
        block.period = 1.into();
        let source = simulate_with_native(
            &SourceFault(&reader),
            &NoHistory,
            &Dpos,
            &Dpos,
            |_| -> Result<Port<'_>, NativePortError> { panic!("sender fault reached factory") },
            &block,
            &transaction,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        );
        assert!(
            matches!(source, Err(SimulationError::Sender(rustaxa_evm::journal::JournalError::State(ConcreteReadError::Io(message)))) if message == "boundary sender source fault")
        );
    }
    for period in [1, 2] {
        for gas in [0, 1, 68728, 89999] {
            let seen = Rc::new(RefCell::new(Seen::default()));
            let drops = Rc::new(Cell::new(0));
            let mut port = Port {
                inner: if period == 1 {
                    Inner::Historical(Box::new(mixed_native::SimulationNativeExecutionPort::new(
                        chain.begin_native_simulation(1.into()).unwrap(),
                    )))
                } else {
                    Inner::Pending(Box::new(mixed_native::MixedNativeExecutionPort::new(
                        chain
                            .begin_native_session_bound([0x91; 32], 2.into(), 1.into())
                            .unwrap(),
                    )))
                },
                seen: seen.clone(),
                drops: drops.clone(),
                fault: Fault::default(),
            };
            let mut journal =
                ExecutionJournal::new(CompleteSeedReader::open(&concrete, &public, identity));
            let checkpoint = journal.checkpoint();
            let invocation = NativeInvocation {
                id: NativeInvocationId {
                    transaction: 0.into(),
                    sequence: 0,
                },
                period: period.into(),
                depth: 0,
                kind: NativeCallKind::Call,
                is_static: false,
                caller: addr(&public["caller"]),
                contract: address(0xfe),
                state_address: address(0xfe),
                value: ExecutionValue::new(0u8.into()),
                input: hex::decode("0b83a727").unwrap(),
                supplied_gas: gas.into(),
            };
            let out = invoke_native(&mut journal, &mut port, &invocation).unwrap();
            assert_eq!(out.required_gas.as_u64(), 90000);
            assert_eq!(out.gas_left.as_u64(), gas);
            assert_eq!(
                out.status,
                CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
            );
            assert!(out.output.is_empty());
            journal.revert_checkpoint(checkpoint).unwrap();
            let settled = journal.settle_transaction().unwrap();
            assert!(
                settled.writes.accounts.is_empty()
                    && settled.writes.raw_storage.is_empty()
                    && settled.logs.is_empty()
            );
            assert_eq!(settled.native_invocations.len(), 1);
            let fact = &settled.native_invocations[0];
            assert_eq!(fact.invocation, invocation);
            assert_eq!(fact.required_gas.as_u64(), 90000);
            assert_eq!(fact.gas_used.as_u64(), 0);
            assert_eq!(
                fact.disposition,
                ConsensusNativeDisposition::OwnFrameReverted
            );
            assert_eq!(fact.status, out.status);
            assert!(fact.output.is_empty() && fact.logs.is_empty());
            assert_eq!(seen.borrow().raw.len(), 1);
            assert!(seen.borrow().accounts.is_empty());
            drop(port);
            drop(journal);
            assert_eq!(drops.get(), 1);
            assert_eq!(concrete_rows(&concrete), original);
            assert_eq!(owner_rows(&app), rows_before);
        }
    }
}

/// Public H1 API plus its explicit effective-nonce driver and the independent
/// pending driver reproduce actual gas/error/effects. Fresh ports/journals are
/// discarded per attempt; complete source and owner rows survive every reopen.
#[test]
fn claim_all_underfunded_boundaries_match_go_and_discard_faults() {
    let public: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_underfunded/public.json")).unwrap();
    let local: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_underfunded/local.json")).unwrap();
    let seed: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(
        public["state_before"]["seed_rows"]
            .as_array()
            .unwrap()
            .len(),
        92
    );
    assert_eq!(public["cases"].as_array().unwrap().len(), 14);
    let concrete = FixturePath::new("claim-all-boundary-concrete");
    let identity = materialize(&public, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-boundary-owner");
    let caller = addr(&public["caller"]);
    let mut original_owner = None;
    let mut attempts = 0;
    let mut failures = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows_before = owner_rows(&app);
        if let Some(before) = &original_owner {
            assert_eq!(&rows_before, before);
        } else {
            original_owner = Some(rows_before.clone());
        }
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        // API H1 deliberately exposes no discarded journal. Its supplemental
        // effective-nonce driver checks exact local nonce and native transcript.
        for mode in 0..3 {
            let period = if mode == 2 { 2 } else { 1 };
            for row in public["cases"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["period"] == period)
            {
                let cap = row["gas"].as_u64().unwrap();
                let transaction = tx(
                    row,
                    caller,
                    if mode == 0 {
                        &row["supplied_nonce"]
                    } else {
                        &row["effective_nonce"]
                    },
                );
                let original_tx = transaction.clone();
                let block = ExecutionBlockContext {
                    period: period.into(),
                    author: address(0x31),
                    timestamp: 1700000000 + period,
                    gas_limit: 1_000_000.into(),
                    chain_id: 666,
                    difficulty: BigUint::default(),
                };
                let mut plan = vec![Fault::default(), Fault::default()];
                if cap < 111272 {
                    for index in 0..14 {
                        for fault_mode in 1..=3 {
                            plan.extend([
                                Fault {
                                    raw: Some((index, fault_mode)),
                                    ..Fault::default()
                                },
                                Fault::default(),
                            ]);
                        }
                    }
                    for index in 0..2 {
                        plan.extend([
                            Fault {
                                account: Some(index),
                                ..Fault::default()
                            },
                            Fault::default(),
                        ]);
                    }
                    plan.extend([
                        Fault {
                            prepare: true,
                            ..Fault::default()
                        },
                        Fault::default(),
                    ]);
                }
                if mode == 0 {
                    plan.extend([
                        Fault {
                            factory: true,
                            ..Fault::default()
                        },
                        Fault::default(),
                    ]);
                }
                let mut index = 0;
                let mut funded_keys = None;
                while index < plan.len() {
                    let fault = plan[index];
                    index += 1;
                    attempts += 1;
                    let seen = Rc::new(RefCell::new(Seen::default()));
                    let drops = Rc::new(Cell::new(0));
                    let make_port = || Port {
                        inner: if period == 1 {
                            Inner::Historical(Box::new(
                                mixed_native::SimulationNativeExecutionPort::new(
                                    chain.begin_native_simulation(1.into()).unwrap(),
                                ),
                            ))
                        } else {
                            Inner::Pending(Box::new(mixed_native::MixedNativeExecutionPort::new(
                                chain
                                    .begin_native_session_bound([0x91; 32], 2.into(), 1.into())
                                    .unwrap(),
                            )))
                        },
                        seen: seen.clone(),
                        drops: drops.clone(),
                        fault,
                    };
                    let failing = fault.factory
                        || (cap >= 21272
                            && (fault.prepare
                                || fault.raw.is_some_and(|(at, _)| at == 0 || cap >= 111272)
                                || (cap >= 111272 && fault.account.is_some())));
                    let result;
                    if mode == 0 {
                        let attempt = simulate_with_native(
                            &reader,
                            &NoHistory,
                            &Dpos,
                            &Dpos,
                            |selected| {
                                assert_eq!(selected, identity);
                                if fault.factory {
                                    return Err(NativePortError::Infrastructure(
                                        "boundary factory fault".into(),
                                    ));
                                }
                                Ok(make_port())
                            },
                            &block,
                            &transaction,
                            EnvelopeRules { cornus: true },
                            TaraxaProfile::new(false),
                        );
                        result = match attempt {
                            Ok(out) => {
                                assert_eq!(out.state, identity);
                                Some(out.execution)
                            }
                            Err(error) => {
                                assert!(failing, "unexpected API {error:?}");
                                assert!(matches!(
                                    error,
                                    SimulationError::NativeSession(_)
                                        | SimulationError::Execution(
                                            rustaxa_evm::driver::ExecutionDriverError::Native(
                                                rustaxa_evm::native::NativeAdapterError::Port(_)
                                            )
                                        )
                                ));
                                None
                            }
                        };
                    } else {
                        let mut journal = ExecutionJournal::new(CompleteSeedReader::open(
                            &concrete, &public, identity,
                        ));
                        let mut port = make_port();
                        let mut sequence = PeriodConsensusSequence::new(period.into());
                        let attempt = execute_top_level_call_with_native(
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
                        result = match attempt {
                            Ok(out) => {
                                assert!(!failing);
                                for (address, expected) in row["accounts"].as_object().unwrap() {
                                    let address: [u8; 20] =
                                        hex::decode(address).unwrap().try_into().unwrap();
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
                                            &journal
                                                .account_metadata(address)
                                                .unwrap()
                                                .nonce
                                                .to_bytes()
                                        )
                                        .to_string(),
                                        expected["nonce"].as_str().unwrap()
                                    );
                                }
                                for (key, expected) in row["after_raw"].as_object().unwrap() {
                                    let key = ConcreteStorageKey(
                                        hex::decode(key).unwrap().try_into().unwrap(),
                                    );
                                    let got = NativeJournalRead::raw_storage(
                                        &journal,
                                        address(0xfe),
                                        &key,
                                    )
                                    .unwrap();
                                    assert_eq!(
                                        match got {
                                            ConcreteRead::Present(value) => value,
                                            _ => vec![],
                                        },
                                        bytes(expected)
                                    );
                                }
                                let settled = journal.settle_transaction().unwrap();
                                assert_eq!(
                                    settled.native_invocations.len(),
                                    usize::from(cap >= 21272)
                                );
                                if cap >= 21272 {
                                    let fact = &settled.native_invocations[0];
                                    assert_eq!(fact.required_gas.as_u64(), 90000);
                                    assert_eq!(
                                        fact.gas_used.as_u64(),
                                        row["native_gas_used"].as_u64().unwrap()
                                    );
                                    assert_eq!(
                                        fact.status,
                                        if cap < 111272 {
                                            CodeExecutionStatus::Failure(
                                                CodeExecutionError::OutOfGas,
                                            )
                                        } else {
                                            CodeExecutionStatus::Success
                                        }
                                    );
                                    assert_eq!(
                                        fact.disposition,
                                        if cap < 111272 {
                                            ConsensusNativeDisposition::OwnFrameReverted
                                        } else {
                                            ConsensusNativeDisposition::Normal
                                        }
                                    );
                                    assert_eq!(fact.logs, logs(&row["logs"]));
                                    assert!(fact.output.is_empty());
                                }
                                if cap < 111272 {
                                    assert!(
                                        settled.writes.raw_storage.is_empty()
                                            && settled.logs.is_empty()
                                    );
                                }
                                Some(out)
                            }
                            Err(error) => {
                                assert!(failing, "unexpected driver {error:?}");
                                assert!(matches!(
                                    error,
                                    rustaxa_evm::driver::ExecutionDriverError::Native(
                                        rustaxa_evm::native::NativeAdapterError::Port(_)
                                    )
                                ));
                                None
                            }
                        };
                        drop(port);
                        drop(journal);
                    }
                    assert_eq!(drops.get(), usize::from(!fault.factory));
                    assert_eq!(
                        result.is_none(),
                        failing,
                        "fault{fault:?} cap{cap} mode{mode}"
                    );
                    let seen = seen.borrow();
                    if let Some(result) = result {
                        assert_result(&result, row);
                        assert_eq!(seen.calls.len(), usize::from(cap >= 21272));
                        assert_eq!(seen.results.len(), usize::from(cap >= 21272));
                        if cap >= 21272 {
                            assert_eq!(seen.quotes[0].required_gas.as_u64(), 90000);
                            assert_eq!(seen.calls[0].supplied_gas.as_u64(), cap - 21272);
                            assert_eq!(seen.calls[0].id.sequence, 0);
                            assert_eq!(seen.calls[0].caller, caller);
                            assert_eq!(seen.calls[0].period.as_u64(), period);
                            assert_eq!(seen.boundaries, vec![1]);
                            assert_eq!(hex::encode(seen.raw[0].0), row["ordered_reads"][8]["key"]);
                            if cap < 111272 {
                                assert_eq!(seen.raw.len(), 1);
                                assert!(seen.accounts.is_empty());
                                assert_eq!(
                                    seen.results[0],
                                    NativeInvocationResult::InsufficientGas {
                                        required_gas: 90000.into()
                                    }
                                );
                            } else {
                                let NativeInvocationResult::Completed(out) = &seen.results[0]
                                else {
                                    panic!("funded")
                                };
                                assert_eq!(out.status, NativeStatus::Success);
                                assert_eq!(out.gas_used.as_u64(), 90000);
                                assert!(out.output.is_empty());
                                assert_eq!(out.logs, logs(&row["logs"]));
                                assert_eq!(raw(out), row["ordered_raw_writes"]);
                                assert_eq!(balances(out), row["ordered_balance_effects"]);
                                assert_eq!(seen.raw.len(), 14);
                                assert_eq!(seen.raw[0], seen.raw[1]);
                                assert_eq!(seen.accounts.len(), 2);
                                if funded_keys.is_none() && cap == 111272 {
                                    for (at, present) in seen.present.iter().enumerate() {
                                        for fault_mode in 1..=if *present { 3 } else { 2 } {
                                            plan.extend([
                                                Fault {
                                                    raw: Some((at, fault_mode)),
                                                    ..Fault::default()
                                                },
                                                Fault::default(),
                                            ]);
                                        }
                                    }
                                    for at in 0..seen.accounts.len() {
                                        plan.extend([
                                            Fault {
                                                account: Some(at),
                                                ..Fault::default()
                                            },
                                            Fault::default(),
                                        ]);
                                    }
                                    plan.extend([
                                        Fault {
                                            prepare: true,
                                            ..Fault::default()
                                        },
                                        Fault::default(),
                                    ]);
                                    funded_keys = Some(seen.raw.clone());
                                }
                                if let Some(keys) = &funded_keys {
                                    assert_eq!(&seen.raw, keys);
                                }
                            }
                        } else {
                            assert!(
                                seen.raw.is_empty()
                                    && seen.accounts.is_empty()
                                    && seen.quotes.is_empty()
                            );
                        }
                    } else {
                        failures += 1;
                        assert!(seen.results.is_empty());
                        if fault.factory {
                            assert!(
                                seen.calls.is_empty()
                                    && seen.raw.is_empty()
                                    && seen.accounts.is_empty()
                            );
                        } else if fault.prepare {
                            assert_eq!(seen.calls.len(), 1);
                            assert!(
                                seen.quotes.is_empty()
                                    && seen.raw.is_empty()
                                    && seen.accounts.is_empty()
                                    && seen.boundaries.is_empty()
                            );
                        } else if let Some((at, _)) = fault.raw {
                            assert_eq!(seen.raw.len(), at + 1);
                            if let Some(keys) = &funded_keys {
                                assert_eq!(&seen.raw, &keys[..at + 1]);
                            }
                            assert_eq!(seen.boundaries, if at == 0 { vec![] } else { vec![1] });
                        } else if let Some(at) = fault.account {
                            assert_eq!(seen.accounts.len(), at + 1);
                        }
                    }
                    assert_eq!(transaction, original_tx);
                    assert_eq!(reader.identity(), identity);
                    assert_eq!(concrete_rows(&concrete), original);
                    assert_eq!(owner_rows(&app), rows_before);
                    assert_eq!(
                        super::redelegate_rewarded_partial::committed(&chain, &seed),
                        owner
                    );
                }
            }
        }
    }
    assert_eq!((attempts, failures), (3328, 368));
    eprintln!("boundary attempts{attempts} failures{failures}");
}
