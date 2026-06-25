//! CXX bridge wrapper for PBFT vote admission sessions.
//!
//! The admission bridge composes canonical vote event fact derivation with the
//! existing PBFT vote pipeline session. It keeps C++ as the executor for live
//! `PbftVote` sidecars, verified-vote mutation, storage, slashing, logging, and
//! network effects while Rust owns the deterministic admission ordering.

use crate::ffi::rustaxa_ffi::{
    PbftCanonicalVoteValidation as FfiPbftCanonicalVoteValidation,
    PbftVoteAdmissionExecutionPlan as FfiPbftVoteAdmissionExecutionPlan,
    PbftVoteAdmissionPrecheckPlan as FfiPbftVoteAdmissionPrecheckPlan,
    PbftVoteEventFactFlags as FfiPbftVoteEventFactFlags,
    PbftVotePipelineTransitionKey as FfiPbftVotePipelineTransitionKey,
    PbftVoteProgressContext as FfiPbftVoteProgressContext,
    PbftVoteProgressFact as FfiPbftVoteProgressFact,
    PbftVoteValidationExternalFacts as FfiPbftVoteValidationExternalFacts,
    VerifiedVoteAddOutcome as FfiVerifiedVoteAddOutcome, VerifiedVotePayload,
};
use crate::ffi::BridgePbftVoteAdmissionSession;
use crate::pbft_vote_progress::{
    add_outcome_to_domain, context_to_domain, error_code, execution_plan_to_ffi,
};
use anyhow::Result;
use rustaxa_consensus::pbft_vote_admission::{
    create_pbft_vote_admission_session as create_domain_pbft_vote_admission_session,
    create_pbft_vote_admission_session_from_validation as create_domain_pbft_vote_admission_session_from_validation,
    PbftVoteAdmissionExecution, PbftVoteAdmissionPrecheck, PbftVoteAdmissionSession,
};
use rustaxa_consensus::pbft_vote_event::PbftVoteEventFactFlags;
use rustaxa_consensus::pbft_vote_pipeline::{PbftVotePipelineStatus, PbftVotePipelineStep};
use rustaxa_consensus::pbft_vote_progress::{
    PbftVoteProgressFact, PbftVoteProgressIntent, PbftVoteProgressPlan, PbftVoteProgressStatus,
};
use rustaxa_consensus::pbft_vote_validation::{
    validate_canonical_pbft_vote, PbftVoteValidationExternalFacts,
};

/// Creates one Rust-owned PBFT vote admission session from canonical vote bytes.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes.
/// - `weight`: already-calculated validation weight used by today's
///   `VoteManager::addVerifiedVote` route.
/// - `flags`: caller-supplied ingress and validation flags.
/// - `context`: scalar vote-progress context.
///
/// Outputs:
/// - A session that owns event fact derivation plus the pipeline session when
///   the vote can progress to verified-vote insertion.
pub fn create_pbft_vote_admission_session(
    canonical_vote_rlp: &[u8],
    weight: u64,
    flags: FfiPbftVoteEventFactFlags,
    context: FfiPbftVoteProgressContext,
) -> Result<Box<BridgePbftVoteAdmissionSession>> {
    Ok(Box::new(BridgePbftVoteAdmissionSession {
        state: create_domain_pbft_vote_admission_session(
            canonical_vote_rlp,
            weight,
            flags_to_domain(flags),
            context_to_domain(&context),
        )?,
        context,
    }))
}

/// Creates one validation-backed PBFT vote admission session from canonical vote bytes.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes.
/// - `validation_facts`: FinalChain/key/VRF facts collected by the C++ shim.
/// - `flags`: ingress and stale-reward facts.
/// - `context`: scalar vote-progress context.
///
/// Outputs:
/// - A session that carries the full Rust validation result through precheck
///   and uses the Rust-calculated validation weight for any verified-vote
///   mutation request.
pub fn create_pbft_vote_admission_session_from_validation_facts(
    canonical_vote_rlp: &[u8],
    validation_facts: FfiPbftVoteValidationExternalFacts,
    flags: FfiPbftVoteEventFactFlags,
    context: FfiPbftVoteProgressContext,
) -> Result<Box<BridgePbftVoteAdmissionSession>> {
    let validation = validate_canonical_pbft_vote(
        canonical_vote_rlp,
        validation_facts_to_domain(validation_facts),
    )?;
    Ok(Box::new(BridgePbftVoteAdmissionSession {
        state: create_domain_pbft_vote_admission_session_from_validation(
            &validation,
            flags_to_domain(flags),
            context_to_domain(&context),
        ),
        context,
    }))
}

