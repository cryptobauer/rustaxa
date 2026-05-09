use crate::ffi::rustaxa_ffi::{
    AtomicVoteInsertOutcome, DagHash, DetermineNewRoundOutcome, NetworkTPlusOneStepLookup,
    RoundMarkerSnapshot, ThresholdDecisionOutcome, TwoTPlusOneInsertOutcome,
    TwoTPlusOneSnapshotEntry, TwoTPlusOneVotedBlockLookup, TwoTPlusOneVotesLookup,
    UniqueVoterCheckOutcome, UniqueVoterInsertOutcome, VerifiedVotePayload,
    VotedValueInsertOutcome,
};
use crate::ffi::BridgeVerifiedVotes;
use ethereum_types::{H160, H256};
use rustaxa_consensus::verified_votes::{
    DetermineNewRoundOutcome as ConsensusDetermineNewRoundOutcome, PbftVoteType,
    ThresholdDecisionOutcome as ConsensusThresholdDecisionOutcome,
    TwoTPlusOneInsertOutcome as ConsensusTwoTPlusOneInsertOutcome, TwoTPlusOneVotedBlockType,
    VerifiedVote, VerifiedVotes,
};

/// Creates an empty Rust verified-votes index for the C++ vote-manager shim.
pub fn create_verified_votes_index() -> Box<BridgeVerifiedVotes> {
    Box::new(BridgeVerifiedVotes(VerifiedVotes::new()))
}

impl BridgeVerifiedVotes {
    /// Returns count of stored verified vote hashes.
    pub fn verified_votes_size(&self) -> u64 {
        self.0.size()
    }

