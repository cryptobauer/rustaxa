//! Literal H1 public simulation compares actual ordered claim-all effects.
//! Physical reader and public FinalChain owner stay fixed after every attempt.
use super::owner_snapshot::owner_rows;
use super::*;
use rustaxa_evm::{
    contracts::{
        ExecutionLog, NativeCallKind, NativeExecutionPort, NativeGasQuote, NativeInvocation,
        NativeInvocationResult, NativeJournalAccount, NativeJournalRead, NativeJournalReadError,
        NativeOrdinaryAccountMutation, NativePortError, NativeRawOperation, NativeStatus,
    },
    simulation::{SimulationError, simulate_with_native},
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
fn tx(row: &Value, caller: [u8; 20]) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: caller,
        receiver: Some(addr(&row["to"])),
        nonce: FinalChainNonce::from_bytes(&number(&row["supplied_nonce"]).to_bytes_be()).unwrap(),
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
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    seen: Rc<RefCell<Vec<NativeInvocation>>>,
    outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
    drops: Rc<Cell<usize>>,
    present: Rc<RefCell<Vec<bool>>>,
    boundaries: Rc<RefCell<Vec<usize>>>,
    fail_read: Option<(usize, u8)>,
    fail_prepare: Option<usize>,
    fail_account: Option<[u8; 20]>,
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
    fail_account: Option<[u8; 20]>,
}
impl NativeJournalRead for Journal<'_> {
    fn account(&self, address: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        if self.fail_account == Some(address) {
            return Err(NativeJournalReadError::Invariant("H1 account fault".into()));
        }
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
        assert_eq!(i.period, 1.into());
        let caller = j.account(i.caller).map_err(NativePortError::Journal)?;
        let native = j.account(address(0xfe)).map_err(NativePortError::Journal)?;
        assert!(caller.exists && native.exists);
        assert_eq!(caller.nonce, FinalChainNonce::from_u64(2));
        assert_eq!(caller.balance.value().to_string(), "3000");
        assert_eq!(native.balance.value().to_string(), "4800");
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
                fail_account: self.fail_account,
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
                fail_account: self.fail_account,
                present: self.present.clone(),
            },
        )?;
        self.outcomes.borrow_mut().push(result.clone());
        Ok(result)
    }
}

