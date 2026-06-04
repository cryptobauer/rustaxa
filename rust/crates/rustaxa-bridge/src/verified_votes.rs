use crate::ffi::rustaxa_ffi::{
    AtomicVoteInsertOutcome, DagHash, DetermineNewRoundOutcome, NetworkTPlusOneStepLookup,
    PbftTwoTPlusOneVoteBundle, PbftVoteAdmissionRuntimeResult, PbftVoteEventFactFlags,
    PbftVoteProgressContext as FfiPbftVoteProgressContext, PbftVoteStorageRecord,
    PbftVoteValidationExternalFacts, RoundMarkerSnapshot, ThresholdDecisionOutcome,
    TwoTPlusOneInsertOutcome, TwoTPlusOneSnapshotEntry, TwoTPlusOneVotedBlockLookup,
    TwoTPlusOneVotesLookup, UniqueVoterCheckOutcome, UniqueVoterInsertOutcome,
    VerifiedStepVotesEntry, VerifiedStepVotesLookup,
    VerifiedVoteAddOutcome as FfiVerifiedVoteAddOutcome, VerifiedVotePayload,
    VotedValueInsertOutcome,
};
use crate::ffi::BridgeVerifiedVotes;
use crate::pbft_vote_progress::{context_to_domain, execution_plan_to_ffi};
use ethereum_types::{H160, H256};
use rustaxa_consensus::pbft_vote_event::PbftVoteEventFactFlags as DomainPbftVoteEventFactFlags;
use rustaxa_consensus::pbft_vote_validation::{
    validate_canonical_pbft_vote, PbftCanonicalVoteValidation,
    PbftVoteValidationExternalFacts as DomainPbftVoteValidationExternalFacts,
};
use rustaxa_consensus::verified_votes::{
    AddVerifiedVoteOutcome as ConsensusAddVerifiedVoteOutcome,
    DetermineNewRoundOutcome as ConsensusDetermineNewRoundOutcome, PbftVoteType,
    ThresholdDecisionOutcome as ConsensusThresholdDecisionOutcome,
    TwoTPlusOneInsertOutcome as ConsensusTwoTPlusOneInsertOutcome, TwoTPlusOneVotedBlockType,
    VerifiedVote,
};
use rustaxa_consensus::{PbftVoteAdmissionRuntime, PbftVoteRuntimeAdmissionOutcome};

/// Creates an empty Rust verified-votes index for the C++ vote-manager shim.
pub fn create_verified_votes_index() -> Box<BridgeVerifiedVotes> {
    Box::new(BridgeVerifiedVotes(PbftVoteAdmissionRuntime::new()))
}

impl BridgeVerifiedVotes {
    /// Returns count of stored verified vote hashes.
    pub fn verified_votes_size(&self) -> u64 {
        self.0.verified_votes().size()
    }

    /// Checks unique-voter acceptance for `vote`.
    pub fn verified_votes_check_unique_voter(
        &self,
        vote: VerifiedVotePayload,
    ) -> Result<UniqueVoterCheckOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.0.verified_votes().check_unique_voter(&vote);
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
        let outcome = self.0.verified_votes_mut().insert_unique_voter(&vote);
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
        Ok(self.0.verified_votes_mut().insert_voted_value(vote)?.into())
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
        let outcome = self.0.verified_votes_mut().insert_vote_atomic(vote)?;
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
        let outcome = self.0.verified_votes_mut().apply_threshold_decision(
            &vote,
            total_weight,
            two_t_plus_one_threshold,
        )?;
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
        self.0.verified_votes().vote_in_verified_map(
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
        self.0
            .verified_votes_mut()
            .set_network_t_plus_one_step(period, round, step)
    }

    /// Returns network t+1 step marker for one round.
    pub fn verified_votes_get_network_t_plus_one_step(
        &self,
        period: u64,
        round: u64,
    ) -> NetworkTPlusOneStepLookup {
        self.0
            .verified_votes()
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
            .verified_votes()
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
            .verified_votes_mut()
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
            .verified_votes()
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
        let voted = self
            .0
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind);
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
            .verified_votes()
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

