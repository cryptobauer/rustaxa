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

/// Exact current live coverage explained by the bounded inverse.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NativeInverseCoverage {
    pub validator_count: u32,
    pub validators: Vec<ValidatorInverseFact>,
    pub scalars: Vec<ScalarInverseFact>,
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

    analyzer.finish(validator_count, validators, scalars)
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

fn iterable_count_key() -> ConcreteStorageKey {
    storage_key(&[VALIDATORS_PREFIX, &[1]])
}

fn iterable_item_key(position: u32) -> ConcreteStorageKey {
    // The pinned Go constructors reuse spare prefix capacity. Initializing the
    // reverse key overwrites the item discriminator zero with two, so actual
    // retained rows use discriminator two for both directions.
    storage_key(&[VALIDATORS_PREFIX, &[2], &position.to_le_bytes()])
}

fn iterable_position_key(address: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[VALIDATORS_PREFIX, &[2], &address])
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
        assert_eq!(coverage.matched_live_entries, 13);
        assert_eq!(coverage.unexplained_live_entries, 1);
        assert!(coverage.live_partition_exact);
        assert!(!coverage.semantic_snapshot_complete);

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
