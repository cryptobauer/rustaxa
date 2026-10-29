//! Authenticated current-profile corpus consumed by the existing mixed owner.
use super::*;
use std::io::Read;

pub(super) fn compare_reward_map(
    context: &FinalChainNativeRewardsContext,
    rewards: &Value,
    end_block: &Value,
    input: &Value,
) -> Result<()> {
    let expected = rewards.as_array().context("reward writes")?;
    let end = end_block.as_array().context("end-block writes")?;
    let validators = input["validators"]
        .as_array()
        .context("one reward distribution")?;
    ensure!(
        expected.len() == validators.len() + 2 || expected.len() == validators.len() + 3,
        "single-distribution reward shape"
    );
    ensure!(expected.len() >= 3, "nonempty Aspen reward map");
    ensure!(
        context.raw_mutations.len() == expected.len() + end.len(),
        "complete reward occurrence count"
    );
    let (actual, actual_end) = context.raw_mutations.split_at(expected.len());
    ensure!(
        fixed::<32>(&expected[0]["key"]) == keccak256([8]).0
            && fixed::<32>(&expected.last().unwrap()["key"]) == keccak256([7]).0,
        "yield/supply barrier identities"
    );
    compare_raw_mutation(&actual[0], &expected[0])?;
    compare_raw_mutation(actual.last().unwrap(), expected.last().unwrap())?;
    let mut first = 1;
    // A positive author bonus precedes the map and writes its row twice.
    if expected.len() == validators.len() + 3 {
        let mut key = vec![0, 2];
        key.extend_from_slice(&fixed::<20>(&input["block_author"]));
        ensure!(
            fixed::<32>(&expected[1]["key"]) == keccak256(key).0,
            "author bonus identity"
        );
        compare_raw_mutation(&actual[1], &expected[1])?;
        first = 2;
    }
    let mut remaining = expected[first..expected.len() - 1]
        .iter()
        .map(|row| {
            (
                (fixed::<20>(&row["address"]), fixed::<32>(&row["key"])),
                row,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    ensure!(
        remaining.len() == expected.len() - 1 - first,
        "one write per disjoint validator map block"
    );
    let expected_map = validators
        .iter()
        .map(|validator| {
            let mut key = vec![0, 2];
            key.extend_from_slice(&fixed::<20>(&validator["validator"]));
            (
                [
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
                ],
                keccak256(key).0,
            )
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        remaining.keys().copied().collect::<BTreeSet<_>>() == expected_map,
        "reward map matches distribution validators"
    );
    for mutation in &actual[first..actual.len() - 1] {
        let row = remaining
            .remove(&(mutation.address, mutation.key.0))
            .context("validator reward-map occurrence")?;
        compare_raw_mutation(mutation, row)?;
    }
    ensure!(remaining.is_empty(), "complete validator reward map");
    for (mutation, expected) in actual_end.iter().zip(end) {
        compare_raw_mutation(mutation, expected)?;
    }
    Ok(())
}

pub(super) fn fixture_stream(stream: &str) -> Result<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_integration");
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    ensure!(manifest["registry"].as_array().context("registry")?.len() == 29);
    let pin = &manifest["streams"][stream];
    let name = string(&pin["file"]);
    ensure!(
        Path::new(name).components().count() == 1,
        "local corpus filename"
    );
    let stored = std::fs::read(root.join(name))?;
    let body = if pin["encoding"] == "gzip" {
        let mut body = Vec::new();
        flate2::read::GzDecoder::new(stored.as_slice()).read_to_end(&mut body)?;
        body
    } else {
        stored
    };
    let hash = revm::precompile::hash::sha256_run(&body, u64::MAX)
        .map_err(|error| anyhow::anyhow!("corpus SHA-256: {error:?}"))?;
    ensure!(
        hex::encode(hash.bytes) == string(&pin["sha256"]),
        "authenticated corpus stream"
    );
    ensure!(
        body.len() as u64 == number(&pin["bytes"]),
        "corpus byte length"
    );
    let fixture: Value = serde_json::from_slice(&body)?;
    if stream != "cpp.api" {
        ensure!(fixture["profile"] == "synthetic_current");
        ensure!(fixture["registry"] == manifest["registry"]);
    }
    Ok(fixture)
}

pub(super) fn verify_retained(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
) -> Result<()> {
    for period in fixture["periods"].as_array().context("periods")? {
        let number = number(&period["number"]);
        if number > committed.period.as_u64() {
            break;
        }
        let reader = historical_reader(path, committed, fixture, number as usize)?;
        verify_final_reader(&reader, &period["final"])?;
    }
    Ok(())
}

#[test]
fn native_integration_signed_periods_publish_reopen_and_retain() -> Result<()> {
    workload("local.observer.traced", true)
}
#[test]
fn native_integration_cacti_bracket_publish_reopen_and_retain() -> Result<()> {
    workload("local.observer.traced.bracket", false)
}
fn workload(stream: &str, apis: bool) -> Result<()> {
    let fixture = fixture_stream(stream)?;
    let periods = fixture["periods"].as_array().context("periods")?;
    let bound = &fixture["period_bound"];
    ensure!(
        periods.len() as u64
            == number(&bound["last_custody"])
                + number(&bound["locking"])
                + number(&bound["delay"])
                + number(&bound["continuation"])
    );
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!(
        "mixed-native-integration-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    let application = root.join("application");
    let concrete = root.join("concrete");
    run_segment(&fixture, &periods[..9], &application, &concrete, true)?;
    run_segment(&fixture, &periods[9..], &application, &concrete, false)?;
    if apis {
        api::verify(&fixture, &application, &concrete)?;
    }
    era_retention::finish(
        scratch,
        era_retention::evidence(
            &fixture,
            "complete native integration periods,receipts/native/history,physicalreopen; API branch recorded by test/case ID; all owners closed",
        )?,
        false,
    )?;
    Ok(())
}

#[path = "native_integration_api.rs"]
pub(crate) mod api;

#[test]
fn native_integration_reward_map_permutation_rejects_barrier_and_occurrence_changes() -> Result<()>
{
    use rustaxa_consensus::native_session::{
        FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRawValue,
    };
    let fixture = fixture_stream("local.observer.traced")?;
    let period = &fixture["periods"][1];
    let rewards = &period["ordered_raw_writes"]["rewards"];
    let end = &period["ordered_raw_writes"]["end_block"];
    let input = &period["reward_input"];
    let mut context = FinalChainNativeRewardsContext::default();
    for row in rewards
        .as_array()
        .unwrap()
        .iter()
        .chain(end.as_array().unwrap())
    {
        let value = bytes(&row["value"]);
        context.raw_mutations.push(FinalChainNativeRawMutation {
            address: fixed(&row["address"]),
            key: ConcreteStorageKey(fixed(&row["key"])),
            expected: ConcreteRead::Absent,
            operation: if value.is_empty() {
                FinalChainNativeRawOperation::Delete
            } else {
                FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(value).unwrap())
            },
        });
    }
    compare_reward_map(&context, rewards, end, input)?;
    let first =
        if rewards.as_array().unwrap().len() == input["validators"].as_array().unwrap().len() + 3 {
            2
        } else {
            1
        };
    let mut permutation = context.clone();
    permutation.raw_mutations.swap(first, first + 1);
    compare_reward_map(&permutation, rewards, end, input)?;
    for kind in 0..7 {
        let mut bad = context.clone();
        match kind {
            0 => bad.raw_mutations.swap(0, first),
            1 => {
                bad.raw_mutations[first] = bad.raw_mutations[first + 1].clone();
            }
            2 => {
                bad.raw_mutations.remove(first);
            }
            3 => bad.raw_mutations.push(bad.raw_mutations[first].clone()),
            4 => {
                bad.raw_mutations[first].operation = FinalChainNativeRawOperation::Put(
                    FinalChainNativeRawValue::new(vec![0xff]).unwrap(),
                )
            }
            5 => bad.raw_mutations[first].address = [9; 20],
            _ => {
                let last = rewards.as_array().unwrap().len() - 1;
                bad.raw_mutations.swap(first, last);
            }
        }
        ensure!(
            compare_reward_map(&bad, rewards, end, input).is_err(),
            "invalid reward permutation {kind}"
        );
    }
    ensure!(
        compare_reward_map(
            &context,
            &serde_json::json!([{}, {}]),
            end,
            &serde_json::json!({"validators":[]})
        )
        .is_err(),
        "short invalid shape fails without indexing"
    );
    Ok(())
}

#[test]
fn native_integration_bounded_recovery_repeats_and_continues() -> Result<()> {
    let fixture = fixture_stream("local.observer.traced")?;
    let periods = fixture["periods"].as_array().unwrap();
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let parent = base.join(format!(
        "native-recovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&parent, &fixture)?;
    std::fs::create_dir(&parent)?;
    era_retention::created(&mut scratch)?;
    let baseline = parent.join("baseline");
    std::fs::create_dir(&baseline)?;
    run_segment(
        &fixture,
        &periods[..9],
        &baseline.join("application"),
        &baseline.join("state_db"),
        true,
    )?;
    run_segment(
        &fixture,
        &periods[9..],
        &baseline.join("application"),
        &baseline.join("state_db"),
        false,
    )?;
    let expected = mixed_recovery::final_facts(&baseline, &fixture)?;
    for fault in [
        mixed_recovery::Fault::BeforeCommit,
        mixed_recovery::Fault::AfterCommit,
    ] {
        let root = parent.join(format!("{fault:?}"));
        std::fs::create_dir(&root)?;
        run_segment_fault(
            &fixture,
            &periods[..9],
            &root.join("application"),
            &root.join("state_db"),
            true,
            fault,
        )?;
        mixed_recovery::recover_period(
            &root,
            &fixture,
            8,
            fault == mixed_recovery::Fault::AfterCommit,
        )?;
        if fault == mixed_recovery::Fault::BeforeCommit {
            run_segment(
                &fixture,
                &periods[8..9],
                &root.join("application"),
                &root.join("state_db"),
                false,
            )?;
        }
        run_segment(
            &fixture,
            &periods[9..],
            &root.join("application"),
            &root.join("state_db"),
            false,
        )?;
        ensure!(
            mixed_recovery::final_facts(&root, &fixture)? == expected,
            "recovered complete workload facts"
        );
    }
    era_retention::finish(
        scratch,
        era_retention::evidence(
            &fixture,
            "uninterrupted baseline plus BeforeCommit/AfterCommit repeatedrecovery/continuation exactfinalfacts; all owners closed",
        )?,
        false,
    )?;
    Ok(())
}

pub(super) fn run_segment(
    fixture: &Value,
    periods: &[Value],
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
) -> Result<()> {
    run_segment_fault(
        fixture,
        periods,
        application_path,
        concrete_path,
        fresh,
        mixed_recovery::Fault::None,
    )
}
pub(super) fn run_segment_fault(
    fixture: &Value,
    periods: &[Value],
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
    fault: mixed_recovery::Fault,
) -> Result<()> {
    run_segment_observing(
        fixture,
        periods,
        application_path,
        concrete_path,
        fresh,
        fault,
        None,
    )
}
pub(super) fn run_segment_observing(
    fixture: &Value,
    periods: &[Value],
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
    fault: mixed_recovery::Fault,
    ordinary: Option<OrdinaryObserver>,
) -> Result<()> {
    let (application, mut chain) = open_chain(application_path, fixture)?;
    let identity = chain.concrete_chain_identity()?;
    let mut concrete = if fresh {
        let concrete = fresh_concrete(concrete_path, &chain, fixture)?;
        chain.hydrate_concrete_genesis_accounts(concrete.prior_reader())?;
        concrete
    } else {
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, identity)?;
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: fixed(&fixture["genesis"]["root"]),
        };
        if fixture["profile"] != "synthetic_mainnet_era" {
            let reader = ConcreteStateReader::open_historical_read_only(
                concrete_path,
                observed.committed,
                genesis,
            )?;
            chain.hydrate_concrete_genesis_accounts(&reader)?;
            drop(reader);
        }
        ConcreteStateLifecycle::open(concrete_path, identity, observed.committed)?
    };
    let mut epoch = StateApiEpoch::new();
    for (index, period) in periods.iter().enumerate() {
        if let Some(observer) = &ordinary {
            let observed = concrete.observation()?;
            observer.borrow_mut().begin(
                concrete_path,
                observed.identity,
                observed.committed,
                number(&period["number"]),
                observed.generation,
            )?;
        }
        let inner = Adapter {
            chain: &chain,
            application: &application,
            concrete: RefCell::new(Some(concrete)),
            staged: RefCell::new(None),
            native: RefCell::new(None),
            native_context: RefCell::new(None),
            state_api_epoch: epoch,
            ordinary_observer: ordinary.clone(),
            fixture: period,
            account_node_publication: if fixture["oracle_lifecycle"] == "public_batched" {
                AccountNodePublication::FinalPeriod {
                    limits: rustaxa_storage::ConcreteStorageInventoryLimits {
                        max_nodes: 1_000_000,
                        max_leaves: 100_000,
                        max_value_bytes: 32_000_000,
                    },
                }
            } else {
                AccountNodePublication::ObserverHistory
            },
        };
        let adapter = mixed_recovery::FaultAdapter {
            inner,
            fault: if index + 1 == periods.len() {
                fault
            } else {
                mixed_recovery::Fault::None
            },
            executions: std::cell::Cell::new(0),
            rewards: std::cell::Cell::new(0),
            commits: std::cell::Cell::new(0),
            original_projection_hash: std::cell::Cell::new(None),
        };
        let execute = || -> Result<()> {
            if index == 0 {
                initialize_and_recover_final_chain_application_state_at_joint_startup(
                    &chain,
                    &adapter.inner,
                )?;
            }
            mainnet_era::bind_legacy_scheduler_order(&chain, fixture, period)?;
            let request = request(fixture, period)?;
            chain.ensure_period_data(
                FinalChainBlockNumber::new(number(&period["number"])),
                &period_data(&request),
            )?;
            let outcome = execute_final_chain_application_task(
                &chain,
                request,
                FinalChainProposalPeriodDagLevelUpdate::default(),
                false,
                fixed(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"]),
                &adapter,
            );
            if adapter.fault != mixed_recovery::Fault::None {
                let error = outcome.expect_err("bounded native integration interruption");
                ensure!(
                    format!("{error:#}").contains("mixed fixture interrupted"),
                    "{error:#}"
                );
                ensure!(
                    adapter.executions.get() == 1
                        && adapter.rewards.get() == 1
                        && adapter.commits.get() == 1
                );
                ensure!(chain.last_block_number_typed()?.as_u64() + 1 == number(&period["number"]));
                ensure!(
                    application
                        .final_chain()
                        .external_evm_pending_publication_raw()?
                        .is_some()
                );
                ensure!(
                    application
                        .final_chain()
                        .block_header_raw(number(&period["number"]))?
                        .is_none()
                );
                return Ok(());
            }
            let report = outcome?;
            ensure!(report.period.as_u64() == number(&period["number"]));
            verify_receipts(&application, period)?;
            ensure!(
                adapter.inner.concrete.borrow().is_none(),
                "concrete commit consumes its period handle"
            );
            let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, identity)?;
            ensure!(
                observed.committed.state_root == fixed(&period["final"]["root"]),
                "published canonical root"
            );
            ensure!(chain.last_block_number_typed()? == observed.committed.period);
            ensure!(
                application
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    .is_none()
            );
            if let Some(observer) = &ordinary {
                observer.borrow_mut().promote(
                    concrete_path,
                    observed.identity,
                    observed.committed,
                    observed.generation,
                )?;
            }
            let reader = ConcreteStateReader::open_read_only(concrete_path, observed.committed)?;
            verify_final_reader(&reader, &period["final"])?;
            drop(reader);
            // The long corpus verifies the current reader at every publication
            // above, and sweeps every retained reader at both distributions and
            // at the concrete close/reopen boundary. Avoid opening the entire
            // same history after each empty non-distribution period.
            let sweep = fixture["profile"] != "synthetic_mainnet_era"
                || observed.committed.period.as_u64().is_multiple_of(100)
                || observed.committed.period.as_u64() == 25_706_905
                || index + 1 == periods.len();
            if sweep {
                verify_retained(concrete_path, observed.committed, fixture)?;
            }
            Ok(())
        };
        execute()
            .with_context(|| format!("native integration period {}", number(&period["number"])))?;
        epoch = adapter.inner.state_api_epoch;
        if adapter.fault != mixed_recovery::Fault::None {
            return Ok(());
        }
        // Reopen the consumed storage handle without replacing the live
        // FinalChain scheduler or the process-local StateAPI epoch.
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, identity)?;
        concrete = ConcreteStateLifecycle::open(concrete_path, identity, observed.committed)?;
    }
    drop(concrete);
    drop(chain);
    drop(application);
    Ok(())
}
