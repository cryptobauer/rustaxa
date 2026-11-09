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
#[path = "mainnet_era_duration.rs"]
pub(super) mod duration;
#[path = "legacy_quote_edges.rs"]
mod legacy_quote_edges;

// Bind the final manifest containing the actual Go streams and C++ estimates.
const MAGNOLIA_FULL_MANIFEST_SHA256: &str =
    "f1b57cbcc19e45a84879431540f9de19e14d8fe7a3ff12a9c67ed97c1c62535c";

fn magnolia_full_manifest() -> Result<Value> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let bytes = std::fs::read(directory.join("magnolia_first_full_distribution.manifest.json"))?;
    let manifest = authenticated_magnolia_full_bytes(&bytes)?;
    ensure!(
        manifest["references"]["public"] == "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b"
            && manifest["references"]["local"] == "bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418"
    );
    Ok(manifest)
}

fn authenticated_magnolia_full_bytes(bytes: &[u8]) -> Result<Value> {
    let hash = revm::precompile::hash::sha256_run(bytes, u64::MAX)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(
        hex::encode(hash.bytes) == MAGNOLIA_FULL_MANIFEST_SHA256,
        "final full-batch manifest identity required"
    );
    Ok(serde_json::from_slice(bytes)?)
}

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

/// Supplies the independently captured C++ pre-Aspen scheduling input to the
/// existing fixture port. This is an ordering input, never a root/reward result.
/// Production scheduler composition must separately prove the same ordering.
pub(super) fn bind_legacy_scheduler_order(
    chain: &FinalChain,
    fixture: &Value,
    period: &Value,
) -> Result<()> {
    if fixture["profile"] != "synthetic_mainnet_era"
        || number(&period["number"])
            >= number(&fixture["configuration"]["hardforks"]["aspen"]["part_one_block"])
    {
        return Ok(());
    }
    let order = period["distribution_order"]
        .as_array()
        .context("captured C++ legacy order")?
        .iter()
        .map(number)
        .collect::<Vec<_>>();
    let distributions = period["distributions"]
        .as_array()
        .context("complete source period invocations")?;
    ensure!(
        order
            .iter()
            .copied()
            .eq(distributions.iter().map(|row| number(&row["period"]))),
        "scheduler input differs from pinned invocation vector"
    );
    if !order.is_empty() {
        chain.configure_synthetic_legacy_distribution_order(number(&period["number"]), order)?;
    }
    Ok(())
}

