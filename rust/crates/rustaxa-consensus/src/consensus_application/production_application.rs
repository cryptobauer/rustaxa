//! Closed existing-chain assembly. This module grants no corrected execution authority.
#[cfg(feature = "production_integration_test")]
pub(crate) mod integration_fixture;
use super::*;
mod certified_publication;
mod dag_gas;
pub use dag_gas::DagGasState;
mod proposal_egress;
mod transition_preparation;
use crate::consensus_query_api::*;
use crate::final_chain::{NativePredecessorCapture, NativePredecessorLimits};
use crate::network_api::*;
use crate::pbft_chain::{load_persisted_pbft_chain_head_identity, restore_pbft_chain_from_storage};
use crate::pbft_execution_validation::PbftExecutionSchedule;
use crate::verified_votes::PbftVoteType;
use crate::{
    DagNonFinalizedIndex, DagRuntimeStatus, PbftSyncIngressStep, SlashingSubmitterIdentity,
    TransactionPoolStatus,
};
pub use certified_publication::ProductionOwnerPublicationAuthority;
pub(crate) use proposal_egress::ProposalEgressPacket;
use rustaxa_storage::application_control::{
    ApplicationConcreteState, ApplicationMutationScope, ApplicationOwner, ApplicationReadScope,
    PBFT_CERTIFIED_INTENT_KEY,
};
use rustaxa_storage::{Column, ConcreteLifecycleObservation};
use rustaxa_types::DposValidatorStake;
use rustaxa_types::concrete_state::ConcreteStateIdentity;
use rustaxa_types::{FinalChainBlockNumber, evm_correction::CorrectionStage};
use std::time::Duration;
pub use transition_preparation::{
    ApplicationTransitionContext, CheckedProposalExecution, ProductionCertificateProofError,
    ProductionPreparationUnavailable, ProposalExecutionCheck, ScheduledCertificateStale,
    VerifiedCertificateEvidence,
};

const READ_WAIT: Duration = Duration::from_secs(1);
const NATIVE_SOURCE_LIMIT: usize = 64 * 1024 * 1024;

/// One permanent construction identity shared by the private application graph.
pub(crate) struct ApplicationScopeSource {
    owner: Arc<ApplicationOwner>,
    identity: Arc<()>,
    schedule: PbftExecutionSchedule,
    storage: Arc<Storage>,
    concrete_state: Arc<ApplicationConcreteState>,
    admission: ProductionAdmissionPolicy,
    cornus_period: u64,
    active: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
pub(crate) enum ApplicationComposition {
    Historical,
    Production(Arc<ApplicationScopeSource>),
}

impl ApplicationComposition {
    pub(crate) fn is_bound_to(&self, source: &Arc<ApplicationScopeSource>) -> bool {
        match self {
            Self::Historical => false,
            Self::Production(actual) => Arc::ptr_eq(actual, source),
        }
    }

    pub(crate) fn require_historical(&self) -> Result<()> {
        ensure!(
            matches!(self, Self::Historical),
            "PRODUCTION_UNBOUND_RUNTIME_ENTRY_REFUSED"
        );
        Ok(())
    }
}

pub(crate) struct BoundApplicationAction<'a> {
    source: &'a ApplicationScopeSource,
    guard: ApplicationReadScope<'a>,
    generation: u64,
}

impl ApplicationScopeSource {
    fn new(
        owner: Arc<ApplicationOwner>,
        schedule: PbftExecutionSchedule,
        storage: Arc<Storage>,
        concrete_state: Arc<ApplicationConcreteState>,
        admission: ProductionAdmissionPolicy,
        cornus_period: u64,
    ) -> Self {
        Self {
            owner,
            identity: Arc::new(()),
            schedule,
            storage,
            concrete_state,
            admission,
            cornus_period,
            active: std::sync::atomic::AtomicBool::new(true),
        }
    }

    #[track_caller]
    pub(crate) fn read(&self) -> Result<BoundApplicationAction<'_>> {
        let caller = std::panic::Location::caller();
        let guard = self.owner.try_read(READ_WAIT).map_err(|error| {
            let message = format!("application read at {caller}: {error}");
            anyhow::Error::new(error).context(message)
        })?;
        ensure!(
            self.active.load(std::sync::atomic::Ordering::Acquire),
            "PRODUCTION_OWNER_REOPEN_REQUIRED"
        );
        let generation = guard.generation();
        Ok(BoundApplicationAction {
            source: self,
            guard,
            generation,
        })
    }

    pub(crate) fn mutation(&self) -> Result<ApplicationMutationScope<'_>> {
        let guard = self.owner.begin_mutation()?;
        ensure!(
            self.active.load(std::sync::atomic::Ordering::Acquire),
            "PRODUCTION_OWNER_REOPEN_REQUIRED"
        );
        Ok(guard)
    }
}

/// The core has no service getter and cannot issue a production client.
pub struct BoundConsensusApplication {
    core: ConsensusApplication,
    source: Arc<ApplicationScopeSource>,
    predecessor: std::sync::Mutex<certified_publication::PublicationPredecessor>,
    query: crate::ConsensusQueryApi,
    production_configuration: ProductionConfiguration,
    transition_policy: transition_preparation::TransitionPolicy,
}

/// A borrowed read action. It cannot publish state or authorize a vote.
pub struct ApplicationExecutionView<'a> {
    application: &'a BoundConsensusApplication,
    action: &'a BoundApplicationAction<'a>,
}

impl BoundApplicationAction<'_> {
    fn validate(&self, application: &BoundConsensusApplication) -> Result<()> {
        ensure!(
            Arc::ptr_eq(&self.source.identity, &application.source.identity)
                && self.guard.is_current()
                && self.guard.generation() == self.generation,
            "PRODUCTION_APPLICATION_READ_BINDING_CHANGED"
        );
        Ok(())
    }
}

impl ApplicationExecutionView<'_> {
    pub fn production_configuration(&self) -> &ProductionConfiguration {
        &self.application.production_configuration
    }

    pub fn execution_configuration(&self) -> &crate::ExecutionConfiguration {
        // Qualification requires this value before the bound core is published.
        self.application
            .core
            .execution_configuration
            .as_ref()
            .expect("qualified configuration")
    }

    pub fn committed_identity(&self) -> Result<ConcreteStateIdentity> {
        self.action.validate(self.application)?;
        read_committed_identity(&self.application.core.storage)
    }
}

struct QualifiedExisting {
    final_chain: Arc<FinalChain>,
    configuration: crate::ExecutionConfiguration,
    production_configuration: ProductionConfiguration,
    concrete: ConcreteLifecycleObservation,
    schedule: PbftExecutionSchedule,
}

fn exact_status(storage: &Storage, field: StatusField, expected: u64) -> Result<()> {
    let raw = storage
        .get_raw(Column::Status, &[field as u8])?
        .context("PRODUCTION_REQUIRED_STATUS_MISSING")?;
    ensure!(
        raw.as_slice() == expected.to_le_bytes(),
        "PRODUCTION_STATUS_MISMATCH"
    );
    Ok(())
}