/// Exact P1 owner and concrete reader are independent and unchanged. Every raw
/// read fault and preparation/account failure discards the whole fresh session;
/// each failure has an immediate clean retry. Original wide nonce is preserved.
#[test]
fn historical_claim_all_matches_go_and_discards_every_fault() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1/local.json"
    ))
    .unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_after"], public["state_before"]);
    assert_eq!(public["go_native_period"], 1);
    assert_eq!(public["go_evm_block"], 1);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    let row = &public["cases"][0];
    let concrete = FixturePath::new("claim-all-H1-concrete");
    let identity = materialize(&public, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-H1-owner");
    let mut baseline_owner = None;
    let mut attempts = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows_before = owner_rows(&app);
        if let Some(before) = &baseline_owner {
            assert_eq!(before, &rows_before, "reopened owner");
        } else {
            baseline_owner = Some(rows_before.clone());
        }
        assert!(
            chain.begin_native_simulation(2.into()).is_err(),
            "unfinalized semantic period"
        );
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        let transaction = tx(row, addr(&public["caller"]));
        let original_tx = transaction.clone();
        assert_eq!(
            BigUint::from_bytes_be(&transaction.nonce.to_bytes()),
            BigUint::from(1u8) << 512usize
        );
        let block = ExecutionBlockContext {
            period: 1.into(),
            author: address(0x31),
            timestamp: 1700000001,
            gas_limit: 1000000.into(),
            chain_id: 666,
            difficulty: BigUint::default(),
        };
        // Reader/block disagreement rejects before the native factory executes.
        let mut wrong = block.clone();
        wrong.period = 2.into();
        let factories = Cell::new(0);
        let mismatch = simulate_with_native(
            &reader,
            &NoHistory,
            &Dpos,
            &Dpos,
            |selected| {
                factories.set(factories.get() + 1);
                Ok(mixed_native::SimulationNativeExecutionPort::new(
                    chain.begin_native_simulation(selected.period).unwrap(),
                ))
            },
            &wrong,
            &transaction,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        );
        assert!(
            matches!(mismatch,Err(SimulationError::StatePeriodMismatch{state,requested})if state==identity&&requested==2.into())
        );
        assert_eq!(factories.get(), 0);
        assert_eq!(owner_rows(&app), rows_before);
        let mut plan = vec![(None, None, None), (None, None, None)];
        let mut at = 0;
        let mut keys: Option<Vec<ConcreteStorageKey>> = None;
        while at < plan.len() {
            let (fail_read, fail_prepare, fail_account) = plan[at];
            at += 1;
            attempts += 1;
            let reads = Rc::new(RefCell::new(Vec::new()));
            let present = Rc::new(RefCell::new(Vec::new()));
            let seen = Rc::new(RefCell::new(Vec::new()));
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let drops = Rc::new(Cell::new(0));
            let boundaries = Rc::new(RefCell::new(Vec::new()));
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
            let result = simulate_with_native(
                &reader,
                &NoHistory,
                &Dpos,
                &Dpos,
                |selected| {
                    assert_eq!(selected, identity);
                    Ok(Port {
                        inner: mixed_native::SimulationNativeExecutionPort::new(
                            chain.begin_native_simulation(selected.period).unwrap(),
                        ),
                        reads: reads.clone(),
                        present: present.clone(),
                        seen: seen.clone(),
                        outcomes: outcomes.clone(),
                        drops: drops.clone(),
                        boundaries: boundaries.clone(),
                        fail_read,
                        fail_prepare,
                        fail_account,
                    })
                },
                &block,
                &transaction,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            );
            if fail_read.is_some() || fail_prepare.is_some() || fail_account.is_some() {
                let Err(SimulationError::Execution(
                    rustaxa_evm::driver::ExecutionDriverError::Native(
                        rustaxa_evm::native::NativeAdapterError::Port(error),
                    ),
                )) = result
                else {
                    panic!("H1 infrastructure failure {result:?}")
                };
                match (fail_read, fail_prepare, fail_account, error) {
                    (None, Some(_), None, NativePortError::Infrastructure(m)) => {
                        assert_eq!(m, "frame prepare fault")
                    }
                    (
                        Some((_, 1)),
                        None,
                        None,
                        NativePortError::Journal(NativeJournalReadError::Invariant(m)),
                    ) => assert_eq!(m, "frame raw fault"),
                    (Some((_, 2 | 3)), None, None, NativePortError::Domain(m)) => {
                        assert!(m.contains("RawIntegrity"), "{m}")
                    }
                    (
                        None,
                        None,
                        Some(_),
                        NativePortError::Journal(NativeJournalReadError::Invariant(m)),
                    ) => assert_eq!(m, "H1 account fault"),
                    (_, _, _, e) => panic!("H1 fault {e:?}"),
                }
                assert!(outcomes.borrow().is_empty());
                let count = if fail_account.is_some() {
                    4
                } else {
                    fail_read.map(|(i, _)| i + 1).unwrap_or(0)
                };
                assert_eq!(&*reads.borrow(), &keys.as_ref().unwrap()[..count]);
                assert_eq!(
                    &*boundaries.borrow(),
                    if count <= 1 { &[][..] } else { &[1][..] }
                );
            } else {
                let simulated = result.unwrap();
                assert_eq!(simulated.state, identity);
                let TransactionExecutionResult::Executed(execution) = simulated.execution else {
                    panic!("admitted H1")
                };
                assert_eq!(execution.status, CodeExecutionStatus::Success);
                assert_eq!(
                    execution.gas_used.as_u64(),
                    row["output"]["gas_used"].as_u64().unwrap()
                );
                assert_eq!(execution.output, bytes(&row["output"]["return"]));
                assert_eq!(execution.logs, logs(&row["output"]["logs"]));
                assert_eq!(row["output"]["effective_nonce"], "1");
                assert_eq!(seen.borrow().len(), 1);
                assert_eq!(*boundaries.borrow(), vec![1]);
                let i = seen.borrow()[0].clone();
                assert_eq!(i.id.sequence, 0);
                assert_eq!(i.id.transaction, 0.into());
                assert_eq!(i.period, 1.into());
                assert_eq!(i.depth, 0);
                assert_eq!(i.kind, NativeCallKind::Call);
                assert!(!i.is_static);
                assert_eq!(i.caller, addr(&public["caller"]));
                assert_eq!(i.input, bytes(&row["input"]));
                assert_eq!(i.value.value(), &BigUint::default());
                assert_eq!(
                    i.supplied_gas.as_u64(),
                    row["quotes"][0]["supplied_native_gas"].as_u64().unwrap()
                );
                assert_eq!(outcomes.borrow().len(), 1);
                let out = outcomes.borrow()[0].clone();
                let NativeInvocationResult::Completed(ref completed) = out else {
                    panic!("funded")
                };
                assert_eq!(completed.status, NativeStatus::Success);
                assert!(completed.output.is_empty());
                assert_eq!(
                    completed.gas_used.as_u64(),
                    row["quotes"][0]["required_gas"].as_u64().unwrap()
                );
                assert_eq!(completed.logs, logs(&row["output"]["logs"]));
                assert_eq!(raw(&out), row["ordered_raw_writes"]);
                assert_eq!(balances(&out), row["ordered_balance_effects"]);
                assert_eq!(
                    (
                        completed.raw_mutations.len(),
                        completed.account_mutations.len(),
                        completed.logs.len()
                    ),
                    (12, 2, 1)
                );
                let mut planned = row["prior_raw"].as_object().unwrap().clone();
                for write in raw(&out).as_array().unwrap() {
                    planned.insert(
                        write["key"].as_str().unwrap().into(),
                        write["value"].clone(),
                    );
                }
                assert_eq!(Value::Object(planned), row["after_raw"]);
                let actual = reads.borrow().clone();
                assert_eq!(actual.len(), 14);
                assert_eq!(actual.iter().collect::<BTreeSet<_>>().len(), 13);
                assert_eq!(actual[0], actual[1]);
                if keys.is_none() {
                    assert_eq!(present.borrow().iter().filter(|x| **x).count(), 12);
                    for (index, is_present) in present.borrow().iter().enumerate() {
                        for mode in 1..=if *is_present { 3 } else { 2 } {
                            plan.extend([(Some((index, mode)), None, None), (None, None, None)]);
                        }
                    }
                    plan.extend([(None, Some(0), None), (None, None, None)]);
                    for account in [address(0xfe), addr(&public["caller"])] {
                        plan.extend([(None, None, Some(account)), (None, None, None)]);
                    }
                    keys = Some(actual.clone());
                }
                assert_eq!(Some(actual), keys);
            }
            assert_eq!(drops.get(), 1);
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
    assert_eq!(attempts, 176);
}

/// Each funded estimate probe owns one discarded H1 simulation. A callback
/// failure stops the search, preserves its typed error, and permits a clean
/// whole-estimate retry against the unchanged reader and FinalChain owner.
#[test]
fn historical_claim_all_estimates_match_go_cpp_and_stop_on_faults() {
    use rustaxa_evm::estimate::{EstimateError, EstimateProbe, estimate_gas};
    #[derive(Clone, Copy, Debug)]
    enum Fault {
        Raw(usize, u8),
        Prepare,
        Account([u8; 20]),
        Factory,
    }
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_estimate/public.json"
    )).unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_estimate/local.json"
    )).unwrap();
    let cpp: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_estimate/cpp.json"
    )).unwrap();
    let direct: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1/public.json"
    ))
    .unwrap();
    let seed: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json"
    )).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], direct["state_before"]);
    assert_eq!(public["state_after"], public["state_before"]);
    assert_eq!(public["go_native_period"], 1);
    assert_eq!(public["go_evm_block"], 1);
    assert_eq!(public["repeat_identical"], true);
    let cases = public["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    assert_eq!(cpp["cases"].as_array().unwrap().len(), 3);
    let concrete = FixturePath::new("claim-all-H1-estimate-concrete");
    let identity = materialize(&direct, &concrete);
    let original = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-H1-estimate-owner");
    let mut baseline_owner = None;
    let mut keys: Option<Vec<ConcreteStorageKey>> = None;
    let mut attempts = 0;
    let mut callbacks = 0;
    let mut allocated = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows_before = owner_rows(&app);
        if let Some(before) = &baseline_owner {
            assert_eq!(before, &rows_before, "reopened estimate owner");
        } else {
            baseline_owner = Some(rows_before.clone());
        }
        let reader = CompleteSeedReader::open(&concrete, &direct, identity);
        let block = ExecutionBlockContext {
            period: 1.into(),
            author: address(0x31),
            timestamp: 1700000001,
            gas_limit: 1000000.into(),
            chain_id: 666,
            difficulty: BigUint::default(),
        };
        for (case_index, case) in cases.iter().enumerate() {
            let probes = case["probes"].as_array().unwrap();
            let reference = &cpp["cases"][case_index];
            assert_eq!(case["name"], reference["name"]);
            assert_eq!(probes.len() as u64, reference["consumed"].as_u64().unwrap());
            let expected_gases: &[u64] = match case_index {
                0 => &[200000, 155636, 133454, 122363, 116817],
                1 => &[111272],
                2 => &[111273],
                _ => unreachable!(),
            };
            assert_eq!(
                probes
                    .iter()
                    .map(|p| p["gas"].as_u64().unwrap())
                    .collect::<Vec<_>>(),
                expected_gases
            );
            let mut prototype = tx(&probes[0], addr(&direct["caller"]));
            prototype.gas_limit = case["cap"].as_u64().unwrap().into();
            let original_tx = prototype.clone();
            assert_eq!(
                BigUint::from_bytes_be(&prototype.nonce.to_bytes()),
                BigUint::from(1u8) << 512usize
            );
            let mut plans = vec![None, None];
            for probe_at in 0..probes.len() {
                let mut faults: Vec<Fault> = (0..14).map(|at| Fault::Raw(at, 1)).collect();
                faults.extend([
                    Fault::Raw(0, 2),
                    Fault::Raw(2, 3),
                    Fault::Prepare,
                    Fault::Account(address(0xfe)),
                    Fault::Account(addr(&direct["caller"])),
                    Fault::Factory,
                ]);
                for fault in faults {
                    plans.extend([Some((probe_at, fault)), None]);
                }
            }
            for plan in plans {
                attempts += 1;
                let mut consumed = 0;
                let mut callback_error = None;
                let estimate = estimate_gas(prototype.gas_limit.as_u64(), |gas| {
                    let index = consumed;
                    consumed += 1;
                    callbacks += 1;
                    let row = &probes[index];
                    assert_eq!(gas, row["gas"].as_u64().unwrap());
                    let fault = plan.filter(|(at, _)| *at == index).map(|(_, f)| f);
                    let reads = Rc::new(RefCell::new(Vec::new()));
                    let present = Rc::new(RefCell::new(Vec::new()));
                    let seen = Rc::new(RefCell::new(Vec::new()));
                    let outcomes = Rc::new(RefCell::new(Vec::new()));
                    let drops = Rc::new(Cell::new(0));
                    let boundaries = Rc::new(RefCell::new(Vec::new()));
                    let factories = Cell::new(0);
                    let mut transaction = prototype.clone();
                    transaction.gas_limit = gas.into();
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
                    let result = simulate_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected| {
                            assert_eq!(selected, identity);
                            factories.set(factories.get() + 1);
                            if matches!(fault, Some(Fault::Factory)) {
                                return Err(NativePortError::Infrastructure(
                                    "H1 estimate factory fault".into(),
                                ));
                            }
                            allocated += 1;
                            Ok(Port {
                                inner: mixed_native::SimulationNativeExecutionPort::new(
                                    chain.begin_native_simulation(selected.period).unwrap(),
                                ),
                                reads: reads.clone(),
                                present: present.clone(),
                                seen: seen.clone(),
                                outcomes: outcomes.clone(),
                                drops: drops.clone(),
                                boundaries: boundaries.clone(),
                                fail_read: match fault {
                                    Some(Fault::Raw(at, mode)) => Some((at, mode)),
                                    _ => None,
                                },
                                fail_prepare: matches!(fault, Some(Fault::Prepare)).then_some(0),
                                fail_account: match fault {
                                    Some(Fault::Account(a)) => Some(a),
                                    _ => None,
                                },
                            })
                        },
                        &block,
                        &transaction,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::new(false),
                    );
                    assert_eq!(factories.get(), 1);
                    assert_eq!(
                        drops.get(),
                        usize::from(!matches!(fault, Some(Fault::Factory)))
                    );
                    assert_eq!(prototype, original_tx);
                    assert_eq!(reader.identity(), identity);
                    assert_eq!(concrete_rows(&concrete), original);
                    assert_eq!(owner_rows(&app), rows_before);
                    assert_eq!(
                        super::redelegate_rewarded_partial::committed(&chain, &seed),
                        owner
                    );
                    if let Some(fault) = fault {
                        let error = result.unwrap_err();
                        let port_error = match &error {
                            SimulationError::NativeSession(e)
                                if matches!(fault, Fault::Factory) =>
                            {
                                e
                            }
                            SimulationError::Execution(
                                rustaxa_evm::driver::ExecutionDriverError::Native(
                                    rustaxa_evm::native::NativeAdapterError::Port(e),
                                ),
                            ) => e,
                            e => panic!("estimate callback fault {fault:?}: {e:?}"),
                        };
                        match (fault, port_error) {
                            (Fault::Factory, NativePortError::Infrastructure(m)) => {
                                assert_eq!(m, "H1 estimate factory fault")
                            }
                            (Fault::Prepare, NativePortError::Infrastructure(m)) => {
                                assert_eq!(m, "frame prepare fault")
                            }
                            (
                                Fault::Raw(_, 1),
                                NativePortError::Journal(NativeJournalReadError::Invariant(m)),
                            ) => assert_eq!(m, "frame raw fault"),
                            (Fault::Raw(_, 2 | 3), NativePortError::Domain(m)) => {
                                assert!(m.contains("RawIntegrity"), "{m}")
                            }
                            (
                                Fault::Account(_),
                                NativePortError::Journal(NativeJournalReadError::Invariant(m)),
                            ) => assert_eq!(m, "H1 account fault"),
                            (_, e) => panic!("estimate port fault {fault:?}: {e:?}"),
                        }
                        assert!(outcomes.borrow().is_empty());
                        let count = match fault {
                            Fault::Raw(at, _) => at + 1,
                            Fault::Account(_) => 4,
                            _ => 0,
                        };
                        assert_eq!(&*reads.borrow(), &keys.as_ref().unwrap()[..count]);
                        assert_eq!(
                            &*boundaries.borrow(),
                            if count <= 1 { &[][..] } else { &[1][..] }
                        );
                        assert_eq!(
                            seen.borrow().len(),
                            usize::from(!matches!(fault, Fault::Factory))
                        );
                        callback_error = Some(error.clone());
                        return Err(error);
                    }
                    let simulated = result.unwrap();
                    assert_eq!(simulated.state, identity);
                    let TransactionExecutionResult::Executed(execution) = simulated.execution
                    else {
                        panic!("funded estimate")
                    };
                    assert_eq!(execution.status, CodeExecutionStatus::Success);
                    assert_eq!(execution.gas_used.as_u64(), 111272);
                    assert_eq!(
                        execution.gas_used.as_u64(),
                        row["output"]["gas_used"].as_u64().unwrap()
                    );
                    assert_eq!(execution.output, bytes(&row["output"]["return"]));
                    assert_eq!(execution.logs, logs(&row["output"]["logs"]));
                    assert_eq!(row["output"]["effective_nonce"], "1");
                    assert_eq!(seen.borrow().len(), 1);
                    let i = seen.borrow()[0].clone();
                    assert_eq!(i.id.sequence, 0);
                    assert_eq!(i.id.transaction, 0.into());
                    assert_eq!(i.period, 1.into());
                    assert_eq!(i.depth, 0);
                    assert_eq!(i.kind, NativeCallKind::Call);
                    assert!(!i.is_static);
                    assert_eq!(i.caller, addr(&direct["caller"]));
                    assert_eq!(i.input, bytes(&row["input"]));
                    assert_eq!(i.value.value(), &BigUint::default());
                    assert_eq!(
                        i.supplied_gas.as_u64(),
                        row["quotes"][0]["supplied_native_gas"].as_u64().unwrap()
                    );
                    assert_eq!(*boundaries.borrow(), vec![1]);
                    assert_eq!(outcomes.borrow().len(), 1);
                    let out = outcomes.borrow()[0].clone();
                    let NativeInvocationResult::Completed(ref completed) = out else {
                        panic!("funded")
                    };
                    assert_eq!(completed.status, NativeStatus::Success);
                    assert_eq!(completed.gas_used.as_u64(), 90000);
                    assert_eq!(row["quotes"][0]["required_gas"], 90000);
                    assert!(completed.output.is_empty());
                    assert_eq!(completed.logs, execution.logs);
                    assert_eq!(raw(&out), row["observation_witness"]["raw"]);
                    assert_eq!(balances(&out), row["observation_witness"]["balances"]);
                    assert_eq!(
                        (
                            completed.raw_mutations.len(),
                            completed.account_mutations.len(),
                            completed.logs.len()
                        ),
                        (12, 2, 1)
                    );
                    let mut planned = row["prior_raw"].as_object().unwrap().clone();
                    for write in raw(&out).as_array().unwrap() {
                        planned.insert(
                            write["key"].as_str().unwrap().into(),
                            write["value"].clone(),
                        );
                    }
                    assert_eq!(Value::Object(planned), row["after_raw"]);
                    let actual = reads.borrow().clone();
                    assert_eq!(actual.len(), 14);
                    assert_eq!(actual.iter().collect::<BTreeSet<_>>().len(), 13);
                    assert_eq!(actual[0], actual[1]);
                    assert_eq!(present.borrow().iter().filter(|p| **p).count(), 12);
                    if keys.is_none() {
                        keys = Some(actual.clone());
                    }
                    assert_eq!(Some(actual), keys);
                    Ok(EstimateProbe::Success {
                        gas_used: execution.gas_used.as_u64(),
                    })
                });
                if let Some((at, _)) = plan {
                    assert_eq!(consumed, at + 1, "no callback after failure");
                    assert_eq!(
                        estimate,
                        Err(EstimateError::Callback(callback_error.unwrap()))
                    );
                } else {
                    assert_eq!(consumed, probes.len());
                    assert_eq!(estimate, Ok(reference["result"].as_u64().unwrap()));
                }
            }
        }
    }
    assert_eq!((attempts, callbacks, allocated), (572, 1788, 1774));
}

