//! Thin CXX conversions for native pillar-vote application tasks.
//!
//! Rust consensus owns vote admission, relevance, weighted bundles, durable
//! lookups, network chunking, and pillar finalization. This module only unwraps
//! the borrowed native FinalChain handle and converts stable FFI carriers.

use crate::ffi::rustaxa_ffi::{
    PillarBlockFinalizationAcknowledgeRequest, PillarBlockFinalizationAcknowledgeResult,
    PillarBlockFinalizationPrepareResult, PillarBlockFinalizationRequest,
    PillarConsensusThresholdLookup, PillarVoteBundleWithFinalChainPlan, PillarVoteRecord,
    PillarVoteRelevancePlan as FfiPillarVoteRelevancePlan, PillarVoteRlpPayload,
    PillarVoteSingleAdmissionContext,
    PillarVoteSingleAdmissionPreparePlan as FfiPillarVoteSingleAdmissionPreparePlan,
    PillarVoteSingleAdmissionWithFinalChainPlan, PillarVotesPayloadLookup,
};
use crate::ffi::{BridgeFinalChain, BridgePbftService};
use anyhow::Result;

#[allow(dead_code)]
impl BridgePbftService {
    pub fn pbft_service_pillar_validate_single_vote_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<FfiPillarVoteSingleAdmissionPreparePlan> {
        let result = self.0.validate_single_pillar_vote_with_final_chain(
            &final_chain.0,
            vote_rlp,
            native_single_context(context),
        )?;
        Ok(FfiPillarVoteSingleAdmissionPreparePlan {
            status: result.status,
            period: result.period,
            vote_hash: result.vote_hash,
            voter: result.voter,
        })
    }

