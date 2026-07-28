//! Native ownership for the DAG manager runtime.
//!
//! The service restores and owns the deterministic DAG graph, durable storage
//! handle, proposer and verifier cursors, retry state, and pending add-block
//! publication. Bridge code may temporarily borrow a short-lived typed guard
//! while FFI-shaped task methods move into this crate; the mutex itself and its
//! poison policy never leave the native owner.

use crate::dag::{
    DagAddBlockEffectInput, DagAddBlockEffectPlan, DagManagerBlock, DagManagerFinalizationPlan,
    DagManagerSnapshot, DagManagerState, DagPersistenceCounters, DagProposerAttemptPlan,
    DagProposerFrontierFacts, DagProposerSignedBlockIntent, DagProposerUnsignedBlockIntent,
    DagReferenceMetadata, apply_finalization_cleanup_from_storage, dag_block_exists_in_storage,
    dag_manager_block_from_rlp, dag_persistence_counters_from_storage,
    ensure_proposal_period_mapping, plan_dag_add_block_effects, validate_pivot_tips_metadata,
};
use crate::pbft_chain::restore_pbft_chain_from_storage;
use crate::sortition::SortitionParams;
use crate::transaction_packing_service::TransactionPackingSelection;
use anyhow::{Context, Result, anyhow};
use ethereum_types::H256;
use rustaxa_storage::Storage;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};

/// Stable failure identifier returned when the native DAG lock is poisoned.
pub const DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED: &str =
    "DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED";

/// Immutable inputs for restoring the native DAG owner.
#[derive(Clone, Copy, Debug)]
pub struct DagServiceConfig {
    /// Nonzero genesis DAG anchor.
    pub genesis_hash: H256,
    /// Number of levels retained behind the live DAG frontier.
    pub dag_expiry_limit: u32,
    /// Initial level whose proposal-period mapping must resolve to period zero.
    pub max_levels_per_period: u64,
}

/// Native equivalent of the CXX proposer-session construction input.
///
/// The bridge converts the public carrier once before retaining these facts in
/// the native cursor. All fields are immutable for the cursor lifetime.
#[derive(Clone)]
pub struct DagProposerSessionBeginInput {
    pub max_non_finalized_transactions: u64,
    pub dag_expiry_level_limit: u64,
    pub wallet_vrf_public_key: [u8; 32],
    pub wallet_vrf_secret: [u8; 64],
    pub proposer_address: [u8; 20],
    pub max_non_finalized_dag_blocks: u64,
    pub max_non_finalized_dag_blocks_low_difficulty: u64,
    pub max_retry_count: u64,
    pub proposal_weight_limit: u64,
    pub total_transaction_shards: u16,
    pub node_transaction_shard: u16,
    pub shard_period_interval: u64,
    pub pbft_gas_limit: u64,
    pub dag_gas_limit: u64,
    pub max_tips: u16,
}

/// Next deterministic external boundary requested by a verification cursor.
#[derive(Clone)]
pub enum DagVerifyBlockSessionAction {
    TransactionQuery(Vec<H256>),
    AuthorizationFacts,
    VdfSortition {
        vote_count: u64,
        max_vote_count: u64,
        vrf_public_key: [u8; 32],
    },
    Gas,
    Complete,
}

/// Ordered native cursor for one DAG block verification call.
pub struct DagVerifyBlockSession {
    pub cursor_id: u64,
    pub fingerprint: [u8; 32],
    pub generation: u64,
    pub action: DagVerifyBlockSessionAction,
    pub tips: Vec<H256>,
    pub proposal_period: u64,
    pub block_rlp: Vec<u8>,
    pub expected_transactions: u64,
    pub reject_code: u32,
    pub sender_eligible_vote_count: u64,
    pub vdf_sortition_max_vote_count: u64,
    pub eligibility_status: u8,
    pub error_code: String,
}

/// Next deterministic external boundary requested by a proposer cursor.
pub enum DagProposerSessionAction {
    CollectFinalChainFacts,
    PackTransactions,
    StartVdf,
    StaleProofSleep,
    SignBlock,
    AddBlock,
    Complete,
}

/// Transaction-pressure snapshot retained by a proposer cursor.
#[derive(Clone, Copy)]
pub struct DagProposerTransactionObservation {
    pub transaction_pool_size: u64,
    pub non_finalized_transaction_count: u64,
}

