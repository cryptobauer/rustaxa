//! CXX bridge wrappers for deterministic pillar-chain planning.
//!
//! This bridge composes pillar-chain state with borrowed Rust FinalChain reads
//! and exposes stable CXX plans to the compatibility shim. Production callers
//! no longer supply Pillar-specific DPoS vote-count or eligibility facts;
//! low-level fact planners remain module-internal test seams. C++ remains
//! responsible for temporary `PillarBlock` object construction, event
//! emission, and network side effects. Rust consensus owns the pillar storage
//! writes routed through `rustaxa-storage`, including generation-checked apply
//! of a planned current block.

use crate::ffi::rustaxa_ffi::{
    PillarBlockCreationRequest as FfiPillarBlockCreationRequest,
    PillarBlockCreationWithVoteCountsPlan as FfiPillarBlockCreationWithVoteCountsPlan,
    PillarBlockLinkagePlan as FfiPillarBlockLinkagePlan,
    PillarBlockLinkageRequest as FfiPillarBlockLinkageRequest,
    PillarChainStartupBootstrap as FfiPillarChainStartupBootstrap,
    PillarCurrentAnchorDecisionRequest as FfiPillarCurrentAnchorDecisionRequest,
    PillarCurrentAnchorDecisionResult as FfiPillarCurrentAnchorDecisionResult,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
#[cfg(test)]
use crate::ffi::BridgeStorage;
use crate::ffi::{BridgeApp, BridgeFinalChain};
use anyhow::{bail, Result};
use ethereum_types::H256;
use rustaxa_consensus::{
    PillarBlockCreationRequest as ConsensusPillarBlockCreationRequest,
    PillarBlockCreationWithVoteCountsPlan as ConsensusPillarBlockCreationWithVoteCountsPlan,
    PillarBlockLinkagePlan as ConsensusPillarBlockLinkagePlan,
    PillarBlockLinkageRequest as ConsensusPillarBlockLinkageRequest,
    PillarCurrentAnchorDecisionRequest as ConsensusPillarCurrentAnchorDecisionRequest,
    PillarCurrentAnchorDecisionResult as ConsensusPillarCurrentAnchorDecisionResult,
    PillarValidatorVoteCount as ConsensusPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as ConsensusPillarValidatorVoteCountChange,
};

/// Creates a Rust-owned pillar-chain runtime for the C++ PillarChainManager
/// shim.
///
/// The runtime owns both the pillar-vote aggregation state and the typed
/// storage handle needed by finalization, so live pillar-manager routes do not
/// pass one bridge handle into another to execute internal consensus behavior.
/// Missing and restored-present snapshots both start at generation zero; zero
/// is a valid process-local baseline, and each successful apply increments it.
/// Malformed persisted current data makes construction fail before a runtime is
/// published.
/// Creates a test-only pending PBFT service using the production composition.
///
/// The production constructor restores every PBFT capability, including pillar
/// state. This wrapper supplies deterministic test configuration but leaves the
/// bootstrap gate pending so boundary tests can verify readiness precedence.
#[cfg(test)]
fn create_pending_pillar_test_service_from_storage(
    storage: &BridgeStorage,
) -> Result<Box<BridgeApp>> {
    crate::dag_transaction_service::create_consensus_application_from_storage(
        storage,
        &[1u8; 32],
        32,
        100,
        crate::ffi::rustaxa_ffi::SortitionRuntimeConfig {
            threshold_upper: 0x100,
            difficulty_min: 1,
            difficulty_max: 10,
            difficulty_stale: 5,
            lambda_bound: 100,
            changes_count_for_average: 8,
            dag_efficiency_target_low: 5_000,
            dag_efficiency_target_high: 10_000,
            changing_interval: 10,
            computation_interval: 5,
        },
        crate::ffi::rustaxa_ffi::TransactionQueueConfig { max_size: 16 },
        crate::ffi::rustaxa_ffi::GasPricerConfig {
            percentile: 50,
            minimum_price: [0; 32],
            history_blocks: 0,
            is_light_node: false,
            blocks_gas_pricer: false,
        },
        1_000_000,
        crate::ffi::rustaxa_ffi::PbftServiceConfig {
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 100,
            cacti_lambda_default_ms: 100,
            cacti_block: u64::MAX,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 400,
            polling_interval_ms: 100,
            report_malicious_behaviour: true,
            magnolia_activation_period: 0,
            ficus_activation_period: 0,
            pillar_blocks_interval: 10,
            sync_level_size: 10,
            is_light_node: false,
            light_node_history: 0,
            committee_size: 1,
            number_of_proposers: 1,
            slashing_submitters: Vec::new(),
        },
    )
}

impl BridgeApp {
    pub fn pbft_service_pillar_ready(&self) -> bool {
        self.0.pillar_is_ready()
    }

    pub fn pbft_service_complete_pillar_bootstrap(&self) -> Result<()> {
        self.0.complete_pillar_bootstrap()
    }

    /// Publishes a block-creation payload only against its sampled generation.
    pub fn pbft_service_pillar_apply_planned_current_block_data(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        self.0
            .apply_pillar_current_block_data_for_generation(data_rlp, expected_anchor_generation)
    }

    pub fn pbft_service_pillar_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        self.0.apply_own_pillar_vote(vote_rlp)
    }

    pub fn pbft_service_pillar_load_startup_bootstrap(
        &self,
    ) -> Result<FfiPillarChainStartupBootstrap> {
        self.0.load_pillar_startup_bootstrap().map(Into::into)
    }

    pub fn pbft_service_pillar_plan_current_anchor_decision(
        &self,
        request: FfiPillarCurrentAnchorDecisionRequest,
    ) -> Result<FfiPillarCurrentAnchorDecisionResult> {
        self.0.ensure_pillar_available()?;
        let request = ConsensusPillarCurrentAnchorDecisionRequest::try_from(request)?;
        self.0
            .plan_pillar_current_anchor_decision(request)
            .map(Into::into)
    }

    /// Plans a pillar block using the root-owned Rust pipeline.
    ///
    /// The FinalChain argument remains for compatibility with the C++ calling
    /// contract, while the complete plan + validation now runs in
    /// `rustaxa-consensus` and is forwarded as-is.
    pub fn pbft_service_pillar_plan_block_creation_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        request: FfiPillarBlockCreationRequest,
    ) -> Result<FfiPillarBlockCreationWithVoteCountsPlan> {
        self.0
            .plan_pillar_block_creation_with_final_chain(&final_chain.0, request.into())
            .map(Into::into)
    }

    pub fn pbft_service_pillar_plan_block_linkage(
        &self,
        request: FfiPillarBlockLinkageRequest,
    ) -> Result<FfiPillarBlockLinkagePlan> {
        self.0
            .plan_pillar_block_linkage(request.into())
            .map(Into::into)
    }

    pub fn pbft_service_pillar_latest_finalized_block_rlp(&self) -> Result<Vec<u8>> {
        self.0.latest_finalized_pillar_block_rlp()
    }
}