fn read_committed_identity(storage: &Storage) -> Result<ConcreteStateIdentity> {
    let raw = storage
        .final_chain()
        .meta_value(FinalChain::DB_META_LAST_NUMBER)?
        .context("PRODUCTION_FINAL_CHAIN_HEAD_MISSING")?;
    let bytes: [u8; 8] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("PRODUCTION_FINAL_CHAIN_HEAD_ENCODING"))?;
    let period = FinalChainBlockNumber::new(u64::from_le_bytes(bytes));
    let raw = storage
        .final_chain()
        .block_header_raw(period.as_u64())?
        .context("PRODUCTION_FINAL_CHAIN_HEADER_MISSING")?;
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw))?;
    Ok(ConcreteStateIdentity {
        period,
        state_root: header.state_root.0,
    })
}

fn check_existing_sources(
    bootstrap: &ConsensusApplicationBootstrap,
    storage: &Storage,
    concrete: &ConcreteLifecycleObservation,
) -> Result<H256> {
    ensure!(
        bootstrap.schema_major != 0,
        "CONSENSUS_APPLICATION_SCHEMA_MAJOR_ZERO"
    );
    exact_status(
        storage,
        StatusField::DbMajorVersion,
        u64::from(bootstrap.schema_major),
    )?;
    exact_status(
        storage,
        StatusField::DbMinorVersion,
        u64::from(bootstrap.schema_minor),
    )?;
    exact_status(
        storage,
        StatusField::FinalChainRootPolicy,
        FINAL_CHAIN_CONCRETE_ROOT_POLICY_V1,
    )?;
    ensure!(
        storage.metadata().genesis_hash()?.as_deref()
            == Some(bootstrap.storage_genesis_hash.as_bytes()),
        "PRODUCTION_GENESIS_MISMATCH"
    );
    let pairing = storage
        .final_chain()
        .concrete_state_pairing_raw()?
        .context("PRODUCTION_CONCRETE_PAIRING_MISSING")?;
    let expected_chain = concrete_chain_config_identity(bootstrap.storage_genesis_hash);
    ensure!(
        concrete.identity.database_id != [0; 32]
            && concrete.identity.chain_id.as_slice() == expected_chain,
        "PRODUCTION_CONCRETE_IDENTITY_MISMATCH"
    );
    let mut canonical = RlpStream::new_list(4);
    canonical
        .append(&1_u64)
        .append(&bootstrap.storage_genesis_hash.as_bytes())
        .append(&concrete.identity.database_id.as_slice())
        .append(&concrete.identity.chain_id.as_slice());
    ensure!(
        canonical.out().as_ref() == pairing,
        "PRODUCTION_CONCRETE_PAIRING_MISMATCH"
    );
    ensure!(
        read_committed_identity(storage)? == concrete.committed,
        "PRODUCTION_RECOVERY_INPUT_REQUIRED: committed identities differ"
    );
    ensure!(
        concrete.pending_marker_rlp.is_empty()
            && storage
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none()
            && storage
                .get_raw(Column::Status, PBFT_CERTIFIED_INTENT_KEY)?
                .is_none(),
        "PRODUCTION_RECOVERY_INPUT_REQUIRED: pending publication"
    );
    ensure!(
        storage.metadata().pbft_execution_schedule_raw()?.is_none(),
        "PRODUCTION_SCHEDULE_APPROVAL_UNAVAILABLE"
    );
    let head = load_persisted_pbft_chain_head_identity(storage)?
        .context("PRODUCTION_PBFT_HEAD_MISSING")?;
    ensure!(
        head.size == concrete.committed.period.as_u64(),
        "PRODUCTION_RECOVERY_INPUT_REQUIRED: PBFT head differs"
    );
    let restored = restore_pbft_chain_from_storage(storage)?;
    ensure!(
        !restored.initialized_default
            && restored.head.size == head.size
            && restored.head.last_pbft_block_hash == head.last_pbft_block_hash,
        "PRODUCTION_PBFT_HEAD_CHANGED"
    );
    if head.size != 0 {
        ensure!(
            storage.period().by_pbft_hash(head.last_pbft_block_hash)? == Some(head.size),
            "PRODUCTION_PBFT_PERIOD_MAPPING_MISMATCH"
        );
        ensure!(
            crate::pbft_chain::load_pbft_block_from_storage(storage, head.last_pbft_block_hash)?
                .found,
            "PRODUCTION_PBFT_BLOCK_MISSING"
        );
    }
    let genesis = storage
        .final_chain()
        .block_header_raw(0)?
        .context("PRODUCTION_GENESIS_HEADER_MISSING")?;
    for period in [0, concrete.committed.period.as_u64()] {
        ensure!(
            storage.final_chain().dpos_snapshot_raw(period)?.is_some()
                && storage
                    .final_chain()
                    .account_snapshot_raw(period)?
                    .is_some(),
            "PRODUCTION_NATIVE_SNAPSHOT_MISSING: period {period}"
        );
    }
    Ok(StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&genesis))?.state_root)
}

impl BoundConsensusApplication {
    #[cfg(feature = "production_integration_test")]
    pub fn integration_runtime_tick(&self) -> Result<u64> {
        let _read = self.source.read()?;
        self.core
            .pbft
            .runtime_session_next()
            .map(|step| step.tick_id)
            .context("PRODUCTION_TEST_RUNTIME_CURSOR_MISSING")
    }

    #[cfg(feature = "production_integration_test")]
    pub fn integration_replace_runtime_tick(&self) -> Result<()> {
        let _operation = self.source.mutation()?;
        self.core.pbft.replace_owned_runtime_session_for_test()
    }

    /// Assembly only. The execution factory owns the concrete handle and all clients.
    /// Account facts must come from the opaque owned lifecycle, not a caller port.
    ///
    /// ```compile_fail
    /// use rustaxa_consensus::ApplicationCommittedStateReader;
    /// struct FabricatedAccounts;
    /// impl ApplicationCommittedStateReader for FabricatedAccounts {}
    /// ```
    #[doc(hidden)]
    pub fn restore_existing(
        selected: &rustaxa_native_config::SelectedProductionConfig,
        storage: Arc<Storage>,
        owner: Arc<ApplicationOwner>,
        schedule: PbftExecutionSchedule,
        concrete_state: Arc<ApplicationConcreteState>,
    ) -> Result<Self> {
        let bootstrap = crate::production_bootstrap::application_bootstrap_from_selected(selected)?;
        let admission = ProductionAdmissionPolicy::from_selected(selected);
        Self::restore_existing_components(
            bootstrap,
            storage,
            owner,
            schedule,
            concrete_state,
            admission,
        )
    }