/// DAG/frontier snapshot retained for cursor revalidation.
#[derive(Clone)]
pub struct DagProposerObservation {
    pub frontier: DagProposerFrontierFacts,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub period_block_hash_found: bool,
    pub period_block_hash: H256,
    pub fingerprint: [u8; 32],
}

/// Ordered native cursor for one DAG proposal attempt.
pub struct DagProposerSession {
    pub action: DagProposerSessionAction,
    pub begin_input: DagProposerSessionBeginInput,
    pub transaction_observation: DagProposerTransactionObservation,
    pub observation: DagProposerObservation,
    pub attempt: DagProposerAttemptPlan,
    pub retry_key: [u8; 32],
    pub minimum_vdf_difficulty: u16,
    pub sortition_params: SortitionParams,
    pub status: u8,
    pub reason_code: u32,
    pub return_value: bool,
    pub update_retry_state: bool,
    pub next_last_propose_level: u64,
    pub next_retry_count: u64,
    pub record_proposed_block: bool,
    pub vdf_message: Vec<u8>,
    pub selected_transaction_hashes: Vec<H256>,
    pub transaction_gas_estimations: Vec<u64>,
    pub selected_transactions: Vec<TransactionPackingSelection>,
    pub vdf_rlp: Vec<u8>,
    pub unsigned_intent: Option<DagProposerUnsignedBlockIntent>,
    pub signed_intent: Option<DagProposerSignedBlockIntent>,
    pub error_code: String,
}

/// Durable retry cursor for one proposer wallet.
pub struct DagProposerRetryState {
    pub last_propose_level: u64,
    pub retry_count: u64,
    pub max_retry_count: u64,
}

/// Transaction payload retained while an add-block cursor is prepared.
#[derive(Clone)]
pub struct DagAddBlockPreparedTransaction {
    pub input_index: u64,
    pub hash: H256,
    pub trx_rlp: Vec<u8>,
    pub transaction_nonce: [u8; 32],
}

/// Copyable add-block effects retained across an unlocked account query.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DagAddBlockStoredPlan {
    pub accepted: bool,
    pub persist_transactions: bool,
    pub persist_block: bool,
    pub add_to_graph: bool,
    pub emit_verified: bool,
    pub gossip: bool,
    pub proposed: bool,
}

/// Pending cursor for one composed accepted-DAG transition.
#[derive(Clone)]
pub struct DagAddBlockSession {
    pub cursor_id: u64,
    pub block: DagManagerBlock,
    pub block_rlp: Vec<u8>,
    pub save: bool,
    pub proposed: bool,
    pub transactions: Vec<DagAddBlockPreparedTransaction>,
    pub plan: DagAddBlockStoredPlan,
}

/// Complete mutable state serialized by [`DagService`].
///
/// Public fields are a temporary CRW-12 bridge escape hatch. Callers must hold
/// [`DagServiceGuard`] and may not retain a reference across an external
/// executor, callback, sleep, thread handoff, or CXX return.
pub struct DagServiceState {
    pub state: DagManagerState,
    pub storage: Arc<Storage>,
    pub next_proposer_session_id: u64,
    pub next_verify_block_session_id: u64,
    pub next_add_block_session_id: u64,
    pub proposer_sessions: BTreeMap<u64, DagProposerSession>,
    pub proposer_retry_states: BTreeMap<[u8; 32], DagProposerRetryState>,
    pub verify_block_session: Option<DagVerifyBlockSession>,
    pub pending_add_block: Option<DagAddBlockSession>,
}

/// Committed native DAG finalization facts consumed by the application root.
pub(crate) struct DagFinalizationCommit {
    pub finalized_count: usize,
    pub expired_hashes: Vec<H256>,
    pub remove_transaction_hashes: Vec<H256>,
}

impl DagServiceState {
    fn restore(storage: Arc<Storage>, config: DagServiceConfig) -> Result<Self> {
        let mut state = Self {
            state: DagManagerState::new(config.genesis_hash, config.dag_expiry_limit)?,
            storage,
            next_proposer_session_id: 1,
            next_verify_block_session_id: 1,
            next_add_block_session_id: 1,
            proposer_sessions: BTreeMap::new(),
            proposer_retry_states: BTreeMap::new(),
            verify_block_session: None,
            pending_add_block: None,
        };
        state.restore_graph()?;
        ensure_proposal_period_mapping(state.storage.as_ref(), config.max_levels_per_period, 0)?;
        Ok(state)
    }

