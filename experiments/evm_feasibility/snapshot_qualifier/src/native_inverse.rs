//! Partial inverse decoding of enumerable live DPoS storage.
//!
//! This diagnostic starts from the global validator iterable, derives only key
//! preimages justified by that index and fixed layout fields, and compares each
//! derived live row with a complete authenticated storage inventory. Hashed
//! paths that cannot be inverted remain unexplained. The result is evidence
//! about current live coverage, not a complete DPoS snapshot, historical-key
//! catalog, import artifact, or state-publication authority.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use num_bigint::BigUint;
use rlp::{Rlp, RlpStream};
use rustaxa_storage::{ConcreteStorageInventory, ConcreteStorageInventoryEntry};
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStorageKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};

/// Highest validator count accepted by the bounded diagnostic.
pub const MAX_VALIDATORS: u32 = 4_096;

/// Highest cumulative delegation count accepted for address-seeded expansion.
pub const MAX_SEEDED_DELEGATIONS: u32 = 4_096;

/// DPoS native-contract address.
pub const DPOS_CONTRACT_ADDRESS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
];

const VALIDATORS_PREFIX: &[u8] = &[0, 5];

/// Strictly decoded facts for one validator reachable from the global iterable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidatorInverseFact {
    pub position: u32,
    pub address_hex: String,
    pub validator_encoding: &'static str,
    pub stake_hex: String,
    pub commission: u16,
    pub last_commission_change: u64,
    pub reward_head: u64,
    pub undelegations_count: Option<u16>,
    pub description_hex: String,
    pub endpoint_hex: String,
    pub delegator_rewards_hex: String,
    pub commission_rewards_hex: String,
    pub owner_hex: String,
    pub vrf_hex: String,
    pub current_reward_node_result: &'static str,
    pub current_reward_per_stake_hex: Option<String>,
    pub current_reward_node_count: Option<u32>,
}

/// Physical classification and decoded value for one fixed global field.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScalarInverseFact {
    pub field: u8,
    pub name: &'static str,
    pub physical_result: &'static str,
    pub value_hex: Option<String>,
}

/// Delegation-map classification for one already authenticated address.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededDelegatorFact {
    pub address_hex: String,
    pub sources: Vec<&'static str>,
    pub delegation_count_result: &'static str,
    pub delegation_count: Option<u32>,
}

/// One live delegation reached through an address-seeded iterable map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededDelegationFact {
    pub delegator_hex: String,
    pub validator_hex: String,
    pub position: u32,
    pub stake_hex: String,
    pub last_updated: u64,
    pub reward_node_result: &'static str,
    pub reward_per_stake_hex: Option<String>,
    pub reward_node_count: Option<u32>,
}

/// Bounded partial expansion rooted only in validator and owner addresses.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SeededDelegationCoverage {
    pub candidate_count: u32,
    pub max_delegation_entries: u32,
    pub live_delegation_entries: u32,
    pub count_results: BTreeMap<String, u64>,
    pub candidates: Vec<SeededDelegatorFact>,
    pub delegations: Vec<SeededDelegationFact>,
    pub global_delegator_set_complete: bool,
}

/// Exact current live coverage explained by the bounded inverse.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NativeInverseCoverage {
    pub validator_count: u32,
    pub validators: Vec<ValidatorInverseFact>,
    pub scalars: Vec<ScalarInverseFact>,
    pub seeded_delegations: SeededDelegationCoverage,
    pub matched_live_entries: u64,
    pub matched_live_value_bytes: u64,
    pub matched_by_family: BTreeMap<String, u64>,
    pub unexplained_live_entries: u64,
    pub unexplained_live_value_bytes: u64,
    pub unexplained_paths_sha256: String,
    pub unexplained_entries_sha256: String,
    pub first_unexplained_paths_hex: Vec<String>,
    pub live_partition_exact: bool,
    pub semantic_snapshot_complete: bool,
    pub unresolved_families: Vec<&'static str>,
}

struct ValidatorRow {
    encoding: &'static str,
    stake: BigUint,
    commission: u16,
    last_commission_change: u64,
    reward_head: u64,
    undelegations_count: Option<u16>,
}

struct DelegationRow {
    stake: BigUint,
    last_updated: u64,
}

struct PhysicalProbe {
    result: &'static str,
    value: Option<Vec<u8>>,
}

struct Analyzer<'a, F> {
    inventory: BTreeMap<[u8; 32], &'a [u8]>,
    read: F,
    known: BTreeMap<[u8; 32], &'static str>,
    matched_by_family: BTreeMap<String, u64>,
    matched_value_bytes: u64,
}