pub(super) fn compare_distribution_writes(
    context: &FinalChainNativeRewardsContext,
    period: &Value,
) -> Result<()> {
    let mut offset = 0;
    for (source_index, row) in period["distributions"]
        .as_array()
        .context("scheduled distributions")?
        .iter()
        .enumerate()
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
        compare_period_distribution_map(
            &part,
            writes,
            &row["input"],
            number(&period["number"]),
            source_index,
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

/// Match scalar barriers in their exact order around one disjoint reward map.
/// Payout coordinates select the native protocol; source indexes select the
/// one-time migration. No source row or scalar occurrence can cross this slice.
fn compare_period_distribution_map(
    context: &FinalChainNativeRewardsContext,
    writes: &[Value],
    input: &Value,
    payout_period: u64,
    source_index: usize,
) -> Result<()> {
    ensure!(
        context.raw_mutations.len() == writes.len(),
        "exact distribution slice"
    );
    if payout_period < 8_118_000 {
        return compare_pre_aspen_reward_map(context, writes, input);
    }
    if payout_period < 8_572_000 {
        let suffix = writes.last().context("Aspen1 minted suffix")?;
        check_dpos_scalar_identity(suffix, 6)?;
        compare_raw_mutation(context.raw_mutations.last().unwrap(), suffix)?;
        let map = FinalChainNativeRewardsContext {
            raw_mutations: context.raw_mutations[..writes.len() - 1].to_vec(),
            ..Default::default()
        };
        return compare_pre_aspen_reward_map(&map, &writes[..writes.len() - 1], input);
    }
    let prefix = if payout_period == 8_572_000 && source_index == 0 {
        ensure!(writes.len() >= 2, "complete Aspen2 migration prefix");
        check_dpos_scalar_identity(&writes[0], 7)?;
        let supply = hex::decode(
            writes[0]["value"]
                .as_str()
                .context("migration supply bytes")?,
        )?;
        ensure!(
            !supply.is_empty() && supply.len() <= 32 && supply[0] != 0,
            "positive canonical migration supply"
        );
        check_dpos_scalar_identity(&writes[1], 6)?;
        ensure!(
            writes[1]["value"].as_str() == Some(""),
            "migration counter deletion"
        );
        compare_raw_mutation(&context.raw_mutations[0], &writes[0])?;
        compare_raw_mutation(&context.raw_mutations[1], &writes[1])?;
        2
    } else {
        0
    };
    let map = FinalChainNativeRewardsContext {
        raw_mutations: context.raw_mutations[prefix..].to_vec(),
        ..Default::default()
    };
    native_integration::compare_reward_map(
        &map,
        &Value::Array(writes[prefix..].to_vec()),
        &serde_json::json!([]),
        input,
    )
}

fn check_dpos_scalar_identity(row: &Value, key: u8) -> Result<()> {
    let mut dpos = [0u8; 20];
    dpos[19] = 0xfe;
    ensure!(
        fixed::<20>(&row["address"]) == dpos && fixed::<32>(&row["key"]) == keccak256([key]).0,
        "exact DPoS scalar identity"
    );
    Ok(())
}

#[test]
fn aspen_comparator_rejects_scalar_and_source_boundary_forgery() -> Result<()> {
    use rustaxa_consensus::native_session::{
        FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRawValue,
    };
    // Validator-only traces. They never enter an execution owner or a writer;
    // the full paired workloads must independently compute all actual effects.
    let validation_context = |period: &Value| -> Result<FinalChainNativeRewardsContext> {
        let rows = period["distributions"]
            .as_array()
            .context("distribution rows")?
            .iter()
            .flat_map(|row| row["ordered_raw_writes"].as_array().unwrap().iter())
            .chain(
                period["ordered_raw_writes"]["end_block"]
                    .as_array()
                    .unwrap()
                    .iter(),
            );
        let raw_mutations = rows
            .map(|row| {
                let value = bytes(&row["value"]);
                Ok(FinalChainNativeRawMutation {
                    address: fixed(&row["address"]),
                    key: ConcreteStorageKey(fixed(&row["key"])),
                    expected: ConcreteRead::Absent,
                    operation: if value.is_empty() {
                        FinalChainNativeRawOperation::Delete
                    } else {
                        FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(value)?)
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(FinalChainNativeRewardsContext {
            raw_mutations,
            ..Default::default()
        })
    };
    for (family, boundary) in [("aspen_one", 8_118_000u64), ("aspen_two", 8_572_000u64)] {
        for fixture in [corpus(family)?, public_batched_fixture(family)?] {
            let period = fixture["periods"]
                .as_array()
                .context("periods")?
                .iter()
                .find(|row| number(&row["number"]) == boundary)
                .context("Aspen payout")?;
            let context = validation_context(period)?;
            compare_distribution_writes(&context, period)?;
            let mut permuted = period.clone();
            let map_start = if family == "aspen_one" { 1 } else { 4 };
            permuted["distributions"][0]["ordered_raw_writes"]
                .as_array_mut()
                .unwrap()
                .swap(map_start, map_start + 1);
            compare_distribution_writes(&context, &permuted)?;
            let count = if family == "aspen_one" { 7 } else { 12 };
            for case in 0..count {
                let mut forged = period.clone();
                if case == count - 1 {
                    forged["distributions"].as_array_mut().unwrap().swap(0, 1);
                } else if family == "aspen_two" && case == 9 {
                    let prefix = period["distributions"][0]["ordered_raw_writes"]
                        .as_array()
                        .unwrap()[..2]
                        .to_vec();
                    forged["distributions"][1]["ordered_raw_writes"]
                        .as_array_mut()
                        .unwrap()
                        .splice(0..0, prefix);
                } else if family == "aspen_two" && case == 10 {
                    forged["number"] = serde_json::json!(boundary + 1);
                } else {
                    let writes = forged["distributions"][0]["ordered_raw_writes"]
                        .as_array_mut()
                        .unwrap();
                    let last = writes.len() - 1;
                    if family == "aspen_one" {
                        match case {
                            0 => {
                                writes.pop();
                            }
                            1 => writes[last]["value"] = serde_json::json!("01"),
                            2 => writes.push(writes[last].clone()),
                            3 => writes.swap(0, last),
                            4 => {
                                writes[last]["address"] =
                                    serde_json::json!("000000000000000000000000000000000000fa1c")
                            }
                            _ => writes[0]["value"] = serde_json::json!("01"),
                        }
                    } else {
                        match case {
                            0 => {
                                writes.remove(0);
                            }
                            1 => writes.swap(0, 1),
                            2 => writes[1]["value"] = serde_json::json!("01"),
                            3 => writes[0]["value"] = serde_json::json!("00"),
                            4 => {
                                writes[0]["address"] =
                                    serde_json::json!("000000000000000000000000000000000000fa1c")
                            }
                            5 => writes[0]["value"] = serde_json::json!("0001"),
                            6 => writes.push(writes[last].clone()),
                            7 => writes[last]["value"] = serde_json::json!("01"),
                            _ => {
                                writes.remove(2);
                            }
                        }
                    }
                }
                ensure!(
                    compare_distribution_writes(&context, &forged).is_err(),
                    "forged Aspen scalar/source boundary accepted {family}/{case}"
                );
            }
        }
    }
    Ok(())
}

/// Before Aspen there is no yield/supply barrier write. The optional author
/// bonus precedes the distinct validator map, exactly as in the pinned Go
/// reward contract. Only that disjoint map may permute; every value, occurrence
/// and author-prefix position remains exact (the accepted v2 comparison rule).
fn compare_pre_aspen_reward_map(
    context: &FinalChainNativeRewardsContext,
    writes: &[Value],
    input: &Value,
) -> Result<()> {
    let validators = input["validators"]
        .as_array()
        .context("legacy reward validators")?;
    ensure!(context.raw_mutations.len() == writes.len());
    ensure!(
        writes.len() == validators.len() || writes.len() == validators.len() + 1,
        "pre-Aspen exact reward shape"
    );
    let first = usize::from(writes.len() == validators.len() + 1);
    if first == 1 {
        let mut key = vec![0, 2];
        key.extend_from_slice(&fixed::<20>(&input["block_author"]));
        ensure!(
            fixed::<32>(&writes[0]["key"]) == keccak256(key).0,
            "legacy author bonus prefix"
        );
        compare_raw_mutation(&context.raw_mutations[0], &writes[0])?;
    }
    let mut remaining = writes[first..]
        .iter()
        .map(|row| {
            (
                (fixed::<20>(&row["address"]), fixed::<32>(&row["key"])),
                row,
            )
        })
        .collect::<BTreeMap<_, _>>();
    ensure!(
        remaining.len() == validators.len(),
        "one legacy write per disjoint validator"
    );
    let keys = validators
        .iter()
        .map(|validator| {
            let mut key = vec![0, 2];
            key.extend_from_slice(&fixed::<20>(&validator["validator"]));
            (
                fixed::<20>(&serde_json::json!(
                    "00000000000000000000000000000000000000fe"
                )),
                keccak256(key).0,
            )
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        remaining.keys().copied().collect::<BTreeSet<_>>() == keys,
        "legacy validator map identity"
    );
    for mutation in &context.raw_mutations[first..] {
        let expected = remaining
            .remove(&(mutation.address, mutation.key.0))
            .context("legacy validator occurrence")?;
        compare_raw_mutation(mutation, expected)?;
    }
    ensure!(remaining.is_empty());
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
        verify_complete_final_reader(&reader, &phase)?;
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
        verify_complete_final_reader(&reader, &period["final"])?;
    }
    Ok(())
}

/// Interpret only the authenticated Magnolia producer's nil pending slice.
/// Arrays keep their original path. Missing facts never acquire an empty value.
fn pending_reward_rows(fixture: &Value) -> Result<&[Value]> {
    match fixture.get("pending_reward_facts") {
        Some(Value::Array(rows)) => Ok(rows),
        Some(Value::Null) => {
            if fixture["scenario"] == "magnolia_first_full_distribution" {
                let manifest = magnolia_full_manifest()?;
                return nil_pending_full_distribution_rows(fixture, &manifest["scenario"]);
            }
            ensure!(fixture["scenario"] == "magnolia", "nil pending facts scope");
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../../experiments/evm_feasibility/fixtures/mainnet_era/magnolia.manifest.json",
            );
            let bytes = std::fs::read(path)?;
            let hash = revm::precompile::hash::sha256_run(&bytes, u64::MAX)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            ensure!(
                hex::encode(hash.bytes)
                    == "8417bda24d9e985f9e268b8ae9a1210528e3e4a34d75e07b0d560ce5c9aa36ba",
                "authenticated Magnolia pending input"
            );
            let manifest: Value = serde_json::from_slice(&bytes)?;
            nil_pending_reward_rows(fixture, &manifest["scenario"])
        }
        _ => anyhow::bail!("pending reward facts"),
    }
}

fn nil_pending_reward_rows<'a>(fixture: &'a Value, scenario: &Value) -> Result<&'a [Value]> {
    ensure!(
        matches!(fixture.get("pending_reward_facts"), Some(Value::Null)),
        "explicit nil pending facts"
    );
    ensure!(
        fixture["profile"] == "synthetic_mainnet_era"
            && fixture["scenario"] == "magnolia"
            && fixture["configuration"]["chain_id"] == 841,
        "nil pending facts profile"
    );
    ensure!(
        fixture.get("head").and_then(Value::as_u64) == Some(5_729_998)
            && fixture["seed"].get("period").and_then(Value::as_u64) == Some(5_729_994),
        "nil pending facts coordinates"
    );
    let history = fixture["seed"]["history"]
        .as_array()
        .context("complete nil pending history")?;
    ensure!(
        history.len() == 6
            && history
                .iter()
                .zip(5_729_989u64..=5_729_994)
                .all(|(row, period)| row.get("period").and_then(Value::as_u64) == Some(period)),
        "contiguous nil pending history"
    );
    // Both pinned owners use frequency one before their first configured rule.
    ensure!(
        fixture["configuration"]["hardforks"]["rewards_distribution_frequency"]
            == serde_json::json!([{"start_period": 5_730_000u64, "frequency": 100u64}]),
        "exact nil pending frequency"
    );
    ensure!(
        scenario["name"] == "magnolia"
            && scenario.get("head").and_then(Value::as_u64) == Some(5_729_998)
            && scenario.get("first_retained").and_then(Value::as_u64) == Some(5_729_994)
            && scenario["periods"] == serde_json::json!([5_729_999u64, 5_730_000u64, 5_730_001u64]),
        "authenticated nil pending input geometry"
    );
    ensure!(
        scenario
            .get("pending_periods")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty),
        "explicit empty producer pending input"
    );
    Ok(&[])
}

fn nil_pending_full_distribution_rows<'a>(
    fixture: &'a Value,
    scenario: &Value,
) -> Result<&'a [Value]> {
    ensure!(matches!(
        fixture.get("pending_reward_facts"),
        Some(Value::Null)
    ));
    ensure!(
        fixture["profile"] == "synthetic_mainnet_era"
            && fixture["scenario"] == "magnolia_first_full_distribution"
            && fixture["configuration"]["chain_id"] == 841
    );
    ensure!(fixture["head"] == 5_729_998u64 && fixture["seed"]["period"] == 5_729_994u64);
    let history = fixture["seed"]["history"]
        .as_array()
        .context("complete nil pending history")?;
    ensure!(
        history.len() == 6
            && history
                .iter()
                .zip(5_729_989u64..=5_729_994)
                .all(|(row, period)| row.get("period").and_then(Value::as_u64) == Some(period))
    );
    ensure!(
        fixture["configuration"]["hardforks"]["rewards_distribution_frequency"]
            == serde_json::json!([{"start_period": 5_730_000u64, "frequency": 100u64}])
    );
    ensure!(
        scenario["name"] == "magnolia_first_full_distribution"
            && scenario["head"] == 5_729_998u64
            && scenario["first_retained"] == 5_729_994u64
    );
    let periods = scenario["periods"]
        .as_array()
        .context("declared full window")?;
    ensure!(periods.iter().map(number).eq(5_729_999u64..=5_730_101));
    ensure!(
        scenario["pending_periods"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    Ok(&[])
}

#[test]
fn captured_nil_pending_reward_facts_require_exact_input_and_history() -> Result<()> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("magnolia.manifest.json"))?)?;
    let scenario = &manifest["scenario"];
    for fixture in [corpus("magnolia")?, public_batched_fixture("magnolia")?] {
        ensure!(matches!(
            fixture.get("pending_reward_facts"),
            Some(Value::Null)
        ));
        ensure!(pending_reward_rows(&fixture)?.is_empty());
        for which in 0..12 {
            let mut invalid = fixture.clone();
            match which {
                0 => {
                    invalid
                        .as_object_mut()
                        .unwrap()
                        .remove("pending_reward_facts");
                }
                1 => invalid["profile"] = serde_json::json!("unknown"),
                2 => invalid["scenario"] = serde_json::json!("phalaenopsis"),
                3 => invalid["head"] = serde_json::json!(5_729_999u64),
                4 => invalid["seed"]["period"] = serde_json::json!(5_729_993u64),
                5 => {
                    invalid["seed"]["history"].as_array_mut().unwrap().remove(0);
                }
                6 => invalid["seed"]["history"][1]["period"] = serde_json::json!(5_729_989u64),
                7 => {
                    invalid["configuration"]["hardforks"]["rewards_distribution_frequency"][0]["start_period"] =
                        serde_json::json!(5_729_998u64)
                }
                8 => {
                    invalid["configuration"]["hardforks"]["rewards_distribution_frequency"][0]["frequency"] =
                        serde_json::json!("100")
                }
                9 => {
                    invalid["configuration"]["hardforks"]["rewards_distribution_frequency"] =
                        Value::Null
                }
                10 => invalid["pending_reward_facts"] = serde_json::json!(false),
                _ => invalid["seed"]["history"][0]["period"] = serde_json::json!("5729989"),
            }
            ensure!(
                pending_reward_rows(&invalid).is_err(),
                "nil pending negative {which}"
            );
        }
        let mut conflicting = scenario.clone();
        conflicting["pending_periods"] = serde_json::json!([5_729_998u64]);
        ensure!(nil_pending_reward_rows(&fixture, &conflicting).is_err());
        conflicting
            .as_object_mut()
            .unwrap()
            .remove("pending_periods");
        ensure!(nil_pending_reward_rows(&fixture, &conflicting).is_err());
    }
    for rows in [
        serde_json::json!([]),
        serde_json::json!([{"number": 2}, {"number": 1}]),
    ] {
        let fixture = serde_json::json!({"pending_reward_facts": rows});
        ensure!(
            pending_reward_rows(&fixture)? == fixture["pending_reward_facts"].as_array().unwrap()
        );
    }
    Ok(())
}

