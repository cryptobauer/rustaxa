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
    abort_pbft_sync_admission_session as abort_domain_pbft_sync_admission_session,
    create_pbft_sync_admission_session as create_domain_pbft_sync_admission_session,
    load_pbft_sync_egress_payload as load_domain_pbft_sync_egress_payload,
    next_pbft_sync_admission_session as next_domain_pbft_sync_admission_session,
    report_pbft_sync_admission_status as report_domain_pbft_sync_admission_status,
    report_pbft_sync_admission_transactions as report_domain_pbft_sync_admission_transactions,
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
    if !runtime.accepts_live_commands() {
        return;
    }
    let mut runtime = runtime.manager_state();
    runtime.pbft_sync_admission_session =
        Some(create_domain_pbft_sync_admission_session(fact.into()));
}

/// Returns the current admission check without advancing the cursor.
pub fn pbft_manager_runtime_pbft_sync_admission_next(
    runtime: &BridgePbftService,
) -> FfiPbftSyncAdmissionSessionStep {
    if !runtime.accepts_live_commands() {
        return sync_admission_not_started_step();
    }
    let mut runtime = runtime.manager_state();
    let step = runtime
        .pbft_sync_admission_session
        .as_ref()
        .map(next_domain_pbft_sync_admission_session)
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step);
    clear_terminal_sync_admission(&mut runtime, &step);
    step
}

/// Reports a final-chain, reward, cert, or pillar validation result.
pub fn pbft_manager_runtime_pbft_sync_admission_report_status(
    runtime: &BridgePbftService,
    report: FfiPbftSyncAdmissionStatusReport,
) -> FfiPbftSyncAdmissionSessionStep {
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.pbft_sync_admission_session.as_mut() else {
        return sync_admission_not_started_step();
    };
    let check = PbftSyncProcessRuntimeNextCheck::from_u8(report.check);
    let step = report_domain_pbft_sync_admission_status(
        session,
        report.cursor,
        check,
        PbftSyncRuntimeFinalChainHashStatus::from_u8(report.status),
        PbftSyncFactStatus::from_u8(report.status),
    )
    .into();
    clear_terminal_sync_admission(&mut runtime, &step);
    step
}