    fn restore_existing_components(
        mut bootstrap: ConsensusApplicationBootstrap,
        storage: Arc<Storage>,
        owner: Arc<ApplicationOwner>,
        schedule: PbftExecutionSchedule,
        concrete_state: Arc<ApplicationConcreteState>,
        admission: ProductionAdmissionPolicy,
    ) -> Result<Self> {
        ensure!(
            owner.is_for(&storage),
            "PRODUCTION_FOREIGN_APPLICATION_OWNER"
        );
        ensure!(
            concrete_state.is_for(&owner),
            "PRODUCTION_FOREIGN_CONCRETE_OWNER"
        );
        ensure!(
            schedule.activation_period().is_none(),
            "PRODUCTION_SCHEDULE_APPROVAL_UNAVAILABLE"
        );
        let qualified = {
            let read = owner.try_read(READ_WAIT)?;
            let concrete = concrete_state.observation(&read)?;
            let genesis_root = check_existing_sources(&bootstrap, &storage, &concrete)?;
            let input = &bootstrap.final_chain;
            let native = FinalChain::prepare_native_execution_configuration(
                input.block_gas_limit,
                input.genesis_timestamp,
                genesis_root,
                true,
                input.genesis_accounts.clone(),
                input.genesis_validators.clone(),
                &input.genesis_dpos,
                input.rewards.clone(),
                bootstrap.consensus.pbft.ficus_activation_period.into(),
            )?
            .fixed(CorrectionStage::Historical);
            let configuration = crate::ExecutionConfiguration::prepare(
                bootstrap.storage_genesis_hash,
                &native,
                &bootstrap.consensus,
                input.bridge_contract_address,
            )?;
            let production_configuration = ProductionConfiguration::new(&configuration, &admission);
            let expected_schedule = PbftExecutionSchedule::new(
                bootstrap.consensus.pbft.network_identity.chain_id,
                bootstrap.storage_genesis_hash.0,
                production_configuration.digest(),
                None,
            );
            ensure!(
                schedule == expected_schedule,
                "PRODUCTION_SCHEDULE_CONFIGURATION_MISMATCH"
            );
            schedule.verify_enrollment(None)?;
            certified_publication::check_rule_binding(&storage, &schedule)?;
            let final_chain = Arc::new(
                FinalChain::new_with_genesis_state_root_and_ficus_activation(
                    Arc::clone(&storage),
                    input.block_gas_limit,
                    input.genesis_timestamp,
                    genesis_root,
                    true,
                    input.genesis_accounts.clone(),
                    input.genesis_validators.clone(),
                    input.genesis_dpos.clone(),
                    input.rewards.clone(),
                    bootstrap.consensus.pbft.ficus_activation_period.into(),
                )?,
            );
            let captured = crate::ExecutionConfiguration::capture(
                &storage,
                bootstrap.storage_genesis_hash,
                &final_chain,
                &bootstrap.consensus,
                input.bridge_contract_address,
            )?;
            ensure!(
                captured == configuration,
                "PRODUCTION_CONFIGURATION_CHANGED_DURING_RESTORE"
            );
            let pending = concrete
                .committed
                .period
                .checked_next()
                .context("PRODUCTION_COMMITTED_PERIOD_OVERFLOW")?;
            {
                let capture = final_chain.capture_native_predecessor(
                    pending,
                    NativePredecessorLimits {
                        max_retained_source_bytes: NATIVE_SOURCE_LIMIT,
                    },
                )?;
                let NativePredecessorCapture::Ready(capture) = capture else {
                    bail!("PRODUCTION_NATIVE_PREDECESSOR_UNAVAILABLE");
                };
                ensure!(
                    capture.committed_identity() == concrete.committed
                        && capture.pairing_identity() == concrete.identity
                        && capture.pairing_storage_genesis() == bootstrap.storage_genesis_hash.0,
                    "PRODUCTION_NATIVE_PREDECESSOR_MISMATCH"
                );
            }
            QualifiedExisting {
                final_chain,
                configuration,
                production_configuration,
                concrete,
                schedule,
            }
        };
        bootstrap.final_chain.state_api_committed_period =
            qualified.concrete.committed.period.as_u64();
        bootstrap.final_chain.state_api_committed_root =
            H256(qualified.concrete.committed.state_root);
        let source = Arc::new(ApplicationScopeSource::new(
            owner,
            qualified.schedule,
            Arc::clone(&storage),
            concrete_state,
            admission,
            bootstrap.consensus.pbft.cornus_activation_period,
        ));
        let signing_addresses: Vec<_> = bootstrap
            .consensus
            .signing_identities
            .iter()
            .map(|identity| identity.address)
            .collect();
        let transition_policy = transition_preparation::TransitionPolicy::new(&bootstrap)?;
        let core = {
            // A complete existing restore needs no default, repair, or normalization write.
            let _read = source.read()?;
            check_existing_sources(&bootstrap, &storage, &qualified.concrete)?;
            let core = ConsensusApplication::restore_composed(
                storage,
                qualified.final_chain,
                bootstrap.consensus,
                bootstrap.final_chain.bridge_contract_address,
                Ok(qualified.configuration),
                ApplicationComposition::Production(Arc::clone(&source)),
            )
            .context("PRODUCTION_EXISTING_RESTORE_UNAVAILABLE")?;
            core.pbft.check_restored_live_votes(&core.final_chain)?;
            core.dag_transaction
                .transaction_recover_non_finalized_read_only()?;
            core
        };
        {
            let _operation = source.mutation()?;
            let startup = crate::consensus_application_startup::prepare_consensus_startup(
                &core.pbft,
                &core.dag_transaction,
                &core.final_chain,
                &signing_addresses,
            )?;
            ensure!(
                startup.finalizations.is_empty(),
                "PRODUCTION_STARTUP_EXECUTION_REQUIRED"
            );
            ensure!(
                core.pbft.finalization_ready(&core.final_chain)?,
                "CONSENSUS_STARTUP_FINALIZATION_NOT_READY"
            );
            crate::consensus_application_startup::hydrate_recently_finalized_transactions(
                &startup,
                &core.dag_transaction,
            )?;
            ensure!(
                read_committed_identity(&core.storage)? == qualified.concrete.committed,
                "PRODUCTION_STARTUP_PREDECESSOR_CHANGED"
            );
            if let Some(persisted) = &startup.persisted_pillar_vote {
                let anchor = core.pbft.current_pillar_block_rlp()?;
                if !anchor.is_empty() {
                    use rustaxa_types::{PillarBlock, PillarVote};
                    let anchor = PillarBlock::decode_rlp(&anchor)?;
                    let vote = PillarVote::decode_rlp(&persisted.vote_rlp)?;
                    if vote.block_hash == anchor.hash() && persisted.dpos_period == anchor.period {
                        let period = anchor.period.into();
                        crate::consensus_application_startup::apply_startup_persisted_pillar_vote(
                            persisted,
                            core.final_chain
                                .dpos_eligible_vote_count(period, persisted.voter)?,
                            core.final_chain.dpos_eligible_total_vote_count(period)?,
                            &core.pbft,
                        )?;
                    }
                }
            }
            core.pbft.persist_prepared_startup_step()?;
            crate::consensus_application_startup::complete_consensus_startup(&core.pbft)?;
        }
        let query = core.consensus_query_api_for_bridge();
        ensure!(
            core.composition.is_bound_to(&source)
                && core.runtime.is_bound_to_scope(&source)
                && core.ingress.composition.is_bound_to(&source)
                && core.network_api.is_bound_to_scope(&source)
                && query.is_bound_to_scope(&source),
            "PRODUCTION_SCOPE_GRAPH_MISMATCH"
        );
        {
            let _operation = source.mutation()?;
            core.final_chain.authorize_reward_scheduler_joint_startup(
                transition_policy.epoch,
                qualified.concrete.committed.period,
            )?;
            core.final_chain
                .complete_reward_scheduler_recovery(transition_policy.epoch)?;
        }
        Ok(Self {
            core,
            source,
            predecessor: std::sync::Mutex::new(certified_publication::PublicationPredecessor::new(
                qualified.concrete,
            )),
            query,
            production_configuration: qualified.production_configuration,
            transition_policy,
        })
    }

