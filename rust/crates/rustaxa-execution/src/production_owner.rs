//! The sole existing-chain factory and its closed operation clients.
#[cfg(feature = "production_integration_test")]
pub mod integration_tests;
#[cfg(feature = "production_integration_test")]
pub mod publication_tests;
use anyhow::{Result, bail, ensure};
use ethereum_types::H256;
use rustaxa_consensus::consensus_application_runtime::{OwnedDagGasReport, OwnedDagGasRequest};
use rustaxa_consensus::verified_votes::PbftVoteType;
use rustaxa_consensus::*;
use rustaxa_consensus::{
    BoundConsensusApplication, ConsensusExecutionPort, ConsensusObserverPort, ConsensusProcessPort,
    ConsensusRunExit, ConsensusSigningPort, ConsensusTransportPort, PillarAnchorStateReport,
    PillarAnchorStateRequest, PublicTransactionSubmissionReport,
    PublicTransactionSubmissionRequest,
};
use rustaxa_storage::{
    ConcreteStateLifecycle, Config, Storage, application_control::ApplicationConcreteState,
};
use rustaxa_types::DposValidatorStake;
use std::path::PathBuf;
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

pub struct ProductionBootstrapConfig {
    pub selected: Arc<rustaxa_native_config::SelectedProductionConfig>,
    pub concrete_path: PathBuf,
}

#[derive(Clone)]
pub struct ProductionApplication {
    inner: Arc<ProductionOwnerInner>,
}

struct ProductionOwnerInner {
    application: Arc<BoundConsensusApplication>,
    publication: ProductionOwnerPublicationAuthority,
}

