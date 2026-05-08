use crate::ffi::rustaxa_ffi::{
    DagHash, ProposedBlockLookup, ProposedBlockPeriodHashes, ProposedBlockSnapshotEntry,
};
use crate::ffi::BridgeProposedBlocks;
use ethereum_types::H256;
use rustaxa_consensus::proposed_blocks::{ProposedBlockPeriod, ProposedBlocks};

/// Creates an empty Rust proposed-block index for the C++ PBFT shim.
pub fn create_proposed_blocks_index() -> Box<BridgeProposedBlocks> {
    Box::new(BridgeProposedBlocks(ProposedBlocks::new()))
}

impl BridgeProposedBlocks {
    /// Inserts a proposed PBFT block and returns whether it was newly inserted.
    pub fn proposed_blocks_push(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> bool {
        self.0.push(period, H256::from(*block_hash), block_rlp)
    }

    /// Marks an existing proposed PBFT block as valid.
    pub fn proposed_blocks_mark_valid(
        &mut self,
        period: u64,
        block_hash: &[u8; 32],
    ) -> Result<(), anyhow::Error> {
        self.0.mark_valid(period, H256::from(*block_hash))
    }

    /// Looks up a proposed PBFT block and its cached validation flag.
    pub fn proposed_blocks_get(&self, period: u64, block_hash: &[u8; 32]) -> ProposedBlockLookup {
        self.0
            .get(period, H256::from(*block_hash))
            .map(|entry| ProposedBlockLookup {
                found: true,
                is_valid: entry.is_valid,
                block_rlp: entry.block_rlp,
            })
            .unwrap_or(ProposedBlockLookup {
                found: false,
                is_valid: false,
                block_rlp: Vec::new(),
            })
    }

    /// Returns whether a proposed PBFT block is present.
    pub fn proposed_blocks_contains(&self, period: u64, block_hash: &[u8; 32]) -> bool {
        self.0.contains(period, H256::from(*block_hash))
    }

    /// Returns cleanup candidates for all periods lower than `period`.
    pub fn proposed_blocks_cleanup_candidates(
        &self,
        period: u64,
    ) -> Vec<ProposedBlockPeriodHashes> {
        self.0
            .cleanup_candidates(period)
            .into_iter()
            .map(|period| ProposedBlockPeriodHashes {
                period: period.period,
                block_hashes: period
                    .block_hashes
                    .into_iter()
                    .map(|hash| DagHash { hash: hash.into() })
                    .collect(),
            })
            .collect()
    }

    /// Removes one period from the in-memory proposed-block index.
    pub fn proposed_blocks_remove_period(&mut self, period: u64) {
        self.0.remove_period(period);
    }

    /// Returns the legacy old-blocks diagnostic string.
    pub fn proposed_blocks_old_blocks_message(&self, current_period: u64) -> String {
        self.0
            .old_blocks_message(current_period)
            .unwrap_or_default()
    }

    /// Returns all proposed PBFT block entries with validation flags.
    pub fn proposed_blocks_snapshot_entries(&self) -> Vec<ProposedBlockSnapshotEntry> {
        self.0
            .snapshot_entries()
            .into_iter()
            .map(|entry| ProposedBlockSnapshotEntry {
                period: entry.period,
                block_hash: entry.block_hash.into(),
                block_rlp: entry.block_rlp,
                is_valid: entry.is_valid,
            })
            .collect()
    }

    /// Returns all proposed PBFT block hashes grouped by period.
    pub fn proposed_blocks_snapshot(&self) -> Vec<ProposedBlockPeriodHashes> {
        self.0.snapshot().into_iter().map(Into::into).collect()
    }
}

impl From<ProposedBlockPeriod> for ProposedBlockPeriodHashes {
    fn from(value: ProposedBlockPeriod) -> Self {
        Self {
            period: value.period,
            block_hashes: value
                .blocks
                .into_iter()
                .map(|entry| DagHash {
                    hash: entry.block_hash.into(),
                })
                .collect(),
        }
    }
}
