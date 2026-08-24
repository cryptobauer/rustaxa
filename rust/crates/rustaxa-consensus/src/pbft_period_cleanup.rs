//! Native PBFT period-state cleanup for verified-vote/proposed-block siblings.
//!
//! This boundary owns one atomic transition step: plan stale verified-vote and
//! proposed-block candidate rows, persist proposal-row deletions when required,
//! and only then publish sibling in-memory cleanup. Rejected transitions do
//! not publish mutable cleanup state.

use crate::{
    pbft_vote_runtime::PbftVerifiedVotesService,
    proposed_blocks::{
        ProposedBlockPeriodHashes, ProposedBlocksService, append_proposed_blocks_cleanup_to_batch,
    },
};
use anyhow::Result;
use rustaxa_storage::{Storage, StorageWriteBatch};

/// Typed PBFT period-state cleanup status emitted by Rust-owned cleanup calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PbftPeriodStateCleanupStatus {
    /// The cleanup request was valid but no stale data existed for persistence or
    /// in-memory cleanup.
    NotRequired,
    /// Cleanup persisted/staged required state and applied in-memory removals.
    Applied,
    /// Cleanup request was rejected due to validation or durable-write failure.
    Rejected,
}

/// Result of one period-state cleanup attempt.
///
/// Counts describe only the mutation published by this call. Rejected results
/// always report zero mutations and `transition_published == false`; valid
/// no-op results publish the transition with zero counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PbftPeriodStateCleanupResult {
    /// Typed cleanup outcome.
    pub status: PbftPeriodStateCleanupStatus,
    /// Stable diagnostic code for rejected calls, otherwise empty.
    pub error_code: String,
    /// Whether the cleanup transition was durably accepted and published.
    pub transition_published: bool,
    /// Finalized PBFT-chain size supplied by the caller.
    pub finalized_chain_size: u64,
    /// New PBFT period supplied by the caller.
    pub new_period: u64,
    /// Complete verified-vote period maps removed from memory.
    pub verified_vote_periods_removed: u64,
    /// Individual verified votes removed from memory.
    pub verified_votes_removed: u64,
    /// Canonical verified-vote payloads removed from memory.
    pub vote_payloads_removed: u64,
    /// Proposed-block period maps removed from memory.
    pub proposed_block_periods_removed: u64,
    /// Individual proposed blocks removed from memory and storage.
    pub proposed_blocks_removed: u64,
    /// Whether durable proposed-block deletion was required.
    pub persistence_required: bool,
    /// Number of durable proposed-block deletes committed by the batch.
    pub persistence_applied_deletes: u64,
}

const CLEANUP_EMPTY_FINALIZED_CHAIN: &str = "PBFT_PERIOD_STATE_CLEANUP_EMPTY_FINALIZED_CHAIN";
const CLEANUP_INVALID_SUCCESSOR: &str = "PBFT_PERIOD_STATE_CLEANUP_INVALID_SUCCESSOR";
const CLEANUP_STORAGE_DELETE: &str = "PBFT_PERIOD_STATE_CLEANUP_STORAGE_DELETE";
const CLEANUP_STORAGE_COMMIT: &str = "PBFT_PERIOD_STATE_CLEANUP_STORAGE_COMMIT";

/// Plans, persists, and publishes one cross-sibling PBFT period cleanup.
///
/// `finalized_chain_size` and `new_period` identify the exact checked period
/// transition. `commit` owns the single durable proposal-deletion batch and is
/// injectable only so native tests can prove failure-before-publication.
/// Successful results report exact vote, payload, and proposal removals;
/// validation or commit failures return typed rejected results with zero
/// published counts.
///
/// Locks are intentionally ordered as verified votes then proposed blocks and
/// remain held through commit. The order must not be inverted by adjacent
/// operations that borrow both siblings.
#[cfg(test)]
pub(crate) fn cleanup_period_state_with_commit<F>(
    verified_votes: &PbftVerifiedVotesService,
    proposed_blocks: &ProposedBlocksService,
    finalized_chain_size: u64,
    new_period: u64,
    commit: F,
) -> Result<PbftPeriodStateCleanupResult>
where
    F: FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
{
    cleanup_period_state_with_commit_and_publish(
        verified_votes,
        proposed_blocks,
        finalized_chain_size,
        new_period,
        commit,
        || {},
    )
}

