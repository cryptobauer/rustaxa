//! Actual-coordinate, mainnet-policy consumers of the separate synthetic corpus.
use super::*;
use std::collections::BTreeMap;
use std::io::Read;
#[path = "mainnet_era_api.rs"]
mod api;
#[path = "mainnet_era_code_objects.rs"]
mod code_objects;
#[path = "mainnet_era_donor_negatives.rs"]
mod donor_negatives;

fn corpus(name: &str) -> Result<Value> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        directory.join(format!("{name}.manifest.json")),
    )?)?;
    ensure!(manifest["references"]["public"] == "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b");
    ensure!(manifest["references"]["local"] == "bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418");
    let pin = &manifest["streams"]["local"];
    let file = string(&pin["file"]);
    ensure!(Path::new(file).components().count() == 1);
    let stored = std::fs::read(directory.join(file))?;
    if file.ends_with(".gz") {
        let hash = revm::precompile::hash::sha256_run(&stored, u64::MAX)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        ensure!(
            stored.len() as u64 == number(&pin["gzip_bytes"])
                && hex::encode(hash.bytes) == string(&pin["gzip_sha256"]),
            "compressed corpus identity"
        );
    }
    let mut body = Vec::new();
    flate2::read::GzDecoder::new(stored.as_slice()).read_to_end(&mut body)?;
    let hash = revm::precompile::hash::sha256_run(&body, u64::MAX)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(
        hex::encode(hash.bytes) == string(&pin["sha256"])
            && body.len() as u64 == number(&pin["bytes"])
    );
    let mut fixture: Value = serde_json::from_slice(&body)?;
    code_objects::decode(&mut fixture)?;
    ensure!(
        fixture["profile"] == "synthetic_mainnet_era"
            && fixture["configuration"]["chain_id"] == 841
    );
    ensure!(fixture["configuration"]["dpos"]["delegation_delay"] == 5);
    Ok(fixture)
}

fn pinned_document(pin: &Value) -> Result<Value> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let file = string(&pin["file"]);
    ensure!(Path::new(file).components().count() == 1, "corpus path");
    let stored = std::fs::read(directory.join(file))?;
    let mut body = Vec::new();
    if file.ends_with(".gz") {
        flate2::read::GzDecoder::new(stored.as_slice()).read_to_end(&mut body)?;
    } else {
        body = stored;
    }
    let hash = revm::precompile::hash::sha256_run(&body, u64::MAX)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(
        hex::encode(hash.bytes) == string(&pin["sha256"])
            && body.len() as u64 == number(&pin["bytes"]),
        "corpus body identity"
    );
    let mut document = serde_json::from_slice(&body)?;
    code_objects::decode(&mut document)?;
    Ok(document)
}

pub(super) fn verify_committed_phase(
    path: &Path,
    identity: ConcreteStateIdentity,
    fixture: &Value,
    phase: &Value,
) -> Result<()> {
    api::verify_committed_phase(path, identity, fixture, phase)
}

pub(super) fn compare_distribution_writes(
    context: &FinalChainNativeRewardsContext,
    period: &Value,
) -> Result<()> {
    let mut offset = 0;
    for row in period["distributions"]
        .as_array()
        .context("scheduled distributions")?
    {
        let writes = row["ordered_raw_writes"]
            .as_array()
            .context("distribution writes")?;
        ensure!(
            offset + writes.len() <= context.raw_mutations.len(),
            "distribution occurrence count"
        );
        let part = FinalChainNativeRewardsContext {
            raw_mutations: context.raw_mutations[offset..offset + writes.len()].to_vec(),
            ..Default::default()
        };
        native_integration::compare_reward_map(
            &part,
            &row["ordered_raw_writes"],
            &serde_json::json!([]),
            &row["input"],
        )?;
        offset += writes.len();
    }
    let end = period["ordered_raw_writes"]["end_block"]
        .as_array()
        .context("end-block writes")?;
    ensure!(
        context.raw_mutations.len() == offset + end.len(),
        "complete terminal writes"
    );
    for (actual, expected) in context.raw_mutations[offset..].iter().zip(end) {
        compare_raw_mutation(actual, expected)?;
    }
    Ok(())
}