impl<'a, F> Analyzer<'a, F>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    fn required(&mut self, family: &'static str, key: ConcreteStorageKey) -> Result<Vec<u8>> {
        let read = self.classified(family, key)?;
        let ConcreteRead::Present(value) = read else {
            bail!("required {family} row is not physically live: {read:?}");
        };
        Ok(value)
    }

    fn classified(
        &mut self,
        family: &'static str,
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>> {
        let read = (self.read)(key)
            .map_err(anyhow::Error::new)
            .with_context(|| format!("read derived {family} logical key {}", hex::encode(key.0)))?;
        match &read {
            ConcreteRead::Present(value) => self.match_live(family, key, value)?,
            ConcreteRead::Absent | ConcreteRead::Tombstone => ensure!(
                !self.inventory.contains_key(&keccak256(&key.0)),
                "derived {family} row is {read:?} but its path is live in the inventory"
            ),
        }
        Ok(read)
    }

    fn optional(
        &mut self,
        family: &'static str,
        key: ConcreteStorageKey,
    ) -> Result<(&'static str, Option<Vec<u8>>)> {
        match (self.read)(key) {
            Ok(ConcreteRead::Present(value)) => {
                self.match_live(family, key, &value)?;
                Ok(("present", Some(value)))
            }
            Ok(ConcreteRead::Absent) => {
                self.ensure_not_live(family, key, "absent")?;
                Ok(("absent", None))
            }
            Ok(ConcreteRead::Tombstone) => {
                self.ensure_not_live(family, key, "tombstone")?;
                Ok(("tombstone", None))
            }
            Err(ConcreteReadError::HistoryUnavailable(_)) => {
                self.ensure_not_live(family, key, "history_unavailable")?;
                Ok(("history_unavailable", None))
            }
            Err(ConcreteReadError::Pruned(_)) => {
                self.ensure_not_live(family, key, "pruned")?;
                Ok(("pruned", None))
            }
            Err(error) => Err(anyhow::Error::new(error)).with_context(|| {
                format!("read derived {family} logical key {}", hex::encode(key.0))
            }),
        }
    }

    fn ensure_not_live(
        &self,
        family: &'static str,
        key: ConcreteStorageKey,
        physical_result: &'static str,
    ) -> Result<()> {
        ensure!(
            !self.inventory.contains_key(&keccak256(&key.0)),
            "derived {family} row is {physical_result} but its path is live in the inventory"
        );
        Ok(())
    }

    fn physical_probe(
        &mut self,
        family: &'static str,
        key: ConcreteStorageKey,
        repeated_live_reference_allowed: bool,
    ) -> Result<PhysicalProbe> {
        let path = keccak256(&key.0);
        match (self.read)(key) {
            Ok(ConcreteRead::Present(value)) => match self.inventory.get(&path) {
                Some(inventory_value) => {
                    ensure!(
                        *inventory_value == value,
                        "derived {family} row differs from live inventory"
                    );
                    if self.known.contains_key(&path) {
                        ensure!(
                            repeated_live_reference_allowed,
                            "derived {family} path was already classified"
                        );
                        Ok(PhysicalProbe {
                            result: "live_present_already_matched",
                            value: Some(value),
                        })
                    } else {
                        self.match_live(family, key, &value)?;
                        Ok(PhysicalProbe {
                            result: "live_present",
                            value: Some(value),
                        })
                    }
                }
                None => Ok(PhysicalProbe {
                    result: "orphan_present",
                    value: Some(value),
                }),
            },
            Ok(ConcreteRead::Absent) => {
                self.ensure_not_live(family, key, "absent")?;
                Ok(PhysicalProbe {
                    result: "absent",
                    value: None,
                })
            }
            Ok(ConcreteRead::Tombstone) => {
                self.ensure_not_live(family, key, "tombstone")?;
                Ok(PhysicalProbe {
                    result: "tombstone",
                    value: None,
                })
            }
            Err(ConcreteReadError::HistoryUnavailable(_)) => {
                self.ensure_not_live(family, key, "history_unavailable")?;
                Ok(PhysicalProbe {
                    result: "history_unavailable",
                    value: None,
                })
            }
            Err(ConcreteReadError::Pruned(_)) => {
                self.ensure_not_live(family, key, "pruned")?;
                Ok(PhysicalProbe {
                    result: "pruned",
                    value: None,
                })
            }
            Err(error) => Err(anyhow::Error::new(error)).with_context(|| {
                format!("read derived {family} logical key {}", hex::encode(key.0))
            }),
        }
    }

    fn scalar(&mut self, field: u8, name: &'static str) -> Result<ScalarInverseFact> {
        let key = storage_key(&[&[field]]);
        let read = self.classified("global_scalar", key)?;
        match read {
            ConcreteRead::Present(value) => {
                let decoded = match field {
                    4 => compact_u64_bytes(decode_compact_u64(&value, name)?),
                    8 => compact_u64_bytes(decode_rlp_u64(&value, "yield")?),
                    _ => {
                        canonical_uint(&value, name)?;
                        value.clone()
                    }
                };
                Ok(ScalarInverseFact {
                    field,
                    name,
                    physical_result: "present",
                    value_hex: Some(hex::encode(decoded)),
                })
            }
            ConcreteRead::Absent => Ok(ScalarInverseFact {
                field,
                name,
                physical_result: "absent",
                value_hex: None,
            }),
            ConcreteRead::Tombstone => Ok(ScalarInverseFact {
                field,
                name,
                physical_result: "tombstone",
                value_hex: None,
            }),
        }
    }

    fn match_live(
        &mut self,
        family: &'static str,
        logical_key: ConcreteStorageKey,
        value: &[u8],
    ) -> Result<()> {
        let path = keccak256(&logical_key.0);
        if let Some(previous) = self.known.insert(path, family) {
            bail!("derived live path collision between {previous} and {family}");
        }
        let inventory_value = self
            .inventory
            .get(&path)
            .with_context(|| format!("derived {family} row is absent from live inventory"))?;
        ensure!(
            *inventory_value == value,
            "derived {family} row differs from live inventory"
        );
        *self.matched_by_family.entry(family.to_owned()).or_default() += 1;
        self.matched_value_bytes = self
            .matched_value_bytes
            .checked_add(u64::try_from(value.len())?)
            .context("matched live value-byte count overflow")?;
        Ok(())
    }

    fn finish(
        self,
        validator_count: u32,
        validators: Vec<ValidatorInverseFact>,
        scalars: Vec<ScalarInverseFact>,
        seeded_delegations: SeededDelegationCoverage,
    ) -> Result<NativeInverseCoverage> {
        let unexplained = self
            .inventory
            .iter()
            .filter(|(path, _)| !self.known.contains_key(*path))
            .map(|(path, value)| ConcreteStorageInventoryEntry {
                hashed_path: *path,
                value: value.to_vec(),
            })
            .collect::<Vec<_>>();
        let matched_live_entries = u64::try_from(self.known.len())?;
        let unexplained_live_entries = u64::try_from(unexplained.len())?;
        let inventory_entries = u64::try_from(self.inventory.len())?;
        ensure!(
            matched_live_entries.checked_add(unexplained_live_entries) == Some(inventory_entries),
            "matched and unexplained rows do not partition the live inventory"
        );
        let unexplained_live_value_bytes = unexplained.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(u64::try_from(entry.value.len())?)
                .context("unexplained live value-byte count overflow")
        })?;
        let first_unexplained_paths_hex = unexplained
            .iter()
            .take(16)
            .map(|entry| hex::encode(entry.hashed_path))
            .collect();

        Ok(NativeInverseCoverage {
            validator_count,
            validators,
            scalars,
            seeded_delegations,
            matched_live_entries,
            matched_live_value_bytes: self.matched_value_bytes,
            matched_by_family: self.matched_by_family,
            unexplained_live_entries,
            unexplained_live_value_bytes,
            unexplained_paths_sha256: digest_paths(&unexplained),
            unexplained_entries_sha256: digest_entries(&unexplained),
            first_unexplained_paths_hex,
            live_partition_exact: true,
            semantic_snapshot_complete: false,
            unresolved_families: vec![
                "delegator-scoped delegation and iterable rows",
                "V1/V2 undelegation rows and delegator indexes",
                "non-head reward graph nodes and delegation cursors",
                "slashing jail-block and double-voting-proof key preimages",
                "deleted and all-ever historical keys",
            ],
        })
    }
}

/// Partially decodes current DPoS rows justified by enumerable key preimages.
///
/// `inventory` must be the complete authenticated live inventory for the DPoS
/// account at the selected identity. `read` must expose exact physical rows at
/// that same identity. Every derived live row is required to agree with the
/// inventory. A physically unavailable derived reward-head row is reported as
/// unavailable only when its path is absent from the authenticated inventory;
/// this does not assert physical absence or synthesize a graph node. Missing or
/// corrupt required inputs fail without returning partial facts.
pub fn analyze_native_head<F>(
    inventory: &ConcreteStorageInventory,
    read: F,
) -> Result<NativeInverseCoverage>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    let indexed = inventory
        .entries
        .iter()
        .map(|entry| (entry.hashed_path, entry.value.as_slice()))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        indexed.len() == inventory.entries.len(),
        "live inventory contains duplicate paths"
    );
    let mut analyzer = Analyzer {
        inventory: indexed,
        read,
        known: BTreeMap::new(),
        matched_by_family: BTreeMap::new(),
        matched_value_bytes: 0,
    };

    let count_bytes = analyzer.required("validator_index_count", iterable_count_key())?;
    let validator_count = decode_le_u32(&count_bytes, "validator count")?;
    ensure!(
        validator_count <= MAX_VALIDATORS,
        "validator count {validator_count} exceeds diagnostic limit {MAX_VALIDATORS}"
    );

    let mut seen = BTreeSet::new();
    let mut seeded_addresses = BTreeMap::<[u8; 20], BTreeSet<&'static str>>::new();
    let mut validators = Vec::with_capacity(usize::try_from(validator_count)?);
    for position in 1..=validator_count {
        let address_bytes =
            analyzer.required("validator_index_item", iterable_item_key(position))?;
        let address: [u8; 20] = address_bytes.try_into().map_err(|value: Vec<u8>| {
            anyhow::anyhow!("validator item has {} bytes", value.len())
        })?;
        ensure!(
            seen.insert(address),
            "duplicate validator address in global iterable"
        );
        seeded_addresses
            .entry(address)
            .or_default()
            .insert("validator");

        let reverse =
            analyzer.required("validator_index_reverse", iterable_position_key(address))?;
        ensure!(
            decode_le_u32(&reverse, "validator reverse position")? == position,
            "validator reverse index disagrees with forward position"
        );

        let validator_bytes =
            analyzer.required("validator_record", storage_key(&[&[0, 0], &address]))?;
        let validator = decode_validator(&validator_bytes).with_context(|| {
            format!(
                "validator position {position} address {} row {}",
                hex::encode(address),
                hex::encode(&validator_bytes)
            )
        })?;
        let (description, endpoint) = decode_pair_bytes(
            &analyzer.required("validator_info", storage_key(&[&[0, 1], &address]))?,
            "validator info",
        )?;
        let (delegator_rewards, commission_rewards) = decode_pair_uints(
            &analyzer.required("validator_rewards", storage_key(&[&[0, 2], &address]))?,
            "validator rewards",
        )?;
        let owner = exact_bytes::<20>(
            analyzer.required("validator_owner", storage_key(&[&[0, 3], &address]))?,
            "validator owner",
        )?;
        seeded_addresses.entry(owner).or_default().insert("owner");
        let vrf = exact_bytes::<32>(
            analyzer.required("validator_vrf", storage_key(&[&[0, 4], &address]))?,
            "validator VRF",
        )?;
        let head = compact_u64_bytes(validator.reward_head);
        let (current_reward_node_result, reward_per_stake, current_reward_node_count) =
            match analyzer.optional("current_reward_node", storage_key(&[&[1], &address, &head]))? {
                (result, Some(value)) => {
                    let (reward_per_stake, count) = decode_reward_node(&value)?;
                    (result, Some(reward_per_stake), Some(count))
                }
                (result, None) => (result, None, None),
            };

        validators.push(ValidatorInverseFact {
            position,
            address_hex: hex::encode(address),
            validator_encoding: validator.encoding,
            stake_hex: uint_hex(&validator.stake),
            commission: validator.commission,
            last_commission_change: validator.last_commission_change,
            reward_head: validator.reward_head,
            undelegations_count: validator.undelegations_count,
            description_hex: hex::encode(description),
            endpoint_hex: hex::encode(endpoint),
            delegator_rewards_hex: uint_hex(&delegator_rewards),
            commission_rewards_hex: uint_hex(&commission_rewards),
            owner_hex: hex::encode(owner),
            vrf_hex: hex::encode(vrf),
            current_reward_node_result,
            current_reward_per_stake_hex: reward_per_stake.as_ref().map(uint_hex),
            current_reward_node_count,
        });
    }

    let mut scalars = Vec::new();
    for (field, name) in [
        (4, "total_eligible_votes"),
        (5, "total_staked"),
        (6, "minted_tokens"),
        (7, "total_supply"),
        (8, "yield"),
    ] {
        scalars.push(analyzer.scalar(field, name)?);
    }

    let seeded_delegations =
        analyze_seeded_delegations(&mut analyzer, seeded_addresses, MAX_SEEDED_DELEGATIONS)?;

    analyzer.finish(validator_count, validators, scalars, seeded_delegations)
}

