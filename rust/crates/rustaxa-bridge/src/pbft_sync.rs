//! CXX bridge wrappers for PBFT sync-period admission planning.
//!
//! C++ supplies compact fact codes and transaction hash vectors gathered from
//! existing shim-owned consensus surfaces. Rust converts them into typed domain
//! facts, runs the deterministic planner, and returns u8-coded side-effect
//! intent flags for the PBFT manager overlay to apply.

use crate::dag_transaction_service::BridgeApp;
use crate::ffi::rustaxa_ffi::{
    PbftCertVoteRlp as FfiPbftCertVoteRlp,
    PbftSyncAdmissionInitialFact as FfiPbftSyncAdmissionInitialFact,
    PbftSyncAdmissionSessionStep as FfiPbftSyncAdmissionSessionStep,
    PbftSyncCertBundleCommand as FfiPbftSyncCertBundleCommand,
    PbftSyncCertBundleStep as FfiPbftSyncCertBundleStep,
    PbftSyncProcessPeriodDataRuntimePlan as FfiPbftSyncProcessPeriodDataRuntimePlan,
    PbftSyncTransactionWarning as FfiPbftSyncTransactionWarning,
    PeriodDataQueueTransactionIdentity as FfiPeriodDataQueueTransactionIdentity,
    PillarVoteRlpPayload,
};
use crate::ffi::BridgeFinalChain;
use crate::verified_votes::{
    empty_slashing_transaction_effect, slashing_transaction_effect_to_ffi,
};
use anyhow::{anyhow, Result};
use ethereum_types::H256;
use rustaxa_consensus::pbft_manager::PbftManagerFinalChainHashStatus;
use rustaxa_consensus::pbft_service::PbftSyncCertBundleStep as DomainPbftSyncCertBundleStep;
use rustaxa_consensus::pbft_sync::{
    PbftSyncAdmissionInitialFact, PbftSyncAdmissionSessionStep, PbftSyncFactStatus,
    PbftSyncProcessPeriodDataRuntimePlan, PbftSyncProcessRuntimeNextCheck,
    PbftSyncRuntimeFinalChainHashStatus, PbftSyncTransactionWarning,
};

