//! CXX conversion for native PBFT period-state cleanup.
//!
//! Native [`rustaxa_consensus::PbftService`] owns validation, sibling lock
//! ordering, durable batch commit, and memory publication. This module keeps
//! only the stable CXX result conversion and entrypoint used by the temporary
//! PBFT manager executor.

use crate::ffi::rustaxa_ffi::PbftPeriodStateCleanupResult as FfiPbftPeriodStateCleanupResult;
use crate::ffi::BridgePbftService;
use anyhow::Result;
use rustaxa_consensus::PbftPeriodStateCleanupResult;

impl From<PbftPeriodStateCleanupResult> for FfiPbftPeriodStateCleanupResult {
    fn from(value: PbftPeriodStateCleanupResult) -> Self {
        Self {
            status: value.status.as_u8(),
            error_code: value.error_code,
            transition_published: value.transition_published,
            finalized_chain_size: value.finalized_chain_size,
            new_period: value.new_period,
            verified_vote_periods_removed: value.verified_vote_periods_removed,
            verified_votes_removed: value.verified_votes_removed,
            vote_payloads_removed: value.vote_payloads_removed,
            proposed_block_periods_removed: value.proposed_block_periods_removed,
            proposed_blocks_removed: value.proposed_blocks_removed,
            persistence_required: value.persistence_required,
            persistence_applied_deletes: value.persistence_applied_deletes,
        }
    }
}

/// Runs native PBFT period-state cleanup and converts its typed result for C++.
///
/// The native PBFT root validates the period transition, acquires sibling
/// locks, owns the durable proposal-deletion batch, and publishes in-memory
/// cleanup only after commit. Errors cross CXX unchanged; this adapter does not
/// inspect or mutate consensus state.
pub fn pbft_service_cleanup_period_state(
    service: &BridgePbftService,
    finalized_chain_size: u64,
    new_period: u64,
) -> Result<FfiPbftPeriodStateCleanupResult> {
    service
        .0
        .cleanup_period_state(finalized_chain_size, new_period)
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_consensus::PbftPeriodStateCleanupStatus;

    #[test]
    fn cleanup_result_conversion_preserves_every_field() {
        assert_eq!(PbftPeriodStateCleanupStatus::NotRequired.as_u8(), 0);
        assert_eq!(PbftPeriodStateCleanupStatus::Rejected.as_u8(), 2);

        let converted: FfiPbftPeriodStateCleanupResult = PbftPeriodStateCleanupResult {
            status: PbftPeriodStateCleanupStatus::Applied,
            error_code: "CLEANUP_SENTINEL".to_owned(),
            transition_published: true,
            finalized_chain_size: 11,
            new_period: 12,
            verified_vote_periods_removed: 13,
            verified_votes_removed: 14,
            vote_payloads_removed: 15,
            proposed_block_periods_removed: 16,
            proposed_blocks_removed: 17,
            persistence_required: true,
            persistence_applied_deletes: 18,
        }
        .into();

        assert_eq!(converted.status, 1);
        assert_eq!(converted.error_code, "CLEANUP_SENTINEL");
        assert!(converted.transition_published);
        assert_eq!(converted.finalized_chain_size, 11);
        assert_eq!(converted.new_period, 12);
        assert_eq!(converted.verified_vote_periods_removed, 13);
        assert_eq!(converted.verified_votes_removed, 14);
        assert_eq!(converted.vote_payloads_removed, 15);
        assert_eq!(converted.proposed_block_periods_removed, 16);
        assert_eq!(converted.proposed_blocks_removed, 17);
        assert!(converted.persistence_required);
        assert_eq!(converted.persistence_applied_deletes, 18);
    }
}
