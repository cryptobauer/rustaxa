//! Partial root-bound slashing jail catalog from justified validator addresses.
//! Empty jailed-list state does not erase persistent jail rows. Residual paths
//! have no decoded meaning. No graph/history/snapshot authority is issued here.
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{ConcreteStorageInventory, ConcreteStorageInventoryEntry};
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStorageKey};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tiny_keccak::{Hasher, Keccak};

pub const MAX_JAIL_SEEDS: usize = 196;
pub const MAX_JAIL_READS: usize = MAX_JAIL_SEEDS + 1;
pub const MAX_JAIL_POINT_BYTES: usize = 1024 * 1024;
pub const SLASHING_ADDRESS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xee,
];

/// Address and one-based position from an independently justified source index.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JailSeed {
    pub address: [u8; 20],
    pub source_position: u32,
}

/// Exact source/key/raw classification for one optional historical jail row.
/// A decoded block is present only for a canonical live inventory member.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JailFact {
    pub validator_hex: String,
    pub source_position: u32,
    pub logical_key_hex: String,
    pub trie_path_hex: String,
    pub physical_result: &'static str,
    pub exact_value_hex: Option<String>,
    pub jail_block: Option<u64>,
}

/// Root-bound live row. Residual rows remain opaque regardless of value shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SlashingLiveRow {
    pub trie_path_hex: String,
    pub exact_value_hex: String,
}

/// Exact independent live partition for this seed source and the fixed list.
/// Source order and expired jail blocks are retained; no policy is applied.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SlashingJailCatalog {
    pub jailed_list_logical_key_hex: String,
    pub jailed_list_exact_value_hex: String,
    pub seeds: Vec<JailFact>,
    pub derived_reads: usize,
    pub accepted_point_value_bytes: usize,
    pub matched_rows: Vec<SlashingLiveRow>,
    pub unexplained_rows: Vec<SlashingLiveRow>,
    pub matched_value_bytes: u64,
    pub unexplained_value_bytes: u64,
    pub unexplained_entries_sha256: String,
    pub live_partition_exact: bool,
    pub all_ever_jail_keys_complete: bool,
    pub semantic_snapshot_complete: bool,
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    let mut hash = Keccak::v256();
    hash.update(bytes);
    hash.finalize(&mut out);
    out
}
/// Direct left-padded field 02, not a hashed native preimage.
pub fn jailed_list_key() -> ConcreteStorageKey {
    let mut key = [0; 32];
    key[31] = 2;
    ConcreteStorageKey(key)
}
/// Native logical key Keccak(00 || validator), before trie-path hashing.
pub fn jail_key(validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(keccak(&[&[0][..], &validator].concat()))
}

