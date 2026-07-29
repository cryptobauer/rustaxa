//! Native PBFT application-service composition and lifecycle ownership.
//!
//! [`PbftService`] restores and publishes the complete PBFT sibling graph from
//! one shared storage handle. Each sibling retains its own synchronization
//! domain; this root owns composition and bootstrap readiness only and never
//! adds a root-wide lock.

use crate::pbft_chain::PbftChainService;
use crate::pbft_manager::{
    PbftManagerGuard, PbftManagerService, PbftManagerStorageStartupFact,
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
}
