//! Native PBFT application-service composition and lifecycle ownership.
//!
//! [`PbftService`] restores and publishes the complete PBFT sibling graph from
//! one shared storage handle. Each sibling retains its own synchronization
//! domain; this root owns composition and bootstrap readiness only and never
//! adds a root-wide lock.

use crate::pbft_chain::PbftChainService;
use crate::pbft_finalize::{
    PbftDynamicLambdaFact, PbftDynamicLambdaPlan, PbftFinalizationPeriodLambdaLookup,
    PbftFinalizationRuntimeAction, PbftFinalizationStatus,
    load_pbft_finalization_last_period_lambda, plan_pbft_dynamic_lambda,
};
use crate::pbft_manager::{
    PbftFinalizationExecutorBoundary, PbftFinalizationExecutorStartRequest,
    PbftFinalizationOwnedActionDrain, PbftManagerGuard, PbftManagerService,
    PbftManagerStorageStartupFact, base_owned_finalization_live_report,
    create_pbft_manager_runtime_from_storage,
};
#[cfg(test)]
use crate::pbft_period_cleanup::{PbftPeriodStateCleanupResult, cleanup_period_state_with_commit};
use crate::pbft_period_cleanup::{
    PbftPeriodStateCleanupStatus, cleanup_period_state_with_commit_and_publish,
};
use crate::pbft_readiness::PbftServiceReadiness;
use crate::pbft_vote_runtime::PbftVerifiedVotesService;
use crate::pillar_chain_service::PillarChainService;
use crate::proposed_blocks::ProposedBlocksService;
use crate::slashing::SlashingProofService;
use anyhow::{Context, Result};
use rustaxa_storage::{Storage, StorageWriteBatch};
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