fn bootstrap(root: &Path, fixture: &Value) -> Result<()> {
    bootstrap_inner(root, fixture, 0)
}

fn bootstrap_followthrough(root: &Path, fixture: &Value) -> Result<OrdinaryObserver> {
    ensure!(
        is_second_distribution_scenario(fixture) && fixture["seed"]["donor_document"].is_null()
    );
    bootstrap(root, fixture)?;
    let (application, chain) = open_chain(&root.join("application"), fixture)?;
    let seed = ConcreteStateLifecycle::inspect_existing(
        root.join("state_db"),
        chain.concrete_chain_identity()?,
    )?;
    ensure!(
        seed.pending_marker_rlp.is_empty()
            && application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none()
            && chain.last_block_number_typed()? == seed.committed.period
    );
    Ok(std::rc::Rc::new(RefCell::new(
        ordinary_observer::KnownOrdinaryStorageIdentities::from_asserted_fresh_bootstrap(
            &root.join("state_db"),
            seed.identity,
            seed.committed,
            seed.generation,
        )?,
    )))
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
        let pending = pending_reward_rows(fixture)?
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
    let mut public_prior_slots =
        catalog_storage_identities(&public["seed"]["native_catalog"]["slots"])?;
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
        // The local observer unconditionally declares three supply probes after
        // EndBlock, even before their rows exist. Public Go never consumes those
        // probes. Keep their independently proved absence as observation evidence,
        // rather than inserting them into the public backend's own catalog.
        let public_final_slots =
            catalog_storage_identities(&period["final"]["native_catalog"]["slots"])?;
        let witness_absent = catalog_storage_identities(
            &witness["final"]["native_catalog"]["coverage"]["absent_slot_identities"],
        )?;
        let witness_slots =
            catalog_storage_identities(&witness["period_catalog_identities"]["slots"])?;
        let mut consumed = BTreeSet::new();
        let mut mutated = BTreeSet::new();
        for tx in witness["transactions"]
            .as_array()
            .context("witness transaction consumers")?
        {
            consumed.extend(catalog_storage_identities(
                &tx["observer"]["catalog_identities"]["slots"],
            )?);
        }
        for rows in period["ordered_raw_writes"]
            .as_object()
            .context("public period writes")?
            .values()
        {
            mutated.extend(catalog_storage_identities(rows)?);
        }
        for tx in period["transactions"]
            .as_array()
            .context("public signed transactions")?
        {
            mutated.extend(catalog_storage_identities(&tx["ordered_raw_writes"])?);
        }
        let mut omitted = BTreeSet::new();
        let mut omission_witness = Vec::new();
        for (tag, name) in [(6u8, "minted_tokens"), (7, "total_supply"), (8, "yield")] {
            let slot = ConcreteStorageSlot {
                address: fixed::<20>(&serde_json::json!(
                    "00000000000000000000000000000000000000fe"
                )),
                key: keccak256([tag]).0,
            };
            if witness_slots.contains(&slot)
                && !public_prior_slots.contains(&slot)
                && !public_final_slots.contains(&slot)
            {
                ensure!(
                    witness_absent.contains(&slot)
                        && !consumed.contains(&slot)
                        && !mutated.contains(&slot),
                    "local-only supply probe is not an unused declared absence"
                );
                let row = witness["final"]["native_catalog"]["slots"]
                    .as_array()
                    .context("local supply absence rows")?
                    .iter()
                    .find(|row| {
                        fixed::<20>(&row["address"]) == slot.address
                            && fixed::<32>(&row["key"]) == slot.key
                    })
                    .context("exact local supply probe")?;
                ensure!(
                    row["present"] == false && row["value"] == "",
                    "local supply probe must be exact absence"
                );
                omitted.insert(slot);
                omission_witness.push(serde_json::json!({"period":period["number"],"name":name,"address":hex::encode(slot.address),"key":hex::encode(slot.key),"local_present":false,"local_value":"","public_prior_and_final_catalog_absent":true,"transaction_consumed":false,"public_mutation":false,"source":"RecordConcreteRewardsProjection/ConcreteRewardsStorageKeys"}));
            }
        }
        let rows = period["period_catalog_identities"]["slots"]
            .as_array_mut()
            .context("period slots")?;
        let original = std::mem::take(rows);
        for row in original {
            let slot = ConcreteStorageSlot {
                address: fixed_prefixed(string(&row["address"]))?,
                key: fixed_prefixed(string(&row["key"]))?,
            };
            if !omitted.contains(&slot) {
                rows.push(row);
            }
        }
        period["local_observer_only_supply_absences"] = Value::Array(omission_witness);
        public_prior_slots = public_final_slots;

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

/// Runs both existing publication interruptions on fresh backend-specific pairs.
/// The caller holds the uninterrupted pair until every recovery and API check ends.
/// Each fault must converge through execution-free repeated reconciliation, full
/// continuation, exact final facts and historical calls/estimates/traces.
fn verify_era_recovery_modes(
    fixture: &Value,
    root: &Path,
    base: &Path,
    pin_label: &str,
) -> Result<()> {
    for fault in [
        mixed_recovery::Fault::BeforeCommit,
        mixed_recovery::Fault::AfterCommit,
    ] {
        let recovered = base.join(format!(
            "mainnet-recovery-{pin_label}-{}-{fault:?}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut recovery_scratch = era_retention::register(&recovered, fixture)?;
        std::fs::create_dir(&recovered)?;
        era_retention::created(&mut recovery_scratch)?;
        if is_second_distribution_scenario(fixture) {
            let ordinary = bootstrap_followthrough(&recovered, fixture)?;
            mixed_recovery::verify_mainnet_recovery_observing(
                fixture,
                root,
                &recovered,
                fault,
                Some(ordinary),
            )?;
        } else {
            bootstrap(&recovered, fixture)?;
            mixed_recovery::verify_mainnet_recovery(fixture, root, &recovered, fault)?;
        }
        api::verify(
            fixture,
            &recovered.join("application"),
            &recovered.join("state_db"),
        )?;
        era_retention::finish(
            recovery_scratch,
            era_retention::final_evidence(
                &recovered,
                fixture,
                &format!(
                    "{fault:?}:actual interruption,repeated recovery,retry,full continuation,final facts equal baseline,retained readers,historical APIs; all owners closed"
                ),
            )?,
            false,
        )?;
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
    let baseline_dependency = scratch.as_ref().map(era_retention::Case::dependent);
    verify_era_recovery_modes(&fixture, &root, &base, "public")?;
    drop(baseline_dependency);
    era_retention::finish(
        scratch,
        era_retention::final_evidence(
            &root,
            &fixture,
            "111 periods, physical reopen, receipts/native/history,56 APIs, both private forks/33 negatives, C++ estimates, BeforeCommit/AfterCommit repeated recovery and continuation with historical APIs; all owners closed",
        )?,
        true,
    )?;
    eprintln!(
        "public batched lifecycle:111 periods, exact public EndBlock writes, separate local intermediate witnesses"
    );
    Ok(())
}

#[test]
fn current_second_distribution_local_five_readers_and_recovery() -> Result<()> {
    second_distribution_followthrough(
        corpus("mainnet_second_distribution_followthrough")?,
        "local",
    )
}

#[test]
fn current_second_distribution_public_five_readers_and_recovery() -> Result<()> {
    second_distribution_followthrough(
        public_batched_fixture("mainnet_second_distribution_followthrough")?,
        "public",
    )
}

#[test]
fn current_second_distribution_reopen_local_five_readers_and_recovery() -> Result<()> {
    second_distribution_followthrough(corpus("mainnet_second_distribution_reopen")?, "local")
}

#[test]
fn current_second_distribution_reopen_public_five_readers_and_recovery() -> Result<()> {
    second_distribution_followthrough(
        public_batched_fixture("mainnet_second_distribution_reopen")?,
        "public",
    )
}

fn second_distribution_followthrough(fixture: Value, label: &str) -> Result<()> {
    second_distribution_inner(fixture, label, None)
}

fn required_companion_field<'a>(
    object: &'a Value,
    field: &str,
    nullable: bool,
) -> Result<&'a Value> {
    let value = object
        .as_object()
        .context("required object")?
        .get(field)
        .with_context(|| format!("missing {field}"))?;
    ensure!(nullable || !value.is_null(), "null {field}");
    Ok(value)
}

fn verify_followthrough_companion(original: &Value, companion: &Value) -> Result<()> {
    for fixture in [original, companion] {
        for field in [
            "profile",
            "configuration",
            "inputs",
            "genesis",
            "head",
            "seed",
            "pending_reward_facts",
            "seed_actions_only",
            "scenario",
            "go_row_model_reopen_after",
            "periods",
            "api_cases",
        ] {
            required_companion_field(fixture, field, false)?;
        }
        ensure!(
            fixture["profile"].is_string()
                && fixture["head"].is_u64()
                && fixture["seed_actions_only"].is_boolean()
        );
        ensure!(fixture["pending_reward_facts"].is_array());
        for (field, keys) in [
            (
                "configuration",
                &[
                    "active_rules",
                    "block_gas_limit",
                    "chain_id",
                    "committee_size",
                    "dpos",
                    "hardforks",
                    "transaction_gas_limit",
                ][..],
            ),
            (
                "inputs",
                &[
                    "genesis_allocations",
                    "initial_validator",
                    "initial_validators",
                ][..],
            ),
            ("genesis", &["native_catalog", "period", "root"][..]),
            (
                "seed",
                &[
                    "declared_projection_catalog",
                    "history",
                    "latest_rows",
                    "native_catalog",
                    "ordered_raw_writes",
                    "period",
                    "projection_configuration",
                    "provenance",
                    "root",
                    "rows",
                ][..],
            ),
        ] {
            for key in keys {
                required_companion_field(&fixture[field], key, false)?;
            }
        }
    }
    ensure!(companion["go_row_model_reopen_after"] == 0);
    required_companion_field(companion, "go_row_model_reopen_after_list", false)?;
    ensure!(original["scenario"] == "mainnet_second_distribution_followthrough");
    ensure!(companion["scenario"] == "mainnet_second_distribution_followthrough_recovery");
    ensure!(original["go_row_model_reopen_after"] == 25_706_905);
    ensure!(
        companion["go_row_model_reopen_after_list"]
            == serde_json::json!([
                25_706_905, 25_706_999, 25_707_000, 25_707_001, 25_707_002, 25_707_003, 25_707_004
            ])
    );
    for field in [
        "profile",
        "configuration",
        "inputs",
        "genesis",
        "head",
        "seed",
        "pending_reward_facts",
        "seed_actions_only",
    ] {
        ensure!(
            original[field] == companion[field],
            "U/companion mismatch: {field}"
        );
    }
    let old_periods = original["periods"].as_array().context("U periods")?;
    let new_periods = companion["periods"]
        .as_array()
        .context("companion periods")?;
    ensure!(old_periods.len() == 116 && new_periods.len() == 116);
    for (old, new) in old_periods.iter().zip(new_periods) {
        for period in [old, new] {
            ensure!(required_companion_field(period, "number", false)?.is_u64());
            for field in ["reward_input", "rules"] {
                ensure!(required_companion_field(period, field, false)?.is_object());
            }
            for field in [
                "block_author",
                "blocks_per_year",
                "max_votes_weight",
                "number",
                "planner_facts",
                "total_dag_blocks_count",
                "total_votes_weight",
                "transactions",
                "validators",
            ] {
                required_companion_field(&period["reward_input"], field, false)?;
            }
            for field in ["cacti", "cornus", "ficus"] {
                required_companion_field(&period["rules"], field, false)?;
            }
        }
        ensure!(
            old["number"] == new["number"]
                && old["reward_input"] == new["reward_input"]
                && old["rules"] == new["rules"],
            "U/companion period input mismatch"
        );
        let a = old["transactions"].as_array().context("U transactions")?;
        let b = new["transactions"]
            .as_array()
            .context("companion transactions")?;
        ensure!(a.len() == b.len());
        for (x, y) in a.iter().zip(b) {
            for tx in [x, y] {
                for field in [
                    "name",
                    "signed_rlp",
                    "hash",
                    "sender",
                    "nonce",
                    "input",
                    "value",
                    "gas_price",
                ] {
                    ensure!(
                        required_companion_field(tx, field, false)?.is_string(),
                        "invalid signed action {field}"
                    );
                }
                for field in ["gas_limit", "chain_id"] {
                    ensure!(required_companion_field(tx, field, false)?.is_u64());
                }
                let to = required_companion_field(tx, "to", true)?;
                ensure!(to.is_null() || to.is_string());
                ensure!(
                    !tx["signed_rlp"].as_str().unwrap().is_empty()
                        && !tx["hash"].as_str().unwrap().is_empty()
                );
            }
            for field in [
                "name",
                "signed_rlp",
                "hash",
                "sender",
                "nonce",
                "to",
                "input",
                "value",
                "gas_limit",
                "gas_price",
                "chain_id",
            ] {
                ensure!(
                    x[field] == y[field],
                    "U/companion signed action mismatch: {field}"
                );
            }
        }
    }
    let old_api = original["api_cases"].as_array().context("U APIs")?;
    let new_api = companion["api_cases"]
        .as_array()
        .context("companion APIs")?;
    ensure!(old_api.len() == 42 && new_api.len() == 42);
    for (x, y) in old_api.iter().zip(new_api) {
        for api in [x, y] {
            for field in [
                "name",
                "head",
                "request",
                "trace_period",
                "trace_sequence",
                "estimates",
            ] {
                required_companion_field(api, field, false)?;
            }
            ensure!(
                api["name"].is_string() && api["head"].is_u64() && api["trace_period"].is_u64()
            );
            for field in [
                "gas_price",
                "input",
                "name",
                "sender",
                "supplied_nonce",
                "to",
                "value",
            ] {
                required_companion_field(&api["request"], field, false)?;
            }
            for field in ["calls", "prefix", "request", "result"] {
                required_companion_field(&api["trace_sequence"], field, false)?;
            }
        }
        for field in ["name", "head", "request", "trace_period", "trace_sequence"] {
            ensure!(
                x[field] == y[field],
                "U/companion API request mismatch: {field}"
            );
        }
        let a = x["estimates"].as_array().context("U estimates")?;
        let b = y["estimates"].as_array().context("companion estimates")?;
        ensure!(a.len() == b.len());
        for (a, b) in a.iter().zip(b) {
            for item in [a, b] {
                ensure!(
                    required_companion_field(item, "name", false)?.is_string()
                        && required_companion_field(item, "cap", false)?.is_u64()
                );
            }
            ensure!(a["name"] == b["name"] && a["cap"] == b["cap"]);
        }
    }
    Ok(())
}

#[test]
fn followthrough_companion_guard_rejects_missing_and_changed_inputs() -> Result<()> {
    // The clone tests only guard behavior; it is not a companion oracle.
    let u = corpus("mainnet_second_distribution_followthrough")?;
    let mut c = u.clone();
    c["scenario"] = serde_json::json!("mainnet_second_distribution_followthrough_recovery");
    c["go_row_model_reopen_after"] = serde_json::json!(0);
    c["go_row_model_reopen_after_list"] = serde_json::json!([
        25_706_905, 25_706_999, 25_707_000, 25_707_001, 25_707_002, 25_707_003, 25_707_004
    ]);
    verify_followthrough_companion(&u, &c)?;
    let mut variants = Vec::new();
    for (field, value) in [
        ("scenario", serde_json::json!("wrong")),
        ("go_row_model_reopen_after", serde_json::json!(25_706_905)),
        (
            "go_row_model_reopen_after_list",
            serde_json::json!(25_706_905),
        ),
        (
            "go_row_model_reopen_after_list",
            serde_json::json!([
                25_706_999, 25_707_000, 25_707_001, 25_707_002, 25_707_003, 25_707_004
            ]),
        ),
        (
            "go_row_model_reopen_after_list",
            serde_json::json!([
                25_706_905, 25_706_999, 25_707_000, 25_707_001, 25_707_002, 25_707_003
            ]),
        ),
        (
            "go_row_model_reopen_after_list",
            serde_json::json!([
                25_706_905, 25_706_999, 25_707_000, 25_707_001, 25_707_002, 25_707_003, 25_707_004,
                25_707_005
            ]),
        ),
        (
            "go_row_model_reopen_after_list",
            serde_json::json!([
                25_706_999, 25_706_905, 25_707_000, 25_707_001, 25_707_002, 25_707_003, 25_707_004
            ]),
        ),
    ] {
        let mut v = c.clone();
        v[field] = value;
        variants.push(v);
    }
    for path in ["configuration", "seed"] {
        let mut v = c.clone();
        v[path] = serde_json::json!({"changed":true});
        variants.push(v);
    }
    let mut v = c.clone();
    v["api_cases"][0]["request"]["input"] = serde_json::json!("changed");
    variants.push(v);
    let mut v = c.clone();
    v["periods"][115]["transactions"][0]["signed_rlp"] = serde_json::json!("changed");
    variants.push(v);
    for v in variants {
        ensure!(verify_followthrough_companion(&u, &v).is_err());
    }
    for field in ["configuration", "seed", "seed_actions_only"] {
        let (mut a, mut b) = (u.clone(), c.clone());
        a.as_object_mut().unwrap().remove(field);
        b.as_object_mut().unwrap().remove(field);
        ensure!(verify_followthrough_companion(&a, &b).is_err());
    }
    for field in ["signed_rlp", "hash"] {
        let (mut a, mut b) = (u.clone(), c.clone());
        a["periods"][115]["transactions"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        b["periods"][115]["transactions"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        ensure!(verify_followthrough_companion(&a, &b).is_err());
    }
    let (mut a, mut b) = (u.clone(), c.clone());
    a["api_cases"][0].as_object_mut().unwrap().remove("request");
    b["api_cases"][0].as_object_mut().unwrap().remove("request");
    ensure!(verify_followthrough_companion(&a, &b).is_err());
    Ok(())
}

fn second_distribution_inner(
    fixture: Value,
    label: &str,
    u_facts: Option<&mixed_recovery::FinalFacts>,
) -> Result<()> {
    ensure!(is_second_distribution_scenario(&fixture));
    ensure!(
        std::env::var_os("MAINNET_ERA_API_PAIR").is_none(),
        "fresh paired replay required"
    );
    let periods = fixture["periods"]
        .as_array()
        .context("followthrough periods")?;
    ensure!(
        periods.len() == 116
            && periods
                .iter()
                .map(|p| number(&p["number"]))
                .eq(25_706_890..=25_707_005)
    );
    ensure!(fixture["seed"]["donor_document"].is_null());
    ensure!(
        fixture["api_cases"]
            .as_array()
            .context("followthrough APIs")?
            .len()
            == 42
    );
    for height in 25_707_000..=25_707_005 {
        let period = periods
            .iter()
            .find(|p| number(&p["number"]) == height)
            .context("signed followthrough period")?;
        let transactions = period["transactions"]
            .as_array()
            .context("actual signed followthrough transactions")?;
        ensure!(
            transactions.len() == 1
                && transactions[0]["name"] == format!("second_distribution_transfer_{height}"),
            "exact signed followthrough action"
        );
        ensure!(
            transactions[0]["status"] == 1
                && transactions[0]["value"] == "1"
                && transactions[0]["gas_price"] == "1000000000",
            "valid signed followthrough transfer"
        );
        ensure!(
            period["prior_root"] != period["final"]["root"],
            "actual followthrough state transition"
        );
    }
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("task artifact root")?;
    let root = base.join(format!(
        "current-second-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    let ordinary = bootstrap_followthrough(&root, &fixture)?;
    if fixture["scenario"] == "mainnet_second_distribution_followthrough_recovery" {
        let first = periods
            .iter()
            .position(|p| number(&p["number"]) == 25_706_906)
            .context("companion first reopen")?;
        let second = periods
            .iter()
            .position(|p| number(&p["number"]) == 25_707_000)
            .context("companion second reopen")?;
        ensure!(first < second);
        for segment in [&periods[..first], &periods[first..second]] {
            native_integration::run_segment_observing(
                &fixture,
                segment,
                &root.join("application"),
                &root.join("state_db"),
                false,
                mixed_recovery::Fault::None,
                Some(ordinary.clone()),
            )?;
        }
        for index in second..periods.len() {
            mixed_recovery::run_period_observing(
                &root,
                &fixture,
                index,
                mixed_recovery::Fault::None,
                Some(ordinary.clone()),
            )?;
        }
    } else if fixture["scenario"] == "mainnet_second_distribution_reopen" {
        let split = periods
            .iter()
            .position(|p| number(&p["number"]) == 25_707_000)
            .context("second distribution")?;
        native_integration::run_segment_observing(
            &fixture,
            &periods[..split],
            &root.join("application"),
            &root.join("state_db"),
            false,
            mixed_recovery::Fault::None,
            Some(ordinary.clone()),
        )?;
        for index in split..periods.len() {
            mixed_recovery::run_period_observing(
                &root,
                &fixture,
                index,
                mixed_recovery::Fault::None,
                Some(ordinary.clone()),
            )?;
        }
    } else {
        let split = periods
            .iter()
            .position(|p| number(&p["number"]) == 25_706_906)
            .context("actual reopen")?;
        native_integration::run_segment_observing(
            &fixture,
            &periods[..split],
            &root.join("application"),
            &root.join("state_db"),
            false,
            mixed_recovery::Fault::None,
            Some(ordinary.clone()),
        )?;
        native_integration::run_segment_observing(
            &fixture,
            &periods[split..],
            &root.join("application"),
            &root.join("state_db"),
            false,
            mixed_recovery::Fault::None,
            Some(ordinary.clone()),
        )?;
    }
    verify_physical_cf5(&root.join("state_db"), &fixture["history_rows"][4])?;
    api::verify(&fixture, &root.join("application"), &root.join("state_db"))?;
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        directory.join(format!("{}.manifest.json", string(&fixture["scenario"]))),
    )?)?;
    api::verify_cpp_estimates(
        &fixture,
        &pinned_document(&manifest["cpp_estimates"])?,
        label,
    )?;
    let dependency = scratch.as_ref().map(era_retention::Case::dependent);
    if fixture["scenario"] == "mainnet_second_distribution_followthrough" {
        let companion = if label == "local" {
            corpus("mainnet_second_distribution_followthrough_recovery")?
        } else {
            ensure!(label == "public");
            public_batched_fixture("mainnet_second_distribution_followthrough_recovery")?
        };
        verify_followthrough_companion(&fixture, &companion)?;
        let facts = mixed_recovery::final_facts(&root, &fixture)?;
        second_distribution_inner(companion, label, Some(&facts))?;
    } else {
        if let Some(expected) = u_facts {
            ensure!(fixture["scenario"] == "mainnet_second_distribution_followthrough_recovery");
            ensure!(
                &mixed_recovery::final_facts(&root, &fixture)? == expected,
                "U/companion complete final facts mismatch"
            );
        } else {
            ensure!(fixture["scenario"] != "mainnet_second_distribution_followthrough_recovery");
        }
        verify_era_recovery_modes(&fixture, &root, &base, label)?;
    }
    drop(dependency);
    era_retention::finish(
        scratch,
        era_retention::final_evidence(
            &root,
            &fixture,
            if fixture["scenario"] == "mainnet_second_distribution_followthrough" {
                "U: 116 actual periods,42 APIs,physical history and C++ estimates; separately pinned companion fault evidence; paired owner closure"
            } else {
                "116 actual periods; second distribution fault recovery; five retained readers through+5;42 APIs; paired owner closure"
            },
        )?,
        true,
    )?;
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
        verify_era_recovery_modes(&fixture, &root, &base, "local")?;
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
fn phala_contract_rejects_reward_occurrence_and_value_forgery() -> Result<()> {
    use rustaxa_consensus::native_session::{
        FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRawValue,
    };
    let fixture = corpus("phalaenopsis")?;
    let row = &fixture["seed"]["donor_document"]["periods"]
        .as_array()
        .context("donor periods")?
        .iter()
        .find(|p| number(&p["number"]) == 6_942_800)
        .context("actual donor distribution")?["distributions"][0];
    let writes = row["ordered_raw_writes"]
        .as_array()
        .context("legacy writes")?;
    let context = FinalChainNativeRewardsContext {
        raw_mutations: writes
            .iter()
            .map(|entry| {
                Ok(FinalChainNativeRawMutation {
                    address: fixed(&entry["address"]),
                    key: ConcreteStorageKey(fixed(&entry["key"])),
                    expected: ConcreteRead::Absent,
                    operation: FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(
                        bytes(&entry["value"]),
                    )?),
                })
            })
            .collect::<Result<Vec<_>>>()?,
        ..Default::default()
    };
    compare_pre_aspen_reward_map(&context, writes, &row["input"])?;
    for mutation in 0..4 {
        let mut forged = writes.clone();
        match mutation {
            0 => forged.swap(0, 1),
            1 => forged[2] = forged[1].clone(),
            2 => {
                forged[1]["value"] = serde_json::json!("01");
            }
            _ => {
                forged.pop();
            }
        }
        ensure!(
            compare_pre_aspen_reward_map(&context, &forged, &row["input"]).is_err(),
            "forged legacy reward occurrence/value accepted"
        );
    }
    Ok(())
}

#[test]
fn phala_contract_rejects_live_or_consumed_supply_probe_omission() -> Result<()> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        directory.join("phalaenopsis.manifest.json"),
    )?)?;
    let public = pinned_document(&manifest["streams"]["public"])?;
    let local = pinned_document(&manifest["streams"]["local"])?;
    let mut valid = public.clone();
    attach_public_witnesses(&mut valid, &local, &manifest["references"]["local"], 0)?;
    ensure!(
        valid["seed"]["donor_document"]["periods"][0]["local_observer_only_supply_absences"]
            .as_array()
            .context("omission witnesses")?
            .len()
            == 3
    );
    let key = hex::encode(keccak256([6]).0);
    for consumed in [false, true] {
        let mut witness = local.clone();
        if consumed {
            witness["periods"][4]["transactions"][0]["observer"]["catalog_identities"]["slots"].as_array_mut().context("actual first boundary transaction")?.push(serde_json::json!({"address":"0x00000000000000000000000000000000000000fe","key":format!("0x{key}")}));
        } else {
            let row =
                witness["seed"]["donor_document"]["periods"][0]["final"]["native_catalog"]["slots"]
                    .as_array_mut()
                    .context("supply probe rows")?
                    .iter_mut()
                    .find(|row| row["key"] == key)
                    .context("minted probe")?;
            row["present"] = serde_json::json!(true);
            row["value"] = serde_json::json!("01");
        }
        let mut candidate = public.clone();
        ensure!(
            attach_public_witnesses(
                &mut candidate,
                &witness,
                &manifest["references"]["local"],
                0
            )
            .is_err(),
            "forged live/consumed supply probe omission accepted"
        );
    }
    Ok(())
}

#[test]
fn phalaenopsis_signed_boundary_donor_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("phalaenopsis")?, "local")
}