fn pending_fact(row: &Value) -> Result<rustaxa_consensus::FinalizedRewardsPeriodFact> {
    use rustaxa_consensus::{
        FinalizedRewardsPeriodFact, RewardDagBlockFact, RewardTransactionFact,
    };
    let planner = &row["planner_facts"];
    Ok(FinalizedRewardsPeriodFact {
        period: number(&row["number"]),
        block_author: H160(fixed(&row["block_author"])),
        blocks_per_year: number(&row["blocks_per_year"]).try_into()?,
        dpos_eligible_total_vote_count: number(&planner["total_eligible_vote_count"]),
        transactions: row["transactions"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|tx| {
                Ok(RewardTransactionFact {
                    hash: H256(fixed(&tx["hash"])),
                    gas_price: decimal_u256(&tx["gas_price"]),
                    gas_used: number(&tx["gas_used"]).into(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
        dag_blocks: planner["dag_blocks"]
            .as_array()
            .context("pending DAG facts")?
            .iter()
            .map(|dag| {
                Ok(RewardDagBlockFact {
                    author: H160(fixed(&dag["author"])),
                    difficulty: number(&dag["difficulty"]).try_into()?,
                    transaction_hashes: dag["transaction_hashes"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .map(|hash| H256(fixed(hash)))
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
        cert_votes: planner["certificate_votes"]
            .as_array()
            .context("pending vote facts")?
            .iter()
            .map(|vote| RewardCertVoteFact {
                voter: H160(fixed(&vote["validator"])),
                period: number(&vote["period"]),
                weight: number(&vote["weight"]),
            })
            .collect(),
    })
}

/// Rebuilds a fresh target from actual Rust donor account/code/raw-slot reads.
/// The Go seed supplies only authenticated catalog identities and comparison
/// facts; no Go native snapshot or expected root is installed by this helper.
fn concrete_from_donor(
    path: &Path,
    chain: &FinalChain,
    seed: &Value,
    first: FinalChainBlockNumber,
    source: &native_integration::api::CompleteReader<ConcreteStateReader>,
) -> Result<ConcreteStateLifecycle> {
    let mut accounts = Vec::new();
    let mut code = BTreeMap::new();
    let mut account_ids = BTreeSet::new();
    for expected in seed["native_catalog"]["accounts"]
        .as_array()
        .context("complete donor account identities")?
    {
        let address = fixed(&expected["address"]);
        ensure!(
            account_ids.insert(address),
            "duplicate donor account identity"
        );
        match source.account(address)? {
            ConcreteRead::Present(actual) => {
                ensure!(
                    expected["present"] == true,
                    "donor source/target account presence"
                );
                if let Some(hash) = actual.account.code_hash {
                    let ConcreteRead::Present(body) = source.code(hash)? else {
                        bail!("actual donor code missing")
                    };
                    ensure!(
                        body.len() as u64 == actual.account.code_size && keccak256(&body).0 == hash,
                        "actual Rust donor code identity"
                    );
                    if let Some(previous) = code.insert(hash, body.clone()) {
                        ensure!(previous == body, "conflicting actual donor code");
                    }
                } else {
                    ensure!(
                        actual.account.code_size == 0,
                        "donor code size without hash"
                    );
                }
                // The incremental writer computes the new storage root from the
                // actual logical slots below; never carry over an expected root.
                accounts.push(ConcreteAccountMutation::Upsert {
                    address,
                    record: record(
                        actual.account.nonce.clone(),
                        actual.account.balance.clone(),
                        actual.account.code_hash,
                        actual.account.code_size,
                    ),
                });
            }
            ConcreteRead::Absent | ConcreteRead::Tombstone => {
                ensure!(
                    expected["present"] == false,
                    "Go-present donor account missing in actual Rust source"
                );
            }
        }
    }
    let mut storage = Vec::new();
    let mut catalog = Vec::new();
    let mut slot_ids = BTreeSet::new();
    for expected in seed["native_catalog"]["slots"]
        .as_array()
        .context("complete donor raw-slot identities")?
    {
        let slot = ConcreteStorageSlot {
            address: fixed(&expected["address"]),
            key: fixed(&expected["key"]),
        };
        ensure!(
            slot_ids.insert(slot) && account_ids.contains(&slot.address),
            "duplicate donor slot or missing account identity"
        );
        let value = match source.storage(slot.address, ConcreteStorageKey(slot.key))? {
            ConcreteRead::Present(value) => value,
            ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
        };
        ensure!(
            value == bytes(&expected["value"])
                && (expected["present"] == true) == !value.is_empty(),
            "actual donor source/declared target raw value"
        );
        if !value.is_empty() {
            storage.push(ConcreteStorageMutation {
                address: slot.address,
                key: ConcreteStorageKey(slot.key),
                value: Some(value),
            });
            catalog.push(slot);
        }
    }
    let batch = rustaxa_storage::ConcreteStateMutationBatch {
        accounts,
        storage,
        code: code
            .into_iter()
            .map(|(code_hash, code)| ConcreteCodeInsertion { code_hash, code })
            .collect(),
    };
    Ok(ConcreteStateLifecycle::create_synthetic_exclusive(
        path,
        chain.concrete_chain_identity()?,
        first,
        batch,
        catalog,
    )?)
}

/// Sweeps distinct retained coordinates using the complete source-bounded CF5
/// inventory and actual trie nonmembership proof for each unknown raw key.
fn verify_complete_history(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
) -> Result<()> {
    for state in fixture["seed"]["history"]
        .as_array()
        .context("retained initialization states")?
    {
        let selected = ConcreteStateIdentity {
            period: number(&state["period"]).into(),
            state_root: fixed(&state["root"]),
        };
        let reader = native_integration::api::CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(path, committed, selected)?,
            known: api::complete_inventory(fixture, selected.period.as_u64())?,
        };
        let mut phase = fixture["seed"].clone();
        phase["accounts"] = fixture["seed"]["native_catalog"]["accounts"].clone();
        phase["root"] = state["root"].clone();
        verify_final_reader(&reader, &phase)?;
    }
    for period in fixture["periods"]
        .as_array()
        .context("retained actual periods")?
    {
        let selected = ConcreteStateIdentity {
            period: number(&period["number"]).into(),
            state_root: fixed(&period["final"]["root"]),
        };
        if selected.period > committed.period {
            break;
        }
        let reader = native_integration::api::CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(path, committed, selected)?,
            known: api::complete_inventory(fixture, selected.period.as_u64())?,
        };
        verify_final_reader(&reader, &period["final"])?;
    }
    Ok(())
}

fn bootstrap(root: &Path, fixture: &Value) -> Result<()> {
    bootstrap_inner(root, fixture, 0)
}

/// Root depth0 admits at most three donor edges. Every donor is independently
/// bootstrapped and executed in a new child directory, then captured from its
/// actual finalized Rust owner. Target initialization never clones its reward
/// cache, StateAPI epoch or process-local cleanup timer.
fn bootstrap_inner(root: &Path, fixture: &Value, depth: usize) -> Result<()> {
    ensure!(depth <= 3, "maximum three synthetic donor edges");
    let seed = &fixture["seed"];
    let history = seed["history"]
        .as_array()
        .context("complete delayed seed history")?;
    ensure!(history.len() == 6, "six explicit prehistory states");
    let first = number(&history[0]["period"]);
    ensure!(
        history
            .iter()
            .enumerate()
            .all(|(i, row)| number(&row["period"]) == first + i as u64),
        "consecutive seed history"
    );
    ensure!(
        number(&history[5]["period"]) == number(&seed["period"]),
        "seed terminal coordinate"
    );
    let source = if let Some(donor) = seed.get("donor_document") {
        ensure!(
            depth < 3 && donor.is_object(),
            "bounded complete donor document"
        );
        ensure!(
            fixture["configuration"] == donor["configuration"]
                && fixture["inputs"] == donor["inputs"],
            "donor configuration/allocation inputs differ"
        );
        for label in [
            "profile",
            "oracle_lifecycle",
            "reference",
            "reference_pin",
            "observer",
            "lifecycle",
        ] {
            ensure!(
                fixture.get(label) == donor.get(label),
                "donor semantic label mismatch: {label}"
            );
        }
        ensure!(
            donor["seed_actions_only"] == true && donor.get("kind").is_none(),
            "actual fully committed donor prefix"
        );
        let periods = donor["periods"]
            .as_array()
            .context("actual donor periods")?;
        let last = periods.last().context("nonempty actual donor execution")?;
        let declared = ConcreteStateIdentity {
            period: number(&seed["donor_identity"]["period"]).into(),
            state_root: fixed(&seed["donor_identity"]["root"]),
        };
        ensure!(
            declared.period.as_u64() == number(&last["number"])
                && declared.state_root == fixed(&last["final"]["root"])
                && declared.state_root == fixed(&last["final"]["committed_root"])
                && declared.state_root == fixed(&seed["root"])
                && declared.period.as_u64() < first,
            "actual donor/target identity binding"
        );
        ensure!(
            history
                .iter()
                .all(|row| fixed::<32>(&row["root"]) == declared.state_root),
            "donor target retained roots"
        );
        ensure!(
            seed["native_catalog"] == seed["declared_donor_catalog"]
                && seed["declared_donor_catalog"] == last["final"]["native_catalog"],
            "complete donor/target catalog binding"
        );
        let donor_root = root.join("donor");
        std::fs::create_dir(&donor_root)?;
        bootstrap_inner(&donor_root, donor, depth + 1)?;
        native_integration::run_segment(
            donor,
            periods,
            &donor_root.join("application"),
            &donor_root.join("state_db"),
            false,
        )?;
        let (application, chain) = open_chain(&donor_root.join("application"), donor)?;
        let state_path = donor_root.join("state_db");
        let observed = ConcreteStateLifecycle::inspect_existing(
            &state_path,
            chain.concrete_chain_identity()?,
        )?;
        ensure!(
            observed.committed == declared
                && observed.pending_marker_rlp.is_empty()
                && chain.last_block_number_typed()? == declared.period
                && application
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    .is_none(),
            "actual Rust donor finalized identity"
        );
        verify_committed_phase(&state_path, observed.committed, donor, &last["final"])?;
        let reader = native_integration::api::CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(
                &state_path,
                observed.committed,
                observed.committed,
            )?,
            known: api::complete_inventory(donor, observed.committed.period.as_u64())?,
        };
        let owner = ConcreteStateLifecycle::open(
            &state_path,
            chain.concrete_chain_identity()?,
            observed.committed,
        )?;
        let token = chain.capture_synthetic_fixture_donor(&owner, &reader.inner, &reader.known)?;
        ensure!(
            token.source_identity() == declared,
            "opaque Rust donor token identity"
        );
        eprintln!(
            "Rust donor captured: depth {}, period {}, native snapshot {}",
            depth + 1,
            declared.period.as_u64(),
            hex::encode(token.native_snapshot_hash())
        );
        drop(owner);
        drop(chain);
        drop(application);
        Some((token, reader))
    } else {
        ensure!(
            seed.get("donor_identity").is_none(),
            "donor identity without actual donor document"
        );
        None
    };
    let (application, chain) = open_chain(&root.join("application"), fixture)?;
    let accounts = seed["native_catalog"]["accounts"]
        .as_array()
        .context("seed accounts")?
        .iter()
        .filter(|row| row["present"] == true)
        .map(|row| fixed(&row["address"]))
        .collect::<Vec<_>>();
    let slots = seed["native_catalog"]["slots"]
        .as_array()
        .context("seed raw slots")?
        .iter()
        .filter(|row| row["present"] == true)
        .map(|row| ConcreteStorageSlot {
            address: fixed(&row["address"]),
            key: fixed(&row["key"]),
        })
        .collect::<Vec<_>>();
    let native_slots = slots
        .iter()
        .copied()
        .filter(|slot| {
            let mut dpos = [0u8; 20];
            dpos[19] = 0xfe;
            let mut slashing = [0u8; 20];
            slashing[19] = 0xee;
            slot.address == dpos || slot.address == slashing
        })
        .collect::<Vec<_>>();
    let mut seed_fixture = fixture.clone();
    seed_fixture["genesis"] = seed.clone();
    let mut concrete = if let Some((_, reader)) = &source {
        concrete_from_donor(&root.join("state_db"), &chain, seed, first.into(), reader)?
    } else {
        mixed_genesis::construct_concrete(
            &root.join("state_db"),
            &chain,
            &seed_fixture,
            Some(first.into()),
        )?
    };
    for (index, state) in history.iter().enumerate() {
        if index != 0 {
            concrete =
                concrete.initialize_synthetic_successor(Default::default(), slots.clone())?;
        }
        let observed = concrete.observation()?;
        ensure!(observed.committed.period.as_u64() == number(&state["period"]));
        ensure!(
            observed.committed.state_root == fixed(&state["root"]),
            "computed seed root"
        );
        ensure!(
            observed.pending_marker_rlp.is_empty(),
            "initialization has no execution marker"
        );
        let pending = fixture["pending_reward_facts"]
            .as_array()
            .context("pending reward facts")?
            .iter()
            .filter(|row| number(&row["number"]) <= observed.committed.period.as_u64())
            .map(pending_fact)
            .collect::<Result<Vec<_>>>()?;
        if let Some((token, _)) = &source {
            ensure!(
                observed.committed.state_root == token.source_identity().state_root,
                "independently rebuilt Rust donor root"
            );
            chain.initialize_synthetic_fixture_snapshot_from_donor(
                &concrete,
                &accounts,
                &native_slots,
                pending,
                token,
            )?;
        } else {
            chain.initialize_synthetic_fixture_snapshot(&concrete, &accounts, &slots, pending)?;
        }
    }
    drop(concrete);
    drop(chain);
    drop(application);
    drop(source);
    Ok(())
}

/// Public Go has no transaction-phase root observer. Retain its exact raw
/// transcripts and use the independently pinned local intermediate witnesses
/// solely to check the incremental writer between equal signed transactions.
/// Final roots, rewards, APIs and every terminal raw write come from public Go.
fn public_batched_fixture(name: &str) -> Result<Value> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        directory.join(format!("{name}.manifest.json")),
    )?)?;
    let mut public = pinned_document(&manifest["streams"]["public"])?;
    let local = pinned_document(&manifest["streams"]["local"])?;
    attach_public_witnesses(&mut public, &local, &manifest["references"]["local"], 0)?;
    Ok(public)
}

/// Add only independently bound local intermediate witnesses. Public final
/// roots, raw native writes, receipts, signed transactions and APIs stay public.
/// Every donor uses the same public-batched native policy as its target.
fn attach_public_witnesses(
    public: &mut Value,
    local: &Value,
    reference: &Value,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 3, "maximum three donor witness edges");
    ensure!(
        public["configuration"] == local["configuration"] && public["inputs"] == local["inputs"],
        "per-pin witness configuration/input mismatch"
    );
    ensure!(
        public["scenario"] == local["scenario"]
            && public["seed"]["period"] == local["seed"]["period"]
            && public["seed"]["root"] == local["seed"]["root"],
        "per-pin witness seed identity mismatch"
    );
    public["oracle_lifecycle"] = Value::String("public_batched".into());
    public["observer_witness_reference"] = reference.clone();
    match (
        public
            .get_mut("seed")
            .and_then(|seed| seed.get_mut("donor_document")),
        local
            .get("seed")
            .and_then(|seed| seed.get("donor_document")),
    ) {
        (Some(source), Some(witness)) => {
            ensure!(depth < 3, "maximum three donor witness edges");
            attach_public_witnesses(source, witness, reference, depth + 1)?;
        }
        (None, None) => {}
        _ => bail!("per-pin donor witness structure differs"),
    }
    let periods = public["periods"].as_array_mut().context("public periods")?;
    let witnesses = local["periods"].as_array().context("local witnesses")?;
    ensure!(periods.len() == witnesses.len());
    for (period, witness) in periods.iter_mut().zip(witnesses) {
        ensure!(
            period["number"] == witness["number"]
                && period["final"]["root"] == witness["final"]["root"],
            "per-mode final root disagreement"
        );
        ensure!(
            period["period_catalog_identities"].is_null(),
            "public has no native observer"
        );
        period["period_catalog_identities"] = witness["period_catalog_identities"].clone();
        // Public's exact terminal writes can touch a different period catalog
        // than the local lifecycle. Bind those identities from the public raw
        // transcript; never move or substitute either mode's write effects.
        let terminal = period["ordered_raw_writes"]["end_block"]
            .as_array()
            .context("public terminal writes")?
            .clone();
        for raw in terminal {
            let address = Value::String(format!("0x{}", string(&raw["address"])));
            let slot = serde_json::json!({"address": address, "key": format!("0x{}", string(&raw["key"]))});
            let accounts = period["period_catalog_identities"]["accounts"]
                .as_array_mut()
                .context("witness accounts")?;
            if !accounts.contains(&address) {
                accounts.push(address);
            }
            let slots = period["period_catalog_identities"]["slots"]
                .as_array_mut()
                .context("witness slots")?;
            if !slots.contains(&slot) {
                slots.push(slot);
            }
        }
        let txs = period["transactions"]
            .as_array_mut()
            .context("public transactions")?;
        let observed = witness["transactions"]
            .as_array()
            .context("witness transactions")?;
        ensure!(txs.len() == observed.len());
        for (tx, witness) in txs.iter_mut().zip(observed) {
            let mut semantic = witness.clone();
            let observer = semantic
                .as_object_mut()
                .context("witness transaction")?
                .remove("observer")
                .context("actual intermediate observation")?;
            ensure!(
                *tx == semantic,
                "signed transaction/result/raw-write witness mismatch"
            );
            tx["observer"] = observer;
        }
    }
    Ok(())
}

#[test]
fn mainnet_public_batched_lifecycle_preserves_exact_cleanup() -> Result<()> {
    let fixture = public_batched_fixture("mainnet_106")?;
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent RUN_ARTIFACT_DIR")?;
    let root = base.join(format!(
        "mainnet-public-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    bootstrap(&root, &fixture)?;
    let periods = fixture["periods"].as_array().context("public periods")?;
    let split = periods
        .iter()
        .position(|row| number(&row["number"]) == 25_706_906)
        .context("public reopen")?;
    native_integration::run_segment(
        &fixture,
        &periods[..split],
        &root.join("application"),
        &root.join("state_db"),
        false,
    )?;
    native_integration::run_segment(
        &fixture,
        &periods[split..],
        &root.join("application"),
        &root.join("state_db"),
        false,
    )?;
    api::verify(&fixture, &root.join("application"), &root.join("state_db"))?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("mainnet_106.manifest.json"))?)?;
    let estimates = pinned_document(&manifest["cpp_estimates"])?;
    api::verify_cpp_estimates(&fixture, &estimates, "public")?;
    for period in [25_706_900u64, 25_707_000] {
        let fork = pinned_document(&manifest["pre_reward_forks"]["public"][period.to_string()])?;
        api::verify_pre_reward(
            &fixture,
            &fork,
            &root.join("application"),
            &root.join("state_db"),
        )?;
        api::verify_pre_reward_negatives(
            &fixture,
            &fork,
            &root.join("application"),
            &root.join("state_db"),
        )?;
        api::verify_cpp_estimates(&fork, &estimates, &format!("public/pre_reward/{period}"))?;
    }
    era_retention::finish(
        scratch,
        era_retention::final_evidence(
            &root,
            &fixture,
            "111 periods, physical reopen, receipts/native/history,56 APIs, both private forks/33 negatives, C++ estimates; all owners closed",
        )?,
        true,
    )?;
    eprintln!(
        "public batched lifecycle:111 periods, exact public EndBlock writes, separate local intermediate witnesses"
    );
    Ok(())
}

#[test]
fn mainnet_106_signed_periods_publish_and_reopen() -> Result<()> {
    let fixture = corpus("mainnet_106")?;
    let periods = fixture["periods"].as_array().context("periods")?;
    let workload = periods
        .iter()
        .filter(|row| row["seed_prefix"] == false)
        .collect::<Vec<_>>();
    ensure!(
        workload.len() == 106
            && number(&workload[0]["number"]) == 25_706_895
            && number(&workload[105]["number"]) == 25_707_000
    );
    ensure!(
        fixture["pending_reward_facts"]
            .as_array()
            .context("94 pending rows")?
            .len()
            == 94
    );
    ensure!(
        fixture["pending_reward_facts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| number(&row["number"]))
            .eq(25_706_801..=25_706_894),
        "complete chronological pending facts"
    );
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent RUN_ARTIFACT_DIR")?;
    let reuse = std::env::var_os("MAINNET_ERA_API_PAIR").map(PathBuf::from);
    let mut scratch = None;
    let root = if let Some(root) = reuse {
        ensure!(
            std::fs::canonicalize(&root)?.starts_with(std::fs::canonicalize(&base)?),
            "API pair must be task-owned"
        );
        let (_storage, chain) = open_chain(&root.join("application"), &fixture)?;
        let observed = ConcreteStateLifecycle::inspect_existing(
            root.join("state_db"),
            chain.concrete_chain_identity()?,
        )?;
        ensure!(
            observed.committed.period.as_u64() == 25_707_000
                && observed.pending_marker_rlp.is_empty()
        );
        ensure!(observed.committed.state_root == fixed(&periods.last().unwrap()["final"]["root"]));
        native_integration::verify_retained(&root.join("state_db"), observed.committed, &fixture)?;
        for period in periods {
            verify_receipts(&_storage, period)?;
        }
        eprintln!(
            "API-only qualification over existing captured task pair: {}",
            root.display()
        );
        root
    } else {
        let root = base.join(format!(
            "mainnet-106-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        scratch = era_retention::register(&root, &fixture)?;
        std::fs::create_dir(&root)?;
        era_retention::created(&mut scratch)?;
        bootstrap(&root, &fixture)?;
        let split = periods
            .iter()
            .position(|row| number(&row["number"]) == 25_706_906)
            .context("close/reopen boundary")?;
        native_integration::run_segment(
            &fixture,
            &periods[..split],
            &root.join("application"),
            &root.join("state_db"),
            false,
        )?;
        native_integration::run_segment(
            &fixture,
            &periods[split..],
            &root.join("application"),
            &root.join("state_db"),
            false,
        )?;
        root
    };
    let baseline_dependency = scratch.as_ref().map(era_retention::Case::dependent);
    api::verify(&fixture, &root.join("application"), &root.join("state_db"))?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("mainnet_106.manifest.json"))?)?;
    let estimates = pinned_document(&manifest["cpp_estimates"])?;
    api::verify_cpp_estimates(&fixture, &estimates, "local")?;
    for period in [25_706_900u64, 25_707_000] {
        let fork = pinned_document(&manifest["pre_reward_forks"]["local"][period.to_string()])?;
        api::verify_pre_reward(
            &fixture,
            &fork,
            &root.join("application"),
            &root.join("state_db"),
        )?;
        api::verify_cpp_estimates(&fork, &estimates, &format!("local/pre_reward/{period}"))?;
        api::verify_pre_reward_negatives(
            &fixture,
            &fork,
            &root.join("application"),
            &root.join("state_db"),
        )?;
    }
    if std::env::var_os("MAINNET_ERA_API_PAIR").is_none() {
        for fault in [
            mixed_recovery::Fault::BeforeCommit,
            mixed_recovery::Fault::AfterCommit,
        ] {
            let recovered = base.join(format!(
                "mainnet-recovery-{}-{fault:?}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            let mut recovery_scratch = era_retention::register(&recovered, &fixture)?;
            std::fs::create_dir(&recovered)?;
            era_retention::created(&mut recovery_scratch)?;
            bootstrap(&recovered, &fixture)?;
            mixed_recovery::verify_mainnet_recovery(&fixture, &root, &recovered, fault)?;
            era_retention::finish(
                recovery_scratch,
                era_retention::final_evidence(
                    &recovered,
                    &fixture,
                    &format!(
                        "{fault:?}:actual interruption,repeated recovery,retry,full continuation,final facts equal baseline,retained readers; all owners closed"
                    ),
                )?,
                false,
            )?;
        }
    }
    drop(baseline_dependency);
    era_retention::finish(
        scratch,
        era_retention::final_evidence(
            &root,
            &fixture,
            "111 periods,94pendingfacts,physical reopen,receipt/native/history,56APIs,privateforks33negatives,C++estimates,BeforeCommit/AfterCommit repeated recovery+continuation complete; all dependencies closed",
        )?,
        true,
    )?;
    Ok(())
}

#[test]
fn mainnet_seed_rejects_incomplete_and_inconsistent_provenance() -> Result<()> {
    let fixture = corpus("mainnet_106")?;
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent RUN_ARTIFACT_DIR")?;
    for case in 0..6 {
        let mut invalid = fixture.clone();
        match case {
            0 => {
                invalid["seed"]["history"].as_array_mut().unwrap().remove(2);
            }
            1 => {
                invalid["seed"]["history"][1]["period"] =
                    invalid["seed"]["history"][0]["period"].clone();
            }
            2 => {
                invalid["seed"]["native_catalog"]["slots"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|row| row["present"] != true);
            }
            3 => {
                invalid["seed"]["native_catalog"]["accounts"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|row| row["present"] != true);
            }
            4 => {
                invalid["pending_reward_facts"]
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
            }
            5 => {
                invalid["seed"]["history"][0]["root"] = Value::String("00".repeat(32));
            }
            _ => unreachable!(),
        }
        let root = base.join(format!(
            "seed-negative-{}-{case}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut scratch = era_retention::register(&root, &invalid)?;
        std::fs::create_dir(&root)?;
        era_retention::created(&mut scratch)?;
        let error = bootstrap(&root, &invalid).expect_err("invalid seed must fail");
        let required = [
            "six explicit prehistory states",
            "consecutive seed history",
            "computed concrete genesis root",
            "live storage insertion has no prior or upserted account",
            "SYNTHETIC_SEED_PENDING_REWARD_INTERVAL",
            "computed seed root",
        ][case];
        ensure!(
            format!("{error:#}").contains(required),
            "expected seed rejection {required}, got {error:#}"
        );
        era_retention::finish(
            scratch,
            serde_json::json!({"mutation":case,"actual_error":format!("{error:#}"),"baseline_fixture_unchanged":true,"all_handles_closed":true}),
            false,
        )?;
    }
    Ok(())
}

#[test]
fn cacti_signed_boundary_donor_reopens_retained_history_and_apis() -> Result<()> {
    cacti_workload(corpus("cacti")?, "local")
}

#[test]
fn cacti_public_batched_boundary_donor_reopens_retained_history_and_apis() -> Result<()> {
    cacti_workload(public_batched_fixture("cacti")?, "public")
}

fn cacti_workload(fixture: Value, pin_label: &str) -> Result<()> {
    let periods = fixture["periods"]
        .as_array()
        .context("complete Cacti execution")?;
    ensure!(
        periods.len() == 105
            && number(&periods[0]["number"]) == 24_350_796
            && number(&periods[104]["number"]) == 24_350_900,
        "Cacti requires complete warmup, actual boundary and next distribution"
    );
    ensure!(
        periods
            .windows(2)
            .all(|pair| number(&pair[0]["number"]) + 1 == number(&pair[1]["number"]))
    );
    for required in [24_350_800u64, 24_350_801, 24_350_802] {
        ensure!(
            periods.iter().any(|row| number(&row["number"]) == required),
            "required actual Cacti boundary cell"
        );
    }
    ensure!(
        fixture["seed"]["donor_document"].is_object(),
        "Cacti custody/jail state needs actual donor execution"
    );
    ensure!(
        fixture["api_cases"]
            .as_array()
            .context("Cacti API cases")?
            .len()
            == 42,
        "six complete Cacti API heads"
    );
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent RUN_ARTIFACT_DIR")?;
    let mut scratch = None;
    let root = if let Some(path) = std::env::var_os(format!(
        "CACTI_ERA_API_{}_PAIR",
        pin_label.to_ascii_uppercase()
    )) {
        let root = PathBuf::from(path);
        ensure!(
            std::fs::canonicalize(&root)?.starts_with(std::fs::canonicalize(&base)?),
            "API pair must be task-owned"
        );
        eprintln!(
            "Cacti {pin_label} API-only qualification over captured task pair:{}",
            root.display()
        );
        root
    } else {
        let root = base.join(format!(
            "cacti-boundary-{pin_label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        scratch = era_retention::register(&root, &fixture)?;
        std::fs::create_dir(&root)?;
        era_retention::created(&mut scratch)?;
        bootstrap(&root, &fixture)?;
        // Preserve the source timer. Reopen both owners for all checks below.
        native_integration::run_segment(
            &fixture,
            periods,
            &root.join("application"),
            &root.join("state_db"),
            false,
        )?;
        root
    };
    {
        let (application, chain) = open_chain(&root.join("application"), &fixture)?;
        let observed = ConcreteStateLifecycle::inspect_existing(
            root.join("state_db"),
            chain.concrete_chain_identity()?,
        )?;
        ensure!(
            observed.committed.period.as_u64() == 24_350_900
                && observed.committed.state_root == fixed(&periods[104]["final"]["root"])
                && observed.pending_marker_rlp.is_empty()
                && application
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    .is_none()
        );
        for period in periods {
            verify_receipts(&application, period)?;
        }
        verify_complete_history(&root.join("state_db"), observed.committed, &fixture)?;
    }
    api::verify(&fixture, &root.join("application"), &root.join("state_db"))?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("cacti.manifest.json"))?)?;
    ensure!(
        manifest["cpp_estimates"].is_object(),
        "actual Cacti C++ estimation capture required"
    );
    let estimates = pinned_document(&manifest["cpp_estimates"])?;
    api::verify_cpp_estimates(&fixture, &estimates, pin_label)?;
    era_retention::finish(
        scratch,
        era_retention::final_evidence(
            &root,
            &fixture,
            "actual Rust donor, six recomputed initialized states,105periods,receipts/native/history,all retained readers after physical reopen,42APIs381probes84traces,C++estimates; all owners closed",
        )?,
        true,
    )?;
    eprintln!(
        "Cacti {pin_label} focused execution:105 periods through24350900, actual Rust donor, six initialization readers plus all retained execution readers after physical close/reopen,42 API cases and actual pinned C++ estimate results"
    );
    Ok(())
}
