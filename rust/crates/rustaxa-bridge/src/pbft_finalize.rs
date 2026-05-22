//! Bridge wrapper for PBFT finalization intent planning.
//!
//! C++ passes a compact, plain fact bundle gathered from existing execute/finalize
//! flow steps (validation, pillar-finalization check, anchor classification, etc.).
//! Rust performs deterministic intent planning and returns bridge-safe flags and
//! status codes so C++ can apply side effects explicitly.

use crate::ffi::rustaxa_ffi::{
    PbftFinalizationCleanupPlan as FfiPbftFinalizationCleanupPlan,
    PbftFinalizationIntentFact as FfiPbftFinalizationIntentFact,
    PbftFinalizationIntentPlan as FfiPbftFinalizationIntentPlan,
};
use ethereum_types::H256;
use rustaxa_consensus::pbft_finalize::{
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationPlan,
};

/// C++/Rust bridge entry for one deterministic PBFT finalization intent.
pub fn plan_pbft_finalization_intent(
    fact: FfiPbftFinalizationIntentFact,
) -> FfiPbftFinalizationIntentPlan {
    plan_domain_pbft_finalization_intent(fact.into()).into()
}

impl From<FfiPbftFinalizationIntentFact> for PbftFinalizationIntentFact {
    fn from(value: FfiPbftFinalizationIntentFact) -> Self {
        Self {
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            pivot_dag_anchor_hash: H256::from(value.pivot_dag_anchor_hash),
            has_pillar_block: value.has_pillar_block,
            pillar_block_finalized: value.pillar_block_finalized,
            request_dynamic_lambda_update: value.request_dynamic_lambda_update,
        }
    }
}

impl From<PbftFinalizationCleanupIntent> for FfiPbftFinalizationCleanupPlan {
    fn from(value: PbftFinalizationCleanupIntent) -> Self {
        Self {
            persist_pbft_block_metadata: value.persist_pbft_block_metadata,
            reset_reward_votes: value.reset_reward_votes,
            set_dag_block_order: value.set_dag_block_order,
            update_sortition_params: value.update_sortition_params,
            update_finalized_transactions_status: value.update_finalized_transactions_status,
            update_pbft_chain: value.update_pbft_chain,
            clear_anchor_dag_cache: value.clear_anchor_dag_cache,
            finalize_final_chain: value.finalize_final_chain,
            maybe_update_dynamic_lambda: value.maybe_update_dynamic_lambda,
            advance_period: value.advance_period,
        }
    }
}

impl From<PbftFinalizationPlan> for FfiPbftFinalizationIntentPlan {
    fn from(value: PbftFinalizationPlan) -> Self {
        Self {
            finalize_block: value.finalize_block,
            anchor: value.anchor.as_u8(),
            executed_pbft_block: value.executed_pbft_block,
            status: value.status.as_u8(),
            cleanup: value.cleanup.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_consensus::pbft_finalize::PbftFinalizationAnchor::{Anchored, Null};
    use rustaxa_consensus::pbft_finalize::PbftFinalizationStatus;

    fn fact() -> FfiPbftFinalizationIntentFact {
        FfiPbftFinalizationIntentFact {
            block_period: 10,
            block_prev_hash: [3; 32],
            chain_last_hash: [3; 32],
            chain_last_period: 9,
            block_in_chain: false,
            pivot_dag_anchor_hash: [4; 32],
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: true,
        }
    }

    #[test]
    fn bridge_bridge_accepts_anchored_block_and_maps_cleanup_intent() {
        let plan = plan_pbft_finalization_intent(fact());

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, Anchored.as_u8());
        assert_eq!(plan.status, PbftFinalizationStatus::Accepted.as_u8());
        assert!(plan.executed_pbft_block);
        assert!(plan.cleanup.persist_pbft_block_metadata);
        assert!(plan.cleanup.update_sortition_params);
        assert!(plan.cleanup.set_dag_block_order);
    }

    #[test]
    fn bridge_maps_anchor_and_status_for_null_and_rejects() {
        let mut rejected = fact();
        rejected.pivot_dag_anchor_hash = [0; 32];
        rejected.has_pillar_block = true;
        rejected.pillar_block_finalized = false;

        let rejected_plan = plan_pbft_finalization_intent(rejected);
        assert!(!rejected_plan.finalize_block);
        assert_eq!(rejected_plan.anchor, Null.as_u8());
        assert_eq!(
            rejected_plan.status,
            PbftFinalizationStatus::PillarDependencyMissing.as_u8()
        );
    }
}