#[test]
fn phalaenopsis_public_batched_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("phalaenopsis")?, "public")
}

#[test]
fn cornus_signed_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("cornus")?, "local")
}
#[test]
fn cornus_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("cornus")?, "public")
}
#[test]
fn ficus_signed_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("ficus")?, "local")
}

#[test]
fn ficus_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("ficus")?, "public")
}

#[test]
fn cornus_v2_maturity_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("cornus_v2_maturity")?, "local")
}
#[test]
fn cornus_v2_maturity_public_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("cornus_v2_maturity")?, "public")
}

/// Qualifies only the explicitly declared pinned historical donor windows.
/// The same execution owners produce every root/write/receipt before the oracle
/// comparisons. Both owners physically reopen at the selected boundary/maturity
/// and at the final API phase. Unsupported scenario names fail closed.
/// No Cacti counts, synthetic root, or API-only reuse can satisfy this case.
#[test]
fn magnolia_signed_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("magnolia")?, "local")
}

#[test]
fn magnolia_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("magnolia")?, "public")
}

#[test]
fn aspen_one_signed_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("aspen_one")?, "local")
}

#[test]
fn aspen_one_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("aspen_one")?, "public")
}

#[test]
fn aspen_two_signed_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("aspen_two")?, "local")
}

#[test]
fn aspen_two_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("aspen_two")?, "public")
}

