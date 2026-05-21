//! CXX bridge wrappers for PBFT sync-period admission planning.
//!
//! C++ supplies compact fact codes and transaction hash vectors gathered from
//! existing shim-owned consensus surfaces. Rust converts them into typed domain
//! facts, runs the deterministic planner, and returns u8-coded side-effect
//! intent flags for the PBFT manager overlay to apply.

use crate::ffi::rustaxa_ffi::{
    PbftSyncPeriodAdmissionFact as FfiPbftSyncPeriodAdmissionFact,
    PbftSyncPeriodAdmissionPlan as FfiPbftSyncPeriodAdmissionPlan,
    PbftSyncProcessPeriodDataRuntimeFact as FfiPbftSyncProcessPeriodDataRuntimeFact,
    PbftSyncProcessPeriodDataRuntimePlan as FfiPbftSyncProcessPeriodDataRuntimePlan,
    PbftSyncRuntimePlan as FfiPbftSyncRuntimePlan,
    PbftSyncTransactionHash as FfiPbftSyncTransactionHash,
    PbftSyncTransactionQueryFact as FfiPbftSyncTransactionQueryFact,
    PbftSyncTransactionQueryPlan as FfiPbftSyncTransactionQueryPlan,
    PbftSyncTransactionWarning as FfiPbftSyncTransactionWarning,
};
use ethereum_types::H256;
use rustaxa_consensus::pbft_sync::{
    plan_pbft_sync_period_admission_runtime as plan_domain_pbft_sync_period_admission_runtime,
    plan_pbft_sync_process_period_data_runtime as plan_domain_pbft_sync_process_period_data_runtime,
    plan_pbft_sync_runtime as plan_domain_pbft_sync_runtime,
    plan_pbft_sync_transaction_query as plan_domain_pbft_sync_transaction_query,
    PbftSyncFactStatus, PbftSyncFinalChainHashStatus, PbftSyncPeriodAdmissionFact,
    PbftSyncPeriodAdmissionPlan, PbftSyncProcessPeriodDataRuntimeFact,
    PbftSyncProcessPeriodDataRuntimePlan, PbftSyncRuntimeFinalChainHashStatus, PbftSyncRuntimePlan,
    PbftSyncTransactionQueryFact, PbftSyncTransactionQueryPlan, PbftSyncTransactionWarning,
};

/// Plans admission for one C++-originated synced PBFT period payload.
pub fn plan_pbft_sync_period_admission(
    fact: FfiPbftSyncPeriodAdmissionFact,
) -> FfiPbftSyncPeriodAdmissionPlan {
    plan_domain_pbft_sync_period_admission_runtime(fact.into())
        .into_plan()
        .into()
}

/// Builds one combined PBFT sync runtime plan from one call.
///
/// This is a side-effect-free orchestration function for `processPeriodData()` paths.
pub fn plan_pbft_sync_runtime(
    period_admission_fact: FfiPbftSyncPeriodAdmissionFact,
    transaction_query_fact: FfiPbftSyncTransactionQueryFact,
) -> FfiPbftSyncRuntimePlan {
    plan_domain_pbft_sync_runtime(period_admission_fact.into(), transaction_query_fact.into())
        .into()
}

/// Plans the next staged PBFT sync runtime action for C++ `processPeriodData`.
pub fn plan_pbft_sync_process_period_data_runtime(
    fact: FfiPbftSyncProcessPeriodDataRuntimeFact,
) -> FfiPbftSyncProcessPeriodDataRuntimePlan {
    plan_domain_pbft_sync_process_period_data_runtime(fact.into()).into()
}

/// Plans finalized-transaction lookups for synced PBFT period data.
pub fn plan_pbft_sync_transaction_query(
    fact: FfiPbftSyncTransactionQueryFact,
) -> FfiPbftSyncTransactionQueryPlan {
    plan_domain_pbft_sync_transaction_query(fact.into()).into()
}

impl From<FfiPbftSyncPeriodAdmissionFact> for PbftSyncPeriodAdmissionFact {
    fn from(value: FfiPbftSyncPeriodAdmissionFact) -> Self {
        Self {
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            final_chain_hash_status: PbftSyncFinalChainHashStatus::from_u8(
                value.final_chain_hash_status,
            ),
            reward_votes_status: PbftSyncFactStatus::from_u8(value.reward_votes_status),
            cert_votes_status: PbftSyncFactStatus::from_u8(value.cert_votes_status),
            missing_transaction_hashes: value
                .missing_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            finalized_transaction_hashes: value
                .finalized_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            contains_finalized_transactions: value.contains_finalized_transactions,
            pillar_data_status: PbftSyncFactStatus::from_u8(value.pillar_data_status),
            pillar_votes_status: PbftSyncFactStatus::from_u8(value.pillar_votes_status),
        }
    }
}

impl From<FfiPbftSyncTransactionQueryFact> for PbftSyncTransactionQueryFact {
    fn from(value: FfiPbftSyncTransactionQueryFact) -> Self {
        Self {
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
        }
    }
}