    /// Pure configuration identity used by the sole execution factory.
    #[doc(hidden)]
    pub fn prepare_existing_schedule(
        selected: &rustaxa_native_config::SelectedProductionConfig,
        storage: &Storage,
        owner: &ApplicationOwner,
        concrete_state: &ApplicationConcreteState,
    ) -> Result<PbftExecutionSchedule> {
        let bootstrap = crate::production_bootstrap::application_bootstrap_from_selected(selected)?;
        let admission = ProductionAdmissionPolicy::from_selected(selected);
        ensure!(
            concrete_state.is_for(owner),
            "PRODUCTION_FOREIGN_CONCRETE_OWNER"
        );
        let concrete = {
            let read = owner.try_read(READ_WAIT)?;
            concrete_state.observation(&read)?
        };
        Self::prepare_existing_component_schedule(&bootstrap, storage, owner, &concrete, &admission)
    }

    fn prepare_existing_component_schedule(
        bootstrap: &ConsensusApplicationBootstrap,
        storage: &Storage,
        owner: &ApplicationOwner,
        concrete: &ConcreteLifecycleObservation,
        admission: &ProductionAdmissionPolicy,
    ) -> Result<PbftExecutionSchedule> {
        ensure!(
            owner.is_for(storage),
            "PRODUCTION_FOREIGN_APPLICATION_OWNER"
        );
        let _read = owner.try_read(READ_WAIT)?;
        let root = check_existing_sources(bootstrap, storage, concrete)?;
        let input = &bootstrap.final_chain;
        let native = FinalChain::prepare_native_execution_configuration(
            input.block_gas_limit,
            input.genesis_timestamp,
            root,
            true,
            input.genesis_accounts.clone(),
            input.genesis_validators.clone(),
            &input.genesis_dpos,
            input.rewards.clone(),
            bootstrap.consensus.pbft.ficus_activation_period.into(),
        )?
        .fixed(CorrectionStage::Historical);
        let configuration = crate::ExecutionConfiguration::prepare(
            bootstrap.storage_genesis_hash,
            &native,
            &bootstrap.consensus,
            input.bridge_contract_address,
        )?;
        let production = ProductionConfiguration::new(&configuration, admission);
        let schedule = PbftExecutionSchedule::new(
            bootstrap.consensus.pbft.network_identity.chain_id,
            bootstrap.storage_genesis_hash.0,
            production.digest(),
            None,
        );
        certified_publication::check_rule_binding(storage, &schedule)?;
        Ok(schedule)
    }

    #[doc(hidden)]
    pub fn is_bound_to(&self, owner: &ApplicationOwner) -> bool {
        std::ptr::eq(self.source.owner.as_ref(), owner) && owner.is_for(&self.core.storage)
    }

    #[doc(hidden)]
    pub fn with_execution_view<R>(
        &self,
        operation: impl for<'scope> FnOnce(ApplicationExecutionView<'scope>) -> Result<R>,
    ) -> Result<R> {
        let action = self.source.read()?;
        action.validate(self)?;
        operation(ApplicationExecutionView {
            application: self,
            action: &action,
        })
    }

    #[doc(hidden)]
    pub fn check_concrete_observation(
        &self,
        observation: &ConcreteLifecycleObservation,
    ) -> Result<()> {
        let _action = self.source.read()?;
        ensure!(
            self.is_current_concrete_observation(observation)?,
            "PRODUCTION_CONCRETE_OBSERVATION_CHANGED"
        );
        ensure!(
            read_committed_identity(&self.core.storage)? == observation.committed,
            "PRODUCTION_PRIMARY_COMMITTED_IDENTITY_CHANGED"
        );
        Ok(())
    }

    #[doc(hidden)]
    /// Reads the requested finalized checkpoint under one application read scope.
    /// A future period has no result. Missing or incompatible retained data fails.
    pub fn with_finalized_state<R>(
        &self,
        period: Option<u64>,
        read: impl FnOnce(
            &rustaxa_storage::ConcreteCheckpointReaders,
            ConcreteStateIdentity,
        ) -> Result<R>,
    ) -> Result<Option<R>> {
        let action = self.source.read()?;
        self.with_finalized_state_in_action(&action, period, read)
    }

    fn with_finalized_state_in_action<R>(
        &self,
        action: &BoundApplicationAction<'_>,
        period: Option<u64>,
        read: impl FnOnce(
            &rustaxa_storage::ConcreteCheckpointReaders,
            ConcreteStateIdentity,
        ) -> Result<R>,
    ) -> Result<Option<R>> {
        action.validate(self)?;
        let Some((committed, requested)) = self.source.finalized_state_identities(period)? else {
            return Ok(None);
        };
        self.source
            .concrete_state
            .with_checkpoint(&action.guard, committed, requested, |checkpoint| {
                read(checkpoint, requested)
            })
            .map(Some)
    }

    pub fn submit_public_transaction_from_native_state(
        &self,
        request: PublicTransactionSubmissionRequest,
    ) -> Result<PublicTransactionSubmissionReport> {
        let _action = self.source.mutation()?;
        self.core
            .submit_public_transaction_from_native_state(request)
    }

    #[doc(hidden)]
    pub fn prune_light_history(
        &self,
        request: rustaxa_storage::LightHistoryPruneRequest,
    ) -> Result<rustaxa_storage::LightHistoryPruneReport> {
        let _action = self.source.mutation()?;
        self.core.prune_light_history(request)
    }

    #[doc(hidden)]
    pub fn consensus_vote_status(&self) -> Result<super::ConsensusVoteStatus> {
        let _action = self.source.read()?;
        self.core.consensus_vote_status()
    }

    #[doc(hidden)]
    pub fn consensus_live_status(&self) -> Result<ConsensusLiveStatus> {
        let _action = self.source.read()?;
        self.core.consensus_live_status()
    }

    #[doc(hidden)]
    pub fn run<P, S, T, E, O>(
        &self,
        process: &P,
        signer: &S,
        transport: &T,
        evm: &E,
        observer: &O,
    ) -> Result<ConsensusRunExit>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        E: ConsensusExecutionPort,
        O: crate::ConsensusObserverPort,
    {
        self.core.runtime.run_bound(
            crate::ConsensusRuntimeServices {
                pbft: &self.core.pbft,
                dag: &self.core.dag_transaction,
                final_chain: &self.core.final_chain,
            },
            &self.source,
            process,
            signer,
            transport,
            evm,
            observer,
        )
    }
}

