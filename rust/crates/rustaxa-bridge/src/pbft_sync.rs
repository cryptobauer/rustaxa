//! CXX bridge wrappers for PBFT sync-period admission planning.
//!
//! C++ supplies compact fact codes and transaction hash vectors gathered from
//! existing shim-owned consensus surfaces. Rust converts them into typed domain
//! facts, runs the deterministic planner, and returns u8-coded side-effect
//! intent flags for the PBFT manager overlay to apply.

use crate::ffi::rustaxa_ffi::{
    PbftSyncAdmissionInitialFact as FfiPbftSyncAdmissionInitialFact,
    PbftSyncAdmissionSessionStep as FfiPbftSyncAdmissionSessionStep,
    PbftSyncAdmissionStatusReport as FfiPbftSyncAdmissionStatusReport,
    PbftSyncAdmissionTransactionReport as FfiPbftSyncAdmissionTransactionReport,
    PbftSyncCertVoteBundleFact as FfiPbftSyncCertVoteBundleFact,
    PbftSyncCertVoteBundleValidation as FfiPbftSyncCertVoteBundleValidation,
    PbftSyncCertVoteFact as FfiPbftSyncCertVoteFact,
    PbftSyncProcessPeriodDataRuntimePlan as FfiPbftSyncProcessPeriodDataRuntimePlan,
    PbftSyncTransactionHash as FfiPbftSyncTransactionHash,
    PbftSyncTransactionQueryPlan as FfiPbftSyncTransactionQueryPlan,
    PbftSyncTransactionWarning as FfiPbftSyncTransactionWarning,
};
use crate::ffi::BridgePbftService;
use ethereum_types::H256;
use rustaxa_consensus::pbft_sync::{
    validate_pbft_sync_cert_vote_bundle as validate_domain_pbft_sync_cert_vote_bundle,
    PbftSyncAdmissionInitialFact, PbftSyncAdmissionSessionStep, PbftSyncAdmissionTransactionReport,
    PbftSyncCertVoteBundleFact, PbftSyncCertVoteBundleValidation, PbftSyncCertVoteFact,
    PbftSyncFactStatus, PbftSyncProcessPeriodDataRuntimePlan, PbftSyncProcessRuntimeNextCheck,
    PbftSyncRuntimeFinalChainHashStatus, PbftSyncTransactionQueryPlan, PbftSyncTransactionWarning,
};

/// Starts a manager-owned synced-period admission cursor.
pub fn pbft_manager_runtime_begin_pbft_sync_admission(
    runtime: &BridgePbftService,
    fact: FfiPbftSyncAdmissionInitialFact,
) {
    runtime.0.begin_pbft_sync_admission(fact.into());
}

/// Returns the current admission check without advancing the cursor.
pub fn pbft_manager_runtime_pbft_sync_admission_next(
    runtime: &BridgePbftService,
) -> FfiPbftSyncAdmissionSessionStep {
    runtime
        .0
        .pbft_sync_admission_next()
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Reports a final-chain, reward, cert, or pillar validation result.
pub fn pbft_manager_runtime_pbft_sync_admission_report_status(
    runtime: &BridgePbftService,
    report: FfiPbftSyncAdmissionStatusReport,
) -> FfiPbftSyncAdmissionSessionStep {
    let check = PbftSyncProcessRuntimeNextCheck::from_u8(report.check);
    runtime
        .0
        .report_pbft_sync_admission_status(
            report.cursor,
            check,
            PbftSyncRuntimeFinalChainHashStatus::from_u8(report.status),
            PbftSyncFactStatus::from_u8(report.status),
        )
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Reports the requested transaction-manager lookup result.
pub fn pbft_manager_runtime_pbft_sync_admission_report_transactions(
    runtime: &BridgePbftService,
    report: FfiPbftSyncAdmissionTransactionReport,
) -> FfiPbftSyncAdmissionSessionStep {
    runtime
        .0
        .report_pbft_sync_admission_transactions(
            report.cursor,
            pbft_sync_admission_transaction_report_from_ffi(report),
        )
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Aborts and clears the current synced-period admission cursor.
pub fn abort_pbft_manager_runtime_pbft_sync_admission(
    runtime: &BridgePbftService,
) -> FfiPbftSyncAdmissionSessionStep {
    runtime
        .0
        .abort_pbft_sync_admission()
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step)
}

/// Validates one synced PBFT cert-vote bundle from compact C++ facts.
///
/// C++ remains the temporary executor for VoteManager signature/weight checks,
/// but Rust owns the deterministic bundle-shape and threshold decision.
pub fn validate_pbft_sync_cert_vote_bundle(
    fact: FfiPbftSyncCertVoteBundleFact,
) -> FfiPbftSyncCertVoteBundleValidation {
    validate_domain_pbft_sync_cert_vote_bundle(fact.into()).into()
}

impl From<FfiPbftSyncCertVoteFact> for PbftSyncCertVoteFact {
    fn from(value: FfiPbftSyncCertVoteFact) -> Self {
        Self {
            vote_hash: H256::from(value.vote_hash),
            block_hash: H256::from(value.block_hash),
            period: value.period,
            round: value.round,
            step: value.step,
            vote_type: value.vote_type,
            live_vote_valid: value.live_vote_valid,
            weight_present: value.weight_present,
            weight: value.weight,
        }
    }
}

impl From<FfiPbftSyncCertVoteBundleFact> for PbftSyncCertVoteBundleFact {
    fn from(value: FfiPbftSyncCertVoteBundleFact) -> Self {
        Self {
            block_period: value.block_period,
            block_hash: H256::from(value.block_hash),
            votes: value.votes.into_iter().map(Into::into).collect(),
            check_weight_threshold: value.check_weight_threshold,
            two_t_plus_one_found: value.two_t_plus_one_found,
            two_t_plus_one: value.two_t_plus_one,
        }
    }
}

impl From<PbftSyncTransactionQueryPlan> for FfiPbftSyncTransactionQueryPlan {
    fn from(plan: PbftSyncTransactionQueryPlan) -> Self {
        Self {
            finalized_lookup_hashes: plan
                .finalized_lookup_hashes
                .into_iter()
                .map(|hash| FfiPbftSyncTransactionHash { hash: hash.into() })
                .collect(),
        }
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
            transaction_query_plan: plan.transaction_query.into(),
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
        }
    }
}

fn pbft_sync_admission_transaction_report_from_ffi(
    report: FfiPbftSyncAdmissionTransactionReport,
) -> PbftSyncAdmissionTransactionReport {
    PbftSyncAdmissionTransactionReport {
        missing_transaction_hashes: report
            .missing_transaction_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect(),
        finalized_transaction_hashes: report
            .finalized_transaction_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect(),
        contains_finalized_transactions: report.contains_finalized_transactions,
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
            transaction_query_plan: FfiPbftSyncTransactionQueryPlan {
                finalized_lookup_hashes: Vec::new(),
            },
            warnings: Vec::new(),
            contains_finalized_transaction_warning: false,
        },
        complete: true,
        can_continue: false,
        error_code: "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED".to_string(),
    }
}