enum ProductionExecutionState<'a> {
    Scheduler(&'a ProductionApplication),
    Network(&'a ProductionApplication),
}

impl ConsensusExecutionPort for ProductionExecutionState<'_> {
    fn estimate_owned_dag_transaction_gas(
        &self,
        request: OwnedDagGasRequest<'_>,
    ) -> Result<Option<OwnedDagGasReport>> {
        let (Self::Scheduler(owner) | Self::Network(owner)) = self;
        owner.inner.application.with_owned_dag_gas_state(
            &owner.inner.publication,
            request,
            crate::production_dag_gas::estimate,
        )
    }

    fn check_proposal_transactions(
        &self,
        block_rlp: &[u8],
        owner_generation: u64,
    ) -> Result<Option<rustaxa_consensus::ProposalExecutionCheck>> {
        let Self::Scheduler(owner) = self else {
            bail!("PRODUCTION_PROPOSAL_EXECUTION_UNAVAILABLE");
        };
        owner
            .inner
            .application
            .with_proposal_transition_preparation(
                &owner.inner.publication,
                block_rlp,
                owner_generation,
                crate::production_transition::check_candidate_transactions,
            )
    }

    fn publish_scheduled_certificate(
        &self,
        selection: rustaxa_consensus::consensus_application_runtime::ScheduledCertificate<'_>,
    ) -> Result<bool> {
        let Self::Scheduler(owner) = self else {
            bail!("PRODUCTION_EXECUTION_PLAN_UNAVAILABLE: certified preparation before writes");
        };
        let prepared = owner
            .inner
            .application
            .with_scheduled_certificate_preparation(&selection, |context, concrete, native| {
                crate::production_transition::execute(context, concrete, native)
                    .map_err(|error| {
                        if error.is::<crate::production_transition::ProductionTransitionHardError>() {
                            rustaxa_consensus::consensus_application_runtime::ScheduledCertificateRejected(error.to_string()).into()
                        } else {
                            error
                        }
                    })?
                    .into_verified()
            })?;
        let Some(prepared) = prepared else {
            return Ok(false);
        };
        match owner.publish_verified_transition(prepared) {
            Ok(_) => Ok(true),
            Err(error) if error.is::<ScheduledCertificateStale>() => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn publish_synced_certificate(
        &self,
        selection: &rustaxa_consensus::consensus_application_runtime::ScheduledSyncedCertificate,
    ) -> Result<bool> {
        let Self::Scheduler(owner) = self else {
            bail!("PRODUCTION_EXECUTION_PLAN_UNAVAILABLE: synced certificate");
        };
        let prepared = owner
            .inner
            .application
            .with_synced_certificate_preparation(selection, |context, concrete, native| {
                crate::production_transition::execute(context, concrete, native)
                    .map_err(|error| {
                        if error.is::<crate::production_transition::ProductionTransitionHardError>() {
                            rustaxa_consensus::consensus_application_runtime::SyncedCertificateRejected(error.to_string()).into()
                        } else {
                            error
                        }
                    })?
                    .into_verified()
            })?;
        let Some(prepared) = prepared else {
            return Ok(false);
        };
        match owner.publish_verified_transition(prepared) {
            Ok(_) => Ok(true),
            Err(error) if error.is::<ScheduledCertificateStale>() => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn load_pillar_anchor_state(
        &self,
        request: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        let Self::Scheduler(owner) = self else {
            bail!("PRODUCTION_PILLAR_SCHEDULER_REQUIRED");
        };
        let period = request.period.into();
        let (bridge_root, bridge_epoch) =
            owner
                .inner
                .application
                .with_finalized_execution_state(period, |state| {
                    ensure!(
                        state.period() == period,
                        "PRODUCTION_PILLAR_ANCHOR_ABOVE_HEAD"
                    );
                    crate::production_dag_gas::pillar_bridge_values(state)
                })?;
        Ok(PillarAnchorStateReport {
            effect_id: request.effect_id,
            succeeded: true,
            block_header_rlp: Vec::new(),
            state_root: [0; 32],
            bridge_root,
            bridge_epoch,
            validator_vote_counts: Vec::new(),
            signer_vote_counts: Vec::new(),
            total_eligible_vote_count: 0,
            error_code: String::new(),
        })
    }

    fn final_chain_native_projection_context(
        &self,
        _request_id: [u8; 32],
        _projection_hash: [u8; 32],
    ) -> Result<
        Option<rustaxa_consensus::native_projection_context::FinalChainNativeProjectionContext>,
    > {
        bail!("PRODUCTION_NATIVE_PROJECTION_UNAVAILABLE")
    }

    fn final_chain_native_lifecycle_authority(
        &self,
        _request_id: [u8; 32],
        _projection_hash: [u8; 32],
    ) -> Result<Option<rustaxa_consensus::native_projection_context::FinalChainLifecycleAuthority>>
    {
        bail!("PRODUCTION_NATIVE_LIFECYCLE_AUTHORITY_UNAVAILABLE")
    }
}

impl ProductionApplication {
    /// Publishes one checked transition and retires this owner until reopen.
    pub fn publish_verified_transition(
        &self,
        prepared: crate::PreparedVerifiedProductionTransition,
    ) -> Result<rustaxa_types::concrete_state::ConcreteStateIdentity> {
        prepared.publish(&self.inner.application, &self.inner.publication, 0)
    }

    #[cfg(feature = "production_integration_test")]
    pub fn integration_publish_at(
        &self,
        prepared: crate::PreparedVerifiedProductionTransition,
        stop: u8,
    ) -> Result<rustaxa_types::concrete_state::ConcreteStateIdentity> {
        prepared.publish(&self.inner.application, &self.inner.publication, stop)
    }
    /// Executes an exact candidate's ordered transactions under one current
    /// owner read scope. The result has no effect or publication authority.
    /// Only a reached malformed proof produces a deterministic invalid result.
    pub fn check_candidate_transactions(
        &self,
        block_rlp: &[u8],
    ) -> Result<rustaxa_consensus::pbft_execution_validation::PbftExecutionValidation> {
        self.inner.application.with_transition_preparation(
            block_rlp,
            crate::production_transition::check_candidate_transactions,
        )
    }

    /// Computes a private transition. This entry cannot publish or authorize a vote.
    pub fn prepare_transition(
        &self,
        block_rlp: &[u8],
    ) -> Result<crate::PreparedProductionTransition> {
        self.inner.application.with_transition_preparation(
            block_rlp,
            |context, concrete, native| {
                crate::production_transition::execute(context, concrete, native)
            },
        )
    }
    /// Computes with an admitted current certificate. The result cannot publish.
    pub fn prepare_certified_transition(
        &self,
        block_rlp: &[u8],
        certified_round: u64,
    ) -> Result<crate::PreparedProductionTransition> {
        self.inner
            .application
            .with_certified_transition_preparation(
                block_rlp,
                certified_round,
                |context, concrete, native| {
                    crate::production_transition::execute(context, concrete, native)
                },
            )
    }

    /// Computes complete checked evidence. The result cannot publish.
    pub fn prepare_verified_certified_transition(
        &self,
        block_rlp: &[u8],
        certified_round: u64,
    ) -> Result<crate::PreparedVerifiedProductionTransition> {
        self.inner
            .application
            .with_verified_certified_transition_preparation(
                block_rlp,
                certified_round,
                |context, concrete, native| {
                    crate::production_transition::execute(context, concrete, native)?
                        .into_verified()
                },
            )
    }

    pub fn open_existing(config: ProductionBootstrapConfig) -> Result<Self> {
        let bootstrap =
            rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(
                &config.selected,
            )?;
        ensure!(
            bootstrap.schema_major != 0,
            "CONSENSUS_APPLICATION_SCHEMA_MAJOR_ZERO"
        );
        let primary_path = bootstrap.storage_path.join("db");
        ensure!(primary_path.is_dir(), "PRODUCTION_PRIMARY_PATH_MISSING");
        ensure!(
            config.concrete_path.is_dir(),
            "PRODUCTION_CONCRETE_PATH_MISSING"
        );
        ensure!(
            primary_path.canonicalize()? != config.concrete_path.canonicalize()?,
            "PRODUCTION_DATABASE_PATHS_OVERLAP"
        );
        let mut hash = Keccak::v256();
        hash.update(b"rustaxa-final-chain-concrete-chain-identity-v1");
        hash.update(bootstrap.storage_genesis_hash.as_bytes());
        let mut chain = [0; 32];
        hash.finalize(&mut chain);
        let inspected = ConcreteStateLifecycle::inspect_existing(&config.concrete_path, chain)?;
        let storage = Arc::new(Storage::open_existing(Config::new(
            bootstrap.storage_path.clone(),
        ))?);
        let owner = Arc::new(storage.claim_application_owner()?);
        let mut lifecycle =
            ConcreteStateLifecycle::open(&config.concrete_path, chain, inspected.committed)?;
        let owned = lifecycle.observation()?;
        ensure!(
            owned == inspected,
            "PRODUCTION_CONCRETE_CHANGED_DURING_OPEN"
        );
        BoundConsensusApplication::recover_existing_publication(
            &config.selected,
            &storage,
            &owner,
            &mut lifecycle,
        )?;
        let concrete = Arc::new(ApplicationConcreteState::new(
            Arc::clone(&owner),
            lifecycle,
        )?);
        let schedule = BoundConsensusApplication::prepare_existing_schedule(
            &config.selected,
            &storage,
            &owner,
            &concrete,
        )?;
        let (application, publication) =
            BoundConsensusApplication::restore_existing_for_execution_owner(
                &config.selected,
                storage,
                owner,
                schedule,
                concrete,
            )?;
        let application = Arc::new(application);
        Ok(Self {
            inner: Arc::new(ProductionOwnerInner {
                application,
                publication,
            }),
        })
    }

    pub fn network_client(&self) -> ProductionNetworkClient {
        ProductionNetworkClient {
            owner: self.clone(),
        }
    }
    pub fn query_client(&self) -> ProductionQueryClient {
        ProductionQueryClient {
            owner: self.clone(),
        }
    }
    pub fn admin_client(&self) -> ProductionAdminClient {
        ProductionAdminClient {
            owner: self.clone(),
        }
    }

    pub fn submit_public_transaction_from_native_state(
        &self,
        request: PublicTransactionSubmissionRequest,
    ) -> Result<PublicTransactionSubmissionReport> {
        self.inner
            .application
            .submit_public_transaction_from_native_state(request)
    }

    pub fn run<P, S, T, O>(
        &self,
        process: &P,
        signer: &S,
        transport: &T,
        observer: &O,
    ) -> Result<ConsensusRunExit>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        O: ConsensusObserverPort,
    {
        self.inner.application.run(
            process,
            signer,
            transport,
            &ProductionExecutionState::Scheduler(self),
            observer,
        )
    }
}

pub struct ProductionNetworkClient {
    owner: ProductionApplication,
}
pub struct ProductionQueryClient {
    owner: ProductionApplication,
}
pub struct ProductionAdminClient {
    owner: ProductionApplication,
}

impl ProductionAdminClient {
    pub fn prune_light_history(
        &self,
        request: rustaxa_storage::LightHistoryPruneRequest,
    ) -> Result<rustaxa_storage::LightHistoryPruneReport> {
        self.owner.inner.application.prune_light_history(request)
    }
}

impl ProductionQueryClient {
    pub fn consensus_vote_status(&self) -> Result<rustaxa_consensus::ConsensusVoteStatus> {
        self.owner.inner.application.consensus_vote_status()
    }

    pub fn consensus_live_status(&self) -> Result<rustaxa_consensus::ConsensusLiveStatus> {
        self.owner.inner.application.consensus_live_status()
    }
}

#[cfg(test)]
mod tests;

impl ProductionNetworkClient {
    pub fn drain_work(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        source_scoped: bool,
        budget: u32,
    ) -> Result<NetworkEffectBatch> {
        self.owner.inner.application.network_drain_work(
            transport_lane,
            source_payload_id,
            source_scoped,
            budget,
        )
    }
    pub fn report_effect_results(
        &self,
        results: Vec<NetworkEffectResult>,
    ) -> Result<NetworkEffectAck> {
        self.owner
            .inner
            .application
            .network_report_effect_results(results)
    }
    pub fn begin_pbft_sync(
        &self,
        request: NetworkPbftSyncStartRequest,
    ) -> Result<NetworkPbftSyncStartOutcome> {
        self.owner
            .inner
            .application
            .network_begin_pbft_sync(request)
    }
    pub fn ingest_status_packet(
        &self,
        request: NetworkStatusPacketRequest,
    ) -> Result<NetworkStatusPacketReport> {
        self.owner
            .inner
            .application
            .network_ingest_status_packet(request)
    }
    pub fn build_status_packet(
        &self,
        request: NetworkStatusPacketBuildRequest,
    ) -> Result<NetworkStatusPacketBuildOutcome> {
        self.owner
            .inner
            .application
            .network_build_status_packet(request)
    }
    pub fn apply_pbft_sync_command(
        &self,
        request: NetworkPbftSyncCommandRequest,
    ) -> Result<NetworkPbftSyncCommandOutcome> {
        self.owner
            .inner
            .application
            .network_apply_pbft_sync_command(request)
    }
    pub fn request_pending_dag_blocks(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        facts: NetworkPendingDagBlocksRequestFacts,
    ) -> Result<NetworkIngressDecision> {
        self.owner
            .inner
            .application
            .network_request_pending_dag_blocks(transport_lane, source_payload_id, facts)
    }
    pub fn ingest_pbft_vote_packet(
        &self,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        self.owner
            .inner
            .application
            .network_ingest_pbft_vote_packet(request, slashing_submitters)
    }
    pub fn ingest_pbft_votes_bundle_packet(
        &self,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        self.owner
            .inner
            .application
            .network_ingest_pbft_votes_bundle_packet(request, slashing_submitters)
    }
    pub fn ingest_pillar_vote_packet(
        &self,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        self.owner
            .inner
            .application
            .network_ingest_pillar_vote_packet(request)
    }
    pub fn ingest_pillar_votes_bundle_packet(
        &self,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        self.owner
            .inner
            .application
            .network_ingest_pillar_votes_bundle_packet(request)
    }
    pub fn ingest_pbft_next_votes_bundle_request(
        &self,
        request: NetworkPbftNextVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        self.owner
            .inner
            .application
            .network_ingest_pbft_next_votes_bundle_request(request)
    }
    pub fn ingest_pillar_votes_bundle_request(
        &self,
        request: NetworkGetPillarVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        self.owner
            .inner
            .application
            .network_ingest_pillar_votes_bundle_request(request)
    }
    pub fn ingest_get_pbft_sync_request(
        &self,
        request: NetworkGetPbftSyncRequest,
    ) -> Result<NetworkIngressDecision> {
        self.owner
            .inner
            .application
            .network_ingest_get_pbft_sync_request(request)
    }
    pub fn ingest_pbft_blocks_bundle(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
    ) -> Result<NetworkIngressDecision> {
        self.owner
            .inner
            .application
            .network_ingest_pbft_blocks_bundle(packet_rlp, source_payload_id)
    }
    pub fn ingest_get_dag_sync_request(
        &self,
        context: NetworkGetDagSyncContext,
        request_rlp: &[u8],
    ) -> Result<NetworkIngressDecision> {
        self.owner
            .inner
            .application
            .network_ingest_get_dag_sync_request(context, request_rlp)
    }
    pub fn ingest_transaction_packet(
        &self,
        context: NetworkTransactionPacketContext,
        packet_rlp: &[u8],
        policy: PublicTransactionSubmissionRequest,
    ) -> Result<NetworkTransactionPacketReport> {
        self.owner
            .inner
            .application
            .network_ingest_transaction_packet(context, packet_rlp, policy)
    }
    pub fn ingest_dag_block_packet(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
    ) -> Result<NetworkDagBlockIngressReport> {
        self.owner
            .inner
            .application
            .network_ingest_dag_block_packet(
                context,
                packet_rlp,
                &ProductionExecutionState::Network(&self.owner),
            )
    }
    pub fn ingest_dag_sync_packet(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        policy: PublicTransactionSubmissionRequest,
    ) -> Result<NetworkDagSyncIngressReport> {
        self.owner.inner.application.network_ingest_dag_sync_packet(
            context,
            packet_rlp,
            policy,
            &ProductionExecutionState::Network(&self.owner),
        )
    }
    pub fn transaction_gas_price_bid(&self) -> Result<[u8; 32]> {
        self.owner
            .inner
            .application
            .network_transaction_gas_price_bid()
    }
    pub fn submit_transaction_from_native_state(
        &self,
        request: PublicTransactionSubmissionRequest,
    ) -> Result<PublicTransactionSubmissionReport> {
        self.owner
            .inner
            .application
            .network_submit_transaction_from_native_state(request)
    }
    pub fn prepare_egress(
        &self,
        request: NetworkEgressPrepareRequest,
    ) -> Result<NetworkEgressPreparation> {
        self.owner.inner.application.network_prepare_egress(request)
    }
    pub fn plan_egress(&self, request: NetworkEgressPlanRequest) -> Result<NetworkIngressDecision> {
        self.owner.inner.application.network_plan_egress(request)
    }
    pub fn cancel_egress(&self, token: u64) -> Result<bool> {
        self.owner.inner.application.network_cancel_egress(token)
    }
    pub fn begin_pbft_sync_ingress(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
        source_peer_id: [u8; 64],
        sync_generation: u64,
        slashing_submitters: Vec<SlashingSubmitterIdentity>,
    ) -> Result<PbftSyncIngressStep> {
        self.owner
            .inner
            .application
            .network_begin_pbft_sync_ingress(
                packet_rlp,
                source_payload_id,
                source_peer_id,
                sync_generation,
                slashing_submitters,
            )
    }
    pub fn report_pbft_sync_ingress_slashing(
        &self,
        proof_hash: H256,
        transaction_inserted: bool,
        source: rustaxa_consensus::pbft_service::PbftSyncIngressSource,
    ) -> Result<PbftSyncIngressStep> {
        self.owner
            .inner
            .application
            .network_report_pbft_sync_ingress_slashing(proof_hash, transaction_inserted, source)
    }
    pub fn report_verified_vote_slashing_transaction_submission(
        &self,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<bool> {
        self.owner
            .inner
            .application
            .network_report_verified_vote_slashing_transaction_submission(
                proof_hash,
                transaction_inserted,
            )
    }
}

impl ProductionQueryClient {
    /// Returns the authenticated account at the requested finalized period.
    pub fn finalized_account(
        &self,
        period: Option<u64>,
        address: [u8; 20],
    ) -> Result<Option<rustaxa_types::concrete_state::ConcreteAccount>> {
        use rustaxa_types::concrete_state::ConcreteRead;
        Ok(self
            .owner
            .inner
            .application
            .with_finalized_state(period, |state, identity| {
                Ok(match state.account_at(identity, address)? {
                    ConcreteRead::Present(record) => Some(record.account),
                    ConcreteRead::Absent | ConcreteRead::Tombstone => None,
                })
            })?
            .flatten())
    }

    /// Returns the authenticated storage word. A missing path is zero.
    pub fn finalized_storage(
        &self,
        period: Option<u64>,
        address: [u8; 20],
        key: [u8; 32],
    ) -> Result<[u8; 32]> {
        use rustaxa_storage::ConcreteStoragePath;
        use rustaxa_types::concrete_state::ConcreteStorageKey;
        Ok(self
            .owner
            .inner
            .application
            .with_finalized_state(period, |state, identity| {
                let mut word = [0; 32];
                if let ConcreteStoragePath::Member(bytes) =
                    state.verify_storage_path_at(identity, address, ConcreteStorageKey(key))?
                {
                    ensure!(
                        bytes.len() <= word.len(),
                        "PRODUCTION_QUERY_STORAGE_EXCEEDS_WORD"
                    );
                    word[32 - bytes.len()..].copy_from_slice(&bytes);
                }
                Ok(word)
            })?
            .unwrap_or_default())
    }

    /// Returns the code reached through the account at the same retained root.
    pub fn finalized_code(&self, period: Option<u64>, address: [u8; 20]) -> Result<Vec<u8>> {
        use rustaxa_types::concrete_state::ConcreteRead;
        Ok(self
            .owner
            .inner
            .application
            .with_finalized_state(period, |state, identity| {
                let ConcreteRead::Present(record) = state.account_at(identity, address)? else {
                    return Ok(Vec::new());
                };
                if record.account.code_size == 0 {
                    return Ok(Vec::new());
                }
                let hash = record
                    .account
                    .code_hash
                    .ok_or_else(|| anyhow::anyhow!("PRODUCTION_QUERY_CODE_HASH_MISSING"))?;
                let ConcreteRead::Present(bytes) = state.code_at(identity, hash)? else {
                    bail!("PRODUCTION_QUERY_CODE_MISSING");
                };
                ensure!(
                    u64::try_from(bytes.len())? == record.account.code_size,
                    "PRODUCTION_QUERY_CODE_SIZE_MISMATCH"
                );
                Ok(bytes)
            })?
            .unwrap_or_default())
    }

    pub fn dag_live_status(&self) -> Result<DagRuntimeStatus> {
        self.owner.inner.application.query_dag_live_status()
    }
    pub fn dag_live_non_finalized_index(&self) -> Result<DagNonFinalizedIndex> {
        self.owner
            .inner
            .application
            .query_dag_live_non_finalized_index()
    }
    pub fn transaction_pool_status(&self) -> Result<TransactionPoolStatus> {
        self.owner.inner.application.query_transaction_pool_status()
    }
    pub fn pbft_sync_status(&self, now_ms: u64) -> Result<NetworkPbftSyncSnapshot> {
        self.owner.inner.application.query_pbft_sync_status(now_ms)
    }
    pub fn live_transaction_is_known(&self, hash: [u8; 32]) -> Result<bool> {
        self.owner
            .inner
            .application
            .query_live_transaction_is_known(hash)
    }
    pub fn verified_vote_count(&self) -> Result<u64> {
        self.owner.inner.application.query_verified_vote_count()
    }
    pub fn pbft_vote_threshold(
        &self,
        period: u64,
        vote_type: PbftVoteType,
    ) -> Result<rustaxa_consensus::pbft_thresholds::PbftTwoTPlusOneThresholdPlan> {
        self.owner
            .inner
            .application
            .query_pbft_vote_threshold(period, vote_type)
    }
    pub fn pbft_progress(&self) -> Result<PbftProgressView> {
        self.owner.inner.application.query_pbft_progress()
    }
    pub fn pbft_sync_block_exists(&self, block_hash: [u8; 32]) -> Result<bool> {
        self.owner
            .inner
            .application
            .query_pbft_sync_block_exists(block_hash)
    }
    pub fn pbft_block_hash_by_period(&self, period: u64) -> Result<QueryHashLookup> {
        self.owner
            .inner
            .application
            .query_pbft_block_hash_by_period(period)
    }
    pub fn pbft_schedule_block_by_period(&self, period: u64) -> Result<PbftScheduleBlockView> {
        self.owner
            .inner
            .application
            .query_pbft_schedule_block_by_period(period)
    }
    pub fn pbft_node_version_by_period(&self, period: u64) -> Result<PbftNodeVersionView> {
        self.owner
            .inner
            .application
            .query_pbft_node_version_by_period(period)
    }
    pub fn pbft_previous_block_cert_votes_by_period(
        &self,
        period: u64,
    ) -> Result<PbftPeriodCertVotesView> {
        self.owner
            .inner
            .application
            .query_pbft_previous_block_cert_votes_by_period(period)
    }
    pub fn final_chain_block_by_number(&self, number: u64) -> Result<FinalChainBlockView> {
        self.owner
            .inner
            .application
            .query_final_chain_block_by_number(number)
    }
    pub fn final_chain_block_number_by_hash(
        &self,
        block_hash: [u8; 32],
    ) -> Result<QueryNumberLookup> {
        self.owner
            .inner
            .application
            .query_final_chain_block_number_by_hash(block_hash)
    }
    pub fn final_chain_last_block_number(&self) -> Result<u64> {
        self.owner
            .inner
            .application
            .query_final_chain_last_block_number()
    }
    pub fn final_chain_dpos_eligible_vote_count(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<u64> {
        self.owner
            .inner
            .application
            .query_final_chain_dpos_eligible_vote_count(block_number, address)
    }
    pub fn final_chain_dpos_eligible_total_vote_count(&self, block_number: u64) -> Result<u64> {
        self.owner
            .inner
            .application
            .query_final_chain_dpos_eligible_total_vote_count(block_number)
    }
    pub fn final_chain_dpos_validators_total_stakes(
        &self,
        block_number: u64,
    ) -> Result<Vec<DposValidatorStake>> {
        self.owner
            .inner
            .application
            .query_final_chain_dpos_validators_total_stakes(block_number)
    }
    pub fn final_chain_dpos_total_amount_delegated(&self, block_number: u64) -> Result<Vec<u8>> {
        self.owner
            .inner
            .application
            .query_final_chain_dpos_total_amount_delegated(block_number)
    }
    pub fn final_chain_dpos_yield(&self, block_number: u64) -> Result<u64> {
        self.owner
            .inner
            .application
            .query_final_chain_dpos_yield(block_number)
    }
    pub fn final_chain_dpos_total_supply(&self, block_number: u64) -> Result<Vec<u8>> {
        self.owner
            .inner
            .application
            .query_final_chain_dpos_total_supply(block_number)
    }
    pub fn final_chain_native_call(
        &self,
        request: rustaxa_consensus::FinalChainCallRequest,
    ) -> Result<rustaxa_types::FinalChainCallOutcome> {
        self.owner
            .inner
            .application
            .with_finalized_execution_state(request.block_number, |state| {
                crate::production_dag_gas::call(state, request)
            })
    }
    pub fn final_chain_structured_trace(&self, period: u64, request_rlp: &[u8]) -> Result<Vec<u8>> {
        self.owner
            .inner
            .application
            .with_finalized_execution_state(period.into(), |state| {
                crate::production_dag_gas::trace(state, request_rlp)
            })
    }

    pub fn period_lambda_by_period(&self, period: u64) -> Result<QueryPeriodLambda> {
        self.owner
            .inner
            .application
            .query_period_lambda_by_period(period)
    }
    pub fn proposal_period_for_dag_level(&self, level: u64) -> Result<QueryNumberLookup> {
        self.owner
            .inner
            .application
            .query_proposal_period_for_dag_level(level)
    }
    pub fn chain_stats(&self) -> Result<ChainStatsView> {
        self.owner.inner.application.query_chain_stats()
    }
    pub fn consensus_status(&self) -> Result<ConsensusStatusView> {
        self.owner.inner.application.query_consensus_status()
    }
    pub fn sortition_params_change_by_period(
        &self,
        period: u64,
    ) -> Result<SortitionParamsChangeView> {
        self.owner
            .inner
            .application
            .query_sortition_params_change_by_period(period)
    }
    pub fn final_chain_blocks_with_bloom(
        &self,
        bloom: [u8; 256],
        from: u64,
        to: u64,
    ) -> Result<Vec<u64>> {
        self.owner
            .inner
            .application
            .query_final_chain_blocks_with_bloom(bloom, from, to)
    }
    pub fn pillar_block_data_by_period(&self, period: u64) -> Result<PillarBlockDataView> {
        self.owner
            .inner
            .application
            .query_pillar_block_data_by_period(period)
    }
    pub fn dag_block_by_hash(&self, hash: [u8; 32]) -> Result<DagBlockView> {
        self.owner.inner.application.query_dag_block_by_hash(hash)
    }
    pub fn dag_blocks_by_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<DagBlockView>> {
        self.owner
            .inner
            .application
            .query_dag_blocks_by_level(level, number_of_levels)
    }
    pub fn finalized_dag_blocks_by_period(&self, period: u64) -> Result<Vec<DagBlockView>> {
        self.owner
            .inner
            .application
            .query_finalized_dag_blocks_by_period(period)
    }
    pub fn transaction_by_hash(&self, hash: [u8; 32]) -> Result<TransactionView> {
        self.owner.inner.application.query_transaction_by_hash(hash)
    }
    pub fn transaction_by_block_number_and_index(
        &self,
        block_number: u64,
        transaction_index: u64,
    ) -> Result<TransactionView> {
        self.owner
            .inner
            .application
            .query_transaction_by_block_number_and_index(block_number, transaction_index)
    }
    pub fn transaction_by_block_hash_and_index(
        &self,
        block_hash: [u8; 32],
        transaction_index: u64,
    ) -> Result<TransactionView> {
        self.owner
            .inner
            .application
            .query_transaction_by_block_hash_and_index(block_hash, transaction_index)
    }
    pub fn transaction_count_by_block_number(&self, block_number: u64) -> Result<u64> {
        self.owner
            .inner
            .application
            .query_transaction_count_by_block_number(block_number)
    }
    pub fn transaction_count_by_block_hash(&self, block_hash: [u8; 32]) -> Result<u64> {
        self.owner
            .inner
            .application
            .query_transaction_count_by_block_hash(block_hash)
    }
    pub fn transaction_receipt_by_hash(&self, hash: [u8; 32]) -> Result<TransactionReceiptView> {
        self.owner
            .inner
            .application
            .query_transaction_receipt_by_hash(hash)
    }
    pub fn transaction_receipts_by_block_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<TransactionReceiptView>> {
        self.owner
            .inner
            .application
            .query_transaction_receipts_by_block_number(block_number)
    }
}