impl From<FfiPbftSyncProcessPeriodDataRuntimeFact> for PbftSyncProcessPeriodDataRuntimeFact {
    fn from(value: FfiPbftSyncProcessPeriodDataRuntimeFact) -> Self {
        Self {
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            final_chain_hash_status: PbftSyncRuntimeFinalChainHashStatus::from_u8(
                value.final_chain_hash_status,
            ),
            reward_votes_status: PbftSyncFactStatus::from_u8(value.reward_votes_status),
            cert_votes_status: PbftSyncFactStatus::from_u8(value.cert_votes_status),
            transactions_status: PbftSyncFactStatus::from_u8(value.transactions_status),
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
            missing_transaction_hashes: value
                .missing_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            finalized_transaction_hashes: value
                .finalized_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            contains_finalized_transactions: value.contains_finalized_transactions,
            pillar_data_status: PbftSyncFactStatus::from_u8(value.pillar_data_status),
            pillar_votes_required: value.pillar_votes_required,
            pillar_votes_status: PbftSyncFactStatus::from_u8(value.pillar_votes_status),
            previous_cert_votes_present: value.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: value.previous_cert_first_vote_has_weight,
        }
    }
}

impl From<PbftSyncPeriodAdmissionPlan> for FfiPbftSyncPeriodAdmissionPlan {
    fn from(plan: PbftSyncPeriodAdmissionPlan) -> Self {
        Self {
            decision: plan.decision.as_u8(),
            status: plan.status.as_u8(),
            clear_sync_queue: plan.clear_sync_queue,
            report_malicious_peer: plan.report_malicious_peer,
            wait_for_finalization: plan.wait_for_finalization,
            accept_period_data: plan.accept_period_data,
            warnings: plan
                .warnings
                .into_iter()
                .map(FfiPbftSyncTransactionWarning::from)
                .collect(),
            contains_finalized_transaction_warning: plan.contains_finalized_transaction_warning,
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

impl From<PbftSyncRuntimePlan> for FfiPbftSyncRuntimePlan {
    fn from(plan: PbftSyncRuntimePlan) -> Self {
        Self {
            action: plan.period_admission.action.as_u8(),
            period_admission_plan: plan.period_admission.plan.into(),
            transaction_query_plan: plan.transaction_query.into(),
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
    use rustaxa_consensus::pbft_sync::{
        PbftSyncAdmissionRuntimeAction, PbftSyncPeriodAdmissionDecision,
        PbftSyncPeriodAdmissionStatus, PbftSyncTransactionWarningKind,
    };

    fn fact() -> FfiPbftSyncPeriodAdmissionFact {
        FfiPbftSyncPeriodAdmissionFact {
            block_period: 101,
            block_prev_hash: [1; 32],
            chain_last_hash: [1; 32],
            chain_last_period: 100,
            block_in_chain: false,
            final_chain_hash_status: PbftSyncFinalChainHashStatus::Valid.as_u8(),
            reward_votes_status: PbftSyncFactStatus::Valid.as_u8(),
            cert_votes_status: PbftSyncFactStatus::Valid.as_u8(),
            missing_transaction_hashes: vec![],
            finalized_transaction_hashes: vec![],
            contains_finalized_transactions: false,
            pillar_data_status: PbftSyncFactStatus::Valid.as_u8(),
            pillar_votes_status: PbftSyncFactStatus::NotRequired.as_u8(),
        }
    }

    #[test]
    fn bridge_plan_accepts_and_carries_warning_signals() {
        let mut fact = fact();
        fact.missing_transaction_hashes = vec![
            FfiPbftSyncTransactionHash { hash: [7; 32] },
            FfiPbftSyncTransactionHash { hash: [8; 32] },
        ];
        fact.finalized_transaction_hashes = vec![FfiPbftSyncTransactionHash { hash: [9; 32] }];
        fact.contains_finalized_transactions = true;

        let plan = plan_pbft_sync_period_admission(fact);

        assert_eq!(
            plan.decision,
            PbftSyncPeriodAdmissionDecision::Accept.as_u8()
        );
        assert_eq!(plan.status, PbftSyncPeriodAdmissionStatus::Accepted.as_u8());
        assert!(plan.accept_period_data);
        assert!(!plan.clear_sync_queue);
        assert_eq!(plan.warnings.len(), 3);
        assert_eq!(
            plan.warnings[0].kind,
            PbftSyncTransactionWarningKind::MissingTransaction.as_u8()
        );
        assert!(plan.contains_finalized_transaction_warning);
    }

    #[test]
    fn bridge_plan_rejects_prev_hash_mismatch() {
        let mut fact = fact();
        fact.block_prev_hash = [2; 32];

        let plan = plan_pbft_sync_period_admission(fact);

        assert_eq!(
            plan.decision,
            PbftSyncPeriodAdmissionDecision::ClearAndReportPeer.as_u8()
        );
        assert_eq!(
            plan.status,
            PbftSyncPeriodAdmissionStatus::PreviousHashMismatch.as_u8()
        );
        assert!(plan.clear_sync_queue);
        assert!(plan.report_malicious_peer);
    }

    #[test]
    fn bridge_plan_waits_for_missing_final_chain() {
        let mut fact = fact();
        fact.final_chain_hash_status = PbftSyncFinalChainHashStatus::Missing.as_u8();

        let plan = plan_pbft_sync_period_admission(fact);

        assert_eq!(
            plan.decision,
            PbftSyncPeriodAdmissionDecision::WaitForFinalization.as_u8()
        );
        assert!(plan.wait_for_finalization);
        assert!(!plan.report_malicious_peer);
    }

    #[test]
    fn bridge_transaction_query_preserves_rust_planned_lookup_order() {
        let plan = plan_pbft_sync_transaction_query(FfiPbftSyncTransactionQueryFact {
            dag_transaction_hashes: vec![
                FfiPbftSyncTransactionHash { hash: [1; 32] },
                FfiPbftSyncTransactionHash { hash: [2; 32] },
                FfiPbftSyncTransactionHash { hash: [1; 32] },
                FfiPbftSyncTransactionHash { hash: [3; 32] },
            ],
            period_data_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [2; 32] }],
        });

        assert_eq!(
            plan.finalized_lookup_hashes
                .into_iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>(),
            vec![[1; 32], [3; 32]]
        );
    }

    #[test]
    fn bridge_runtime_plan_wraps_admission_and_transaction_lookup() {
        let plan = plan_pbft_sync_runtime(
            fact(),
            FfiPbftSyncTransactionQueryFact {
                dag_transaction_hashes: vec![
                    FfiPbftSyncTransactionHash { hash: [1; 32] },
                    FfiPbftSyncTransactionHash { hash: [1; 32] },
                    FfiPbftSyncTransactionHash { hash: [2; 32] },
                ],
                period_data_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [2; 32] }],
            },
        );

        assert_eq!(plan.action, PbftSyncAdmissionRuntimeAction::Accept.as_u8());
        assert_eq!(
            plan.period_admission_plan.decision,
            PbftSyncPeriodAdmissionDecision::Accept.as_u8()
        );
        assert_eq!(
            plan.period_admission_plan.status,
            PbftSyncPeriodAdmissionStatus::Accepted.as_u8()
        );
        assert_eq!(
            plan.transaction_query_plan
                .finalized_lookup_hashes
                .into_iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>(),
            vec![[1; 32]]
        );
    }

    fn runtime_fact() -> FfiPbftSyncProcessPeriodDataRuntimeFact {
        FfiPbftSyncProcessPeriodDataRuntimeFact {
            block_period: 101,
            block_prev_hash: [1; 32],
            chain_last_hash: [1; 32],
            chain_last_period: 100,
            block_in_chain: false,
            final_chain_hash_status: PbftSyncRuntimeFinalChainHashStatus::NotChecked.as_u8(),
            reward_votes_status: PbftSyncFactStatus::NotChecked.as_u8(),
            cert_votes_status: PbftSyncFactStatus::NotChecked.as_u8(),
            transactions_status: PbftSyncFactStatus::NotChecked.as_u8(),
            dag_transaction_hashes: vec![
                FfiPbftSyncTransactionHash { hash: [4; 32] },
                FfiPbftSyncTransactionHash { hash: [5; 32] },
            ],
            period_data_transaction_hashes: vec![FfiPbftSyncTransactionHash { hash: [5; 32] }],
            missing_transaction_hashes: vec![],
            finalized_transaction_hashes: vec![],
            contains_finalized_transactions: false,
            pillar_data_status: PbftSyncFactStatus::NotChecked.as_u8(),
            pillar_votes_required: true,
            pillar_votes_status: PbftSyncFactStatus::NotChecked.as_u8(),
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

    #[test]
    fn bridge_process_period_runtime_requests_staged_checks() {
        let plan = plan_pbft_sync_process_period_data_runtime(runtime_fact());

        assert_eq!(plan.runtime_action, 0);
        assert_eq!(plan.next_check, 1);
        assert!(!plan.accept_period_data);
        assert!(plan.replace_previous_block_cert_votes);
    }

    #[test]
    fn bridge_process_period_runtime_accepts_after_all_checks() {
        let mut fact = runtime_fact();
        fact.final_chain_hash_status = PbftSyncRuntimeFinalChainHashStatus::Valid.as_u8();
        fact.reward_votes_status = PbftSyncFactStatus::Valid.as_u8();
        fact.cert_votes_status = PbftSyncFactStatus::Valid.as_u8();
        fact.transactions_status = PbftSyncFactStatus::Valid.as_u8();
        fact.missing_transaction_hashes = vec![FfiPbftSyncTransactionHash { hash: [4; 32] }];
        fact.contains_finalized_transactions = true;
        fact.pillar_data_status = PbftSyncFactStatus::Valid.as_u8();
        fact.pillar_votes_status = PbftSyncFactStatus::Valid.as_u8();

        let plan = plan_pbft_sync_process_period_data_runtime(fact);

        assert_eq!(plan.runtime_action, 1);
        assert_eq!(plan.next_check, 0);
        assert!(plan.accept_period_data);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.contains_finalized_transaction_warning);
        assert_eq!(
            plan.transaction_query_plan
                .finalized_lookup_hashes
                .into_iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>(),
            vec![[4; 32]]
        );
    }
}
