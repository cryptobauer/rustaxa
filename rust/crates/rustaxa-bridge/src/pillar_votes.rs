//! Thin CXX conversions for native pillar-vote application tasks.
//!
//! Rust consensus owns vote admission, relevance, weighted bundles, durable
//! lookups, network chunking, and pillar finalization. This module only unwraps
//! the borrowed native FinalChain handle and converts stable FFI carriers.

use crate::ffi::rustaxa_ffi::{
    PillarBlockFinalizationAcknowledgeRequest, PillarBlockFinalizationAcknowledgeResult,
    PillarBlockFinalizationPrepareResult, PillarBlockFinalizationRequest,
    PillarConsensusThresholdLookup, PillarVoteBundleHash, PillarVoteBundleWithFinalChainPlan,
    PillarVoteInspection, PillarVoteNetworkBundleChunk, PillarVoteNetworkBundleLookup,
    PillarVoteRecord, PillarVoteRelevancePlan as FfiPillarVoteRelevancePlan, PillarVoteRlpPayload,
    PillarVoteRuntimeRelevanceContext, PillarVoteSingleAdmissionContext,
    PillarVoteSingleAdmissionPreparePlan as FfiPillarVoteSingleAdmissionPreparePlan,
    PillarVoteSingleAdmissionWithFinalChainPlan, PillarVotesPayloadLookup,
};
use crate::ffi::{BridgeFinalChain, BridgePbftService};
use anyhow::Result;
use rustaxa_consensus::{
    inspect_pillar_vote_from_rlp, PillarVoteInspection as ConsensusPillarVoteInspection,
};

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
        context: PillarVoteRuntimeRelevanceContext,
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

    pub fn pbft_service_pillar_build_verified_vote_network_bundles(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        max_votes_per_bundle: usize,
    ) -> Result<PillarVoteNetworkBundleLookup> {
        let result =
            self.0
                .build_pillar_vote_network_bundles(period, block_hash, max_votes_per_bundle)?;
        Ok(PillarVoteNetworkBundleLookup {
            from_storage: result.from_storage,
            chunks: result
                .chunks
                .into_iter()
                .map(|chunk| PillarVoteNetworkBundleChunk {
                    vote_hashes: chunk
                        .vote_hashes
                        .into_iter()
                        .map(|hash| PillarVoteBundleHash { hash: hash.hash })
                        .collect(),
                    votes_bundle_rlp: chunk.votes_bundle_rlp,
                })
                .collect(),
        })
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

/// Inspects a legacy-encoded PillarVote payload without mutating state.
///
/// Use this before inserting a vote to recover voter/address and check
/// signature validity from vote RLP alone.
pub fn pillar_vote_inspect(vote_rlp: &[u8]) -> Result<PillarVoteInspection> {
    Ok(inspect_pillar_vote_from_rlp(vote_rlp)?.into())
}

impl From<ConsensusPillarVoteInspection> for PillarVoteInspection {
    fn from(value: ConsensusPillarVoteInspection) -> Self {
        Self {
            status: u8::from(!value.signature_valid),
            period: value.period,
            block_hash: value.block_hash.into(),
            vote_hash: value.vote_hash.into(),
            voter: value.voter.into(),
            signature_valid: value.signature_valid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use k256::ecdsa::SigningKey;
    use rustaxa_types::PillarVote;

    fn keccak256(data: &[u8]) -> H256 {
        use tiny_keccak::{Hasher, Keccak};

        let mut output = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(data);
        hasher.finalize(&mut output);
        H256::from(output)
    }

    fn signed_vote(seed: u8, period: u64, block: u64) -> (PillarVote, [u8; 20]) {
        let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
        signed_vote_with_key(&signing_key, period, block)
    }

    fn signed_vote_with_key(
        signing_key: &SigningKey,
        period: u64,
        block: u64,
    ) -> (PillarVote, [u8; 20]) {
        signed_vote_with_key_and_hash(signing_key, period, H256::from_low_u64_be(block))
    }

    fn signed_vote_with_key_and_hash(
        signing_key: &SigningKey,
        period: u64,
        block_hash: H256,
    ) -> (PillarVote, [u8; 20]) {
        let mut vote = PillarVote {
            period,
            block_hash,
            signature: [0u8; 65],
        };
        let unsigned_hash = vote.hash(false);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(unsigned_hash.as_bytes())
            .unwrap();
        let signature_bytes_fixed = signature.to_bytes();
        let mut signature_bytes = [0u8; 65];
        signature_bytes[..64].copy_from_slice(&signature_bytes_fixed);
        signature_bytes[64] = recovery_id.to_byte();
        vote.signature = signature_bytes;

        let voter = {
            let verifying_key = signing_key.verifying_key();
            let public_key = verifying_key.to_encoded_point(false);
            let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
            public_key_hash.as_bytes()[12..].try_into().unwrap()
        };

        (vote, voter)
    }

    #[test]
    fn inspect_pillar_vote_recovers_voter_and_signature_status() {
        let (vote, voter) = signed_vote(0x11, 9_999, 77);
        let inspected = pillar_vote_inspect(&vote.encode_rlp()).unwrap();

        assert!(inspected.signature_valid);
        assert_eq!(inspected.status, 0);
        assert_eq!(inspected.period, 9_999);
        assert_eq!(H256::from(inspected.block_hash), H256::from_low_u64_be(77));
        assert_eq!(H256::from(inspected.vote_hash), vote.hash(true));
        assert_eq!(inspected.voter, voter);
    }

    #[test]
    fn inspect_pillar_vote_reports_invalid_signature_without_error() {
        let (mut vote, _) = signed_vote(0x12, 100, 78);
        vote.signature = [0u8; 65];

        let inspected = pillar_vote_inspect(&vote.encode_rlp()).unwrap();

        assert!(!inspected.signature_valid);
        assert_eq!(inspected.status, 1);
        assert_eq!(inspected.voter, [0u8; 20]);
    }

    #[test]
    fn inspect_pillar_vote_rejects_out_of_range_recovery_id() {
        let (mut vote, _) = signed_vote(0x13, 101, 79);
        vote.signature[64] = 4;

        let inspected = pillar_vote_inspect(&vote.encode_rlp()).unwrap();

        assert!(!inspected.signature_valid);
        assert_eq!(inspected.status, 1);
        assert_eq!(inspected.voter, [0u8; 20]);
    }

    #[test]
    fn inspect_pillar_vote_rejects_malformed_rlp() {
        assert!(pillar_vote_inspect(&[1, 2, 3]).is_err());
    }
}
