//! Bounded undelegation inverse rooted in already authenticated addresses.
//!
//! This diagnostic extends the native inverse only through validator and owner
//! addresses recovered from the global validator index. It verifies current
//! V1 and V2 undelegation iterables and objects against the authenticated live
//! inventory. The seed set is not a global delegator catalog, so the result is
//! additive current-live coverage rather than semantic snapshot completeness.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use num_bigint::BigUint;
use rlp::{Rlp, RlpStream};
use rustaxa_storage::{ConcreteStorageInventory, ConcreteStorageInventoryEntry};
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStorageKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};

/// Maximum number of already authenticated addresses accepted as seeds.
pub const MAX_SEEDED_UNDELEGATORS: u32 = 4_096;

/// Maximum aggregate number of V1 undelegations accepted across all seeds.
pub const MAX_V1_ENTRIES: u32 = 4_096;

/// Maximum aggregate number of V2 validator groups accepted across all seeds.
pub const MAX_V2_VALIDATOR_GROUPS: u32 = 4_096;

/// Maximum aggregate number of V2 undelegations accepted across all groups.
pub const MAX_V2_ENTRIES: u32 = 4_096;

/// Worst-case point reads permitted by the four aggregate ceilings.
///
/// Each seed probes the V1 count, V2 count, and V2 last ID. Every V1 entry
/// requires an item, reverse, and object read. Every V2 group requires an outer
/// item, reverse, and nested count; every V2 entry requires an item, reverse,
/// and object read.
pub const MAX_DERIVED_READS: u64 = 3 * (MAX_SEEDED_UNDELEGATORS as u64)
    + 3 * (MAX_V1_ENTRIES as u64)
    + 3 * (MAX_V2_VALIDATOR_GROUPS as u64)
    + 3 * (MAX_V2_ENTRIES as u64);

/// Physical results and decoded counts for one authenticated address seed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededUndelegatorFact {
    pub delegator_hex: String,
    pub v1_validator_count_result: &'static str,
    pub v1_validator_count: Option<u32>,
    pub v2_validator_count_result: &'static str,
    pub v2_validator_count: Option<u32>,
    pub v2_last_id_result: &'static str,
    pub v2_last_id: Option<u64>,
}

/// One current V1 undelegation reached through a seeded validator iterable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededUndelegationV1Fact {
    pub delegator_hex: String,
    pub validator_hex: String,
    pub position: u32,
    pub amount_hex: String,
    pub block: u64,
}

/// One V2 validator group reached through a seeded validator iterable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededUndelegationV2GroupFact {
    pub delegator_hex: String,
    pub validator_hex: String,
    pub position: u32,
    pub id_count: u32,
}

/// One current V2 undelegation reached through a seeded nested ID iterable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededUndelegationV2Fact {
    pub delegator_hex: String,
    pub validator_hex: String,
    pub validator_position: u32,
    pub id_position: u32,
    pub id: u64,
    pub amount_hex: String,
    pub block: u64,
}

/// Additive exact current-live coverage produced by the seeded undelegation inverse.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededUndelegationCoverage {
    pub candidate_count: u32,
    pub max_seeded_undelegators: u32,
    pub max_v1_entries: u32,
    pub max_v2_validator_groups: u32,
    pub max_v2_entries: u32,
    pub max_derived_reads: u64,
    pub derived_reads: u64,
    pub v1_entries: Vec<SeededUndelegationV1Fact>,
    pub v2_validator_groups: Vec<SeededUndelegationV2GroupFact>,
    pub v2_entries: Vec<SeededUndelegationV2Fact>,
    pub probe_results: BTreeMap<String, u64>,
    pub candidates: Vec<SeededUndelegatorFact>,
    pub additional_matched_live_entries: u64,
    pub additional_matched_live_value_bytes: u64,
    pub additional_matched_by_family: BTreeMap<String, u64>,
    pub resulting_matched_live_entries: u64,
    pub resulting_matched_live_value_bytes: u64,
    pub resulting_unexplained_live_entries: u64,
    pub resulting_unexplained_live_value_bytes: u64,
    pub resulting_unexplained_paths_sha256: String,
    pub resulting_unexplained_entries_sha256: String,
    pub live_partition_exact: bool,
    pub global_delegator_set_complete: bool,
    pub semantic_snapshot_complete: bool,
}

#[derive(Clone, Debug)]
struct UndelegationRow {
    amount: BigUint,
    block: u64,
}

struct Probe {
    result: &'static str,
    value: Option<Vec<u8>>,
}

struct Analyzer<'a, F> {
    inventory: BTreeMap<[u8; 32], &'a [u8]>,
    base_matched_paths: &'a BTreeSet<[u8; 32]>,
    read: F,
    matched: BTreeMap<[u8; 32], &'static str>,
    matched_by_family: BTreeMap<String, u64>,
    matched_value_bytes: u64,
    probe_results: BTreeMap<String, u64>,
    derived_reads: u64,
}

