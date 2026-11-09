//! Historical API parity over authenticated complete mainnet-policy histories.
use super::*;
use native_integration::api::{CompleteReader, Port};
use rustaxa_evm::{
    contracts::{
        NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
        NativeJournalRead, NativePortError,
    },
    estimate::{EstimateError, EstimateProbe, estimate_gas},
    simulation::simulate_with_native,
    trace_runner::run_structured_trace_with_native,
};
use std::collections::BTreeMap;
use std::rc::Rc;
type NativeSlotValues = BTreeMap<([u8; 20], [u8; 32]), Vec<u8>>;

fn native_slots(fork: &Value) -> Result<NativeSlotValues> {
    let rows = fork["pre_reward"]["native_catalog"]["slots"]
        .as_array()
        .context("complete private native catalog")?;
    let values = rows
        .iter()
        .map(|row| {
            (
                (fixed(&row["address"]), fixed(&row["key"])),
                if row["present"] == true {
                    bytes(&row["value"])
                } else {
                    Vec::new()
                },
            )
        })
        .collect::<NativeSlotValues>();
    ensure!(values.len() == rows.len(), "duplicate private native key");
    Ok(values)
}

pub(super) fn copy_owned_application(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "synthetic application copy rejects symlinks"
        );
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_owned_application(&entry.path(), &target)?;
        } else {
            ensure!(kind.is_file());
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

struct PrivateEmptyPhase<'a> {
    parent: &'a CompleteReader<ConcreteStateReader>,
    phase: ConcreteStateIdentity,
}
impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead for PrivateEmptyPhase<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.phase
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        ConcreteStateRead::account(self.parent, address)
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteStateRead::storage(self.parent, address, key)
    }
    fn code(
        &self,
        hash: [u8; 32],
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteStateRead::code(self.parent, hash)
    }
}

pub(super) fn verify(fixture: &Value, application: &Path, concrete: &Path) -> Result<()> {
    verify_cases(
        fixture,
        fixture["api_cases"].as_array().context("API cases")?,
        None,
        application,
        concrete,
    )
}

pub(super) fn verify_committed_phase(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
    phase: &Value,
) -> Result<()> {
    let reader = CompleteReader {
        inner: ConcreteStateReader::open_historical_read_only(path, committed, committed)?,
        known: complete_inventory(fixture, committed.period.as_u64())?,
    };
    ensure!(
        committed.state_root == fixed(&phase["root"]),
        "actual committed phase root"
    );
    verify_complete_final_reader(&reader, phase)
}

fn pre_reward_binding(fixture: &Value, fork: &Value) -> Result<(u64, ConcreteStateIdentity)> {
    let period = number(&fork["pre_reward"]["number"]);
    ensure!(
        fixture["profile"] == "synthetic_mainnet_era"
            && fixture["scenario"] == "mainnet_106"
            && fixture["configuration"]["dpos"]["delegation_delay"] == 5
            && fork["kind"] == "pre_reward_fork"
            && fork["scenario"] == fixture["scenario"]
            && fork["head"] == fixture["head"],
        "private fork source/scenario binding"
    );
    ensure!(
        [25_706_900, 25_707_000].contains(&period),
        "bounded private fork"
    );
    let row = fixture["periods"]
        .as_array()
        .context("periods")?
        .iter()
        .find(|r| number(&r["number"]) == period)
        .context("fork period")?;
    ensure!(
        row["transactions"]
            .as_array()
            .context("fork tx")?
            .is_empty()
            && row["ordered_raw_writes"]["begin_block"]
                .as_array()
                .context("fork BeginBlock")?
                .is_empty(),
        "nonempty prefix is unsupported"
    );
    let parent = selected_state(fixture, period - 1)?;
    ensure!(
        parent.state_root == fixed(&fork["pre_reward"]["root"]),
        "independently computed parent/private root"
    );
    let prefix = fixture["periods"].as_array().unwrap().iter().filter(|r| number(&r["number"]) < period)
        .map(|r| serde_json::json!({"period":r["number"],"prior_root":r["prior_root"],"root":r["final"]["root"],
            "transaction_hashes":r["transactions"].as_array().unwrap().iter().map(|tx| tx["hash"].clone()).collect::<Vec<_>>()})).collect::<Vec<_>>();
    ensure!(
        fork["prefix_period_identities"] == Value::Array(prefix),
        "complete authenticated fork prefix"
    );
    let cases = fork["api_cases"].as_array().context("fork cases")?;
    ensure!(
        cases.len() == 7 && cases.iter().all(|case| number(&case["head"]) == period),
        "private case head binding"
    );
    Ok((period, parent))
}

pub(super) fn verify_pre_reward(
    fixture: &Value,
    fork: &Value,
    application: &Path,
    concrete: &Path,
) -> Result<()> {
    let (period, parent) = pre_reward_binding(fixture, fork)?;
    let (storage, chain) = open_chain(application, fixture)?;
    let committed =
        ConcreteStateLifecycle::inspect_existing(concrete, chain.concrete_chain_identity()?)?
            .committed;
    let reader = CompleteReader {
        inner: ConcreteStateReader::open_historical_read_only(concrete, committed, parent)?,
        known: complete_inventory(fixture, period - 1)?,
    };
    verify_complete_final_reader(&reader, &fork["pre_reward"])?;
    // The native constructor also validates every canonical snapshot row after
    // graph advancement; query eligibility is independently selected at p-5.
    drop(reader);
    drop(chain);
    drop(storage);
    verify_cases(
        fixture,
        fork["api_cases"].as_array().context("fork API cases")?,
        Some(fork),
        application,
        concrete,
    )
}