// Valid signed execution component only; outer admission is a separate gate.
#[test]
fn soleirolia_signed_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("soleirolia")?, "local")
}

#[test]
fn soleirolia_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(public_batched_fixture("soleirolia")?, "public")
}

#[test]
fn claim_all_legacy_local_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(corpus("claim_all_legacy_batch_correction")?, "local")
}

#[test]
fn claim_all_legacy_public_boundary_reopens_history_apis_and_recovers() -> Result<()> {
    historical_workload(
        public_batched_fixture("claim_all_legacy_batch_correction")?,
        "public",
    )
}

#[test]
fn magnolia_first_full_distribution_local_history_and_recovery() -> Result<()> {
    magnolia_full_manifest()?;
    let fixture = corpus("magnolia_first_full_distribution")?;
    verify_magnolia_full_prefix(&fixture, &corpus("magnolia")?)?;
    historical_workload(fixture, "local")
}

#[test]
fn magnolia_first_full_distribution_public_history_and_recovery() -> Result<()> {
    magnolia_full_manifest()?;
    let fixture = public_batched_fixture("magnolia_first_full_distribution")?;
    verify_magnolia_full_prefix(&fixture, &public_batched_fixture("magnolia")?)?;
    historical_workload(fixture, "public")
}

fn verify_magnolia_full_prefix(fixture: &Value, prior: &Value) -> Result<()> {
    ensure!(
        fixture["scenario"] == "magnolia_first_full_distribution"
            && prior["scenario"] == "magnolia"
    );
    ensure!(
        fixture["seed"] == prior["seed"] && fixture["configuration"] == prior["configuration"],
        "accepted Magnolia seed/configuration must be preserved"
    );
    let periods = fixture["periods"]
        .as_array()
        .context("new full-batch periods")?;
    let previous = prior["periods"]
        .as_array()
        .context("accepted Magnolia periods")?;
    ensure!(previous.len() == 7);
    let mut new_periods = BTreeMap::new();
    for row in periods {
        ensure!(
            new_periods.insert(number(&row["number"]), row).is_none(),
            "duplicate new period identity"
        );
    }
    let mut old_periods = BTreeMap::new();
    for row in previous {
        ensure!(
            old_periods.insert(number(&row["number"]), row).is_none(),
            "duplicate accepted period identity"
        );
    }
    ensure!(old_periods.keys().copied().eq(5_729_995u64..=5_730_001));
    // Global cross-capture write interleaving remains OPEN. Compare all other
    // fields exactly and retain every occurrence in each key's write sequence.
    for (period, previous) in old_periods {
        let current = new_periods
            .get(&period)
            .context("accepted period identity missing")?;
        compare_prefix_period_diagnostic(previous, current)?;
    }
    let old_api = prior["api_cases"]
        .as_array()
        .context("accepted Magnolia API cases")?;
    let new_api = fixture["api_cases"]
        .as_array()
        .context("full-batch API cases")?;
    ensure!(old_api.len() == 21 && new_api.len() == 42);
    let mut new_cases = BTreeMap::new();
    for row in new_api {
        let name = row["name"].as_str().context("new API case identity")?;
        ensure!(
            new_cases.insert(name, row).is_none(),
            "duplicate new API case identity"
        );
    }
    let mut old_names = BTreeSet::new();
    let mut heads = BTreeMap::new();
    for row in old_api {
        let head = row
            .get("head")
            .and_then(Value::as_u64)
            .context("API head")?;
        *heads.entry(head).or_insert(0usize) += 1;
        let name = row["name"].as_str().context("accepted API case identity")?;
        ensure!(
            old_names.insert(name),
            "duplicate accepted API case identity"
        );
        let current = *new_cases.get(name).context("accepted API case missing")?;
        for field in [
            "name",
            "head",
            "request",
            "call",
            "repeat_call",
            "estimates",
        ] {
            ensure!(
                row.get(field).is_some() && row.get(field) == current.get(field),
                "accepted Magnolia API field changed: {field}"
            );
        }
        let head = row
            .get("head")
            .and_then(Value::as_u64)
            .context("API head")?;
        match head {
            5_729_999 | 5_730_000 => ensure!(current == row, "accepted API row changed"),
            5_730_001 => {
                // These traces execute at different periods. They are distinct
                // requests and their cross-tip equivalence remains OPEN.
                ensure!(row.get("trace_period").and_then(Value::as_u64) == Some(5_730_001));
                ensure!(current.get("trace_period").and_then(Value::as_u64) == Some(5_730_002));
            }
            _ => bail!("unexpected accepted API head"),
        }
    }
    ensure!(heads == BTreeMap::from([(5_729_999, 7), (5_730_000, 7), (5_730_001, 7)]));
    // The full physical history belongs to different terminal heads. No
    // cross-head physical-prefix equality is asserted without an independently
    // authenticated shared-history projection.
    Ok(())
}