impl<'a, F> Analyzer<'a, F>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    fn probe(&mut self, family: &'static str, key: ConcreteStorageKey) -> Result<Probe> {
        self.derived_reads = self
            .derived_reads
            .checked_add(1)
            .context("seeded undelegation derived-read count overflow")?;
        ensure!(
            self.derived_reads <= MAX_DERIVED_READS,
            "seeded undelegation derived reads exceed aggregate bound"
        );
        let path = hashed_storage_path(key);
        let probe = match (self.read)(key) {
            Ok(ConcreteRead::Present(value)) => match self.inventory.get(&path) {
                Some(inventory_value) => {
                    ensure!(
                        *inventory_value == value,
                        "derived {family} row differs from live inventory"
                    );
                    ensure!(
                        !self.base_matched_paths.contains(&path),
                        "derived {family} row was already matched by the base inverse"
                    );
                    if let Some(previous) = self.matched.insert(path, family) {
                        bail!("derived live path collision between {previous} and {family}");
                    }
                    *self.matched_by_family.entry(family.to_owned()).or_default() += 1;
                    self.matched_value_bytes = self
                        .matched_value_bytes
                        .checked_add(u64::try_from(value.len())?)
                        .context("seeded undelegation matched byte count overflow")?;
                    Probe {
                        result: "live_present",
                        value: Some(value),
                    }
                }
                None => Probe {
                    result: "orphan_present",
                    value: Some(value),
                },
            },
            Ok(ConcreteRead::Absent) => {
                self.ensure_not_live(family, path, "absent")?;
                Probe {
                    result: "absent",
                    value: None,
                }
            }
            Ok(ConcreteRead::Tombstone) => {
                self.ensure_not_live(family, path, "tombstone")?;
                Probe {
                    result: "tombstone",
                    value: None,
                }
            }
            Err(ConcreteReadError::HistoryUnavailable(_)) => {
                self.ensure_not_live(family, path, "history_unavailable")?;
                Probe {
                    result: "history_unavailable",
                    value: None,
                }
            }
            Err(ConcreteReadError::Pruned(_)) => {
                self.ensure_not_live(family, path, "pruned")?;
                Probe {
                    result: "pruned",
                    value: None,
                }
            }
            Err(error) => {
                return Err(anyhow::Error::new(error)).with_context(|| {
                    format!("read derived {family} logical key {}", hex::encode(key.0))
                });
            }
        };
        *self
            .probe_results
            .entry(format!("{family}:{}", probe.result))
            .or_default() += 1;
        Ok(probe)
    }

    fn required(&mut self, family: &'static str, key: ConcreteStorageKey) -> Result<Vec<u8>> {
        let probe = self.probe(family, key)?;
        ensure!(
            probe.result == "live_present",
            "required {family} row is not physically live: {}",
            probe.result
        );
        probe
            .value
            .context("required live seeded undelegation row has no value")
    }

    fn ensure_not_live(
        &self,
        family: &'static str,
        path: [u8; 32],
        physical_result: &'static str,
    ) -> Result<()> {
        ensure!(
            !self.inventory.contains_key(&path),
            "derived {family} row is {physical_result} but its path is live in the inventory"
        );
        Ok(())
    }
}

