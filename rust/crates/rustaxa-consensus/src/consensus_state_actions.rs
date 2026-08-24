//! Application-root composition for PBFT filter, certify, and finish actions.
//!
//! This module translates live native consensus state into operation-shaped
//! vote tasks. Manager plans and sidecar sessions remain implementation details;
//! callers receive canonical block bytes plus the exact vote family and durable
//! follow-up that must occur only after every local signer has been processed.

use anyhow::{Result, ensure};
use ethereum_types::H256;

use crate::FinalChain;
use crate::dag_transaction_service::DagTransactionService;
use crate::pbft_leader_selection::{PbftComposedLeaderSelectionRequest, PbftLeaderSelectionStatus};
use crate::pbft_manager::{
    PbftManagerRuntimeStateCode, PbftManagerStateActionFact, PbftManagerStateActionIntent,
    PbftManagerStateActionStatus, plan_pbft_manager_state_action,
};
use crate::pbft_service::{
    PbftProposedBlockAdmissionRequest, PbftProposedBlockAdmissionStatus, PbftService,
};
use crate::pbft_vote_validation::inspect_canonical_pbft_vote;
use crate::verified_votes::{PbftVoteType, TwoTPlusOneVotedBlockType};

/// One application-root state-action request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsensusStateActionRequest {
    /// Milliseconds elapsed since the current round began.
    pub round_elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConsensusStateActionPolicy {
    pbft_gas_limit: u64,
    extra_data_required: bool,
    pillar_block_required: bool,
}

/// Durable publication required after a generated vote is admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsensusStateVoteCommit {
    None,
    SaveCertVotedBlock,
    MarkNextVotedValue,
    MarkNextVotedNull,
}

/// Canonical target for local vote generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusStateVoteTask {
    pub period: u64,
    pub round: u64,
    pub step: u64,
    pub vote_type: PbftVoteType,
    pub block_hash: H256,
    /// Canonical signed PBFT block bytes. Null votes intentionally carry none.
    pub proposed_block_rlp: Vec<u8>,
    pub commit: ConsensusStateVoteCommit,
}

/// Complete operation-shaped state-action result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusStateActionBatch {
    pub state: PbftManagerRuntimeStateCode,
    pub votes: Vec<ConsensusStateVoteTask>,
    pub go_finish_state: bool,
    pub loop_back_finish_state: bool,
}