/// Reports the requested transaction-manager lookup result.
pub fn pbft_manager_runtime_pbft_sync_admission_report_transactions(
    runtime: &BridgePbftService,
    report: FfiPbftSyncAdmissionTransactionReport,
) -> FfiPbftSyncAdmissionSessionStep {
    let mut runtime = runtime.manager_state();
    let Some(session) = runtime.pbft_sync_admission_session.as_mut() else {
        return sync_admission_not_started_step();
    };
    let step = report_domain_pbft_sync_admission_transactions(
        session,
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
    .into();
    clear_terminal_sync_admission(&mut runtime, &step);
    step
}

fn clear_terminal_sync_admission(
    runtime: &mut crate::ffi::BridgePbftManagerRuntimeState,
    step: &FfiPbftSyncAdmissionSessionStep,
) {
    if step.complete || !step.can_continue {
        runtime.pbft_sync_admission_session = None;
    }
}

/// Aborts and clears the current synced-period admission cursor.
pub fn abort_pbft_manager_runtime_pbft_sync_admission(
    runtime: &BridgePbftService,
) -> FfiPbftSyncAdmissionSessionStep {
    let mut runtime = runtime.manager_state();
    let step = runtime
        .pbft_sync_admission_session
        .as_mut()
        .map(abort_domain_pbft_sync_admission_session)
        .map(Into::into)
        .unwrap_or_else(sync_admission_not_started_step);
    runtime.pbft_sync_admission_session = None;
    step
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
    let runtime = runtime.manager_state();
    let payload = load_domain_pbft_sync_egress_payload(
        runtime.storage.as_ref(),
        PbftSyncRewardVoteAttachmentFact {
            block_period,
            last_block,
            pbft_chain_synced,
            reward_votes_present,
            reward_votes_period,
        },
    )?;
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
    use crate::pbft_manager::{
        create_pbft_manager_runtime_from_storage, TestPbftManagerStartupFact,
    };
    use crate::storage::create_storage;
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn startup_fact() -> TestPbftManagerStartupFact {
        TestPbftManagerStartupFact {
            current_period: 10,
            cacti_active_at_chain_size: false,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            cacti_block: 1,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
        }
    }

    fn admission_initial_fact() -> FfiPbftSyncAdmissionInitialFact {
        FfiPbftSyncAdmissionInitialFact {
            block_period: 10,
            block_prev_hash: [9; 32],
            chain_last_hash: [9; 32],
            chain_last_period: 9,
            block_in_chain: false,
            dag_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [1; 32] }],
            period_data_transaction_hashes: vec![],
            extra_data_required: false,
            extra_data_present: false,
            extra_data_pillar_block_hash_present: false,
            pillar_votes_required: false,
            pillar_votes_present: false,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

    #[test]
    fn bridge_manager_runtime_owns_sync_admission_cursor() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_sync_admission_cursor");
        let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
        let mut runtime =
            create_pbft_manager_runtime_from_storage(&storage, startup_fact()).unwrap();

        pbft_manager_runtime_begin_pbft_sync_admission(&mut runtime, admission_initial_fact());
        let first = pbft_manager_runtime_pbft_sync_admission_next(&mut runtime);
        assert!(first.has_check);
        assert_eq!(first.cursor, 0);
        assert_eq!(first.next_check, 1);

        let second = pbft_manager_runtime_pbft_sync_admission_report_status(
            &mut runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: first.cursor,
                check: first.next_check,
                status: 0,
            },
        );
        assert_eq!(second.cursor, 1);
        assert_eq!(second.next_check, 2);

        let third = pbft_manager_runtime_pbft_sync_admission_report_status(
            &mut runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: second.cursor,
                check: second.next_check,
                status: 0,
            },
        );
        assert_eq!(third.next_check, 3);
        let transactions = pbft_manager_runtime_pbft_sync_admission_report_status(
            &mut runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: third.cursor,
                check: third.next_check,
                status: 0,
            },
        );
        assert_eq!(transactions.next_check, 4);
        assert_eq!(
            transactions
                .plan
                .transaction_query_plan
                .finalized_lookup_hashes
                .len(),
            1
        );
        let accepted = pbft_manager_runtime_pbft_sync_admission_report_transactions(
            &mut runtime,
            FfiPbftSyncAdmissionTransactionReport {
                cursor: transactions.cursor,
                missing_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [1; 32] }],
                finalized_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [2; 32] }],
                contains_finalized_transactions: true,
            },
        );
        assert!(accepted.complete);
        assert!(accepted.plan.accept_period_data);
        assert_eq!(accepted.plan.warnings.len(), 2);
        assert!(runtime
            .manager_state()
            .pbft_sync_admission_session
            .is_none());
        let missing = pbft_manager_runtime_pbft_sync_admission_next(&mut runtime);
        assert_eq!(
            missing.error_code,
            "PBFT_SYNC_ADMISSION_SESSION_NOT_STARTED"
        );

        drop(runtime);
        drop(storage);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_sync_admission_waits_then_rechecks_same_candidate() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_sync_admission_wait");
        let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
        let mut runtime =
            create_pbft_manager_runtime_from_storage(&storage, startup_fact()).unwrap();
        pbft_manager_runtime_begin_pbft_sync_admission(&mut runtime, admission_initial_fact());
        let first = pbft_manager_runtime_pbft_sync_admission_next(&mut runtime);
        let retry = pbft_manager_runtime_pbft_sync_admission_report_status(
            &mut runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: first.cursor,
                check: first.next_check,
                status: 1,
            },
        );
        assert!(retry.has_check);
        assert!(!retry.complete);
        assert!(retry.plan.wait_for_finalization);
        assert_eq!(retry.next_check, 1);
        let reward = pbft_manager_runtime_pbft_sync_admission_report_status(
            &mut runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: retry.cursor,
                check: retry.next_check,
                status: 0,
            },
        );
        assert_eq!(reward.next_check, 2);

        let mismatch = pbft_manager_runtime_pbft_sync_admission_report_status(
            &mut runtime,
            FfiPbftSyncAdmissionStatusReport {
                cursor: reward.cursor + 1,
                check: reward.next_check,
                status: 0,
            },
        );
        assert!(mismatch.complete);
        assert!(!mismatch.can_continue);
        assert!(runtime
            .manager_state()
            .pbft_sync_admission_session
            .is_none());

        drop(runtime);
        drop(storage);
        let _ = fs::remove_dir_all(temp_dir);
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
    fn bridge_egress_payload_uses_runtime_storage_and_attachment_plan() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_sync_egress");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            storage
                .0
                .pbft()
                .write_manager_field(0, 1)
                .expect("round seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(1, 1)
                .expect("step seed should persist");
            storage
                .0
                .pbft()
                .write_manager_field(2, 1_500)
                .expect("lambda seed should persist");
            storage
                .0
                .period()
                .write(9, &vec![0xC8, 0xC0, 0xC1])
                .expect("period data should persist");

            let runtime = create_pbft_manager_runtime_from_storage(&storage, startup_fact())
                .expect("runtime should restore");
            let payload = load_pbft_sync_egress_payload(&runtime, 9, true, true, true, 9)
                .expect("egress payload should load");

            assert_eq!(payload.period_data_rlp, vec![0xC8, 0xC0, 0xC1]);
            assert!(payload.attach_reward_votes);

            let payload = load_pbft_sync_egress_payload(&runtime, 9, true, true, true, 8)
                .expect("egress payload should load");
            assert!(!payload.attach_reward_votes);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