fn analyze_seeded_delegations<F>(
    analyzer: &mut Analyzer<'_, F>,
    seeded_addresses: BTreeMap<[u8; 20], BTreeSet<&'static str>>,
    max_entries: u32,
) -> Result<SeededDelegationCoverage>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    let candidate_count = u32::try_from(seeded_addresses.len())?;
    let mut total_entries = 0_u32;
    let mut count_results = BTreeMap::<String, u64>::new();
    let mut candidates = Vec::with_capacity(seeded_addresses.len());
    let mut delegations = Vec::new();

    for (delegator, sources) in seeded_addresses {
        let prefix = delegator_validators_prefix(delegator);
        let count_probe = analyzer.physical_probe(
            "seeded_delegation_count",
            iterable_count_key_for(&prefix),
            false,
        )?;
        *count_results
            .entry(count_probe.result.to_owned())
            .or_default() += 1;
        let delegation_count = if count_probe.result == "live_present" {
            let value = count_probe
                .value
                .as_deref()
                .context("live delegation count has no value")?;
            let count = decode_le_u32(value, "seeded delegation count")?;
            total_entries = total_entries
                .checked_add(count)
                .context("seeded delegation count overflow")?;
            ensure!(
                total_entries <= max_entries,
                "seeded delegation entries {total_entries} exceed diagnostic limit {max_entries}"
            );
            Some(count)
        } else {
            None
        };

        candidates.push(SeededDelegatorFact {
            address_hex: hex::encode(delegator),
            sources: sources.into_iter().collect(),
            delegation_count_result: count_probe.result,
            delegation_count,
        });

        let Some(delegation_count) = delegation_count else {
            continue;
        };
        let mut seen_validators = BTreeSet::new();
        for position in 1..=delegation_count {
            let validator = exact_bytes::<20>(
                analyzer.required(
                    "seeded_delegation_item",
                    iterable_item_key_for(&prefix, position),
                )?,
                "seeded delegation validator",
            )?;
            ensure!(
                seen_validators.insert(validator),
                "duplicate validator in seeded delegation iterable"
            );
            let reverse = analyzer.required(
                "seeded_delegation_reverse",
                iterable_position_key_for(&prefix, &validator),
            )?;
            ensure!(
                decode_le_u32(&reverse, "seeded delegation reverse position")? == position,
                "seeded delegation reverse index disagrees with forward position"
            );
            let row = decode_delegation(&analyzer.required(
                "seeded_delegation_record",
                storage_key(&[&[2, 0], &validator, &delegator]),
            )?)?;
            let reward_probe = analyzer.physical_probe(
                "seeded_delegation_reward_node",
                storage_key(&[&[1], &validator, &compact_u64_bytes(row.last_updated)]),
                true,
            )?;
            let (reward_per_stake, reward_node_count) = match reward_probe.result {
                "live_present" | "live_present_already_matched" => {
                    let (reward_per_stake, count) = decode_reward_node(
                        reward_probe
                            .value
                            .as_deref()
                            .context("live delegation reward node has no value")?,
                    )?;
                    (Some(uint_hex(&reward_per_stake)), Some(count))
                }
                _ => (None, None),
            };
            delegations.push(SeededDelegationFact {
                delegator_hex: hex::encode(delegator),
                validator_hex: hex::encode(validator),
                position,
                stake_hex: uint_hex(&row.stake),
                last_updated: row.last_updated,
                reward_node_result: reward_probe.result,
                reward_per_stake_hex: reward_per_stake,
                reward_node_count,
            });
        }
    }

    Ok(SeededDelegationCoverage {
        candidate_count,
        max_delegation_entries: max_entries,
        live_delegation_entries: total_entries,
        count_results,
        candidates,
        delegations,
        global_delegator_set_complete: false,
    })
}

/// Maximum justified addresses admitted by the transfer-window diagnostic.
pub const MAX_TRANSFER_ADDRESSES: usize = 20;
/// Maximum aggregate delegation entries admitted before child expansion.
pub const MAX_TRANSFER_DELEGATIONS: u32 = 256;
/// Point calls, including repeated references, permitted by the transfer bounds.
pub const MAX_TRANSFER_READS: usize =
    MAX_TRANSFER_ADDRESSES + 4 * MAX_TRANSFER_DELEGATIONS as usize;
/// Aggregate accepted point-value bytes; not a RocksDB allocation bound.
pub const MAX_TRANSFER_VALUE_BYTES: usize = 1024 * 1024;

/// Independent partial delegation catalog for caller-justified addresses.
///
/// Matched paths are unique and may overlap other inverse catalogs. Never add
/// independent counts without checking their path union. Unexplained paths have
/// no semantic interpretation; this result grants no snapshot/import authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AddressDelegationCatalog {
    pub seeded_delegations: SeededDelegationCoverage,
    pub derived_reads: usize,
    pub accepted_point_value_bytes: usize,
    pub matched_paths_hex: Vec<String>,
    pub references: Vec<DelegationCatalogReference>,
    pub matched_live_entries: usize,
    pub matched_live_value_bytes: u64,
    pub unexplained_live_entries: usize,
    pub unexplained_live_value_bytes: u64,
    pub unexplained_entries_sha256: String,
    pub live_partition_exact: bool,
    pub semantic_snapshot_complete: bool,
}

/// A justified key reference; repeated cursor references remain separate.
/// Join logical keys to raw observations rather than inferring physical values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DelegationCatalogReference {
    pub delegator_hex: String,
    pub sources: Vec<&'static str>,
    pub position: Option<u32>,
    pub family: &'static str,
    pub logical_key_hex: String,
    pub trie_path_hex: String,
}

