//! CXX bridge wrappers for deterministic pillar-vote aggregation.
//!
//! The bridge accepts plain C++-style vote payloads (`vote_hash`, `block_hash`,
//! `voter`, `period`, `weight`, `vote_rlp`) and converts them into
//! `rustaxa_consensus::VerifiedPillarVote` domain values.
//!
//! Cryptographic checks and signature validation remain outside this module; this
//! layer only enforces local bridge-domain invariants and delegates aggregation
//! rules to [`PillarVotes`].

use crate::ffi::rustaxa_ffi::{
    PillarVoteInsertOutcome, PillarVotePayload, PillarVoteRef, PillarVoteUniqueOutcome,
    PillarVotesLookup,
};
use crate::ffi::BridgePillarVotes;
use anyhow::{ensure, Result};
use ethereum_types::{H160, H256};
use rustaxa_consensus::{
    PillarVoteInsertOutcome as ConsensusPillarVoteInsertOutcome, PillarVotes, VerifiedPillarVote,
};
use rustaxa_types::PillarVote;

/// Creates an empty Rust pillar-vote registry for the C++ pillar-vote shim.
pub fn create_pillar_votes_index() -> Box<BridgePillarVotes> {
    Box::new(BridgePillarVotes(PillarVotes::new()))
}

impl BridgePillarVotes {
    /// Returns whether threshold/vote state exists for `period`.
    pub fn pillar_votes_period_data_initialized(&self, period: u64) -> bool {
        self.0.period_data_initialized(period)
    }

    /// Initializes period-wide threshold data.
    ///
    /// The first initialization for `period` wins; existing state is unchanged
    /// for repeated calls with the same period.
    pub fn pillar_votes_init_period_data(&mut self, period: u64, threshold: u64) -> bool {
        self.0.initialize_period_data(period, threshold)
    }

    /// Checks exact `(period, block_hash, vote_hash)` membership.
    pub fn pillar_votes_vote_exists(&self, vote: PillarVotePayload) -> Result<bool> {
        let vote = payload_to_vote(vote)?;
        Ok(self.0.vote_exists(&vote))
    }

    /// Checks whether a vote is unique for `(period, voter)` without mutating state.
    pub fn pillar_votes_is_unique_vote(
        &self,
        vote: PillarVotePayload,
    ) -> Result<PillarVoteUniqueOutcome> {
        let vote = payload_to_vote(vote)?;
        Ok(PillarVoteUniqueOutcome {
            is_unique: self.0.is_unique_vote(&vote),
        })
    }

    /// Inserts one verified pillar vote and returns deterministic aggregate state.
    pub fn pillar_votes_insert_vote(
        &mut self,
        vote: PillarVotePayload,
    ) -> Result<PillarVoteInsertOutcome> {
        let vote = payload_to_vote(vote)?;
        Ok(self.0.add_verified_vote(vote)?.into())
    }