/// Extends an existing exact live partition with undelegation rows derived from
/// authenticated validator/owner addresses.
///
/// `inventory`, `base_matched_paths`, and `read` must select the same qualified
/// concrete-state identity and DPoS account used by the base inverse. Every base
/// path must be live in that inventory, and every newly derived live read must
/// match the inventory's exact bytes.
///
/// The four aggregate ceilings bound all nested iterables before their entries
/// are read. A successful result proves only current live rows reachable from
/// `seeded_addresses`. It does not prove the seed set globally complete,
/// reconstruct deleted history, or authorize snapshot publication.
pub fn analyze_seeded_undelegations<F>(
    inventory: &ConcreteStorageInventory,
    base_matched_paths: &BTreeSet<[u8; 32]>,
    base_matched_value_bytes: u64,
    seeded_addresses: BTreeSet<[u8; 20]>,
    read: F,
) -> Result<SeededUndelegationCoverage>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    let candidate_count = u32::try_from(seeded_addresses.len())?;
    ensure!(
        candidate_count <= MAX_SEEDED_UNDELEGATORS,
        "seeded undelegator count {candidate_count} exceeds aggregate bound {MAX_SEEDED_UNDELEGATORS}"
    );
    let indexed = inventory
        .entries
        .iter()
        .map(|entry| (entry.hashed_path, entry.value.as_slice()))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        indexed.len() == inventory.entries.len(),
        "live inventory contains duplicate paths"
    );
    ensure!(
        base_matched_paths
            .iter()
            .all(|path| indexed.contains_key(path)),
        "base matched paths include a path outside the live inventory"
    );

    let mut analyzer = Analyzer {
        inventory: indexed,
        base_matched_paths,
        read,
        matched: BTreeMap::new(),
        matched_by_family: BTreeMap::new(),
        matched_value_bytes: 0,
        probe_results: BTreeMap::new(),
        derived_reads: 0,
    };
    let mut total_v1_entries = 0_u32;
    let mut total_v2_groups = 0_u32;
    let mut total_v2_entries = 0_u32;
    let mut candidates = Vec::with_capacity(seeded_addresses.len());
    let mut v1_entries = Vec::new();
    let mut v2_validator_groups = Vec::new();
    let mut v2_entries = Vec::new();

    for delegator in seeded_addresses {
        let v1_prefix = undelegation_v1_validators_prefix(delegator);
        let v1_count_probe = analyzer.probe(
            "seeded_undelegation_v1_validator_count",
            iterable_count_key(&v1_prefix),
        )?;
        let v1_count =
            decode_optional_count(&v1_count_probe, "seeded V1 undelegation validator count")?;
        if let Some(count) = v1_count {
            total_v1_entries = total_v1_entries
                .checked_add(count)
                .context("seeded V1 undelegation count overflow")?;
            ensure!(
                total_v1_entries <= MAX_V1_ENTRIES,
                "seeded V1 undelegation entries {total_v1_entries} exceed aggregate bound {MAX_V1_ENTRIES}"
            );
            let mut seen = BTreeSet::new();
            for position in 1..=count {
                let validator = exact_bytes::<20>(
                    analyzer.required(
                        "seeded_undelegation_v1_validator_item",
                        iterable_item_key(&v1_prefix, position),
                    )?,
                    "seeded V1 undelegation validator",
                )?;
                ensure!(
                    seen.insert(validator),
                    "duplicate validator in seeded V1 undelegation iterable"
                );
                let reverse = analyzer.required(
                    "seeded_undelegation_v1_validator_reverse",
                    iterable_position_key(&v1_prefix, &validator),
                )?;
                ensure!(
                    decode_le_u32(&reverse, "seeded V1 validator reverse position")? == position,
                    "seeded V1 undelegation reverse index disagrees with forward position"
                );
                let row = decode_undelegation_v1(&analyzer.required(
                    "seeded_undelegation_v1_record",
                    undelegation_v1_key(delegator, validator),
                )?)?;
                v1_entries.push(SeededUndelegationV1Fact {
                    delegator_hex: hex::encode(delegator),
                    validator_hex: hex::encode(validator),
                    position,
                    amount_hex: uint_hex(&row.amount),
                    block: row.block,
                });
            }
        }

        let last_id_probe =
            analyzer.probe("seeded_undelegation_v2_last_id", last_v2_id_key(delegator))?;
        let last_id = decode_optional_le_u64(&last_id_probe, "seeded V2 last ID")?;

        let v2_prefix = undelegation_v2_validators_prefix(delegator);
        let v2_count_probe = analyzer.probe(
            "seeded_undelegation_v2_validator_count",
            iterable_count_key(&v2_prefix),
        )?;
        let v2_count =
            decode_optional_count(&v2_count_probe, "seeded V2 undelegation validator count")?;
        if let Some(count) = v2_count {
            total_v2_groups = total_v2_groups
                .checked_add(count)
                .context("seeded V2 validator-group count overflow")?;
            ensure!(
                total_v2_groups <= MAX_V2_VALIDATOR_GROUPS,
                "seeded V2 validator groups {total_v2_groups} exceed aggregate bound {MAX_V2_VALIDATOR_GROUPS}"
            );
            let mut seen_validators = BTreeSet::new();
            for validator_position in 1..=count {
                let validator = exact_bytes::<20>(
                    analyzer.required(
                        "seeded_undelegation_v2_validator_item",
                        iterable_item_key(&v2_prefix, validator_position),
                    )?,
                    "seeded V2 undelegation validator",
                )?;
                ensure!(
                    seen_validators.insert(validator),
                    "duplicate validator in seeded V2 undelegation iterable"
                );
                let reverse = analyzer.required(
                    "seeded_undelegation_v2_validator_reverse",
                    iterable_position_key(&v2_prefix, &validator),
                )?;
                ensure!(
                    decode_le_u32(&reverse, "seeded V2 validator reverse position")?
                        == validator_position,
                    "seeded V2 undelegation reverse index disagrees with forward position"
                );

                let ids_prefix = undelegation_v2_ids_prefix(delegator, validator);
                let id_count = decode_le_u32(
                    &analyzer.required(
                        "seeded_undelegation_v2_id_count",
                        iterable_count_key(&ids_prefix),
                    )?,
                    "seeded V2 undelegation ID count",
                )?;
                ensure!(
                    id_count > 0,
                    "seeded V2 validator group has an empty ID iterable"
                );
                total_v2_entries = total_v2_entries
                    .checked_add(id_count)
                    .context("seeded V2 undelegation ID count overflow")?;
                ensure!(
                    total_v2_entries <= MAX_V2_ENTRIES,
                    "seeded V2 undelegation entries {total_v2_entries} exceed aggregate bound {MAX_V2_ENTRIES}"
                );
                v2_validator_groups.push(SeededUndelegationV2GroupFact {
                    delegator_hex: hex::encode(delegator),
                    validator_hex: hex::encode(validator),
                    position: validator_position,
                    id_count,
                });

                let mut seen_ids = BTreeSet::new();
                for id_position in 1..=id_count {
                    let id_bytes = analyzer.required(
                        "seeded_undelegation_v2_id_item",
                        iterable_item_key(&ids_prefix, id_position),
                    )?;
                    let id = decode_le_u64(&id_bytes, "seeded V2 undelegation ID")?;
                    ensure!(
                        seen_ids.insert(id),
                        "duplicate ID in seeded V2 undelegation iterable"
                    );
                    let reverse = analyzer.required(
                        "seeded_undelegation_v2_id_reverse",
                        iterable_position_key(&ids_prefix, &id_bytes),
                    )?;
                    ensure!(
                        decode_le_u32(&reverse, "seeded V2 ID reverse position")? == id_position,
                        "seeded V2 undelegation ID reverse index disagrees with forward position"
                    );
                    let (row, embedded_id) = decode_undelegation_v2(&analyzer.required(
                        "seeded_undelegation_v2_record",
                        undelegation_v2_key(delegator, validator, id),
                    )?)?;
                    ensure!(
                        embedded_id == id,
                        "seeded V2 undelegation object ID disagrees with key ID"
                    );
                    v2_entries.push(SeededUndelegationV2Fact {
                        delegator_hex: hex::encode(delegator),
                        validator_hex: hex::encode(validator),
                        validator_position,
                        id_position,
                        id,
                        amount_hex: uint_hex(&row.amount),
                        block: row.block,
                    });
                }
            }
        }

        candidates.push(SeededUndelegatorFact {
            delegator_hex: hex::encode(delegator),
            v1_validator_count_result: v1_count_probe.result,
            v1_validator_count: v1_count,
            v2_validator_count_result: v2_count_probe.result,
            v2_validator_count: v2_count,
            v2_last_id_result: last_id_probe.result,
            v2_last_id: last_id,
        });
    }

    let additional_matched_live_entries = u64::try_from(analyzer.matched.len())?;
    let resulting_matched_live_entries = u64::try_from(base_matched_paths.len())?
        .checked_add(additional_matched_live_entries)
        .context("resulting matched live-entry count overflow")?;
    let resulting_matched_live_value_bytes = base_matched_value_bytes
        .checked_add(analyzer.matched_value_bytes)
        .context("resulting matched value-byte count overflow")?;
    let unexplained = inventory
        .entries
        .iter()
        .filter(|entry| {
            !base_matched_paths.contains(&entry.hashed_path)
                && !analyzer.matched.contains_key(&entry.hashed_path)
        })
        .collect::<Vec<_>>();
    let resulting_unexplained_live_entries = u64::try_from(unexplained.len())?;
    ensure!(
        resulting_matched_live_entries.checked_add(resulting_unexplained_live_entries)
            == Some(u64::try_from(inventory.entries.len())?),
        "seeded undelegation matches and unexplained rows do not partition live inventory"
    );
    let resulting_unexplained_live_value_bytes =
        unexplained.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(u64::try_from(entry.value.len())?)
                .context("unexplained live value-byte count overflow")
        })?;
    ensure!(
        resulting_matched_live_value_bytes.checked_add(resulting_unexplained_live_value_bytes)
            == Some(inventory.value_bytes),
        "seeded undelegation matched and unexplained bytes do not partition live inventory"
    );

    Ok(SeededUndelegationCoverage {
        candidate_count,
        max_seeded_undelegators: MAX_SEEDED_UNDELEGATORS,
        max_v1_entries: MAX_V1_ENTRIES,
        max_v2_validator_groups: MAX_V2_VALIDATOR_GROUPS,
        max_v2_entries: MAX_V2_ENTRIES,
        max_derived_reads: MAX_DERIVED_READS,
        derived_reads: analyzer.derived_reads,
        v1_entries,
        v2_validator_groups,
        v2_entries,
        probe_results: analyzer.probe_results,
        candidates,
        additional_matched_live_entries,
        additional_matched_live_value_bytes: analyzer.matched_value_bytes,
        additional_matched_by_family: analyzer.matched_by_family,
        resulting_matched_live_entries,
        resulting_matched_live_value_bytes,
        resulting_unexplained_live_entries,
        resulting_unexplained_live_value_bytes,
        resulting_unexplained_paths_sha256: digest_paths(&unexplained),
        resulting_unexplained_entries_sha256: digest_entries(&unexplained),
        live_partition_exact: true,
        global_delegator_set_complete: false,
        semantic_snapshot_complete: false,
    })
}