// Diagnostic only: no claim is made about ordering across different keys.
fn compare_prefix_period_diagnostic(previous: &Value, current: &Value) -> Result<()> {
    fn grouped(rows: &Value) -> Result<BTreeMap<(String, String), Vec<Value>>> {
        let mut groups: BTreeMap<_, Vec<Value>> = BTreeMap::new();
        for row in rows.as_array().context("reward write list")? {
            let address = row
                .get("address")
                .and_then(Value::as_str)
                .context("write address")?;
            let key = row
                .get("key")
                .and_then(Value::as_str)
                .context("write key")?;
            groups
                .entry((address.to_owned(), key.to_owned()))
                .or_default()
                .push(row.clone());
        }
        Ok(groups)
    }
    let mut projected = previous.clone();
    let old_rewards = &previous["ordered_raw_writes"]["rewards"];
    let new_rewards = &current["ordered_raw_writes"]["rewards"];
    ensure!(
        grouped(old_rewards)? == grouped(new_rewards)?,
        "prefix per-key reward sequences"
    );
    projected["ordered_raw_writes"]["rewards"] = new_rewards.clone();
    let old_parts = previous["distributions"]
        .as_array()
        .context("old distributions")?;
    let new_parts = current["distributions"]
        .as_array()
        .context("new distributions")?;
    ensure!(
        old_parts.len() == new_parts.len(),
        "prefix distribution count"
    );
    for (index, (old, new)) in old_parts.iter().zip(new_parts).enumerate() {
        let old_writes = &old["ordered_raw_writes"];
        let new_writes = &new["ordered_raw_writes"];
        ensure!(
            grouped(old_writes)? == grouped(new_writes)?,
            "prefix distribution per-key sequences"
        );
        projected["distributions"][index]["ordered_raw_writes"] = new_writes.clone();
    }
    ensure!(&projected == current, "prefix non-write fields changed");
    Ok(())
}

