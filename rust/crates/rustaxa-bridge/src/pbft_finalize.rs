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
    PbftFinalizationPositionedHash as FfiPbftFinalizationPositionedHash,
    PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
};
use ethereum_types::H256;
use rustaxa_consensus::pbft_finalize::{
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationPlan,
    PbftFinalizationPositionedHash, PbftFinalizationStorageWriteIntent,
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
            block_hash: H256::from(value.block_hash),
            pbft_head_hash: H256::from(value.pbft_head_hash),
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            pivot_dag_anchor_hash: H256::from(value.pivot_dag_anchor_hash),
            has_pillar_block: value.has_pillar_block,
            pillar_block_finalized: value.pillar_block_finalized,
            request_dynamic_lambda_update: value.request_dynamic_lambda_update,
            cert_vote_count: value.cert_vote_count,
            sample_cert_vote_block_hash: H256::from(value.sample_cert_vote_block_hash),
            sample_cert_vote_period: value.sample_cert_vote_period,
            sample_cert_vote_round: value.sample_cert_vote_round,
            sample_cert_vote_step: value.sample_cert_vote_step,
            block_lambda: value.block_lambda,
            last_saved_period_lambda_found: value.last_saved_period_lambda_found,
            last_saved_period_lambda: value.last_saved_period_lambda,
            dynamic_blocks_per_year: value.dynamic_blocks_per_year,
            dpos_blocks_per_year: value.dpos_blocks_per_year,
            period_data_rlp: value.period_data_rlp,
            ordered_dag_block_hashes: value
                .ordered_dag_block_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            ordered_transaction_hashes: value
                .ordered_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
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

impl From<PbftFinalizationStorageWriteIntent> for FfiPbftFinalizationStorageWritePlan {
    fn from(value: PbftFinalizationStorageWriteIntent) -> Self {
        Self {
            persist_pbft_head: value.persist_pbft_head,
            persist_period_data: value.persist_period_data,
            reset_reward_votes: value.reset_reward_votes,
            update_sortition_params: value.update_sortition_params,
            apply_dynamic_lambda_update: value.apply_dynamic_lambda_update,
            persist_period_lambda: value.persist_period_lambda,
            persist_executed_pbft_status: value.persist_executed_pbft_status,
            pbft_block_hash: value.pbft_block_hash.0,
            pbft_head_hash: value.pbft_head_hash.0,
            block_period: value.block_period,
            null_anchor: value.null_anchor,
            reward_vote_period: value.reward_vote_period,
            reward_vote_round: value.reward_vote_round,
            reward_vote_step: value.reward_vote_step,
            reward_vote_block_hash: value.reward_vote_block_hash.0,
            period_lambda: value.period_lambda,
            blocks_per_year: value.blocks_per_year,
            executed_pbft_status: value.executed_pbft_status,
            period_data_rlp: value.period_data_rlp,
            dag_block_period_writes: value
                .dag_block_period_writes
                .into_iter()
                .map(Into::into)
                .collect(),
            transaction_location_writes: value
                .transaction_location_writes
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<PbftFinalizationPositionedHash> for FfiPbftFinalizationPositionedHash {
    fn from(value: PbftFinalizationPositionedHash) -> Self {
        Self {
            hash: value.hash.0,
            position: value.position,
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
            storage_write_intent: value.storage_write_intent.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftFinalizationHash as FfiPbftFinalizationHash;
    use rustaxa_consensus::pbft_finalize::PbftFinalizationAnchor::{Anchored, Null};
    use rustaxa_consensus::pbft_finalize::PbftFinalizationStatus;

    fn fact() -> FfiPbftFinalizationIntentFact {
        FfiPbftFinalizationIntentFact {
            block_hash: [7; 32],
            pbft_head_hash: [8; 32],
            block_period: 10,
            block_prev_hash: [3; 32],
            chain_last_hash: [3; 32],
            chain_last_period: 9,
            block_in_chain: false,
            pivot_dag_anchor_hash: [4; 32],
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: true,
            cert_vote_count: 3,
            sample_cert_vote_block_hash: [7; 32],
            sample_cert_vote_period: 10,
            sample_cert_vote_round: 2,
            sample_cert_vote_step: 5,
            block_lambda: 1_500,
            last_saved_period_lambda_found: false,
            last_saved_period_lambda: 0,
            dynamic_blocks_per_year: 1_000,
            dpos_blocks_per_year: 500,
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![
                FfiPbftFinalizationHash { hash: [1; 32] },
                FfiPbftFinalizationHash { hash: [2; 32] },
            ],
            ordered_transaction_hashes: vec![FfiPbftFinalizationHash { hash: [3; 32] }],
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
        assert!(plan.storage_write_intent.persist_pbft_head);
        assert!(plan.storage_write_intent.persist_period_data);
        assert!(plan.storage_write_intent.reset_reward_votes);
        assert!(plan.storage_write_intent.update_sortition_params);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.persist_executed_pbft_status);
        assert_eq!(plan.storage_write_intent.pbft_block_hash, [7; 32]);
        assert_eq!(plan.storage_write_intent.pbft_head_hash, [8; 32]);
        assert_eq!(plan.storage_write_intent.reward_vote_block_hash, [7; 32]);
        assert_eq!(plan.storage_write_intent.period_lambda, 1_500);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 1_000);
        assert_eq!(plan.storage_write_intent.period_data_rlp, vec![0xc0]);
        assert_eq!(plan.storage_write_intent.dag_block_period_writes.len(), 2);
        assert_eq!(
            plan.storage_write_intent.dag_block_period_writes[1].position,
            1
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes.len(),
            1
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes[0].hash,
            [3; 32]
        );
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