/// Native callbacks are forbidden in the qualified guard lane. These are not
/// Go's lazy bootstrap callbacks; ordinary source/frame reads stay separate.
struct NoGuardReads;
impl NativeJournalRead for NoGuardReads {
    fn account(&self, _: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        panic!("guard native account callback")
    }
    fn raw_storage(
        &self,
        _: [u8; 20],
        _: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        panic!("guard native raw callback")
    }
}
struct NonpayablePort<'a> {
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    seen: Rc<RefCell<Vec<NativeInvocation>>>,
    outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
    drops: Rc<Cell<usize>>,
    fault: u8,
}
impl Drop for NonpayablePort<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
impl NativeExecutionPort for NonpayablePort<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.seen.borrow_mut().push(i.clone());
        // Diagnostic ordinary frame getters match the Go quote observer. The
        // semantic port gets NoGuardReads and cannot use these getters.
        let caller = j.account(i.caller).map_err(NativePortError::Journal)?;
        let native = j.account(address(0xfe)).map_err(NativePortError::Journal)?;
        assert_eq!(caller.balance.value().to_string(), "2999");
        assert_eq!(native.balance.value().to_string(), "4801");
        assert_eq!(
            caller.nonce,
            FinalChainNonce::from_u64(if i.period == 1.into() { 2 } else { 1 })
        );
        assert_eq!(i.id.sequence, 0);
        assert_eq!(i.id.transaction, 0.into());
        assert_eq!(i.depth, 0);
        assert_eq!(i.kind, NativeCallKind::Call);
        assert!(!i.is_static);
        assert_eq!(i.value.value(), &BigUint::from(1u8));
        assert_eq!(i.input, hex::decode("0b83a727").unwrap());
        if self.fault == 1 {
            return Err(NativePortError::Infrastructure(
                "nonpayable prepare fault".into(),
            ));
        }
        let q = self.inner.prepare(i, &NoGuardReads)?;
        assert_eq!(q.required_gas.as_u64(), 0);
        Ok(q)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        if self.fault == 2 {
            return Err(NativePortError::Infrastructure(
                "nonpayable invoke fault".into(),
            ));
        }
        let out = self.inner.invoke(i, q, &NoGuardReads)?;
        let NativeInvocationResult::Completed(completed) = &out else {
            panic!("guard completed")
        };
        assert!(
            matches!(&completed.status,NativeStatus::ContractFailure(e) if e.error=="Method is not payable")
        );
        assert_eq!(completed.gas_used.as_u64(), 0);
        assert!(
            completed.output.is_empty()
                && completed.raw_mutations.is_empty()
                && completed.account_mutations.is_empty()
                && completed.logs.is_empty()
        );
        self.outcomes.borrow_mut().push(out.clone());
        Ok(out)
    }
}
struct NonpayableSource {
    reader: CompleteSeedReader,
    fail: bool,
}
impl ConcreteStateRead for NonpayableSource {
    fn identity(&self) -> ConcreteStateIdentity {
        self.reader.identity()
    }
    fn account(
        &self,
        a: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if self.fail {
            Err(ConcreteReadError::Io(
                "nonpayable source account fault".into(),
            ))
        } else {
            self.reader.account(a)
        }
    }
    fn storage(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.reader.storage(a, k)
    }
    fn code(&self, h: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.reader.code(h)
    }
}
fn nonpayable_corpus() -> Value {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_nonpayable/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_nonpayable/local.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], seed["state_before"]);
    assert_eq!(public["state_after"], public["state_before"]);
    assert_eq!(public["caller"], seed["caller"]);
    assert_eq!(public["owner"], seed["owner"]);
    assert_eq!(
        public["state_before"]["seed_rows"]
            .as_array()
            .unwrap()
            .len(),
        92
    );
    assert_eq!(public["cases"].as_array().unwrap().len(), 14);
    public
}
/// Explicit common-source composition, not generic reader authentication. Both
/// views come from the same accepted positive P1 history; association compares
/// ordinary accounts, ordered validators/stakes, votes and all bootstrap bytes
/// and absence before an empty-callback semantic port can be made.
fn associate_nonpayable_p1(
    chain: &FinalChain,
    reader: &CompleteSeedReader,
    public: &Value,
    seed: &Value,
) {
    assert_eq!(public["state_before"], seed["state_before"]);
    super::redelegate_rewarded_partial::committed(chain, seed);
    assert_eq!(reader.identity().period, 1.into());
    assert_eq!(chain.dpos_eligible_total_vote_count(1.into()).unwrap(), 400);
    for row in public["state_before"]["accounts"].as_array().unwrap() {
        let a = addr(&row["address"]);
        let ConcreteRead::Present(got) = reader.account(a).unwrap() else {
            panic!("associated account")
        };
        assert_eq!(
            got.account.balance.value().to_string(),
            row["balance"].as_str().unwrap()
        );
        assert_eq!(
            BigUint::from_bytes_be(&got.account.nonce.to_bytes()).to_string(),
            row["nonce"].as_str().unwrap()
        );
    }
    for (key, value) in [
        (
            "f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393",
            Some("0190"),
        ),
        (
            "dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795",
            Some("0fa0"),
        ),
        (
            "ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc",
            None,
        ),
        (
            "d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873",
            Some("0320"),
        ),
    ] {
        let got = reader
            .storage(
                address(0xfe),
                ConcreteStorageKey(hex::decode(key).unwrap().try_into().unwrap()),
            )
            .unwrap();
        assert_eq!(
            got,
            match value {
                Some(v) => ConcreteRead::Present(hex::decode(v).unwrap()),
                None => ConcreteRead::Absent,
            }
        );
    }
    // Constructor qualification additionally proves the full positive topology,
    // minted800, provenance and graph before changing only private context.
    drop(
        chain
            .begin_native_claim_all_nonpayable(1.into(), reader.identity().period)
            .unwrap(),
    );
    drop(
        chain
            .begin_native_claim_all_nonpayable(2.into(), reader.identity().period)
            .unwrap(),
    );
}
fn nonpayable_nonce(value: &Value) -> FinalChainNonce {
    let n = number(value);
    if n == BigUint::default() {
        FinalChainNonce::default()
    } else {
        FinalChainNonce::from_bytes(&n.to_bytes_be()).unwrap()
    }
}
fn nonpayable_tx(row: &Value, caller: [u8; 20]) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: caller,
        receiver: Some(addr(&row["to"])),
        nonce: nonpayable_nonce(&row["supplied_nonce"]),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: row["gas"].as_u64().unwrap().into(),
        value: ExecutionValue::new(1u8.into()),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}