impl BridgePbftVoteAdmissionSession {
    /// Returns this session's pre-insert admission plan.
    pub fn pbft_vote_admission_precheck(&mut self) -> FfiPbftVoteAdmissionPrecheckPlan {
        precheck_to_ffi(self.state.precheck(), &self.state)
    }

    /// Reports the verified-vote insertion result and returns a terminal plan.
    pub fn pbft_vote_admission_complete(
        &mut self,
        add_vote_outcome: FfiVerifiedVoteAddOutcome,
    ) -> FfiPbftVoteAdmissionExecutionPlan {
        if !report_matches_session(&add_vote_outcome.vote, &self.state) {
            return invalid_report_execution(self);
        }

        let Ok(outcome) = add_outcome_to_domain(add_vote_outcome) else {
            return invalid_report_execution(self);
        };
        execution_to_ffi(self.state.report_verified_vote_add(outcome), self)
    }
}

fn flags_to_domain(value: FfiPbftVoteEventFactFlags) -> PbftVoteEventFactFlags {
    PbftVoteEventFactFlags {
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: value.valid_stale_reward_vote,
    }
}

fn validation_facts_to_domain(
    value: FfiPbftVoteValidationExternalFacts,
) -> PbftVoteValidationExternalFacts {
    PbftVoteValidationExternalFacts {
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
        has_preverified_weight: value.has_preverified_weight,
        preverified_weight: value.preverified_weight,
    }
}

fn report_matches_session(vote: &VerifiedVotePayload, session: &PbftVoteAdmissionSession) -> bool {
    let Some(fact) = session.progress_fact() else {
        return false;
    };

    vote.vote_hash == fact.identity.vote_hash.0
        && vote.block_hash == fact.identity.block_hash.0
        && vote.voter == fact.identity.voter.0
        && vote.period == fact.identity.period
        && vote.round == fact.identity.round
        && vote.step == fact.identity.step
        && vote.vote_type == u8::from(fact.vote_type)
        && vote.weight == fact.weight
}

fn transition_key(fact: Option<&PbftVoteProgressFact>) -> FfiPbftVotePipelineTransitionKey {
    let Some(fact) = fact else {
        return FfiPbftVotePipelineTransitionKey {
            vote_hash: [0; 32],
            period: 0,
            round: 0,
            step: 0,
            voter: [0; 20],
        };
    };

    FfiPbftVotePipelineTransitionKey {
        vote_hash: fact.identity.vote_hash.0,
        period: fact.identity.period,
        round: fact.identity.round,
        step: fact.identity.step,
        voter: fact.identity.voter.0,
    }
}

fn precheck_to_ffi(
    precheck: PbftVoteAdmissionPrecheck,
    session: &PbftVoteAdmissionSession,
) -> FfiPbftVoteAdmissionPrecheckPlan {
    let progress_fact = precheck.progress_fact;
    let (pipeline_status, progress_status, should_insert, has_threshold, threshold) = precheck
        .pipeline_step
        .as_ref()
        .map(precheck_pipeline_fields)
        .unwrap_or((
            0,
            PbftVoteProgressStatus::RejectedInvalidVote.as_u8(),
            false,
            false,
            0,
        ));

    FfiPbftVoteAdmissionPrecheckPlan {
        admission_status: precheck.admission_status.as_u8(),
        has_validation: precheck.validation.is_some(),
        validation: precheck
            .validation
            .clone()
            .map(FfiPbftCanonicalVoteValidation::from)
            .unwrap_or_else(empty_validation),
        event_status: precheck.event_status.as_u8(),
        pipeline_status,
        status: progress_status,
        error_code: precheck_error_code(&precheck).to_owned(),
        transition_key: transition_key(session.progress_fact()),
        has_progress_fact: progress_fact.is_some(),
        progress_fact: progress_fact
            .map(progress_fact_to_ffi)
            .unwrap_or_else(empty_progress_fact),
        should_insert_verified_vote: should_insert,
        has_two_t_plus_one_threshold: has_threshold,
        two_t_plus_one_threshold: threshold,
        complete: precheck.complete,
    }
}