impl From<ConsensusPillarValidatorVoteCountChange> for FfiPillarValidatorVoteCountChange {
    fn from(value: ConsensusPillarValidatorVoteCountChange) -> Self {
        Self {
            address: value.address.into(),
            vote_count_change: value.vote_count_change,
        }
    }
}

impl From<ConsensusPillarValidatorVoteCount> for FfiPillarValidatorVoteCount {
    fn from(value: ConsensusPillarValidatorVoteCount) -> Self {
        Self {
            address: value.address.into(),
            vote_count: value.vote_count,
        }
    }
}

impl From<FfiPillarBlockCreationRequest> for ConsensusPillarBlockCreationRequest {
    fn from(value: FfiPillarBlockCreationRequest) -> Self {
        Self {
            pillar_block_period: value.pillar_block_period,
            state_root: value.state_root.into(),
            bridge_root: value.bridge_root.into(),
            bridge_epoch: value.bridge_epoch.into(),
            first_pillar_block_period: value.first_pillar_block_period,
            pillar_blocks_interval: value.pillar_blocks_interval,
        }
    }
}

impl From<FfiPillarBlockLinkageRequest> for ConsensusPillarBlockLinkageRequest {
    fn from(value: FfiPillarBlockLinkageRequest) -> Self {
        Self {
            pillar_block_period: value.pillar_block_period,
            pillar_block_previous_hash: value.pillar_block_previous_hash.into(),
            first_pillar_block_period: value.first_pillar_block_period,
            pillar_blocks_interval: value.pillar_blocks_interval,
        }
    }
}

impl TryFrom<FfiPillarCurrentAnchorDecisionRequest>
    for ConsensusPillarCurrentAnchorDecisionRequest
{
    type Error = anyhow::Error;

    fn try_from(value: FfiPillarCurrentAnchorDecisionRequest) -> Result<Self> {
        match value.operation {
            0 => Ok(Self::ValidateCandidate {
                candidate_hash: value
                    .has_candidate_hash
                    .then_some(H256::from(value.candidate_hash)),
            }),
            1 => Ok(Self::SelectPreviousPeriod {
                pbft_period: value.pbft_period,
            }),
            2 => Ok(Self::RestartPostProcessing {
                pbft_period: value.pbft_period,
                pillar_blocks_interval: value.pillar_blocks_interval,
            }),
            operation => bail!("unknown current pillar anchor operation: {operation}"),
        }
    }
}