/// Returns the irreversible trie path for a native logical storage key.
pub fn hashed_storage_path(key: ConcreteStorageKey) -> [u8; 32] {
    storage_key(&[&key.0]).0
}

fn decode_optional_count(probe: &Probe, label: &str) -> Result<Option<u32>> {
    match probe.result {
        "live_present" => Ok(Some(decode_le_u32(
            probe.value.as_deref().context("live count has no value")?,
            label,
        )?)),
        _ => Ok(None),
    }
}

fn decode_optional_le_u64(probe: &Probe, label: &str) -> Result<Option<u64>> {
    match probe.result {
        "live_present" => Ok(Some(decode_le_u64(
            probe.value.as_deref().context("live uint64 has no value")?,
            label,
        )?)),
        _ => Ok(None),
    }
}

fn undelegation_v1_validators_prefix(delegator: [u8; 20]) -> Vec<u8> {
    [&[3, 1][..], &delegator].concat()
}

fn undelegation_v2_validators_prefix(delegator: [u8; 20]) -> Vec<u8> {
    [&[3, 2][..], &delegator].concat()
}

fn undelegation_v2_ids_prefix(delegator: [u8; 20], validator: [u8; 20]) -> Vec<u8> {
    [&[3, 3][..], &delegator, &validator].concat()
}