fn verify_magnolia_batch_membership(periods: &[Value]) -> Result<()> {
    let activation = periods
        .iter()
        .find(|p| number(&p["number"]) == 5_730_000)
        .context("activation batch")?;
    let first_rows = activation["distributions"]
        .as_array()
        .context("activation distributions")?;
    ensure!(first_rows.len() == 1 && number(&first_rows[0]["period"]) == 5_730_000);
    let full = periods
        .iter()
        .find(|p| number(&p["number"]) == 5_730_100)
        .context("full batch")?;
    let rows = full["distributions"]
        .as_array()
        .context("full batch distributions")?;
    ensure!(
        rows.len() == 100
            && rows
                .iter()
                .map(|r| number(&r["period"]))
                .collect::<BTreeSet<_>>()
                == (5_730_001u64..=5_730_100).collect(),
        "actual first full batch must have all 100 source periods"
    );
    Ok(())
}

#[test]
fn magnolia_first_full_distribution_negative_controls() -> Result<()> {
    let fixture = corpus("magnolia_first_full_distribution")?;
    let manifest = magnolia_full_manifest()?;
    let periods = fixture["periods"].as_array().context("actual periods")?;
    ensure!(periods.len() == 107);
    verify_magnolia_batch_membership(periods)?;
    nil_pending_full_distribution_rows(&fixture, &manifest["scenario"])?;
    let mut missing = fixture.clone();
    missing
        .as_object_mut()
        .context("fixture object")?
        .remove("pending_reward_facts");
    ensure!(nil_pending_full_distribution_rows(&missing, &manifest["scenario"]).is_err());
    ensure!(authenticated_magnolia_full_bytes(b"incorrect manifest bytes").is_err());
    let mut duplicate = periods.clone();
    let batch = duplicate
        .iter_mut()
        .find(|p| number(&p["number"]) == 5_730_100)
        .context("full batch")?;
    let rows = batch["distributions"]
        .as_array_mut()
        .context("batch rows")?;
    rows[1]["period"] = rows[0]["period"].clone();
    ensure!(verify_magnolia_batch_membership(&duplicate).is_err());
    let mut missing_batch = periods.clone();
    missing_batch
        .iter_mut()
        .find(|p| number(&p["number"]) == 5_730_100)
        .context("full batch")?["distributions"]
        .as_array_mut()
        .context("batch rows")?
        .pop();
    ensure!(verify_magnolia_batch_membership(&missing_batch).is_err());
    let old = corpus("magnolia")?;
    let row = &old["periods"][0];
    compare_prefix_period_diagnostic(row, row)?;
    let mut economic = row.clone();
    economic["final"]["root"] = serde_json::json!("changed");
    ensure!(compare_prefix_period_diagnostic(row, &economic).is_err());
    let mut sequence = row.clone();
    let writes = sequence["ordered_raw_writes"]["rewards"]
        .as_array_mut()
        .context("reward writes")?;
    let mut second = writes[0].clone();
    second["value"] = serde_json::json!("changed");
    writes.push(second);
    let mut reordered = sequence.clone();
    let writes = reordered["ordered_raw_writes"]["rewards"]
        .as_array_mut()
        .context("reward writes")?;
    let last = writes.len() - 1;
    writes.swap(0, last);
    ensure!(compare_prefix_period_diagnostic(&sequence, &reordered).is_err());
    let mut changed = sequence.clone();
    changed["ordered_raw_writes"]["rewards"][0]["value"] = serde_json::json!("changed value");
    ensure!(compare_prefix_period_diagnostic(&sequence, &changed).is_err());
    Ok(())
}