    /// Looks up one pillar block's votes for threshold or non-threshold views.
    ///
    /// With `above_threshold = false`, all votes are returned in vote-hash order.
    /// With `above_threshold = true`, the minimal deterministic weighted prefix is
    /// returned only when cumulative weight reaches the period threshold.
    pub fn pillar_votes_get_verified_votes(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> PillarVotesLookup {
        self.0
            .get_verified_votes(period, H256::from(*block_hash), above_threshold)
            .into()
    }

    /// Removes all pillar-vote state for periods lower than `min_period`.
    pub fn pillar_votes_cleanup_votes_by_period(&mut self, min_period: u64) {
        self.0.erase_votes(min_period);
    }

    /// Returns all stored vote refs for C++ shim sidecar pruning.
    pub fn pillar_votes_snapshot_refs(&self) -> Vec<PillarVoteRef> {
        self.0
            .snapshot_votes()
            .into_iter()
            .map(PillarVoteRef::from)
            .collect()
    }
}

fn payload_to_vote(value: PillarVotePayload) -> Result<VerifiedPillarVote> {
    let vote = PillarVote::decode_rlp(&value.vote_rlp)?;
    ensure!(
        value.period == vote.period,
        "pillar vote payload period mismatch: {payload_period} != {vote_period}",
        payload_period = value.period,
        vote_period = vote.period
    );
    ensure!(
        H256::from(value.block_hash) == vote.block_hash,
        "pillar vote payload block hash mismatch for period {}",
        value.period
    );
    ensure!(
        H256::from(value.vote_hash) == vote.hash(true),
        "pillar vote payload hash mismatch for period {}",
        value.period
    );

    VerifiedPillarVote::from_parts(
        vote,
        H256::from(value.vote_hash),
        H160::from(value.voter),
        value.weight,
    )
}

impl From<ConsensusPillarVoteInsertOutcome> for PillarVoteInsertOutcome {
    fn from(value: ConsensusPillarVoteInsertOutcome) -> Self {
        Self {
            accepted: value.accepted,
            duplicate: value.duplicate,
            conflicting_vote_hash: value.conflicting_vote_hash.unwrap_or_default().into(),
            block_weight: value.block_weight,
            conflict_found: value.conflicting_vote_hash.is_some(),
        }
    }
}

impl From<rustaxa_consensus::VerifiedPillarVote> for PillarVoteRef {
    fn from(value: rustaxa_consensus::VerifiedPillarVote) -> Self {
        Self {
            vote_hash: value.vote_hash.into(),
            weight: value.weight,
        }
    }
}

impl From<rustaxa_consensus::PillarVotesLookup> for PillarVotesLookup {
    fn from(value: rustaxa_consensus::PillarVotesLookup) -> Self {
        Self {
            threshold_met: value.threshold_met,
            block_weight: value.block_weight,
            selected_weight: value.selected_weight,
            votes: value.votes.into_iter().map(PillarVoteRef::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ethereum_types::H256;

    fn signature(seed: u8) -> [u8; 65] {
        let mut signature = [seed; 65];
        signature[64] = seed & 1;
        signature
    }

    fn vote(period: u64, block: u64, voter: u64, seed: u8, weight: u64) -> PillarVotePayload {
        let vote = PillarVote {
            period,
            block_hash: H256::from_low_u64_be(block),
            signature: signature(seed),
        };
        PillarVotePayload {
            vote_hash: vote.hash(true).into(),
            block_hash: vote.block_hash.into(),
            voter: [voter as u8; 20],
            period,
            weight,
            vote_rlp: vote.encode_rlp(),
        }
    }

    fn clone_payload(value: &PillarVotePayload) -> PillarVotePayload {
        PillarVotePayload {
            vote_hash: value.vote_hash,
            block_hash: value.block_hash,
            voter: value.voter,
            period: value.period,
            weight: value.weight,
            vote_rlp: value.vote_rlp.clone(),
        }
    }

    #[test]
    fn insert_vote_accepts_votes_and_tracks_weight() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(10, 10));

        let first = vote(10, 11, 1, 0xAA, 4);
        let second = vote(10, 11, 2, 0xAB, 6);

        let first_outcome = votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        let second_outcome = votes.pillar_votes_insert_vote(second).unwrap();

        assert!(first_outcome.accepted);
        assert!(!first_outcome.duplicate);
        assert_eq!(first_outcome.block_weight, 4);
        assert!(second_outcome.accepted);
        assert!(!second_outcome.duplicate);
        assert_eq!(second_outcome.block_weight, 10);
    }

    #[test]
    fn duplicate_vote_hash_is_rejected_for_weight_recount_and_kept_unique() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(11, 1));

        let first = vote(11, 12, 1, 0xAC, 6);
        votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        let duplicate = votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();

        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert!(!duplicate.conflict_found);
        assert_eq!(duplicate.block_weight, 6);

        let lookup = votes.pillar_votes_get_verified_votes(11, &first.block_hash, false);
        assert_eq!(lookup.votes.len(), 1);
        assert_eq!(lookup.votes[0].vote_hash, first.vote_hash);
    }

