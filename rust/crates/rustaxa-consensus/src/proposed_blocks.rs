//! Proposed PBFT block index for the Rust rewrite shim.
//!
//! This module owns the deterministic period/hash cache, proposal payload bytes,
//! and validation flags used by PBFT proposal handling. Legacy C++ behavior is
//! preserved at the domain boundary: duplicate insertions are rejected, validity
//! is tracked per `(period, hash)`, and stale-period cleanup returns removed
//! block hashes so callers can remove persisted DB entries.

use anyhow::{Result, anyhow};
use ethereum_types::H256;
use std::collections::{BTreeMap, btree_map::Entry};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProposedBlockState {
    block_rlp: Vec<u8>,
    is_valid: bool,
}

/// Snapshot of one proposed PBFT block entry.
///
/// Inputs/outputs:
/// - `period`: PBFT period that owns the proposal.
/// - `block_hash`: proposed PBFT block hash.
/// - `block_rlp`: serialized PBFT block bytes for bridge reconstruction.
/// - `is_valid`: cached result of expensive PBFT block validation.
///
/// Invariants:
/// - each `(period, block_hash)` pair appears at most once in a
///   `ProposedBlocks` index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockEntry {
    pub period: u64,
    pub block_hash: H256,
    pub block_rlp: Vec<u8>,
    pub is_valid: bool,
}

/// Proposed block hashes grouped by PBFT period.
///
/// This payload is intended for cleanup and DB-delete callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockPeriodHashes {
    pub period: u64,
    pub block_hashes: Vec<H256>,
}

/// Proposed blocks grouped by PBFT period.
///
/// Period order is ascending. Block order within each period is ascending by
/// block hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedBlockPeriod {
    pub period: u64,
    pub blocks: Vec<ProposedBlockEntry>,
}

/// Rust-owned proposed PBFT block index.
///
/// The index tracks proposal membership, proposal payload bytes, and validation
/// flag per period/hash. Persistence is intentionally external: callers decide
/// when to write or delete DB entries and can use cleanup/snapshot outputs to
/// drive those side effects.
#[derive(Debug, Default, Clone)]
pub struct ProposedBlocks {
    blocks: BTreeMap<u64, BTreeMap<H256, ProposedBlockState>>,
}

