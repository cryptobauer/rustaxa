//! Native PBFT application-service composition and lifecycle ownership.
//!
//! [`PbftService`] restores and publishes the complete PBFT sibling graph from
//! one shared storage handle. Each sibling retains its own synchronization
//! domain; this root owns composition and bootstrap readiness only and never
//! adds a root-wide lock.

use crate::pbft_chain::PbftChainService;
use crate::pbft_manager::{
    PbftFinalizationExecutorBoundary, PbftFinalizationExecutorStartRequest,
    PbftFinalizationOwnedActionDrain, PbftManagerGuard, PbftManagerService,
    PbftManagerStorageStartupFact, base_owned_finalization_live_report,
    create_pbft_manager_runtime_from_storage,
};
use crate::pbft_period_cleanup::{PbftPeriodStateCleanupResult, cleanup_period_state_with_commit};
use crate::pbft_readiness::PbftServiceReadiness;
use crate::pbft_vote_runtime::PbftVerifiedVotesService;
use crate::pillar_chain_service::PillarChainService;
use crate::proposed_blocks::ProposedBlocksService;
use crate::slashing::SlashingProofService;
use anyhow::{Context, Result};
use rustaxa_storage::Storage;
use std::sync::Arc;

const SLASHING_PROOF_CACHE_MAX_SIZE: usize = 1000;
const SLASHING_PROOF_CACHE_DELETE_STEP: usize = 100;

/// Validated immutable configuration for native PBFT service restoration.
///
/// Millisecond values constrained by the legacy manager runtime are already
/// narrowed to `u32` by the external adapter. Construction derives the current
/// period and Cacti activation from the restored PBFT-chain head; callers
/// cannot inject either fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PbftServiceConfig {
    pub genesis_lambda_ms: u32,
    pub cacti_lambda_max_ms: u32,
    pub cacti_lambda_default_ms: u32,
    pub cacti_block: u64,
    pub max_exponential_lambda_ms: u64,
    pub max_steps: u64,
    pub deadline_ms: u64,
    pub polling_interval_ms: u64,
    pub report_malicious_behaviour: bool,
    pub magnolia_activation_period: u64,
}

/// CXX-free native owner of the complete PBFT application-service graph.
///
/// Restoration validates storage-independent slashing configuration first,
/// constructs every storage-backed sibling from the same `Arc<Storage>`, and
/// returns only after all siblings have succeeded, preventing publication of a
/// partially initialized root. The chain is restored before manager startup
/// facts are derived. Bootstrap readiness starts pending and is published
/// monotonically through [`Self::complete_bootstrap`].
pub struct PbftService {
    manager: PbftManagerService,
    chain: PbftChainService,
    proposed_blocks: ProposedBlocksService,
    verified_votes: PbftVerifiedVotesService,
    slashing: SlashingProofService,
    readiness: PbftServiceReadiness,
    pillar: PillarChainService,
}