/// Decodes only delegations reached from at most twenty justified addresses.
///
/// The caller binds the complete DPoS inventory and raw reader to one identity
/// and supplies provenance for every seed. Live rows must match inventory bytes;
/// required indexed rows fail on missing, orphan, malformed or unavailable data.
/// Optional counts/cursors retain physical distinctions and never invent zero.
/// All source positions are preserved. Aggregate entry, point-call and accepted
/// value-byte bounds fail closed, with no partial successful catalog returned.
pub fn analyze_address_delegations<F>(
    inventory: &ConcreteStorageInventory,
    seeds: BTreeMap<[u8; 20], BTreeSet<&'static str>>,
    mut read: F,
) -> Result<AddressDelegationCatalog>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    ensure!(
        inventory.address == DPOS_CONTRACT_ADDRESS,
        "catalog inventory is not DPoS"
    );
    ensure!(
        seeds.len() <= MAX_TRANSFER_ADDRESSES,
        "transfer seed count exceeds limit"
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
    let mut derived_reads = 0_usize;
    let mut point_bytes = 0_usize;
    let mut analyzer = Analyzer {
        inventory: indexed,
        read: |key| {
            if derived_reads >= MAX_TRANSFER_READS {
                return Err(ConcreteReadError::Corrupt(
                    "transfer point-read limit exceeded".into(),
                ));
            }
            derived_reads += 1;
            let result = read(key);
            if let Ok(ConcreteRead::Present(value)) = &result {
                let Some(bytes) = point_bytes.checked_add(value.len()) else {
                    return Err(ConcreteReadError::Corrupt(
                        "transfer value-byte overflow".into(),
                    ));
                };
                if bytes > MAX_TRANSFER_VALUE_BYTES {
                    return Err(ConcreteReadError::Corrupt(
                        "transfer value-byte limit exceeded".into(),
                    ));
                }
                point_bytes = bytes;
            }
            result
        },
        known: BTreeMap::new(),
        matched_by_family: BTreeMap::new(),
        matched_value_bytes: 0,
    };
    let seeded_delegations =
        analyze_seeded_delegations(&mut analyzer, seeds, MAX_TRANSFER_DELEGATIONS)?;
    let unexplained = analyzer
        .inventory
        .iter()
        .filter(|(path, _)| !analyzer.known.contains_key(*path))
        .map(|(path, value)| ConcreteStorageInventoryEntry {
            hashed_path: *path,
            value: value.to_vec(),
        })
        .collect::<Vec<_>>();
    let unexplained_bytes = unexplained.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(u64::try_from(entry.value.len())?)
            .context("catalog byte-count overflow")
    })?;
    let mut references = Vec::new();
    for candidate in &seeded_delegations.candidates {
        let delegator: [u8; 20] = hex::decode(&candidate.address_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("decoded candidate width differs"))?;
        let prefix = delegator_validators_prefix(delegator);
        let mut push = |position, family, key: ConcreteStorageKey| {
            references.push(DelegationCatalogReference {
                delegator_hex: candidate.address_hex.clone(),
                sources: candidate.sources.clone(),
                position,
                family,
                logical_key_hex: hex::encode(key.0),
                trie_path_hex: hex::encode(keccak256(&key.0)),
            });
        };
        push(None, "delegation_count", iterable_count_key_for(&prefix));
        for delegation in seeded_delegations
            .delegations
            .iter()
            .filter(|row| row.delegator_hex == candidate.address_hex)
        {
            let validator: [u8; 20] = hex::decode(&delegation.validator_hex)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("decoded validator width differs"))?;
            let position = Some(delegation.position);
            push(
                position,
                "delegation_item",
                iterable_item_key_for(&prefix, delegation.position),
            );
            push(
                position,
                "delegation_reverse",
                iterable_position_key_for(&prefix, &validator),
            );
            push(
                position,
                "delegation_principal",
                storage_key(&[&[2, 0], &validator, &delegator]),
            );
            push(
                position,
                "delegation_cursor",
                storage_key(&[
                    &[1],
                    &validator,
                    &compact_u64_bytes(delegation.last_updated),
                ]),
            );
        }
    }
    let matched_paths = analyzer.known.keys().map(hex::encode).collect::<Vec<_>>();
    let matched_bytes = analyzer.matched_value_bytes;
    ensure!(
        matched_paths.len() + unexplained.len() == inventory.entries.len(),
        "catalog row partition differs"
    );
    ensure!(
        matched_bytes.checked_add(unexplained_bytes) == Some(inventory.value_bytes),
        "catalog byte partition differs"
    );
    drop(analyzer);
    Ok(AddressDelegationCatalog {
        seeded_delegations,
        derived_reads,
        accepted_point_value_bytes: point_bytes,
        matched_live_entries: matched_paths.len(),
        matched_paths_hex: matched_paths,
        references,
        matched_live_value_bytes: matched_bytes,
        unexplained_live_entries: unexplained.len(),
        unexplained_live_value_bytes: unexplained_bytes,
        unexplained_entries_sha256: digest_entries(&unexplained),
        live_partition_exact: true,
        semantic_snapshot_complete: false,
    })
}

/// Deterministic SHA-256 over sorted inventory path/value entries.
pub fn digest_entries(entries: &[ConcreteStorageInventoryEntry]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.hashed_path);
        digest.update((entry.value.len() as u64).to_be_bytes());
        digest.update(&entry.value);
    }
    hex::encode(digest.finalize())
}

fn digest_paths(entries: &[ConcreteStorageInventoryEntry]) -> String {
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.hashed_path);
    }
    hex::encode(digest.finalize())
}

/// Pure global validator-count key for offline authenticated seed-link audits.
/// Output is the logical key K(00 05 01), before storage-trie hashing.
pub fn validator_seed_count_key() -> ConcreteStorageKey {
    iterable_count_key()
}
/// Pure retained forward key K(00 05 02 || position_u32_le).
/// Callers must enforce their own index bounds; this helper never reads state.
pub fn validator_seed_item_key(position: u32) -> ConcreteStorageKey {
    iterable_item_key(position)
}
/// Pure retained reverse key K(00 05 02 || validator_address20).
pub fn validator_seed_reverse_key(address: [u8; 20]) -> ConcreteStorageKey {
    iterable_position_key(address)
}
/// Pure validator-owner key K(00 03 || validator_address20), with no state access.
pub fn validator_seed_owner_key(address: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[0, 3], &address])
}

fn iterable_count_key() -> ConcreteStorageKey {
    iterable_count_key_for(VALIDATORS_PREFIX)
}

fn iterable_item_key(position: u32) -> ConcreteStorageKey {
    // The pinned Go constructors reuse spare prefix capacity. Initializing the
    // reverse key overwrites the item discriminator zero with two, so actual
    // retained rows use discriminator two for both directions.
    iterable_item_key_for(VALIDATORS_PREFIX, position)
}

fn iterable_position_key(address: [u8; 20]) -> ConcreteStorageKey {
    iterable_position_key_for(VALIDATORS_PREFIX, &address)
}

fn iterable_count_key_for(prefix: &[u8]) -> ConcreteStorageKey {
    storage_key(&[prefix, &[1]])
}

fn iterable_item_key_for(prefix: &[u8], position: u32) -> ConcreteStorageKey {
    // Every pinned AddressesIMap constructor aliases the spare discriminator
    // byte while initializing the reverse prefix, as described above.
    storage_key(&[prefix, &[2], &position.to_le_bytes()])
}

fn iterable_position_key_for(prefix: &[u8], item: &[u8]) -> ConcreteStorageKey {
    storage_key(&[prefix, &[2], item])
}

fn delegator_validators_prefix(delegator: [u8; 20]) -> Vec<u8> {
    [&[2, 1][..], &delegator].concat()
}

/// Derives the observed Go delegation iterable count's logical key only.
/// Input is one justified address; output is Keccak([2,1] || address || [1]).
/// This pure helper grants no membership, count, history or expansion authority.
pub fn delegation_count_key(address: [u8; 20]) -> ConcreteStorageKey {
    iterable_count_key_for(&delegator_validators_prefix(address))
}

/// Derive the V1 validator-map count slot for one justified address.
/// Uses Go's [03,01] address prefix and count suffix [01]; no state access.
pub fn undelegation_v1_count_key(address: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[3, 1], &address, &[1]])
}
/// Derive the V2 validator-map count slot with prefix [03,02] and suffix [01].
pub fn undelegation_v2_count_key(address: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[3, 2], &address, &[1]])
}
/// Derive the V2 last allocated ID slot with prefix [03,04], without a suffix.
/// This slot is a cursor; its raw value is not a count or a membership proof.
pub fn undelegation_v2_last_id_key(address: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[3, 4], &address])
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

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    storage_key(&[bytes]).0
}