fn historical_workload(fixture: Value, pin_label: &str) -> Result<()> {
    let scenario = string(&fixture["scenario"]).to_owned();
    let (first, last, boundary, api_cases) = match scenario.as_str() {
        "magnolia" => (5729995u64, 5730001u64, 5730000u64, 21usize),
        "magnolia_first_full_distribution" => (5729995, 5730101, 5730000, 42),
        "aspen_one" => (8_117_995, 8_118_001, 8_118_000, 21),
        "aspen_two" => (8_571_995, 8_572_001, 8_572_000, 21),
        "soleirolia" => (17_379_995, 17_380_001, 17_380_000, 21),
        "phalaenopsis" => (6_942_995u64, 6_943_005u64, 6_943_000u64, 35usize),
        "claim_all_legacy_batch_correction" => (7_599_995, 7_600_005, 7_600_000, 35),
        "ficus" => (11_615_995, 11_616_005, 11_616_000, 35),
        "cornus" => (15_609_995, 15_610_005, 15_610_000, 35),
        "cornus_v2_maturity" => (15_773_564, 15_773_574, 15_773_569, 28),
        _ => bail!("unqualified historical workload"),
    };
    let periods = fixture["periods"]
        .as_array()
        .context("complete historical periods")?;
    let period_count = last - first + 1;
    ensure!(periods.len() as u64 == last - first + 1);
    ensure!(
        periods
            .iter()
            .map(|row| number(&row["number"]))
            .eq(first..=last)
    );
    if matches!(
        scenario.as_str(),
        "magnolia" | "magnolia_first_full_distribution" | "aspen_one" | "aspen_two" | "soleirolia"
    ) {
        ensure!(
            fixture["seed"]["donor_document"].is_null(),
            "registered no-donor window has no donor"
        );
    } else {
        ensure!(
            fixture["seed"]["donor_document"].is_object(),
            "actual signed same-era donor required"
        );
    }
    ensure!(
        fixture["api_cases"]
            .as_array()
            .context("historical API cases")?
            .len()
            == api_cases
    );
    for p in [boundary - 1, boundary, boundary + 1] {
        let period = periods
            .iter()
            .find(|row| number(&row["number"]) == p)
            .context("activation period")?;
        ensure!(
            !period["transactions"]
                .as_array()
                .context("signed boundary transactions")?
                .is_empty()
        );
    }
    if scenario == "magnolia_first_full_distribution" {
        verify_magnolia_batch_membership(periods)?;
    }
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent RUN_ARTIFACT_DIR")?;
    let root = base.join(format!(
        "{scenario}-{pin_label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    bootstrap(&root, &fixture)?;
    let dependency = scratch.as_ref().map(era_retention::Case::dependent);
    let split = periods
        .iter()
        .position(|row| number(&row["number"]) == boundary)
        .context("historical physical reopen")?;
    for segment in [&periods[..split], &periods[split..]] {
        native_integration::run_segment(
            &fixture,
            segment,
            &root.join("application"),
            &root.join("state_db"),
            false,
        )?;
    }
    {
        let (application, chain) = open_chain(&root.join("application"), &fixture)?;
        let observed = ConcreteStateLifecycle::inspect_existing(
            root.join("state_db"),
            chain.concrete_chain_identity()?,
        )?;
        ensure!(
            observed.committed.period.as_u64() == last
                && observed.committed.state_root
                    == fixed(&periods.last().unwrap()["final"]["root"])
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
    if scenario == "claim_all_legacy_batch_correction"
        || scenario == "magnolia_first_full_distribution"
    {
        verify_physical_rows(&root.join("state_db"), &fixture["history_rows"])?;
    }
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/mainnet_era");
    let manifest: Value = serde_json::from_slice(&std::fs::read(
        directory.join(format!("{scenario}.manifest.json")),
    )?)?;
    let estimates_pin: Value = if scenario == "phalaenopsis" {
        serde_json::from_slice(&std::fs::read(
            directory.join("phalaenopsis.cpp-estimates.pin.json"),
        )?)?
    } else {
        manifest["cpp_estimates"].clone()
    };
    ensure!(
        estimates_pin.is_object(),
        "actual historical C++ estimation capture required"
    );
    let estimates = pinned_document(&estimates_pin)?;
    api::verify_cpp_estimates(&fixture, &estimates, pin_label)?;
    verify_era_recovery_modes(&fixture, &root, &base, pin_label)?;
    drop(dependency);
    era_retention::finish(
        scratch,
        era_retention::final_evidence(
            &root,
            &fixture,
            &format!(
                "{scenario} declared seed,{period_count}periods,{first}..{last},boundary-1/boundary/boundary+1,exact ordered writes/receipts/roots/history,{api_cases}APIs/call/estimate/trace,C++estimates,physical reopen,both commit interruptions,repeated reconciliation,continuation and retained readers; all owners closed"
            ),
        )?,
        true,
    )?;
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