pub(crate) enum ApplicationOperationScope<'a> {
    Historical,
    Read(BoundApplicationAction<'a>),
    Mutation(&'a ApplicationScopeSource, ApplicationMutationScope<'a>),
}

impl ApplicationOperationScope<'_> {
    pub(crate) fn generation(&self) -> Option<u64> {
        match self {
            Self::Historical => None,
            Self::Read(action) => Some(action.generation),
            Self::Mutation(_, scope) => Some(scope.generation()),
        }
    }

    pub(crate) fn follows(&self, previous: Option<u64>) -> bool {
        match (previous, self.generation()) {
            (None, None) => true,
            (Some(previous), Some(current)) => previous.checked_add(1) == Some(current),
            _ => false,
        }
    }
}

impl ApplicationComposition {
    pub(crate) fn has_live_production_successor(&self) -> bool {
        matches!(self, Self::Production(source) if source.active.load(std::sync::atomic::Ordering::Acquire))
    }

    pub(crate) fn mutation(&self) -> Result<ApplicationOperationScope<'_>> {
        match self {
            Self::Historical => Ok(ApplicationOperationScope::Historical),
            Self::Production(source) => Ok(ApplicationOperationScope::Mutation(
                source,
                source.mutation()?,
            )),
        }
    }

    #[track_caller]
    pub(crate) fn callback(&self) -> Result<ApplicationOperationScope<'_>> {
        match self {
            Self::Historical => Ok(ApplicationOperationScope::Historical),
            Self::Production(source) => {
                source.owner.ensure_not_waiting()?;
                Ok(ApplicationOperationScope::Read(source.read()?))
            }
        }
    }

    pub(crate) fn require_execution_plan(&self, boundary: &'static str) -> Result<()> {
        match self {
            Self::Historical => Ok(()),
            Self::Production(_) => bail!("PRODUCTION_EXECUTION_PLAN_UNAVAILABLE: {boundary}"),
        }
    }
}

impl ApplicationScopeSource {
    pub(crate) fn with_finalized_state_in_mutation<R>(
        &self,
        scope: &ApplicationMutationScope<'_>,
        period: Option<u64>,
        read: impl FnOnce(
            &rustaxa_storage::ConcreteCheckpointReaders,
            ConcreteStateIdentity,
        ) -> Result<R>,
    ) -> Result<Option<R>> {
        let Some((committed, requested)) = self.finalized_state_identities(period)? else {
            return Ok(None);
        };
        self.concrete_state
            .with_mutation_checkpoint(scope, committed, requested, |checkpoint| {
                read(checkpoint, requested)
            })
            .map(Some)
    }

    fn finalized_state_identities(
        &self,
        period: Option<u64>,
    ) -> Result<Option<(ConcreteStateIdentity, ConcreteStateIdentity)>> {
        let committed = read_committed_identity(&self.storage)?;
        let committed_period = committed.period.as_u64();
        let requested = match period {
            Some(period) if period > committed_period => return Ok(None),
            Some(period) if period < committed_period => {
                let bytes = self
                    .storage
                    .final_chain()
                    .block_header_raw(period)?
                    .context("PRODUCTION_QUERY_FINAL_CHAIN_HEADER_MISSING")?;
                let header =
                    StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&bytes))?;
                ConcreteStateIdentity {
                    period: FinalChainBlockNumber::new(period),
                    state_root: header.state_root.0,
                }
            }
            _ => committed,
        };
        Ok(Some((committed, requested)))
    }

    pub(crate) fn account_records(
        &self,
        expected_period: FinalChainBlockNumber,
        addresses: &[[u8; 20]],
    ) -> Result<Vec<Option<rustaxa_types::concrete_state::ConcreteAccountRecord>>> {
        let operation = self.mutation()?;
        let expected = read_committed_identity(&self.storage)?;
        ensure!(
            expected.period == expected_period,
            "PRODUCTION_ACCOUNT_HEAD_MISMATCH"
        );
        let records = self
            .concrete_state
            .accounts(&operation, expected, addresses)?;
        ensure!(
            records.len() == addresses.len(),
            "PRODUCTION_ACCOUNT_READ_COUNT_MISMATCH"
        );
        Ok(records)
    }
}

pub(crate) fn bounded_account_integer(bytes: &[u8]) -> Result<U256> {
    ensure!(
        bytes.len() <= 32,
        "PRODUCTION_ACCOUNT_ADMISSION_WIDTH_UNAVAILABLE"
    );
    Ok(U256::from_big_endian(bytes))
}

impl ApplicationScopeSource {
    pub(crate) fn wait_callback(
        &self,
    ) -> Result<rustaxa_storage::application_control::ApplicationWaitCallbackScope<'_>> {
        Ok(self.owner.try_wait_callback()?)
    }
}

/// Immutable admission values from the selected network. They are not network approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionAdmissionPolicy {
    transaction_max_gas: rustaxa_types::FinalChainGas,
    transaction_min_gas_price: U256,
}

impl ProductionAdmissionPolicy {
    pub fn transaction_max_gas(&self) -> rustaxa_types::FinalChainGas {
        self.transaction_max_gas
    }
    pub fn transaction_min_gas_price(&self) -> U256 {
        self.transaction_min_gas_price
    }
    fn from_selected(source: &rustaxa_native_config::SelectedProductionConfig) -> Self {
        Self {
            transaction_max_gas: source.transaction_max_gas().into(),
            transaction_min_gas_price: U256::from(source.transaction_min_gas_price()),
        }
    }
}

/// Complete production constructor identity. Historical execution bytes stay unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionConfiguration {
    canonical: Vec<u8>,
    digest: [u8; 32],
}

