//! Atomic PBFT period cleanup across service-owned vote and proposal state.
//!
//! Period advance previously exposed independent verified-vote and proposed-
//! block cleanup calls. This module plans both while holding the service lock
//! order, commits all durable proposal deletions once, and only then applies
//! exact, infallible in-memory removals to both owners.

use crate::ffi::rustaxa_ffi::PbftPeriodStateCleanupResult;
use crate::ffi::BridgePbftService;
use anyhow::{Context, Result};
use rustaxa_consensus::proposed_blocks::{
    append_proposed_blocks_cleanup_to_batch, ProposedBlockPeriodHashes,
};
use rustaxa_storage::{Storage, StorageWriteBatch};

const CLEANUP_NOT_REQUIRED: u8 = 0;
const CLEANUP_APPLIED: u8 = 1;
const CLEANUP_REJECTED: u8 = 2;

impl BridgePbftService {
    /// Atomically cleans service-owned period state after PBFT finalization.
    ///
    /// `finalized_chain_size` must be nonzero and `new_period` must be its exact
    /// checked successor. Verified votes older than the finalized chain size
    /// and proposed blocks older than the new period are planned under the
    /// fixed `verified_votes -> proposed_blocks` lock order. When proposal rows
    /// exist, one caller-owned Rust storage batch commits before either memory
    /// owner changes. Rejected validation or persistence publishes no mutation.
    pub fn pbft_service_cleanup_period_state(
        &self,
        finalized_chain_size: u64,
        new_period: u64,
    ) -> Result<PbftPeriodStateCleanupResult> {
        self.cleanup_period_state_with_commit(finalized_chain_size, new_period, |storage, batch| {
            storage
                .commit_write_batch_with_sync(batch, false)
                .context("PBFT_PERIOD_STATE_CLEANUP_COMMIT")
        })
    }