fn assert_nonpayable_execution(result: TransactionExecutionResult, row: &Value) {
    let gas = match result {
        TransactionExecutionResult::ConsensusFailure(out) => {
            assert_eq!(row["gas"], 21271);
            assert_eq!(
                out.error,
                rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
            );
            assert!(out.output.is_empty());
            out.gas_used
        }
        TransactionExecutionResult::Executed(out) => {
            assert!(
                matches!(&out.status,CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) if e.error==row["execution_error"].as_str().unwrap())
            );
            assert!(out.output.is_empty() && out.logs.is_empty());
            out.gas_used
        }
    };
    assert_eq!(gas.as_u64(), row["gas_used"].as_u64().unwrap());
}

/// Direct ordinary owner checks both measured period pairs, rollback, envelope
/// nonce, all3 accounts/23 keys and native failure facts. Source/prepare/invoke
/// hard faults have immediate fresh retries; owners are checked after EACH call.
#[test]
fn nonpayable_claim_all_driver_matches_both_go_owners_and_retries_faults() {
    use rustaxa_evm::contracts::ConsensusNativeDisposition;
    use rustaxa_evm::{
        driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
        journal::ExecutionJournal,
    };
    let public = nonpayable_corpus();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let concrete = FixturePath::new("nonpayable-driver-source");
    let identity = materialize(&public, &concrete);
    let source_rows = concrete_rows(&concrete);
    let app = FixturePath::new("nonpayable-driver-owner");
    let mut baseline = None;
    let mut attempts = 0;
    let mut hard_faults = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows = owner_rows(&app);
        if let Some(old) = &baseline {
            assert_eq!(old, &rows)
        } else {
            baseline = Some(rows.clone())
        }
        associate_nonpayable_p1(
            &chain,
            &CompleteSeedReader::open(&concrete, &public, identity),
            &public,
            &seed,
        );
        for row in public["cases"].as_array().unwrap() {
            let period = row["period"].as_u64().unwrap();
            let admitted = row["gas"].as_u64().unwrap() >= 21272;
            let supplied = nonpayable_tx(row, addr(&public["caller"]));
            let original = supplied.clone();
            let mut effective = supplied.clone();
            effective.nonce = nonpayable_nonce(&row["effective_nonce"]);
            for fault in [0, 1, 0, 2, 0, 3, 0, 4, 0] {
                attempts += 1;
                let mut journal = ExecutionJournal::new(NonpayableSource {
                    reader: CompleteSeedReader::open(&concrete, &public, identity),
                    fail: fault == 3,
                });
                let seen = Rc::new(RefCell::new(Vec::new()));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let mut port = NonpayablePort {
                    inner: mixed_native::SimulationNativeExecutionPort::new(
                        chain
                            .begin_native_claim_all_nonpayable(period.into(), identity.period)
                            .unwrap(),
                    ),
                    seen: seen.clone(),
                    outcomes: outcomes.clone(),
                    drops: drops.clone(),
                    fault,
                };
                let result = execute_top_level_call_with_native(
                    &mut journal,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    &mut port,
                    &mut PeriodConsensusSequence::new(period.into()),
                    &super::metadata_trace::block(period),
                    &effective,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                drop(port);
                assert_eq!(drops.get(), 1);
                if fault == 3 || (admitted && matches!(fault, 1 | 2)) {
                    hard_faults += 1;
                    let err = result.unwrap_err();
                    if fault == 3 {
                        assert!(
                            matches!(err,rustaxa_evm::driver::ExecutionDriverError::Envelope(rustaxa_evm::envelope::EnvelopeError::Journal(rustaxa_evm::journal::JournalError::State(ConcreteReadError::Io(m)))) if m=="nonpayable source account fault")
                        );
                    } else {
                        assert!(
                            matches!(err,rustaxa_evm::driver::ExecutionDriverError::Native(rustaxa_evm::native::NativeAdapterError::Port(NativePortError::Infrastructure(m))) if m==if fault==1 {"nonpayable prepare fault"} else {"nonpayable invoke fault"})
                        );
                    }
                    assert!(outcomes.borrow().is_empty());
                } else {
                    assert_nonpayable_execution(result.unwrap(), row);
                    assert_eq!(seen.borrow().len(), usize::from(admitted));
                    assert_eq!(outcomes.borrow().len(), usize::from(admitted));
                    for (a, expected) in row["accounts"].as_object().unwrap() {
                        let a: [u8; 20] = hex::decode(a).unwrap().try_into().unwrap();
                        assert_eq!(
                            journal.account(a).unwrap().balance.value().to_string(),
                            expected["balance"].as_str().unwrap()
                        );
                        assert_eq!(
                            BigUint::from_bytes_be(
                                &journal.account_metadata(a).unwrap().nonce.to_bytes()
                            )
                            .to_string(),
                            expected["nonce"].as_str().unwrap()
                        );
                    }
                    for (k, v) in row["after_raw"].as_object().unwrap() {
                        let got = journal
                            .raw_storage(
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
                    let settled = journal.settle_transaction().unwrap();
                    assert_eq!(settled.native_invocations.len(), usize::from(admitted));
                    if admitted {
                        let fact = &settled.native_invocations[0];
                        assert_eq!(fact.required_gas.as_u64(), 0);
                        assert_eq!(fact.gas_used.as_u64(), 0);
                        assert_eq!(
                            fact.disposition,
                            ConsensusNativeDisposition::OwnFrameReverted
                        );
                        assert!(fact.output.is_empty() && fact.logs.is_empty());
                        assert_eq!(fact.invocation, seen.borrow()[0]);
                    }
                }
                assert_eq!(supplied, original);
                assert_eq!(concrete_rows(&concrete), source_rows);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner
                );
                assert_eq!(owner_rows(&app), rows);
            }
        }
    }
    assert_eq!(attempts, 252);
    assert_eq!(hard_faults, 76);
}

/// Literal public H1 DryRunner preserves the caller's supplied wide nonce,
/// checks selected identity before construction and drops every disposable port.
#[test]
fn nonpayable_claim_all_public_h1_preserves_input_and_factory_boundaries() {
    let public = nonpayable_corpus();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let concrete = FixturePath::new("nonpayable-public-source");
    let identity = materialize(&public, &concrete);
    let source_rows = concrete_rows(&concrete);
    let app = FixturePath::new("nonpayable-public-owner");
    let mut baseline = None;
    let mut attempts = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows = owner_rows(&app);
        if let Some(old) = &baseline {
            assert_eq!(old, &rows)
        } else {
            baseline = Some(rows.clone())
        }
        let reader = CompleteSeedReader::open(&concrete, &public, identity);
        associate_nonpayable_p1(&chain, &reader, &public, &seed);
        for row in public["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["period"] == 1)
        {
            let transaction = nonpayable_tx(row, addr(&public["caller"]));
            let original = transaction.clone();
            assert_eq!(
                BigUint::from_bytes_be(&transaction.nonce.to_bytes()),
                BigUint::from(1u8) << 512usize
            );
            for fail in [false, true, false] {
                attempts += 1;
                let drops = Rc::new(Cell::new(0));
                let seen = Rc::new(RefCell::new(Vec::new()));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let result = simulate_with_native(
                    &reader,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |selected| {
                        assert_eq!(selected, identity);
                        if fail {
                            return Err(NativePortError::Infrastructure(
                                "nonpayable factory fault".into(),
                            ));
                        }
                        Ok(NonpayablePort {
                            inner: mixed_native::SimulationNativeExecutionPort::new(
                                chain
                                    .begin_native_claim_all_nonpayable(
                                        selected.period,
                                        selected.period,
                                    )
                                    .unwrap(),
                            ),
                            seen: seen.clone(),
                            outcomes: outcomes.clone(),
                            drops: drops.clone(),
                            fault: 0,
                        })
                    },
                    &super::metadata_trace::block(1),
                    &transaction,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                if fail {
                    assert!(
                        matches!(result,Err(SimulationError::NativeSession(NativePortError::Infrastructure(m))) if m=="nonpayable factory fault")
                    );
                    assert_eq!(drops.get(), 0);
                } else {
                    let result = result.unwrap();
                    assert_eq!(result.state, identity);
                    assert_nonpayable_execution(result.execution, row);
                    assert_eq!(drops.get(), 1);
                }
                assert_eq!(transaction, original);
                assert_eq!(concrete_rows(&concrete), source_rows);
                assert_eq!(owner_rows(&app), rows);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner
                );
            }
            let factories = Cell::new(0);
            let mismatch = simulate_with_native(
                &reader,
                &NoHistory,
                &Dpos,
                &Dpos,
                |_| {
                    factories.set(factories.get() + 1);
                    Ok(mixed_native::SimulationNativeExecutionPort::new(
                        chain
                            .begin_native_claim_all_nonpayable(1.into(), 1.into())
                            .unwrap(),
                    ))
                },
                &super::metadata_trace::block(2),
                &transaction,
                EnvelopeRules { cornus: true },
                TaraxaProfile::for_phase(TaraxaPhase::Ficus),
            );
            assert!(matches!(
                mismatch,
                Err(SimulationError::StatePeriodMismatch { .. })
            ));
            assert_eq!(factories.get(), 0);
        }
    }
    assert_eq!(attempts, 42);
}

/// Out-of-profile value affordability remains an ordinary consensus rejection
/// before quote, and cannot become the authenticated nonpayable failure.
#[test]
fn nonpayable_claim_all_keeps_transfer_affordability_before_quote() {
    use rustaxa_evm::{
        driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
        journal::ExecutionJournal,
    };
    let public = nonpayable_corpus();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let concrete = FixturePath::new("nonpayable-affordability-source");
    let identity = materialize(&public, &concrete);
    let source_rows = concrete_rows(&concrete);
    let app = FixturePath::new("nonpayable-affordability-owner");
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows = owner_rows(&app);
        let mut transaction = nonpayable_tx(&public["cases"][10], addr(&public["caller"]));
        transaction.value = ExecutionValue::new(3001u32.into());
        let original = transaction.clone();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let outcomes = Rc::new(RefCell::new(Vec::new()));
        let drops = Rc::new(Cell::new(0));
        let mut port = NonpayablePort {
            inner: mixed_native::SimulationNativeExecutionPort::new(
                chain
                    .begin_native_claim_all_nonpayable(2.into(), identity.period)
                    .unwrap(),
            ),
            seen: seen.clone(),
            outcomes: outcomes.clone(),
            drops: drops.clone(),
            fault: 0,
        };
        let mut journal =
            ExecutionJournal::new(CompleteSeedReader::open(&concrete, &public, identity));
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &Dpos,
            &Dpos,
            &mut port,
            &mut PeriodConsensusSequence::new(2.into()),
            &super::metadata_trace::block(2),
            &transaction,
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(TaraxaPhase::Ficus),
        )
        .unwrap();
        let TransactionExecutionResult::ConsensusFailure(out) = result else {
            panic!("value affordability consensus lane")
        };
        assert_eq!(
            out.error,
            rustaxa_evm::contracts::ConsensusFailure::InsufficientBalanceForTransfer
        );
        assert!(seen.borrow().is_empty() && outcomes.borrow().is_empty());
        assert_eq!(
            journal
                .account(transaction.sender)
                .unwrap()
                .balance
                .value()
                .to_string(),
            "3000"
        );
        assert_eq!(
            journal
                .account(address(0xfe))
                .unwrap()
                .balance
                .value()
                .to_string(),
            "4800"
        );
        assert!(
            journal
                .settle_transaction()
                .unwrap()
                .native_invocations
                .is_empty()
        );
        drop(port);
        assert_eq!(drops.get(), 1);
        assert_eq!(transaction, original);
        assert_eq!(concrete_rows(&concrete), source_rows);
        assert_eq!(owner_rows(&app), rows);
        assert_eq!(
            super::redelegate_rewarded_partial::committed(&chain, &seed),
            owner
        );
    }
}