fn iterable_count_key(prefix: &[u8]) -> ConcreteStorageKey {
    storage_key(&[prefix, &[1]])
}

fn iterable_item_key(prefix: &[u8], position: u32) -> ConcreteStorageKey {
    // The pinned iterable constructors alias item and reverse discriminator 2.
    storage_key(&[prefix, &[2], &position.to_le_bytes()])
}

fn iterable_position_key(prefix: &[u8], item: &[u8]) -> ConcreteStorageKey {
    storage_key(&[prefix, &[2], item])
}

fn undelegation_v1_key(delegator: [u8; 20], validator: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[3, 0], &validator, &delegator])
}

fn last_v2_id_key(delegator: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[3, 4], &delegator])
}

fn undelegation_v2_key(delegator: [u8; 20], validator: [u8; 20], id: u64) -> ConcreteStorageKey {
    storage_key(&[&[3, 0], &delegator, &validator, &id.to_le_bytes()])
}

fn storage_key(parts: &[&[u8]]) -> ConcreteStorageKey {
    let mut hasher = Keccak::v256();
    for part in parts {
        hasher.update(part);
    }
    let mut key = [0; 32];
    hasher.finalize(&mut key);
    ConcreteStorageKey(key)
}

fn decode_undelegation_v1(bytes: &[u8]) -> Result<UndelegationRow> {
    decode_undelegation_row(
        exact_rlp(bytes, "V1 undelegation")?,
        bytes,
        "V1 undelegation",
    )
}

fn decode_undelegation_v2(bytes: &[u8]) -> Result<(UndelegationRow, u64)> {
    let outer = exact_rlp(bytes, "V2 undelegation")?;
    ensure!(
        outer.item_count()? == 2,
        "V2 undelegation must have two fields"
    );
    let embedded = outer.at(0)?;
    ensure!(
        embedded.is_list(),
        "V2 undelegation base must be an RLP list"
    );
    let row = decode_undelegation_row(embedded, outer.at(0)?.as_raw(), "V2 undelegation base")?;
    let id_item = outer.at(1)?;
    require_data(&id_item, "V2 undelegation ID")?;
    let id = id_item.as_val::<u64>().context("V2 undelegation ID")?;
    let mut base = RlpStream::new_list(2);
    append_uint(&mut base, &row.amount);
    base.append(&row.block);
    let base = base.out();
    let mut canonical = RlpStream::new_list(2);
    canonical.append_raw(&base, 1).append(&id);
    ensure!(
        canonical.out().as_ref() == bytes,
        "V2 undelegation is not canonical or completely consumed"
    );
    Ok((row, id))
}

fn decode_undelegation_row(row: Rlp<'_>, bytes: &[u8], label: &str) -> Result<UndelegationRow> {
    ensure!(row.is_list(), "{label} must be an RLP list");
    ensure!(row.item_count()? == 2, "{label} must have two fields");
    let amount = decode_uint_item(&row.at(0)?, "undelegation amount")?;
    let block_item = row.at(1)?;
    require_data(&block_item, "undelegation block")?;
    let block = block_item.as_val::<u64>().context("undelegation block")?;
    let mut canonical = RlpStream::new_list(2);
    append_uint(&mut canonical, &amount);
    canonical.append(&block);
    ensure!(
        canonical.out().as_ref() == bytes,
        "{label} is not canonical or completely consumed"
    );
    Ok(UndelegationRow { amount, block })
}