/// Performs cleanup and an infallible owner publication in one lock epoch.
///
/// `publish` runs after durable commit and live sibling cleanup, but before
/// either sibling guard is released. Callers must prevalidate it so no
/// fallible operation remains after cleanup publication begins.
pub(crate) fn cleanup_period_state_with_commit_and_publish<F, P>(
    verified_votes: &PbftVerifiedVotesService,
    proposed_blocks: &ProposedBlocksService,
    finalized_chain_size: u64,
    new_period: u64,
    commit: F,
    publish: P,
) -> Result<PbftPeriodStateCleanupResult>
where
    F: FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    P: FnOnce(),
{
    if finalized_chain_size == 0 {
        return Ok(rejected(
            finalized_chain_size,
            new_period,
            CLEANUP_EMPTY_FINALIZED_CHAIN,
        ));
    }

    if finalized_chain_size.checked_add(1) != Some(new_period) {
        return Ok(rejected(
            finalized_chain_size,
            new_period,
            CLEANUP_INVALID_SUCCESSOR,
        ));
    }

    let mut runtime = verified_votes
        .lock()
        .map_err(|_| anyhow::Error::msg("PBFT_VERIFIED_VOTES_SERVICE_LOCK_POISONED"))?;
    let mut proposed_blocks = proposed_blocks
        .write()
        .map_err(|_| anyhow::Error::msg("PBFT_PROPOSED_BLOCKS_SERVICE_LOCK_POISONED"))?;

    let vote_plan = runtime.plan_cleanup_votes_by_period(finalized_chain_size);
    let proposed_plan = proposed_blocks.cleanup_candidates(new_period);
    let proposed_blocks_removed = proposed_block_count(&proposed_plan);
    let any_memory_cleanup = vote_plan.periods_removed() != 0
        || vote_plan.payloads_removed() != 0
        || !proposed_plan.is_empty();

    if !any_memory_cleanup {
        publish();
        return Ok(PbftPeriodStateCleanupResult {
            status: PbftPeriodStateCleanupStatus::NotRequired,
            error_code: String::new(),
            transition_published: true,
            finalized_chain_size,
            new_period,
            verified_vote_periods_removed: 0,
            verified_votes_removed: 0,
            vote_payloads_removed: 0,
            proposed_block_periods_removed: 0,
            proposed_blocks_removed: 0,
            persistence_required: false,
            persistence_applied_deletes: 0,
        });
    }

    if proposed_blocks_removed != 0 {
        let storage = verified_votes.storage();
        let mut batch = storage.create_write_batch();
        let appended =
            match append_proposed_blocks_cleanup_to_batch(storage, &mut batch, &proposed_plan) {
                Ok(appended) => appended,
                Err(_) => {
                    return Ok(rejected(
                        finalized_chain_size,
                        new_period,
                        CLEANUP_STORAGE_DELETE,
                    ));
                }
            };
        if commit(storage, batch).is_err() {
            return Ok(rejected(
                finalized_chain_size,
                new_period,
                CLEANUP_STORAGE_COMMIT,
            ));
        }
        debug_assert_eq!(appended, proposed_blocks_removed);
    }

    let result = PbftPeriodStateCleanupResult {
        status: PbftPeriodStateCleanupStatus::Applied,
        error_code: String::new(),
        transition_published: true,
        finalized_chain_size,
        new_period,
        verified_vote_periods_removed: vote_plan.periods_removed(),
        verified_votes_removed: vote_plan.votes_removed(),
        vote_payloads_removed: vote_plan.payloads_removed(),
        proposed_block_periods_removed: proposed_plan.len() as u64,
        proposed_blocks_removed,
        persistence_required: proposed_blocks_removed != 0,
        persistence_applied_deletes: proposed_blocks_removed,
    };

    runtime.apply_cleanup_votes_by_period(&vote_plan);
    for plan in &proposed_plan {
        proposed_blocks.remove_period(plan.period);
    }
    publish();
    Ok(result)
}

fn proposed_block_count(plan: &[ProposedBlockPeriodHashes]) -> u64 {
    plan.iter()
        .map(|entry| entry.block_hashes.len() as u64)
        .sum()
}