impl From<rustaxa_consensus::PillarChainStartupBootstrap> for FfiPillarChainStartupBootstrap {
    fn from(value: rustaxa_consensus::PillarChainStartupBootstrap) -> Self {
        Self {
            own_vote_rlp: value.own_vote_rlp,
            current_block_data_rlp: value.current_block_data_rlp,
            latest_pillar_votes_period_data_rlp: value.latest_pillar_votes_period_data_rlp,
        }
    }
}

impl From<ConsensusPillarCurrentAnchorDecisionResult> for FfiPillarCurrentAnchorDecisionResult {
    fn from(value: ConsensusPillarCurrentAnchorDecisionResult) -> Self {
        let (has_current_anchor, current_period, current_hash) = value
            .current_anchor
            .map(|anchor| (true, anchor.period, anchor.hash.into()))
            .unwrap_or((false, 0, [0; 32]));
        Self {
            status: value.plan.status.as_u8(),
            selected: value.plan.selected,
            has_current_anchor,
            current_period,
            current_hash,
            anchor_generation: value.anchor_generation,
        }
    }
}

impl From<ConsensusPillarBlockLinkagePlan> for FfiPillarBlockLinkagePlan {
    fn from(value: ConsensusPillarBlockLinkagePlan) -> Self {
        Self {
            status: value.status.as_u8(),
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
        }
    }
}