fn precheck_pipeline_fields(step: &PbftVotePipelineStep) -> (u8, u8, bool, bool, u64) {
    let threshold = step
        .progress_plan
        .intents
        .iter()
        .find_map(|intent| match intent {
            PbftVoteProgressIntent::InsertVerifiedVote {
                two_t_plus_one_threshold,
                ..
            } => *two_t_plus_one_threshold,
            _ => None,
        });

    (
        step.pipeline_status.as_u8(),
        step.progress_plan.status.as_u8(),
        step.progress_plan.status == PbftVoteProgressStatus::PendingVerifiedVoteInsert,
        threshold.is_some(),
        threshold.unwrap_or_default(),
    )
}

fn precheck_error_code(precheck: &PbftVoteAdmissionPrecheck) -> &'static str {
    if !precheck.error_code.is_empty() {
        return precheck.error_code;
    }

    precheck
        .pipeline_step
        .as_ref()
        .map(|step| error_code(step.progress_plan.status))
        .unwrap_or_default()
}

fn execution_to_ffi(
    execution: PbftVoteAdmissionExecution,
    session: &BridgePbftVoteAdmissionSession,
) -> FfiPbftVoteAdmissionExecutionPlan {
    let progress = execution_plan_to_ffi(
        execution.pipeline_step.progress_plan,
        session
            .state
            .progress_fact()
            .copied()
            .unwrap_or_else(empty_domain_progress_fact),
        copy_context(&session.context),
    );
    FfiPbftVoteAdmissionExecutionPlan {
        admission_status: execution.admission_status.as_u8(),
        pipeline_status: execution.pipeline_step.pipeline_status.as_u8(),
        status: progress.status,
        error_code: progress.error_code,
        transition_key: transition_key(session.state.progress_fact()),
        accepted: progress.accepted,
        mark_vote_known: progress.mark_vote_known,
        mark_vote_known_hash: progress.mark_vote_known_hash,
        request_proposed_block_sidecar: progress.request_proposed_block_sidecar,
        proposed_block_sidecar_hash: progress.proposed_block_sidecar_hash,
        proposed_block_sidecar_period: progress.proposed_block_sidecar_period,
        gossip_vote: progress.gossip_vote,
        gossip_vote_hash: progress.gossip_vote_hash,
        report_slashing: progress.report_slashing,
        slashing_incoming_vote_hash: progress.slashing_incoming_vote_hash,
        slashing_conflicting_vote_hash: progress.slashing_conflicting_vote_hash,
        persist_extra_reward_vote: progress.persist_extra_reward_vote,
        extra_reward_vote_hash: progress.extra_reward_vote_hash,
        network_t_plus_one_step_updated: progress.network_t_plus_one_step_updated,
        drive_pbft_progress: progress.drive_pbft_progress,
        progress_period: progress.progress_period,
        progress_round: progress.progress_round,
        persist_two_t_plus_one_votes: progress.persist_two_t_plus_one_votes,
        two_t_plus_one_kind: progress.two_t_plus_one_kind,
        two_t_plus_one_period: progress.two_t_plus_one_period,
        two_t_plus_one_round: progress.two_t_plus_one_round,
        two_t_plus_one_step: progress.two_t_plus_one_step,
        two_t_plus_one_block_hash: progress.two_t_plus_one_block_hash,
        complete: execution.complete,
    }
}

fn invalid_report_execution(
    session: &BridgePbftVoteAdmissionSession,
) -> FfiPbftVoteAdmissionExecutionPlan {
    let execution = PbftVoteAdmissionExecution {
        admission_status: rustaxa_consensus::PbftVoteAdmissionStatus::InvalidStage,
        pipeline_step: PbftVotePipelineStep {
            pipeline_status: PbftVotePipelineStatus::InvalidStage,
            progress_plan: PbftVoteProgressPlan {
                status: PbftVoteProgressStatus::RejectedExecutorReport,
                intents: Vec::new(),
                add_vote_outcome: None,
                threshold_decision: None,
                conflicting_vote_hash: None,
            },
            complete: true,
        },
        complete: true,
    };
    let mut result = execution_to_ffi(execution, session);
    result.error_code = "PBFT_VOTE_ADMISSION_INVALID_EXECUTOR_REPORT".to_owned();
    result
}