fn rejected(
    finalized_chain_size: u64,
    new_period: u64,
    error_code: &str,
) -> PbftPeriodStateCleanupResult {
    PbftPeriodStateCleanupResult {
        status: PbftPeriodStateCleanupStatus::Rejected,
        error_code: error_code.to_owned(),
        transition_published: false,
        finalized_chain_size,
        new_period,
        verified_vote_periods_removed: 0,
        verified_votes_removed: 0,
        vote_payloads_removed: 0,
        proposed_block_periods_removed: 0,
        proposed_blocks_removed: 0,
        persistence_required: false,
        persistence_applied_deletes: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PbftService,
        pbft_manager::{
            PbftManagerTransitionFact, PbftManagerTransitionKind, plan_pbft_manager_transition,
        },
        pbft_service::PbftServiceConfig,
        pbft_vote_runtime::PbftVoteAdmissionRuntime,
        verified_votes::{PbftVoteType, VerifiedVote},
    };
    use ethereum_types::{H160, H256};
    use rustaxa_storage::{Column, Config};
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use std::convert::TryFrom;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_storage(name: &str) -> (Arc<Storage>, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        (storage, path)
    }

    fn test_service(
        storage: Option<Arc<Storage>>,
        runtime: PbftVoteAdmissionRuntime,
    ) -> (PbftService, Option<PathBuf>) {
        let (storage, path) = match storage {
            Some(storage) => (storage, None),
            None => {
                let (storage, path) = test_storage("pbft_period_cleanup_service");
                (storage, Some(path))
            }
        };
        let service = PbftService::restore(
            storage,
            PbftServiceConfig {
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
                dag_blocks_size: 50,
                ghost_path_move_back: 0,
                node_version: (0, 0, 0, 0),
                node_version_suffix: b"T".to_vec(),
                default_pbft_gas_limit: 1_000_000,
                cornus_activation_period: u64::MAX,
                cornus_pbft_gas_limit: 1_000_000,
                process_synced_policy: crate::pbft_service::PbftProcessSyncedPolicy {
                    chain_id: 2999,
                    lambda_min_ms: 100,
                    lambda_change_interval: 10,
                    lambda_change_ms: 10,
                    consensus_delay_ms: 400,
                    dpos_blocks_per_year: 500,
                    recently_finalized_factor: 3,
                },
            },
        )
        .unwrap();
        *service.verified_votes().lock().unwrap() = runtime;
        (service, path)
    }

    fn vote(hash: u64, period: u64) -> VerifiedVote {
        VerifiedVote::new(
            H256::from_low_u64_be(hash),
            H256::from_low_u64_be(hash + 100),
            H160::from_low_u64_be(hash),
            period,
            1,
            3,
            PbftVoteType::Cert,
            1,
        )
        .unwrap()
    }

    fn proposed_block(period: u64, timestamp: u64) -> (Vec<u8>, PbftBlockLink) {
        let mut stream = rlp::RlpStream::new_list(8);
        stream.append(&H256::from_low_u64_be(period));
        stream.append(&H256::from_low_u64_be(period + 1));
        stream.append(&H256::from_low_u64_be(period + 2));
        stream.append(&H256::from_low_u64_be(period + 3));
        stream.append(&period);
        stream.append(&timestamp);
        stream.append(&H256::from_low_u64_be(period + 4));
        stream.append(&vec![0_u8; 65]);
        let block_rlp = stream.out().to_vec();
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp))
            .expect("test PBFT block should decode");
        (block_rlp, link)
    }

    fn record_committed_reset(service: &PbftService, target_period: u64) {
        let plan = plan_pbft_manager_transition(PbftManagerTransitionFact {
            kind: PbftManagerTransitionKind::ResetConsensus,
            period: 1,
            round: 1,
            step: 1,
            target_round: 1,
            current_round_lambda_ms: 100,
            target_round_lambda_ms: 100,
            default_lambda_ms: 100,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            network_next_voting_step: 0,
            deadline_ms: 400,
            polling_interval_ms: 100,
            next_step_time_ms: 400,
            cacti_hardfork: false,
            has_cert_voted_block: false,
            executed_pbft_block: false,
        });
        assert!(plan.error_code.is_empty());
        service
            .manager_state()
            .state
            .record_committed_reset(target_period, &plan);
    }

    #[test]
    fn period_advance_commit_failure_preserves_state_and_retries() {
        let (storage, path) = test_storage("rustaxa_consensus_period_cleanup_retry");
        let mut runtime = PbftVoteAdmissionRuntime::new();
        runtime
            .verified_votes_mut()
            .add_verified_vote(vote(1, 11), None)
            .unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(vote(2, 12), None)
            .unwrap();
        let service = test_service(Some(storage.clone()), runtime).0;
        let (old_rlp, old_link) = proposed_block(11, 900);
        let (kept_rlp, kept_link) = proposed_block(13, 901);
        let old_hash = old_link.block_hash;
        let kept_hash = kept_link.block_hash;
        {
            let mut proposed = service.proposed_blocks().write().unwrap();
            proposed.push(11, old_hash, old_link.pivot_dag_block_hash, old_rlp.clone());
            proposed.push(
                13,
                kept_hash,
                kept_link.pivot_dag_block_hash,
                kept_rlp.clone(),
            );
        }

        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                old_hash.as_bytes(),
                &old_rlp,
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                kept_hash.as_bytes(),
                &kept_rlp,
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let missing_reset = service
            .apply_period_advance(13)
            .expect("missing provenance returns a rejected snapshot");
        assert_eq!(
            missing_reset.error_code,
            "PBFT_MANAGER_ADVANCE_PERIOD_RESET_NOT_COMMITTED"
        );
        assert_eq!(
            service
                .verified_votes()
                .lock()
                .unwrap()
                .verified_votes()
                .size(),
            2
        );
        assert!(
            service
                .proposed_blocks()
                .read()
                .unwrap()
                .contains(11, old_hash)
        );

        record_committed_reset(&service, 13);
        let commit_error = service
            .apply_period_advance_with_commit(13, |_, _| {
                Err(anyhow::anyhow!("injected commit failure"))
            })
            .expect_err("durable cleanup failure must stop publication");
        assert!(commit_error.to_string().contains(CLEANUP_STORAGE_COMMIT));
        assert_eq!(service.manager_state().state.snapshot().period, 1);
        assert!(
            service
                .manager_state()
                .state
                .plan_advance_period_after_reset(12)
                .accepted,
            "failed cleanup must preserve committed-reset provenance"
        );
        assert_eq!(
            service
                .verified_votes()
                .lock()
                .unwrap()
                .verified_votes()
                .size(),
            2
        );
        assert!(
            service
                .proposed_blocks()
                .read()
                .unwrap()
                .contains(11, old_hash)
        );
        assert!(
            storage
                .get_raw(Column::ProposedPbftBlocks, old_hash.as_bytes())
                .unwrap()
                .is_some()
        );

        let applied = service
            .apply_period_advance(13)
            .expect("combined period commit should recover on retry");
        assert_eq!(applied.period, 13);
        assert!(applied.error_code.is_empty());
        assert_eq!(
            service
                .verified_votes()
                .lock()
                .unwrap()
                .verified_votes()
                .size(),
            1
        );
        assert!(
            !service
                .proposed_blocks()
                .read()
                .unwrap()
                .contains(11, old_hash)
        );
        assert!(
            service
                .proposed_blocks()
                .read()
                .unwrap()
                .contains(13, kept_hash)
        );
        assert!(
            storage
                .get_raw(Column::ProposedPbftBlocks, old_hash.as_bytes())
                .unwrap()
                .is_none()
        );
        assert!(
            storage
                .get_raw(Column::ProposedPbftBlocks, kept_hash.as_bytes())
                .unwrap()
                .is_some()
        );

        let duplicate = service
            .apply_period_advance(13)
            .expect("duplicate report returns a rejected snapshot");
        assert_eq!(duplicate.period, 13);
        assert_eq!(
            duplicate.error_code,
            "PBFT_MANAGER_ADVANCE_PERIOD_NON_INCREASING_PERIOD"
        );

        drop(service);
        let restarted = test_service(Some(storage.clone()), PbftVoteAdmissionRuntime::new()).0;
        assert!(
            !restarted
                .proposed_blocks()
                .read()
                .unwrap()
                .contains(11, old_hash)
        );
        assert!(
            restarted
                .proposed_blocks()
                .read()
                .unwrap()
                .contains(13, kept_hash)
        );
        drop(restarted);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn cleanup_noop_and_invalid_relation_are_typed() {
        let (service, path) = test_service(None, PbftVoteAdmissionRuntime::new());
        let no_op = service.cleanup_period_state(12, 13).unwrap();
        assert_eq!(no_op.status, PbftPeriodStateCleanupStatus::NotRequired);
        assert!(no_op.transition_published);
        assert!(!no_op.persistence_required);
        record_committed_reset(&service, 13);
        let snapshot = service.apply_period_advance(13).unwrap();
        assert_eq!(snapshot.period, 13);
        assert!(snapshot.error_code.is_empty());

        service
            .verified_votes()
            .lock()
            .unwrap()
            .verified_votes_mut()
            .add_verified_vote(vote(3, 11), None)
            .unwrap();
        let vote_only = service.cleanup_period_state(12, 13).unwrap();
        assert_eq!(vote_only.status, PbftPeriodStateCleanupStatus::Applied);
        assert_eq!(vote_only.verified_votes_removed, 1);
        assert_eq!(vote_only.proposed_blocks_removed, 0);
        assert!(!vote_only.persistence_required);
        assert_eq!(vote_only.persistence_applied_deletes, 0);

        let empty_chain = service.cleanup_period_state(0, 1).unwrap();
        assert_eq!(empty_chain.status, PbftPeriodStateCleanupStatus::Rejected);
        assert_eq!(empty_chain.error_code, CLEANUP_EMPTY_FINALIZED_CHAIN);

        let invalid = service.cleanup_period_state(12, 14).unwrap();
        assert_eq!(invalid.status, PbftPeriodStateCleanupStatus::Rejected);
        assert!(!invalid.transition_published);
        assert_eq!(invalid.error_code, CLEANUP_INVALID_SUCCESSOR);

        let overflow = service.cleanup_period_state(u64::MAX, u64::MAX).unwrap();
        assert_eq!(overflow.status, PbftPeriodStateCleanupStatus::Rejected);
        assert_eq!(overflow.error_code, CLEANUP_INVALID_SUCCESSOR);

        drop(service);
        if let Some(path) = path {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