    /// Returns all voted values and their vote hashes for one step.
    pub fn verified_votes_get_step_votes(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> VerifiedStepVotesLookup {
        let Some(step_votes) = self.0.verified_votes().get_step_votes(period, round, step) else {
            return VerifiedStepVotesLookup {
                found: false,
                entries: Vec::new(),
            };
        };

        VerifiedStepVotesLookup {
            found: true,
            entries: step_votes
                .into_iter()
                .map(|entry| VerifiedStepVotesEntry {
                    block_hash: entry.block_hash.into(),
                    total_weight: entry.total_weight,
                    vote_hashes: entry
                        .vote_hashes
                        .into_iter()
                        .map(|vote_hash| DagHash {
                            hash: vote_hash.into(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Adds one vote fact with optional threshold side effects.
    pub fn verified_votes_add_verified_vote(
        &mut self,
        vote: VerifiedVotePayload,
        two_t_plus_one_threshold: u64,
        apply_threshold_decision: bool,
    ) -> Result<FfiVerifiedVoteAddOutcome, anyhow::Error> {
        let report_vote = copy_vote_payload(&vote);
        let vote = payload_to_vote(vote)?;
        let threshold = if apply_threshold_decision {
            Some(two_t_plus_one_threshold)
        } else {
            None
        };

        let outcome = self
            .0
            .verified_votes_mut()
            .add_verified_vote(vote, threshold)?;
        Ok(outcome_to_ffi_add_vote_outcome(report_vote, outcome))
    }

    /// Runs one validation-backed PBFT vote admission transition.
    ///
    /// Rust validates canonical vote bytes from caller-supplied external facts,
    /// records vote payload sidecars, mutates the single Rust verified-vote
    /// index owned by this bridge handle, and returns explicit executor
    /// effects for the C++ VoteManager shim.
    pub fn verified_votes_admit_validated_vote(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation_facts: PbftVoteValidationExternalFacts,
        flags: PbftVoteEventFactFlags,
        context: FfiPbftVoteProgressContext,
    ) -> Result<PbftVoteAdmissionRuntimeResult, anyhow::Error> {
        let validation = validate_canonical_pbft_vote(
            canonical_vote_rlp,
            validation_facts_to_domain(validation_facts),
        )?;
        let outcome = self.0.admit_validated_vote(
            canonical_vote_rlp,
            &validation,
            flags_to_domain(flags),
            context_to_domain(&context),
        )?;
        Ok(runtime_outcome_to_ffi(validation, outcome, context))
    }

    /// Removes periods lower than `pbft_period`.
    pub fn verified_votes_cleanup_votes_by_period(&mut self, pbft_period: u64) {
        self.0.cleanup_votes_by_period(pbft_period);
    }

    /// Returns deterministic flat vote snapshot.
    pub fn verified_votes_snapshot_votes(&self) -> Vec<VerifiedVotePayload> {
        self.0
            .verified_votes()
            .snapshot_votes()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Returns deterministic 2t+1 mapping snapshot.
    pub fn verified_votes_snapshot_two_t_plus_one(&self) -> Vec<TwoTPlusOneSnapshotEntry> {
        self.0
            .verified_votes()
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
            .verified_votes()
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

fn copy_vote_payload(value: &VerifiedVotePayload) -> VerifiedVotePayload {
    VerifiedVotePayload {
        vote_hash: value.vote_hash,
        block_hash: value.block_hash,
        voter: value.voter,
        period: value.period,
        round: value.round,
        step: value.step,
        vote_type: value.vote_type,
        weight: value.weight,
    }
}

fn flags_to_domain(value: PbftVoteEventFactFlags) -> DomainPbftVoteEventFactFlags {
    DomainPbftVoteEventFactFlags {
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: value.valid_stale_reward_vote,
    }
}

fn validation_facts_to_domain(
    value: PbftVoteValidationExternalFacts,
) -> DomainPbftVoteValidationExternalFacts {
    DomainPbftVoteValidationExternalFacts {
        voter_dpos_ready: value.voter_dpos_ready,
        voter_dpos_vote_count: value.voter_dpos_vote_count,
        total_dpos_ready: value.total_dpos_ready,
        total_dpos_vote_count: value.total_dpos_vote_count,
        future_dpos_state: value.future_dpos_state,
        unknown_error: value.unknown_error,
        vrf_key_ready: value.vrf_key_ready,
        has_vrf_key: value.has_vrf_key,
        vrf_public_key: value.vrf_public_key,
        strict_vrf: value.strict_vrf,
        committee_size: value.committee_size,
        number_of_proposers: value.number_of_proposers,
    }
}

fn runtime_outcome_to_ffi(
    validation: PbftCanonicalVoteValidation,
    outcome: PbftVoteRuntimeAdmissionOutcome,
    context: FfiPbftVoteProgressContext,
) -> PbftVoteAdmissionRuntimeResult {
    let progress_fact = outcome.precheck.progress_fact;
    let empty_vote = empty_vote_payload();
    let vote = progress_fact
        .map(progress_fact_to_vote)
        .unwrap_or(empty_vote);
    let progress = outcome.execution.as_ref().map(|execution| {
        execution_plan_to_ffi(
            execution.pipeline_step.progress_plan.clone(),
            progress_fact.unwrap_or_else(empty_domain_progress_fact),
            context,
        )
    });
    let status = progress
        .as_ref()
        .map(|progress| progress.status)
        .unwrap_or(outcome.precheck.event_status.as_u8());
    let error_code = progress
        .as_ref()
        .map(|progress| progress.error_code.clone())
        .unwrap_or_else(|| outcome.precheck.error_code.to_owned());
    let add_outcome = outcome
        .add_outcome
        .map(|add| outcome_to_ffi_add_vote_outcome(copy_vote_payload(&vote), add));
    let storage_vote = outcome
        .storage_vote
        .map(Into::into)
        .unwrap_or_else(empty_storage_record);
    let extra_reward_vote = if progress
        .as_ref()
        .map(|progress| progress.persist_extra_reward_vote)
        .unwrap_or(false)
    {
        PbftVoteStorageRecord {
            hash: storage_vote.hash,
            vote_rlp: storage_vote.vote_rlp.clone(),
        }
    } else {
        empty_storage_record()
    };
    let two_t_plus_one_bundle = if let (Some(progress), Some(bundle)) =
        (progress.as_ref(), outcome.two_t_plus_one_bundle)
    {
        PbftTwoTPlusOneVoteBundle {
            kind: progress.two_t_plus_one_kind,
            period: progress.two_t_plus_one_period,
            round: progress.two_t_plus_one_round,
            step: progress.two_t_plus_one_step,
            block_hash: progress.two_t_plus_one_block_hash,
            votes_bundle_rlp: bundle.votes_bundle_rlp,
        }
    } else {
        empty_two_t_plus_one_bundle()
    };
    let (slashing_incoming_vote, slashing_conflicting_vote) =
        if let Some(payloads) = outcome.slashing_payloads {
            (payloads.incoming.into(), payloads.conflicting.into())
        } else {
            (empty_storage_record(), empty_storage_record())
        };

    PbftVoteAdmissionRuntimeResult {
        status,
        error_code,
        accepted: progress
            .as_ref()
            .map(|progress| progress.accepted)
            .unwrap_or(false),
        rejected: !validation.accepted,
        has_validation: true,
        replay_inserted: validation.mark_validated_replay,
        validation: validation.into(),
        has_vote: progress_fact.is_some(),
        vote,
        has_verified_vote_add: add_outcome.is_some(),
        verified_vote_add: add_outcome.unwrap_or_else(|| {
            outcome_to_ffi_add_vote_outcome(empty_vote_payload(), empty_add_outcome())
        }),
        has_storage_vote: storage_vote.hash != [0; 32],
        storage_vote,
        persist_extra_reward_vote: progress
            .as_ref()
            .map(|progress| progress.persist_extra_reward_vote)
            .unwrap_or(false),
        extra_reward_vote,
        persist_two_t_plus_one_votes: progress
            .as_ref()
            .map(|progress| progress.persist_two_t_plus_one_votes)
            .unwrap_or(false),
        two_t_plus_one_bundle,
        report_slashing: progress
            .as_ref()
            .map(|progress| progress.report_slashing)
            .unwrap_or(false),
        slashing_incoming_vote,
        slashing_conflicting_vote,
        network_t_plus_one_step_updated: progress
            .as_ref()
            .map(|progress| progress.network_t_plus_one_step_updated)
            .unwrap_or(false),
        drive_pbft_progress: progress
            .as_ref()
            .map(|progress| progress.drive_pbft_progress)
            .unwrap_or(false),
        progress_period: progress
            .as_ref()
            .map(|progress| progress.progress_period)
            .unwrap_or_default(),
        progress_round: progress
            .as_ref()
            .map(|progress| progress.progress_round)
            .unwrap_or_default(),
    }
}

fn progress_fact_to_vote(value: rustaxa_consensus::PbftVoteProgressFact) -> VerifiedVotePayload {
    VerifiedVotePayload {
        vote_hash: value.identity.vote_hash.0,
        block_hash: value.identity.block_hash.0,
        voter: value.identity.voter.0,
        period: value.identity.period,
        round: value.identity.round,
        step: value.identity.step,
        vote_type: value.vote_type.into(),
        weight: value.weight,
    }
}

fn empty_domain_progress_fact() -> rustaxa_consensus::PbftVoteProgressFact {
    rustaxa_consensus::PbftVoteProgressFact {
        identity: rustaxa_consensus::PbftVoteIdentity {
            vote_hash: [0; 32].into(),
            block_hash: [0; 32].into(),
            period: 0,
            round: 0,
            step: 0,
            voter: [0; 20].into(),
        },
        vote_type: PbftVoteType::Soft,
        weight: 0,
        vote_already_known: false,
        carries_proposed_block: false,
        valid_stale_reward_vote: false,
    }
}

fn empty_vote_payload() -> VerifiedVotePayload {
    VerifiedVotePayload {
        vote_hash: [0; 32],
        block_hash: [0; 32],
        voter: [0; 20],
        period: 0,
        round: 0,
        step: 0,
        vote_type: 0,
        weight: 0,
    }
}

fn empty_add_outcome() -> ConsensusAddVerifiedVoteOutcome {
    ConsensusAddVerifiedVoteOutcome {
        inserted: false,
        total_weight: 0,
        votes_count: 0,
        conflicting_vote_hash: None,
        used_secondary_slot: false,
        duplicate_vote_hash: false,
        threshold_decision: None,
    }
}

fn empty_storage_record() -> PbftVoteStorageRecord {
    PbftVoteStorageRecord {
        hash: [0; 32],
        vote_rlp: Vec::new(),
    }
}

fn empty_two_t_plus_one_bundle() -> PbftTwoTPlusOneVoteBundle {
    PbftTwoTPlusOneVoteBundle {
        kind: 0,
        period: 0,
        round: 0,
        step: 0,
        block_hash: [0; 32],
        votes_bundle_rlp: Vec::new(),
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

fn outcome_to_ffi_add_vote_outcome(
    vote: VerifiedVotePayload,
    value: ConsensusAddVerifiedVoteOutcome,
) -> FfiVerifiedVoteAddOutcome {
    let (
        threshold_applied,
        t_plus_one_reached,
        network_t_plus_one_step_updated,
        two_t_plus_one_reached,
        two_t_plus_one_kind_found,
        two_t_plus_one_kind,
        two_t_plus_one_round_found,
        two_t_plus_one_inserted,
    ) = value
        .threshold_decision
        .map(|threshold| {
            (
                true,
                threshold.t_plus_one_reached,
                threshold.network_t_plus_one_step_updated,
                threshold.two_t_plus_one_reached,
                threshold.two_t_plus_one_kind.is_some(),
                threshold.two_t_plus_one_kind.map(Into::into).unwrap_or(0),
                threshold
                    .two_t_plus_one_insert_outcome
                    .map(|outcome| outcome.round_found)
                    .unwrap_or(false),
                threshold
                    .two_t_plus_one_insert_outcome
                    .map(|outcome| outcome.inserted)
                    .unwrap_or(false),
            )
        })
        .unwrap_or((false, false, false, false, false, 0u8, false, false));

    FfiVerifiedVoteAddOutcome {
        vote,
        inserted: value.inserted,
        total_weight: value.total_weight,
        votes_count: value.votes_count,
        conflict_found: value.conflicting_vote_hash.is_some(),
        conflicting_vote_hash: value.conflicting_vote_hash.unwrap_or_default().into(),
        used_secondary_slot: value.used_secondary_slot,
        duplicate_vote_hash: value.duplicate_vote_hash,
        threshold_applied,
        t_plus_one_reached,
        network_t_plus_one_step_updated,
        two_t_plus_one_reached,
        two_t_plus_one_kind_found,
        two_t_plus_one_kind,
        two_t_plus_one_round_found,
        two_t_plus_one_inserted,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(id: u64) -> [u8; 32] {
        H256::from_low_u64_be(id).into()
    }

    fn address(id: u64) -> [u8; 20] {
        H160::from_low_u64_be(id).into()
    }

    fn payload(
        vote_hash: u64,
        block_hash: u64,
        voter: u64,
        step: u64,
        weight: u64,
    ) -> VerifiedVotePayload {
        VerifiedVotePayload {
            vote_hash: hash(vote_hash),
            block_hash: hash(block_hash),
            voter: address(voter),
            period: 3,
            round: 2,
            step,
            vote_type: PbftVoteType::Next.into(),
            weight,
        }
    }

    #[test]
    fn bridge_add_verified_vote_reports_threshold_and_step_snapshot() {
        let mut votes = create_verified_votes_index();

        let first = votes
            .verified_votes_add_verified_vote(payload(1, 44, 1, 5, 3), 5, true)
            .expect("first vote is accepted");
        assert!(first.inserted);
        assert!(first.threshold_applied);
        assert!(first.t_plus_one_reached);
        assert!(!first.two_t_plus_one_reached);

        let second = votes
            .verified_votes_add_verified_vote(payload(2, 44, 2, 5, 2), 5, true)
            .expect("second vote is accepted");
        assert!(second.inserted);
        assert!(second.two_t_plus_one_reached);
        assert!(second.two_t_plus_one_kind_found);
        assert!(second.two_t_plus_one_inserted);

        let step_votes = votes.verified_votes_get_step_votes(3, 2, 5);
        assert!(step_votes.found);
        assert_eq!(step_votes.entries.len(), 1);
        assert_eq!(step_votes.entries[0].block_hash, hash(44));
        assert_eq!(step_votes.entries[0].total_weight, 5);
        assert_eq!(step_votes.entries[0].vote_hashes.len(), 2);
    }
}
