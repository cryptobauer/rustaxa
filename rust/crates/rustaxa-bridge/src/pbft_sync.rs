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
    PbftSyncEgressPayload as FfiPbftSyncEgressPayload,
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
    PbftSyncRewardVoteAttachmentFact, PbftSyncRuntimeFinalChainHashStatus,
    PbftSyncTransactionQueryPlan, PbftSyncTransactionWarning,
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
            },
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

/// Loads the storage-backed payload for one PBFT sync egress packet.
///
/// C++ still owns packet wrapping, transport, and temporary reward-vote
/// sidecar materialization. Rust owns the canonical `PeriodData` storage read
/// and the deterministic decision about whether those sidecars belong on the
/// packet.
pub fn load_pbft_sync_egress_payload(
    runtime: &BridgePbftService,
    block_period: u64,
    last_block: bool,
    pbft_chain_synced: bool,
    reward_votes_present: bool,
    reward_votes_period: u64,
) -> anyhow::Result<FfiPbftSyncEgressPayload> {
    let payload = runtime
        .0
        .load_pbft_sync_egress_payload(PbftSyncRewardVoteAttachmentFact {
            block_period,
            last_block,
            pbft_chain_synced,
            reward_votes_present,
            reward_votes_period,
        })?;
    Ok(FfiPbftSyncEgressPayload {
        period_data_rlp: payload.period_data_rlp,
        attach_reward_votes: payload.attach_reward_votes,
    })
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
    use rustaxa_consensus::{PbftService, PbftServiceConfig};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos))
    }

    fn service(path: &std::path::Path) -> (Box<crate::ffi::BridgeStorage>, BridgePbftService) {
        let storage = crate::storage::create_storage(path.to_str().expect("UTF-8 path")).unwrap();
        let service = BridgePbftService(
            PbftService::restore(
                storage.0.clone(),
                PbftServiceConfig {
                    genesis_lambda_ms: 100,
                    cacti_lambda_max_ms: 1_500,
                    cacti_lambda_default_ms: 500,
                    cacti_block: 1,
                    max_exponential_lambda_ms: 60_000,
                    max_steps: 13,
                    deadline_ms: 1_000,
                    polling_interval_ms: 100,
                    report_malicious_behaviour: true,
                    magnolia_activation_period: 0,
                },
            )
            .unwrap(),
        );
        (storage, service)
    }

    fn admission_fact() -> FfiPbftSyncAdmissionInitialFact {
        FfiPbftSyncAdmissionInitialFact {
            block_period: 10,
            block_prev_hash: [9; 32],
            chain_last_hash: [9; 32],
            chain_last_period: 9,
            block_in_chain: false,
            dag_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [1; 32] }],
            period_data_transaction_hashes: Vec::new(),
            extra_data_required: false,
            extra_data_present: false,
            extra_data_pillar_block_hash_present: false,
            pillar_votes_required: false,
            pillar_votes_present: false,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

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

        assert!(result.valid);
        assert_eq!(result.status, 0);
        assert_eq!(result.total_weight, 5);
        assert_eq!(result.two_t_plus_one, 5);

        let result = validate_pbft_sync_cert_vote_bundle(FfiPbftSyncCertVoteBundleFact {
            block_period: 101,
            block_hash: [9; 32],
            votes: vec![cert_vote(2)],
            check_weight_threshold: true,
            two_t_plus_one_found: true,
            two_t_plus_one: 5,
        });

        assert!(!result.valid);
        assert_eq!(result.status, 10);
        assert_eq!(result.total_weight, 2);
    }

    #[test]
    fn bridge_projects_native_sync_admission_and_egress() {
        let path = unique_temp_dir("rustaxa_bridge_pbft_sync_projection");
        let (storage, runtime) = service(&path);
        storage
            .0
            .period()
            .write(9, &[0xc8, 0xc0, 0xc1])
            .expect("period data persists");

        pbft_manager_runtime_begin_pbft_sync_admission(&runtime, admission_fact());
        assert_eq!(
            pbft_manager_runtime_pbft_sync_admission_next(&runtime).error_code,
            "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED"
        );

        runtime.0.complete_bootstrap();
        pbft_manager_runtime_begin_pbft_sync_admission(&runtime, admission_fact());
        let mut step = pbft_manager_runtime_pbft_sync_admission_next(&runtime);
        assert!(step.has_check);
        for _ in 0..3 {
            step = pbft_manager_runtime_pbft_sync_admission_report_status(
                &runtime,
                FfiPbftSyncAdmissionStatusReport {
                    cursor: step.cursor,
                    check: step.next_check,
                    status: 0,
                },
            );
            assert!(step.has_check);
        }

        let accepted = pbft_manager_runtime_pbft_sync_admission_report_transactions(
            &runtime,
            FfiPbftSyncAdmissionTransactionReport {
                cursor: step.cursor,
                missing_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [1; 32] }],
                finalized_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [2; 32] }],
                contains_finalized_transactions: true,
            },
        );
        assert!(accepted.complete);
        assert!(accepted.plan.accept_period_data);
        assert_eq!(accepted.plan.warnings.len(), 2);
        assert_eq!(accepted.plan.warnings[0].hash, [1; 32]);
        assert_eq!(accepted.plan.warnings[1].hash, [2; 32]);
        assert!(accepted.plan.contains_finalized_transaction_warning);

        pbft_manager_runtime_begin_pbft_sync_admission(&runtime, admission_fact());
        let step = pbft_manager_runtime_pbft_sync_admission_next(&runtime);
        let mismatch = pbft_manager_runtime_pbft_sync_admission_report_status(
            &runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: step.cursor + 1,
                check: step.next_check,
                status: 0,
            },
        );
        assert!(mismatch.complete);
        assert!(!mismatch.can_continue);
        assert_eq!(
            pbft_manager_runtime_pbft_sync_admission_next(&runtime).error_code,
            "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED"
        );

        let payload = load_pbft_sync_egress_payload(&runtime, 9, true, true, true, 9).unwrap();
        assert_eq!(payload.period_data_rlp, vec![0xc8, 0xc0, 0xc1]);
        assert!(payload.attach_reward_votes);

        drop(runtime);
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }
}
