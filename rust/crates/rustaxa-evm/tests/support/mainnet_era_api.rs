//! Historical API parity over authenticated complete mainnet-policy histories.
use super::*;
use native_integration::api::{CompleteReader, Port};
use rustaxa_evm::{
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
    verify_final_reader(&reader, phase)
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
    verify_final_reader(&reader, &fork["pre_reward"])?;
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
            &block(period, fixture),
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
        &block(period, fixture),
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

fn block(period: u64, fixture: &Value) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: period.into(),
        author: fixed(&fixture["periods"][0]["reward_input"]["block_author"]),
        timestamp: 0,
        gas_limit: number(&fixture["configuration"]["block_gas_limit"]).into(),
        chain_id: 841,
        difficulty: BigUint::default(),
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

    ensure!(!cases.is_empty(), "required API target execution");
    for case in cases {
        let head = number(&case["head"]);
        let selected = selected_state(fixture, head - u64::from(fork.is_some()))?;
        let reader = CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(concrete, committed, selected)?,
            known: complete_inventory(fixture, selected.period.as_u64())?,
        };
        let call = |probe: &Value| -> Result<EstimateProbe> {
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
                    Ok(Port {
                        inner: mixed_native::SimulationNativeExecutionPort::new(
                            chain
                                .begin_native_simulation(identity.period)
                                .map_err(|error| {
                                    rustaxa_evm::contracts::NativePortError::Domain(
                                        error.to_string(),
                                    )
                                })?,
                        ),
                        outcomes: outcomes.clone(),
                    })
                },
                &block(head, fixture),
                &request,
                EnvelopeRules { cornus: true },
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
            native_integration::api::compare_raw(&outcomes.borrow(), &probe["ordered_raw_writes"])?;
            native_integration::api::compare(&result.execution, &probe["output"])
        };
        call(&case["call"])?;
        call(&case["repeat_call"])?;
        for estimate in case["estimates"].as_array().context("estimate cases")? {
            let rows = estimate["probes"].as_array().context("actual Go probes")?;
            let mut at = 0;
            let result = estimate_gas(number(&estimate["cap"]), |gas| {
                let row = rows.get(at).context("complete estimate transcript")?;
                at += 1;
                ensure!(gas == number(&row["gas"]), "unchanged search probe gas");
                call(row)
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
            let result = run_structured_trace_with_native(
                &trace_reader,
                &ChainHashes(&chain),
                &AllNatives(profile_at(trace_period, fixture), &chain),
                &ConsensusNatives(&chain),
                |identity, execution| {
                    Ok(Port {
                        inner: mixed_native::SimulationNativeExecutionPort::new(
                            chain
                                .begin_native_trace(identity.period, execution)
                                .map_err(|error| {
                                    rustaxa_evm::contracts::NativePortError::Domain(
                                        error.to_string(),
                                    )
                                })?,
                        ),
                        outcomes: outcomes.clone(),
                    })
                },
                &block(trace_period, fixture),
                &prefix,
                &targets,
                EnvelopeRules { cornus: true },
                profile_at(trace_period, fixture),
            )?;
            ensure!(
                result.state == trace_state,
                "trace selected parent identity"
            );
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
            verify_final_reader(&reader, &fork["pre_reward"])?;
        } else {
            let period = fixture["periods"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| number(&row["number"]) == head)
                .unwrap();
            verify_final_reader(&reader, &period["final"])?;
        }
    }
    ensure!(
        probes > cases.len() * 3 && traces == cases.len() * 2,
        "complete nonzero historical API execution"
    );
    eprintln!(
        "era API parity: {} cases, {probes} probes, {traces} traces",
        cases.len()
    );
    Ok(())
}