/// Actual accepted H1 responses and unchanged C++ estimation both terminate on
/// the first failed probe. Every callback uses a fresh disposable port and
/// retains the original wide request. Infrastructure errors stop with their
/// typed callback lane and have immediate clean retries over the same owners.
#[test]
fn nonpayable_claim_all_h1_estimates_stop_at_actual_initial_failure() {
    use rustaxa_evm::estimate::{EstimateError, EstimateProbe, estimate_gas};
    let public = nonpayable_corpus();
    let cpp:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_nonpayable_estimate/cpp.json")).unwrap();
    assert_eq!(
        cpp["algorithm_sha256"],
        "7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759"
    );
    assert_eq!(cpp["cases"].as_array().unwrap().len(), 7);
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let concrete = FixturePath::new("nonpayable-estimate-source");
    let identity = materialize(&public, &concrete);
    let source_rows = concrete_rows(&concrete);
    let app = FixturePath::new("nonpayable-estimate-owner");
    let mut baseline = None;
    let mut attempts = 0;
    let mut callback_errors = 0;
    let mut allocated = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner = super::redelegate_rewarded_partial::committed(&chain, &seed);
        let rows = owner_rows(&app);
        if let Some(old) = &baseline {
            assert_eq!(old, &rows)
        } else {
            baseline = Some(rows.clone())
        }
        associate_nonpayable_p1(
            &chain,
            &CompleteSeedReader::open(&concrete, &public, identity),
            &public,
            &seed,
        );
        for (index, row) in public["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["period"] == 1)
            .enumerate()
        {
            let cap = row["gas"].as_u64().unwrap();
            let admitted = cap >= 21272;
            let reference = &cpp["cases"][index];
            assert_eq!(reference["name"], format!("claim_all_H1_cap{cap}"));
            assert_eq!(reference["consumed"], 1);
            assert_eq!(
                reference["error"].as_str().unwrap(),
                if admitted {
                    row["execution_error"].as_str().unwrap()
                } else {
                    row["consensus_error"].as_str().unwrap()
                }
            );
            let prototype = nonpayable_tx(row, addr(&public["caller"]));
            let original = prototype.clone();
            for fault in [0, 1, 0, 2, 0, 3, 0, 4, 0] {
                attempts += 1;
                let mut candidates = Vec::new();
                let drops = Rc::new(Cell::new(0));
                let factories = Cell::new(0);
                let seen = Rc::new(RefCell::new(Vec::new()));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let source = NonpayableSource {
                    reader: CompleteSeedReader::open(&concrete, &public, identity),
                    fail: fault == 3,
                };
                let estimate = estimate_gas(cap, |candidate| {
                    candidates.push(candidate);
                    assert_eq!(candidate, cap, "no midpoint after initial error");
                    let mut request = prototype.clone();
                    request.gas_limit = candidate.into();
                    let original_probe = request.clone();
                    let result = simulate_with_native(
                        &source,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected| {
                            factories.set(factories.get() + 1);
                            assert_eq!(selected, identity);
                            if fault == 4 {
                                return Err(NativePortError::Infrastructure(
                                    "nonpayable estimate factory fault".into(),
                                ));
                            }
                            Ok(NonpayablePort {
                                inner: mixed_native::SimulationNativeExecutionPort::new(
                                    chain
                                        .begin_native_claim_all_nonpayable(
                                            selected.period,
                                            selected.period,
                                        )
                                        .unwrap(),
                                ),
                                seen: seen.clone(),
                                outcomes: outcomes.clone(),
                                drops: drops.clone(),
                                fault,
                            })
                        },
                        &super::metadata_trace::block(1),
                        &request,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    );
                    assert_eq!(request, original_probe);
                    let result = result?;
                    assert_eq!(result.state, identity);
                    assert_nonpayable_execution(result.execution.clone(), row);
                    Ok(match result.execution {
                        TransactionExecutionResult::ConsensusFailure(_) => {
                            EstimateProbe::ConsensusFailure {
                                error: row["consensus_error"].as_str().unwrap().into(),
                            }
                        }
                        TransactionExecutionResult::Executed(out) => {
                            let CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) =
                                out.status
                            else {
                                panic!("native nonpayable initial estimate")
                            };
                            EstimateProbe::CodeFailure { error: e.error }
                        }
                    })
                });
                assert_eq!(candidates, vec![cap]);
                assert_eq!(prototype, original);
                if matches!(fault, 3 | 4) || (admitted && matches!(fault, 1 | 2)) {
                    callback_errors += 1;
                    let Err(EstimateError::Callback(error)) = estimate else {
                        panic!("typed estimate callback {estimate:?}")
                    };
                    match (fault, error) {
                        (
                            3,
                            SimulationError::Sender(rustaxa_evm::journal::JournalError::State(
                                ConcreteReadError::Io(m),
                            )),
                        ) => assert_eq!(m, "nonpayable source account fault"),
                        (4, SimulationError::NativeSession(NativePortError::Infrastructure(m))) => {
                            assert_eq!(m, "nonpayable estimate factory fault")
                        }
                        (
                            1 | 2,
                            SimulationError::Execution(
                                rustaxa_evm::driver::ExecutionDriverError::Native(
                                    rustaxa_evm::native::NativeAdapterError::Port(
                                        NativePortError::Infrastructure(m),
                                    ),
                                ),
                            ),
                        ) => assert_eq!(
                            m,
                            if fault == 1 {
                                "nonpayable prepare fault"
                            } else {
                                "nonpayable invoke fault"
                            }
                        ),
                        (_, error) => panic!("wrong estimate callback owner {error:?}"),
                    }
                    assert!(outcomes.borrow().is_empty());
                } else {
                    let expected = reference["error"].as_str().unwrap().to_owned();
                    assert_eq!(
                        estimate,
                        Err(if admitted {
                            EstimateError::CodeFailure { error: expected }
                        } else {
                            EstimateError::ConsensusFailure { error: expected }
                        })
                    );
                    assert_eq!(seen.borrow().len(), usize::from(admitted));
                    assert_eq!(outcomes.borrow().len(), usize::from(admitted));
                }
                let expected_drop = usize::from(!matches!(fault, 3 | 4));
                assert_eq!(drops.get(), expected_drop);
                assert_eq!(factories.get(), usize::from(fault != 3));
                allocated += expected_drop;
                assert_eq!(concrete_rows(&concrete), source_rows);
                assert_eq!(owner_rows(&app), rows);
                assert_eq!(
                    super::redelegate_rewarded_partial::committed(&chain, &seed),
                    owner
                );
            }
        }
    }
    assert_eq!(attempts, 126);
    assert_eq!(callback_errors, 52);
    assert_eq!(allocated, 98);
}