    fn restore_graph(&mut self) -> Result<()> {
        let pbft_restore = restore_pbft_chain_from_storage(self.storage.as_ref())
            .context("DAG_RUNTIME_RESTORE_PBFT_HEAD")?;
        let stored_anchor = pbft_restore.head.last_non_null_pbft_dag_anchor_hash;
        let anchor = if stored_anchor == H256::zero() {
            self.state.anchor()
        } else {
            stored_anchor
        };
        let anchor_level = if stored_anchor == H256::zero() {
            0
        } else {
            self.storage
                .dag()
                .by_hash(anchor)
                .with_context(|| format!("DAG_RUNTIME_RESTORE_ANCHOR_BLOCK: {anchor:?}"))?
                .level
        };

        let mut non_finalized_blocks = Vec::new();
        for (_level, blocks) in self
            .storage
            .dag()
            .non_finalized()
            .context("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCKS")?
        {
            for block_rlp in blocks {
                non_finalized_blocks.push(
                    dag_manager_block_from_rlp(&block_rlp)
                        .context("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCK_DECODE")?,
                );
            }
        }

        let max_level = non_finalized_blocks
            .iter()
            .map(|block| block.level)
            .chain((stored_anchor != H256::zero()).then_some(anchor_level))
            .max()
            .unwrap_or(0);
        let non_finalized_min_difficulty = non_finalized_blocks
            .iter()
            .map(|block| block.difficulty)
            .min()
            .unwrap_or(u32::MAX);
        let dag_expiry_level = max_level.saturating_sub(u64::from(self.state.dag_expiry_limit()));

        self.state
            .rebuild_from_snapshot(DagManagerSnapshot {
                old_anchor: anchor,
                anchor,
                anchor_level,
                period: pbft_restore.head.size,
                max_level,
                dag_expiry_level,
                non_finalized_min_difficulty,
                non_finalized_blocks,
            })
            .context("DAG_RUNTIME_RESTORE_REBUILD")
    }

    /// Plans one add-block transition from live graph and native storage facts.
    pub(crate) fn plan_add_block(
        &self,
        block: &DagManagerBlock,
        save: bool,
        proposed: bool,
    ) -> Result<DagAddBlockEffectPlan> {
        let block_in_state = self.state.has_vertex(block.hash);
        let block_in_storage = dag_block_exists_in_storage(self.storage.as_ref(), block.hash)
            .context("DAG_RUNTIME_ADD_BLOCK_EXISTS")?;
        let block_exists = if save {
            block_in_storage
        } else {
            block_in_state || block_in_storage
        };
        let pivot_tips = if save
            && !block_in_state
            && !block_exists
            && block.level >= self.state.dag_expiry_level()
        {
            let pivot = self.reference_metadata(block.pivot)?;
            let tips = block
                .tips
                .iter()
                .map(|tip| self.reference_metadata(*tip))
                .collect::<Result<Vec<_>>>()?;
            validate_pivot_tips_metadata(block.level, pivot, &tips)
        } else {
            crate::dag::DagPivotTipsValidation {
                ok: true,
                expected_level: block.level,
                level_matches: true,
                missing_references: Vec::new(),
            }
        };
        let mut plan = plan_dag_add_block_effects(DagAddBlockEffectInput {
            save,
            proposed,
            block_exists,
            block_level: block.level,
            dag_expiry_level: self.state.dag_expiry_level(),
            references_available: pivot_tips.ok,
            missing_references: pivot_tips.missing_references,
        });
        if save && block_in_state && !block_in_storage && plan.accepted && !plan.duplicate {
            plan.add_to_graph = false;
            plan.emit_verified = false;
            plan.gossip = false;
        }
        Ok(plan)
    }

    /// Reads canonical persisted DAG counters from the shared storage owner.
    pub(crate) fn persistence_counters(&self) -> Result<DagPersistenceCounters> {
        dag_persistence_counters_from_storage(self.storage.as_ref())
    }

