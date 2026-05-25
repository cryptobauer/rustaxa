//! CXX bridge wrappers for deterministic pillar-chain planning.
//!
//! This bridge exposes storage-free pillar-chain helpers to C++ shims. The
//! boundary accepts plain vote-count and linkage facts, converts them into
//! Rust consensus-domain values, and returns stable CXX payloads. C++ remains
//! responsible for FinalChain queries, `PillarBlock` object construction,
//! persistence, event emission, and network side effects.

use crate::ffi::rustaxa_ffi::{
    PillarBlockLinkageFact as FfiPillarBlockLinkageFact,
    PillarBlockLinkagePlan as FfiPillarBlockLinkagePlan,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
use anyhow::Result;
use ethereum_types::{H160, H256};
use rustaxa_consensus::{
    plan_pillar_block_linkage as consensus_plan_pillar_block_linkage,
    plan_pillar_vote_count_changes as consensus_plan_vote_count_changes,
    PillarBlockLinkageFact as ConsensusPillarBlockLinkageFact,
    PillarBlockLinkagePlan as ConsensusPillarBlockLinkagePlan,
    PillarValidatorVoteCount as ConsensusPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as ConsensusPillarValidatorVoteCountChange,
};

/// Computes ordered validator vote-count changes for a pillar block.
///
/// The C++ shim supplies the current DPoS vote-count snapshot and the previous
/// current-pillar snapshot when one exists. Rust returns legacy-compatible
/// signed deltas without constructing a `PillarBlock`.
pub fn plan_pillar_vote_count_changes(
    current_vote_counts: Vec<FfiPillarValidatorVoteCount>,
    previous_vote_counts: Vec<FfiPillarValidatorVoteCount>,
) -> Result<Vec<FfiPillarValidatorVoteCountChange>> {
    let current_vote_counts = current_vote_counts
        .into_iter()
        .map(vote_count_to_consensus)
        .collect::<Vec<_>>();
    let previous_vote_counts = previous_vote_counts
        .into_iter()
        .map(vote_count_to_consensus)
        .collect::<Vec<_>>();

    Ok(
        consensus_plan_vote_count_changes(&current_vote_counts, &previous_vote_counts)?
            .into_iter()
            .map(FfiPillarValidatorVoteCountChange::from)
            .collect(),
    )
}

/// Validates pillar-block parent linkage and returns an explicit status code.
pub fn plan_pillar_block_linkage(
    fact: FfiPillarBlockLinkageFact,
) -> Result<FfiPillarBlockLinkagePlan> {
    Ok(FfiPillarBlockLinkagePlan::from(
        consensus_plan_pillar_block_linkage(linkage_fact_to_consensus(fact))?,
    ))
}

fn vote_count_to_consensus(
    value: FfiPillarValidatorVoteCount,
) -> ConsensusPillarValidatorVoteCount {
    ConsensusPillarValidatorVoteCount {
        address: H160::from(value.address),
        vote_count: value.vote_count,
    }
}

fn linkage_fact_to_consensus(value: FfiPillarBlockLinkageFact) -> ConsensusPillarBlockLinkageFact {
    ConsensusPillarBlockLinkageFact {
        pillar_block_period: value.pillar_block_period,
        pillar_block_previous_hash: H256::from(value.pillar_block_previous_hash),
        first_pillar_block_period: value.first_pillar_block_period,
        pillar_blocks_interval: value.pillar_blocks_interval,
        last_finalized_period: value
            .has_last_finalized_pillar_block
            .then_some(value.last_finalized_period),
        last_finalized_hash: value
            .has_last_finalized_pillar_block
            .then_some(H256::from(value.last_finalized_hash)),
    }
}

impl From<ConsensusPillarValidatorVoteCountChange> for FfiPillarValidatorVoteCountChange {
    fn from(value: ConsensusPillarValidatorVoteCountChange) -> Self {
        Self {
            address: value.address.into(),
            vote_count_change: value.vote_count_change,
        }
    }
}

impl From<ConsensusPillarBlockLinkagePlan> for FfiPillarBlockLinkagePlan {
    fn from(value: ConsensusPillarBlockLinkagePlan) -> Self {
        Self {
            status: value.status.as_u8(),
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(value: u8) -> [u8; 20] {
        [value; 20]
    }

    fn hash(value: u64) -> [u8; 32] {
        H256::from_low_u64_be(value).into()
    }

    fn vote_count(address: u8, vote_count: u64) -> FfiPillarValidatorVoteCount {
        FfiPillarValidatorVoteCount {
            address: addr(address),
            vote_count,
        }
    }

    #[test]
    fn bridge_plans_vote_count_changes() {
        let changes = plan_pillar_vote_count_changes(
            vec![vote_count(3, 5), vote_count(1, 7)],
            vec![vote_count(3, 2), vote_count(2, 4)],
        )
        .unwrap();

        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].address, addr(1));
        assert_eq!(changes[0].vote_count_change, 7);
        assert_eq!(changes[1].address, addr(2));
        assert_eq!(changes[1].vote_count_change, -4);
        assert_eq!(changes[2].address, addr(3));
        assert_eq!(changes[2].vote_count_change, 3);
    }

    #[test]
    fn bridge_plans_pillar_block_linkage() {
        let valid = plan_pillar_block_linkage(FfiPillarBlockLinkageFact {
            pillar_block_period: 8,
            pillar_block_previous_hash: hash(44),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            has_last_finalized_pillar_block: true,
            last_finalized_period: 4,
            last_finalized_hash: hash(44),
        })
        .unwrap();

        assert!(valid.valid);
        assert_eq!(valid.status, 0);

        let invalid = plan_pillar_block_linkage(FfiPillarBlockLinkageFact {
            pillar_block_period: 8,
            pillar_block_previous_hash: hash(45),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            has_last_finalized_pillar_block: true,
            last_finalized_period: 4,
            last_finalized_hash: hash(44),
        })
        .unwrap();

        assert!(!invalid.valid);
        assert_eq!(invalid.status, 4);
    }
}