pub(super) fn verify_pre_reward_negatives(
    fixture: &Value,
    fork: &Value,
    application: &Path,
    concrete: &Path,
) -> Result<()> {
    let (period, _) = pre_reward_binding(fixture, fork)?;
    for case in 0..5 {
        let mut wrong = fork.clone();
        match case {
            0 => wrong["pre_reward"]["root"] = Value::String("00".repeat(32)),
            1 => wrong["head"] = Value::from(1),
            2 => wrong["pre_reward"]["number"] = Value::from(period + 1),
            3 => wrong["prefix_period_identities"]
                .as_array_mut()
                .unwrap()
                .clear(),
            4 => wrong["api_cases"][0]["head"] = Value::from(period - 1),
            _ => unreachable!(),
        }
        ensure!(
            pre_reward_binding(fixture, &wrong).is_err(),
            "private binding negative {case}"
        );
    }
    for phase in ["transactions", "begin_block"] {
        let mut wrong = fixture.clone();
        let row = wrong["periods"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| number(&r["number"]) == period)
            .unwrap();
        if phase == "transactions" {
            row[phase].as_array_mut().unwrap().push(Value::Null);
        } else {
            row["ordered_raw_writes"][phase]
                .as_array_mut()
                .unwrap()
                .push(Value::Null);
        }
        ensure!(
            pre_reward_binding(&wrong, fork).is_err(),
            "nonempty private phase {phase}"
        );
    }
    for case in 0..3 {
        let mut wrong = fork.clone();
        let slots = wrong["pre_reward"]["native_catalog"]["slots"]
            .as_array_mut()
            .unwrap();
        match case {
            0 => {
                slots.push(slots[0].clone());
            }
            1 => {
                let row = slots.iter_mut().find(|r| r["present"] == true).unwrap();
                row["value"] = Value::String("ff".into());
            }
            2 => {
                let index = slots
                    .iter()
                    .position(|r| {
                        r["key"]
                            == "ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc"
                    })
                    .context("required canonical supply row")?;
                slots.remove(index);
            }
            _ => unreachable!(),
        }
        ensure!(
            verify_pre_reward(fixture, &wrong, application, concrete).is_err(),
            "private catalog negative {case}"
        );
    }
    if period == 25_707_000 {
        let mut wrong = fork.clone();
        let row = wrong["pre_reward"]["native_catalog"]["slots"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| {
                row["key"] == "6c5a165a9d1a52b89db4a526ea04ad523db116755ace7c4268cb2ddcbed51d12"
            })
            .context("retained physical V1 zero-count row")?;
        ensure!(
            row["present"] == true && row["value"] == "00000000",
            "exact retained zero-count provenance"
        );
        row["value"] = Value::String("01000000".into());
        ensure!(
            verify_pre_reward(fixture, &wrong, application, concrete).is_err(),
            "corrupt retained physical extra row"
        );
    }
    let (_storage, chain) = open_chain(application, fixture)?;
    ensure!(
        chain
            .begin_synthetic_empty_pre_reward_simulation(
                (period - 2).into(),
                period.into(),
                &BTreeMap::new()
            )
            .is_err(),
        "wrong private parent"
    );
    ensure!(
        chain
            .begin_synthetic_empty_pre_reward_simulation(
                period.into(),
                (period + 1).into(),
                &BTreeMap::new()
            )
            .is_err(),
        "wrong private stop"
    );
    let before =
        ConcreteStateLifecycle::inspect_existing(concrete, chain.concrete_chain_identity()?)?;
    let parent = selected_state(fixture, period - 1)?;
    let reader = CompleteReader {
        inner: ConcreteStateReader::open_historical_read_only(concrete, before.committed, parent)?,
        known: complete_inventory(fixture, period - 1)?,
    };
    for substituted in [period - 1, period - 6] {
        use rustaxa_evm::{
            driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
            journal::ExecutionJournal,
        };
        let session = chain
            .begin_native_simulation(substituted.into())
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let mut port = Port {
            inner: mixed_native::SimulationNativeExecutionPort::new(session),
            outcomes: Rc::new(RefCell::new(Vec::new())),
        };
        let phase = ConcreteStateIdentity {
            period: period.into(),
            state_root: parent.state_root,
        };
        let mut journal = ExecutionJournal::new(PrivateEmptyPhase {
            parent: &reader,
            phase,
        });
        let mut request =
            native_integration::api::request(&fork["api_cases"][1]["request"], 500_000, 0);
        request.nonce = journal.account(request.sender)?.nonce.next();
        let mut sequence = PeriodConsensusSequence::new(period.into());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &ChainHashes(&chain),
            &AllNatives(profile_at(period, fixture), &chain),
            &ConsensusNatives(&chain),
            &mut port,
            &mut sequence,
            &block(period, fixture)?,
            &request,
            EnvelopeRules { cornus: true },
            profile_at(period, fixture),
        );
        let error =
            result.expect_err("committed live/p-6 session cannot substitute a private phase");
        ensure!(
            format!("{error:?}").contains("PeriodMismatch"),
            "exact substitution identity rejection"
        );
    }
    drop(reader);
    let delayed_raw = _storage
        .final_chain()
        .dpos_snapshot_raw(period - 5)?
        .context("real delayed snapshot")?;
    drop(chain);
    drop(_storage);
    let mut delayed_cases = Vec::new();
    for corrupt in [false, true] {
        let copy = application
            .parent()
            .context("owned task pair")?
            .join(format!(
                "private-delay-negative-{period}-{corrupt}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
        let mut scratch = era_retention::register(&copy, fixture)?;
        copy_owned_application(application, &copy)?;
        era_retention::created(&mut scratch)?;
        let storage = Storage::new(rustaxa_storage::Config::new(copy.clone()))?;
        let mut key = b"rustaxa:dpos_snapshot:".to_vec();
        key.extend_from_slice(&(period - 5).to_le_bytes());
        ensure!(
            storage
                .get_raw(rustaxa_storage::Column::FinalChainMeta, &key)?
                .as_deref()
                == Some(delayed_raw.as_slice()),
            "exact delayed corruption target"
        );
        let mut batch = storage.create_write_batch();
        if corrupt {
            storage.batch_put_raw(
                &mut batch,
                rustaxa_storage::Column::FinalChainMeta,
                &key,
                &[0xff],
            )?;
        } else {
            storage.batch_delete_raw(&mut batch, rustaxa_storage::Column::FinalChainMeta, &key)?;
        }
        storage.commit_write_batch_with_sync(batch, true)?;
        ensure!(
            storage.final_chain().dpos_snapshot_raw(period - 5)?
                == if corrupt { Some(vec![0xff]) } else { None },
            "fault mutation durably reached"
        );
        drop(storage);
        let attempt = (|| -> Result<()> {
            let (_storage, chain) = open_chain(&copy, fixture)?;
            chain
                .begin_synthetic_empty_pre_reward_simulation(
                    (period - 1).into(),
                    period.into(),
                    &native_slots(fork)?,
                )
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            Ok(())
        })();
        ensure!(attempt.is_err(), "missing/corrupt delayed state must fail");
        let error = attempt.as_ref().unwrap_err();
        ensure!(
            if corrupt {
                error
                    .chain()
                    .any(|cause| cause.downcast_ref::<rlp::DecoderError>().is_some())
                    || format!("{error:#}").starts_with("DPoS snapshot RLP must contain exactly ")
            } else {
                format!("{error:#}")
                    .contains(&format!("DPoS snapshot for finalized block {}", period - 5))
            },
            "specific delayed-state rejection, got {error:#}"
        );
        delayed_cases.push((scratch, serde_json::json!({"mutation":{"column":"FinalChainMeta","key":hex::encode(key),"old_bytes":hex::encode(&delayed_raw),"replacement":if corrupt {Some("ff")}else{None}},"actual_error":format!("{:#}",attempt.unwrap_err()),"period":period,"root":hex::encode(parent.state_root)})));
    }
    let (storage, _chain) = open_chain(application, fixture)?;
    ensure!(
        storage.final_chain().dpos_snapshot_raw(period - 5)? == Some(delayed_raw),
        "private negatives leave original delayed state unchanged"
    );
    ensure!(
        ConcreteStateLifecycle::inspect_existing(concrete, _chain.concrete_chain_identity()?)?
            == before,
        "private negatives leave concrete provenance/catalog/generation unchanged"
    );
    drop(_chain);
    drop(storage);
    for (scratch, mut result) in delayed_cases {
        result["original_snapshot_and_concrete_provenance_catalog_generation_unchanged"] =
            Value::Bool(true);
        result["all_handles_closed"] = Value::Bool(true);
        era_retention::finish(scratch, result, false)?;
    }
    eprintln!(
        "private prepared negatives:{} binding/prefix/catalog/parent/stop/substitution/delayed-history cases at {period}",
        if period == 25_707_000 { 17 } else { 16 }
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pre_reward_call(
    reader: &CompleteReader<ConcreteStateReader>,
    chain: &FinalChain,
    fixture: &Value,
    fork: &Value,
    case: &Value,
    probe: &Value,
    mut request: rustaxa_evm::contracts::ExecutionTransaction,
    outcomes: Rc<RefCell<Vec<NativeOutcome>>>,
) -> Result<EstimateProbe> {
    use rustaxa_evm::{
        driver::{PeriodConsensusSequence, execute_top_level_call_with_native},
        journal::ExecutionJournal,
    };
    let period = number(&case["head"]);
    let slots = native_slots(fork)?;
    let session = chain
        .begin_synthetic_empty_pre_reward_simulation((period - 1).into(), period.into(), &slots)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let mut port = Port {
        inner: mixed_native::SimulationNativeExecutionPort::new(session),
        outcomes: outcomes.clone(),
    };
    // This is a disposable driver journal over the authenticated concrete
    // parent. It does not implement a committed reader for the private p view.
    let phase = ConcreteStateIdentity {
        period: period.into(),
        state_root: fixed(&fork["pre_reward"]["root"]),
    };
    ensure!(
        reader.identity().period.as_u64() + 1 == period
            && reader.identity().state_root == phase.state_root,
        "private phase parent binding"
    );
    let mut journal = ExecutionJournal::new(PrivateEmptyPhase {
        parent: reader,
        phase,
    });
    request.nonce = journal.account(request.sender)?.nonce.next();
    let mut sequence = PeriodConsensusSequence::new(period.into());
    let execution = execute_top_level_call_with_native(
        &mut journal,
        &ChainHashes(chain),
        &AllNatives(profile_at(period, fixture), chain),
        &ConsensusNatives(chain),
        &mut port,
        &mut sequence,
        &block(period, fixture)?,
        &request,
        EnvelopeRules { cornus: true },
        profile_at(period, fixture),
    )
    .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    native_integration::api::compare_raw(&outcomes.borrow(), &probe["ordered_raw_writes"])?;
    native_integration::api::compare(&execution, &probe["output"])
}

pub(super) fn verify_cpp_estimates(document: &Value, cpp: &Value, source: &str) -> Result<()> {
    let rows = cpp["cases"].as_array().context("actual C++ estimates")?;
    let selected = rows
        .iter()
        .filter(|row| row["source"] == source)
        .collect::<Vec<_>>();
    let mut count = 0;
    for case in document["api_cases"].as_array().context("API cases")? {
        for estimate in case["estimates"].as_array().context("estimates")? {
            // Equal caps can deliberately occur twice (e.g. a failing call
            // consumes its entire cap). Authenticate every ordered occurrence.
            let cpp = selected
                .get(count)
                .context("complete C++ estimate occurrences")?;
            ensure!(
                cpp["name"] == estimate["name"],
                "ordered C++ estimate identity"
            );
            let probes = estimate["probes"].as_array().context("probes")?;
            ensure!(
                number(&cpp["consumed"]) as usize == probes.len(),
                "C++ consumes every probe"
            );
            match expected_estimate(probes, number(&estimate["cap"])) {
                Ok(gas) => ensure!(
                    cpp["result"] == gas && cpp.get("error").is_none(),
                    "C++ result"
                ),
                Err(error) => ensure!(
                    cpp["error"] == error && cpp.get("result").is_none(),
                    "C++ error"
                ),
            }
            count += 1;
        }
    }
    ensure!(
        count > 0 && count == selected.len(),
        "nonzero complete C++ cases"
    );
    Ok(())
}

fn selected_state(fixture: &Value, period: u64) -> Result<ConcreteStateIdentity> {
    let state = fixture["periods"]
        .as_array()
        .context("period history")?
        .iter()
        .find(|row| number(&row["number"]) == period)
        .context("selected committed period")?;
    Ok(ConcreteStateIdentity {
        period: period.into(),
        state_root: fixed(&state["final"]["root"]),
    })
}

pub(super) fn complete_inventory(fixture: &Value, period: u64) -> Result<BTreeSet<[u8; 32]>> {
    let mut known = BTreeSet::new();
    for key in fixture["history_rows"][4]
        .as_object()
        .context("complete unpruned physical history")?
        .keys()
    {
        let key = hex::decode(key)?;
        ensure!(key.len() == 40, "physical history key width");
        if u64::from_be_bytes(key[32..].try_into()?) <= period {
            known.insert(key[..32].try_into()?);
        }
    }
    Ok(known)
}

fn block(period: u64, fixture: &Value) -> Result<ExecutionBlockContext> {
    Ok(ExecutionBlockContext {
        period: period.into(),
        author: fixed(&fixture["periods"][0]["reward_input"]["block_author"]),
        timestamp: 0,
        gas_limit: duration_active_rules(fixture, period)?
            .map_or_else(
                || number(&fixture["configuration"]["block_gas_limit"]),
                |rules| rules.0,
            )
            .into(),
        chain_id: 841,
        difficulty: BigUint::default(),
    })
}

#[test]
fn duration_call_and_trace_use_actual_historical_rules() {
    let fixture = serde_json::json!({"profile":"synthetic_mainnet_era", "scenario":"magnolia_duration",
        "configuration":{"block_gas_limit":2100000000_u64,"hardforks":{"cornus":{"block":15610000_u64,"pbft_gas_limit":2100000000_u64}}},
        "periods":[{"reward_input":{"block_author":"0000000000000000000000000000000000000000"}}]});
    for period in [5_730_001, 5_730_002] {
        let context = block(period, &fixture).unwrap();
        assert_eq!(context.gas_limit.as_u64(), 315_000_000);
        assert!(!duration_active_rules(&fixture, period).unwrap().unwrap().1);
    }
}

fn profile_at(period: u64, fixture: &Value) -> TaraxaProfile {
    let hf = &fixture["configuration"]["hardforks"];
    let coordinate = |v: &Value| v.as_u64().unwrap_or_else(|| string(v).parse().unwrap());
    fixture_execution_profile(&serde_json::json!({"rules": {
        "ficus": period >= coordinate(&hf["ficus"]["block"]),
        "cacti": period >= coordinate(&hf["cacti"]["block"])
    }}))
}

fn expected_estimate(rows: &[Value], cap: u64) -> std::result::Result<u64, String> {
    let mut hi = cap;
    for (index, row) in rows.iter().enumerate() {
        let out = &row["output"];
        if !string(&out["consensus_error"]).is_empty() {
            return Err(string(&out["consensus_error"]).to_owned());
        }
        if index == 0 && !string(&out["execution_error"]).is_empty() {
            return Err(string(&out["execution_error"]).to_owned());
        }
        if index != 0 && string(&out["execution_error"]).is_empty() {
            hi = number(&row["gas"]);
        }
    }
    Ok(hi)
}

type DurationNativeCall = (&'static str, u32, u64, u64);
struct DurationOriginPort<'a> {
    inner: Port<'a>,
    calls: Rc<RefCell<Vec<DurationNativeCall>>>,
}
impl NativeExecutionPort for DurationOriginPort<'_> {
    fn input_mutations(
        &mut self,
        id: rustaxa_evm::contracts::NativeInvocationId,
    ) -> std::result::Result<Vec<rustaxa_evm::contracts::NativeInputMutation>, NativePortError>
    {
        self.inner.input_mutations(id)
    }
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> std::result::Result<NativeGasQuote, NativePortError> {
        let id = invocation.id;
        self.calls.borrow_mut().push((
            "prepare_begin",
            id.transaction.as_u32(),
            id.sequence,
            invocation.period.as_u64(),
        ));
        let result = self.inner.prepare(invocation, journal);
        self.calls.borrow_mut().push((
            if result.is_ok() {
                "prepare_ok"
            } else {
                "prepare_error"
            },
            id.transaction.as_u32(),
            id.sequence,
            invocation.period.as_u64(),
        ));
        result
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> std::result::Result<NativeInvocationResult, NativePortError> {
        let id = invocation.id;
        self.calls.borrow_mut().push((
            "invoke_begin",
            id.transaction.as_u32(),
            id.sequence,
            invocation.period.as_u64(),
        ));
        let result = self.inner.invoke(invocation, quote, journal);
        let terminal = match &result {
            Ok(NativeInvocationResult::Completed(_)) => "invoke_completed",
            Ok(NativeInvocationResult::InsufficientGas { .. }) => "invoke_insufficient_gas",
            Err(_) => "invoke_error",
        };
        self.calls.borrow_mut().push((
            terminal,
            id.transaction.as_u32(),
            id.sequence,
            invocation.period.as_u64(),
        ));
        result
    }
}
fn duration_check_native_calls(
    calls: &[DurationNativeCall],
    transaction_count: usize,
    execution_period: u64,
) -> Result<Vec<Value>> {
    let mut next_sequence = 0_u64;
    let mut previous_transaction = 0_u32;
    let mut pending: Option<(u32, u64, &str)> = None;
    let mut terminals = Vec::new();
    for &(kind, transaction, sequence, period) in calls {
        ensure!(period == execution_period, "native execution period");
        ensure!(
            (transaction as usize) < transaction_count && transaction >= previous_transaction,
            "native transaction order"
        );
        let id = (transaction, sequence);
        match kind {
            "prepare_begin" => {
                ensure!(pending.is_none(), "unfinished native attempt");
                ensure!(sequence == next_sequence, "native sequence discontinuity");
                next_sequence = next_sequence
                    .checked_add(1)
                    .context("native sequence overflow")?;
                previous_transaction = transaction;
                pending = Some((transaction, sequence, "preparing"));
            }
            "prepare_ok" => {
                ensure!(
                    pending == Some((id.0, id.1, "preparing")),
                    "unpaired native prepare result"
                );
                pending = Some((transaction, sequence, "prepared"));
            }
            "prepare_error" => bail!("native prepare failed in completed API scope"),
            "invoke_begin" => {
                ensure!(
                    pending == Some((id.0, id.1, "prepared")),
                    "native invoke without matching prepare"
                );
                pending = Some((transaction, sequence, "invoking"));
            }
            "invoke_completed" | "invoke_insufficient_gas" => {
                ensure!(
                    pending == Some((id.0, id.1, "invoking")),
                    "unpaired native invoke result"
                );
                pending = None;
                terminals.push(serde_json::json!({"transaction":transaction,"sequence":sequence,"terminal":kind}));
            }
            "invoke_error" => bail!("native invoke failed in completed API scope"),
            _ => bail!("unknown native origin call"),
        }
    }
    ensure!(pending.is_none(), "unfinished native attempt");
    Ok(terminals)
}

fn duration_check_go_origin(
    origin: &Value,
    parent: u64,
    stages: Option<&[Value]>,
    tokens: &mut BTreeSet<u64>,
) -> Result<()> {
    ensure!(
        origin.as_object().is_some_and(|row| row.len() == 3) && origin["complete"] == true,
        "incomplete Go origin scope"
    );
    let token = origin["token"].as_u64().context("Go origin token")?;
    ensure!(
        token != 0 && tokens.last().is_none_or(|last| token > *last) && tokens.insert(token),
        "reused or unordered Go origin token"
    );
    let events = origin["events"].as_array().context("Go origin events")?;
    let mut main_open = 0;
    let mut live_open = 0;
    let mut active: Option<&str> = None;
    let mut stage = 0usize;
    let mut pending_site: Option<(u64, &str, &str, u64)> = None;
    let execution = parent
        .checked_add(u64::from(stages.is_some()))
        .context("Go origin execution period")?;
    for (index, event) in events.iter().enumerate() {
        ensure!(
            event.as_object().is_some_and(|row| row.len() == 9)
                && [
                    "ordinal",
                    "kind",
                    "phase",
                    "stage",
                    "stage_ordinal",
                    "requested",
                    "returned_kind",
                    "returned_period",
                    "token"
                ]
                .iter()
                .all(|key| event.get(*key).is_some()),
            "Go origin event schema"
        );
        ensure!(
            event["ordinal"].as_u64() == Some(index as u64)
                && event["token"].as_u64() == Some(token),
            "Go origin event identity"
        );
        let kind = event["kind"].as_str().context("Go origin kind")?;
        let phase = event["phase"].as_str().context("Go origin phase")?;
        ensure!(
            phase != "between" && phase != "teardown",
            "unexplained Go origin between stages"
        );
        if pending_site.is_some() {
            ensure!(
                matches!(kind, "factory" | "live_factory"),
                "reader site lacks immediate factory return"
            );
        }
        match kind {
            "dpos_current" | "dpos_delayed" | "slashing_delayed" => {
                let requested = event["requested"]
                    .as_u64()
                    .context("reader site coordinate")?;
                let expected = if kind == "dpos_current" {
                    execution
                } else {
                    execution.saturating_sub(5)
                };
                let stage_name = event["stage"].as_str().context("reader site stage")?;
                let stage_number = event["stage_ordinal"]
                    .as_u64()
                    .context("reader site stage ordinal")?;
                ensure!(
                    main_open == 1
                        && (stages.is_none() || live_open == 1)
                        && requested == expected
                        && event["returned_kind"] == "reader_site"
                        && event["returned_period"].is_null()
                        && (phase == "setup"
                            || (phase == "stage"
                                && active == Some(stage_name)
                                && stage_number == stage as u64)),
                    "Go reader site origin"
                );
                pending_site = Some((requested, phase, stage_name, stage_number));
            }
            "main_reader_open" => {
                ensure!(
                    phase == "setup"
                        && main_open == 0
                        && live_open == 0
                        && event["requested"].as_u64() == Some(parent)
                        && event["returned_kind"] == "historical"
                        && event["returned_period"].as_u64() == Some(parent),
                    "Go main reader origin"
                );
                main_open += 1;
            }
            "live_open" => {
                ensure!(
                    stages.is_some()
                        && phase == "setup"
                        && main_open == 1
                        && live_open == 0
                        && event["requested"].as_u64() == Some(parent)
                        && event["returned_kind"] == "live_block_state"
                        && event["returned_period"].is_null(),
                    "Go trace live origin"
                );
                live_open += 1;
            }
            "factory" | "live_factory" => {
                let (requested, site_phase, site_stage, site_ordinal) = pending_site
                    .take()
                    .context("factory without actual reader site")?;
                ensure!(
                    event["requested"].as_u64() == Some(requested)
                        && event["phase"] == site_phase
                        && event["stage"] == site_stage
                        && event["stage_ordinal"].as_u64() == Some(site_ordinal),
                    "Go factory/site association"
                );
                if kind == "factory" {
                    ensure!(
                        stages.is_none()
                            && phase == "setup"
                            && event["returned_kind"] == "historical"
                            && event["returned_period"].as_u64() == Some(requested),
                        "Go historical factory origin"
                    );
                } else {
                    ensure!(
                        stages.is_some()
                            && live_open == 1
                            && event["returned_kind"] == "live_block_state"
                            && event["returned_period"].is_null(),
                        "Go live factory origin"
                    );
                }
            }
            "stage_begin" => {
                let expected = stages
                    .and_then(|rows| rows.get(stage))
                    .context("Go trace stage")?;
                let name = event["stage"].as_str().context("Go trace stage name")?;
                ensure!(
                    live_open == 1
                        && active.is_none()
                        && phase == "stage"
                        && event["stage_ordinal"].as_u64() == Some(stage as u64)
                        && expected["stage"] == name,
                    "Go trace stage begin"
                );
                active = Some(name);
            }
            "stage_end" => {
                ensure!(
                    phase == "stage"
                        && active.is_some()
                        && active == event["stage"].as_str()
                        && event["stage_ordinal"].as_u64() == Some(stage as u64),
                    "Go trace stage end"
                );
                active = None;
                stage += 1;
            }
            _ => bail!("unknown Go origin event"),
        }
    }
    ensure!(
        main_open == 1 && active.is_none() && pending_site.is_none(),
        "missing Go origin opening or stage end"
    );
    if let Some(rows) = stages {
        ensure!(
            live_open == 1 && stage == rows.len(),
            "incomplete Go trace stages"
        );
    } else {
        ensure!(live_open == 0 && stage == 0, "unexpected Go probe trace");
    }
    Ok(())
}

#[test]
fn duration_origin_rejects_changed_or_missing_observations() {
    let event = |ordinal,
                 kind,
                 phase,
                 stage,
                 stage_ordinal,
                 requested: Value,
                 returned_kind,
                 returned_period: Value| {
        serde_json::json!({"ordinal":ordinal,"kind":kind,"phase":phase,"stage":stage,
            "stage_ordinal":stage_ordinal,"requested":requested,"returned_kind":returned_kind,
            "returned_period":returned_period,"token":9})
    };
    let mut probe = serde_json::json!({"token":9,"complete":true,"events":[
        event(0,"main_reader_open","setup","",0,serde_json::json!(12),"historical",serde_json::json!(12)),
        event(1,"dpos_delayed","setup","",0,serde_json::json!(7),"reader_site",Value::Null),
        event(2,"factory","setup","",0,serde_json::json!(7),"historical",serde_json::json!(7))]});
    let mut seen = BTreeSet::new();
    duration_check_go_origin(&probe, 12, None, &mut seen).unwrap();
    assert!(duration_check_go_origin(&probe, 12, None, &mut seen).is_err());
    probe["events"][2]["returned_period"] = serde_json::json!(8);
    assert!(duration_check_go_origin(&probe, 12, None, &mut BTreeSet::new()).is_err());
    probe["events"][2]["returned_period"] = serde_json::json!(7);
    probe["events"][1]["requested"] = Value::Null;
    assert!(duration_check_go_origin(&probe, 12, None, &mut BTreeSet::new()).is_err());
    probe["events"][1]["requested"] = serde_json::json!(7);
    probe["events"][1]["kind"] = serde_json::json!("dpos_current");
    assert!(duration_check_go_origin(&probe, 12, None, &mut BTreeSet::new()).is_err());
    probe["events"][1]["kind"] = serde_json::json!("dpos_delayed");
    probe["events"].as_array_mut().unwrap().remove(0);
    assert!(duration_check_go_origin(&probe, 12, None, &mut BTreeSet::new()).is_err());
    let mut trace = serde_json::json!({"token":9,"complete":true,"events":[
        event(0,"main_reader_open","setup","",0,serde_json::json!(12),"historical",serde_json::json!(12)),
        event(1,"live_open","setup","",0,serde_json::json!(12),"live_block_state",Value::Null),
        event(2,"stage_begin","stage","target",0,serde_json::json!(0),"none",Value::Null),
        event(3,"slashing_delayed","stage","target",0,serde_json::json!(8),"reader_site",Value::Null),
        event(4,"live_factory","stage","target",0,serde_json::json!(8),"live_block_state",Value::Null),
        event(5,"stage_end","stage","target",0,serde_json::json!(0),"none",Value::Null)]});
    let stages = vec![serde_json::json!({"stage":"target"})];
    duration_check_go_origin(&trace, 12, Some(&stages), &mut BTreeSet::new()).unwrap();
    trace["events"][4]["token"] = serde_json::json!(10);
    assert!(duration_check_go_origin(&trace, 12, Some(&stages), &mut BTreeSet::new()).is_err());
    trace["events"][4]["token"] = serde_json::json!(9);
    trace["events"][4]["stage_ordinal"] = serde_json::json!(1);
    assert!(duration_check_go_origin(&trace, 12, Some(&stages), &mut BTreeSet::new()).is_err());
    trace["events"][4]["stage_ordinal"] = serde_json::json!(0);
    trace["events"][4]["root"] = serde_json::json!("forged");
    assert!(duration_check_go_origin(&trace, 12, Some(&stages), &mut BTreeSet::new()).is_err());
}

#[test]
fn duration_native_origin_rejects_bad_attempt_identity() {
    let valid = [
        ("prepare_begin", 0, 0, 12),
        ("prepare_ok", 0, 0, 12),
        ("invoke_begin", 0, 0, 12),
        ("invoke_insufficient_gas", 0, 0, 12),
        ("prepare_begin", 1, 1, 12),
        ("prepare_ok", 1, 1, 12),
        ("invoke_begin", 1, 1, 12),
        ("invoke_completed", 1, 1, 12),
    ];
    let terminals = duration_check_native_calls(&valid, 2, 12).unwrap();
    assert_eq!(terminals[0]["terminal"], "invoke_insufficient_gas");
    let mut bad = valid;
    bad[4].2 = 0;
    assert!(duration_check_native_calls(&bad, 2, 12).is_err());
    bad = valid;
    bad[6].1 = 0;
    assert!(duration_check_native_calls(&bad, 2, 12).is_err());
    bad = valid;
    bad[7].3 = 13;
    assert!(duration_check_native_calls(&bad, 2, 12).is_err());
    assert!(duration_check_native_calls(&valid[..2], 2, 12).is_err());
    let omitted = [valid[0], valid[1], valid[4], valid[5], valid[6], valid[7]];
    assert!(duration_check_native_calls(&omitted, 2, 12).is_err());
}

fn duration_origin_required(fixture: &Value, cases: &[Value]) -> bool {
    fixture["duration_require_origin"] == true
        || cases.iter().any(|case| {
            case.get("call_origin").is_some()
                || case.get("repeat_origin").is_some()
                || ["trace", "trace_sequence"]
                    .iter()
                    .any(|key| case[*key].get("origin").is_some())
                || case["estimates"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item.get("origins").is_some()))
        })
}

#[test]
fn duration_origin_schema_activation_rejects_partial_fallback() {
    let fixture = serde_json::json!({});
    assert!(!duration_origin_required(
        &fixture,
        &[serde_json::json!({})]
    ));
    for case in [
        serde_json::json!({"call_origin":null}),
        serde_json::json!({"repeat_origin":null}),
        serde_json::json!({"trace":{"origin":null}}),
        serde_json::json!({"trace_sequence":{"origin":null}}),
        serde_json::json!({"estimates":[{"origins":null}]}),
    ] {
        assert!(duration_origin_required(&fixture, &[case]));
    }
    assert!(duration_origin_required(
        &serde_json::json!({"duration_require_origin":true}),
        &[]
    ));
}

fn verify_cases(
    fixture: &Value,
    cases: &[Value],
    fork: Option<&Value>,
    application: &Path,
    concrete: &Path,
) -> Result<()> {
    let (_storage, chain) = open_chain(application, fixture)?;
    let committed =
        ConcreteStateLifecycle::inspect_existing(concrete, chain.concrete_chain_identity()?)?
            .committed;
    let mut probes = 0;
    let mut traces = 0;
    let require_origin = duration_origin_required(fixture, cases);
    let mut origin_tokens = BTreeSet::new();
    let retained_origins = RefCell::new(Vec::<Value>::new());

    ensure!(!cases.is_empty(), "required API target execution");
    for case in cases {
        let head = number(&case["head"]);
        let selected = selected_state(fixture, head - u64::from(fork.is_some()))?;
        let reader = CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(concrete, committed, selected)?,
            known: complete_inventory(fixture, selected.period.as_u64())?,
        };
        let mut call = |probe: &Value, origin: &Value| -> Result<EstimateProbe> {
            if require_origin {
                duration_check_go_origin(origin, head, None, &mut origin_tokens)?;
            }
            let request =
                native_integration::api::request(&case["request"], number(&probe["gas"]), 0);
            let original = request.clone();
            let ConcreteRead::Present(sender) = reader.account(request.sender)? else {
                bail!("required API sender")
            };
            let effective =
                BigUint::from_bytes_be(&sender.account.nonce.to_bytes()) + BigUint::from(1u8);
            ensure!(
                effective.to_string() == string(&probe["effective_nonce"]),
                "full-width historical nonce policy"
            );
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let native_origins = Rc::new(RefCell::new(Vec::new()));
            let native_calls = Rc::new(RefCell::new(Vec::new()));
            if let Some(fork) = fork {
                return pre_reward_call(
                    &reader, &chain, fixture, fork, case, probe, request, outcomes,
                );
            }
            let result = simulate_with_native(
                &reader,
                &ChainHashes(&chain),
                &AllNatives(profile_at(head, fixture), &chain),
                &ConsensusNatives(&chain),
                |identity| {
                    let session =
                        chain
                            .begin_native_simulation(identity.period)
                            .map_err(|error| {
                                rustaxa_evm::contracts::NativePortError::Domain(error.to_string())
                            })?;
                    native_origins
                        .borrow_mut()
                        .push((identity, session.synthetic_origins().to_vec()));
                    Ok(DurationOriginPort {
                        inner: Port {
                            inner: mixed_native::SimulationNativeExecutionPort::new(session),
                            outcomes: outcomes.clone(),
                        },
                        calls: native_calls.clone(),
                    })
                },
                &block(head, fixture)?,
                &request,
                EnvelopeRules {
                    cornus: duration_active_rules(fixture, head)?.is_none_or(|rules| rules.1),
                },
                profile_at(head, fixture),
            )
            .map_err(|error| anyhow::anyhow!("{error:?}"))
            .with_context(|| {
                format!("era historical call {} gas {}", case["name"], probe["gas"])
            })?;
            ensure!(
                result.state == selected && request == original,
                "fresh probe state/request identity"
            );
            if require_origin {
                ensure!(
                    reader.identity() == selected,
                    "actual concrete probe reader"
                );
                for (identity, entries) in native_origins.borrow().iter() {
                    ensure!(
                        *identity == reader.identity()
                            && entries.len() == 2
                            && entries[0].role == "simulation_historical"
                            && entries[0].period.as_u64() == head
                            && entries[1].role == "eligibility_historical"
                            && entries[1].period.as_u64() == head.saturating_sub(5),
                        "actual Rust probe origin"
                    );
                }
                let terminals = duration_check_native_calls(&native_calls.borrow(), 1, head)?;
                retained_origins.borrow_mut().push(serde_json::json!({"kind":"probe", "case":case["name"], "go_token":origin["token"], "head":head,
                    "gas":probe["gas"], "reader_period":reader.identity().period.as_u64(),
                    "reader_root":hex::encode(reader.identity().state_root),
                    "sessions":native_origins.borrow().iter().map(|(_, entries)| entries.iter().map(|entry| serde_json::json!({"role":entry.role,"period":entry.period.as_u64()})).collect::<Vec<_>>()).collect::<Vec<_>>(),
                    "transaction_stages":["probe"], "native_calls":native_calls.borrow().clone(), "native_terminals":terminals}));
                ensure!(
                    native_origins.borrow().len() == 1,
                    "missing or duplicate native session origin"
                );
            }
            native_integration::api::compare_raw(&outcomes.borrow(), &probe["ordered_raw_writes"])?;
            native_integration::api::compare(&result.execution, &probe["output"])
        };
        call(&case["call"], &case["call_origin"])?;
        call(&case["repeat_call"], &case["repeat_origin"])?;
        for estimate in case["estimates"].as_array().context("estimate cases")? {
            let rows = estimate["probes"].as_array().context("actual Go probes")?;
            let origins = estimate["origins"].as_array();
            if require_origin {
                ensure!(
                    origins.is_some_and(|items| items.len() == rows.len()),
                    "Go estimate origin count"
                );
            }
            let mut at = 0;
            let result = estimate_gas(number(&estimate["cap"]), |gas| {
                let row = rows.get(at).context("complete estimate transcript")?;
                at += 1;
                ensure!(gas == number(&row["gas"]), "unchanged search probe gas");
                call(
                    row,
                    origins
                        .and_then(|items| items.get(at - 1))
                        .unwrap_or(&Value::Null),
                )
            });
            ensure!(at == rows.len() && at != 0, "all required estimate probes");
            let result = match result {
                Ok(value) => Ok(value),
                Err(
                    EstimateError::CodeFailure { error }
                    | EstimateError::ConsensusFailure { error },
                ) => Err(error),
                Err(error) => bail!("unexpected estimate infrastructure result: {error:?}"),
            };
            ensure!(
                result == expected_estimate(rows, number(&estimate["cap"])),
                "estimate result"
            );
            probes += at;
        }
        let trace_period = number(&case["trace_period"]);
        let trace_state = selected_state(fixture, trace_period.saturating_sub(1))?;
        let trace_reader = CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(
                concrete,
                committed,
                trace_state,
            )?,
            known: complete_inventory(fixture, trace_state.period.as_u64())?,
        };
        for name in ["trace", "trace_sequence"] {
            let trace = &case[name];
            if require_origin {
                let stages = trace["calls"].as_array().context("Go trace calls")?;
                let prefix_count = trace["prefix"].as_array().context("Go trace prefix")?.len();
                ensure!(
                    stages.len() == prefix_count + 1
                        && trace.get("panic").is_none()
                        && trace.get("interrupted_call").is_none(),
                    "Go trace completion"
                );
                for (index, stage) in stages.iter().enumerate() {
                    ensure!(
                        stage["stage"]
                            == if index < prefix_count {
                                "prefix"
                            } else {
                                "target"
                            },
                        "Go trace stage role"
                    );
                }
                duration_check_go_origin(
                    &trace["origin"],
                    trace_period.saturating_sub(1),
                    Some(stages),
                    &mut origin_tokens,
                )?;
            }
            let prefix = trace["prefix"]
                .as_array()
                .context("trace prefix")?
                .iter()
                .enumerate()
                .map(|(i, row)| native_integration::api::request(row, 500_000, i))
                .collect::<Vec<_>>();
            let targets = [native_integration::api::request(
                &trace["request"],
                500_000,
                prefix.len(),
            )];
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let native_origins = Rc::new(RefCell::new(Vec::new()));
            let native_calls = Rc::new(RefCell::new(Vec::new()));
            let result = run_structured_trace_with_native(
                &trace_reader,
                &ChainHashes(&chain),
                &AllNatives(profile_at(trace_period, fixture), &chain),
                &ConsensusNatives(&chain),
                |identity, execution| {
                    let session = chain
                        .begin_native_trace(identity.period, execution)
                        .map_err(|error| {
                            rustaxa_evm::contracts::NativePortError::Domain(error.to_string())
                        })?;
                    native_origins
                        .borrow_mut()
                        .push((identity, session.synthetic_origins().to_vec()));
                    Ok(DurationOriginPort {
                        inner: Port {
                            inner: mixed_native::SimulationNativeExecutionPort::new(session),
                            outcomes: outcomes.clone(),
                        },
                        calls: native_calls.clone(),
                    })
                },
                &block(trace_period, fixture)?,
                &prefix,
                &targets,
                EnvelopeRules {
                    cornus: duration_active_rules(fixture, trace_period)?
                        .is_none_or(|rules| rules.1),
                },
                profile_at(trace_period, fixture),
            )?;
            ensure!(
                result.state == trace_state,
                "trace selected parent identity"
            );
            if require_origin {
                ensure!(
                    trace_reader.identity() == trace_state,
                    "actual concrete trace reader"
                );
                for (identity, entries) in native_origins.borrow().iter() {
                    ensure!(
                        *identity == trace_reader.identity()
                            && entries.len() == 3
                            && entries[0].role == "trace_parent_historical"
                            && entries[0].period.as_u64() == trace_period.saturating_sub(1)
                            && entries[1].role == "trace_advance"
                            && entries[1].period.as_u64() == trace_period
                            && entries[2].role == "eligibility_live_clone"
                            && entries[2].period.as_u64() == trace_period,
                        "actual Rust trace origin"
                    );
                }
                ensure!(
                    native_origins.borrow().len() == 1,
                    "missing or duplicate trace native session origin"
                );
                let terminals = duration_check_native_calls(
                    &native_calls.borrow(),
                    prefix.len() + targets.len(),
                    trace_period,
                )?;
                retained_origins.borrow_mut().push(serde_json::json!({"kind":name,"case":case["name"],"go_token":trace["origin"]["token"],"execution_period":trace_period,
                    "reader_period":trace_reader.identity().period.as_u64(),
                    "reader_root":hex::encode(trace_reader.identity().state_root),
                    "sessions":native_origins.borrow().iter().map(|(_, entries)| entries.iter().map(|entry| serde_json::json!({"role":entry.role,"period":entry.period.as_u64()})).collect::<Vec<_>>()).collect::<Vec<_>>(),
                    "transaction_stages":prefix.iter().map(|_| "prefix").chain(std::iter::once("target")).collect::<Vec<_>>(), "native_calls":native_calls.borrow().clone(), "native_terminals":terminals}));
            }
            ensure!(
                serde_json::from_slice::<Value>(&result.output)? == trace["result"],
                "default trace {} {name}: actual {} expected {}",
                case["name"],
                String::from_utf8_lossy(&result.output),
                trace["result"]
            );
            let expected = trace["calls"]
                .as_array()
                .context("trace native observer calls")?
                .iter()
                .flat_map(|row| {
                    row["ordered_raw_writes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .cloned()
                })
                .collect::<Vec<_>>();
            native_integration::api::compare_raw(&outcomes.borrow(), &Value::Array(expected))?;
            traces += 1;
        }
        if let Some(fork) = fork {
            verify_complete_final_reader(&reader, &fork["pre_reward"])?;
        } else {
            let period = fixture["periods"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| number(&row["number"]) == head)
                .unwrap();
            verify_complete_final_reader(&reader, &period["final"])?;
        }
    }
    ensure!(
        probes > cases.len() * 3 && traces == cases.len() * 2,
        "complete nonzero historical API execution"
    );
    if require_origin {
        use std::io::Write;
        let path = application
            .parent()
            .context("duration API artifact parent")?
            .join("rust-api-origins.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        for row in retained_origins.into_inner() {
            serde_json::to_writer(&mut file, &row)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
    }
    eprintln!(
        "era API parity: {} cases, {probes} probes, {traces} traces",
        cases.len()
    );
    Ok(())
}