/// Native dynamic-lambda decision composed with its durable prior-lambda fact.
///
/// `plan` contains the deterministic finalization policy decision. The lookup
/// is populated only for an accepted, active dynamic-lambda plan; inactive or
/// rejected plans carry `found = false` and value zero without reading storage.
pub struct PbftFinalizationDynamicLambdaDecision {
    /// Deterministic dynamic-lambda policy result.
    pub plan: PbftDynamicLambdaPlan,
    /// Closest persisted lambda at or before the preceding finalized period.
    pub last_saved_period_lambda: PbftFinalizationPeriodLambdaLookup,
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
            expired_dag_hashes: drain.expired_dag_hashes,
            refresh_dag_counters: drain.refresh_dag_counters,
            snapshot: manager.state.snapshot(),
            error_code: drain.error_code,
        })
    }

    /// Plans one finalization dynamic-lambda update with native storage facts.
    ///
    /// The service derives the prior-period lookup from its manager-owned
    /// storage handle. Active accepted plans query the closest persisted lambda
    /// at or before `finalized_period - 1`. Period zero has no predecessor and
    /// returns an empty lookup without reading a period-zero row. Disabled or
    /// rejected plans likewise return an empty lookup. Storage failures are
    /// returned without mutating manager or durable state.
    pub fn plan_finalization_dynamic_lambda(
        &self,
        fact: PbftDynamicLambdaFact,
    ) -> Result<PbftFinalizationDynamicLambdaDecision> {
        let dynamic_lambda_active = fact.dynamic_lambda_active;
        let finalized_period = fact.finalized_period;
        let plan = plan_pbft_dynamic_lambda(fact);
        let last_saved_period_lambda = if dynamic_lambda_active
            && plan.status == PbftFinalizationStatus::Accepted
            && finalized_period > 0
        {
            let manager = self.manager_state();
            load_pbft_finalization_last_period_lambda(
                manager.storage.as_ref(),
                finalized_period.saturating_sub(1),
            )?
        } else {
            PbftFinalizationPeriodLambdaLookup {
                found: false,
                value: 0,
            }
        };
        Ok(PbftFinalizationDynamicLambdaDecision {
            plan,
            last_saved_period_lambda,
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
    #[cfg(test)]
    pub(crate) fn cleanup_period_state(
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

    /// Commits one externally executed PBFT period advance under native ownership.
    ///
    /// The manager reset provenance is validated before cleanup. Rust then
    /// acquires verified-vote and proposed-block siblings in the canonical
    /// manager-first order, commits durable cleanup before live cleanup
    /// publication, and publishes the new manager period only after cleanup
    /// succeeds. Invalid or duplicate period reports return the unchanged
    /// rejected manager snapshot; storage and cleanup failures return an error
    /// while preserving reset provenance so the operation can be retried.
    pub fn apply_period_advance(
        &self,
        new_period: u64,
    ) -> Result<crate::pbft_manager::PbftManagerRuntimeSnapshot> {
        self.apply_period_advance_with_commit(new_period, |storage, batch| {
            storage
                .commit_write_batch_with_sync(batch, false)
                .context("PBFT_PERIOD_ADVANCE_CLEANUP_COMMIT")
        })
    }

    /// Applies one period advance with an injected durable cleanup commit.
    ///
    /// This is the single native implementation behind the production commit
    /// boundary. The injected operation is used by tests to prove that a
    /// durable-write failure leaves manager reset provenance and both cleanup
    /// siblings unchanged, allowing the same transition to be retried.
    pub(crate) fn apply_period_advance_with_commit<F>(
        &self,
        new_period: u64,
        commit: F,
    ) -> Result<crate::pbft_manager::PbftManagerRuntimeSnapshot>
    where
        F: FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    {
        let Some(finalized_chain_size) = new_period.checked_sub(1) else {
            return Ok(self
                .manager
                .lock()
                .state
                .apply_committed_period_advance(new_period));
        };
        let mut manager = self.manager.lock();
        let plan = manager
            .state
            .plan_advance_period_after_reset(finalized_chain_size);
        if !plan.accepted || plan.new_period != new_period {
            return Ok(manager.state.apply_committed_period_advance(new_period));
        }

        let mut snapshot = None;
        let cleanup = cleanup_period_state_with_commit_and_publish(
            self.verified_votes(),
            self.proposed_blocks(),
            finalized_chain_size,
            new_period,
            commit,
            || {
                snapshot = Some(manager.state.apply_committed_period_advance(new_period));
            },
        )?;
        if cleanup.status == PbftPeriodStateCleanupStatus::Rejected || !cleanup.transition_published
        {
            return Err(anyhow::Error::msg(cleanup.error_code));
        }

        snapshot.context("PBFT_PERIOD_ADVANCE_PUBLICATION_MISSING")
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
        let drain = manager.start_finalization_executor(
            dag_transaction_service,
            self.verified_votes(),
            request,
        )?;
        Ok(PbftFinalizationExecutorBoundary {
            next_step: drain.next_step,
            cleared_anchor_dag_cache: drain.cleared_anchor_dag_cache,
            has_snapshot: drain.has_snapshot,
            expired_dag_hashes: drain.expired_dag_hashes,
            refresh_dag_counters: drain.refresh_dag_counters,
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
    /// Rust derives the expected action and accepted write intent, performs
    /// native finalized-order mutation, drains subsequent manager-owned actions,
    /// and returns the next external boundary under one manager lock.
    pub fn advance_finalization_dag_order(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let (step, expired_dag_hashes, refresh_dag_counters) =
                manager.advance_finalization_set_dag_order(dag_transaction_service, cursor)?;
            let mut drain = manager.continue_finalization_executor_from_step(step)?;
            drain.expired_dag_hashes = expired_dag_hashes;
            drain.refresh_dag_counters = refresh_dag_counters;
            Ok(drain)
        })
    }

    /// Advances a specific external finalization action reported by the boundary.
    ///
    /// The action is decoded from the canonical action code and mapped to the
    /// corresponding Rust-owned external-effect leaf implementation. Leaf-specific
    /// payloads are consumed only for matching actions.
    pub fn advance_finalization_action(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
        action: u8,
        last_block: u64,
        request_period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        let finalization_action = PbftFinalizationRuntimeAction::from_u8(action)
            .ok_or_else(|| anyhow::anyhow!("PBFT_FINALIZE_UNKNOWN_ACTION"))?;

        match finalization_action {
            PbftFinalizationRuntimeAction::CommitSortitionRuntime => {
                self.advance_finalization_sortition_commit(dag_transaction_service, cursor)
            }
            PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime => {
                self.advance_finalization_reward_votes_reset(cursor)
            }
            PbftFinalizationRuntimeAction::SetDagBlockOrder => {
                self.advance_finalization_dag_order(dag_transaction_service, cursor)
            }
            PbftFinalizationRuntimeAction::UpdateFinalizedTransactions => self
                .advance_finalization_transaction_status(
                    dag_transaction_service,
                    cursor,
                    retention_window,
                    account_nonce_facts,
                ),
            PbftFinalizationRuntimeAction::FinalizeFinalChain => {
                self.advance_finalization_final_chain_dispatch(cursor, last_block)
            }
            PbftFinalizationRuntimeAction::AdvancePeriod => {
                self.advance_finalization_advance_period(cursor)
            }
            PbftFinalizationRuntimeAction::ProcessPillarBlock => {
                self.advance_finalization_pillar_post_processing(cursor, request_period)
            }
            _ => Err(anyhow::anyhow!("PBFT_FINALIZE_UNSUPPORTED_ACTION")),
        }
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

    /// Starts a manager-owned synced-period admission cursor when bootstrap is ready.
    ///
    /// The immutable candidate facts move directly into the native manager
    /// owner. A pending bootstrap rejects the command without allocating or
    /// replacing a session.
    pub fn begin_pbft_sync_admission(
        &self,
        fact: crate::pbft_sync::PbftSyncAdmissionInitialFact,
    ) -> bool {
        if !self.is_ready() {
            return false;
        }
        self.manager.begin_pbft_sync_admission(fact);
        true
    }

    /// Returns the current native synced-period admission step.
    ///
    /// `None` denotes either an incomplete bootstrap or no active cursor.
    /// Terminal/error steps are returned once and consumed by the manager.
    pub fn pbft_sync_admission_next(
        &self,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.is_ready()
            .then(|| self.manager.pbft_sync_admission_next())
            .flatten()
    }

    /// Reports one non-transaction validation fact to the native admission cursor.
    ///
    /// Unknown or stale reports are converted by the native session into a
    /// terminal contract error and consume the cursor.
    pub fn report_pbft_sync_admission_status(
        &self,
        cursor: u32,
        check: crate::pbft_sync::PbftSyncProcessRuntimeNextCheck,
        final_chain_status: crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus,
        fact_status: crate::pbft_sync::PbftSyncFactStatus,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.manager.report_pbft_sync_admission_status(
            cursor,
            check,
            final_chain_status,
            fact_status,
        )
    }

    /// Reports the requested transaction lookup result to the native cursor.
    ///
    /// The manager owns cursor validation, state mutation, terminal cleanup,
    /// and the complete resulting admission plan.
    pub fn report_pbft_sync_admission_transactions(
        &self,
        cursor: u32,
        report: crate::pbft_sync::PbftSyncAdmissionTransactionReport,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.manager
            .report_pbft_sync_admission_transactions(cursor, report)
    }

    /// Aborts and consumes the current synced-period admission cursor.
    pub fn abort_pbft_sync_admission(
        &self,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.manager.abort_pbft_sync_admission()
    }

    /// Loads canonical PBFT sync egress bytes through manager-owned storage.
    ///
    /// Rust also decides whether the temporary reward-vote sidecar belongs on
    /// the last packet. C++ retains only packet wrapping and transport.
    pub fn load_pbft_sync_egress_payload(
        &self,
        fact: crate::pbft_sync::PbftSyncRewardVoteAttachmentFact,
    ) -> Result<crate::pbft_sync::PbftSyncEgressPayload> {
        self.manager.load_pbft_sync_egress_payload(fact)
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
    use crate::dag_service::DagServiceConfig;
    use crate::dag_transaction_service::{DagTransactionService, DagTransactionServiceConfig};
    use crate::gas_pricer::GasPricerConfig;
    use crate::pbft_vote_event::PbftVoteEventFactFlags;
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::pbft_vote_progress::PbftVoteProgressContext;
    use crate::pbft_vote_validation::{
        PbftVoteValidationExternalFacts, validate_canonical_pbft_vote,
    };
    use crate::sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams};
    use crate::transaction_service::TransactionServiceConfig;
    use crate::verified_votes::PbftVoteType;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rustaxa_storage::Config;
    use rustaxa_types::pillar::{CurrentPillarBlockDataDb, PillarBlock, ValidatorVoteCount};
    use rustaxa_vdf::vrf;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tiny_keccak::{Hasher, Keccak};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    const NODE_SECRET: [u8; 32] = [0x35; 32];
    const NODE_SECRET_TWO: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

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

    fn dynamic_lambda_fact(finalized_period: u64) -> PbftDynamicLambdaFact {
        PbftDynamicLambdaFact {
            dynamic_lambda_active: true,
            finalized_period,
            finalized_round: 1,
            pre_adjust_rounds_count_dynamic_lambda: 9,
            pre_adjust_dynamic_lambda: 1_500,
            config: crate::pbft_finalize::PbftDynamicLambdaConfig {
                cacti_block_num: 10,
                lambda_min: 500,
                lambda_max: 1_500,
                lambda_default: 2_000,
                lambda_change_interval: 10,
                lambda_change: 10,
                consensus_delay: 400,
                dpos_blocks_per_year: 500,
            },
        }
    }

    fn dag_service(storage: Arc<Storage>) -> DagTransactionService {
        DagTransactionService::restore(
            storage,
            DagTransactionServiceConfig {
                transaction: TransactionServiceConfig {
                    queue_max_size: 16,
                    gas_pricer_config: GasPricerConfig {
                        percentile: 50,
                        minimum_price: U256::one(),
                        history_blocks: 0,
                        is_light_node: false,
                        blocks_gas_pricer: false,
                    },
                    proposal_dag_gas_limit: 1_000_000,
                },
                dag: DagServiceConfig {
                    genesis_hash: H256::repeat_byte(1),
                    dag_expiry_limit: 32,
                    max_levels_per_period: 100,
                },
                sortition: SortitionConfig {
                    params: SortitionParams {
                        vrf: VrfParams {
                            threshold_upper: 0x100,
                        },
                        vdf: VdfParams {
                            difficulty_min: 1,
                            difficulty_max: 10,
                            difficulty_stale: 5,
                            lambda_bound: 100,
                        },
                    },
                    changes_count_for_average: 8,
                    dag_efficiency_targets: (5_000, 10_000),
                    changing_interval: 10,
                    computation_interval: 5,
                },
            },
        )
        .expect("DAG transaction service restores")
    }

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn cert_vote_rlp(block_hash: H256, secret: [u8; 32]) -> Vec<u8> {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash,
            vote_type: PbftVoteType::Cert,
            period: 12,
            round: 2,
            step: 3,
            node_secret: secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
        .vote_rlp
    }

    fn seed_reward_cert_votes(service: &PbftService, block_hash: H256) {
        for secret in [NODE_SECRET, NODE_SECRET_TWO] {
            let vote_rlp = cert_vote_rlp(block_hash, secret);
            let validation = validate_canonical_pbft_vote(
                &vote_rlp,
                PbftVoteValidationExternalFacts {
                    voter_dpos_ready: true,
                    voter_dpos_vote_count: 40,
                    total_dpos_ready: true,
                    total_dpos_vote_count: 100,
                    future_dpos_state: false,
                    unknown_error: false,
                    vrf_key_ready: true,
                    has_vrf_key: true,
                    vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
                    strict_vrf: true,
                    committee_size: 100,
                    number_of_proposers: 20,
                    has_preverified_weight: false,
                    preverified_weight: 0,
                },
            )
            .unwrap();
            service
                .verified_votes()
                .lock()
                .unwrap()
                .admit_validated_vote(
                    &vote_rlp,
                    &validation,
                    PbftVoteEventFactFlags {
                        vote_already_known: false,
                        carries_proposed_block: true,
                        valid_stale_reward_vote: false,
                    },
                    PbftVoteProgressContext {
                        current_period: 12,
                        current_round: 2,
                        max_future_period_delta: 0,
                        two_t_plus_one_threshold: Some(80),
                        require_proposed_block_sidecar: false,
                        slashing_enabled: true,
                    },
                )
                .unwrap();
        }
    }

    fn reward_finalization_start_request(block_hash: H256) -> PbftFinalizationExecutorStartRequest {
        use crate::pbft_finalize::{
            PbftFinalizationAnchor, PbftFinalizationCleanupIntent, PbftFinalizationPlan,
            PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
            PbftFinalizationStorageWriteStage,
        };
        use crate::pbft_manager::PbftFinalizationExecutorStartMode;

        PbftFinalizationExecutorStartRequest {
            plan: PbftFinalizationPlan {
                finalize_block: true,
                anchor: PbftFinalizationAnchor::Anchored,
                executed_pbft_block: false,
                cleanup: PbftFinalizationCleanupIntent {
                    persist_pbft_block_metadata: true,
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
                storage_write_intent: PbftFinalizationStorageWriteIntent {
                    persist_pbft_head: true,
                    persist_period_data: false,
                    reset_reward_votes: true,
                    update_sortition_params: false,
                    apply_dynamic_lambda_update: false,
                    persist_period_lambda: false,
                    persist_executed_pbft_status: false,
                    process_pillar_block: false,
                    pbft_block_hash: block_hash,
                    pbft_head_hash: block_hash,
                    block_period: 12,
                    null_anchor: false,
                    anchor_hash: H256::zero(),
                    reward_vote_period: 12,
                    reward_vote_round: 2,
                    reward_vote_step: 3,
                    reward_vote_block_hash: block_hash,
                    period_lambda: 0,
                    blocks_per_year: 0,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    executed_pbft_status: false,
                    pbft_head_payload: vec![0xde, 0xad, 0xbe, 0xef],
                    period_data_rlp: Vec::new(),
                    dag_block_period_writes: Vec::new(),
                    transaction_location_writes: Vec::new(),
                },
                status: PbftFinalizationStatus::Accepted,
            },
            mode: PbftFinalizationExecutorStartMode::Fresh {
                primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                sync: false,
            },
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
    fn dynamic_lambda_planning_and_storage_lookup_are_owned_by_native_service() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_dynamic_lambda");
        storage
            .metadata()
            .write_period_lambda(19, 1_234)
            .expect("period lambda persists");
        storage
            .metadata()
            .write_period_lambda(0, 999)
            .expect("period-zero lambda persists for lower-bound regression");
        let service = PbftService::restore(storage, config(1)).unwrap();

        let decision = service
            .plan_finalization_dynamic_lambda(dynamic_lambda_fact(20))
            .expect("dynamic-lambda decision succeeds");
        assert_eq!(decision.plan.status, PbftFinalizationStatus::Accepted);
        assert!(decision.plan.apply_dynamic_lambda_update);
        assert_eq!(decision.plan.period_lambda, 1_500);
        assert_eq!(decision.plan.blocks_per_year, 9_275_294);
        assert_eq!(decision.plan.rounds_count_dynamic_lambda, 0);
        assert_eq!(decision.plan.dynamic_lambda, 1_490);
        assert_eq!(
            decision.last_saved_period_lambda,
            PbftFinalizationPeriodLambdaLookup {
                found: true,
                value: 1_234,
            }
        );

        let missing = service
            .plan_finalization_dynamic_lambda(dynamic_lambda_fact(0))
            .expect("period zero has no prior lambda");
        assert_eq!(
            missing.last_saved_period_lambda,
            PbftFinalizationPeriodLambdaLookup {
                found: false,
                value: 0,
            }
        );

        let mut inactive_fact = dynamic_lambda_fact(20);
        inactive_fact.dynamic_lambda_active = false;
        let inactive = service
            .plan_finalization_dynamic_lambda(inactive_fact)
            .expect("inactive dynamic-lambda decision succeeds");
        assert!(!inactive.plan.apply_dynamic_lambda_update);
        assert!(!inactive.last_saved_period_lambda.found);

        let mut rejected_fact = dynamic_lambda_fact(20);
        rejected_fact.config.lambda_change_interval = 0;
        let rejected = service
            .plan_finalization_dynamic_lambda(rejected_fact)
            .expect("rejected policy does not read prior lambda");
        assert_eq!(rejected.plan.status, PbftFinalizationStatus::ContractError);
        assert!(!rejected.last_saved_period_lambda.found);

        drop(service);
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
    fn sync_admission_and_egress_are_owned_by_native_pbft_service() {
        use crate::pbft_sync::{
            PbftSyncAdmissionInitialFact, PbftSyncAdmissionTransactionReport, PbftSyncFactStatus,
            PbftSyncRewardVoteAttachmentFact, PbftSyncRuntimeFinalChainHashStatus,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_sync_owner");
        storage
            .period()
            .write(9, &[0xc8, 0xc0, 0xc1])
            .expect("period data persists");
        let service = PbftService::restore(storage, config(1)).unwrap();
        let initial = PbftSyncAdmissionInitialFact {
            block_period: 10,
            block_prev_hash: H256::repeat_byte(9),
            chain_last_hash: H256::repeat_byte(9),
            chain_last_period: 9,
            block_in_chain: false,
            dag_transaction_hashes: vec![H256::repeat_byte(1)],
            period_data_transaction_hashes: Vec::new(),
            extra_data_required: false,
            extra_data_present: false,
            extra_data_pillar_block_hash_present: false,
            pillar_votes_required: false,
            pillar_votes_present: false,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        };

        assert!(!service.begin_pbft_sync_admission(initial.clone()));
        assert!(service.pbft_sync_admission_next().is_none());
        service.complete_bootstrap();
        assert!(service.begin_pbft_sync_admission(initial.clone()));

        let final_chain = service.pbft_sync_admission_next().expect("session starts");
        let reward = service
            .report_pbft_sync_admission_status(
                final_chain.cursor,
                final_chain.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("FinalChain report advances");
        let cert = service
            .report_pbft_sync_admission_status(
                reward.cursor,
                reward.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("reward report advances");
        let transactions = service
            .report_pbft_sync_admission_status(
                cert.cursor,
                cert.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("cert report advances");
        let accepted = service
            .report_pbft_sync_admission_transactions(
                transactions.cursor,
                PbftSyncAdmissionTransactionReport {
                    missing_transaction_hashes: vec![H256::repeat_byte(1)],
                    finalized_transaction_hashes: vec![H256::repeat_byte(2)],
                    contains_finalized_transactions: true,
                },
            )
            .expect("transaction report completes");
        assert!(accepted.complete);
        assert!(accepted.plan.accept_period_data);
        assert_eq!(accepted.plan.warnings.len(), 2);
        assert!(service.pbft_sync_admission_next().is_none());

        assert!(service.begin_pbft_sync_admission(initial));
        let step = service
            .pbft_sync_admission_next()
            .expect("replacement starts");
        let mismatch = service
            .report_pbft_sync_admission_status(
                step.cursor + 1,
                step.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("mismatch returns terminal step");
        assert!(mismatch.complete);
        assert!(!mismatch.can_continue);
        assert!(service.pbft_sync_admission_next().is_none());

        let payload = service
            .load_pbft_sync_egress_payload(PbftSyncRewardVoteAttachmentFact {
                block_period: 9,
                last_block: true,
                pbft_chain_synced: true,
                reward_votes_present: true,
                reward_votes_period: 9,
            })
            .expect("egress payload loads");
        assert_eq!(payload.period_data_rlp, vec![0xc8, 0xc0, 0xc1]);
        assert!(payload.attach_reward_votes);

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
    fn fresh_finalization_prepares_and_publishes_reward_votes_through_native_root() {
        use crate::pbft_finalize::PbftFinalizationRuntimeAction;

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_reward_start");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        let block_hash = H256::repeat_byte(0x61);
        seed_reward_cert_votes(&service, block_hash);

        let boundary = service
            .start_finalization_executor(&dag, reward_finalization_start_request(block_hash))
            .expect("native reward stage prepares and persists");
        assert_eq!(
            boundary.next_step.action,
            Some(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime)
        );
        assert!(storage.extra_reward_votes_reset_generation() > 0);
        let durable = storage
            .pbft()
            .finalized_reward_vote_cursor()
            .unwrap()
            .expect("reward cursor persisted with primary storage");
        assert_eq!(durable.period, 12);
        assert_eq!(durable.round, 2);
        assert_eq!(durable.step, 3);
        assert_eq!(durable.block_hash, block_hash);
        assert!(!durable.votes_bundle_rlp.is_empty());

        let completed = service
            .advance_finalization_reward_votes_reset(boundary.next_step.action_index)
            .expect("native reward cursor publishes");
        assert!(completed.next_step.complete);
        let snapshot = service
            .verified_votes()
            .reward_vote_cursor_snapshot()
            .unwrap();
        assert!(snapshot.found);
        assert_eq!(snapshot.period, 12);
        assert_eq!(snapshot.round, 2);
        assert_eq!(snapshot.step, 3);
        assert_eq!(snapshot.block_hash, block_hash);

        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn fresh_finalization_reward_identity_failure_clears_stale_manager_state() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_reward_start_reject");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        seed_reward_cert_votes(&service, H256::repeat_byte(0x61));
        install_finalization_executor(
            &service,
            11,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: true,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::UpdatePbftChain],
        );
        {
            let mut manager = service.manager_state();
            manager.finalization_reward_votes_reset_generation = 99;
        }

        let error = service
            .start_finalization_executor(
                &dag,
                reward_finalization_start_request(H256::repeat_byte(0x62)),
            )
            .expect_err("mismatched reward identity rejects fresh start");
        assert!(
            error
                .to_string()
                .contains("PBFT_REWARD_VOTES_RESET_CERT_IDENTITY_MISMATCH")
        );
        assert_eq!(storage.extra_reward_votes_reset_generation(), 0);
        assert!(
            storage
                .pbft()
                .finalized_reward_vote_cursor()
                .unwrap()
                .is_none()
        );
        let manager = service.manager_state();
        assert!(manager.finalization_runtime_session.is_none());
        assert!(manager.finalization_runtime_plan.is_none());
        assert!(manager.finalization_sortition_commit_request.is_none());
        assert_eq!(manager.finalization_reward_votes_reset_generation, 0);
        drop(manager);

        drop(service);
        drop(dag);
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
    fn finalization_dag_advancement_rejects_wrong_action_before_mutation() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_dag_wrong_action");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage);
        let initial_period = dag.lock_dag().unwrap().state.period();
        install_finalization_executor(
            &service,
            1,
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
            .advance_finalization_dag_order(&dag, 0)
            .expect("wrong action returns a terminal boundary");
        assert!(!boundary.refresh_dag_counters);
        assert!(boundary.expired_dag_hashes.is_empty());
        assert_eq!(dag.lock_dag().unwrap().state.period(), initial_period);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_none()
        );

        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_dag_operational_error_clears_application_root_state() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_finalization_dag_error");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage);
        install_finalization_executor(
            &service,
            1,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: true,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::SetDagBlockOrder],
        );

        let error = service
            .advance_finalization_dag_order(&dag, 0)
            .expect_err("missing retained anchor must reject native DAG application");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK")
        );
        let manager = service.manager_state();
        assert!(manager.finalization_runtime_session.is_none());
        assert!(manager.finalization_runtime_plan.is_none());
        drop(manager);

        drop(service);
        drop(dag);
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