fn progress_fact_to_ffi(value: PbftVoteProgressFact) -> FfiPbftVoteProgressFact {
    FfiPbftVoteProgressFact {
        vote: VerifiedVotePayload {
            vote_hash: value.identity.vote_hash.into(),
            block_hash: value.identity.block_hash.into(),
            voter: value.identity.voter.0,
            period: value.identity.period,
            round: value.identity.round,
            step: value.identity.step,
            vote_type: value.vote_type.into(),
            weight: value.weight,
        },
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: value.valid_stale_reward_vote,
    }
}

fn empty_progress_fact() -> FfiPbftVoteProgressFact {
    progress_fact_to_ffi(empty_domain_progress_fact())
}

fn empty_validation() -> FfiPbftCanonicalVoteValidation {
    FfiPbftCanonicalVoteValidation {
        status: 0,
        error_code: String::new(),
        accepted: false,
        rejected: false,
        mark_validated_replay: false,
        vote_hash: [0; 32],
        signing_hash: [0; 32],
        block_hash: [0; 32],
        period: 0,
        round: 0,
        step: 0,
        vote_type: 0,
        recovered_voter: [0; 20],
        recovered_public_key: [0; 64],
        signature_valid: false,
        vrf_valid: false,
        has_sortition_threshold: false,
        sortition_threshold: 0,
        weight_calculated: false,
        calculated_weight: 0,
        vrf_output: [0; 64],
    }
}

fn empty_domain_progress_fact() -> PbftVoteProgressFact {
    PbftVoteProgressFact {
        identity: rustaxa_consensus::PbftVoteIdentity {
            vote_hash: [0; 32].into(),
            block_hash: [0; 32].into(),
            period: 0,
            round: 0,
            step: 0,
            voter: [0; 20].into(),
        },
        vote_type: rustaxa_consensus::verified_votes::PbftVoteType::Soft,
        weight: 0,
        vote_already_known: false,
        carries_proposed_block: false,
        valid_stale_reward_vote: false,
    }
}