impl ProductionConfiguration {
    fn new(historical: &crate::ExecutionConfiguration, policy: &ProductionAdmissionPolicy) -> Self {
        use tiny_keccak::{Hasher, Keccak};
        let mut record = RlpStream::new_list(5);
        record
            .append(&b"rustaxa/production-configuration".as_slice())
            .append(&1_u8)
            .append(&historical.canonical_bytes())
            .append(&policy.transaction_max_gas.as_u64())
            .append(&policy.transaction_min_gas_price);
        let canonical = record.out().to_vec();
        let mut digest = [0; 32];
        let mut hash = Keccak::v256();
        hash.update(&canonical);
        hash.finalize(&mut digest);
        Self { canonical, digest }
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

impl ApplicationScopeSource {
    pub(crate) fn check_admission_request(
        &self,
        request: &PublicTransactionSubmissionRequest,
        committed: FinalChainBlockNumber,
    ) -> Result<()> {
        ensure!(
            request.expected_chain_id == self.schedule.chain_id()
                && request.last_block_number == committed.as_u64()
                && request.maximum_gas_limit == self.admission.transaction_max_gas.as_u64()
                && request.minimum_gas_price == self.admission.transaction_min_gas_price
                && request.cornus_active == (committed.as_u64() >= self.cornus_period),
            "PRODUCTION_TRANSACTION_ADMISSION_POLICY_MISMATCH"
        );
        Ok(())
    }
}

#[cfg(test)]
mod wait_tests;

#[cfg(test)]
mod configuration_tests;

impl BoundConsensusApplication {
    #[doc(hidden)]
    pub fn network_drain_work(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        source_scoped: bool,
        budget: u32,
    ) -> Result<NetworkEffectBatch> {
        let action = self.source.mutation()?;
        let batch = match self.source.authorize_proposal_egress_drain(
            action.generation(),
            transport_lane,
            source_payload_id,
            source_scoped,
        ) {
            Some(true) => self.core.pbft.network_service().drain_proposal_egress(
                transport_lane,
                source_payload_id,
                budget,
            )?,
            Some(false) => {
                self.core
                    .pbft
                    .network_service()
                    .cancel_proposal_egress_effects(transport_lane, source_payload_id)?;
                bail!("PRODUCTION_PROPOSAL_EGRESS_DRAIN_STALE");
            }
            None => self.core.network_api.drain_work(
                transport_lane,
                source_payload_id,
                source_scoped,
                budget,
            )?,
        };
        self.source.note_proposal_egress_drain(
            action.generation(),
            transport_lane,
            source_payload_id,
            source_scoped,
            &batch,
        );
        Ok(batch)
    }
    #[doc(hidden)]
    pub fn network_report_effect_results(
        &self,
        results: Vec<NetworkEffectResult>,
    ) -> Result<NetworkEffectAck> {
        let action = self.source.mutation()?;
        let ids: Vec<_> = results.iter().map(|result| result.effect_id).collect();
        let ack = self.core.network_api.report_effect_results(results)?;
        self.source
            .note_proposal_egress_acknowledgement(action.generation(), &ids, &ack);
        Ok(ack)
    }
    #[doc(hidden)]
    pub fn network_begin_pbft_sync(
        &self,
        request: NetworkPbftSyncStartRequest,
    ) -> Result<NetworkPbftSyncStartOutcome> {
        let _action = self.source.mutation()?;
        self.core.network_api.begin_pbft_sync(request)
    }
    #[doc(hidden)]
    pub fn network_ingest_status_packet(
        &self,
        request: NetworkStatusPacketRequest,
    ) -> Result<NetworkStatusPacketReport> {
        let _action = self.source.mutation()?;
        self.core.network_api.ingest_status_packet(request)
    }
    #[doc(hidden)]
    pub fn network_build_status_packet(
        &self,
        request: NetworkStatusPacketBuildRequest,
    ) -> Result<NetworkStatusPacketBuildOutcome> {
        let _action = self.source.mutation()?;
        self.core.network_api.build_status_packet(request)
    }
    #[doc(hidden)]
    pub fn network_apply_pbft_sync_command(
        &self,
        request: NetworkPbftSyncCommandRequest,
    ) -> Result<NetworkPbftSyncCommandOutcome> {
        let _action = self.source.mutation()?;
        self.core.network_api.apply_pbft_sync_command(request)
    }
    #[doc(hidden)]
    pub fn network_request_pending_dag_blocks(
        &self,
        transport_lane: u32,
        source_payload_id: u64,
        facts: NetworkPendingDagBlocksRequestFacts,
    ) -> Result<NetworkIngressDecision> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .request_pending_dag_blocks(transport_lane, source_payload_id, facts)
    }
    #[doc(hidden)]
    pub fn network_ingest_pbft_vote_packet(
        &self,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_pbft_vote_packet(request, slashing_submitters)
    }
    #[doc(hidden)]
    pub fn network_ingest_pbft_votes_bundle_packet(
        &self,
        request: NetworkConsensusPacketRequest,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<NetworkPbftVotePacketReport> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_pbft_votes_bundle_packet(request, slashing_submitters)
    }
    #[doc(hidden)]
    pub fn network_ingest_pillar_vote_packet(
        &self,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        let _action = self.source.mutation()?;
        self.core.network_api.ingest_pillar_vote_packet(request)
    }
    #[doc(hidden)]
    pub fn network_ingest_pillar_votes_bundle_packet(
        &self,
        request: NetworkConsensusPacketRequest,
    ) -> Result<NetworkPillarVotePacketReport> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_pillar_votes_bundle_packet(request)
    }
    #[doc(hidden)]
    pub fn network_ingest_pbft_next_votes_bundle_request(
        &self,
        request: NetworkPbftNextVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_pbft_next_votes_bundle_request(request)
    }
    #[doc(hidden)]
    pub fn network_ingest_pillar_votes_bundle_request(
        &self,
        request: NetworkGetPillarVotesBundlePacketRequest,
    ) -> Result<NetworkIngressDecision> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_pillar_votes_bundle_request(request)
    }
    #[doc(hidden)]
    pub fn network_ingest_get_pbft_sync_request(
        &self,
        request: NetworkGetPbftSyncRequest,
    ) -> Result<NetworkIngressDecision> {
        let _action = self.source.mutation()?;
        self.core.network_api.ingest_get_pbft_sync_request(request)
    }
    #[doc(hidden)]
    pub fn network_ingest_pbft_blocks_bundle(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
    ) -> Result<NetworkIngressDecision> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_pbft_blocks_bundle(packet_rlp, source_payload_id)
    }
    #[doc(hidden)]
    pub fn network_ingest_get_dag_sync_request(
        &self,
        context: NetworkGetDagSyncContext,
        request_rlp: &[u8],
    ) -> Result<NetworkIngressDecision> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_get_dag_sync_request(context, request_rlp)
    }
    #[doc(hidden)]
    pub fn network_ingest_transaction_packet(
        &self,
        context: NetworkTransactionPacketContext,
        packet_rlp: &[u8],
        policy: PublicTransactionSubmissionRequest,
    ) -> Result<NetworkTransactionPacketReport> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_transaction_packet(context, packet_rlp, policy)
    }
    #[doc(hidden)]
    pub fn network_ingest_dag_block_packet<E: ConsensusExecutionPort>(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        execution: &E,
    ) -> Result<NetworkDagBlockIngressReport> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .ingest_dag_block_packet(context, packet_rlp, execution)
    }
    #[doc(hidden)]
    pub fn network_ingest_dag_sync_packet<E: ConsensusExecutionPort>(
        &self,
        context: NetworkDagBlockIngressContext,
        packet_rlp: &[u8],
        _policy: PublicTransactionSubmissionRequest,
        execution: &E,
    ) -> Result<NetworkDagSyncIngressReport> {
        let _action = self.source.mutation()?;
        let period = self.core.final_chain.last_block_number()?;
        let policy = PublicTransactionSubmissionRequest {
            transaction_rlp: Vec::new(),
            expected_chain_id: self.source.schedule.chain_id(),
            last_block_number: period,
            maximum_gas_limit: self.source.admission.transaction_max_gas.as_u64(),
            minimum_gas_price: self.source.admission.transaction_min_gas_price,
            cornus_active: period >= self.source.cornus_period,
        };
        self.core.network_api.ingest_dag_sync_packet(
            context,
            packet_rlp,
            policy,
            execution,
            Some(period),
        )
    }
    #[doc(hidden)]
    pub fn network_transaction_gas_price_bid(&self) -> Result<[u8; 32]> {
        let _action = self.source.mutation()?;
        self.core.network_api.transaction_gas_price_bid()
    }
    #[doc(hidden)]
    pub fn network_submit_transaction_from_native_state(
        &self,
        request: PublicTransactionSubmissionRequest,
    ) -> Result<PublicTransactionSubmissionReport> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .submit_transaction_from_native_state(request)
    }
    #[doc(hidden)]
    pub fn network_prepare_egress(
        &self,
        request: NetworkEgressPrepareRequest,
    ) -> Result<NetworkEgressPreparation> {
        let action = self.source.mutation()?;
        if !proposal_egress::requires_proposal_egress_lease(&request) {
            return self.core.network_api.prepare_egress(request);
        }
        self.source
            .authorize_proposal_egress_preparation(action.generation(), &request)?;
        let lane = request.transport_lane;
        self.core
            .pbft
            .network_service()
            .check_proposal_egress_slot(lane, request.source_payload_id)?;
        let prepared = self.core.network_api.prepare_egress(request)?;
        self.source
            .note_proposal_egress_preparation(action.generation(), prepared.token, lane)?;
        Ok(prepared)
    }
    #[doc(hidden)]
    pub fn network_plan_egress(
        &self,
        request: NetworkEgressPlanRequest,
    ) -> Result<NetworkIngressDecision> {
        let action = self.source.mutation()?;
        if !self
            .core
            .pbft
            .network_service()
            .proposal_egress_token(request.token)?
        {
            return self.core.network_api.plan_egress(request);
        }
        self.source
            .authorize_proposal_egress_plan(action.generation(), request.token)?;
        let result = self
            .core
            .pbft
            .network_service()
            .plan_proposal_egress(request)?;
        self.source.note_proposal_egress_plan(action.generation())?;
        Ok(result)
    }
    #[doc(hidden)]
    pub fn network_cancel_egress(&self, token: u64) -> Result<bool> {
        let _action = self.source.mutation()?;
        self.core.network_api.cancel_egress(token)
    }
    #[doc(hidden)]
    pub fn network_begin_pbft_sync_ingress(
        &self,
        packet_rlp: &[u8],
        source_payload_id: u64,
        source_peer_id: [u8; 64],
        sync_generation: u64,
        _slashing_submitters: Vec<SlashingSubmitterIdentity>,
    ) -> Result<PbftSyncIngressStep> {
        let _action = self.source.mutation()?;
        let correlation = self.core.pbft.network_service().admit_pbft_sync_source(
            crate::network_api::NetworkPbftSyncSourceRequest {
                peer_id: source_peer_id,
                source: crate::network_api::NETWORK_PBFT_SYNC_SOURCE_ACTIVE,
            },
        )?;
        if !correlation.accepted || correlation.generation != sync_generation {
            return Ok(PbftService::dropped_sync_ingress(
                source_payload_id,
                if correlation.accepted {
                    "PRODUCTION_PBFT_SYNC_GENERATION_CHANGED".into()
                } else {
                    correlation.error_code
                },
            ));
        }
        let submitters = self
            .core
            .runtime
            .slashing_submitters(&self.core.final_chain)?;
        self.core.network_api.begin_pbft_sync_ingress(
            packet_rlp,
            source_payload_id,
            source_peer_id,
            sync_generation,
            submitters,
        )
    }
    #[doc(hidden)]
    pub fn network_report_pbft_sync_ingress_slashing(
        &self,
        proof_hash: H256,
        transaction_inserted: bool,
        source: crate::pbft_service::PbftSyncIngressSource,
    ) -> Result<PbftSyncIngressStep> {
        let _action = self.source.mutation()?;
        let correlation = self.core.pbft.network_service().admit_pbft_sync_source(
            crate::network_api::NetworkPbftSyncSourceRequest {
                peer_id: source.peer_id,
                source: crate::network_api::NETWORK_PBFT_SYNC_SOURCE_ACTIVE,
            },
        )?;
        if !correlation.accepted || correlation.generation != source.generation {
            return Ok(PbftService::dropped_sync_ingress(
                source.payload_id,
                "PRODUCTION_PBFT_SYNC_GENERATION_CHANGED",
            ));
        }
        self.core.network_api.report_pbft_sync_ingress_slashing(
            proof_hash,
            transaction_inserted,
            source,
        )
    }
    #[doc(hidden)]
    pub fn network_report_verified_vote_slashing_transaction_submission(
        &self,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<bool> {
        let _action = self.source.mutation()?;
        self.core
            .network_api
            .report_verified_vote_slashing_transaction_submission(proof_hash, transaction_inserted)
    }
    #[doc(hidden)]
    pub fn query_dag_live_status(&self) -> Result<DagRuntimeStatus> {
        let _action = self.source.read()?;
        self.query.dag_live_status()
    }
    #[doc(hidden)]
    pub fn query_dag_live_non_finalized_index(&self) -> Result<DagNonFinalizedIndex> {
        let _action = self.source.read()?;
        self.query.dag_live_non_finalized_index()
    }
    #[doc(hidden)]
    pub fn query_transaction_pool_status(&self) -> Result<TransactionPoolStatus> {
        let _action = self.source.read()?;
        self.query.transaction_pool_status()
    }
    #[doc(hidden)]
    pub fn query_pbft_sync_status(&self, now_ms: u64) -> Result<NetworkPbftSyncSnapshot> {
        let _action = self.source.read()?;
        self.query.pbft_sync_status(now_ms)
    }
    #[doc(hidden)]
    pub fn query_live_transaction_is_known(&self, hash: [u8; 32]) -> Result<bool> {
        let _action = self.source.read()?;
        self.query.live_transaction_is_known(hash)
    }
    #[doc(hidden)]
    pub fn query_verified_vote_count(&self) -> Result<u64> {
        let _action = self.source.read()?;
        self.query.verified_vote_count()
    }
    #[doc(hidden)]
    pub fn query_pbft_vote_threshold(
        &self,
        period: u64,
        vote_type: PbftVoteType,
    ) -> Result<crate::pbft_thresholds::PbftTwoTPlusOneThresholdPlan> {
        let _action = self.source.read()?;
        self.query.pbft_vote_threshold(period, vote_type)
    }
    #[doc(hidden)]
    pub fn query_pbft_progress(&self) -> Result<PbftProgressView> {
        let _action = self.source.read()?;
        self.query.pbft_progress()
    }
    #[doc(hidden)]
    pub fn query_pbft_sync_block_exists(&self, block_hash: [u8; 32]) -> Result<bool> {
        let _action = self.source.read()?;
        self.query.pbft_sync_block_exists(block_hash)
    }
    #[doc(hidden)]
    pub fn query_pbft_block_hash_by_period(&self, period: u64) -> Result<QueryHashLookup> {
        let _action = self.source.read()?;
        self.query.pbft_block_hash_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_pbft_schedule_block_by_period(
        &self,
        period: u64,
    ) -> Result<PbftScheduleBlockView> {
        let _action = self.source.read()?;
        self.query.pbft_schedule_block_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_pbft_node_version_by_period(&self, period: u64) -> Result<PbftNodeVersionView> {
        let _action = self.source.read()?;
        self.query.pbft_node_version_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_pbft_previous_block_cert_votes_by_period(
        &self,
        period: u64,
    ) -> Result<PbftPeriodCertVotesView> {
        let _action = self.source.read()?;
        self.query.pbft_previous_block_cert_votes_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_final_chain_block_by_number(&self, number: u64) -> Result<FinalChainBlockView> {
        let _action = self.source.read()?;
        self.query.final_chain_block_by_number(number)
    }
    #[doc(hidden)]
    pub fn query_final_chain_block_number_by_hash(
        &self,
        block_hash: [u8; 32],
    ) -> Result<QueryNumberLookup> {
        let _action = self.source.read()?;
        self.query.final_chain_block_number_by_hash(block_hash)
    }
    #[doc(hidden)]
    pub fn query_final_chain_last_block_number(&self) -> Result<u64> {
        let _action = self.source.read()?;
        self.query.final_chain_last_block_number()
    }
    #[doc(hidden)]
    pub fn query_final_chain_dpos_eligible_vote_count(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<u64> {
        let _action = self.source.mutation()?;
        self.query
            .final_chain_dpos_eligible_vote_count(block_number, address)
    }
    #[doc(hidden)]
    pub fn query_final_chain_dpos_eligible_total_vote_count(
        &self,
        block_number: u64,
    ) -> Result<u64> {
        let _action = self.source.mutation()?;
        self.query
            .final_chain_dpos_eligible_total_vote_count(block_number)
    }
    #[doc(hidden)]
    pub fn query_final_chain_dpos_validators_total_stakes(
        &self,
        block_number: u64,
    ) -> Result<Vec<DposValidatorStake>> {
        let _action = self.source.mutation()?;
        self.query
            .final_chain_dpos_validators_total_stakes(block_number)
    }
    #[doc(hidden)]
    pub fn query_final_chain_dpos_total_amount_delegated(
        &self,
        block_number: u64,
    ) -> Result<Vec<u8>> {
        let _action = self.source.mutation()?;
        self.query
            .final_chain_dpos_total_amount_delegated(block_number)
    }
    #[doc(hidden)]
    pub fn query_final_chain_dpos_yield(&self, block_number: u64) -> Result<u64> {
        let _action = self.source.mutation()?;
        self.query.final_chain_dpos_yield(block_number)
    }
    #[doc(hidden)]
    pub fn query_final_chain_dpos_total_supply(&self, block_number: u64) -> Result<Vec<u8>> {
        let _action = self.source.mutation()?;
        self.query.final_chain_dpos_total_supply(block_number)
    }
    #[doc(hidden)]
    pub fn query_period_lambda_by_period(&self, period: u64) -> Result<QueryPeriodLambda> {
        let _action = self.source.read()?;
        self.query.period_lambda_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_proposal_period_for_dag_level(&self, level: u64) -> Result<QueryNumberLookup> {
        let _action = self.source.read()?;
        self.query.proposal_period_for_dag_level(level)
    }
    #[doc(hidden)]
    pub fn query_chain_stats(&self) -> Result<ChainStatsView> {
        let _action = self.source.read()?;
        self.query.chain_stats()
    }
    #[doc(hidden)]
    pub fn query_consensus_status(&self) -> Result<ConsensusStatusView> {
        let _action = self.source.read()?;
        self.query.consensus_status()
    }
    #[doc(hidden)]
    pub fn query_sortition_params_change_by_period(
        &self,
        period: u64,
    ) -> Result<SortitionParamsChangeView> {
        let _action = self.source.read()?;
        self.query.sortition_params_change_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_final_chain_blocks_with_bloom(
        &self,
        bloom: [u8; 256],
        from: u64,
        to: u64,
    ) -> Result<Vec<u64>> {
        let _action = self.source.mutation()?;
        self.query.final_chain_blocks_with_bloom(bloom, from, to)
    }
    #[doc(hidden)]
    pub fn query_pillar_block_data_by_period(&self, period: u64) -> Result<PillarBlockDataView> {
        let _action = self.source.read()?;
        self.query.pillar_block_data_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_dag_block_by_hash(&self, hash: [u8; 32]) -> Result<DagBlockView> {
        let _action = self.source.read()?;
        self.query.dag_block_by_hash(hash)
    }
    #[doc(hidden)]
    pub fn query_dag_blocks_by_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<DagBlockView>> {
        let _action = self.source.read()?;
        self.query.dag_blocks_by_level(level, number_of_levels)
    }
    #[doc(hidden)]
    pub fn query_finalized_dag_blocks_by_period(&self, period: u64) -> Result<Vec<DagBlockView>> {
        let _action = self.source.read()?;
        self.query.finalized_dag_blocks_by_period(period)
    }
    #[doc(hidden)]
    pub fn query_transaction_by_hash(&self, hash: [u8; 32]) -> Result<TransactionView> {
        let _action = self.source.read()?;
        self.query.transaction_by_hash(hash)
    }
    #[doc(hidden)]
    pub fn query_transaction_by_block_number_and_index(
        &self,
        block_number: u64,
        transaction_index: u64,
    ) -> Result<TransactionView> {
        let _action = self.source.read()?;
        self.query
            .transaction_by_block_number_and_index(block_number, transaction_index)
    }
    #[doc(hidden)]
    pub fn query_transaction_by_block_hash_and_index(
        &self,
        block_hash: [u8; 32],
        transaction_index: u64,
    ) -> Result<TransactionView> {
        let _action = self.source.read()?;
        self.query
            .transaction_by_block_hash_and_index(block_hash, transaction_index)
    }
    #[doc(hidden)]
    pub fn query_transaction_count_by_block_number(&self, block_number: u64) -> Result<u64> {
        let _action = self.source.read()?;
        self.query.transaction_count_by_block_number(block_number)
    }
    #[doc(hidden)]
    pub fn query_transaction_count_by_block_hash(&self, block_hash: [u8; 32]) -> Result<u64> {
        let _action = self.source.read()?;
        self.query.transaction_count_by_block_hash(block_hash)
    }
    #[doc(hidden)]
    pub fn query_transaction_receipt_by_hash(
        &self,
        hash: [u8; 32],
    ) -> Result<TransactionReceiptView> {
        let _action = self.source.read()?;
        self.query.transaction_receipt_by_hash(hash)
    }
    #[doc(hidden)]
    pub fn query_transaction_receipts_by_block_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<TransactionReceiptView>> {
        let _action = self.source.read()?;
        self.query
            .transaction_receipts_by_block_number(block_number)
    }
}