    fn cleanup_period_state_with_commit<F>(
        &self,
        finalized_chain_size: u64,
        new_period: u64,
        commit: F,
    ) -> Result<PbftPeriodStateCleanupResult>
    where
        F: FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    {
        if finalized_chain_size == 0 {
            return Ok(rejected(
                finalized_chain_size,
                new_period,
                "PBFT_PERIOD_STATE_CLEANUP_EMPTY_FINALIZED_CHAIN",
            ));
        }
        if finalized_chain_size.checked_add(1) != Some(new_period) {
            return Ok(rejected(
                finalized_chain_size,
                new_period,
                "PBFT_PERIOD_STATE_CLEANUP_INVALID_SUCCESSOR",
            ));
        }

        let mut verified_votes = self
            .verified_votes
            .lock()
            .expect("verified votes lock poisoned");
        let Some(runtime) = verified_votes.as_mut() else {
            return Ok(rejected(
                finalized_chain_size,
                new_period,
                "PBFT_SERVICE_VERIFIED_VOTES_UNAVAILABLE",
            ));
        };
        let mut proposed_blocks = self
            .proposed_blocks
            .write()
            .expect("proposed blocks lock poisoned");

        let vote_plan = runtime.plan_cleanup_votes_by_period(finalized_chain_size);
        let proposed_plan = proposed_blocks.cleanup_candidates(new_period);
        let proposed_blocks_removed = proposed_block_count(&proposed_plan);
        let any_memory_cleanup = vote_plan.periods_removed() != 0
            || vote_plan.payloads_removed() != 0
            || !proposed_plan.is_empty();

        if !any_memory_cleanup {
            return Ok(PbftPeriodStateCleanupResult {
                status: CLEANUP_NOT_REQUIRED,
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
            let Some(storage) = self.storage.as_ref() else {
                return Ok(rejected(
                    finalized_chain_size,
                    new_period,
                    "PBFT_SERVICE_STORAGE_UNAVAILABLE",
                ));
            };
            let mut batch = storage.create_write_batch();
            let appended = match append_proposed_blocks_cleanup_to_batch(
                storage.as_ref(),
                &mut batch,
                &proposed_plan,
            ) {
                Ok(appended) => appended,
                Err(_) => {
                    return Ok(rejected(
                        finalized_chain_size,
                        new_period,
                        "PBFT_PERIOD_STATE_CLEANUP_STORAGE_DELETE",
                    ));
                }
            };
            if commit(storage.as_ref(), batch).is_err() {
                return Ok(rejected(
                    finalized_chain_size,
                    new_period,
                    "PBFT_PERIOD_STATE_CLEANUP_STORAGE_COMMIT",
                ));
            }
            debug_assert_eq!(appended, proposed_blocks_removed);
        }

        let result = PbftPeriodStateCleanupResult {
            status: CLEANUP_APPLIED,
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
        for period in &proposed_plan {
            proposed_blocks.remove_period(period.period);
        }
        Ok(result)
    }
}

/// CXX entrypoint for atomic service-owned PBFT period cleanup.
pub fn pbft_service_cleanup_period_state(
    service: &BridgePbftService,
    finalized_chain_size: u64,
    new_period: u64,
) -> Result<PbftPeriodStateCleanupResult> {
    service.pbft_service_cleanup_period_state(finalized_chain_size, new_period)
}

fn proposed_block_count(plan: &[ProposedBlockPeriodHashes]) -> u64 {
    plan.iter()
        .map(|period| period.block_hashes.len() as u64)
        .sum()
}

fn rejected(
    finalized_chain_size: u64,
    new_period: u64,
    error_code: &str,
) -> PbftPeriodStateCleanupResult {
    PbftPeriodStateCleanupResult {
        status: CLEANUP_REJECTED,
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
    use crate::ffi::BridgePbftChainState;
    use ethereum_types::{H160, H256};
    use rustaxa_consensus::pbft_chain::{PbftChain, PbftChainHead};
    use rustaxa_consensus::verified_votes::{PbftVoteType, VerifiedVote};
    use rustaxa_consensus::PbftVoteAdmissionRuntime;
    use rustaxa_storage::{Column, Config};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn storage(name: &str) -> (Arc<Storage>, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}_{nonce}"));
        (
            Arc::new(Storage::new(Config::new(path.clone())).unwrap()),
            path,
        )
    }

    fn service(
        storage: Option<Arc<Storage>>,
        runtime: Option<PbftVoteAdmissionRuntime>,
    ) -> BridgePbftService {
        BridgePbftService {
            manager: Mutex::new(None),
            chain: Arc::new(RwLock::new(BridgePbftChainState {
                state: PbftChain::new(PbftChainHead {
                    head_hash: H256::zero(),
                    size: 0,
                    non_empty_size: 0,
                    last_pbft_block_hash: H256::zero(),
                    last_non_null_pbft_dag_anchor_hash: H256::zero(),
                })
                .unwrap(),
                initialized_default: true,
            })),
            proposed_blocks: RwLock::new(Default::default()),
            verified_votes: Mutex::new(runtime),
            slashing: None,
            storage,
            bootstrap_complete: AtomicBool::new(true),
            pillar: None,
            pillar_ready: AtomicBool::new(false),
        }
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

    #[test]
    fn cleanup_rejects_commit_without_mutation_then_retries_with_exact_counts() {
        let (storage, path) = storage("rustaxa_period_cleanup_retry");
        let mut runtime = PbftVoteAdmissionRuntime::new();
        runtime
            .verified_votes_mut()
            .add_verified_vote(vote(1, 11), None)
            .unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(vote(2, 12), None)
            .unwrap();
        let service = service(Some(storage.clone()), Some(runtime));
        let old_hash = H256::from_low_u64_be(900);
        let kept_hash = H256::from_low_u64_be(901);
        service
            .proposed_blocks
            .write()
            .unwrap()
            .push(11, old_hash, H256::zero(), vec![0x11]);
        service
            .proposed_blocks
            .write()
            .unwrap()
            .push(13, kept_hash, H256::zero(), vec![0x13]);
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                old_hash.as_bytes(),
                &[0x11],
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::ProposedPbftBlocks,
                kept_hash.as_bytes(),
                &[0x13],
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let rejected = service
            .cleanup_period_state_with_commit(12, 13, |_, _| {
                Err(anyhow::anyhow!("injected commit failure"))
            })
            .unwrap();
        assert_eq!(rejected.status, CLEANUP_REJECTED);
        assert!(!rejected.transition_published);
        assert_eq!(rejected.verified_votes_removed, 0);
        assert_eq!(rejected.proposed_blocks_removed, 0);
        assert_eq!(
            service
                .verified_votes
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .verified_votes()
                .size(),
            2
        );
        assert!(service
            .proposed_blocks
            .read()
            .unwrap()
            .contains(11, old_hash));
        assert!(storage
            .get_raw(Column::ProposedPbftBlocks, old_hash.as_bytes())
            .unwrap()
            .is_some());

        let applied = service.pbft_service_cleanup_period_state(12, 13).unwrap();
        assert_eq!(applied.status, CLEANUP_APPLIED);
        assert!(applied.transition_published);
        assert_eq!(applied.verified_vote_periods_removed, 1);
        assert_eq!(applied.verified_votes_removed, 1);
        assert_eq!(applied.vote_payloads_removed, 0);
        assert_eq!(applied.proposed_block_periods_removed, 1);
        assert_eq!(applied.proposed_blocks_removed, 1);
        assert!(applied.persistence_required);
        assert_eq!(applied.persistence_applied_deletes, 1);
        assert_eq!(
            service
                .verified_votes
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .verified_votes()
                .size(),
            1
        );
        assert!(!service
            .proposed_blocks
            .read()
            .unwrap()
            .contains(11, old_hash));
        assert!(service
            .proposed_blocks
            .read()
            .unwrap()
            .contains(13, kept_hash));
        assert!(storage
            .get_raw(Column::ProposedPbftBlocks, old_hash.as_bytes())
            .unwrap()
            .is_none());
        assert!(storage
            .get_raw(Column::ProposedPbftBlocks, kept_hash.as_bytes())
            .unwrap()
            .is_some());

        drop(service);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn cleanup_noop_invalid_relation_and_chain_only_are_typed() {
        let empty = service(None, Some(PbftVoteAdmissionRuntime::new()));
        let no_op = empty.pbft_service_cleanup_period_state(12, 13).unwrap();
        assert_eq!(no_op.status, CLEANUP_NOT_REQUIRED);
        assert!(no_op.transition_published);
        assert!(!no_op.persistence_required);

        let invalid = empty.pbft_service_cleanup_period_state(12, 14).unwrap();
        assert_eq!(invalid.status, CLEANUP_REJECTED);
        assert!(!invalid.transition_published);
        assert_eq!(
            invalid.error_code,
            "PBFT_PERIOD_STATE_CLEANUP_INVALID_SUCCESSOR"
        );
        let overflow = empty
            .pbft_service_cleanup_period_state(u64::MAX, u64::MAX)
            .unwrap();
        assert_eq!(overflow.status, CLEANUP_REJECTED);

        let chain_only = service(None, None);
        let rejected = chain_only
            .pbft_service_cleanup_period_state(12, 13)
            .unwrap();
        assert_eq!(rejected.status, CLEANUP_REJECTED);
        assert_eq!(
            rejected.error_code,
            "PBFT_SERVICE_VERIFIED_VOTES_UNAVAILABLE"
        );
        assert!(!rejected.transition_published);
    }
}