fn decode_validator(bytes: &[u8]) -> Result<ValidatorRow> {
    let outer = exact_rlp(bytes, "validator")?;
    let count = outer.item_count().context("validator item count")?;
    let (row, encoding, undelegations_count, embedded_encoding) = match count {
        4 => (outer, "validator_v1", None, None),
        2 if outer.at(0)?.is_list() => {
            let embedded = outer.at(0)?;
            ensure!(
                embedded.item_count()? == 4,
                "embedded ValidatorV1 must have four fields"
            );
            let count_item = outer.at(1)?;
            require_data(&count_item, "validator undelegations count")?;
            (
                embedded,
                "embedded_validator_v1_with_count",
                Some(
                    count_item
                        .as_val::<u16>()
                        .context("validator undelegations count")?,
                ),
                Some(outer.at(0)?.as_raw().to_vec()),
            )
        }
        _ => bail!("validator must be ValidatorV1 or embedded extended form"),
    };
    let stake = decode_uint_item(&row.at(0)?, "validator stake")?;
    let commission_item = row.at(1)?;
    require_data(&commission_item, "validator commission")?;
    let commission = commission_item.as_val().context("validator commission")?;
    let commission_change_item = row.at(2)?;
    require_data(&commission_change_item, "validator last commission change")?;
    let last_commission_change = commission_change_item
        .as_val()
        .context("validator last commission change")?;
    let reward_head_item = row.at(3)?;
    require_data(&reward_head_item, "validator reward head")?;
    let reward_head = reward_head_item.as_val().context("validator reward head")?;

    let mut validator_v1 = RlpStream::new_list(4);
    append_uint(&mut validator_v1, &stake);
    validator_v1
        .append(&commission)
        .append(&last_commission_change)
        .append(&reward_head);
    let validator_v1 = validator_v1.out();
    let canonical = if let Some(embedded_encoding) = embedded_encoding {
        ensure!(
            embedded_encoding == validator_v1.as_ref(),
            "embedded ValidatorV1 is not canonical or completely consumed"
        );
        let mut extended = RlpStream::new_list(2);
        extended.append_raw(&validator_v1, 1);
        extended.append(&undelegations_count.expect("extended count is present"));
        extended.out()
    } else {
        validator_v1
    };
    ensure!(
        canonical.as_ref() == bytes,
        "validator is not canonical or completely consumed"
    );
    Ok(ValidatorRow {
        encoding,
        stake,
        commission,
        last_commission_change,
        reward_head,
        undelegations_count,
    })
}

fn decode_delegation(bytes: &[u8]) -> Result<DelegationRow> {
    let row = exact_rlp(bytes, "delegation")?;
    ensure!(row.item_count()? == 2, "delegation must have two fields");
    let stake = decode_uint_item(&row.at(0)?, "delegation stake")?;
    let last_updated_item = row.at(1)?;
    require_data(&last_updated_item, "delegation last updated")?;
    let last_updated = last_updated_item
        .as_val()
        .context("delegation last updated")?;
    let mut canonical = RlpStream::new_list(2);
    append_uint(&mut canonical, &stake);
    canonical.append(&last_updated);
    ensure!(
        canonical.out().as_ref() == bytes,
        "delegation is not canonical or completely consumed"
    );
    Ok(DelegationRow {
        stake,
        last_updated,
    })
}

fn decode_pair_bytes(bytes: &[u8], label: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let row = exact_rlp(bytes, label)?;
    ensure!(row.item_count()? == 2, "{label} must have two fields");
    let first = row.at(0)?;
    let second = row.at(1)?;
    let first = require_data(&first, label)?.to_vec();
    let second = require_data(&second, label)?.to_vec();
    let mut canonical = RlpStream::new_list(2);
    canonical
        .append(&first.as_slice())
        .append(&second.as_slice());
    ensure!(
        canonical.out().as_ref() == bytes,
        "{label} is not canonical or completely consumed"
    );
    Ok((first, second))
}

fn decode_pair_uints(bytes: &[u8], label: &str) -> Result<(BigUint, BigUint)> {
    let row = exact_rlp(bytes, label)?;
    ensure!(row.item_count()? == 2, "{label} must have two fields");
    let first = decode_uint_item(&row.at(0)?, label)?;
    let second = decode_uint_item(&row.at(1)?, label)?;
    let mut canonical = RlpStream::new_list(2);
    append_uint(&mut canonical, &first);
    append_uint(&mut canonical, &second);
    ensure!(
        canonical.out().as_ref() == bytes,
        "{label} is not canonical or completely consumed"
    );
    Ok((first, second))
}

fn decode_reward_node(bytes: &[u8]) -> Result<(BigUint, u32)> {
    let row = exact_rlp(bytes, "reward node")?;
    ensure!(row.item_count()? == 2, "reward node must have two fields");
    let reward_per_stake = decode_uint_item(&row.at(0)?, "reward per stake")?;
    let count_item = row.at(1)?;
    require_data(&count_item, "reward node count")?;
    let count = count_item.as_val().context("reward node count")?;
    let mut canonical = RlpStream::new_list(2);
    append_uint(&mut canonical, &reward_per_stake);
    canonical.append(&count);
    ensure!(
        canonical.out().as_ref() == bytes,
        "reward node is not canonical or completely consumed"
    );
    Ok((reward_per_stake, count))
}

fn exact_rlp<'a>(bytes: &'a [u8], label: &str) -> Result<Rlp<'a>> {
    let rlp = Rlp::new(bytes);
    let payload = rlp.payload_info().with_context(|| format!("{label} RLP"))?;
    let total = payload
        .header_len
        .checked_add(payload.value_len)
        .with_context(|| format!("{label} RLP length overflow"))?;
    ensure!(total == bytes.len(), "{label} has trailing bytes");
    Ok(rlp)
}

fn decode_uint_item(item: &Rlp<'_>, label: &str) -> Result<BigUint> {
    let bytes = require_data(item, label)?;
    canonical_uint(bytes, label)?;
    Ok(BigUint::from_bytes_be(bytes))
}

fn require_data<'a>(item: &'a Rlp<'_>, label: &str) -> Result<&'a [u8]> {
    ensure!(!item.is_list(), "{label} must be RLP data");
    item.data().with_context(|| format!("{label} bytes"))
}

fn append_uint(stream: &mut RlpStream, value: &BigUint) {
    let bytes = canonical_uint_bytes(value);
    stream.append(&bytes.as_slice());
}

fn canonical_uint_bytes(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    if bytes == [0] { Vec::new() } else { bytes }
}

fn canonical_uint(bytes: &[u8], label: &str) -> Result<()> {
    ensure!(
        bytes.first() != Some(&0),
        "{label} has a leading-zero unsigned integer"
    );
    Ok(())
}

fn decode_rlp_u64(bytes: &[u8], label: &str) -> Result<u64> {
    let item = exact_rlp(bytes, label)?;
    require_data(&item, label)?;
    let value = item
        .as_val()
        .with_context(|| format!("{label} unsigned integer"))?;
    ensure!(
        rlp::encode(&value).as_ref() == bytes,
        "{label} is not canonical"
    );
    Ok(value)
}

fn decode_compact_u64(bytes: &[u8], label: &str) -> Result<u64> {
    canonical_uint(bytes, label)?;
    ensure!(bytes.len() <= 8, "{label} exceeds uint64");
    let mut word = [0; 8];
    word[8 - bytes.len()..].copy_from_slice(bytes);
    Ok(u64::from_be_bytes(word))
}

fn decode_le_u32(bytes: &[u8], label: &str) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes.try_into().with_context(|| {
        format!("{label} must contain four little-endian bytes")
    })?))
}

fn exact_bytes<const N: usize>(value: Vec<u8>, label: &str) -> Result<[u8; N]> {
    value
        .try_into()
        .map_err(|value: Vec<u8>| anyhow::anyhow!("{label} has {} bytes", value.len()))
}

fn compact_u64_bytes(value: u64) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }
    let bytes = value.to_be_bytes();
    bytes[bytes.iter().position(|byte| *byte != 0).unwrap()..].to_vec()
}

