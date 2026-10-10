//! Exact P0 ordinary-prefix trace parity and disposable owner preservation.
use super::metadata_trace::requests;
use super::owner_snapshot::owner_rows;
use super::reward_claim_all_h1_trace::{Port, balances, logs, raw};
use super::*;
use rustaxa_evm::contracts::{
    NativeCallKind, NativeInvocationResult, NativeJournalReadError, NativePortError, NativeStatus,
};
use rustaxa_evm::trace_runner::{
    StructuredTraceRunnerError, TraceSequenceStage, run_structured_claim_all_prefix_trace,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
fn h1_block() -> ExecutionBlockContext {
    let mut block = super::metadata_trace::block(1);
    block.timestamp = 1_700_000_001;
    block
}

#[test]
fn claim_all_h1_p0_prefix_trace_matches_go_and_discards_every_attempt() {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_prefix_trace/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_prefix_trace/local.json")).unwrap();
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
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    let source = json!({"state_before": public["p0_source_before"]});
    let concrete = FixturePath::new("claim-all-trace-concrete");
    let identity = materialize(&source, &concrete);
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
        let reader = CompleteSeedReader::open(&concrete, &source, identity);
        for case in public["cases"].as_array().unwrap() {
            let prefix = requests(&case["prefix"], 0);
            let targets = requests(&case["targets"], prefix.len());
            assert_eq!(prefix.len(), 1);
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
                let result = run_structured_claim_all_prefix_trace(
                    &reader,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |proof| {
                        let selected = proof.state();
                        let execution = proof.block().period;
                        assert_eq!(proof.target(), &targets[0]);
                        assert_eq!(selected, identity);
                        assert_eq!(execution, 1.into());
                        assert_eq!(selected.period, 0.into());
                        Ok(Port {
                            inner: mixed_native::SimulationNativeExecutionPort::new(
                                chain
                                    .begin_native_trace_claim_all_ordinary_prefix(
                                        execution,
                                        selected.period,
                                        proof.target().sender,
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
                    &h1_block(),
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
                assert_eq!(i.period, 1.into());
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
                assert_eq!(
                    actual, expected_reads,
                    "count/items then scalar serialization authority"
                );
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
    assert_eq!(sessions, 168);
}

/// All exclusions run against disposable reads, then retry the same accepted
/// request. No rejected shape or failed prefix can reach the token factory.
struct FaultReader<'a> {
    inner: &'a CompleteSeedReader,
    caller: [u8; 20],
    mode: u8,
}
impl ConcreteStateRead for FaultReader<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        let mut id = self.inner.identity();
        if self.mode == 1 {
            id.period = 1.into();
        }
        if self.mode == 2 {
            id.state_root[0] ^= 1;
        }
        id
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if address == self.caller {
            if self.mode == 3 {
                return Err(ConcreteReadError::Io("prefix source fault".into()));
            }
            if self.mode == 4 {
                return Ok(ConcreteRead::Absent);
            }
            let mut row = self.inner.account(address)?;
            if let ConcreteRead::Present(record) = &mut row {
                if self.mode == 5 {
                    record.account.nonce = FinalChainNonce::from_bytes(
                        &((BigUint::from(1u8) << 512usize) + 1u8).to_bytes_be(),
                    )
                    .unwrap();
                }
                if self.mode == 6 {
                    record.account.code_size = 1;
                    record.account.code_hash = None;
                }
                if self.mode == 7 {
                    record.account.code_size = 1;
                    record.account.code_hash = Some(keccak256([0]).0);
                }
            }
            return Ok(row);
        }
        self.inner.account(address)
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.inner.storage(address, key)
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        if self.mode == 7 && hash == keccak256([0]).0 {
            return Ok(ConcreteRead::Present(vec![0]));
        }
        self.inner.code(hash)
    }
}
struct Classification {
    extra: Option<[u8; 20]>,
    dpos: bool,
}
impl rustaxa_evm::driver::NativeAddressClassifier for Classification {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.extra == Some(address) || (self.dpos && address == super::address(0xfe))
    }
}
#[test]
fn claim_all_h1_p0_prefix_trace_rejects_shapes_and_preserves_sources() {
    let oracle:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_prefix_trace/public.json")).unwrap();
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    let source = json!({"state_before":oracle["p0_source_before"]});
    let concrete = FixturePath::new("claim-all-prefix-exclusions");
    let identity = materialize(&source, &concrete);
    let concrete_before = concrete_rows(&concrete);
    let app = FixturePath::new("claim-all-prefix-exclusion-owner");
    let chain = super::redelegate_rewarded_partial::history(&app, true, &seed);
    let owner = owner_rows(&app);
    let reader = CompleteSeedReader::open(&concrete, &source, identity);
    let case = &oracle["cases"][0];
    let accepted_prefix = requests(&case["prefix"], 0);
    let accepted_targets = requests(&case["targets"], 1);
    for fault in 1..=39 {
        for selected_fault in [fault, 0] {
            let mut prefix = accepted_prefix.clone();
            let mut targets = accepted_targets.clone();
            let mut block = h1_block();
            let mut rules = EnvelopeRules { cornus: true };
            let mut profile = TaraxaProfile::for_phase(TaraxaPhase::Ficus);
            let mut all = Classification {
                extra: None,
                dpos: true,
            };
            let mut consensus = Classification {
                extra: None,
                dpos: true,
            };
            let mut reader_mode = 0;
            match selected_fault {
                0 => (),
                1 => prefix.clear(),
                2 => prefix.push(prefix[0].clone()),
                3 => targets.clear(),
                4 => targets.push(targets[0].clone()),
                5 => prefix[0].kind = ExecutionTransactionKind::Create,
                6 => prefix[0].position = 1.into(),
                7 => prefix[0].receiver = Some(address(0xa1)),
                8 => prefix[0].gas_limit = 20999.into(),
                9 => prefix[0].gas_limit = 21001.into(),
                10 => prefix[0].gas_price = ExecutionGasPrice::new(1u8.into()),
                11 => prefix[0].value = ExecutionValue::new(1u8.into()),
                12 => prefix[0].input.push(0),
                13 => prefix[0].nonce = FinalChainNonce::zero(),
                14 => targets[0].nonce = FinalChainNonce::zero(),
                15 => targets[0].sender = address(0xa1),
                16 => targets[0].receiver = Some(address(0xa1)),
                17 => targets[0].gas_limit = 199999.into(),
                18 => targets[0].gas_limit = 200001.into(),
                19 => targets[0].gas_price = ExecutionGasPrice::new(1u8.into()),
                20 => targets[0].value = ExecutionValue::new(1u8.into()),
                21 => targets[0].input.push(0),
                22 => targets[0].kind = ExecutionTransactionKind::Create,
                23 => targets[0].position = 0.into(),
                24 => targets[0].position = 2.into(),
                25 => all.extra = Some(prefix[0].sender),
                26 => consensus.extra = Some(prefix[0].sender),
                27 => consensus.dpos = false,
                28 => rules.cornus = false,
                29 => profile = TaraxaProfile::for_phase(TaraxaPhase::Cacti),
                30..=36 => reader_mode = selected_fault - 29,
                37 => block.period = 2.into(),
                38 => (), // Factory infrastructure fault after the valid prefix.
                39 => profile = TaraxaProfile::for_phase(TaraxaPhase::Californicum),
                _ => unreachable!(),
            }
            // Every mutation is checked against these exact supplied inputs.
            let original_prefix = prefix.clone();
            let original_targets = targets.clone();
            let calls = Cell::new(0);
            let result = run_structured_claim_all_prefix_trace(
                &FaultReader {
                    inner: &reader,
                    caller: accepted_prefix[0].sender,
                    mode: reader_mode,
                },
                &NoHistory,
                &all,
                &consensus,
                |proof| {
                    calls.set(calls.get() + 1);
                    if selected_fault == 38 {
                        return Err(NativePortError::Infrastructure(
                            "prefix factory fault".into(),
                        ));
                    }
                    if proof.state() != identity {
                        return Err(NativePortError::Infrastructure(
                            "prefix identity binding".into(),
                        ));
                    }
                    assert_eq!(proof.block(), &block);
                    assert_eq!(proof.target(), &accepted_targets[0]);
                    Ok(mixed_native::SimulationNativeExecutionPort::new(
                        chain
                            .begin_native_trace_claim_all_ordinary_prefix(
                                proof.block().period,
                                proof.state().period,
                                proof.target().sender,
                            )
                            .unwrap(),
                    ))
                },
                &block,
                &prefix,
                &targets,
                rules,
                profile,
            );
            if selected_fault == 0 {
                assert_eq!(
                    serde_json::from_slice::<Value>(&result.unwrap().output).unwrap(),
                    case["result"]
                );
                assert_eq!(calls.get(), 1);
            } else {
                assert!(result.is_err(), "fault {selected_fault}");
                assert_eq!(
                    calls.get(),
                    if reader_mode == 2 || selected_fault == 38 {
                        1
                    } else {
                        0
                    },
                    "factory reached by fault {selected_fault}"
                );
                if selected_fault <= 29 || selected_fault == 39 || matches!(reader_mode, 4 | 5 | 7)
                {
                    assert_eq!(result, Err(StructuredTraceRunnerError::ClaimAllPrefixScope));
                }
            }
            assert_eq!(prefix, original_prefix);
            assert_eq!(targets, original_targets);
            assert_eq!(concrete_rows(&concrete), concrete_before);
            assert_eq!(owner_rows(&app), owner);
        }
    }
}