/// Composes one filter, certify, first-finish, or second-finish action.
///
/// Vote quorum lookups, leader selection, proposed-block admission, and
/// cert-recovery loading occur behind the native root. Missing or rejected live
/// candidates are valid no-work outcomes. Unknown states and malformed planner
/// facts fail without returning partial work.
pub fn compose_consensus_state_action(
    service: &PbftService,
    final_chain: &FinalChain,
    dag_transaction: &DagTransactionService,
    request: ConsensusStateActionRequest,
) -> Result<ConsensusStateActionBatch> {
    let (snapshot, deadline_ms, polling_interval_ms) = {
        let manager = service.manager_state();
        let snapshot = manager.state.snapshot();
        let (deadline_ms, polling_interval_ms) = manager.state.state_action_timing_policy();
        (snapshot, deadline_ms, polling_interval_ms)
    };
    ensure!(
        matches!(
            snapshot.state,
            PbftManagerRuntimeStateCode::Filter
                | PbftManagerRuntimeStateCode::Certify
                | PbftManagerRuntimeStateCode::Finish
                | PbftManagerRuntimeStateCode::FinishPolling
        ),
        "CONSENSUS_STATE_ACTION_UNSUPPORTED_STATE"
    );
    let (ficus_activation_period, pillar_blocks_interval) = service.pillar_schedule();
    let first_pillar_period = if ficus_activation_period == 0 {
        pillar_blocks_interval
    } else {
        ficus_activation_period
    };
    let policy = ConsensusStateActionPolicy {
        pbft_gas_limit: service.pbft_gas_limit_for_period(snapshot.period),
        extra_data_required: ficus_activation_period != u64::MAX
            && snapshot.period >= ficus_activation_period,
        pillar_block_required: ficus_activation_period != u64::MAX
            && snapshot.period >= first_pillar_period
            && snapshot.period % pillar_blocks_interval == 1,
    };

    let previous_round = snapshot.round.saturating_sub(1);
    let previous_next_null = if snapshot.round >= 2 {
        service.verified_votes_get_two_t_plus_one_voted_block(
            snapshot.period,
            previous_round,
            TwoTPlusOneVotedBlockType::NextVotedNullBlock,
        )?
    } else {
        None
    };
    let previous_next_value = if snapshot.round >= 2 {
        service.verified_votes_get_two_t_plus_one_voted_block(
            snapshot.period,
            previous_round,
            TwoTPlusOneVotedBlockType::NextVotedBlock,
        )?
    } else {
        None
    };
    let current_soft = service.verified_votes_get_two_t_plus_one_voted_block(
        snapshot.period,
        snapshot.round,
        TwoTPlusOneVotedBlockType::SoftVotedBlock,
    )?;

    let plan = plan_pbft_manager_state_action(PbftManagerStateActionFact {
        state: snapshot.state,
        period: snapshot.period,
        round: snapshot.round,
        step: snapshot.step,
        elapsed_round_ms: request.round_elapsed_ms,
        deadline_ms,
        current_round_lambda_ms: snapshot.current_round_lambda_ms,
        polling_interval_ms,
        has_previous_round_next_null: previous_next_null.is_some(),
        has_previous_round_next_value: previous_next_value.is_some(),
        previous_round_next_value_hash: previous_next_value
            .as_ref()
            .map_or([0; 32], |value| value.block_hash.0),
        has_current_round_soft_value: current_soft.is_some(),
        current_round_soft_value_hash: current_soft
            .as_ref()
            .map_or([0; 32], |value| value.block_hash.0),
        has_cert_voted_block: snapshot.has_cert_voted_block,
        cert_voted_block_hash: snapshot.cert_voted_block_hash.0,
        already_next_voted_value: snapshot.already_next_voted_value,
        already_next_voted_null: snapshot.already_next_voted_null,
    });
    ensure!(
        plan.status == PbftManagerStateActionStatus::Ready,
        "CONSENSUS_STATE_ACTION_PLAN_REJECTED: {}",
        plan.error_code
    );

    let mut votes = Vec::with_capacity(2);
    for (intent, hash) in [
        (plan.primary_intent, plan.primary_hash),
        (plan.secondary_intent, plan.secondary_hash),
    ] {
        if let Some(vote) = compose_vote_task(
            service,
            final_chain,
            dag_transaction,
            policy,
            snapshot.period,
            snapshot.round,
            snapshot.step,
            snapshot.state,
            intent,
            H256(hash),
        )? {
            votes.push(vote);
        }
    }

    Ok(ConsensusStateActionBatch {
        state: snapshot.state,
        votes,
        go_finish_state: plan.go_finish_state,
        loop_back_finish_state: plan.loop_back_finish_state,
    })
}