impl From<PbftSyncCertVoteBundleValidation> for FfiPbftSyncCertVoteBundleValidation {
    fn from(value: PbftSyncCertVoteBundleValidation) -> Self {
        Self {
            valid: value.valid,
            status: value.status.as_u8(),
            total_weight: value.total_weight,
            two_t_plus_one: value.two_t_plus_one,
            first_bad_vote_hash: value.first_bad_vote_hash.into(),
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

    fn cert_vote(weight: u64) -> FfiPbftSyncCertVoteFact {
        FfiPbftSyncCertVoteFact {
            vote_hash: [weight as u8; 32],
            block_hash: [9; 32],
            period: 101,
            round: 2,
            step: 3,
            vote_type: 3,
            live_vote_valid: true,
            weight_present: true,
            weight,
        }
    }

    #[test]
    fn bridge_cert_vote_bundle_validation_projects_status() {
        let result = validate_pbft_sync_cert_vote_bundle(FfiPbftSyncCertVoteBundleFact {
            block_period: 101,
            block_hash: [9; 32],
            votes: vec![cert_vote(2), cert_vote(3)],
            check_weight_threshold: true,
            two_t_plus_one_found: true,
            two_t_plus_one: 5,
        });

        assert_eq!(
            (
                result.valid,
                result.status,
                result.total_weight,
                result.two_t_plus_one
            ),
            (true, 0, 5, 5)
        );

        let result = validate_pbft_sync_cert_vote_bundle(FfiPbftSyncCertVoteBundleFact {
            block_period: 101,
            block_hash: [9; 32],
            votes: vec![cert_vote(2)],
            check_weight_threshold: true,
            two_t_plus_one_found: true,
            two_t_plus_one: 5,
        });

        assert_eq!(
            (result.valid, result.status, result.total_weight),
            (false, 10, 2)
        );
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
        assert!(domain.extra_data_required);
        assert!(!domain.extra_data_present);
        assert!(domain.extra_data_pillar_block_hash_present);
        assert!(domain.pillar_votes_required);
        assert!(!domain.pillar_votes_present);
        assert!(domain.previous_cert_votes_present);
        assert!(!domain.previous_cert_first_vote_has_weight);
    }

    #[test]
    fn bridge_projects_native_sync_admission_transaction_report_projection() {
        let domain = pbft_sync_admission_transaction_report_from_ffi(
            FfiPbftSyncAdmissionTransactionReport {
                cursor: 7,
                missing_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [6; 32] }],
                finalized_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [7; 32] }],
                contains_finalized_transactions: true,
            },
        );
        assert_eq!(
            domain,
            PbftSyncAdmissionTransactionReport {
                missing_transaction_hashes: vec![H256::from([6; 32])],
                finalized_transaction_hashes: vec![H256::from([7; 32])],
                contains_finalized_transactions: true,
            }
        );
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
        assert!(step
            .plan
            .transaction_query_plan
            .finalized_lookup_hashes
            .is_empty());
        assert!(step.plan.warnings.is_empty());
    }
}