/// Decodes the fixed empty-list profile and optional persistent jail rows.
///
/// Caller must bind the complete EE inventory, raw reader and justified seeds
/// to one identity. At most 196 unique addresses and distinct valid positions
/// are accepted. Non-live optional rows keep their physical class; unexpected
/// errors, live mismatches, malformed live integers or any exceeded cap fail
/// without a partial successful catalog. The list must be live canonical c0.
/// Aggregate accepted point bytes include repeated/orphan physical values.
pub fn analyze_slashing_jails<F>(
    inventory: &ConcreteStorageInventory,
    seeds: &[JailSeed],
    mut read: F,
) -> Result<SlashingJailCatalog>
where
    F: FnMut(ConcreteStorageKey) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
{
    ensure!(
        inventory.address == SLASHING_ADDRESS,
        "inventory is not slashing"
    );
    ensure!(seeds.len() <= MAX_JAIL_SEEDS, "jail seed bound exceeded");
    let mut addresses = BTreeSet::new();
    let mut positions = BTreeSet::new();
    for seed in seeds {
        ensure!(
            addresses.insert(seed.address),
            "duplicate jail seed address"
        );
        ensure!(
            seed.source_position > 0
                && seed.source_position <= MAX_JAIL_SEEDS as u32
                && positions.insert(seed.source_position),
            "invalid jail seed position"
        );
    }
    let indexed = inventory
        .entries
        .iter()
        .map(|row| (row.hashed_path, row.value.as_slice()))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        indexed.len() == inventory.entries.len(),
        "duplicate live slashing paths"
    );
    let mut matched = BTreeSet::new();
    let mut calls = 0;
    let mut bytes = 0_usize;
    let mut probe = |key: ConcreteStorageKey| -> Result<(&'static str, Option<Vec<u8>>)> {
        ensure!(calls < MAX_JAIL_READS, "jail point-read bound exceeded");
        calls += 1;
        let path = keccak(&key.0);
        let (result, value) = match read(key) {
            Ok(ConcreteRead::Present(value)) => {
                bytes = bytes
                    .checked_add(value.len())
                    .context("jail point-byte overflow")?;
                ensure!(
                    bytes <= MAX_JAIL_POINT_BYTES,
                    "jail point-byte bound exceeded"
                );
                if let Some(live) = indexed.get(&path) {
                    ensure!(*live == value, "jail raw bytes differ from live inventory");
                    ensure!(matched.insert(path), "duplicate derived slashing path");
                    ("live_present", Some(value))
                } else {
                    ("orphan_present", Some(value))
                }
            }
            Ok(ConcreteRead::Absent) => ("absent", None),
            Ok(ConcreteRead::Tombstone) => ("tombstone", None),
            Err(ConcreteReadError::HistoryUnavailable(_)) => ("history_unavailable", None),
            Err(ConcreteReadError::Pruned(_)) => ("pruned", None),
            Err(error) => return Err(anyhow::Error::new(error)).context("slashing point read"),
        };
        ensure!(
            result == "live_present" || !indexed.contains_key(&path),
            "non-live slashing read at live path"
        );
        Ok((result, value))
    };
    let list_key = jailed_list_key();
    let (list_result, list) = probe(list_key)?;
    ensure!(
        list_result == "live_present" && list.as_deref() == Some(&[0xc0][..]),
        "fixed jailed list must be live canonical c0"
    );
    let mut facts = Vec::new();
    for seed in seeds {
        let key = jail_key(seed.address);
        let (physical_result, value) = probe(key)?;
        let jail_block = if physical_result == "live_present" {
            let raw = value.as_deref().context("live jail row has no bytes")?;
            let block: u64 = rlp::decode(raw).context("decode jail block")?;
            ensure!(
                rlp::encode(&block).as_ref() == raw,
                "jail block is not canonical or fully consumed"
            );
            Some(block)
        } else {
            None
        };
        facts.push(JailFact {
            validator_hex: hex::encode(seed.address),
            source_position: seed.source_position,
            logical_key_hex: hex::encode(key.0),
            trie_path_hex: hex::encode(keccak(&key.0)),
            physical_result,
            exact_value_hex: value.map(hex::encode),
            jail_block,
        });
    }
    let mut matched_rows = Vec::new();
    let mut unexplained_rows = Vec::new();
    let mut residual = Vec::new();
    let mut matched_bytes = 0_u64;
    let mut residual_bytes = 0_u64;
    for (path, value) in indexed {
        let row = SlashingLiveRow {
            trie_path_hex: hex::encode(path),
            exact_value_hex: hex::encode(value),
        };
        if matched.contains(&path) {
            matched_bytes += u64::try_from(value.len())?;
            matched_rows.push(row);
        } else {
            residual_bytes += u64::try_from(value.len())?;
            unexplained_rows.push(row);
            residual.push(ConcreteStorageInventoryEntry {
                hashed_path: path,
                value: value.to_vec(),
            });
        }
    }
    ensure!(
        matched_rows.len() + unexplained_rows.len() == inventory.entries.len()
            && matched_bytes.checked_add(residual_bytes) == Some(inventory.value_bytes),
        "slashing partition drift"
    );
    Ok(SlashingJailCatalog {
        jailed_list_logical_key_hex: hex::encode(list_key.0),
        jailed_list_exact_value_hex: "c0".into(),
        seeds: facts,
        derived_reads: calls,
        accepted_point_value_bytes: bytes,
        matched_rows,
        unexplained_rows,
        matched_value_bytes: matched_bytes,
        unexplained_value_bytes: residual_bytes,
        unexplained_entries_sha256: crate::native_inverse::digest_entries(&residual),
        live_partition_exact: true,
        all_ever_jail_keys_complete: false,
        semantic_snapshot_complete: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_types::{FinalChainBlockNumber, concrete_state::ConcreteStateIdentity};
    fn inventory(rows: &[(ConcreteStorageKey, Vec<u8>)]) -> ConcreteStorageInventory {
        let mut entries = rows
            .iter()
            .map(|(key, value)| ConcreteStorageInventoryEntry {
                hashed_path: keccak(&key.0),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|row| row.hashed_path);
        ConcreteStorageInventory {
            identity: ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(50),
                state_root: [7; 32],
            },
            address: SLASHING_ADDRESS,
            storage_root: Some([8; 32]),
            nodes_visited: 1,
            value_bytes: entries.iter().map(|row| row.value.len() as u64).sum(),
            entries,
        }
    }
    fn seed() -> JailSeed {
        JailSeed {
            address: [3; 20],
            source_position: 1,
        }
    }
    #[test]
    fn exact_keys_expired_row_source_order_and_partition() {
        assert_eq!(jailed_list_key().0[31], 2);
        assert_ne!(jailed_list_key().0, keccak(&[2]));
        assert_eq!(jail_key([3; 20]).0, keccak(&[&[0][..], &[3; 20]].concat()));
        let rows = vec![
            (jailed_list_key(), vec![0xc0]),
            (jail_key(seed().address), rlp::encode(&4_u64).to_vec()),
            (ConcreteStorageKey([9; 32]), vec![1]),
        ];
        let cat = analyze_slashing_jails(&inventory(&rows), &[seed()], |key| {
            Ok(ConcreteRead::Present(
                rows.iter().find(|row| row.0 == key).unwrap().1.clone(),
            ))
        })
        .unwrap();
        assert_eq!(cat.seeds[0].jail_block, Some(4));
        assert_eq!(cat.matched_rows.len(), 2);
        assert_eq!(cat.unexplained_rows.len(), 1);
        assert_eq!(cat.derived_reads, 2);
        assert!(!cat.all_ever_jail_keys_complete);
        assert!(!cat.semantic_snapshot_complete);
    }
    #[test]
    fn optional_physical_results_never_become_zero_and_unexpected_errors_fail() {
        let inv = inventory(&[(jailed_list_key(), vec![0xc0])]);
        for (read, expected) in [
            (Ok(ConcreteRead::Absent), "absent"),
            (Ok(ConcreteRead::Tombstone), "tombstone"),
            (Ok(ConcreteRead::Present(vec![0x80])), "orphan_present"),
            (
                Err(ConcreteReadError::HistoryUnavailable(inv.identity)),
                "history_unavailable",
            ),
            (Err(ConcreteReadError::Pruned(inv.identity)), "pruned"),
        ] {
            let cat = analyze_slashing_jails(&inv, &[seed()], |key| {
                if key == jailed_list_key() {
                    Ok(ConcreteRead::Present(vec![0xc0]))
                } else {
                    read.clone()
                }
            })
            .unwrap();
            assert_eq!(cat.seeds[0].physical_result, expected);
            assert_eq!(cat.seeds[0].jail_block, None);
        }
        assert!(
            analyze_slashing_jails(&inv, &[seed()], |key| if key == jailed_list_key() {
                Ok(ConcreteRead::Present(vec![0xc0]))
            } else {
                Err(ConcreteReadError::Corrupt("unexpected".into()))
            })
            .is_err()
        );
        let rows = [
            (jailed_list_key(), vec![0xc0]),
            (jail_key(seed().address), vec![0x80]),
        ];
        let cat = analyze_slashing_jails(&inventory(&rows), &[seed()], |key| {
            Ok(ConcreteRead::Present(
                rows.iter().find(|row| row.0 == key).unwrap().1.clone(),
            ))
        })
        .unwrap();
        assert_eq!(cat.seeds[0].jail_block, Some(0));
    }
    #[test]
    fn malformed_live_integer_list_drift_and_required_history_fail() {
        for raw in [
            vec![0xc0],
            vec![0x81, 0],
            vec![1, 2],
            vec![0x89, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        ] {
            let rows = [
                (jailed_list_key(), vec![0xc0]),
                (jail_key(seed().address), raw),
            ];
            assert!(
                analyze_slashing_jails(&inventory(&rows), &[seed()], |key| Ok(
                    ConcreteRead::Present(rows.iter().find(|row| row.0 == key).unwrap().1.clone())
                ))
                .is_err()
            );
        }
        let inv = inventory(&[(jailed_list_key(), vec![0xc0])]);
        for row in [
            ConcreteRead::Absent,
            ConcreteRead::Tombstone,
            ConcreteRead::Present(vec![0xc1, 0x80]),
        ] {
            assert!(analyze_slashing_jails(&inv, &[], |_| Ok(row.clone())).is_err());
        }
        let inv = inventory(&[
            (jailed_list_key(), vec![0xc0]),
            (jail_key(seed().address), vec![1]),
        ]);
        assert!(
            analyze_slashing_jails(&inv, &[seed()], |key| if key == jailed_list_key() {
                Ok(ConcreteRead::Present(vec![0xc0]))
            } else {
                Err(ConcreteReadError::HistoryUnavailable(inv.identity))
            })
            .is_err()
        );
        assert!(
            analyze_slashing_jails(&inv, &[seed()], |key| if key == jailed_list_key() {
                Ok(ConcreteRead::Present(vec![0xc0]))
            } else {
                Ok(ConcreteRead::Present(vec![2]))
            })
            .is_err()
        );
    }
    #[test]
    fn source_and_byte_bounds_fail_before_followup_reads() {
        let inv = inventory(&[(jailed_list_key(), vec![0xc0])]);
        assert!(
            analyze_slashing_jails(&inv, &[seed(), seed()], |_| panic!("duplicate seed read"))
                .is_err()
        );
        let mut invalid = seed();
        invalid.source_position = 0;
        assert!(
            analyze_slashing_jails(&inv, &[invalid], |_| panic!("invalid position read")).is_err()
        );
        let seeds = (0..197)
            .map(|n| JailSeed {
                address: [n as u8; 20],
                source_position: n + 1,
            })
            .collect::<Vec<_>>();
        assert!(analyze_slashing_jails(&inv, &seeds, |_| panic!("over-bound seed read")).is_err());
        let mut calls = 0;
        assert!(
            analyze_slashing_jails(&inv, &[seed()], |key| {
                calls += 1;
                if key == jailed_list_key() {
                    Ok(ConcreteRead::Present(vec![0xc0]))
                } else {
                    Ok(ConcreteRead::Present(vec![1; MAX_JAIL_POINT_BYTES]))
                }
            })
            .is_err()
        );
        assert_eq!(calls, 2);
    }
    #[test]
    fn exact_maximum_calls_preserve_supplied_source_order() {
        let inv = inventory(&[(jailed_list_key(), vec![0xc0])]);
        let seeds = (1..=196)
            .rev()
            .map(|n| JailSeed {
                address: [n as u8; 20],
                source_position: n,
            })
            .collect::<Vec<_>>();
        let cat = analyze_slashing_jails(&inv, &seeds, |key| {
            if key == jailed_list_key() {
                Ok(ConcreteRead::Present(vec![0xc0]))
            } else {
                Ok(ConcreteRead::Absent)
            }
        })
        .unwrap();
        assert_eq!(cat.derived_reads, MAX_JAIL_READS);
        assert_eq!(cat.seeds[0].source_position, 196);
        assert_eq!(cat.seeds[195].source_position, 1);
    }
}
