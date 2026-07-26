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
        let result = self
            .pillar
            .pbft_service_pillar_validate_single_vote_with_final_chain(
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
        let result = self
            .pillar
            .pbft_service_pillar_apply_single_vote_with_final_chain(
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
            .pillar
            .pbft_service_pillar_consensus_threshold_with_final_chain(&final_chain.0, period)?;
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
        let result = self.pillar.pbft_service_pillar_plan_vote_relevance(
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
        let result = self
            .pillar
            .pbft_service_pillar_apply_rlp_bundle_with_final_chain(
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
        let result = self.pillar.pbft_service_pillar_get_verified_vote_payloads(
            period,
            block_hash,
            above_threshold,
        )?;
        Ok(native_payload_lookup_to_ffi(result))
    }

    pub fn pbft_service_pillar_build_verified_vote_network_bundles(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        max_votes_per_bundle: usize,
    ) -> Result<PillarVoteNetworkBundleLookup> {
        let result = self
            .pillar
            .pbft_service_pillar_build_verified_vote_network_bundles(
                period,
                block_hash,
                max_votes_per_bundle,
            )?;
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
        let result = self
            .pillar
            .pbft_service_pillar_prepare_finalized_block_for_pbft(
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
        let result = self
            .pillar
            .pbft_service_pillar_ack_finalize_block_for_pbft(
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
    use crate::ffi::rustaxa_ffi;
    use crate::final_chain::create_final_chain;
    use crate::pillar_chain::create_pillar_test_service_from_storage;
    use crate::storage::create_storage;
    use ethereum_types::H256;
    use k256::ecdsa::SigningKey;
    use rustaxa_types::{CurrentPillarBlockDataDb, PillarBlock, PillarVote};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn u256_be(value: u64) -> Vec<u8> {
        ethereum_types::U256::from(value).to_big_endian().to_vec()
    }

    fn final_chain_for_voters(
        storage: &crate::ffi::BridgeStorage,
        voters: &[[u8; 20]],
    ) -> Box<BridgeFinalChain> {
        let validators = voters
            .iter()
            .map(|address| rustaxa_ffi::GenesisValidator {
                address: *address,
                owner: *address,
                vrf_key: [address[0]; 32],
                commission: 0,
                description: String::new(),
                endpoint: String::new(),
                total_stake: u256_be(5_000),
                delegations: vec![rustaxa_ffi::GenesisDelegation {
                    delegator: *address,
                    stake: u256_be(5_000),
                }],
            })
            .collect();
        create_final_chain(
            storage,
            0,
            0,
            Vec::new(),
            validators,
            rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: u256_be(1_000),
                vote_eligibility_balance_step: u256_be(1_000),
                validator_maximum_stake: u256_be(30_000),
                minimum_deposit: Vec::new(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .unwrap()
    }

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

    fn current_data(period: u64) -> (PillarBlock, Vec<u8>) {
        let block = PillarBlock {
            period,
            state_root: H256::from_low_u64_be(1),
            previous_pillar_block_hash: H256::from_low_u64_be(2),
            bridge_root: H256::from_low_u64_be(3),
            epoch: 4,
            validator_vote_count_changes: Vec::new(),
        };
        let bytes = CurrentPillarBlockDataDb {
            pillar_block: block.clone(),
            vote_counts: Vec::new(),
        }
        .encode_rlp();
        (block, bytes)
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

    #[test]
    fn composed_single_vote_maps_zero_weight_and_cleans_exact_preparation() {
        let storage_dir = unique_temp_dir("pillar_composed_single_zero");
        {
            let storage = create_storage(storage_dir.to_str().unwrap()).unwrap();
            let final_chain = final_chain_for_voters(&storage, &[]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(0);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(
                &SigningKey::from_slice(&[0x71; 32]).unwrap(),
                1,
                anchor.hash(),
            );

            let plan = runtime
                .pbft_service_pillar_validate_single_vote_with_final_chain(
                    &final_chain,
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert_eq!(plan.status, 7);
            assert_eq!(plan.period, 1);
            assert_eq!(plan.vote_hash, vote.hash(true).0);

            let missing = runtime
                .pillar
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    rustaxa_consensus::pillar_vote_service::PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 5,
                        has_threshold: false,
                        threshold: 0,
                    },
                )
                .unwrap();
            assert_eq!(missing.status, 11);
        }
        let _ = fs::remove_dir_all(storage_dir);
    }

    #[test]
    fn composed_single_vote_checked_apply_queries_weight_and_threshold() {
        let storage_dir = unique_temp_dir("pillar_composed_single_apply");
        {
            let storage = create_storage(storage_dir.to_str().unwrap()).unwrap();
            let key = SigningKey::from_slice(&[0x72; 32]).unwrap();
            let (_, voter) = signed_vote_with_key(&key, 1, 1);
            let final_chain = final_chain_for_voters(&storage, &[voter]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(0);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 1, anchor.hash());

            let applied = runtime
                .pbft_service_pillar_apply_single_vote_with_final_chain(
                    &final_chain,
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 10,
                    },
                    false,
                )
                .unwrap();
            assert_eq!(applied.status, 0);
            assert!(applied.accepted);
            assert!(applied.validator_vote_count > 0);
            assert_eq!(applied.voter, voter);
        }
        let _ = fs::remove_dir_all(storage_dir);
    }

    #[test]
    fn composed_bundle_distinguishes_missing_total_from_first_zero_weight() {
        let future_dir = unique_temp_dir("pillar_composed_bundle_future");
        {
            let storage = create_storage(future_dir.to_str().unwrap()).unwrap();
            let key = SigningKey::from_slice(&[0x73; 32]).unwrap();
            let (_, voter) = signed_vote_with_key(&key, 42, 1);
            let final_chain = final_chain_for_voters(&storage, &[voter]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, anchor.hash());
            let plan = runtime
                .pbft_service_pillar_apply_rlp_bundle_with_final_chain(
                    &final_chain,
                    vec![PillarVoteRlpPayload {
                        vote_rlp: vote.encode_rlp(),
                    }],
                    42,
                )
                .unwrap();
            assert!(plan.missing_threshold);
        }
        let _ = fs::remove_dir_all(future_dir);

        let zero_dir = unique_temp_dir("pillar_composed_bundle_zero");
        {
            let storage = create_storage(zero_dir.to_str().unwrap()).unwrap();
            let final_chain = final_chain_for_voters(&storage, &[]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(0);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(
                &SigningKey::from_slice(&[0x74; 32]).unwrap(),
                1,
                anchor.hash(),
            );
            let vote_hash: [u8; 32] = vote.hash(true).into();
            let plan = runtime
                .pbft_service_pillar_apply_rlp_bundle_with_final_chain(
                    &final_chain,
                    vec![PillarVoteRlpPayload {
                        vote_rlp: vote.encode_rlp(),
                    }],
                    1,
                )
                .unwrap();
            assert_eq!(plan.status, 5);
            assert_eq!(plan.first_bad_vote_hash, vote_hash);
        }
        let _ = fs::remove_dir_all(zero_dir);
    }
}