fn exact_rlp<'a>(bytes: &'a [u8], label: &str) -> Result<Rlp<'a>> {
    let row = Rlp::new(bytes);
    let payload = row.payload_info().with_context(|| format!("{label} RLP"))?;
    let total = payload
        .header_len
        .checked_add(payload.value_len)
        .with_context(|| format!("{label} RLP length overflow"))?;
    ensure!(total == bytes.len(), "{label} has trailing bytes");
    Ok(row)
}

fn decode_uint_item(item: &Rlp<'_>, label: &str) -> Result<BigUint> {
    let bytes = require_data(item, label)?;
    ensure!(
        bytes.first() != Some(&0),
        "{label} has a leading-zero unsigned integer"
    );
    Ok(BigUint::from_bytes_be(bytes))
}

fn require_data<'a>(item: &'a Rlp<'_>, label: &str) -> Result<&'a [u8]> {
    ensure!(!item.is_list(), "{label} must be RLP data");
    item.data().with_context(|| format!("{label} bytes"))
}

fn append_uint(stream: &mut RlpStream, value: &BigUint) {
    let bytes = value.to_bytes_be();
    let bytes = if bytes == [0] { Vec::new() } else { bytes };
    stream.append(&bytes.as_slice());
}

fn uint_hex(value: &BigUint) -> String {
    let bytes = value.to_bytes_be();
    hex::encode(if bytes == [0] { &[][..] } else { &bytes })
}

fn decode_le_u32(bytes: &[u8], label: &str) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes.try_into().with_context(|| {
        format!("{label} must contain four little-endian bytes")
    })?))
}

fn decode_le_u64(bytes: &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_le_bytes(bytes.try_into().with_context(|| {
        format!("{label} must contain eight little-endian bytes")
    })?))
}

fn exact_bytes<const N: usize>(value: Vec<u8>, label: &str) -> Result<[u8; N]> {
    value
        .try_into()
        .map_err(|value: Vec<u8>| anyhow::anyhow!("{label} has {} bytes", value.len()))
}

fn digest_paths(entries: &[&ConcreteStorageInventoryEntry]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.hashed_path);
    }
    hex::encode(digest.finalize())
}

