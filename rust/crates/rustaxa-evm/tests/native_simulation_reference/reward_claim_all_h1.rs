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