    pub fn pbft_service_pillar_apply_single_vote_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
        trusted_local_or_restore: bool,
    ) -> Result<PillarVoteSingleAdmissionWithFinalChainPlan> {
        let result = self.0.apply_single_pillar_vote_with_final_chain(
            &final_chain.0,
            vote_rlp,
            native_single_context(context),
            trusted_local_or_restore,
        )?;
        Ok(PillarVoteSingleAdmissionWithFinalChainPlan {
            status: result.status,
            accepted: result.accepted,
            duplicate: result.duplicate,
            conflict_found: result.conflict_found,
            conflicting_vote_hash: result.conflicting_vote_hash,
            block_weight: result.block_weight,
            validator_vote_count: result.validator_vote_count,
            period: result.period,
            vote_hash: result.vote_hash,
            voter: result.voter,
        })
    }

    pub fn pbft_service_pillar_consensus_threshold_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        period: u64,
    ) -> Result<PillarConsensusThresholdLookup> {
        let result = self
            .0
            .pillar_consensus_threshold_with_final_chain(&final_chain.0, period)?;
        Ok(PillarConsensusThresholdLookup {
            available: result.available,
            threshold: result.threshold,
            error_code: result.error_code,
        })
    }

    pub fn pbft_service_pillar_plan_vote_relevance(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<FfiPillarVoteRelevancePlan> {
        let result = self.0.plan_pillar_vote_relevance(
            vote_rlp,
            rustaxa_consensus::pillar_vote_service::PillarVoteRuntimeRelevanceContext {
                first_pillar_block_period: context.first_pillar_block_period,
                pillar_blocks_interval: context.pillar_blocks_interval,
            },
        )?;
        Ok(FfiPillarVoteRelevancePlan {
            status: result.status,
            is_relevant: result.is_relevant,
        })
    }

    pub fn pbft_service_pillar_apply_rlp_bundle_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        vote_rlps: Vec<PillarVoteRlpPayload>,
        required_votes_period: u64,
    ) -> Result<PillarVoteBundleWithFinalChainPlan> {
        let result = self.0.apply_pillar_vote_bundle_with_final_chain(
            &final_chain.0,
            vote_rlps
                .into_iter()
                .map(
                    |value| rustaxa_consensus::pillar_vote_service::PillarVoteRlpPayload {
                        vote_rlp: value.vote_rlp,
                    },
                )
                .collect(),
            required_votes_period,
        )?;
        Ok(PillarVoteBundleWithFinalChainPlan {
            prepare_status: result.prepare_status,
            missing_threshold: result.missing_threshold,
            status: result.status,
            block_weight: result.block_weight,
            selected_weight: result.selected_weight,
            first_bad_vote_hash: result.first_bad_vote_hash,
            insert_failed: result.insert_failed,
            insert_failed_vote_hash: result.insert_failed_vote_hash,
            applied_votes: result.applied_votes,
        })
    }

    pub fn pbft_service_pillar_get_verified_vote_payloads(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> Result<PillarVotesPayloadLookup> {
        let result = self
            .0
            .pillar_verified_vote_payloads(period, block_hash, above_threshold)?;
        Ok(native_payload_lookup_to_ffi(result))
    }

    pub fn pbft_service_pillar_prepare_finalized_block_for_pbft(
        &self,
        request: PillarBlockFinalizationRequest,
    ) -> Result<PillarBlockFinalizationPrepareResult> {
        let result = self.0.prepare_pillar_block_finalization(
            rustaxa_consensus::pillar_vote_service::PillarBlockFinalizationRequest {
                requested_pillar_block_hash: request.requested_pillar_block_hash,
            },
        )?;
        Ok(PillarBlockFinalizationPrepareResult {
            status: result.status,
            success: result.success,
            should_request_votes: result.should_request_votes,
            has_request_votes_period: result.has_request_votes_period,
            request_votes_period: result.request_votes_period,
            should_emit: result.should_emit,
            current_period: result.current_period,
            current_hash: result.current_hash,
            block_weight: result.block_weight,
            selected_weight: result.selected_weight,
            selected_vote_count: result.selected_vote_count,
            prepared_pillar_block_period: result.prepared_pillar_block_period,
            prepared_pillar_block_rlp: result.prepared_pillar_block_rlp,
            has_prepared_pillar_block: result.has_prepared_pillar_block,
            preparation_anchor_generation: result.preparation_anchor_generation,
            preparation_token: result.preparation_token,
            votes: result
                .votes
                .into_iter()
                .map(native_vote_record_to_ffi)
                .collect(),
        })
    }

    pub fn pbft_service_pillar_ack_finalize_block_for_pbft(
        &self,
        request: PillarBlockFinalizationAcknowledgeRequest,
    ) -> Result<PillarBlockFinalizationAcknowledgeResult> {
        let result = self.0.acknowledge_pillar_block_finalization(
            rustaxa_consensus::pillar_vote_service::PillarBlockFinalizationAcknowledgeRequest {
                anchor_generation: request.anchor_generation,
                preparation_token: request.preparation_token,
            },
        )?;
        Ok(PillarBlockFinalizationAcknowledgeResult {
            should_emit: result.should_emit,
            latest_finalized_period: result.latest_finalized_period,
            latest_finalized_hash: result.latest_finalized_hash,
        })
    }
}

fn native_single_context(
    value: PillarVoteSingleAdmissionContext,
) -> rustaxa_consensus::pillar_vote_service::PillarVoteSingleAdmissionContext {
    rustaxa_consensus::pillar_vote_service::PillarVoteSingleAdmissionContext {
        first_pillar_block_period: value.first_pillar_block_period,
        pillar_blocks_interval: value.pillar_blocks_interval,
    }
}

fn native_vote_record_to_ffi(
    value: rustaxa_consensus::pillar_vote_service::PillarVoteRecord,
) -> PillarVoteRecord {
    PillarVoteRecord {
        vote_hash: value.vote_hash,
        weight: value.weight,
        vote_rlp: value.vote_rlp,
    }
}

fn native_payload_lookup_to_ffi(
    value: rustaxa_consensus::pillar_vote_service::PillarVotesPayloadLookup,
) -> PillarVotesPayloadLookup {
    PillarVotesPayloadLookup {
        threshold_met: value.threshold_met,
        block_weight: value.block_weight,
        selected_weight: value.selected_weight,
        votes: value
            .votes
            .into_iter()
            .map(native_vote_record_to_ffi)
            .collect(),
    }
}