fn digest_entries(entries: &[&ConcreteStorageInventoryEntry]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.hashed_path);
        digest.update((entry.value.len() as u64).to_be_bytes());
        digest.update(&entry.value);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_types::FinalChainBlockNumber;
    use rustaxa_types::concrete_state::ConcreteStateIdentity;
    use std::cell::Cell;

    const DELEGATOR: [u8; 20] = [0x11; 20];
    const VALIDATOR: [u8; 20] = [0x22; 20];

    #[test]
    fn exact_go_v1_and_v2_rows_extend_the_live_partition() {
        let v1_prefix = undelegation_v1_validators_prefix(DELEGATOR);
        let v2_prefix = undelegation_v2_validators_prefix(DELEGATOR);
        let ids_prefix = undelegation_v2_ids_prefix(DELEGATOR, VALIDATOR);
        let id = 1_u64;
        let id_bytes = id.to_le_bytes();
        let mut rows = BTreeMap::new();

        insert_live(
            &mut rows,
            iterable_count_key(&v1_prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_item_key(&v1_prefix, 1),
            VALIDATOR.to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_position_key(&v1_prefix, &VALIDATOR),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            undelegation_v1_key(DELEGATOR, VALIDATOR),
            hex::decode("c482012c04").unwrap(),
        );

        // Last IDs are monotonic allocation cursors and may exceed live IDs.
        insert_live(
            &mut rows,
            last_v2_id_key(DELEGATOR),
            2_u64.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_count_key(&v2_prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_item_key(&v2_prefix, 1),
            VALIDATOR.to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_position_key(&v2_prefix, &VALIDATOR),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_count_key(&ids_prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_item_key(&ids_prefix, 1),
            id_bytes.to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_position_key(&ids_prefix, &id_bytes),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            undelegation_v2_key(DELEGATOR, VALIDATOR, id),
            hex::decode("c6c482012c0401").unwrap(),
        );

        let coverage = analyze_rows(rows).unwrap();
        assert_eq!(coverage.derived_reads, 12);
        assert_eq!(coverage.additional_matched_live_entries, 12);
        assert_eq!(coverage.resulting_unexplained_live_entries, 0);
        assert_eq!(coverage.v1_entries.len(), 1);
        assert_eq!(coverage.v1_entries[0].amount_hex, "012c");
        assert_eq!(coverage.v1_entries[0].block, 4);
        assert_eq!(coverage.v2_entries.len(), 1);
        assert_eq!(coverage.v2_entries[0].amount_hex, "012c");
        assert_eq!(coverage.v2_entries[0].block, 4);
        assert_eq!(coverage.v2_entries[0].id, 1);
        assert_eq!(coverage.candidates[0].v2_last_id, Some(2));
        assert!(coverage.live_partition_exact);

        let zero_v1 = decode_undelegation_v1(&hex::decode("c28080").unwrap()).unwrap();
        assert_eq!(zero_v1.amount, BigUint::from(0_u8));
        assert_eq!(zero_v1.block, 0);
    }

    #[test]
    fn malformed_canonical_v1_rlp_is_rejected() {
        let prefix = undelegation_v1_validators_prefix(DELEGATOR);
        let mut rows = BTreeMap::new();
        insert_live(
            &mut rows,
            iterable_count_key(&prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(&mut rows, iterable_item_key(&prefix, 1), VALIDATOR.to_vec());
        insert_live(
            &mut rows,
            iterable_position_key(&prefix, &VALIDATOR),
            1_u32.to_le_bytes().to_vec(),
        );
        let mut malformed = RlpStream::new_list(2);
        malformed.append(&[0_u8].as_slice()).append(&7_u64);
        insert_live(
            &mut rows,
            undelegation_v1_key(DELEGATOR, VALIDATOR),
            malformed.out().to_vec(),
        );

        let error = analyze_rows(rows).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("undelegation amount has a leading-zero unsigned integer")
        );
    }

    #[test]
    fn v1_reverse_position_mismatch_is_rejected() {
        let prefix = undelegation_v1_validators_prefix(DELEGATOR);
        let mut rows = BTreeMap::new();
        insert_live(
            &mut rows,
            iterable_count_key(&prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(&mut rows, iterable_item_key(&prefix, 1), VALIDATOR.to_vec());
        insert_live(
            &mut rows,
            iterable_position_key(&prefix, &VALIDATOR),
            2_u32.to_le_bytes().to_vec(),
        );

        let error = analyze_rows(rows).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("seeded V1 undelegation reverse index disagrees")
        );
    }

    #[test]
    fn v2_embedded_id_mismatch_is_rejected() {
        let validator_prefix = undelegation_v2_validators_prefix(DELEGATOR);
        let ids_prefix = undelegation_v2_ids_prefix(DELEGATOR, VALIDATOR);
        let id = 7_u64;
        let id_bytes = id.to_le_bytes();
        let mut rows = BTreeMap::new();
        insert_live(
            &mut rows,
            iterable_count_key(&validator_prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_item_key(&validator_prefix, 1),
            VALIDATOR.to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_position_key(&validator_prefix, &VALIDATOR),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_count_key(&ids_prefix),
            1_u32.to_le_bytes().to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_item_key(&ids_prefix, 1),
            id_bytes.to_vec(),
        );
        insert_live(
            &mut rows,
            iterable_position_key(&ids_prefix, &id_bytes),
            1_u32.to_le_bytes().to_vec(),
        );
        let mut base = RlpStream::new_list(2);
        base.append(&5_u64).append(&9_u64);
        let mut object = RlpStream::new_list(2);
        object.append_raw(&base.out(), 1).append(&(id + 1));
        insert_live(
            &mut rows,
            undelegation_v2_key(DELEGATOR, VALIDATOR, id),
            object.out().to_vec(),
        );

        let error = analyze_rows(rows).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("seeded V2 undelegation object ID disagrees with key ID")
        );
    }

    #[test]
    fn authenticated_live_paths_reject_unavailable_and_tombstone_reads() {
        let key = iterable_count_key(&undelegation_v1_validators_prefix(DELEGATOR));
        let mut rows = BTreeMap::new();
        insert_live(&mut rows, key, 0_u32.to_le_bytes().to_vec());
        let inventory = inventory_from_rows(&rows);
        let identity = test_identity();

        let unavailable = analyze_seeded_undelegations(
            &inventory,
            &BTreeSet::new(),
            0,
            BTreeSet::from([DELEGATOR]),
            |read_key| {
                if read_key == key {
                    Err(ConcreteReadError::HistoryUnavailable(identity))
                } else {
                    Ok(ConcreteRead::Absent)
                }
            },
        )
        .unwrap_err();
        assert!(unavailable.to_string().contains("history_unavailable"));
        assert!(unavailable.to_string().contains("path is live"));

        let tombstone = analyze_seeded_undelegations(
            &inventory,
            &BTreeSet::new(),
            0,
            BTreeSet::from([DELEGATOR]),
            |read_key| {
                Ok(if read_key == key {
                    ConcreteRead::Tombstone
                } else {
                    ConcreteRead::Absent
                })
            },
        )
        .unwrap_err();
        assert!(tombstone.to_string().contains("tombstone"));
        assert!(tombstone.to_string().contains("path is live"));
    }

    #[test]
    fn authenticated_nonlive_results_remain_typed_and_do_not_become_zero_counts() {
        let v1_key = iterable_count_key(&undelegation_v1_validators_prefix(DELEGATOR));
        let last_id_key = last_v2_id_key(DELEGATOR);
        let identity = test_identity();
        let coverage = analyze_seeded_undelegations(
            &inventory_from_rows(&BTreeMap::new()),
            &BTreeSet::new(),
            0,
            BTreeSet::from([DELEGATOR]),
            |key| {
                if key == v1_key {
                    Err(ConcreteReadError::HistoryUnavailable(identity))
                } else if key == last_id_key {
                    Ok(ConcreteRead::Tombstone)
                } else {
                    Ok(ConcreteRead::Absent)
                }
            },
        )
        .unwrap();

        let candidate = &coverage.candidates[0];
        assert_eq!(candidate.v1_validator_count_result, "history_unavailable");
        assert_eq!(candidate.v1_validator_count, None);
        assert_eq!(candidate.v2_last_id_result, "tombstone");
        assert_eq!(candidate.v2_last_id, None);
        assert_eq!(candidate.v2_validator_count_result, "absent");
        assert_eq!(candidate.v2_validator_count, None);
        assert_eq!(coverage.additional_matched_live_entries, 0);
    }

    #[test]
    fn aggregate_count_bound_rejects_before_item_reads() {
        let count_key = iterable_count_key(&undelegation_v1_validators_prefix(DELEGATOR));
        let mut rows = BTreeMap::new();
        insert_live(
            &mut rows,
            count_key,
            (MAX_V1_ENTRIES + 1).to_le_bytes().to_vec(),
        );
        let inventory = inventory_from_rows(&rows);
        let reads = Cell::new(0_u32);
        let error = analyze_seeded_undelegations(
            &inventory,
            &BTreeSet::new(),
            0,
            BTreeSet::from([DELEGATOR]),
            |key| {
                reads.set(reads.get() + 1);
                Ok(if key == count_key {
                    ConcreteRead::Present((MAX_V1_ENTRIES + 1).to_le_bytes().to_vec())
                } else {
                    ConcreteRead::Absent
                })
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("exceed aggregate bound"));
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn base_overlap_and_inventory_byte_mismatch_are_rejected() {
        let count_key = iterable_count_key(&undelegation_v1_validators_prefix(DELEGATOR));
        let path = hashed_storage_path(count_key);
        let mut rows = BTreeMap::new();
        insert_live(&mut rows, count_key, 0_u32.to_le_bytes().to_vec());
        let inventory = inventory_from_rows(&rows);

        let overlap = analyze_seeded_undelegations(
            &inventory,
            &BTreeSet::from([path]),
            4,
            BTreeSet::from([DELEGATOR]),
            |key| {
                Ok(if key == count_key {
                    ConcreteRead::Present(0_u32.to_le_bytes().to_vec())
                } else {
                    ConcreteRead::Absent
                })
            },
        )
        .unwrap_err();
        assert!(
            overlap
                .to_string()
                .contains("already matched by the base inverse")
        );

        let mismatch = analyze_seeded_undelegations(
            &inventory,
            &BTreeSet::new(),
            0,
            BTreeSet::from([DELEGATOR]),
            |key| {
                Ok(if key == count_key {
                    ConcreteRead::Present(1_u32.to_le_bytes().to_vec())
                } else {
                    ConcreteRead::Absent
                })
            },
        )
        .unwrap_err();
        assert!(mismatch.to_string().contains("differs from live inventory"));
    }

    fn analyze_rows(
        rows: BTreeMap<[u8; 32], (ConcreteStorageKey, Vec<u8>)>,
    ) -> Result<SeededUndelegationCoverage> {
        let inventory = inventory_from_rows(&rows);
        analyze_seeded_undelegations(
            &inventory,
            &BTreeSet::new(),
            0,
            BTreeSet::from([DELEGATOR]),
            |key| {
                Ok(rows
                    .get(&key.0)
                    .map(|(_, value)| ConcreteRead::Present(value.clone()))
                    .unwrap_or(ConcreteRead::Absent))
            },
        )
    }

    fn inventory_from_rows(
        rows: &BTreeMap<[u8; 32], (ConcreteStorageKey, Vec<u8>)>,
    ) -> ConcreteStorageInventory {
        let mut entries = rows
            .values()
            .map(|(key, value)| ConcreteStorageInventoryEntry {
                hashed_path: hashed_storage_path(*key),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.hashed_path);
        let value_bytes = entries.iter().map(|entry| entry.value.len() as u64).sum();
        ConcreteStorageInventory {
            identity: test_identity(),
            address: [0xfe; 20],
            storage_root: Some([0x44; 32]),
            nodes_visited: 1,
            value_bytes,
            entries,
        }
    }

    fn test_identity() -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(1),
            state_root: [0x33; 32],
        }
    }

    fn insert_live(
        rows: &mut BTreeMap<[u8; 32], (ConcreteStorageKey, Vec<u8>)>,
        key: ConcreteStorageKey,
        value: Vec<u8>,
    ) {
        assert!(rows.insert(key.0, (key, value)).is_none());
    }
}