fn uint_hex(value: &BigUint) -> String {
    hex::encode(canonical_uint_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_types::FinalChainBlockNumber;
    use rustaxa_types::concrete_state::ConcreteStateIdentity;

    const CANDIDATE: [u8; 20] = [
        0xfb, 0xb8, 0x5d, 0x00, 0xca, 0x77, 0xb0, 0xd4, 0x9d, 0xa4, 0xf7, 0x1a, 0x91, 0xde, 0x55,
        0x2b, 0xce, 0x88, 0xb0, 0x83,
    ];

    #[test]
    fn observed_iterable_alias_keys_and_candidate_validator_decode_are_exact() {
        assert_eq!(
            hex::encode(iterable_item_key(128).0),
            "634cff04eb2d3d263206e30d949eef029317db184545f623c4ae5465078992f2"
        );
        assert_eq!(
            hex::encode(iterable_position_key(CANDIDATE).0),
            "5a48c40f7a0db86e77facc73a338b7d303d0ca37bc9ab0076e0b1c7f2e0ec6be"
        );
        let row = hex::decode("d8d6893635c9adc5dea000008201f483e30e5684014dadb580").unwrap();
        let decoded = decode_validator(&row).unwrap();
        assert_eq!(decoded.encoding, "embedded_validator_v1_with_count");
        assert_eq!(decoded.stake.to_str_radix(10), "1000000000000000000000");
        assert_eq!(decoded.commission, 500);
        assert_eq!(decoded.last_commission_change, 14_880_342);
        assert_eq!(decoded.reward_head, 21_867_957);
        assert_eq!(decoded.undelegations_count, Some(0));
    }

    #[test]
    fn yield_is_rlp_u64_and_metadata_remains_arbitrary_bytes() {
        assert_eq!(
            decode_rlp_u64(&[0x83, 0x01, 0x6d, 0xb8], "yield").unwrap(),
            93_624
        );
        assert!(decode_rlp_u64(&[0x01, 0x6d, 0xb8], "yield").is_err());

        let mut info = rlp::RlpStream::new_list(2);
        info.append(&[0xff, 0xfe].as_slice());
        info.append(&[0x80].as_slice());
        assert_eq!(
            decode_pair_bytes(&info.out(), "validator info").unwrap(),
            (vec![0xff, 0xfe], vec![0x80])
        );
    }

    #[test]
    fn flat_five_field_validator_and_noncanonical_uint_are_rejected() {
        let mut flat = rlp::RlpStream::new_list(5);
        flat.append(&1_u64)
            .append(&2_u16)
            .append(&3_u64)
            .append(&4_u64)
            .append(&0_u16);
        assert!(decode_validator(&flat.out()).is_err());

        let noncanonical = Rlp::new(&[0x82, 0, 1]);
        assert!(decode_uint_item(&noncanonical, "noncanonical").is_err());
        assert!(decode_uint_item(&Rlp::new(&[0xc0]), "list integer").is_err());
    }

    #[test]
    fn strict_decoders_reject_list_fields_and_unconsumed_children() {
        assert!(decode_pair_bytes(&[0xc2, 0xc0, 0x80], "list metadata").is_err());
        assert!(decode_pair_bytes(&[0xc3, 0x01, 0x02, 0xb8], "truncated pair").is_err());

        let mut list_uint = RlpStream::new_list(2);
        list_uint.begin_list(0).append(&1_u64);
        assert!(decode_pair_uints(&list_uint.out(), "list uint").is_err());
    }

    #[test]
    fn zero_big_uint_fields_reencode_as_empty_rlp_data() {
        let zero_pair = decode_pair_uints(&[0xc2, 0x80, 0x80], "zero pair").unwrap();
        assert_eq!(zero_pair, (BigUint::default(), BigUint::default()));

        let zero_node = decode_reward_node(&[0xc2, 0x80, 0x80]).unwrap();
        assert_eq!(zero_node, (BigUint::default(), 0));

        let mut validator = RlpStream::new_list(4);
        validator
            .append_empty_data()
            .append(&0_u16)
            .append(&0_u64)
            .append(&1_u64);
        let decoded = decode_validator(&validator.out()).unwrap();
        assert_eq!(decoded.stake, BigUint::default());
        assert_eq!(uint_hex(&decoded.stake), "");
    }

    #[test]
    fn synthetic_enumeration_partitions_matched_and_unexplained_live_paths() {
        let validator = [0x31; 20];
        let owner = [0x32; 20];
        let vrf = [0x33; 32];
        let mut rows = BTreeMap::new();
        rows.insert(
            iterable_count_key(),
            ConcreteRead::Present(1_u32.to_le_bytes().to_vec()),
        );
        rows.insert(
            iterable_item_key(1),
            ConcreteRead::Present(validator.to_vec()),
        );
        rows.insert(
            iterable_position_key(validator),
            ConcreteRead::Present(1_u32.to_le_bytes().to_vec()),
        );

        let mut validator_row = rlp::RlpStream::new_list(4);
        validator_row
            .append(&10_u64)
            .append(&20_u16)
            .append(&30_u64)
            .append(&40_u64);
        rows.insert(
            storage_key(&[&[0, 0], &validator]),
            ConcreteRead::Present(validator_row.out().to_vec()),
        );
        let mut info = rlp::RlpStream::new_list(2);
        info.append(&b"description".as_slice())
            .append(&b"endpoint".as_slice());
        rows.insert(
            storage_key(&[&[0, 1], &validator]),
            ConcreteRead::Present(info.out().to_vec()),
        );
        let mut rewards = rlp::RlpStream::new_list(2);
        rewards.append(&11_u64).append(&12_u64);
        rows.insert(
            storage_key(&[&[0, 2], &validator]),
            ConcreteRead::Present(rewards.out().to_vec()),
        );
        rows.insert(
            storage_key(&[&[0, 3], &validator]),
            ConcreteRead::Present(owner.to_vec()),
        );
        rows.insert(
            storage_key(&[&[0, 4], &validator]),
            ConcreteRead::Present(vrf.to_vec()),
        );
        let mut node = rlp::RlpStream::new_list(2);
        node.append(&[0x44].as_slice()).append(&2_u32);
        let graph_key = storage_key(&[&[1], &validator, &compact_u64_bytes(40)]);
        rows.insert(graph_key, ConcreteRead::Present(node.out().to_vec()));
        rows.insert(storage_key(&[&[4]]), ConcreteRead::Present(vec![1]));
        rows.insert(storage_key(&[&[5]]), ConcreteRead::Present(vec![2]));
        rows.insert(storage_key(&[&[6]]), ConcreteRead::Tombstone);
        rows.insert(storage_key(&[&[7]]), ConcreteRead::Present(vec![3]));
        rows.insert(
            storage_key(&[&[8]]),
            ConcreteRead::Present(rlp::encode(&4_u64).to_vec()),
        );
        let delegation_prefix = delegator_validators_prefix(validator);
        rows.insert(
            iterable_count_key_for(&delegation_prefix),
            ConcreteRead::Present(1_u32.to_le_bytes().to_vec()),
        );
        rows.insert(
            iterable_item_key_for(&delegation_prefix, 1),
            ConcreteRead::Present(validator.to_vec()),
        );
        rows.insert(
            iterable_position_key_for(&delegation_prefix, &validator),
            ConcreteRead::Present(1_u32.to_le_bytes().to_vec()),
        );
        let mut delegation = RlpStream::new_list(2);
        delegation.append(&50_u64).append(&30_u64);
        rows.insert(
            storage_key(&[&[2, 0], &validator, &validator]),
            ConcreteRead::Present(delegation.out().to_vec()),
        );
        let mut delegation_reward = RlpStream::new_list(2);
        delegation_reward.append(&[0x22].as_slice()).append(&1_u32);
        let delegation_reward_key = storage_key(&[&[1], &validator, &compact_u64_bytes(30)]);
        rows.insert(
            delegation_reward_key,
            ConcreteRead::Present(delegation_reward.out().to_vec()),
        );

        let mut entries = rows
            .iter()
            .filter_map(|(key, value)| match value {
                ConcreteRead::Present(value) => Some(ConcreteStorageInventoryEntry {
                    hashed_path: keccak256(&key.0),
                    value: value.clone(),
                }),
                ConcreteRead::Absent | ConcreteRead::Tombstone => None,
            })
            .collect::<Vec<_>>();
        entries.push(ConcreteStorageInventoryEntry {
            hashed_path: [0xaa; 32],
            value: vec![0xbb],
        });
        entries.sort_by_key(|entry| entry.hashed_path);
        let inventory = ConcreteStorageInventory {
            identity: ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(7),
                state_root: [7; 32],
            },
            address: DPOS_CONTRACT_ADDRESS,
            storage_root: Some([8; 32]),
            nodes_visited: 1,
            value_bytes: entries.iter().map(|entry| entry.value.len() as u64).sum(),
            entries,
        };

        let coverage = analyze_native_head(&inventory, |key| {
            Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
        })
        .unwrap();
        assert_eq!(coverage.validator_count, 1);
        assert_eq!(coverage.matched_live_entries, 18);
        assert_eq!(coverage.unexplained_live_entries, 1);
        assert!(coverage.live_partition_exact);
        assert!(!coverage.semantic_snapshot_complete);
        assert_eq!(coverage.seeded_delegations.candidate_count, 2);
        assert_eq!(coverage.seeded_delegations.live_delegation_entries, 1);
        assert_eq!(coverage.seeded_delegations.delegations.len(), 1);
        assert_eq!(coverage.seeded_delegations.delegations[0].stake_hex, "32");
        assert_eq!(
            coverage.seeded_delegations.count_results,
            BTreeMap::from([("absent".to_owned(), 1), ("live_present".to_owned(), 1)])
        );

        let mut inventory_without_delegation_reward = inventory.clone();
        inventory_without_delegation_reward
            .entries
            .retain(|entry| entry.hashed_path != keccak256(&delegation_reward_key.0));
        let orphan_reward = analyze_native_head(&inventory_without_delegation_reward, |key| {
            Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
        })
        .unwrap();
        assert_eq!(
            orphan_reward.seeded_delegations.delegations[0].reward_node_result,
            "orphan_present"
        );
        assert_eq!(
            orphan_reward.seeded_delegations.delegations[0].reward_per_stake_hex,
            None
        );

        let owner_count_key = iterable_count_key_for(&delegator_validators_prefix(owner));
        rows.insert(
            owner_count_key,
            ConcreteRead::Present(0_u32.to_le_bytes().to_vec()),
        );
        let orphan_count = analyze_native_head(&inventory, |key| {
            Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
        })
        .unwrap();
        assert_eq!(
            orphan_count.seeded_delegations.count_results,
            BTreeMap::from([
                ("live_present".to_owned(), 1),
                ("orphan_present".to_owned(), 1),
            ])
        );

        let mut inventory_without_graph = inventory.clone();
        inventory_without_graph
            .entries
            .retain(|entry| entry.hashed_path != keccak256(&graph_key.0));
        let unavailable = analyze_native_head(&inventory_without_graph, |key| {
            if key == graph_key {
                Err(ConcreteReadError::HistoryUnavailable(
                    inventory_without_graph.identity,
                ))
            } else {
                Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
            }
        })
        .unwrap();
        assert_eq!(
            unavailable.validators[0].current_reward_node_result,
            "history_unavailable"
        );
        assert_eq!(unavailable.validators[0].current_reward_per_stake_hex, None);

        let scalar_key = storage_key(&[&[6]]);
        let mut contradictory_inventory = inventory.clone();
        contradictory_inventory
            .entries
            .push(ConcreteStorageInventoryEntry {
                hashed_path: keccak256(&scalar_key.0),
                value: vec![9],
            });
        contradictory_inventory
            .entries
            .sort_by_key(|entry| entry.hashed_path);
        assert!(
            analyze_native_head(&contradictory_inventory, |key| {
                Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
            })
            .unwrap_err()
            .to_string()
            .contains("but its path is live")
        );

        let delegation_count_key = iterable_count_key_for(&delegation_prefix);
        let over_limit = (MAX_SEEDED_DELEGATIONS + 1).to_le_bytes().to_vec();
        rows.insert(
            delegation_count_key,
            ConcreteRead::Present(over_limit.clone()),
        );
        let mut over_limit_inventory = inventory.clone();
        over_limit_inventory
            .entries
            .iter_mut()
            .find(|entry| entry.hashed_path == keccak256(&delegation_count_key.0))
            .unwrap()
            .value = over_limit;
        assert!(
            analyze_native_head(&over_limit_inventory, |key| {
                Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
            })
            .unwrap_err()
            .to_string()
            .contains("exceed diagnostic limit")
        );

        let reverse_key = iterable_position_key(validator);
        let invalid_reverse = 2_u32.to_le_bytes().to_vec();
        rows.insert(reverse_key, ConcreteRead::Present(invalid_reverse.clone()));
        let mut invalid_inventory = inventory.clone();
        invalid_inventory
            .entries
            .iter_mut()
            .find(|entry| entry.hashed_path == keccak256(&reverse_key.0))
            .unwrap()
            .value = invalid_reverse;
        assert!(
            analyze_native_head(&invalid_inventory, |key| {
                Ok(rows.get(&key).cloned().unwrap_or(ConcreteRead::Absent))
            })
            .unwrap_err()
            .to_string()
            .contains("reverse index")
        );
    }
}