#[allow(clippy::too_many_arguments)]
fn compose_vote_task(
    service: &PbftService,
    final_chain: &FinalChain,
    dag_transaction: &DagTransactionService,
    policy: ConsensusStateActionPolicy,
    period: u64,
    round: u64,
    current_step: u64,
    state: PbftManagerRuntimeStateCode,
    intent: PbftManagerStateActionIntent,
    requested_hash: H256,
) -> Result<Option<ConsensusStateVoteTask>> {
    let (vote_type, step, block_hash, block_rlp, commit) = match intent {
        PbftManagerStateActionIntent::Noop | PbftManagerStateActionIntent::GoFinish => {
            return Ok(None);
        }
        PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote => {
            let selected = service.select_leader_composed(
                final_chain,
                dag_transaction,
                PbftComposedLeaderSelectionRequest {
                    period,
                    round,
                    pbft_gas_limit: policy.pbft_gas_limit,
                    extra_data_required: policy.extra_data_required,
                    pillar_block_required: policy.pillar_block_required,
                },
            )?;
            if selected.status != PbftLeaderSelectionStatus::Selected || !selected.selected {
                return Ok(None);
            }
            let inspection = inspect_canonical_pbft_vote(&selected.selected_vote.vote_rlp)?;
            (
                PbftVoteType::Soft,
                PbftVoteType::Soft as u64,
                inspection.block_hash,
                selected.selected_block_rlp,
                ConsensusStateVoteCommit::None,
            )
        }
        PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue
        | PbftManagerStateActionIntent::CertVoteCurrentSoftValue
        | PbftManagerStateActionIntent::NextVotePreviousRoundValue
        | PbftManagerStateActionIntent::NextVoteCurrentSoftValue => {
            let admitted = service.admit_proposed_block(
                final_chain,
                dag_transaction,
                PbftProposedBlockAdmissionRequest {
                    period,
                    block_hash: requested_hash,
                    pbft_gas_limit: policy.pbft_gas_limit,
                    extra_data_required: policy.extra_data_required,
                    pillar_block_required: policy.pillar_block_required,
                },
            )?;
            if !matches!(
                admitted.status,
                PbftProposedBlockAdmissionStatus::AcceptedAlreadyValid
                    | PbftProposedBlockAdmissionStatus::AcceptedNewlyValidated
            ) {
                return Ok(None);
            }
            match intent {
                PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue => (
                    PbftVoteType::Soft,
                    PbftVoteType::Soft as u64,
                    requested_hash,
                    admitted.block_rlp,
                    ConsensusStateVoteCommit::None,
                ),
                PbftManagerStateActionIntent::CertVoteCurrentSoftValue => (
                    PbftVoteType::Cert,
                    PbftVoteType::Cert as u64,
                    requested_hash,
                    admitted.block_rlp,
                    ConsensusStateVoteCommit::SaveCertVotedBlock,
                ),
                PbftManagerStateActionIntent::NextVoteCurrentSoftValue => (
                    PbftVoteType::Next,
                    current_step,
                    requested_hash,
                    admitted.block_rlp,
                    ConsensusStateVoteCommit::MarkNextVotedValue,
                ),
                PbftManagerStateActionIntent::NextVotePreviousRoundValue => (
                    PbftVoteType::Next,
                    current_step,
                    requested_hash,
                    admitted.block_rlp,
                    ConsensusStateVoteCommit::None,
                ),
                _ => unreachable!("matched direct block vote intent"),
            }
        }
        PbftManagerStateActionIntent::NextVoteCertVotedBlock => {
            let block_rlp = service.cert_voted_block_in_round()?;
            if block_rlp.is_empty() {
                return Ok(None);
            }
            (
                PbftVoteType::Next,
                current_step,
                requested_hash,
                block_rlp,
                ConsensusStateVoteCommit::None,
            )
        }
        PbftManagerStateActionIntent::NextVoteNullBlock => (
            PbftVoteType::Next,
            current_step,
            H256::zero(),
            Vec::new(),
            if state == PbftManagerRuntimeStateCode::FinishPolling {
                ConsensusStateVoteCommit::MarkNextVotedNull
            } else {
                ConsensusStateVoteCommit::None
            },
        ),
        PbftManagerStateActionIntent::ProposeNewBlock
        | PbftManagerStateActionIntent::ReproposePreviousRoundNextValue => {
            return Ok(None);
        }
    };
    Ok(Some(ConsensusStateVoteTask {
        period,
        round,
        step,
        vote_type,
        block_hash,
        proposed_block_rlp: block_rlp,
        commit,
    }))
}