    /// Checks unique-voter acceptance for `vote`.
    pub fn verified_votes_check_unique_voter(
        &self,
        vote: VerifiedVotePayload,
    ) -> Result<UniqueVoterCheckOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.0.check_unique_voter(&vote);
        Ok(UniqueVoterCheckOutcome {
            is_unique: outcome.is_unique,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
        })
    }

    /// Inserts `vote` into unique-voter tracking.
    pub fn verified_votes_insert_unique_voter(
        &mut self,
        vote: VerifiedVotePayload,
    ) -> Result<UniqueVoterInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.0.insert_unique_voter(&vote);
        Ok(UniqueVoterInsertOutcome {
            accepted: outcome.accepted,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
        })
    }

    /// Inserts `vote` into voted-value aggregation.
    pub fn verified_votes_insert_voted_value(
        &mut self,
        vote: VerifiedVotePayload,
    ) -> Result<VotedValueInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        Ok(self.0.insert_voted_value(vote)?.into())
    }

    /// Atomically inserts `vote` into unique-voter and voted-value state.
    ///
    /// This returns conflict details for slashing decisions when uniqueness
    /// fails and voted-value aggregation counters when insertion succeeds.
    pub fn verified_votes_insert_vote_atomic(
        &mut self,
        vote: VerifiedVotePayload,
    ) -> Result<AtomicVoteInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.0.insert_vote_atomic(vote)?;
        Ok(AtomicVoteInsertOutcome {
            inserted: outcome.inserted,
            total_weight: outcome.total_weight,
            votes_count: outcome.votes_count,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
        })
    }

    /// Applies deterministic threshold decisions to verified-votes state.
    ///
    /// The caller provides `total_weight` for vote's voted-value bucket and
    /// `two_t_plus_one_threshold` for this vote type/period.
    pub fn verified_votes_apply_threshold_decision(
        &mut self,
        vote: VerifiedVotePayload,
        total_weight: u64,
        two_t_plus_one_threshold: u64,
    ) -> Result<ThresholdDecisionOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome =
            self.0
                .apply_threshold_decision(&vote, total_weight, two_t_plus_one_threshold)?;
        Ok(outcome.into())
    }

    /// Returns whether exact `(period, round, step, block_hash, vote_hash)` exists.
    pub fn verified_votes_vote_in_verified_map(
        &self,
        period: u64,
        round: u64,
        step: u64,
        block_hash: &[u8; 32],
        vote_hash: &[u8; 32],
    ) -> bool {
        self.0.vote_in_verified_map(
            period,
            round,
            step,
            H256::from(*block_hash),
            H256::from(*vote_hash),
        )
    }

    /// Sets network t+1 step marker for one round.
    pub fn verified_votes_set_network_t_plus_one_step(
        &mut self,
        period: u64,
        round: u64,
        step: u64,
    ) -> bool {
        self.0.set_network_t_plus_one_step(period, round, step)
    }

    /// Returns network t+1 step marker for one round.
    pub fn verified_votes_get_network_t_plus_one_step(
        &self,
        period: u64,
        round: u64,
    ) -> NetworkTPlusOneStepLookup {
        self.0
            .network_t_plus_one_step(period, round)
            .map(|step| NetworkTPlusOneStepLookup { found: true, step })
            .unwrap_or(NetworkTPlusOneStepLookup {
                found: false,
                step: 0,
            })
    }

    /// Determines next round from Rust-owned next-vote 2t+1 mappings.
    pub fn verified_votes_determine_new_round(
        &self,
        period: u64,
        current_round: u64,
    ) -> DetermineNewRoundOutcome {
        self.0
            .determine_new_round(period, current_round)
            .map(Into::into)
            .unwrap_or(DetermineNewRoundOutcome {
                found: false,
                new_round: 0,
                source_round: 0,
                source_kind: 0,
                block_hash: [0u8; 32],
                step: 0,
            })
    }

    /// Inserts one 2t+1 voted-block mapping for existing round.
    pub fn verified_votes_insert_two_t_plus_one_voted_block(
        &mut self,
        period: u64,
        round: u64,
        kind: u8,
        block_hash: &[u8; 32],
        step: u64,
    ) -> Result<TwoTPlusOneInsertOutcome, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        Ok(self
            .0
            .insert_two_t_plus_one_voted_block(period, round, kind, H256::from(*block_hash), step)
            .into())
    }

    /// Gets one 2t+1 voted-block mapping.
    pub fn verified_votes_get_two_t_plus_one_voted_block(
        &self,
        period: u64,
        round: u64,
        kind: u8,
    ) -> Result<TwoTPlusOneVotedBlockLookup, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        Ok(self
            .0
            .get_two_t_plus_one_voted_block(period, round, kind)
            .map(|value| TwoTPlusOneVotedBlockLookup {
                found: true,
                block_hash: value.hash.into(),
                step: value.step,
            })
            .unwrap_or(TwoTPlusOneVotedBlockLookup {
                found: false,
                block_hash: [0u8; 32],
                step: 0,
            }))
    }

    /// Gets vote hashes for one mapped 2t+1 voted block.
    pub fn verified_votes_get_two_t_plus_one_voted_block_votes(
        &self,
        period: u64,
        round: u64,
        kind: u8,
    ) -> Result<TwoTPlusOneVotesLookup, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        let voted = self.0.get_two_t_plus_one_voted_block(period, round, kind);
        let Some(voted) = voted else {
            return Ok(TwoTPlusOneVotesLookup {
                found: false,
                block_hash: [0u8; 32],
                step: 0,
                vote_hashes: Vec::new(),
            });
        };

        let vote_hashes = self
            .0
            .get_two_t_plus_one_voted_block_vote_hashes(period, round, kind)
            .into_iter()
            .map(|hash| DagHash { hash: hash.into() })
            .collect();

        Ok(TwoTPlusOneVotesLookup {
            found: true,
            block_hash: voted.hash.into(),
            step: voted.step,
            vote_hashes,
        })
    }

    /// Removes periods lower than `pbft_period`.
    pub fn verified_votes_cleanup_votes_by_period(&mut self, pbft_period: u64) {
        self.0.cleanup_votes_by_period(pbft_period);
    }

    /// Returns deterministic flat vote snapshot.
    pub fn verified_votes_snapshot_votes(&self) -> Vec<VerifiedVotePayload> {
        self.0
            .snapshot_votes()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Returns deterministic 2t+1 mapping snapshot.
    pub fn verified_votes_snapshot_two_t_plus_one(&self) -> Vec<TwoTPlusOneSnapshotEntry> {
        self.0
            .snapshot_two_t_plus_one()
            .into_iter()
            .map(|entry| TwoTPlusOneSnapshotEntry {
                period: entry.period,
                round: entry.round,
                kind: entry.kind.into(),
                block_hash: entry.block_hash.into(),
                step: entry.step,
            })
            .collect()
    }

    /// Returns deterministic round marker snapshot.
    pub fn verified_votes_snapshot_round_markers(&self) -> Vec<RoundMarkerSnapshot> {
        self.0
            .snapshot_round_markers()
            .into_iter()
            .map(|entry| RoundMarkerSnapshot {
                period: entry.period,
                round: entry.round,
                network_t_plus_one_step: entry.network_t_plus_one_step,
            })
            .collect()
    }
}