/// Starts a manager-owned synced-period admission cursor and returns its first step.
pub fn pbft_manager_runtime_begin_pbft_sync_admission(
    runtime: &BridgeApp,
    fact: FfiPbftSyncAdmissionInitialFact,
) -> FfiPbftSyncAdmissionSessionStep {
    runtime.0.begin_pbft_sync_admission(fact.into());
    runtime
        .0
        .pbft_sync_admission_next()
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Resolves a native FinalChain hash check or reports an external certificate result.
///
/// FinalChain lookup and exact status reporting are composed inside the native
/// service so an intervening session cannot consume the result. When the
/// resulting cursor requests reward votes, the service derives and exact-reports
/// that stage immediately from immutable session hashes. Accepted weighted
/// payloads travel on the returned step; stale or missing sessions return the
/// stable not-started contract result. FinalChain infrastructure failures abort
/// only the captured native request and propagate to the C++ executor.
pub fn pbft_manager_runtime_pbft_sync_admission_report_status(
    runtime: &BridgeApp,
    final_chain: &BridgeFinalChain,
    cursor: u32,
    check_code: u8,
    status: u8,
) -> Result<FfiPbftSyncAdmissionSessionStep> {
    let check = PbftSyncProcessRuntimeNextCheck::from_u8(check_code);
    let result = if check == PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash {
        let result = runtime
            .0
            .validate_pbft_sync_admission_final_chain_hash(&final_chain.0);
        match result {
            Some((_, _, validation))
                if validation.status == PbftManagerFinalChainHashStatus::Unknown =>
            {
                return Err(anyhow!(validation.error_code));
            }
            Some((step, records, _)) => Some((step, records)),
            None => None,
        }
    } else {
        runtime
            .0
            .report_pbft_sync_admission_status_with_reward_votes(
                cursor,
                check,
                PbftSyncRuntimeFinalChainHashStatus::from_u8(status),
                PbftSyncFactStatus::from_u8(status),
            )
    };
    let Some((step, records)) = result else {
        return Ok(sync_admission_not_started_step());
    };
    let mut step = FfiPbftSyncAdmissionSessionStep::from(step);
    step.reward_vote_rlps = records
        .into_iter()
        .map(|record| FfiPbftCertVoteRlp {
            vote_rlp: record.vote_rlp,
        })
        .collect();
    Ok(step)
}

/// Executes and reports the exact native sync pillar-vote admission task.
///
/// The PBFT root owns cursor identity, pillar readiness, FinalChain-weighted
/// bundle application, deterministic validity, and terminal queue effects.
/// Empty, unavailable, rejected, or infrastructure-failed bundles preserve the
/// legacy invalid-fact result. A stale or missing cursor returns the existing
/// not-started contract step without mutating a replacement.
pub fn pbft_manager_runtime_pbft_sync_admission_validate_pillar_votes(
    runtime: &BridgeApp,
    final_chain: &BridgeFinalChain,
    vote_rlps: Vec<PillarVoteRlpPayload>,
) -> FfiPbftSyncAdmissionSessionStep {
    runtime
        .0
        .validate_pbft_sync_admission_pillar_votes(
            &final_chain.0,
            vote_rlps
                .into_iter()
                .map(
                    |value| rustaxa_consensus::pillar_vote_service::PillarVoteRlpPayload {
                        vote_rlp: value.vote_rlp,
                    },
                )
                .collect(),
        )
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Executes and reports the exact native sync transaction-admission task.
///
/// Rust owns the requested missing-hash lookup, recent/durable finalized
/// checks, FinalChain account-nonce enrichment, warning facts, and exact
/// generation/cursor report. C++ supplies only the queue-retained canonical
/// transaction identities and the narrow native service handles. Native
/// storage or FinalChain failures terminate only the exact captured admission;
/// a stale completion returns the not-started step without mutating its
/// replacement.
pub fn pbft_manager_runtime_pbft_sync_admission_validate_transactions(
    runtime: &BridgeApp,
    dag_transaction_service: &BridgeApp,
    final_chain: &BridgeFinalChain,
    identities: Vec<FfiPeriodDataQueueTransactionIdentity>,
) -> FfiPbftSyncAdmissionSessionStep {
    dag_transaction_service
        .validate_pbft_sync_admission_transactions(
            runtime,
            final_chain,
            identities
                .into_iter()
                .map(|identity| {
                    rustaxa_consensus::period_data_queue::PeriodDataQueueTransactionIdentity {
                        input_index: identity.input_index,
                        hash: H256::from(identity.hash),
                        transaction_nonce: identity.transaction_nonce,
                        sender: identity.sender,
                    }
                })
                .collect(),
        )
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Aborts and clears the current synced-period admission cursor.
pub fn abort_pbft_manager_runtime_pbft_sync_admission(
    runtime: &BridgeApp,
) -> FfiPbftSyncAdmissionSessionStep {
    runtime
        .0
        .abort_pbft_sync_admission()
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Executes one begin, report, or exact-abort command for the resumable native
/// current-certificate admission session without exposing a bridge runtime.
pub fn pbft_service_pbft_sync_cert_bundle_session(
    service: &BridgeApp,
    final_chain: &BridgeFinalChain,
    command: FfiPbftSyncCertBundleCommand,
) -> Result<FfiPbftSyncCertBundleStep> {
    match command.action {
        0 => service
            .0
            .begin_pbft_sync_cert_bundle(
                &final_chain.0,
                command.block_period,
                H256::from(command.block_hash),
                command
                    .cert_vote_rlps
                    .into_iter()
                    .map(|vote| vote.vote_rlp)
                    .collect(),
            )
            .map(Into::into),
        1 => service
            .0
            .report_pbft_sync_cert_bundle_slashing(
                command.session_id,
                command.effect_id,
                H256::from(command.proof_hash),
                command.transaction_inserted,
            )
            .map(Into::into),
        2 => {
            service.0.abort_pbft_sync_cert_bundle(command.session_id)?;
            Ok(FfiPbftSyncCertBundleStep {
                action: 2,
                session_id: command.session_id,
                effect_id: 0,
                status: 0,
                total_weight: 0,
                two_t_plus_one: 0,
                first_bad_vote_hash: [0; 32],
                error_code: "PBFT_SYNC_CERT_BUNDLE_ABORTED".into(),
                weighted_vote_rlps: Vec::new(),
                has_slashing_effect: false,
                slashing_transaction_effect: empty_slashing_transaction_effect(),
            })
        }
        _ => anyhow::bail!("PBFT_SYNC_CERT_BUNDLE_UNKNOWN_COMMAND"),
    }
}

impl From<PbftSyncProcessPeriodDataRuntimePlan> for FfiPbftSyncProcessPeriodDataRuntimePlan {
    fn from(plan: PbftSyncProcessPeriodDataRuntimePlan) -> Self {
        Self {
            runtime_action: plan.runtime_action.as_u8(),
            status: plan.status.as_u8(),
            next_check: plan.next_check.as_u8(),
            clear_sync_queue: plan.clear_sync_queue,
            report_malicious_peer: plan.report_malicious_peer,
            wait_for_finalization: plan.wait_for_finalization,
            accept_period_data: plan.accept_period_data,
            retry_same_candidate: plan.retry_same_candidate,
            replace_previous_block_cert_votes: plan.replace_previous_block_cert_votes,
            warnings: plan
                .warnings
                .into_iter()
                .map(FfiPbftSyncTransactionWarning::from)
                .collect(),
            contains_finalized_transaction_warning: plan.contains_finalized_transaction_warning,
        }
    }
}

impl From<FfiPbftSyncAdmissionInitialFact> for PbftSyncAdmissionInitialFact {
    fn from(value: FfiPbftSyncAdmissionInitialFact) -> Self {
        Self {
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            dag_transaction_hashes: value
                .dag_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            period_data_transaction_hashes: value
                .period_data_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            reward_vote_hashes: value
                .reward_vote_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            candidate_final_chain_hash: H256::from(value.candidate_final_chain_hash),
            extra_data_required: value.extra_data_required,
            extra_data_present: value.extra_data_present,
            extra_data_pillar_block_hash_present: value.extra_data_pillar_block_hash_present,
            pillar_votes_required: value.pillar_votes_required,
            pillar_votes_present: value.pillar_votes_present,
            previous_cert_votes_present: value.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: value.previous_cert_first_vote_has_weight,
        }
    }
}

impl From<PbftSyncAdmissionSessionStep> for FfiPbftSyncAdmissionSessionStep {
    fn from(value: PbftSyncAdmissionSessionStep) -> Self {
        Self {
            status: value.status.as_u8(),
            cursor: value.cursor,
            has_check: value.has_check,
            next_check: value.next_check.as_u8(),
            plan: value.plan.into(),
            complete: value.complete,
            can_continue: value.can_continue,
            error_code: value.error_code,
            reward_vote_rlps: Vec::new(),
        }
    }
}

fn sync_admission_not_started_step() -> FfiPbftSyncAdmissionSessionStep {
    FfiPbftSyncAdmissionSessionStep {
        status: 4,
        cursor: 0,
        has_check: false,
        next_check: 0,
        plan: FfiPbftSyncProcessPeriodDataRuntimePlan {
            runtime_action: 5,
            status: 11,
            next_check: 0,
            clear_sync_queue: false,
            report_malicious_peer: false,
            wait_for_finalization: false,
            accept_period_data: false,
            retry_same_candidate: false,
            replace_previous_block_cert_votes: false,
            warnings: Vec::new(),
            contains_finalized_transaction_warning: false,
        },
        complete: true,
        can_continue: false,
        error_code: "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED".to_string(),
        reward_vote_rlps: Vec::new(),
    }
}

impl From<DomainPbftSyncCertBundleStep> for FfiPbftSyncCertBundleStep {
    fn from(value: DomainPbftSyncCertBundleStep) -> Self {
        let has_slashing_effect = value.slashing_transaction_effect.is_some();
        let slashing_transaction_effect = value
            .slashing_transaction_effect
            .map(slashing_transaction_effect_to_ffi)
            .unwrap_or_else(empty_slashing_transaction_effect);
        Self {
            action: value.action.as_u8(),
            session_id: value.session_id,
            effect_id: value.effect_id,
            status: value.status.as_u8(),
            total_weight: value.total_weight,
            two_t_plus_one: value.two_t_plus_one,
            first_bad_vote_hash: value.first_bad_vote_hash.0,
            error_code: value.error_code,
            weighted_vote_rlps: value
                .weighted_vote_rlps
                .into_iter()
                .map(|vote_rlp| FfiPbftCertVoteRlp { vote_rlp })
                .collect(),
            has_slashing_effect,
            slashing_transaction_effect,
        }
    }
}

impl From<PbftSyncTransactionWarning> for FfiPbftSyncTransactionWarning {
    fn from(value: PbftSyncTransactionWarning) -> Self {
        Self {
            hash: value.hash.into(),
            kind: value.kind.as_u8(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftSyncTransactionHash as FfiPbftSyncTransactionHash;
    use rustaxa_consensus::pbft_service::PbftSyncCertBundleAction as DomainPbftSyncCertBundleAction;
    use rustaxa_consensus::pbft_sync::PbftSyncCertVoteBundleStatus as DomainPbftSyncCertVoteBundleStatus;

    #[test]
    fn bridge_projects_terminal_cert_bundle_step() {
        let result = FfiPbftSyncCertBundleStep::from(DomainPbftSyncCertBundleStep {
            action: DomainPbftSyncCertBundleAction::Accepted,
            session_id: 11,
            effect_id: 0,
            status: DomainPbftSyncCertVoteBundleStatus::Accepted,
            total_weight: 10,
            two_t_plus_one: 5,
            first_bad_vote_hash: H256::from([7; 32]),
            error_code: String::new(),
            weighted_vote_rlps: vec![vec![1, 2, 3]],
            slashing_transaction_effect: None,
        });
        assert_eq!(
            result.action,
            DomainPbftSyncCertBundleAction::Accepted.as_u8()
        );
        assert_eq!(result.weighted_vote_rlps[0].vote_rlp, [1, 2, 3]);
    }

    #[test]
    fn bridge_projects_native_sync_admission_initial_fact_projection() {
        let domain: PbftSyncAdmissionInitialFact = FfiPbftSyncAdmissionInitialFact {
            block_period: 10,
            block_prev_hash: [1; 32],
            chain_last_hash: [2; 32],
            chain_last_period: 9,
            block_in_chain: true,
            dag_transaction_hashes: vec![
                FfiPbftSyncTransactionHash { hash: [3; 32] },
                FfiPbftSyncTransactionHash { hash: [4; 32] },
            ],
            period_data_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [5; 32] }],
            reward_vote_hashes: Vec::new(),
            candidate_final_chain_hash: [6; 32],
            extra_data_required: true,
            extra_data_present: false,
            extra_data_pillar_block_hash_present: true,
            pillar_votes_required: true,
            pillar_votes_present: false,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
        .into();
        assert_eq!(domain.block_period, 10);
        assert_eq!(domain.block_prev_hash, H256::from([1; 32]));
        assert_eq!(domain.chain_last_hash, H256::from([2; 32]));
        assert_eq!(domain.chain_last_period, 9);
        assert!(domain.block_in_chain);
        assert_eq!(
            domain.dag_transaction_hashes,
            [H256::from([3; 32]), H256::from([4; 32])]
        );
        assert_eq!(domain.period_data_transaction_hashes, [H256::from([5; 32])]);
        assert!(domain.reward_vote_hashes.is_empty());
        assert_eq!(domain.candidate_final_chain_hash, H256::from([6; 32]));
        assert!(domain.extra_data_required);
        assert!(!domain.extra_data_present);
        assert!(domain.extra_data_pillar_block_hash_present);
        assert!(domain.pillar_votes_required);
        assert!(!domain.pillar_votes_present);
        assert!(domain.previous_cert_votes_present);
        assert!(!domain.previous_cert_first_vote_has_weight);
    }

    #[test]
    fn bridge_projects_native_sync_admission_not_started_step_projection() {
        let step = sync_admission_not_started_step();
        assert_eq!(step.status, 4);
        assert_eq!(step.cursor, 0);
        assert!(!step.has_check);
        assert_eq!(step.next_check, 0);
        assert!(step.complete);
        assert!(!step.can_continue);
        assert_eq!(step.error_code, "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED");
        assert_eq!(step.plan.runtime_action, 5);
        assert_eq!(step.plan.status, 11);
        assert_eq!(step.plan.next_check, 0);
        assert!(!step.plan.clear_sync_queue);
        assert!(!step.plan.report_malicious_peer);
        assert!(!step.plan.wait_for_finalization);
        assert!(!step.plan.accept_period_data);
        assert!(!step.plan.retry_same_candidate);
        assert!(!step.plan.replace_previous_block_cert_votes);
        assert!(!step.plan.contains_finalized_transaction_warning);
        assert!(step.plan.warnings.is_empty());
    }
}