impl PbftService {
    fn run_finalization_executor_task(
        &self,
        task: impl FnOnce(&mut PbftManagerGuard<'_>) -> Result<PbftFinalizationOwnedActionDrain>,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        let mut manager = self.manager_state();
        let result = task(&mut manager);
        let drain = manager.finish_finalization_executor(result)?;
        Ok(PbftFinalizationExecutorBoundary {
            next_step: drain.next_step,
            cleared_anchor_dag_cache: drain.cleared_anchor_dag_cache,
            has_snapshot: drain.has_snapshot,
            snapshot: manager.state.snapshot(),
            error_code: drain.error_code,
        })
    }

    /// Restores the coherent native PBFT service graph from shared storage.
    ///
    /// Errors preserve construction order: slashing configuration is checked
    /// first, followed by chain, verified votes, proposed blocks, manager
    /// runtime, and pillar restoration. No service root escapes on failure.
    pub fn restore(storage: Arc<Storage>, config: PbftServiceConfig) -> Result<Self> {
        let slashing = SlashingProofService::new(
            config.report_malicious_behaviour,
            config.magnolia_activation_period,
            SLASHING_PROOF_CACHE_MAX_SIZE,
            SLASHING_PROOF_CACHE_DELETE_STEP,
        )?;
        let chain = PbftChainService::restore(storage.clone())?;
        let verified_votes = PbftVerifiedVotesService::restore(storage.clone())?;
        let proposed_blocks = ProposedBlocksService::restore(storage.clone())?;
        let chain_head = chain.head();
        let runtime = create_pbft_manager_runtime_from_storage(
            &storage,
            PbftManagerStorageStartupFact {
                current_period: chain_head.size.saturating_add(1),
                cacti_active_at_chain_size: chain_head.size >= config.cacti_block,
                genesis_lambda_ms: config.genesis_lambda_ms,
                cacti_lambda_max_ms: config.cacti_lambda_max_ms,
                cacti_lambda_default_ms: config.cacti_lambda_default_ms,
                cacti_block: config.cacti_block,
                max_exponential_lambda_ms: config.max_exponential_lambda_ms,
                max_steps: config.max_steps,
                deadline_ms: config.deadline_ms,
                polling_interval_ms: config.polling_interval_ms,
            },
        )?;
        let pillar = PillarChainService::restore(storage.clone())?;

        Ok(Self {
            manager: PbftManagerService::new(runtime, storage, chain.clone()),
            chain,
            proposed_blocks,
            verified_votes,
            slashing,
            readiness: PbftServiceReadiness::pending(),
            pillar,
        })
    }

    /// Publishes completion of PBFT startup replay.
    pub fn complete_bootstrap(&self) {
        self.readiness.mark_ready();
    }

    /// Atomically cleans service-owned period state after PBFT finalization.
    ///
    /// `finalized_chain_size` must be nonzero and `new_period` must be its exact
    /// checked successor. The operation acquires verified votes before proposed
    /// blocks, plans both cleanups, commits all durable proposed-block deletes
    /// in one Rust storage batch, and only then publishes exact in-memory
    /// removals. Valid no-op transitions are published with zero counts.
    /// Validation or storage failures return a typed rejected result without
    /// memory publication; lock poison remains an operational error.
    pub fn cleanup_period_state(
        &self,
        finalized_chain_size: u64,
        new_period: u64,
    ) -> Result<PbftPeriodStateCleanupResult> {
        cleanup_period_state_with_commit(
            self.verified_votes(),
            self.proposed_blocks(),
            finalized_chain_size,
            new_period,
            |storage, batch| {
                storage
                    .commit_write_batch_with_sync(batch, false)
                    .context("PBFT_PERIOD_STATE_CLEANUP_COMMIT")
            },
        )
    }

    /// Starts or resumes one PBFT finalization executor under native ownership.
    ///
    /// The application root acquires the manager serialization domain, invokes
    /// the complete lock-held start/resume task against the supplied native
    /// DAG/transaction sibling, and captures the compatibility snapshot before
    /// releasing the manager lock. The first external action is returned as a
    /// typed boundary; no C++ callback occurs while native locks are held.
    ///
    /// Operational errors and terminal outcomes clear retained executor state
    /// inside the manager task. Active boundaries retain their plan, cursor,
    /// authenticated reward-reset generation, and prepared sortition request.
    pub fn start_finalization_executor(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        request: PbftFinalizationExecutorStartRequest,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        let mut manager = self.manager_state();
        let drain = manager.start_finalization_executor(dag_transaction_service, request)?;
        Ok(PbftFinalizationExecutorBoundary {
            next_step: drain.next_step,
            cleared_anchor_dag_cache: drain.cleared_anchor_dag_cache,
            has_snapshot: drain.has_snapshot,
            snapshot: manager.state.snapshot(),
            error_code: drain.error_code,
        })
    }

    /// Reports failure of the current external finalization leaf.
    ///
    /// The manager validates the echoed cursor, records the supplied external
    /// status and error, clears the terminal session, and captures the coherent
    /// compatibility snapshot before releasing its lock.
    pub fn fail_finalization_external_effect(
        &self,
        cursor: u32,
        status: u8,
        error_code: String,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step = manager.fail_finalization_external_effect(cursor, status, error_code)?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Advances the finalized DAG-order external leaf.
    ///
    /// Rust derives the expected action and accepted write intent, validates the
    /// finalized count, drains subsequent manager-owned actions, and returns the
    /// next external boundary under one manager lock.
    pub fn advance_finalization_dag_order(
        &self,
        cursor: u32,
        finalized_count: u64,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.dag_finalized_count = finalized_count;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Commits native sortition state and advances finalization.
    ///
    /// Manager-before-sortition lock order is retained. The prepared request is
    /// consumed exactly once, validated against live sortition facts, followed
    /// by native owned-action draining and terminal cleanup.
    pub fn advance_finalization_sortition_commit(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_sortition_commit(dag_transaction_service, cursor)?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Commits the native reward-vote cursor and advances finalization.
    ///
    /// The PBFT root composes its manager and verified-vote siblings in fixed
    /// order, validates reset provenance, drains manager-owned actions, and
    /// returns only the next external boundary.
    pub fn advance_finalization_reward_votes_reset(
        &self,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_reward_votes_reset(self.verified_votes(), cursor)?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Applies finalized transaction status and advances finalization.
    ///
    /// The PBFT root composes manager-before-DAG/transaction ownership while
    /// C++ supplies only the retained external-EVM account nonce facts.
    pub fn advance_finalization_transaction_status(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step = manager.advance_finalization_transaction_status(
                dag_transaction_service,
                cursor,
                retention_window,
                account_nonce_facts,
            )?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Reports successful FinalChain/EVM dispatch and advances finalization.
    ///
    /// Only the observed FinalChain height crosses this boundary. Rust derives
    /// blocks-per-year and every manager-owned identity from the retained plan.
    pub fn advance_finalization_final_chain_dispatch(
        &self,
        cursor: u32,
        last_block: u64,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.final_chain_dispatched = true;
                    report.final_chain_blocks_per_year = write_set.blocks_per_year;
                    report.final_chain_last_block = last_block;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Reports pillar post-processing facts and advances finalization.
    ///
    /// The manager period is sampled under the same serialization lock as
    /// cursor validation. Rust derives the processed period from its retained
    /// plan; callers supply only the request period observed at the pillar leaf.
    pub fn advance_finalization_pillar_post_processing(
        &self,
        cursor: u32,
        request_period: u64,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let manager_period = manager.state.snapshot().period;
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.manager_period = manager_period;
                    report.pillar_processed_period = write_set.block_period;
                    report.pillar_request_period = request_period;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Reports the native period-cleanup result and advances finalization.
    ///
    /// The resulting manager period is read from the lock-held native snapshot;
    /// action identity, cursor validation, owned draining, cleanup, and
    /// boundary capture remain native.
    pub fn advance_finalization_advance_period(
        &self,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let manager_period = manager.state.snapshot().period;
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.manager_period = manager_period;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Returns whether PBFT startup replay has been published complete.
    pub fn is_ready(&self) -> bool {
        self.readiness.is_ready()
    }

    /// Locks the native manager serialization domain.
    pub fn manager_state(&self) -> PbftManagerGuard<'_> {
        self.manager.lock()
    }

    /// Returns the native PBFT-chain sibling.
    pub fn chain(&self) -> &PbftChainService {
        &self.chain
    }

    /// Returns the native proposed-block sibling.
    pub fn proposed_blocks(&self) -> &ProposedBlocksService {
        &self.proposed_blocks
    }

    /// Returns the native verified-vote sibling.
    pub fn verified_votes(&self) -> &PbftVerifiedVotesService {
        &self.verified_votes
    }

    /// Returns the native slashing sibling.
    pub fn slashing(&self) -> &SlashingProofService {
        &self.slashing
    }

    /// Returns the native readiness capability.
    pub fn readiness(&self) -> &PbftServiceReadiness {
        &self.readiness
    }

    /// Returns the native pillar sibling.
    pub fn pillar(&self) -> &PillarChainService {
        &self.pillar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, H256};
    use rustaxa_storage::Config;
    use rustaxa_types::pillar::{CurrentPillarBlockDataDb, PillarBlock, ValidatorVoteCount};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_storage(name: &str) -> (PathBuf, Arc<Storage>) {
        let path = std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).expect("storage opens"));
        (path, storage)
    }

    fn config(cacti_block: u64) -> PbftServiceConfig {
        PbftServiceConfig {
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            cacti_block,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
            report_malicious_behaviour: true,
            magnolia_activation_period: 0,
        }
    }

    fn install_finalization_executor(
        service: &PbftService,
        block_period: u64,
        cleanup: crate::pbft_finalize::PbftFinalizationCleanupIntent,
        actions: Vec<crate::pbft_finalize::PbftFinalizationRuntimeAction>,
    ) {
        use crate::pbft_finalize::{
            PbftFinalizationAnchor, PbftFinalizationPlan, PbftFinalizationRuntimePlan,
            PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
            start_pbft_finalization_runtime,
        };

        let plan = PbftFinalizationPlan {
            finalize_block: true,
            anchor: PbftFinalizationAnchor::Anchored,
            executed_pbft_block: false,
            cleanup,
            storage_write_intent: PbftFinalizationStorageWriteIntent {
                persist_pbft_head: false,
                persist_period_data: false,
                reset_reward_votes: false,
                update_sortition_params: false,
                apply_dynamic_lambda_update: false,
                persist_period_lambda: false,
                persist_executed_pbft_status: false,
                process_pillar_block: false,
                pbft_block_hash: H256::repeat_byte(7),
                pbft_head_hash: H256::repeat_byte(8),
                block_period,
                null_anchor: false,
                anchor_hash: H256::repeat_byte(4),
                reward_vote_period: block_period,
                reward_vote_round: 2,
                reward_vote_step: 3,
                reward_vote_block_hash: H256::repeat_byte(7),
                period_lambda: 0,
                blocks_per_year: 777,
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 0,
                executed_pbft_status: false,
                pbft_head_payload: Vec::new(),
                period_data_rlp: Vec::new(),
                dag_block_period_writes: Vec::new(),
                transaction_location_writes: Vec::new(),
            },
            status: PbftFinalizationStatus::Accepted,
        };
        let runtime_plan = PbftFinalizationRuntimePlan {
            finalize_block: true,
            status: PbftFinalizationStatus::Accepted,
            actions,
        };
        let mut manager = service.manager_state();
        manager.finalization_runtime_session = Some(start_pbft_finalization_runtime(&runtime_plan));
        manager.finalization_runtime_plan = Some(plan);
    }

    #[test]
    fn restore_derives_period_and_cacti_activation_from_chain() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_derivation");

        let active = PbftService::restore(storage.clone(), config(0)).unwrap();
        let active_snapshot = active.manager_state().state.snapshot();
        assert_eq!(active_snapshot.period, 1);
        assert_eq!(active_snapshot.current_round_lambda_ms, 1_500);

        let inactive = PbftService::restore(storage, config(1)).unwrap();
        let inactive_snapshot = inactive.manager_state().state.snapshot();
        assert_eq!(inactive_snapshot.period, 1);
        assert_eq!(inactive_snapshot.current_round_lambda_ms, 100);

        drop(active);
        drop(inactive);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn bootstrap_readiness_is_pending_then_monotonic() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_readiness");
        let service = PbftService::restore(storage, config(1)).unwrap();

        assert!(!service.is_ready());
        service.complete_bootstrap();
        assert!(service.is_ready());
        service.complete_bootstrap();
        assert!(service.is_ready());

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn manager_and_public_chain_share_one_native_owner() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_shared_chain");
        let service = PbftService::restore(storage, config(1)).unwrap();

        service
            .chain()
            .update(
                ethereum_types::H256::from([7; 32]),
                ethereum_types::H256::from([4; 32]),
            )
            .unwrap();
        let public_head = service.chain().head();
        let manager_head = service
            .manager_state()
            .chain
            .read()
            .expect("PBFT chain lock should remain healthy")
            .state
            .head();
        assert_eq!(public_head, manager_head);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn pillar_state_restarts_through_the_same_native_root() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_pillar_restart");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        service.pillar().complete_bootstrap().unwrap();
        let data = CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period: 1,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::zero(),
                bridge_root: H256::from_low_u64_be(2),
                epoch: 3,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: vec![ValidatorVoteCount {
                address: H160::from_low_u64_be(4),
                vote_count: 5,
            }],
        }
        .encode_rlp();
        let generation = service.pillar().sample_anchor_generation().unwrap();
        service
            .pillar()
            .apply_planned_current_block_data(data.clone(), generation)
            .unwrap();
        drop(service);

        let restarted = PbftService::restore(storage, config(1)).unwrap();
        assert!(!restarted.pillar().is_ready());
        assert_eq!(
            restarted
                .pillar()
                .load_startup_bootstrap()
                .unwrap()
                .current_block_data_rlp,
            data
        );

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn invalid_configuration_fails_before_root_publication() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_failure");
        let mut invalid = config(1);
        invalid.genesis_lambda_ms = 0;

        let error = PbftService::restore(storage, invalid)
            .err()
            .expect("invalid immutable configuration must reject construction");
        assert!(
            error
                .to_string()
                .contains("PBFT_MANAGER_STARTUP_INVALID_LAMBDA_CONFIG")
        );

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_advancement_error_clears_application_root_state() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_error_cleanup");
        let service = PbftService::restore(storage, config(1)).unwrap();
        install_finalization_executor(
            &service,
            1,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: true,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime],
        );

        let error = service
            .advance_finalization_reward_votes_reset(0)
            .expect_err("missing reset generation must reject cursor publication");
        assert!(
            error
                .to_string()
                .contains("PBFT_FINALIZE_POST_STORAGE_REWARD_VOTES_INVARIANT")
        );
        let manager = service.manager_state();
        assert!(manager.finalization_runtime_session.is_none());
        assert!(manager.finalization_runtime_plan.is_none());
        assert!(manager.finalization_sortition_commit_request.is_none());
        assert_eq!(manager.finalization_reward_votes_reset_generation, 0);
        drop(manager);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_period_and_pillar_advancement_share_native_boundary() {
        use crate::pbft_finalize::{
            PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeStatus,
        };

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_period_pillar");
        let service = PbftService::restore(storage, config(1)).unwrap();
        service.manager_state().state.set_period_for_test(3);
        install_finalization_executor(
            &service,
            2,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: true,
                process_pillar_block: true,
            },
            vec![
                PbftFinalizationRuntimeAction::AdvancePeriod,
                PbftFinalizationRuntimeAction::ProcessPillarBlock,
            ],
        );

        let period = service
            .advance_finalization_advance_period(0)
            .expect("period advancement reaches pillar leaf");
        assert_eq!(
            period.next_step.action,
            Some(PbftFinalizationRuntimeAction::ProcessPillarBlock)
        );
        assert_eq!(period.snapshot.period, 3);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_some()
        );

        let pillar = service
            .advance_finalization_pillar_post_processing(1, 1)
            .expect("pillar acknowledgement completes finalization");
        assert_eq!(
            pillar.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );
        assert!(pillar.next_step.complete);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_none()
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn final_chain_advancement_derives_retained_blocks_per_year() {
        use crate::pbft_finalize::{
            PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeStatus,
        };

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_final_chain");
        let service = PbftService::restore(storage, config(1)).unwrap();
        install_finalization_executor(
            &service,
            2,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::FinalizeFinalChain],
        );

        let boundary = service
            .advance_finalization_final_chain_dispatch(0, 2)
            .expect("retained blocks-per-year validates FinalChain dispatch");
        assert_eq!(
            boundary.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );
        assert!(boundary.next_step.complete);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_none()
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }
}