#[cfg(test)]
mod address_catalog_tests {
    use super::*;
    use rustaxa_types::{FinalChainBlockNumber, concrete_state::ConcreteStateIdentity};
    fn seeds(addresses: &[[u8; 20]]) -> BTreeMap<[u8; 20], BTreeSet<&'static str>> {
        addresses
            .iter()
            .map(|a| (*a, BTreeSet::from(["test_evidence"])))
            .collect()
    }
    fn inventory(
        rows: &BTreeMap<ConcreteStorageKey, ConcreteRead<Vec<u8>>>,
    ) -> ConcreteStorageInventory {
        let mut entries = rows
            .iter()
            .filter_map(|(key, row)| match row {
                ConcreteRead::Present(value) => Some(ConcreteStorageInventoryEntry {
                    hashed_path: keccak256(&key.0),
                    value: value.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|row| row.hashed_path);
        ConcreteStorageInventory {
            identity: ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(7),
                state_root: [7; 32],
            },
            address: DPOS_CONTRACT_ADDRESS,
            storage_root: Some([8; 32]),
            nodes_visited: 1,
            value_bytes: entries.iter().map(|row| row.value.len() as u64).sum(),
            entries,
        }
    }
    fn rows() -> BTreeMap<ConcreteStorageKey, ConcreteRead<Vec<u8>>> {
        let mut rows = BTreeMap::new();
        for delegator in [[1; 20], [2; 20]] {
            let prefix = delegator_validators_prefix(delegator);
            rows.insert(
                iterable_count_key_for(&prefix),
                ConcreteRead::Present(2_u32.to_le_bytes().to_vec()),
            );
            // Preserve insertion order rather than sorting validators.
            for (position, validator) in [(1, [4; 20]), (2, [3; 20])] {
                rows.insert(
                    iterable_item_key_for(&prefix, position),
                    ConcreteRead::Present(validator.to_vec()),
                );
                rows.insert(
                    iterable_position_key_for(&prefix, &validator),
                    ConcreteRead::Present(position.to_le_bytes().to_vec()),
                );
                let mut row = RlpStream::new_list(2);
                row.append(&50_u64).append(&30_u64);
                rows.insert(
                    storage_key(&[&[2, 0], &validator, &delegator]),
                    ConcreteRead::Present(row.out().to_vec()),
                );
                let mut node = RlpStream::new_list(2);
                node.append(&10_u64).append(&2_u32);
                rows.insert(
                    storage_key(&[&[1], &validator, &compact_u64_bytes(30)]),
                    ConcreteRead::Present(node.out().to_vec()),
                );
            }
        }
        rows
    }
    #[test]
    fn catalog_preserves_order_principal_shared_cursors_and_exact_partition() {
        let rows = rows();
        let inventory = inventory(&rows);
        let catalog = analyze_address_delegations(&inventory, seeds(&[[1; 20], [2; 20]]), |key| {
            Ok(rows[&key].clone())
        })
        .unwrap();
        assert_eq!(catalog.derived_reads, 18);
        assert_eq!(catalog.references.len(), 18);
        assert_eq!(catalog.matched_live_entries, 16);
        assert_eq!(catalog.unexplained_live_entries, 0);
        assert_eq!(
            catalog.seeded_delegations.delegations[0].validator_hex,
            hex::encode([4; 20])
        );
        assert_eq!(
            catalog.seeded_delegations.delegations[1].validator_hex,
            hex::encode([3; 20])
        );
        assert_eq!(catalog.seeded_delegations.delegations[0].stake_hex, "32");
        assert_eq!(
            catalog.seeded_delegations.delegations[2].reward_node_result,
            "live_present_already_matched"
        );
        assert!(!catalog.semantic_snapshot_complete);
        assert!(!catalog.seeded_delegations.global_delegator_set_complete);
    }
    #[test]
    fn catalog_retains_non_live_counts_and_explicit_zero() {
        let candidate = [1; 20];
        let key = iterable_count_key_for(&delegator_validators_prefix(candidate));
        let empty = inventory(&BTreeMap::new());
        for (read, expected) in [
            (ConcreteRead::Absent, "absent"),
            (ConcreteRead::Tombstone, "tombstone"),
            (
                ConcreteRead::Present(0_u32.to_le_bytes().to_vec()),
                "orphan_present",
            ),
        ] {
            let catalog =
                analyze_address_delegations(&empty, seeds(&[candidate]), |_| Ok(read.clone()))
                    .unwrap();
            assert_eq!(
                catalog.seeded_delegations.candidates[0].delegation_count_result,
                expected
            );
            assert_eq!(
                catalog.seeded_delegations.candidates[0].delegation_count,
                None
            );
        }
        let rows = BTreeMap::from([(key, ConcreteRead::Present(0_u32.to_le_bytes().to_vec()))]);
        let live = analyze_address_delegations(&inventory(&rows), seeds(&[candidate]), |_| {
            Ok(rows[&key].clone())
        })
        .unwrap();
        assert_eq!(
            live.seeded_delegations.candidates[0].delegation_count,
            Some(0)
        );
        for (error, expected) in [
            (
                ConcreteReadError::HistoryUnavailable(empty.identity),
                "history_unavailable",
            ),
            (ConcreteReadError::Pruned(empty.identity), "pruned"),
        ] {
            let catalog =
                analyze_address_delegations(&empty, seeds(&[candidate]), |_| Err(error.clone()))
                    .unwrap();
            assert_eq!(
                catalog.seeded_delegations.candidates[0].delegation_count_result,
                expected
            );
        }
    }
    #[test]
    fn catalog_rejects_required_missing_malformed_reverse_duplicate_and_byte_drift() {
        for case in 0..6 {
            let mut rows = rows();
            let prefix = delegator_validators_prefix([1; 20]);
            let original = inventory(&rows);
            let key = match case {
                0 => iterable_item_key_for(&prefix, 1),
                1 => iterable_position_key_for(&prefix, &[4; 20]),
                2 => storage_key(&[&[2, 0], &[4; 20], &[1; 20]]),
                3 => iterable_item_key_for(&prefix, 2),
                _ => iterable_count_key_for(&prefix),
            };
            let value = match case {
                0 => ConcreteRead::Tombstone,
                1 => ConcreteRead::Present(2_u32.to_le_bytes().to_vec()),
                2 => ConcreteRead::Present(vec![0xc0]),
                3 => ConcreteRead::Present([4; 20].to_vec()),
                4 => ConcreteRead::Present(vec![1]),
                _ => ConcreteRead::Present(0_u32.to_le_bytes().to_vec()),
            };
            rows.insert(key, value);
            let selected = if case == 5 {
                original
            } else {
                inventory(&rows)
            };
            assert!(
                analyze_address_delegations(&selected, seeds(&[[1; 20]]), |key| Ok(rows
                    .get(&key)
                    .cloned()
                    .unwrap_or(ConcreteRead::Absent)))
                .is_err(),
                "case {case}"
            );
        }
    }
    #[test]
    fn catalog_bounds_stop_before_child_expansion_or_extra_seed_reads() {
        let candidate = [1; 20];
        let count_key = iterable_count_key_for(&delegator_validators_prefix(candidate));
        let rows = BTreeMap::from([(
            count_key,
            ConcreteRead::Present(257_u32.to_le_bytes().to_vec()),
        )]);
        let mut calls = 0;
        assert!(
            analyze_address_delegations(&inventory(&rows), seeds(&[candidate]), |key| {
                calls += 1;
                assert_eq!(key, count_key);
                Ok(rows[&key].clone())
            })
            .is_err()
        );
        assert_eq!(calls, 1);
        let many = (0..21).map(|n| [n; 20]).collect::<Vec<_>>();
        assert!(
            analyze_address_delegations(&inventory(&BTreeMap::new()), seeds(&many), |_| panic!(
                "over-limit seeds read"
            ))
            .is_err()
        );
        let oversized = vec![0; MAX_TRANSFER_VALUE_BYTES + 1];
        assert!(
            analyze_address_delegations(&inventory(&BTreeMap::new()), seeds(&[candidate]), |_| Ok(
                ConcreteRead::Present(oversized.clone())
            ))
            .unwrap_err()
            .to_string()
            .contains("read derived")
        );
    }
    #[test]
    fn catalog_maximum_reads_and_aggregate_bytes_are_bounded() {
        let mut rows = BTreeMap::new();
        let addresses = (1..=20).map(|n| [n; 20]).collect::<Vec<_>>();
        for (index, delegator) in addresses.iter().enumerate() {
            let prefix = delegator_validators_prefix(*delegator);
            rows.insert(
                iterable_count_key_for(&prefix),
                ConcreteRead::Present(
                    (if index == 0 { 256_u32 } else { 0 })
                        .to_le_bytes()
                        .to_vec(),
                ),
            );
            if index != 0 {
                continue;
            }
            for position in 1..=256_u32 {
                let mut validator = [0x44; 20];
                validator[..4].copy_from_slice(&position.to_le_bytes());
                rows.insert(
                    iterable_item_key_for(&prefix, position),
                    ConcreteRead::Present(validator.to_vec()),
                );
                rows.insert(
                    iterable_position_key_for(&prefix, &validator),
                    ConcreteRead::Present(position.to_le_bytes().to_vec()),
                );
                let mut principal = RlpStream::new_list(2);
                principal.append(&1_u64).append(&30_u64);
                rows.insert(
                    storage_key(&[&[2, 0], &validator, delegator]),
                    ConcreteRead::Present(principal.out().to_vec()),
                );
                let mut cursor = RlpStream::new_list(2);
                cursor.append(&1_u64).append(&1_u32);
                rows.insert(
                    storage_key(&[&[1], &validator, &compact_u64_bytes(30)]),
                    ConcreteRead::Present(cursor.out().to_vec()),
                );
            }
        }
        let result = analyze_address_delegations(&inventory(&rows), seeds(&addresses), |key| {
            Ok(rows[&key].clone())
        })
        .unwrap();
        assert_eq!(result.derived_reads, MAX_TRANSFER_READS);
        assert_eq!(
            result.seeded_delegations.live_delegation_entries,
            MAX_TRANSFER_DELEGATIONS
        );
        let mut calls = 0;
        let error = analyze_address_delegations(
            &inventory(&BTreeMap::new()),
            seeds(&addresses[..2]),
            |_| {
                calls += 1;
                Ok(ConcreteRead::Present(vec![
                    1;
                    MAX_TRANSFER_VALUE_BYTES / 2 + 1
                ]))
            },
        )
        .unwrap_err();
        assert_eq!(calls, 2);
        assert!(format!("{error:#}").contains("value-byte limit"));
    }

    #[test]
    fn catalog_non_live_cursor_is_unresolved_and_live_history_contradiction_fails() {
        let mut rows = rows();
        let cursor = storage_key(&[&[1], &[4; 20], &compact_u64_bytes(30)]);
        let live = inventory(&rows);
        rows.remove(&cursor);
        let partial = inventory(&rows);
        let catalog = analyze_address_delegations(&partial, seeds(&[[1; 20]]), |key| {
            if key == cursor {
                Err(ConcreteReadError::HistoryUnavailable(partial.identity))
            } else {
                Ok(rows[&key].clone())
            }
        })
        .unwrap();
        assert_eq!(
            catalog.seeded_delegations.delegations[0].reward_node_result,
            "history_unavailable"
        );
        assert_eq!(
            catalog.seeded_delegations.delegations[0].reward_per_stake_hex,
            None
        );
        assert!(
            analyze_address_delegations(&live, seeds(&[[1; 20]]), |key| {
                if key == cursor {
                    Err(ConcreteReadError::HistoryUnavailable(live.identity))
                } else {
                    Ok(rows[&key].clone())
                }
            })
            .is_err()
        );
    }
}