impl ProposedBlocks {
    /// Creates an empty proposed block index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a proposed block with an initially invalid validation cache bit.
    ///
    /// Returns `true` when the block was newly inserted and `false` when the
    /// same period/hash already existed. Existing entries are not overwritten.
    pub fn push(&mut self, period: u64, block_hash: H256, block_rlp: Vec<u8>) -> bool {
        match self.blocks.entry(period).or_default().entry(block_hash) {
            Entry::Vacant(entry) => {
                entry.insert(ProposedBlockState {
                    block_rlp,
                    is_valid: false,
                });
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Marks an existing proposed block as valid.
    ///
    /// Returns an error when the block is absent. The C++ legacy code asserted
    /// this invariant; the Rust rewrite reports it explicitly instead of
    /// dereferencing an invalid iterator.
    pub fn mark_valid(&mut self, period: u64, block_hash: H256) -> Result<()> {
        let valid = self
            .blocks
            .get_mut(&period)
            .and_then(|period_blocks| period_blocks.get_mut(&block_hash));
        let Some(valid) = valid else {
            return Err(anyhow!(
                "cannot mark missing proposed PBFT block as valid: period {period}, hash {block_hash:#x}"
            ));
        };
        valid.is_valid = true;
        Ok(())
    }

    /// Returns the proposed block entry for `period` and `block_hash`.
    pub fn get(&self, period: u64, block_hash: H256) -> Option<ProposedBlockEntry> {
        self.blocks
            .get(&period)
            .and_then(|period_blocks| period_blocks.get(&block_hash))
            .map(|state| ProposedBlockEntry {
                period,
                block_hash,
                block_rlp: state.block_rlp.clone(),
                is_valid: state.is_valid,
            })
    }

    /// Returns true when `period` contains `block_hash`.
    pub fn contains(&self, period: u64, block_hash: H256) -> bool {
        self.get(period, block_hash).is_some()
    }

    /// Removes and returns all proposed block hashes with period lower than `period`.
    pub fn cleanup_before(&mut self, period: u64) -> Vec<ProposedBlockPeriodHashes> {
        let old_periods = self
            .blocks
            .keys()
            .copied()
            .take_while(|candidate| *candidate < period)
            .collect::<Vec<_>>();

        old_periods
            .into_iter()
            .filter_map(|old_period| {
                self.blocks
                    .remove(&old_period)
                    .map(|blocks| ProposedBlockPeriodHashes {
                        period: old_period,
                        block_hashes: blocks.into_keys().collect(),
                    })
            })
            .collect()
    }

    /// Returns all cleanup candidate periods and hashes for `< period`.
    ///
    /// This is the non-mutating half of legacy cleanup flow where callers first
    /// delete persisted entries and then drop periods from memory.
    pub fn cleanup_candidates(&self, period: u64) -> Vec<ProposedBlockPeriodHashes> {
        self.blocks
            .iter()
            .take_while(|(candidate, _)| **candidate < period)
            .map(|(candidate, hashes)| ProposedBlockPeriodHashes {
                period: *candidate,
                block_hashes: hashes.keys().copied().collect(),
            })
            .collect()
    }

    /// Removes one period from the in-memory cache.
    pub fn remove_period(&mut self, period: u64) {
        self.blocks.remove(&period);
    }

    /// Returns the legacy old-blocks diagnostic message when stale periods exist.
    ///
    /// The format is exactly `"period -> count. "` repeated in ascending period
    /// order.
    pub fn old_blocks_message(&self, current_period: u64) -> Option<String> {
        let msg = self
            .blocks
            .iter()
            .take_while(|(period, _)| **period < current_period)
            .map(|(period, blocks)| format!("{period} -> {}. ", blocks.len()))
            .collect::<String>();

        (!msg.is_empty()).then_some(msg)
    }

    /// Returns all proposed blocks grouped by ascending period.
    pub fn snapshot(&self) -> Vec<ProposedBlockPeriod> {
        self.blocks
            .iter()
            .map(|(period, blocks)| ProposedBlockPeriod {
                period: *period,
                blocks: blocks
                    .iter()
                    .map(|(block_hash, state)| ProposedBlockEntry {
                        period: *period,
                        block_hash: *block_hash,
                        block_rlp: state.block_rlp.clone(),
                        is_valid: state.is_valid,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Returns all proposed blocks as a flat list in deterministic order.
    pub fn snapshot_entries(&self) -> Vec<ProposedBlockEntry> {
        self.snapshot()
            .into_iter()
            .flat_map(|period| period.blocks)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn rlp(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    #[test]
    fn push_tracks_membership_and_payload_without_overwriting_existing_entry() {
        let mut blocks = ProposedBlocks::new();

        assert!(blocks.push(2, hash(10), rlp(&[0xAA])));
        assert!(!blocks.push(2, hash(10), rlp(&[0xBB])));
        assert!(blocks.contains(2, hash(10)));
        assert!(!blocks.contains(2, hash(11)));

        let entry = blocks.get(2, hash(10)).unwrap();
        assert_eq!(entry.period, 2);
        assert_eq!(entry.block_hash, hash(10));
        assert_eq!(entry.block_rlp, rlp(&[0xAA]));
        assert!(!entry.is_valid);
    }

    #[test]
    fn mark_valid_updates_existing_entry_and_rejects_missing_entry() {
        let mut blocks = ProposedBlocks::new();

        assert!(blocks.push(3, hash(30), rlp(&[0xC0])));
        blocks.mark_valid(3, hash(30)).unwrap();
        assert!(blocks.get(3, hash(30)).unwrap().is_valid);

        let err = blocks.mark_valid(3, hash(31)).unwrap_err().to_string();
        assert!(err.contains("missing proposed PBFT block"));
    }

    #[test]
    fn cleanup_removes_only_periods_lower_than_current_period() {
        let mut blocks = ProposedBlocks::new();
        blocks.push(1, hash(11), rlp(&[0x11]));
        blocks.push(2, hash(22), rlp(&[0x22]));
        blocks.push(3, hash(33), rlp(&[0x33]));

        let removed = blocks.cleanup_before(3);
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].period, 1);
        assert_eq!(removed[0].block_hashes, vec![hash(11)]);
        assert_eq!(removed[1].period, 2);
        assert_eq!(removed[1].block_hashes, vec![hash(22)]);
        assert!(!blocks.contains(1, hash(11)));
        assert!(blocks.contains(3, hash(33)));
    }

    #[test]
    fn old_blocks_message_matches_legacy_format() {
        let mut blocks = ProposedBlocks::new();
        blocks.push(1, hash(10), rlp(&[0x10]));
        blocks.push(1, hash(11), rlp(&[0x11]));
        blocks.push(2, hash(20), rlp(&[0x20]));
        blocks.push(3, hash(30), rlp(&[0x30]));

        assert_eq!(
            blocks.old_blocks_message(3),
            Some("1 -> 2. 2 -> 1. ".to_owned())
        );
        assert_eq!(blocks.old_blocks_message(1), None);
    }

    #[test]
    fn snapshot_and_entries_preserve_payload_and_validation_state() {
        let mut blocks = ProposedBlocks::new();
        blocks.push(1, hash(1), rlp(&[0xA1]));
        blocks.push(1, hash(2), rlp(&[0xA2]));
        blocks.push(2, hash(3), rlp(&[0xB3]));
        blocks.mark_valid(1, hash(2)).unwrap();

        let periods = blocks.snapshot();
        assert_eq!(periods.len(), 2);
        assert_eq!(periods[0].period, 1);
        assert_eq!(periods[0].blocks.len(), 2);
        assert_eq!(periods[1].period, 2);
        assert_eq!(periods[1].blocks.len(), 1);

        let entries = blocks.snapshot_entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].block_hash, hash(1));
        assert!(!entries[0].is_valid);
        assert_eq!(entries[1].block_hash, hash(2));
        assert!(entries[1].is_valid);
        assert_eq!(entries[2].block_hash, hash(3));
        assert_eq!(entries[2].block_rlp, rlp(&[0xB3]));
    }
}