impl From<ConsensusPillarBlockCreationWithVoteCountsPlan>
    for FfiPillarBlockCreationWithVoteCountsPlan
{
    fn from(value: ConsensusPillarBlockCreationWithVoteCountsPlan) -> Self {
        Self {
            status: value.creation.status.as_u8(),
            valid: value.creation.valid,
            expected_previous_period: value.creation.expected_previous_period,
            previous_pillar_block_hash: value.creation.previous_pillar_block_hash.0,
            state_root: value.creation.state_root.0,
            bridge_root: value.creation.bridge_root.0,
            bridge_epoch: value.creation.bridge_epoch.0,
            vote_count_changes: value
                .vote_count_changes
                .into_iter()
                .map(Into::into)
                .collect(),
            current_vote_counts: value
                .current_vote_counts
                .into_iter()
                .map(Into::into)
                .collect(),
            anchor_generation: value.anchor_generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi;
    use crate::final_chain::create_final_chain;
    use crate::storage::create_storage;
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

    fn final_chain_with_validator(
        storage: &BridgeStorage,
        address: [u8; 20],
    ) -> Box<BridgeFinalChain> {
        create_final_chain(
            storage,
            0,
            0,
            Vec::new(),
            vec![rustaxa_ffi::GenesisValidator {
                address,
                owner: address,
                vrf_key: [7; 32],
                commission: 0,
                description: String::new(),
                endpoint: String::new(),
                total_stake: u256_be(5_000),
                delegations: vec![rustaxa_ffi::GenesisDelegation {
                    delegator: address,
                    stake: u256_be(5_000),
                }],
            }],
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
        .expect("FinalChain should initialize")
    }

    fn anchor_request(operation: u8) -> FfiPillarCurrentAnchorDecisionRequest {
        FfiPillarCurrentAnchorDecisionRequest {
            operation,
            has_candidate_hash: false,
            candidate_hash: [0; 32],
            pbft_period: 0,
            pillar_blocks_interval: 0,
        }
    }

    #[test]
    fn current_anchor_adapter_preserves_readiness_tag_and_status_mapping() {
        let temp_dir = unique_temp_dir("pillar_bridge_anchor_adapter");
        {
            let storage = create_storage(temp_dir.to_str().expect("UTF-8 temp path"))
                .expect("storage should initialize");
            let service = create_pending_pillar_test_service_from_storage(&storage)
                .expect("pending PBFT service should restore");
            let unavailable = match service
                .pbft_service_pillar_plan_current_anchor_decision(anchor_request(99))
            {
                Ok(_) => panic!("readiness must precede FFI tag decoding"),
                Err(error) => error,
            };
            assert_eq!(unavailable.to_string(), "PBFT_SERVICE_PILLAR_UNAVAILABLE");

            service
                .pbft_service_complete_pillar_bootstrap()
                .expect("pillar service should become ready");
            let missing = service
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 0,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 0,
                        pillar_blocks_interval: 0,
                    },
                )
                .expect("missing-anchor result should project");
            assert_eq!(missing.status, 1);
            assert!(!missing.selected);
            assert!(!missing.has_current_anchor);
            assert_eq!(missing.current_period, 0);
            assert_eq!(missing.current_hash, [0; 32]);
            let unknown = match service
                .pbft_service_pillar_plan_current_anchor_decision(anchor_request(99))
            {
                Ok(_) => panic!("ready service must reject unknown FFI tag"),
                Err(error) => error,
            };
            assert!(unknown
                .to_string()
                .contains("unknown current pillar anchor operation"));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn typed_storage_adapter_preserves_bytes_missing_reads_and_errors() {
        let temp_dir = unique_temp_dir("pillar_bridge_storage_adapter");
        {
            let storage = create_storage(temp_dir.to_str().expect("UTF-8 temp path"))
                .expect("storage should initialize");
            let storage_for_ops = BridgeStorage(storage.0.clone());
            drop(storage);
            assert!(storage_for_ops
                .pillar_chain_storage_load_current_block_data()
                .expect("missing current read")
                .is_empty());
            assert!(storage_for_ops
                .pillar_chain_storage_load_own_vote()
                .expect("missing own-vote read")
                .is_empty());
            assert!(storage_for_ops
                .pillar_chain_storage_load_latest_block()
                .expect("missing latest read")
                .is_empty());
            assert!(storage_for_ops
                .pillar_chain_storage_load_block(42)
                .expect("missing period read")
                .is_empty());

            storage_for_ops
                .pillar_chain_storage_apply_current_block_data(vec![0xc1, 0x01])
                .expect("current bytes should persist");
            storage_for_ops
                .pillar_chain_storage_apply_own_vote(vec![0xc1, 0x02])
                .expect("own-vote bytes should persist");
            storage_for_ops
                .pillar_chain_storage_apply_finalized_block(42, vec![0xc1, 0x03])
                .expect("finalized bytes should persist");
            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current bytes should load"),
                vec![0xc1, 0x01]
            );
            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_load_own_vote()
                    .expect("own-vote bytes should load"),
                vec![0xc1, 0x02]
            );
            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_load_latest_block()
                    .expect("latest bytes should load"),
                vec![0xc1, 0x03]
            );
            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_load_block(42)
                    .expect("period bytes should load"),
                vec![0xc1, 0x03]
            );

            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_apply_current_block_data(Vec::new())
                    .expect_err("empty current payload must reject")
                    .to_string(),
                "PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"
            );
            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_apply_own_vote(Vec::new())
                    .expect_err("empty own-vote payload must reject")
                    .to_string(),
                "PILLAR_OWN_VOTE_EMPTY_PAYLOAD"
            );
            assert_eq!(
                storage_for_ops
                    .pillar_chain_storage_apply_finalized_block(43, Vec::new())
                    .expect_err("empty finalized payload must reject")
                    .to_string(),
                "PILLAR_FINALIZED_BLOCK_EMPTY_PAYLOAD"
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn final_chain_handle_adapter_projects_native_block_creation() {
        let temp_dir = unique_temp_dir("pillar_bridge_final_chain_adapter");
        {
            let storage = create_storage(temp_dir.to_str().expect("UTF-8 temp path"))
                .expect("storage should initialize");
            let final_chain = final_chain_with_validator(&storage, [9; 20]);
            let service = create_pending_pillar_test_service_from_storage(&storage)
                .expect("PBFT service should restore");
            service
                .pbft_service_complete_pillar_bootstrap()
                .expect("pillar service should become ready");

            let plan = service
                .pbft_service_pillar_plan_block_creation_with_final_chain(
                    &final_chain,
                    FfiPillarBlockCreationRequest {
                        pillar_block_period: 0,
                        state_root: [1; 32],
                        bridge_root: [2; 32],
                        bridge_epoch: [0; 32],
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 10,
                    },
                )
                .expect("native plan should cross the retained handle adapter");
            assert_eq!(plan.status, 1);
            assert!(plan.valid);
            assert_eq!(plan.expected_previous_period, 0);
            assert_eq!(plan.previous_pillar_block_hash, [0; 32]);
            assert_eq!(plan.state_root, [1; 32]);
            assert_eq!(plan.bridge_root, [2; 32]);
            assert_eq!(plan.bridge_epoch, [0; 32]);
            assert_eq!(plan.current_vote_counts.len(), 1);
            assert_eq!(plan.current_vote_counts[0].address, [9; 20]);
            assert_eq!(plan.current_vote_counts[0].vote_count, 5);
            assert_eq!(plan.vote_count_changes.len(), 1);
            assert_eq!(plan.vote_count_changes[0].address, [9; 20]);
            assert_eq!(plan.vote_count_changes[0].vote_count_change, 5);
            assert_eq!(plan.anchor_generation, 0);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }
}