fn copy_context(value: &FfiPbftVoteProgressContext) -> FfiPbftVoteProgressContext {
    FfiPbftVoteProgressContext {
        current_period: value.current_period,
        current_round: value.current_round,
        max_future_period_delta: value.max_future_period_delta,
        has_two_t_plus_one_threshold: value.has_two_t_plus_one_threshold,
        two_t_plus_one_threshold: value.two_t_plus_one_threshold,
        require_proposed_block_sidecar: value.require_proposed_block_sidecar,
        slashing_enabled: value.slashing_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftVoteGenerationInput;
    use crate::pbft_vote_generation::pbft_generate_signed_vote;
    use k256::ecdsa::SigningKey;

    const NODE_SECRET: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn flags() -> FfiPbftVoteEventFactFlags {
        FfiPbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    fn context() -> FfiPbftVoteProgressContext {
        FfiPbftVoteProgressContext {
            current_period: 12,
            current_round: 2,
            max_future_period_delta: 0,
            has_two_t_plus_one_threshold: true,
            two_t_plus_one_threshold: 10,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = tiny_keccak::Keccak::v256();
        tiny_keccak::Hasher::update(&mut hasher, &public_key.as_bytes()[1..]);
        tiny_keccak::Hasher::finalize(hasher, &mut output);
        output[12..].try_into().unwrap()
    }

    fn vote_rlp() -> Vec<u8> {
        let generated = pbft_generate_signed_vote(PbftVoteGenerationInput {
            block_hash: [7; 32],
            vote_type: 3,
            period: 12,
            round: 2,
            step: 3,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET),
            expected_vrf_public_key: rustaxa_vdf::vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        assert!(generated.accepted);
        generated.vote_rlp.into_iter().collect()
    }

    fn validation_facts() -> FfiPbftVoteValidationExternalFacts {
        FfiPbftVoteValidationExternalFacts {
            voter_dpos_ready: true,
            voter_dpos_vote_count: 42,
            total_dpos_ready: true,
            total_dpos_vote_count: 100,
            future_dpos_state: false,
            unknown_error: false,
            vrf_key_ready: true,
            has_vrf_key: true,
            vrf_public_key: rustaxa_vdf::vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            strict_vrf: true,
            committee_size: 100,
            number_of_proposers: 20,
            has_preverified_weight: false,
            preverified_weight: 0,
        }
    }

    fn add_outcome(vote: VerifiedVotePayload) -> FfiVerifiedVoteAddOutcome {
        FfiVerifiedVoteAddOutcome {
            vote,
            inserted: true,
            total_weight: 42,
            votes_count: 1,
            conflict_found: false,
            conflicting_vote_hash: [0; 32],
            used_secondary_slot: false,
            duplicate_vote_hash: false,
            threshold_applied: false,
            t_plus_one_reached: false,
            network_t_plus_one_step_updated: false,
            two_t_plus_one_reached: false,
            two_t_plus_one_kind_found: false,
            two_t_plus_one_kind: 0,
            two_t_plus_one_round_found: false,
            two_t_plus_one_inserted: false,
        }
    }

    #[test]
    fn admission_bridge_precheck_and_complete_share_transition_key() {
        let mut session =
            create_pbft_vote_admission_session(&vote_rlp(), 42, flags(), context()).unwrap();
        let precheck = session.pbft_vote_admission_precheck();

        assert_eq!(precheck.admission_status, 2);
        assert_eq!(precheck.event_status, 0);
        assert_eq!(precheck.pipeline_status, 1);
        assert!(precheck.has_progress_fact);
        assert!(precheck.should_insert_verified_vote);
        assert_eq!(precheck.progress_fact.vote.weight, 42);

        let execution =
            session.pbft_vote_admission_complete(add_outcome(precheck.progress_fact.vote));
        assert_eq!(execution.admission_status, 3);
        assert_eq!(
            execution.transition_key.vote_hash,
            precheck.transition_key.vote_hash
        );
        assert!(execution.accepted);
    }

    #[test]
    fn admission_bridge_validation_backed_precheck_carries_validation_result() {
        let mut session = create_pbft_vote_admission_session_from_validation_facts(
            &vote_rlp(),
            validation_facts(),
            flags(),
            context(),
        )
        .unwrap();
        let precheck = session.pbft_vote_admission_precheck();

        assert_eq!(precheck.admission_status, 2);
        assert_eq!(precheck.event_status, 0);
        assert_eq!(precheck.validation.status, 1);
        assert!(precheck.has_validation);
        assert!(precheck.validation.accepted);
        assert_eq!(precheck.validation.calculated_weight, 42);
        assert_eq!(precheck.progress_fact.vote.weight, 42);
        assert_eq!(
            precheck.validation.vote_hash,
            precheck.progress_fact.vote.vote_hash
        );
    }

    #[test]
    fn admission_bridge_rejects_mismatched_executor_report() {
        let mut session =
            create_pbft_vote_admission_session(&vote_rlp(), 42, flags(), context()).unwrap();
        let precheck = session.pbft_vote_admission_precheck();

        let mut vote = precheck.progress_fact.vote;
        vote.vote_hash = [9; 32];
        let execution = session.pbft_vote_admission_complete(add_outcome(vote));

        assert_eq!(execution.admission_status, 4);
        assert_eq!(
            execution.error_code,
            "PBFT_VOTE_ADMISSION_INVALID_EXECUTOR_REPORT"
        );
        assert!(!execution.accepted);
    }

    #[test]
    fn admission_bridge_event_rejection_does_not_request_insert() {
        let mut session =
            create_pbft_vote_admission_session(&[0x01, 0x02], 42, flags(), context()).unwrap();
        let precheck = session.pbft_vote_admission_precheck();

        assert_eq!(precheck.admission_status, 1);
        assert_eq!(precheck.event_status, 1);
        assert!(!precheck.has_progress_fact);
        assert!(!precheck.should_insert_verified_vote);
        assert!(precheck.complete);
    }
}