fn payload_to_vote(value: VerifiedVotePayload) -> Result<VerifiedVote, anyhow::Error> {
    VerifiedVote::new(
        H256::from(value.vote_hash),
        H256::from(value.block_hash),
        H160::from(value.voter),
        value.period,
        value.round,
        value.step,
        PbftVoteType::try_from(value.vote_type)?,
        value.weight,
    )
}

impl From<rustaxa_consensus::verified_votes::VotedValueInsertOutcome> for VotedValueInsertOutcome {
    fn from(value: rustaxa_consensus::verified_votes::VotedValueInsertOutcome) -> Self {
        Self {
            inserted: value.inserted,
            total_weight: value.total_weight,
            votes_count: value.votes_count,
        }
    }
}

impl From<ConsensusTwoTPlusOneInsertOutcome> for TwoTPlusOneInsertOutcome {
    fn from(value: ConsensusTwoTPlusOneInsertOutcome) -> Self {
        Self {
            round_found: value.round_found,
            inserted: value.inserted,
        }
    }
}

impl From<ConsensusDetermineNewRoundOutcome> for DetermineNewRoundOutcome {
    fn from(value: ConsensusDetermineNewRoundOutcome) -> Self {
        Self {
            found: true,
            new_round: value.new_round,
            source_round: value.source_round,
            source_kind: value.source_kind.into(),
            block_hash: value.block_hash.into(),
            step: value.step,
        }
    }
}

impl From<ConsensusThresholdDecisionOutcome> for ThresholdDecisionOutcome {
    fn from(value: ConsensusThresholdDecisionOutcome) -> Self {
        let (kind_found, kind) = value
            .two_t_plus_one_kind
            .map(|kind| (true, kind.into()))
            .unwrap_or((false, 0));
        let (round_found, inserted) = value
            .two_t_plus_one_insert_outcome
            .map(|outcome| (outcome.round_found, outcome.inserted))
            .unwrap_or((false, false));

        Self {
            t_plus_one_reached: value.t_plus_one_reached,
            network_t_plus_one_step_updated: value.network_t_plus_one_step_updated,
            two_t_plus_one_reached: value.two_t_plus_one_reached,
            two_t_plus_one_kind_found: kind_found,
            two_t_plus_one_kind: kind,
            two_t_plus_one_round_found: round_found,
            two_t_plus_one_inserted: inserted,
        }
    }
}

impl From<VerifiedVote> for VerifiedVotePayload {
    fn from(value: VerifiedVote) -> Self {
        Self {
            vote_hash: value.vote_hash.into(),
            block_hash: value.block_hash.into(),
            voter: value.voter.into(),
            period: value.period,
            round: value.round,
            step: value.step,
            vote_type: value.vote_type.into(),
            weight: value.weight,
        }
    }
}