    /// Applies a finalized order through candidate state and one Rust storage batch.
    ///
    /// The candidate DAG state is published only after all cleanup facts are
    /// preflighted and the durable counter, DAG-row, and transaction-row batch
    /// commits. Empty-anchor periods advance without requiring a stored block.
    pub(crate) fn apply_finalized_order(
        &mut self,
        new_anchor: H256,
        new_period: u64,
        finalized_order: Vec<H256>,
    ) -> Result<DagFinalizationCommit> {
        let mut candidate_state = self.state.clone();
        let plan = if new_anchor == H256::zero() {
            candidate_state
                .advance_empty_period(new_period)
                .context("DAG_RUNTIME_ADVANCE_EMPTY_PERIOD")?;
            DagManagerFinalizationPlan {
                previous_period: self.state.period(),
                new_period,
                previous_anchor: self.state.anchor(),
                current_anchor: self.state.anchor(),
                finalized_count: 0,
                dag_expiry_level: candidate_state.dag_expiry_level(),
                counter_update_hashes: Vec::new(),
                expired_hashes: Vec::new(),
                remaining_hashes: candidate_state
                    .non_finalized_blocks()
                    .values()
                    .flatten()
                    .copied()
                    .collect(),
            }
        } else {
            let anchor_level = self
                .storage
                .dag()
                .by_hash(new_anchor)
                .with_context(|| format!("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK: {new_anchor:?}"))?
                .level;
            candidate_state
                .set_finalized_order(new_anchor, new_period, &finalized_order, anchor_level)
                .context("DAG_RUNTIME_SET_FINALIZED_ORDER")?
        };
        let cleanup = apply_finalization_cleanup_from_storage(
            self.storage.as_ref(),
            &plan.counter_update_hashes,
            &plan.expired_hashes,
            &plan.remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_STORAGE_APPLY")?;
        self.state = candidate_state;
        Ok(DagFinalizationCommit {
            finalized_count: plan.finalized_count,
            expired_hashes: cleanup.expired_hashes,
            remove_transaction_hashes: cleanup.remove_transaction_hashes,
        })
    }

    fn reference_metadata(&self, hash: H256) -> Result<DagReferenceMetadata> {
        let metadata = self.state.reference_metadata(hash);
        if metadata.found {
            return Ok(metadata);
        }
        if self
            .storage
            .dag()
            .by_hash_rlp_optional(hash)
            .context("DAG_RUNTIME_REFERENCE_STORAGE_LOOKUP")?
            .is_none()
        {
            return Ok(metadata);
        }
        let block = self
            .storage
            .dag()
            .by_hash(hash)
            .context("DAG_RUNTIME_REFERENCE_STORAGE_DECODE")?;
        Ok(DagReferenceMetadata {
            hash,
            found: true,
            level: block.level,
        })
    }
}

/// Native owner of DAG construction, restoration, sessions, and locking.
pub struct DagService {
    state: Mutex<DagServiceState>,
}

impl DagService {
    /// Restores all DAG state before publishing the mutex-owning service.
    pub fn restore(storage: Arc<Storage>, config: DagServiceConfig) -> Result<Self> {
        Ok(Self {
            state: Mutex::new(DagServiceState::restore(storage, config)?),
        })
    }

    /// Locks the complete DAG serialization domain.
    pub fn lock(&self) -> Result<DagServiceGuard<'_>> {
        Ok(DagServiceGuard(self.state.lock().map_err(|_| {
            anyhow!(DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED)
        })?))
    }
}

/// Exclusive short-lived guard over the native DAG runtime.
pub struct DagServiceGuard<'a>(MutexGuard<'a, DagServiceState>);