    #[test]
    fn unique_vote_rejects_conflicting_voter() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(12, 1));

        let first = vote(12, 13, 1, 0xB0, 5);
        let conflict = vote(12, 14, 1, 0xB1, 5);

        let inserted = votes.pillar_votes_insert_vote(first).unwrap();
        assert!(inserted.accepted);

        let unique = votes.pillar_votes_is_unique_vote(conflict).unwrap();
        assert!(!unique.is_unique);
    }

    #[test]
    fn vote_exists_and_period_initialized_mirror_registry_state() {
        let mut votes = create_pillar_votes_index();
        let first = vote(12, 13, 1, 0xAF, 5);

        assert!(!votes.pillar_votes_period_data_initialized(12));
        votes.pillar_votes_init_period_data(12, 1);
        assert!(votes.pillar_votes_period_data_initialized(12));
        assert!(!votes
            .pillar_votes_vote_exists(clone_payload(&first))
            .unwrap());

        votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        assert!(votes.pillar_votes_vote_exists(first).unwrap());
    }

    #[test]
    fn above_threshold_lookup_selects_minimum_prefix_when_met() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(13, 7));

        let low = vote(13, 15, 1, 0xC0, 1);
        let mid = vote(13, 15, 2, 0xC1, 3);
        let high = vote(13, 15, 3, 0xC2, 4);
        votes.pillar_votes_insert_vote(low).unwrap();
        votes.pillar_votes_insert_vote(clone_payload(&mid)).unwrap();
        votes
            .pillar_votes_insert_vote(clone_payload(&high))
            .unwrap();

        let lookup = votes.pillar_votes_get_verified_votes(13, &high.block_hash, true);
        assert!(lookup.threshold_met);
        assert_eq!(lookup.block_weight, 8);
        assert_eq!(lookup.selected_weight, 7);
        assert_eq!(lookup.votes.len(), 2);
        assert_eq!(lookup.votes[0].vote_hash, high.vote_hash);
        assert_eq!(lookup.votes[1].vote_hash, mid.vote_hash);
    }

    #[test]
    fn above_threshold_lookup_returns_empty_until_threshold() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(14, 10));

        let first = vote(14, 16, 1, 0xD0, 4);
        let second = vote(14, 16, 2, 0xD1, 5);
        votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        votes
            .pillar_votes_insert_vote(clone_payload(&second))
            .unwrap();

        let lookup = votes.pillar_votes_get_verified_votes(14, &first.block_hash, true);
        assert!(!lookup.threshold_met);
        assert_eq!(lookup.block_weight, 9);
        assert_eq!(lookup.selected_weight, 0);
        assert!(lookup.votes.is_empty());
    }

    #[test]
    fn cleanup_votes_removes_only_stale_periods() {
        let mut votes = create_pillar_votes_index();
        for period in 20..23 {
            assert!(votes.pillar_votes_init_period_data(period, 1));
            votes
                .pillar_votes_insert_vote(vote(
                    period,
                    20,
                    period,
                    (period as u8).wrapping_add(0x10),
                    1,
                ))
                .unwrap();
        }

        votes.pillar_votes_cleanup_votes_by_period(22);

        assert!(votes
            .pillar_votes_insert_vote(vote(20, 20, 30, 0xE0, 1))
            .is_err());
        assert!(votes
            .pillar_votes_is_unique_vote(vote(22, 20, 22, 0xE2, 1))
            .is_ok());
        assert!(votes
            .pillar_votes_is_unique_vote(vote(22, 20, 23, 0xE3, 1))
            .is_ok());
        assert_eq!(
            votes
                .pillar_votes_snapshot_refs()
                .into_iter()
                .map(|vote| vote.weight)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn plain_payload_round_trips_vote_rlp_and_hash_fields() {
        let vote = PillarVote {
            period: 50,
            block_hash: H256::from_low_u64_be(99),
            signature: [0x11u8; 65],
        };
        let payload = PillarVotePayload {
            vote_hash: vote.hash(true).into(),
            block_hash: vote.block_hash.into(),
            voter: [5u8; 20],
            period: vote.period,
            weight: 7,
            vote_rlp: vote.encode_rlp(),
        };

        let decoded = payload_to_vote(clone_payload(&payload)).unwrap();
        assert_eq!(decoded.vote.period, payload.period);
        assert_eq!(decoded.voter, H160::from(payload.voter));
        assert_eq!(decoded.weight, payload.weight);
        assert_eq!(decoded.vote.encode_rlp(), payload.vote_rlp);
    }
}