impl Deref for DagServiceGuard<'_> {
    type Target = DagServiceState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for DagServiceGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use rlp::RlpStream;
    use rustaxa_storage::Config;
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn signed_pbft_block(period: u64, pivot: H256) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(10));
        block.append(&pivot);
        block.append(&H256::from_low_u64_be(12));
        block.append(&H256::from_low_u64_be(13));
        block.append(&period);
        block.append(&123u64);
        block.begin_list(0);
        block.append(&vec![0u8; 65]);
        block.out().to_vec()
    }

    fn period_data(pbft_block: &[u8]) -> Vec<u8> {
        let mut data = RlpStream::new_list(4);
        data.append_raw(pbft_block, 1);
        data.append_empty_data();
        data.append_empty_data();
        data.begin_list(0);
        data.out().to_vec()
    }

    fn seed_pbft_head(storage: &Storage, period: u64, pivot: H256) -> Result<()> {
        let pbft_block = signed_pbft_block(period, pivot);
        let pbft_link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&pbft_block))?;
        storage.period().write(period, &period_data(&pbft_block))?;
        storage
            .period()
            .write_pbft_period(pbft_link.block_hash, period)?;
        storage.pbft().write_head(
            H256::zero(),
            format!(
                r#"{{"head_hash":"0x{:064x}","size":{},"non_empty_size":{},"last_pbft_block_hash":"0x{:064x}"}}"#,
                0, period, period, pbft_link.block_hash
            )
            .as_bytes(),
        )?;
        Ok(())
    }

    fn dag_block(pivot: H256, level: u64, difficulty: u16) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11u8; 80]);
        vdf.append(&vec![0x22u8]);
        vdf.append(&vec![0x33u8]);
        vdf.append(&difficulty);

        let mut block = RlpStream::new_list(8);
        block.append(&pivot);
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    #[test]
    fn fresh_restore_publishes_complete_empty_session_owner() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_fresh");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        let runtime = service.lock()?;
        assert!(Arc::ptr_eq(&runtime.storage, &storage));
        assert_eq!(runtime.state.anchor(), H256::repeat_byte(1));
        assert_eq!(runtime.next_proposer_session_id, 1);
        assert_eq!(runtime.next_verify_block_session_id, 1);
        assert_eq!(runtime.next_add_block_session_id, 1);
        assert!(runtime.proposer_sessions.is_empty());
        assert!(runtime.proposer_retry_states.is_empty());
        assert!(runtime.verify_block_session.is_none());
        assert!(runtime.pending_add_block.is_none());
        drop(runtime);
        drop(service);
        let restarted = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        assert_eq!(restarted.lock()?.state.anchor(), H256::repeat_byte(1));
        drop(restarted);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn poisoned_lock_returns_stable_identifier() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_poison");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = Arc::new(DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(2),
                dag_expiry_limit: 8,
                max_levels_per_period: 10,
            },
        )?);
        let poison_owner = service.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_owner.lock().expect("lock before poisoning");
            panic!("poison dag service");
        })
        .join();
        let error = service.lock().err().expect("poisoned lock must fail");
        assert_eq!(error.to_string(), DAG_TRANSACTION_SERVICE_DAG_LOCK_POISONED);
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_rebuilds_persisted_head_and_non_finalized_graph() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_restore");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let genesis = H256::repeat_byte(1);
        let anchor_rlp = dag_block(genesis, 3, 3);
        let anchor = dag_manager_block_from_rlp(&anchor_rlp)?;
        let live_rlp = dag_block(anchor.hash, 4, 4);
        let live = dag_manager_block_from_rlp(&live_rlp)?;

        seed_pbft_head(storage.as_ref(), 1, anchor.hash)?;
        storage.dag().write(
            anchor.hash,
            anchor.level,
            anchor.tips.len() as u64,
            &anchor_rlp,
        )?;
        storage
            .dag()
            .write(live.hash, live.level, live.tips.len() as u64, &live_rlp)?;

        let service = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: genesis,
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )?;
        let runtime = service.lock()?;
        assert_eq!(runtime.state.period(), 1);
        assert_eq!(runtime.state.anchor(), anchor.hash);
        assert!(runtime.state.has_vertex(live.hash));
        assert_eq!(runtime.state.max_level(), 4);
        assert_eq!(runtime.state.non_finalized_min_difficulty(), 3);
        assert_eq!(runtime.state.non_finalized_blocks_size().1, 2);
        drop(runtime);
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_fails_before_publication_when_persisted_anchor_is_missing() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_missing_anchor");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        seed_pbft_head(storage.as_ref(), 1, H256::repeat_byte(9))?;
        let error = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )
        .err()
        .expect("missing anchor must reject restoration");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_RESTORE_ANCHOR_BLOCK")
        );
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_fails_before_publication_on_malformed_non_finalized_payload() -> Result<()> {
        let path = temp_path("rustaxa_consensus_dag_service_malformed");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        storage.dag().write(H256::repeat_byte(7), 1, 0, &[0xc0])?;
        let error = DagService::restore(
            storage.clone(),
            DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
        )
        .err()
        .expect("malformed DAG payload must reject restoration");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCKS"),
            "{error:#}"
        );
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
