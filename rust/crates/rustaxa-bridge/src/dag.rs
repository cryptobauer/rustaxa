use crate::ffi::rustaxa_ffi::{
    DagAddBlockEffectPlan, DagAddBlockRuntimeInput, DagBlockLookup, DagFrontier, DagHash,
    DagLevelHashes, DagManagerAnchors, DagManagerBlock, DagManagerFinalizationApplyPayload,
    DagManagerNonFinalizedSize, DagManagerNonFinalizedSyncPayload, DagOrder,
    DagPersistenceCounters, DagPivotTipsValidation, DagProposerAddBlockReport,
    DagProposerExternalProposalFactsReport, DagProposerSessionBeginInput, DagProposerSessionStep,
    DagProposerSignedBlockIntent, DagProposerSigningReport, DagProposerStorageTipSelectionInput,
    DagProposerTipSelectionPlan, DagProposerTransactionPackReport,
    DagProposerTransactionPackRequest, DagProposerVdfProofReport, DagProposerWorkerCommand,
    DagProposerWorkerCommandInput, DagSyncBlockRlp, DagTransactionHash, DagTransactionRlpLookup,
    DagVerifyBlockAuthorizationReport, DagVerifyBlockGasReport, DagVerifyBlockSessionInput,
    DagVerifyBlockSessionStep, DagVerifyBlockTransactionReport, DagVerifyBlockVdfReport,
    DagVerifyVdfSortitionFromBlockInput, DagVerifyVdfSortitionResult, HashLookup,
    SortitionRuntimeParams,
};
use crate::ffi::{BridgeDagManagerRuntime, BridgeStorage};
use anyhow::{ensure, Context, Result};
use ethereum_types::H256;
#[cfg(test)]
use rustaxa_consensus::dag::collect_finalization_cleanup_from_storage;
use rustaxa_consensus::dag::{
    apply_finalization_cleanup_from_storage, collect_non_finalized_sync_payload_from_storage,
    construct_dag_vdf_message, dag_block_exists_in_storage,
    dag_manager_block_from_rlp as domain_dag_manager_block_from_rlp,
    dag_persistence_counters_from_storage, decide_dag_verify_vdf_dpos_authorization,
    ensure_proposal_period_mapping, finalize_dag_proposer_signed_block_intent,
    load_dag_block_from_storage, period_block_hash_from_storage, plan_dag_add_block_effects,
    plan_dag_proposer_attempt, plan_dag_proposer_block_construction_from_storage,
    plan_dag_proposer_block_intent, plan_dag_proposer_post_pack, plan_dag_proposer_retry_reset,
    plan_dag_proposer_stale_proof, plan_dag_proposer_tip_selection_from_storage,
    plan_dag_proposer_vdf_wait, plan_dag_proposer_worker_command,
    plan_dag_verify_transaction_query, proposal_period_for_level_from_storage,
    save_dag_block_to_storage, validate_dag_verify_gas,
    validate_dag_verify_transaction_availability, validate_pivot_tips_metadata,
    verify_dag_vdf_sortition_from_block, verify_precheck_from_storage,
    DagManagerBlock as DomainDagManagerBlock,
    DagManagerFinalizationCleanupStoragePayload as DomainDagManagerFinalizationCleanupStoragePayload,
    DagManagerFinalizationPlan as DomainDagManagerFinalizationPlan,
    DagManagerSnapshot as DomainDagManagerSnapshot, DagManagerState,
    DagProposerAttemptInput as DomainDagProposerAttemptInput,
    DagProposerAttemptPlan as DomainDagProposerAttemptPlan,
    DagProposerBlockIntentInput as DomainDagProposerBlockIntentInput,
    DagProposerFrontierFacts as DomainDagProposerFrontierFacts,
    DagProposerSignedBlockIntentInput as DomainDagProposerSignedBlockIntentInput,
    DagProposerStorageBlockConstructionInput as DomainDagProposerStorageBlockConstructionInput,
    DagProposerStorageTipSelectionInput as DomainDagProposerStorageTipSelectionInput,
    DagProposerUnsignedBlockIntent as DomainDagProposerUnsignedBlockIntent,
    DagProposerWorkerCommandInput as DomainDagProposerWorkerCommandInput,
    DagReferenceMetadata as ReferenceMetadata, DagTipGas,
    DagVdfSortitionBlockInput as DomainDagVdfSortitionBlockInput,
    DagVerifyGasInput as DomainDagVerifyGasInput,
    DagVerifyPrecheckStorageInput as DomainDagVerifyPrecheckStorageInput,
    DagVerifyTransactionAvailabilityInput as DomainDagVerifyTransactionAvailabilityInput,
    DagVerifyVdfDposFacts as DomainDagVerifyVdfDposFacts,
};
use rustaxa_consensus::pbft_chain::restore_pbft_chain_from_storage;
use rustaxa_consensus::sortition::{SortitionParams, VdfParams, VrfParams};
use rustaxa_storage::Storage;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_keccak::{Hasher, Keccak};

/// Module-private deterministic plan for advancing DAG finalization state.
///
/// The carrier is produced from a validated anchor transition and contains the
/// finalized count plus the block sets needed to derive Rust-storage counter
/// updates and cleanup. Hash vectors retain domain ordering and may be empty for
/// an empty period. The plan performs no I/O, never crosses CXX, and is returned
/// only after state-transition validation succeeds.
struct DagManagerFinalizationPlan {
    finalized_count: u64,
    counter_update_hashes: Vec<DagHash>,
    expired_hashes: Vec<DagHash>,
    remaining_hashes: Vec<DagHash>,
}

/// Module-private storage-derived counter update for one finalized DAG block.
///
/// `hash` identifies the finalized block, while `level` and `tips_count` are
/// authoritative values read from Rust storage. The value is an intermediate
/// cleanup fact, owns no storage state, and is created only after the referenced
/// block resolves successfully.
#[cfg_attr(not(test), allow(dead_code))]
struct DagFinalizedCounterUpdate {
    hash: [u8; 32],
    level: u64,
    tips_count: u64,
}

/// Module-private cleanup result for a finalized DAG order.
///
/// The carrier combines storage-derived counter updates, expired DAG hashes,
/// and expired transaction hashes selected for Rust-owned deletion. Ordering is
/// preserved for deterministic application and live sidecar reporting. It never
/// crosses CXX directly; storage/decode failures are propagated before it is
/// constructed, and every vector may be empty for an empty transition.
#[cfg_attr(not(test), allow(dead_code))]
struct DagManagerFinalizationCleanupPayload {
    counter_updates: Vec<DagFinalizedCounterUpdate>,
    expired_hashes: Vec<DagHash>,
    remove_transaction_hashes: Vec<DagTransactionHash>,
}

const DAG_VERIFY_SESSION_STATUS_ACTIVE: u8 = 0;
const DAG_VERIFY_SESSION_STATUS_COMPLETE: u8 = 1;
const DAG_VERIFY_SESSION_STATUS_INVALID_REPORT: u8 = 2;
const DAG_VERIFY_SESSION_ACTION_NONE: u8 = 0;
const DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY: u8 = 1;
const DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS: u8 = 2;
const DAG_VERIFY_SESSION_ACTION_VDF_SORTITION: u8 = 3;
const DAG_VERIFY_SESSION_ACTION_GAS: u8 = 4;

const DAG_PROPOSER_SESSION_STATUS_ACTIVE: u8 = 0;
const DAG_PROPOSER_SESSION_STATUS_COMPLETE: u8 = 1;
const DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT: u8 = 2;
const DAG_PROPOSER_SESSION_ACTION_NONE: u8 = 0;
const DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS: u8 = 1;
const DAG_PROPOSER_SESSION_ACTION_START_VDF: u8 = 2;
const DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF: u8 = 3;
const DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP: u8 = 4;
const DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK: u8 = 5;
const DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK: u8 = 6;
const DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS: u8 = 7;

#[derive(Clone)]
enum DagVerifyBlockSessionAction {
    TransactionQuery(Vec<H256>),
    AuthorizationFacts,
    VdfSortition {
        vote_count: u64,
        max_vote_count: u64,
    },
    Gas,
    Complete,
}

/// Ordered Rust-owned cursor for one `DagManager::verifyBlock` call.
///
/// The session owns deterministic validation ordering and terminal reject
/// selection. C++ supplies only requested live facts: transaction
/// materialization counts, FinalChain DPoS/VRF authorization facts, VDF
/// verifier status, and EVM-backed gas-estimation facts.
pub struct DagVerifyBlockSession {
    action: DagVerifyBlockSessionAction,
    proposal_period: u64,
    expected_transactions: u64,
    reject_code: u32,
    sender_eligible_vote_count: u64,
    vdf_sortition_max_vote_count: u64,
    eligibility_status: u8,
    error_code: String,
}

enum DagProposerSessionAction {
    CollectExternalProposalFacts,
    PackTransactions,
    StartVdf,
    StaleProofSleep,
    SignBlock,
    AddBlock,
    Complete,
}

/// Ordered Rust-owned cursor for one `DagBlockProposer::proposeDagBlock` attempt.
///
/// The session owns deterministic proposer stage ordering and retry-cursor
/// updates. C++ still executes external boundaries: live transaction packing,
/// VDF proof work, compatibility sleep, signing/materialization, `addDagBlock`,
/// and network effects owned by downstream executors.
pub struct DagProposerSession {
    action: DagProposerSessionAction,
    begin_input: DagProposerSessionBeginInput,
    observation: DagProposerObservation,
    attempt: DomainDagProposerAttemptPlan,
    retry_key: [u8; 32],
    minimum_vdf_difficulty: u16,
    status: u8,
    reason_code: u32,
    return_value: bool,
    update_retry_state: bool,
    next_last_propose_level: u64,
    next_retry_count: u64,
    record_proposed_block: bool,
    vdf_message: Vec<u8>,
    selected_transaction_hashes: Vec<H256>,
    transaction_gas_estimations: Vec<u64>,
    vdf_rlp: Vec<u8>,
    unsigned_intent: Option<DomainDagProposerUnsignedBlockIntent>,
    signed_intent: Option<rustaxa_consensus::dag::DagProposerSignedBlockIntent>,
    error_code: String,
}

#[derive(Clone)]
struct DagProposerObservation {
    frontier: DomainDagProposerFrontierFacts,
    proposal_period_found: bool,
    proposal_period: u64,
    period_block_hash_found: bool,
    period_block_hash: H256,
    fingerprint: [u8; 32],
}

/// Rust-owned durable retry cursor for one configured DAG proposer wallet.
///
/// The DAG manager runtime keys this by proposer wallet VRF public key. It is
/// read when a proposal session begins and updated atomically with terminal
/// session steps that request retry-state changes.
pub struct DagProposerRetryState {
    last_propose_level: u64,
    retry_count: u64,
    max_retry_count: u64,
}

struct DagManagerRuntimeSyncSnapshot {
    period: u64,
    selected_hashes: Vec<DagHash>,
}

/// Creates a Rust-owned DagManager runtime with direct storage access.
///
/// The runtime owns deterministic graph/index state and a cloned Rust storage
/// handle. C++ callers use it for DagManager persistence so the migration path
/// is `DagManager shim -> Rust DagManager runtime -> rustaxa-storage`, without
/// routing through legacy DagManager storage logic.
pub fn create_dag_manager_runtime_from_storage(
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
    storage: &BridgeStorage,
) -> Result<Box<BridgeDagManagerRuntime>> {
    Ok(Box::new(BridgeDagManagerRuntime {
        state: DagManagerState::new(to_h256(genesis), dag_expiry_limit)?,
        storage: storage.0.clone(),
        next_proposer_session_id: 1,
        proposer_sessions: BTreeMap::new(),
        proposer_retry_states: BTreeMap::new(),
        verify_block_session: None,
    }))
}

impl BridgeDagManagerRuntime {
    /// Rebuilds the in-memory DAG runtime from canonical Rust storage.
    ///
    /// Inputs:
    /// - the runtime's existing Rust storage handle, which owns PBFT-chain head
    ///   recovery and non-finalized DAG block payload rows.
    ///
    /// Outputs:
    /// - replaces the runtime state with a snapshot derived from Rust storage.
    ///
    /// Invariants and edge behavior:
    /// - PBFT period and current anchor come from Rust PBFT-chain storage
    ///   restore, including default-head initialization when the head row is
    ///   absent.
    /// - Non-finalized DAG block facts are decoded from canonical signed DAG
    ///   block RLP bytes in Rust storage; malformed rows are returned as
    ///   bridge errors.
    /// - The previous anchor is not persisted separately in Rust storage today,
    ///   so it is restored to the current anchor. Finalization transitions
    ///   update both anchors through the Rust runtime after startup.
    pub fn dag_manager_runtime_restore_from_storage(&mut self) -> Result<()> {
        let pbft_restore = restore_pbft_chain_from_storage(self.storage.as_ref())
            .context("DAG_RUNTIME_RESTORE_PBFT_HEAD")?;
        let anchor = pbft_restore.head.last_non_null_pbft_dag_anchor_hash;
        let anchor_level = if anchor == H256::zero() {
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
                    domain_dag_manager_block_from_rlp(&block_rlp)
                        .context("DAG_RUNTIME_RESTORE_NON_FINALIZED_BLOCK_DECODE")?,
                );
            }
        }

        let max_level = non_finalized_blocks
            .iter()
            .map(|block| block.level)
            .chain((anchor != H256::zero()).then_some(anchor_level))
            .max()
            .unwrap_or(0);
        let non_finalized_min_difficulty = non_finalized_blocks
            .iter()
            .map(|block| block.difficulty)
            .min()
            .unwrap_or(u32::MAX);
        let dag_expiry_level = max_level.saturating_sub(u64::from(self.state.dag_expiry_limit()));

        self.state
            .rebuild_from_snapshot(DomainDagManagerSnapshot {
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

    /// Adds one accepted DAG block to the in-memory Rust state.
    pub fn dag_manager_runtime_add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.state.add_block(to_domain_block(block))
    }

    /// Plans one add-block execution from Rust-owned runtime graph state.
    ///
    /// Inputs are compact block facts plus public add-block flags. The runtime
    /// derives duplicate, expiry, and pivot/tip availability facts from its
    /// in-memory DAG state before delegating to the pure add-block planner. C++
    /// remains the executor for storage writes, transaction sidecars, events,
    /// gossip, and temporary compatibility-object materialization.
    pub fn dag_manager_runtime_plan_add_block(
        &self,
        input: DagAddBlockRuntimeInput,
    ) -> Result<DagAddBlockEffectPlan> {
        let block_in_state = self.state.has_vertex(H256::from(input.block_hash));
        let block_in_storage =
            dag_block_exists_in_storage(self.storage.as_ref(), H256::from(input.block_hash))
                .context("DAG_RUNTIME_ADD_BLOCK_EXISTS")?;
        let block_exists = if input.save {
            block_in_storage
        } else {
            block_in_state || block_in_storage
        };

        let tips = input
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let pivot_tips = if input.save
            && !block_in_state
            && !block_exists
            && input.block_level >= self.state.dag_expiry_level()
        {
            let pivot = dag_reference_metadata_from_runtime_or_storage(
                &self.state,
                self.storage.as_ref(),
                H256::from(input.pivot),
            )?;
            let tips = tips
                .iter()
                .map(|tip| {
                    dag_reference_metadata_from_runtime_or_storage(
                        &self.state,
                        self.storage.as_ref(),
                        *tip,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            validate_pivot_tips_metadata(input.block_level, pivot, &tips)
        } else {
            rustaxa_consensus::dag::DagPivotTipsValidation {
                ok: true,
                expected_level: input.block_level,
                level_matches: true,
                missing_references: Vec::new(),
            }
        };

        let plan = plan_dag_add_block_effects(rustaxa_consensus::dag::DagAddBlockEffectInput {
            save: input.save,
            proposed: input.proposed,
            block_exists,
            block_level: input.block_level,
            dag_expiry_level: self.state.dag_expiry_level(),
            references_available: pivot_tips.ok,
            missing_references: pivot_tips.missing_references,
        });
        let mut plan = to_bridge_add_block_effect_plan(plan);
        if input.save && block_in_state && !block_in_storage && plan.accepted && !plan.duplicate {
            plan.add_to_graph = false;
            plan.emit_verified = false;
            plan.gossip = false;
        }
        Ok(plan)
    }

    /// Validates pivot/tip availability from Rust runtime state and storage.
    ///
    /// Inputs:
    /// - `block_level`: level declared by the candidate DAG block.
    /// - `pivot` and `tips`: candidate references in legacy block order.
    ///
    /// Output:
    /// - compact reference availability and expected-level facts. Missing
    ///   references are returned as data, not errors, so compatibility callers
    ///   can preserve the public `(bool, missing_hashes)` API without
    ///   materializing C++ `DagBlock` objects.
    ///
    /// Edge behavior:
    /// - storage backend or payload decode failures are bridge errors because
    ///   canonical DAG storage is the authoritative source for persisted block
    ///   metadata in Rust mode.
    pub fn dag_manager_runtime_validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> Result<DagPivotTipsValidation> {
        let pivot = dag_reference_metadata_from_runtime_or_storage(
            &self.state,
            self.storage.as_ref(),
            to_h256(pivot),
        )?;
        let tips = tips
            .into_iter()
            .map(|tip| {
                dag_reference_metadata_from_runtime_or_storage(
                    &self.state,
                    self.storage.as_ref(),
                    H256::from(tip.hash),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let validation = validate_pivot_tips_metadata(block_level, pivot, &tips);

        Ok(DagPivotTipsValidation {
            ok: validation.ok,
            expected_level: validation.expected_level,
            level_matches: validation.level_matches,
            missing_references: to_dag_hashes(validation.missing_references),
        })
    }

    /// Applies one finalized DAG order directly to Rust state and advances period/anchor.
    ///
    /// Inputs:
    /// - `new_anchor`: hash of the new anchor block, or zero for an empty
    ///   PBFT period without a DAG anchor transition.
    /// - `new_anchor_level`: storage-resolved anchor level for non-empty anchors.
    /// - `new_period`: expected to be `state.period + 1`.
    /// - `finalized_order`: hashes finalized in this order transition.
    ///
    /// Output:
    /// - deterministic finalization plan including unique finalized count and side-effect hashes.
    #[cfg(test)]
    fn dag_manager_runtime_set_finalized_order(
        &mut self,
        new_anchor: [u8; 32],
        new_anchor_level: u64,
        new_period: u64,
        finalized_order: Vec<DagHash>,
    ) -> Result<DagManagerFinalizationPlan> {
        let new_anchor = to_h256(&new_anchor);
        if new_anchor == H256::zero() {
            self.state
                .advance_empty_period(new_period)
                .context("DAG_RUNTIME_ADVANCE_EMPTY_PERIOD")?;
            return Ok(DagManagerFinalizationPlan {
                finalized_count: 0,
                counter_update_hashes: Vec::new(),
                expired_hashes: Vec::new(),
                remaining_hashes: to_dag_hashes(
                    self.state
                        .non_finalized_blocks()
                        .values()
                        .flatten()
                        .copied()
                        .collect(),
                ),
            });
        }

        let finalized_order = finalized_order
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        self.state
            .set_finalized_order(new_anchor, new_period, &finalized_order, new_anchor_level)
            .context("DAG_RUNTIME_SET_FINALIZED_ORDER")
            .map(to_bridge_finalization_plan)
    }

    /// Builds storage-backed cleanup facts for a finalized DAG order plan.
    ///
    /// This method is a test-only convenience wrapper for callers that already
    /// have a full `DagManagerFinalizationPlan`.
    #[cfg(test)]
    fn dag_manager_runtime_finalization_cleanup_payload(
        &self,
        plan: DagManagerFinalizationPlan,
    ) -> Result<DagManagerFinalizationCleanupPayload> {
        let DagManagerFinalizationPlan {
            finalized_count: _,
            counter_update_hashes,
            expired_hashes,
            remaining_hashes,
        } = plan;

        let counter_update_hashes = counter_update_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let expired_hashes = expired_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let remaining_hashes = remaining_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let payload = collect_finalization_cleanup_from_storage(
            self.storage.as_ref(),
            &counter_update_hashes,
            &expired_hashes,
            &remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_CLEANUP_BUILD_FAILED")?;

        Ok(to_bridge_finalization_cleanup_payload(payload))
    }

    /// Applies one finalized DAG order through Rust state and Rust storage.
    ///
    /// Inputs:
    /// - `new_anchor`: the new finalized DAG anchor, or zero for an empty PBFT
    ///   period without a DAG anchor transition.
    /// - `new_period`: expected to be the next runtime period.
    /// - `finalized_order`: finalized DAG block hashes in legacy order.
    ///
    /// Output:
    /// - finalized block count plus the live C++ side-effect facts that cannot
    ///   move yet: expired block hashes for `seen_blocks_` cleanup and expired
    ///   transaction hashes for transaction-manager sidecar cleanup. Returned
    ///   transaction hashes have already been removed from Rust-owned storage.
    ///
    /// Behavior:
    /// - resolves the anchor level from Rust storage when the anchor is nonzero
    /// - computes finalization on a candidate state before mutating this runtime
    /// - preflights storage-backed cleanup facts before persistent writes
    /// - updates Rust DAG counters, removes expired DAG blocks, and removes
    ///   expired non-finalized transaction payloads through `rustaxa-storage`
    /// - commits the candidate state only after the Rust-owned storage writes
    ///   complete
    pub fn dag_manager_runtime_apply_finalized_order(
        &mut self,
        new_anchor: [u8; 32],
        new_period: u64,
        finalized_order: Vec<DagHash>,
    ) -> Result<DagManagerFinalizationApplyPayload> {
        let new_anchor = H256::from(new_anchor);
        let mut candidate_state = self.state.clone();

        let plan = if new_anchor == H256::zero() {
            candidate_state
                .advance_empty_period(new_period)
                .context("DAG_RUNTIME_ADVANCE_EMPTY_PERIOD")?;
            DagManagerFinalizationPlan {
                finalized_count: 0,
                counter_update_hashes: Vec::new(),
                expired_hashes: Vec::new(),
                remaining_hashes: to_dag_hashes(
                    candidate_state
                        .non_finalized_blocks()
                        .values()
                        .flatten()
                        .copied()
                        .collect(),
                ),
            }
        } else {
            let anchor_level = self
                .storage
                .dag()
                .by_hash(new_anchor)
                .with_context(|| format!("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK: {new_anchor:?}"))?
                .level;
            let finalized_order = finalized_order
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect::<Vec<_>>();
            candidate_state
                .set_finalized_order(new_anchor, new_period, &finalized_order, anchor_level)
                .context("DAG_RUNTIME_SET_FINALIZED_ORDER")
                .map(to_bridge_finalization_plan)?
        };

        let finalized_count = plan.finalized_count;
        let counter_update_hashes = plan
            .counter_update_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let expired_hashes = plan
            .expired_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let remaining_hashes = plan
            .remaining_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let cleanup = apply_finalization_cleanup_from_storage(
            self.storage.as_ref(),
            &counter_update_hashes,
            &expired_hashes,
            &remaining_hashes,
        )
        .context("DAG_RUNTIME_FINALIZATION_STORAGE_APPLY")?;

        self.state = candidate_state;
        let cleanup = to_bridge_finalization_cleanup_payload(cleanup);

        Ok(DagManagerFinalizationApplyPayload {
            finalized_count,
            expired_hashes: cleanup.expired_hashes,
            remove_transaction_hashes: cleanup.remove_transaction_hashes,
        })
    }

    /// Returns a one-shot sync snapshot containing the current period and the
    /// deterministic selection of non-finalized block hashes that are not in
    /// `known_hashes`.
    fn dag_manager_runtime_non_finalized_sync_snapshot(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> DagManagerRuntimeSyncSnapshot {
        let known_hashes = known_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        DagManagerRuntimeSyncSnapshot {
            period: self.state.period(),
            selected_hashes: to_dag_hashes(
                self.state
                    .select_non_finalized_hashes_excluding_known(&known_hashes),
            ),
        }
    }

    /// Builds one-shot non-finalized DAG sync materialization data through
    /// Rust storage only.
    ///
    /// Returns selected block RLP payloads plus a de-duplicated transaction lookup
    /// list that preserves the sync snapshot block order and per-block
    /// transaction order.
    pub fn dag_manager_runtime_non_finalized_sync_payload(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> Result<DagManagerNonFinalizedSyncPayload> {
        let snapshot = self.dag_manager_runtime_non_finalized_sync_snapshot(known_hashes);
        let selected_hashes = snapshot
            .selected_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        let payload = collect_non_finalized_sync_payload_from_storage(
            self.storage.as_ref(),
            &selected_hashes,
        )
        .context("DAG_RUNTIME_SYNC_STORAGE_PAYLOAD")?;

        Ok(DagManagerNonFinalizedSyncPayload {
            period: snapshot.period,
            blocks: to_bridge_sync_blocks(payload.blocks),
            transactions: to_bridge_transaction_rlp_lookups(payload.transactions),
        })
    }

    /// Computes deterministic DAG order for a target anchor.
    pub fn dag_manager_runtime_compute_order(&self, anchor: &[u8; 32]) -> DagOrder {
        match self.state.compute_order(to_h256(anchor)) {
            Some(hashes) => DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            },
            None => DagOrder {
                found: false,
                hashes: Vec::new(),
            },
        }
    }

    /// Returns non-finalized DAG block hashes excluding already-known hashes.
    ///
    /// This method applies the deterministic `DagManagerState` selection helper at
    /// the runtime boundary so C++ can request next-sync candidates without
    /// reordering responsibility.
    #[cfg(test)]
    fn dag_manager_runtime_select_non_finalized_hashes(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> Vec<DagHash> {
        let known_hashes = known_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect::<Vec<_>>();
        to_dag_hashes(
            self.state
                .select_non_finalized_hashes_excluding_known(&known_hashes),
        )
    }

    /// Returns the current Rust-owned DAG frontier.
    pub fn dag_manager_runtime_frontier(&self) -> DagFrontier {
        to_bridge_frontier(self.state.frontier())
    }

    /// Opens a runtime-owned DAG proposer cursor for one proposal attempt.
    ///
    /// The cursor atomically derives the frontier, proposal period, period hash, and an observation fingerprint from
    /// Rust-owned runtime state. A valid observation first requests only FinalChain authorization and sortition facts;
    /// the planner runs after those facts arrive and only if the observation still matches. Later stages advance through
    /// explicit external reports or runtime-derived VDF frontier polls.
    pub fn begin_proposer_session(&mut self, input: DagProposerSessionBeginInput) -> Result<u64> {
        let retry_key = input.wallet_vrf_public_key;
        let observation = self.proposer_observation()?;
        let attempt = placeholder_attempt(&observation, &input);
        let action = if observation.proposal_period_found {
            DagProposerSessionAction::CollectExternalProposalFacts
        } else {
            DagProposerSessionAction::Complete
        };
        let status = if matches!(action, DagProposerSessionAction::Complete) {
            DAG_PROPOSER_SESSION_STATUS_COMPLETE
        } else {
            DAG_PROPOSER_SESSION_STATUS_ACTIVE
        };
        let session_id = self.next_proposer_session_id;
        self.next_proposer_session_id = self.next_proposer_session_id.saturating_add(1);
        ensure!(
            !self.proposer_sessions.contains_key(&session_id),
            "DAG_PROPOSER_SESSION_ID_COLLISION"
        );

        self.proposer_sessions.insert(
            session_id,
            DagProposerSession {
                action,
                status,
                begin_input: input,
                observation,
                retry_key,
                reason_code: if attempt.proposal_period_found {
                    rustaxa_consensus::dag::DAG_PROPOSER_REASON_OK
                } else {
                    rustaxa_consensus::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD
                },
                return_value: false,
                update_retry_state: attempt.update_retry_state,
                next_last_propose_level: attempt.next_last_propose_level,
                next_retry_count: attempt.next_retry_count,
                record_proposed_block: false,
                minimum_vdf_difficulty: 0,
                vdf_message: Vec::new(),
                selected_transaction_hashes: Vec::new(),
                transaction_gas_estimations: Vec::new(),
                vdf_rlp: Vec::new(),
                unsigned_intent: None,
                signed_intent: None,
                attempt,
                error_code: String::new(),
            },
        );
        Ok(session_id)
    }

    fn proposer_observation(&self) -> Result<DagProposerObservation> {
        let frontier = self.state.proposer_frontier_facts();
        let proposal_period =
            proposal_period_for_level_from_storage(self.storage.as_ref(), frontier.propose_level)?;
        let period_block_hash = if proposal_period.found {
            let lookup =
                period_block_hash_from_storage(self.storage.as_ref(), proposal_period.period)?;
            if !lookup.found && proposal_period.period == 0 {
                rustaxa_consensus::dag::DagHashStorageLookup {
                    found: true,
                    hash: H256::zero(),
                }
            } else {
                lookup
            }
        } else {
            rustaxa_consensus::dag::DagHashStorageLookup {
                found: false,
                hash: H256::zero(),
            }
        };
        let fingerprint = proposer_observation_fingerprint(
            &frontier,
            proposal_period.found,
            proposal_period.period,
            period_block_hash.found,
            period_block_hash.hash,
        );
        Ok(DagProposerObservation {
            frontier,
            proposal_period_found: proposal_period.found,
            proposal_period: proposal_period.period,
            period_block_hash_found: period_block_hash.found,
            period_block_hash: period_block_hash.hash,
            fingerprint,
        })
    }

    /// Returns the ghost path from a source block.
    pub fn dag_manager_runtime_ghost_path(&self, source: &[u8; 32]) -> Vec<DagHash> {
        to_dag_hashes(self.state.ghost_path(to_h256(source)))
    }

    /// Returns the ghost path rooted at the current anchor.
    pub fn dag_manager_runtime_anchor_ghost_path(&self) -> Vec<DagHash> {
        to_dag_hashes(self.state.anchor_ghost_path())
    }

    /// Renders the selected Rust-owned DAG graph as GraphViz dot text.
    pub fn dag_manager_runtime_graphviz_dot(&self, pivot_tree: bool) -> String {
        self.state.graphviz_dot(pivot_tree)
    }

    /// Returns current in-memory DAG vertex count.
    pub fn dag_manager_runtime_vertex_count(&self) -> usize {
        self.state.vertex_count()
    }

    /// Returns current in-memory DAG edge count.
    pub fn dag_manager_runtime_edge_count(&self) -> usize {
        self.state.edge_count()
    }

    /// Returns current max DAG level mirrored in Rust state.
    pub fn dag_manager_runtime_max_level(&self) -> u64 {
        self.state.max_level()
    }

    /// Returns latest finalized period mirrored in Rust state.
    pub fn dag_manager_runtime_latest_period(&self) -> u64 {
        self.state.period()
    }

    /// Returns old/current anchors mirrored in Rust state.
    pub fn dag_manager_runtime_anchors(&self) -> DagManagerAnchors {
        let (old_anchor, anchor) = self.state.anchors();
        DagManagerAnchors {
            old_anchor: old_anchor.into(),
            anchor: anchor.into(),
        }
    }

    /// Returns configured DAG expiry limit.
    pub fn dag_manager_runtime_dag_expiry_limit(&self) -> u32 {
        self.state.dag_expiry_limit()
    }

    /// Returns current DAG expiry level.
    pub fn dag_manager_runtime_dag_expiry_level(&self) -> u64 {
        self.state.dag_expiry_level()
    }

    /// Returns current non-finalized DAG block index by level.
    pub fn dag_manager_runtime_non_finalized_blocks(&self) -> Vec<DagLevelHashes> {
        self.state
            .non_finalized_blocks()
            .iter()
            .map(|(level, hashes)| DagLevelHashes {
                level: *level,
                hashes: to_dag_hashes(hashes.iter().copied().collect()),
            })
            .collect()
    }

    /// Returns non-finalized level and block counts.
    pub fn dag_manager_runtime_non_finalized_blocks_size(&self) -> DagManagerNonFinalizedSize {
        let (levels, blocks) = self.state.non_finalized_blocks_size();
        DagManagerNonFinalizedSize {
            levels: levels as u64,
            blocks: blocks as u64,
        }
    }

    /// Returns current non-finalized minimum difficulty.
    pub fn dag_manager_runtime_non_finalized_min_difficulty(&self) -> u32 {
        self.state.non_finalized_min_difficulty()
    }

    /// Returns whether the Rust DAG runtime knows a block in live graph state
    /// or canonical Rust storage.
    ///
    /// This is the Rust-mode authority for `DagManager::isDagBlockKnown`.
    /// Compatibility caches may still retain materialized `DagBlock` sidecars
    /// for public/test/event edges, but they do not decide membership.
    pub fn dag_manager_runtime_is_block_known(&self, hash: &[u8; 32]) -> Result<bool> {
        let hash = to_h256(hash);
        Ok(
            self.state.has_vertex(hash)
                || dag_block_exists_in_storage(self.storage.as_ref(), hash)?,
        )
    }

    /// Loads per-tip gas facts directly from Rust storage for DAG block verification.
    ///
    /// Inputs:
    /// - `tips`: candidate tip hashes in the original block order.
    ///
    /// Outputs:
    /// - one `DagTipGas` per input hash. Missing tips are returned as
    ///   `found = false` so the Rust verification session can select the
    ///   legacy `MissingTip` status without C++ materializing `DagBlock`
    ///   objects or deriving gas facts from compatibility caches.
    ///
    /// Edge behavior:
    /// - storage backend and decode failures are bridge errors because they
    ///   indicate corrupt or unavailable canonical DAG payloads rather than a
    ///   consensus-invalid missing tip.
    pub fn dag_manager_runtime_tip_gas_estimations(
        &self,
        tips: Vec<DagHash>,
    ) -> Result<Vec<crate::ffi::rustaxa_ffi::DagTipGas>> {
        tips.into_iter()
            .map(|tip| {
                let hash = H256::from(tip.hash);
                if self
                    .storage
                    .dag()
                    .by_hash_rlp_optional(hash)
                    .context("DAG_RUNTIME_TIP_GAS_LOOKUP")?
                    .is_none()
                {
                    return Ok(crate::ffi::rustaxa_ffi::DagTipGas {
                        found: false,
                        gas_estimation: 0,
                    });
                }

                let block = self
                    .storage
                    .dag()
                    .by_hash(hash)
                    .context("DAG_RUNTIME_TIP_GAS_DECODE")?;
                Ok(crate::ffi::rustaxa_ffi::DagTipGas {
                    found: true,
                    gas_estimation: block.gas_estimation,
                })
            })
            .collect()
    }

    /// Loads canonical DAG block RLP from Rust storage.
    pub fn dag_manager_runtime_load_block(&self, hash: &[u8; 32]) -> Result<DagBlockLookup> {
        let lookup = load_dag_block_from_storage(self.storage.as_ref(), to_h256(hash))?;
        Ok(DagBlockLookup {
            found: lookup.found,
            block_rlp: lookup.block_rlp,
        })
    }

    /// Persists one non-finalized DAG block through Rust storage and updates
    /// persistent DAG counters atomically.
    pub fn dag_manager_runtime_save_block(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
        block_rlp: Vec<u8>,
    ) -> Result<()> {
        save_dag_block_to_storage(
            self.storage.as_ref(),
            to_h256(hash),
            level,
            tips_count,
            &block_rlp,
        )
    }

    /// Selects proposer tips with tip metadata loaded from Rust storage.
    ///
    /// This backs the legacy `DagBlockProposer::selectDagBlockTips` compatibility API. Rust owns storage metadata
    /// loading, sender recovery, missing-tip skipping, proposer grouping, level ordering, gas-limit enforcement, and
    /// max-tip enforcement. C++ only supplies candidate hashes and materializes the returned hash list.
    pub fn dag_manager_runtime_plan_proposal_tip_selection(
        &self,
        input: DagProposerStorageTipSelectionInput,
    ) -> Result<DagProposerTipSelectionPlan> {
        let plan = plan_dag_proposer_tip_selection_from_storage(
            self.storage.as_ref(),
            DomainDagProposerStorageTipSelectionInput {
                frontier_tips: input
                    .frontier_tips
                    .into_iter()
                    .map(|hash| H256::from(hash.hash))
                    .collect(),
                gas_limit: input.gas_limit,
                max_tips: input.max_tips,
            },
        )?;
        Ok(DagProposerTipSelectionPlan {
            selected_tips: plan
                .selected
                .into_iter()
                .map(|hash| DagHash { hash: hash.0 })
                .collect(),
            skipped_missing_tips: plan.skipped_missing,
        })
    }

    /// Ensures the proposal-period mapping exists for `level`.
    ///
    /// Returns true when a mapping write was required and false when the
    /// existing lookup already resolves to `period`.
    pub fn dag_manager_runtime_ensure_proposal_period_mapping(
        &self,
        level: u64,
        period: u64,
    ) -> Result<bool> {
        ensure_proposal_period_mapping(self.storage.as_ref(), level, period)
    }

    /// Resolves the finalized proposal period for a DAG level through the
    /// runtime-owned Rust storage handle.
    ///
    /// Inputs and outputs mirror `DbStorage::getProposalPeriodForDagLevel`:
    /// Rust storage returns the first persisted `(level -> period)` row at or
    /// after the requested level. Missing rows are reported as `found = false`
    /// instead of errors, while malformed storage/backend failures are errors.
    /// Returns the canonical PBFT block hash for finalized `period`.
    ///
    /// The hash is derived from item 0 of the canonical `PeriodData` RLP stored
    /// in Rust storage, matching legacy `DbStorage::getPeriodBlockHash`. Missing
    /// period data returns `found = false`; corrupt period data is a bridge
    /// error so C++ verification can reject rather than silently use bad facts.
    pub fn dag_manager_runtime_period_block_hash(&self, period: u64) -> Result<HashLookup> {
        let lookup = period_block_hash_from_storage(self.storage.as_ref(), period)?;
        Ok(HashLookup {
            found: lookup.found,
            hash: lookup.hash.into(),
        })
    }

    /// Reads persisted DAG counters directly from Rust storage.
    pub fn dag_manager_runtime_persistence_counters(&self) -> Result<DagPersistenceCounters> {
        let counters = dag_persistence_counters_from_storage(self.storage.as_ref())?;
        Ok(DagPersistenceCounters {
            dag_blocks: counters.dag_blocks,
            dag_edges: counters.dag_edges,
        })
    }

    /// Opens a runtime-owned ordered `DagManager::verifyBlock` session.
    ///
    /// The runtime performs storage-backed prechecks immediately, then returns
    /// either a terminal reject/complete step or a transaction-query request.
    /// Later advancement happens only through explicit live-fact reports from
    /// the C++ executor boundary.
    pub fn begin_verify_block_session(&mut self, input: DagVerifyBlockSessionInput) -> Result<()> {
        let tips = input
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let precheck = verify_precheck_from_storage(
            self.storage.as_ref(),
            DomainDagVerifyPrecheckStorageInput {
                block_level: input.block_level,
                pivot: H256::from(input.pivot),
                tips,
                dag_expiry_level: self.state.dag_expiry_level(),
            },
        )
        .context("DAG_RUNTIME_VERIFY_SESSION_PRECHECK")?;

        let expected_transactions = input.block_transaction_hashes.len() as u64;
        let action = if precheck.continue_validation {
            let block_transaction_hashes = to_transaction_hashes(input.block_transaction_hashes);
            let supplied_transaction_hashes =
                to_transaction_hashes(input.supplied_transaction_hashes);
            let query_plan = plan_dag_verify_transaction_query(
                &block_transaction_hashes,
                &supplied_transaction_hashes,
            );
            DagVerifyBlockSessionAction::TransactionQuery(query_plan.query_hashes)
        } else {
            DagVerifyBlockSessionAction::Complete
        };

        self.verify_block_session = Some(DagVerifyBlockSession {
            action,
            proposal_period: precheck.proposal_period,
            expected_transactions,
            reject_code: precheck.reject_code,
            sender_eligible_vote_count: 0,
            vdf_sortition_max_vote_count: 0,
            eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
            error_code: String::new(),
        });

        Ok(())
    }
}

/// Opens a DAG block verification cursor inside the long-lived DAG manager runtime.
///
/// Inputs:
/// - `runtime`: the DAG manager runtime that owns graph state, storage, and the temporary verification cursor.
/// - `input`: compact block facts and supplied transaction hashes for one `DagManager::verifyBlock` call.
///
/// Outputs:
/// - Replaces any previous runtime verification cursor. C++ drives the cursor with
///   `dag_manager_runtime_verify_block_session_next` and report functions.
///
/// Invariants and edge behavior:
/// - The verification cursor is DAG-manager implementation state and is not exported as a standalone CXX handle.
/// - Starting a new verification replaces any incomplete previous cursor, matching the legacy per-call allocation
///   behavior.
pub fn dag_manager_runtime_begin_verify_block_session(
    runtime: &mut BridgeDagManagerRuntime,
    input: DagVerifyBlockSessionInput,
) -> Result<()> {
    runtime.begin_verify_block_session(input)
}

/// Returns the next requested action for the runtime-owned DAG verification cursor.
pub fn dag_manager_runtime_verify_block_session_next(
    runtime: &mut BridgeDagManagerRuntime,
) -> DagVerifyBlockSessionStep {
    let Some(session) = runtime.verify_block_session.as_ref() else {
        return verify_block_session_not_started_step();
    };
    verify_block_session_step(session)
}

/// Reports resolved transaction availability to the runtime-owned DAG verification cursor.
pub fn dag_manager_runtime_verify_block_session_report_transactions(
    runtime: &mut BridgeDagManagerRuntime,
    report: DagVerifyBlockTransactionReport,
) -> DagVerifyBlockSessionStep {
    let Some(session) = runtime.verify_block_session.as_mut() else {
        return verify_block_session_not_started_step();
    };
    if !matches!(
        session.action,
        DagVerifyBlockSessionAction::TransactionQuery(_)
    ) {
        return invalid_verify_block_report(
            session,
            "DAG_VERIFY_SESSION_UNEXPECTED_TRANSACTION_REPORT",
        );
    }

    let availability =
        validate_dag_verify_transaction_availability(DomainDagVerifyTransactionAvailabilityInput {
            expected_transactions: session.expected_transactions,
            resolved_transactions: report.resolved_transactions,
        });
    if !availability.continue_validation {
        return complete_verify_block_session(session, availability.reject_code);
    }

    session.action = DagVerifyBlockSessionAction::AuthorizationFacts;
    verify_block_session_step(session)
}

/// Reports FinalChain authorization facts to the runtime-owned DAG verification cursor.
pub fn dag_manager_runtime_verify_block_session_report_authorization(
    runtime: &mut BridgeDagManagerRuntime,
    report: DagVerifyBlockAuthorizationReport,
) -> DagVerifyBlockSessionStep {
    let Some(session) = runtime.verify_block_session.as_mut() else {
        return verify_block_session_not_started_step();
    };
    if !matches!(
        session.action,
        DagVerifyBlockSessionAction::AuthorizationFacts
    ) {
        return invalid_verify_block_report(
            session,
            "DAG_VERIFY_SESSION_UNEXPECTED_AUTHORIZATION_REPORT",
        );
    }

    session.sender_eligible_vote_count = report.sender_eligible_vote_count;
    session.vdf_sortition_max_vote_count = report.vdf_sortition_max_vote_count;
    session.eligibility_status = report.eligibility_status;

    let dpos_status = if report.eligibility_status
        == rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    {
        rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    } else {
        rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED
    };
    let decision = decide_dag_verify_vdf_dpos_authorization(DomainDagVerifyVdfDposFacts {
        vrf_key_found: report.vrf_key_found,
        sender_eligible_vote_count: report.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: report.vdf_sortition_max_vote_count,
        vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_NOT_CHECKED,
        dpos_status,
    });
    if !decision.continue_validation {
        return complete_verify_block_session(session, decision.reject_code);
    }

    session.action = DagVerifyBlockSessionAction::VdfSortition {
        vote_count: decision.vote_count,
        max_vote_count: decision.max_vote_count,
    };
    verify_block_session_step(session)
}

/// Reports VDF verification status to the runtime-owned DAG verification cursor.
pub fn dag_manager_runtime_verify_block_session_report_vdf(
    runtime: &mut BridgeDagManagerRuntime,
    report: DagVerifyBlockVdfReport,
) -> DagVerifyBlockSessionStep {
    let Some(session) = runtime.verify_block_session.as_mut() else {
        return verify_block_session_not_started_step();
    };
    if !matches!(
        session.action,
        DagVerifyBlockSessionAction::VdfSortition { .. }
    ) {
        return invalid_verify_block_report(session, "DAG_VERIFY_SESSION_UNEXPECTED_VDF_REPORT");
    }

    let dpos_status = if session.eligibility_status
        == rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    {
        rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
    } else {
        rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED
    };
    let vdf_decision = decide_dag_verify_vdf_dpos_authorization(DomainDagVerifyVdfDposFacts {
        vrf_key_found: true,
        sender_eligible_vote_count: session.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: session.vdf_sortition_max_vote_count,
        vdf_status: report.vdf_status,
        dpos_status,
    });
    if !vdf_decision.continue_validation {
        return complete_verify_block_session(session, vdf_decision.reject_code);
    }

    let dpos_decision = decide_dag_verify_vdf_dpos_authorization(DomainDagVerifyVdfDposFacts {
        vrf_key_found: true,
        sender_eligible_vote_count: session.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: session.vdf_sortition_max_vote_count,
        vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
        dpos_status: session.eligibility_status,
    });
    if !dpos_decision.continue_validation {
        return complete_verify_block_session(session, dpos_decision.reject_code);
    }

    session.action = DagVerifyBlockSessionAction::Gas;
    verify_block_session_step(session)
}

/// Reports gas facts to the runtime-owned DAG verification cursor.
pub fn dag_manager_runtime_verify_block_session_report_gas(
    runtime: &mut BridgeDagManagerRuntime,
    report: DagVerifyBlockGasReport,
) -> DagVerifyBlockSessionStep {
    let Some(session) = runtime.verify_block_session.as_mut() else {
        return verify_block_session_not_started_step();
    };
    if !matches!(session.action, DagVerifyBlockSessionAction::Gas) {
        return invalid_verify_block_report(session, "DAG_VERIFY_SESSION_UNEXPECTED_GAS_REPORT");
    }

    let result = validate_dag_verify_gas(DomainDagVerifyGasInput {
        block_gas_estimation: report.block_gas_estimation,
        estimated_transactions_weight: report.estimated_transactions_weight,
        dag_gas_limit: report.dag_gas_limit,
        pbft_gas_limit: report.pbft_gas_limit,
        tip_gas_estimations: report
            .tip_gas_estimations
            .into_iter()
            .map(|tip| DagTipGas {
                found: tip.found,
                gas_estimation: tip.gas_estimation,
            })
            .collect(),
    });
    complete_verify_block_session(session, result.reject_code)
}

/// Opens a DAG proposal cursor inside the long-lived DAG manager runtime.
///
/// Inputs:
/// - `runtime`: the DAG manager runtime that owns graph state, storage, and proposal cursors.
/// - `input`: wallet/configuration and live transaction-pressure facts for one attempt. Frontier and proposal-period
///   facts are derived from the runtime and are never accepted from C++.
///
/// Outputs:
/// - Returns the runtime-local cursor id that C++ must pass to `dag_manager_runtime_proposer_session_next` and report
///   functions.
///
/// Invariants and edge behavior:
/// - Proposal cursors are DAG-manager implementation state and are not exported as standalone CXX handles.
/// - Multiple wallets may hold active proposal cursors concurrently; each cursor advances only through its returned id.
/// - A changed runtime observation terminates before planner or retry-state mutation.
/// - Terminal cursors are removed after their terminal step is observed.
pub fn dag_manager_runtime_begin_proposer_session(
    runtime: &mut BridgeDagManagerRuntime,
    input: DagProposerSessionBeginInput,
) -> Result<u64> {
    runtime.begin_proposer_session(input)
}

/// Removes a runtime-owned DAG proposal cursor without applying retry-state effects.
///
/// Inputs: `runtime` owns the cursor registry and `session_id` identifies the cursor to remove.
/// Output: `true` only when a live cursor was removed. Missing, already-terminal, and previously aborted ids return
/// `false`, making cleanup idempotent and safe during exception unwinding.
/// Invariants and edge behavior: abort never creates or updates retry state, never runs a planner, and never reports an
/// error.
pub fn dag_manager_runtime_abort_proposer_session(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
) -> bool {
    runtime.proposer_sessions.remove(&session_id).is_some()
}

/// Returns the current requested action for a runtime-owned DAG proposal cursor.
///
/// Inputs: `runtime` owns the cursor and `session_id` selects it. Output is a complete executor instruction snapshot;
/// active calls do not advance the cursor. A terminal step removes the cursor, and a missing id returns an
/// `INVALID_REPORT` step with no retry effects.
pub fn dag_manager_runtime_proposer_session_next(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
) -> DagProposerSessionStep {
    let Some(session) = runtime.proposer_sessions.get(&session_id) else {
        return dag_proposer_session_not_started_step();
    };
    let step = dag_proposer_session_step(session);
    finish_dag_proposer_session_step(runtime, session_id, step)
}

/// Accepts FinalChain authorization and sortition facts for the runtime-derived proposal period.
///
/// Inputs: `session_id` must name a cursor requesting external facts; `report` contains only the external facts Rust
/// cannot derive from the DAG runtime. Output is the next packing or terminal instruction.
/// Invariants and edge behavior: Rust recomputes the observation before planning. A changed observation terminates the
/// cursor without retry mutation; an out-of-order report returns an invalid terminal step. Storage/decode/planner errors
/// return `Err` and remove the cursor before returning, so no fallible path leaves an unreachable live session.
pub fn dag_manager_runtime_proposer_session_report_external_facts(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    report: DagProposerExternalProposalFactsReport,
) -> Result<DagProposerSessionStep> {
    let Some(session) = runtime.proposer_sessions.get(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(
        session.action,
        DagProposerSessionAction::CollectExternalProposalFacts
    ) {
        let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
        let step = invalid_dag_proposer_report(
            session,
            "DAG_PROPOSER_SESSION_UNEXPECTED_EXTERNAL_FACTS_REPORT",
        );
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }

    let current = match runtime.proposer_observation() {
        Ok(current) => current,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let original_fingerprint = runtime.proposer_sessions[&session_id]
        .observation
        .fingerprint;
    if current.fingerprint != original_fingerprint {
        let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
        session.action = DagProposerSessionAction::Complete;
        session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
        session.reason_code = rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION;
        session.error_code = "DAG_PROPOSER_SESSION_STALE_OBSERVATION".to_owned();
        let step = dag_proposer_session_step(session);
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }

    let session = &runtime.proposer_sessions[&session_id];
    let retry = runtime.proposer_retry_states.get(&session.retry_key);
    let last_propose_level = retry.map_or(0, |state| state.last_propose_level);
    let retry_count = retry.map_or(0, |state| state.retry_count);
    let input = domain_attempt_input(
        &session.begin_input,
        &session.observation,
        report,
        last_propose_level,
        retry_count,
    );
    let minimum_vdf_difficulty = input.sortition_params.vdf.difficulty_min;
    let attempt = match plan_dag_proposer_attempt(input) {
        Ok(attempt) => attempt,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let action = if attempt.action == rustaxa_consensus::dag::DAG_PROPOSER_ACTION_CONTINUE {
        DagProposerSessionAction::PackTransactions
    } else {
        DagProposerSessionAction::Complete
    };
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    runtime
        .proposer_retry_states
        .entry(session.retry_key)
        .or_insert(DagProposerRetryState {
            last_propose_level,
            retry_count,
            max_retry_count: session.begin_input.max_retry_count,
        })
        .max_retry_count = session.begin_input.max_retry_count;
    session.status = if matches!(action, DagProposerSessionAction::Complete) {
        DAG_PROPOSER_SESSION_STATUS_COMPLETE
    } else {
        DAG_PROPOSER_SESSION_STATUS_ACTIVE
    };
    session.action = action;
    session.reason_code = attempt.reason_code;
    session.update_retry_state = attempt.update_retry_state;
    session.next_last_propose_level = attempt.next_last_propose_level;
    session.next_retry_count = attempt.next_retry_count;
    session.minimum_vdf_difficulty = minimum_vdf_difficulty;
    session.attempt = attempt;
    let step = dag_proposer_session_step(session);
    Ok(finish_dag_proposer_session_step(runtime, session_id, step))
}

/// Reports live transaction-packing results to the runtime-owned DAG proposal cursor.
pub fn dag_manager_runtime_proposer_session_report_transactions(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    report: DagProposerTransactionPackReport,
) -> DagProposerSessionStep {
    let step = {
        let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
            return dag_proposer_session_not_started_step();
        };
        if !matches!(session.action, DagProposerSessionAction::PackTransactions) {
            invalid_dag_proposer_report(
                session,
                "DAG_PROPOSER_SESSION_UNEXPECTED_TRANSACTION_REPORT",
            )
        } else {
            let post_pack =
                plan_dag_proposer_post_pack(rustaxa_consensus::dag::DagProposerPostPackInput {
                    proposal_level: session.attempt.proposal_level,
                    network_throttled: report.network_throttled,
                    packed_transaction_count: report.transaction_hashes.len() as u64,
                });
            session.reason_code = post_pack.reason_code;
            session.update_retry_state = post_pack.update_retry_state;
            session.next_last_propose_level = post_pack.next_last_propose_level;
            session.next_retry_count = post_pack.next_retry_count;

            if post_pack.action != rustaxa_consensus::dag::DAG_PROPOSER_ACTION_CONTINUE {
                session.action = DagProposerSessionAction::Complete;
                session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
                session.return_value = false;
                dag_proposer_session_step(session)
            } else if report.transaction_hashes.len() != report.transaction_gas_estimations.len() {
                invalid_dag_proposer_report(
                    session,
                    "DAG_PROPOSER_SESSION_TRANSACTION_REPORT_LENGTH_MISMATCH",
                )
            } else {
                session.selected_transaction_hashes = report
                    .transaction_hashes
                    .into_iter()
                    .map(|hash| H256::from(hash.hash))
                    .collect();
                session.transaction_gas_estimations = report.transaction_gas_estimations;
                session.vdf_message = construct_dag_vdf_message(
                    session.attempt.frontier.pivot,
                    &session.selected_transaction_hashes,
                );
                session.action = DagProposerSessionAction::StartVdf;
                dag_proposer_session_step(session)
            }
        }
    };
    finish_dag_proposer_session_step(runtime, session_id, step)
}

/// Polls whether the runtime-owned proposal cursor should cancel its in-flight VDF.
///
/// The current proposal level is derived from the Rust DAG frontier. A matching active VDF remains active; sufficient
/// frontier advancement returns a terminal cancel step with retry-reset facts. Missing or out-of-order ids return an
/// invalid-report step.
pub fn dag_manager_runtime_proposer_session_poll_vdf(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
) -> DagProposerSessionStep {
    let latest_proposal_level = runtime.state.proposer_frontier_facts().propose_level;
    let step = {
        let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
            return dag_proposer_session_not_started_step();
        };
        if !matches!(session.action, DagProposerSessionAction::StartVdf) {
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_UNEXPECTED_VDF_WAIT_REPORT")
        } else {
            let wait =
                plan_dag_proposer_vdf_wait(rustaxa_consensus::dag::DagProposerVdfWaitInput {
                    proposal_level: session.attempt.proposal_level,
                    latest_proposal_level,
                    vdf_difficulty: session.attempt.vdf_difficulty,
                    minimum_vdf_difficulty: session.minimum_vdf_difficulty,
                });
            if !wait.cancel_in_flight_proof {
                dag_proposer_session_step(session)
            } else {
                let retry = plan_dag_proposer_retry_reset(
                    rustaxa_consensus::dag::DagProposerRetryResetInput {
                        proposal_level: session.attempt.proposal_level,
                    },
                );
                let mut step = dag_proposer_session_step(session);
                step.action = DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF;
                step.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
                step.return_value = true;
                step.update_retry_state = retry.update_retry_state;
                step.next_last_propose_level = retry.next_last_propose_level;
                step.next_retry_count = retry.next_retry_count;
                step
            }
        }
    };
    finish_dag_proposer_session_step(runtime, session_id, step)
}

fn revalidate_proposer_session_observation(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
) -> Result<Option<DagProposerSessionStep>> {
    let current = match runtime.proposer_observation() {
        Ok(current) => current,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    if current.fingerprint
        == runtime.proposer_sessions[&session_id]
            .observation
            .fingerprint
    {
        return Ok(None);
    }
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    session.action = DagProposerSessionAction::Complete;
    session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
    session.reason_code = rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION;
    session.return_value = false;
    session.update_retry_state = false;
    session.error_code = "DAG_PROPOSER_SESSION_STALE_OBSERVATION".to_owned();
    let step = dag_proposer_session_step(session);
    Ok(Some(finish_dag_proposer_session_step(
        runtime, session_id, step,
    )))
}

fn prepare_proposer_session_signing(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    vdf_rlp: Vec<u8>,
) -> Result<DagProposerSessionStep> {
    let session = &runtime.proposer_sessions[&session_id];
    let frontier_tips = session.observation.frontier.frontier.tips.clone();
    let transaction_gas_estimations = session.transaction_gas_estimations.clone();
    let pbft_gas_limit = session.begin_input.pbft_gas_limit;
    let dag_gas_limit = session.begin_input.dag_gas_limit;
    let max_tips = session.begin_input.max_tips;
    let pivot = session.observation.frontier.frontier.pivot;
    let proposal_level = session.attempt.proposal_level;
    let transaction_hashes = session.selected_transaction_hashes.clone();

    let prepared = (|| -> Result<DomainDagProposerUnsignedBlockIntent> {
        let construction = plan_dag_proposer_block_construction_from_storage(
            runtime.storage.as_ref(),
            DomainDagProposerStorageBlockConstructionInput {
                frontier_tips,
                transaction_gas_estimations,
                pbft_gas_limit,
                dag_gas_limit,
                max_tips,
            },
        )?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("DAG_PROPOSER_CURRENT_TIMESTAMP")?
            .as_secs();
        Ok(plan_dag_proposer_block_intent(
            DomainDagProposerBlockIntentInput {
                pivot,
                level: proposal_level,
                timestamp,
                vdf_rlp,
                selected_tips: construction.selected_tips,
                transaction_hashes,
                block_gas_estimation: construction.block_gas_estimation,
            },
        ))
    })();
    let intent = match prepared {
        Ok(intent) => intent,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    session.vdf_rlp = intent.vdf_rlp.clone();
    session.unsigned_intent = Some(intent);
    session.action = DagProposerSessionAction::SignBlock;
    Ok(dag_proposer_session_step(session))
}

/// Consumes VDF proof completion and constructs the session-owned unsigned block intent.
///
/// Inputs are only proof success and canonical VDF RLP; all other block fields come from the cursor. Rust revalidates
/// the complete observation, performs storage-backed tip construction, chooses the timestamp, and returns signing action
/// 5 with the canonical signing hash. Stale observations terminate without retry mutation. Storage, timestamp, and
/// planning errors remove the cursor before returning `Err`; missing/out-of-order ids return invalid terminal steps.
pub fn dag_manager_runtime_proposer_session_report_vdf_proof(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    report: DagProposerVdfProofReport,
) -> Result<DagProposerSessionStep> {
    let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(session.action, DagProposerSessionAction::StartVdf) {
        let step = invalid_dag_proposer_report(
            session,
            "DAG_PROPOSER_SESSION_UNEXPECTED_VDF_PROOF_REPORT",
        );
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    if !report.proof_ok {
        let step = invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_VDF_PROOF_FAILED");
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    if let Some(step) = revalidate_proposer_session_observation(runtime, session_id)? {
        return Ok(step);
    }
    if runtime.proposer_sessions[&session_id].attempt.vdf_stale {
        let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
        session.vdf_rlp = report.vdf_rlp;
        session.action = DagProposerSessionAction::StaleProofSleep;
        return Ok(dag_proposer_session_step(session));
    }
    prepare_proposer_session_signing(runtime, session_id, report.vdf_rlp)
}

/// Resumes a stale-proof cursor after the external compatibility sleep.
///
/// Rust revalidates the complete observation before using the stored proof. An unchanged observation constructs the
/// unsigned intent and returns signing action 5; a stale observation terminates without retry mutation. Construction
/// errors remove the cursor before returning `Err`, and missing/out-of-order ids return invalid terminal steps.
pub fn dag_manager_runtime_proposer_session_resume_stale_proof(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
) -> Result<DagProposerSessionStep> {
    let latest_proposal_level = runtime.state.proposer_frontier_facts().propose_level;
    let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(session.action, DagProposerSessionAction::StaleProofSleep) {
        let step = invalid_dag_proposer_report(
            session,
            "DAG_PROPOSER_SESSION_UNEXPECTED_STALE_PROOF_REPORT",
        );
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    if let Some(step) = revalidate_proposer_session_observation(runtime, session_id)? {
        return Ok(step);
    }
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    let stale = plan_dag_proposer_stale_proof(rustaxa_consensus::dag::DagProposerStaleProofInput {
        proposal_level: session.attempt.proposal_level,
        latest_proposal_level,
    });
    session.reason_code = stale.reason_code;
    session.update_retry_state = stale.update_retry_state;
    session.next_last_propose_level = stale.next_last_propose_level;
    session.next_retry_count = stale.next_retry_count;
    if stale.action != rustaxa_consensus::dag::DAG_PROPOSER_ACTION_CONTINUE {
        session.action = DagProposerSessionAction::Complete;
        session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
        session.return_value = false;
        let step = dag_proposer_session_step(session);
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    let vdf_rlp = session.vdf_rlp.clone();
    prepare_proposer_session_signing(runtime, session_id, vdf_rlp)
}

/// Finalizes the cursor's unsigned intent with an external recoverable signature.
///
/// The report contains only the 65-byte recoverable signature over the previously returned signing hash. Rust requires
/// recovery to match the trusted proposer address captured at begin, then assembles/stores canonical signed RLP/hash and
/// returns add-block action 6. Malformed or wrong-key signatures and finalization errors remove the cursor before
/// returning `Err`; missing/out-of-order reports return invalid terminal steps.
pub fn dag_manager_runtime_proposer_session_report_signing(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    report: DagProposerSigningReport,
) -> Result<DagProposerSessionStep> {
    let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
        return Ok(dag_proposer_session_not_started_step());
    };
    if !matches!(session.action, DagProposerSessionAction::SignBlock) {
        let step =
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_UNEXPECTED_SIGNING_REPORT");
        return Ok(finish_dag_proposer_session_step(runtime, session_id, step));
    }
    let intent = session
        .unsigned_intent
        .clone()
        .expect("signing action must own an unsigned intent");
    let proposer_address = session.begin_input.proposer_address;
    let signed = match (|| -> Result<rustaxa_consensus::dag::DagProposerSignedBlockIntent> {
        let signed =
            finalize_dag_proposer_signed_block_intent(DomainDagProposerSignedBlockIntentInput {
                intent,
                signature: report.signature,
            })?;
        let block = rustaxa_types::dag::DagBlock::try_from(
            rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&signed.block_rlp),
        )
        .context("DAG_PROPOSER_SIGNED_BLOCK_DECODE")?;
        let recovered = block
            .recover_sender()
            .context("DAG_PROPOSER_SIGNATURE_RECOVERY")?;
        ensure!(
            recovered.0 == proposer_address,
            "DAG_PROPOSER_SIGNATURE_PROPOSER_MISMATCH"
        );
        Ok(signed)
    })() {
        Ok(signed) => signed,
        Err(error) => {
            runtime.proposer_sessions.remove(&session_id);
            return Err(error);
        }
    };
    let session = runtime.proposer_sessions.get_mut(&session_id).unwrap();
    session.signed_intent = Some(signed);
    session.action = DagProposerSessionAction::AddBlock;
    Ok(dag_proposer_session_step(session))
}

/// Reports `DagManager::addDagBlock` execution to the runtime-owned DAG proposal cursor.
pub fn dag_manager_runtime_proposer_session_report_add_block(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    report: DagProposerAddBlockReport,
) -> DagProposerSessionStep {
    let step = {
        let Some(session) = runtime.proposer_sessions.get_mut(&session_id) else {
            return dag_proposer_session_not_started_step();
        };
        if !matches!(session.action, DagProposerSessionAction::AddBlock) {
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_UNEXPECTED_ADD_BLOCK_REPORT")
        } else if ((report.accepted || report.duplicate) && report.expired)
            || (report.accepted && !report.missing_references.is_empty())
        {
            invalid_dag_proposer_report(session, "DAG_PROPOSER_SESSION_INVALID_ADD_BLOCK_REPORT")
        } else {
            let retry =
                plan_dag_proposer_retry_reset(rustaxa_consensus::dag::DagProposerRetryResetInput {
                    proposal_level: session.attempt.proposal_level,
                });
            session.action = DagProposerSessionAction::Complete;
            session.status = DAG_PROPOSER_SESSION_STATUS_COMPLETE;
            session.reason_code = if report.accepted {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_OK
            } else if report.expired {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_ADD_BLOCK_EXPIRED
            } else if !report.missing_references.is_empty() {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_ADD_BLOCK_MISSING_REFERENCES
            } else {
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_ADD_BLOCK_REJECTED
            };
            session.return_value = report.accepted;
            session.update_retry_state = retry.update_retry_state;
            session.next_last_propose_level = retry.next_last_propose_level;
            session.next_retry_count = retry.next_retry_count;
            session.record_proposed_block = report.accepted;
            dag_proposer_session_step(session)
        }
    };
    finish_dag_proposer_session_step(runtime, session_id, step)
}
/// Plans one DAG proposer worker-loop command from live executor facts.
///
/// C++ still owns the worker thread, network object, and timer. Rust owns the
/// command choice so scheduling policy does not live in the proposer shell.
pub fn dag_plan_proposer_worker_command(
    input: DagProposerWorkerCommandInput,
) -> DagProposerWorkerCommand {
    let command = plan_dag_proposer_worker_command(DomainDagProposerWorkerCommandInput {
        pbft_syncing: input.pbft_syncing,
        packet_queue_over_limit: input.packet_queue_over_limit,
        has_attempt_result: input.has_attempt_result,
        attempt_returned_proposed: input.attempt_returned_proposed,
    });
    DagProposerWorkerCommand {
        attempt_proposal: command.attempt_proposal,
        sleep_after_tick: command.sleep_after_tick,
        sleep_ms: command.sleep_ms,
        reason_code: command.reason_code,
    }
}

/// Verifies DAG VDF sortition after building canonical legacy messages in Rust.
///
/// C++ passes only the block payload and sortition context; Rust rebuilds:
/// - `vrf_input`: sequential RLP items `block_level`, `proposal_period_hash`
/// - `vdf_input`: sequential RLP items `pivot`, then each transaction hash
///
/// It then verifies the embedded proof using `vrf_public_key`.
pub fn dag_verify_vdf_sortition_from_block(
    input: DagVerifyVdfSortitionFromBlockInput,
) -> Result<DagVerifyVdfSortitionResult> {
    let result = verify_dag_vdf_sortition_from_block(DomainDagVdfSortitionBlockInput {
        block_rlp: input.block_rlp,
        block_level: input.block_level,
        proposal_period_hash: H256::from(input.proposal_period_hash),
        vrf_public_key: input.vrf_public_key,
        sortition_params: to_domain_sortition_params(input.sortition_params),
        sender_eligible_vote_count: input.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: input.vdf_sortition_max_vote_count,
    })?;

    Ok(DagVerifyVdfSortitionResult {
        vdf_status: result.vdf_status,
        difficulty: result.difficulty,
        expected_difficulty: result.expected_difficulty,
    })
}

/// Builds the legacy DAG VDF message for a pivot and ordered transaction hashes.
///
/// This bridge is used by the C++ DagManager shim to preserve the public
/// `DagManager::getVdfMessage` API while moving the consensus byte construction
/// into Rust. The output is a sequence of RLP items, matching legacy C++
/// `dev::RLPStream << pivot << tx_hash...` behavior.
pub fn dag_vdf_message(pivot: &[u8; 32], transaction_hashes: Vec<DagHash>) -> Vec<u8> {
    let hashes = transaction_hashes
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    construct_dag_vdf_message(H256::from(*pivot), &hashes)
}

pub fn dag_manager_block_from_rlp(block_rlp: Vec<u8>) -> Result<DagManagerBlock> {
    let block = domain_dag_manager_block_from_rlp(&block_rlp)?;
    Ok(DagManagerBlock {
        hash: block.hash.into(),
        pivot: block.pivot.into(),
        tips: to_dag_hashes(block.tips),
        level: block.level,
        difficulty: block.difficulty,
    })
}

fn to_bridge_add_block_effect_plan(
    plan: rustaxa_consensus::dag::DagAddBlockEffectPlan,
) -> DagAddBlockEffectPlan {
    DagAddBlockEffectPlan {
        accepted: plan.accepted,
        duplicate: plan.duplicate,
        expired: plan.expired,
        persist_transactions: plan.persist_transactions,
        persist_block: plan.persist_block,
        add_to_graph: plan.add_to_graph,
        emit_verified: plan.emit_verified,
        gossip: plan.gossip,
        proposed: plan.proposed,
        missing_references: plan
            .missing_references
            .into_iter()
            .map(|hash| DagHash { hash: hash.0 })
            .collect(),
    }
}

fn to_h256(hash: &[u8; 32]) -> H256 {
    H256::from(*hash)
}

fn to_domain_sortition_params(params: SortitionRuntimeParams) -> SortitionParams {
    SortitionParams {
        vrf: VrfParams {
            threshold_upper: params.threshold_upper,
        },
        vdf: VdfParams {
            difficulty_min: params.difficulty_min,
            difficulty_max: params.difficulty_max,
            difficulty_stale: params.difficulty_stale,
            lambda_bound: params.lambda_bound,
        },
    }
}

fn domain_attempt_input(
    input: &DagProposerSessionBeginInput,
    observation: &DagProposerObservation,
    report: DagProposerExternalProposalFactsReport,
    last_propose_level: u64,
    retry_count: u64,
) -> DomainDagProposerAttemptInput {
    DomainDagProposerAttemptInput {
        transaction_pool_size: input.transaction_pool_size,
        non_finalized_transaction_count: input.non_finalized_transaction_count,
        max_non_finalized_transactions: input.max_non_finalized_transactions,
        frontier: observation.frontier.clone(),
        proposal_period_found: observation.proposal_period_found,
        proposal_period: observation.proposal_period,
        last_finalized_period: report.last_finalized_period,
        dag_expiry_level_limit: input.dag_expiry_level_limit,
        period_block_hash_found: observation.period_block_hash_found,
        period_block_hash: observation.period_block_hash,
        wallet_vrf_public_key: input.wallet_vrf_public_key,
        wallet_vrf_secret: input.wallet_vrf_secret,
        authorization_facts: rustaxa_consensus::dag::DagDposAuthorizationFacts {
            vrf_key: report
                .authorization_facts
                .vrf_key
                .as_slice()
                .try_into()
                .ok(),
            vrf_key_found: report.authorization_facts.vrf_key_found,
            sender_eligible_vote_count: report.authorization_facts.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: report.authorization_facts.vdf_sortition_max_vote_count,
            eligibility_status: report.authorization_facts.eligibility_status,
        },
        sortition_params: to_domain_sortition_params(report.sortition_params),
        max_non_finalized_dag_blocks: input.max_non_finalized_dag_blocks,
        max_non_finalized_dag_blocks_low_difficulty: input
            .max_non_finalized_dag_blocks_low_difficulty,
        last_propose_level,
        retry_count,
        max_retry_count: input.max_retry_count,
        proposal_weight_limit: input.proposal_weight_limit,
        total_transaction_shards: input.total_transaction_shards,
        node_transaction_shard: input.node_transaction_shard,
        shard_period_interval: input.shard_period_interval,
    }
}

fn placeholder_attempt(
    observation: &DagProposerObservation,
    input: &DagProposerSessionBeginInput,
) -> DomainDagProposerAttemptPlan {
    DomainDagProposerAttemptPlan {
        action: rustaxa_consensus::dag::DAG_PROPOSER_ACTION_SKIP,
        reason_code: rustaxa_consensus::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD,
        frontier: observation.frontier.frontier.clone(),
        anchor: observation.frontier.anchor,
        proposal_level: observation.frontier.propose_level,
        proposal_period_found: observation.proposal_period_found,
        proposal_period: observation.proposal_period,
        last_finalized_period: 0,
        period_block_hash_found: observation.period_block_hash_found,
        period_block_hash: observation.period_block_hash,
        vrf_input: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        vdf_difficulty: 0,
        vdf_stale: false,
        old_proposal: false,
        update_retry_state: false,
        next_last_propose_level: 0,
        next_retry_count: 0,
        transaction_request: rustaxa_consensus::dag::DagProposerTransactionPackRequest {
            proposal_period: observation.proposal_period,
            weight_limit: input.proposal_weight_limit,
            total_transaction_shards: input.total_transaction_shards,
            node_transaction_shard: input.node_transaction_shard,
            shard_period_interval: input.shard_period_interval,
        },
    }
}

fn proposer_observation_fingerprint(
    frontier: &DomainDagProposerFrontierFacts,
    proposal_period_found: bool,
    proposal_period: u64,
    period_block_hash_found: bool,
    period_block_hash: H256,
) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(frontier.frontier.pivot.as_bytes());
    for tip in &frontier.frontier.tips {
        hasher.update(tip.as_bytes());
    }
    hasher.update(&frontier.propose_level.to_be_bytes());
    hasher.update(frontier.anchor.as_bytes());
    hasher.update(&(frontier.non_finalized_block_count as u64).to_be_bytes());
    hasher.update(&frontier.non_finalized_min_difficulty.to_be_bytes());
    hasher.update(&[u8::from(proposal_period_found)]);
    hasher.update(&proposal_period.to_be_bytes());
    hasher.update(&[u8::from(period_block_hash_found)]);
    hasher.update(period_block_hash.as_bytes());
    let mut output = [0_u8; 32];
    hasher.finalize(&mut output);
    output
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
        .collect()
}

fn to_transaction_hashes(hashes: Vec<DagTransactionHash>) -> Vec<H256> {
    hashes.into_iter().map(|hash| hash.hash.into()).collect()
}

fn to_bridge_transaction_hashes(hashes: Vec<H256>) -> Vec<DagTransactionHash> {
    hashes
        .into_iter()
        .map(|hash| DagTransactionHash { hash: hash.0 })
        .collect()
}

fn dag_proposer_session_step(session: &DagProposerSession) -> DagProposerSessionStep {
    let action = match session.action {
        DagProposerSessionAction::CollectExternalProposalFacts => {
            DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
        }
        DagProposerSessionAction::PackTransactions => DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS,
        DagProposerSessionAction::StartVdf => DAG_PROPOSER_SESSION_ACTION_START_VDF,
        DagProposerSessionAction::StaleProofSleep => DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP,
        DagProposerSessionAction::SignBlock => DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK,
        DagProposerSessionAction::AddBlock => DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK,
        DagProposerSessionAction::Complete => DAG_PROPOSER_SESSION_ACTION_NONE,
    };
    DagProposerSessionStep {
        status: session.status,
        action,
        reason_code: session.reason_code,
        return_value: session.return_value,
        update_retry_state: session.update_retry_state,
        next_last_propose_level: session.next_last_propose_level,
        next_retry_count: session.next_retry_count,
        frontier_pivot: session.attempt.frontier.pivot.into(),
        proposal_level: session.attempt.proposal_level,
        proposal_period: session.attempt.proposal_period,
        last_finalized_period: session.attempt.last_finalized_period,
        vrf_input: session.attempt.vrf_input.clone(),
        vote_count: session.attempt.vote_count,
        max_vote_count: session.attempt.max_vote_count,
        vdf_difficulty: session.attempt.vdf_difficulty,
        vdf_stale: session.attempt.vdf_stale,
        old_proposal: session.attempt.old_proposal,
        vdf_message: session.vdf_message.clone(),
        selected_transaction_hashes: to_dag_hashes(session.selected_transaction_hashes.clone()),
        transaction_request: DagProposerTransactionPackRequest {
            proposal_period: session.attempt.transaction_request.proposal_period,
            weight_limit: session.attempt.transaction_request.weight_limit,
            total_transaction_shards: session.attempt.transaction_request.total_transaction_shards,
            node_transaction_shard: session.attempt.transaction_request.node_transaction_shard,
            shard_period_interval: session.attempt.transaction_request.shard_period_interval,
        },
        signing_hash: session
            .unsigned_intent
            .as_ref()
            .map_or([0; 32], |intent| intent.signing_hash.0),
        signed_block: session.signed_intent.as_ref().map_or(
            DagProposerSignedBlockIntent {
                block_rlp: Vec::new(),
                block_hash: [0; 32],
            },
            |intent| DagProposerSignedBlockIntent {
                block_rlp: intent.block_rlp.clone(),
                block_hash: intent.block_hash.0,
            },
        ),
        record_proposed_block: session.record_proposed_block,
        vdf_poll_interval_ms: rustaxa_consensus::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
        stale_proof_sleep_ms: rustaxa_consensus::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
        error_code: session.error_code.clone(),
    }
}

fn invalid_dag_proposer_report(
    session: &mut DagProposerSession,
    error_code: &str,
) -> DagProposerSessionStep {
    session.action = DagProposerSessionAction::Complete;
    session.status = DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT;
    session.return_value = false;
    session.error_code = error_code.to_string();
    dag_proposer_session_step(session)
}

fn finish_dag_proposer_session_step(
    runtime: &mut BridgeDagManagerRuntime,
    session_id: u64,
    step: DagProposerSessionStep,
) -> DagProposerSessionStep {
    if !dag_proposer_session_step_is_terminal(&step) {
        return step;
    }

    if step.update_retry_state {
        if let Some(session) = runtime.proposer_sessions.get(&session_id) {
            if let Some(retry_state) = runtime.proposer_retry_states.get_mut(&session.retry_key) {
                retry_state.last_propose_level = step.next_last_propose_level;
                retry_state.retry_count = step.next_retry_count;
            }
        }
    }
    runtime.proposer_sessions.remove(&session_id);
    step
}

fn dag_proposer_session_step_is_terminal(step: &DagProposerSessionStep) -> bool {
    step.status != DAG_PROPOSER_SESSION_STATUS_ACTIVE
}

fn dag_proposer_session_not_started_step() -> DagProposerSessionStep {
    DagProposerSessionStep {
        status: DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT,
        action: DAG_PROPOSER_SESSION_ACTION_NONE,
        reason_code: rustaxa_consensus::dag::DAG_PROPOSER_REASON_OK,
        return_value: false,
        update_retry_state: false,
        next_last_propose_level: 0,
        next_retry_count: 0,
        frontier_pivot: [0; 32],
        proposal_level: 0,
        proposal_period: 0,
        last_finalized_period: 0,
        vrf_input: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        vdf_difficulty: 0,
        vdf_stale: false,
        old_proposal: false,
        vdf_message: Vec::new(),
        selected_transaction_hashes: Vec::new(),
        transaction_request: DagProposerTransactionPackRequest {
            proposal_period: 0,
            weight_limit: 0,
            total_transaction_shards: 0,
            node_transaction_shard: 0,
            shard_period_interval: 0,
        },
        signing_hash: [0; 32],
        signed_block: DagProposerSignedBlockIntent {
            block_rlp: Vec::new(),
            block_hash: [0; 32],
        },
        record_proposed_block: false,
        vdf_poll_interval_ms: rustaxa_consensus::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
        stale_proof_sleep_ms: rustaxa_consensus::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
        error_code: "DAG_PROPOSER_SESSION_NOT_STARTED".to_string(),
    }
}

fn verify_block_session_step(session: &DagVerifyBlockSession) -> DagVerifyBlockSessionStep {
    match &session.action {
        DagVerifyBlockSessionAction::TransactionQuery(query_hashes) => DagVerifyBlockSessionStep {
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            query_hashes: to_bridge_transaction_hashes(query_hashes.clone()),
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::AuthorizationFacts => DagVerifyBlockSessionStep {
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            query_hashes: Vec::new(),
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::VdfSortition {
            vote_count,
            max_vote_count,
        } => DagVerifyBlockSessionStep {
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_VDF_SORTITION,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            query_hashes: Vec::new(),
            vote_count: *vote_count,
            max_vote_count: *max_vote_count,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::Gas => DagVerifyBlockSessionStep {
            status: DAG_VERIFY_SESSION_STATUS_ACTIVE,
            action: DAG_VERIFY_SESSION_ACTION_GAS,
            complete: false,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            query_hashes: Vec::new(),
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
        DagVerifyBlockSessionAction::Complete => DagVerifyBlockSessionStep {
            status: DAG_VERIFY_SESSION_STATUS_COMPLETE,
            action: DAG_VERIFY_SESSION_ACTION_NONE,
            complete: true,
            reject_code: session.reject_code,
            proposal_period: session.proposal_period,
            query_hashes: Vec::new(),
            vote_count: 0,
            max_vote_count: 0,
            error_code: session.error_code.clone(),
        },
    }
}

fn invalid_verify_block_report(
    session: &mut DagVerifyBlockSession,
    error_code: &str,
) -> DagVerifyBlockSessionStep {
    session.action = DagVerifyBlockSessionAction::Complete;
    session.error_code = error_code.to_string();
    DagVerifyBlockSessionStep {
        status: DAG_VERIFY_SESSION_STATUS_INVALID_REPORT,
        action: DAG_VERIFY_SESSION_ACTION_NONE,
        complete: true,
        reject_code: session.reject_code,
        proposal_period: session.proposal_period,
        query_hashes: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        error_code: session.error_code.clone(),
    }
}

fn verify_block_session_not_started_step() -> DagVerifyBlockSessionStep {
    DagVerifyBlockSessionStep {
        status: DAG_VERIFY_SESSION_STATUS_INVALID_REPORT,
        action: DAG_VERIFY_SESSION_ACTION_NONE,
        complete: true,
        reject_code: 0,
        proposal_period: 0,
        query_hashes: Vec::new(),
        vote_count: 0,
        max_vote_count: 0,
        error_code: "DAG_VERIFY_SESSION_NOT_STARTED".to_string(),
    }
}

fn complete_verify_block_session(
    session: &mut DagVerifyBlockSession,
    reject_code: u32,
) -> DagVerifyBlockSessionStep {
    session.reject_code = reject_code;
    session.action = DagVerifyBlockSessionAction::Complete;
    verify_block_session_step(session)
}

fn to_bridge_sync_blocks(
    blocks: Vec<rustaxa_consensus::dag::DagSyncBlockRlp>,
) -> Vec<DagSyncBlockRlp> {
    blocks
        .into_iter()
        .map(|block| DagSyncBlockRlp {
            hash: block.hash.into(),
            block_rlp: block.block_rlp,
        })
        .collect()
}

fn to_bridge_transaction_rlp_lookups(
    lookups: Vec<rustaxa_consensus::dag::DagTransactionStorageLookup>,
) -> Vec<DagTransactionRlpLookup> {
    lookups
        .into_iter()
        .map(|lookup| DagTransactionRlpLookup {
            hash: lookup.hash.into(),
            found: lookup.found,
            finalized: lookup.finalized,
            tx_rlp: lookup.tx_rlp,
        })
        .collect()
}

fn to_bridge_frontier(frontier: &rustaxa_consensus::dag::DagFrontier) -> DagFrontier {
    DagFrontier {
        pivot: frontier.pivot.into(),
        tips: to_dag_hashes(frontier.tips.clone()),
    }
}

fn to_bridge_finalization_plan(
    plan: DomainDagManagerFinalizationPlan,
) -> DagManagerFinalizationPlan {
    DagManagerFinalizationPlan {
        finalized_count: plan.finalized_count as u64,
        counter_update_hashes: to_dag_hashes(plan.counter_update_hashes),
        expired_hashes: to_dag_hashes(plan.expired_hashes),
        remaining_hashes: to_dag_hashes(plan.remaining_hashes),
    }
}

fn to_bridge_finalization_cleanup_payload(
    payload: DomainDagManagerFinalizationCleanupStoragePayload,
) -> DagManagerFinalizationCleanupPayload {
    DagManagerFinalizationCleanupPayload {
        counter_updates: payload
            .counter_updates
            .into_iter()
            .map(|update| DagFinalizedCounterUpdate {
                hash: update.hash.into(),
                level: update.level,
                tips_count: update.tips_count,
            })
            .collect(),
        expired_hashes: to_dag_hashes(payload.expired_hashes),
        remove_transaction_hashes: to_bridge_transaction_hashes(payload.remove_transaction_hashes),
    }
}

fn to_domain_block(block: DagManagerBlock) -> DomainDagManagerBlock {
    DomainDagManagerBlock {
        hash: H256::from(block.hash),
        pivot: H256::from(block.pivot),
        tips: block
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect(),
        level: block.level,
        difficulty: block.difficulty,
    }
}

fn dag_reference_metadata_from_runtime_or_storage(
    state: &DagManagerState,
    storage: &Storage,
    hash: H256,
) -> Result<ReferenceMetadata> {
    let metadata = state.reference_metadata(hash);
    if metadata.found {
        return Ok(metadata);
    }

    if storage
        .dag()
        .by_hash_rlp_optional(hash)
        .context("DAG_RUNTIME_REFERENCE_STORAGE_LOOKUP")?
        .is_none()
    {
        return Ok(metadata);
    }

    let block = storage
        .dag()
        .by_hash(hash)
        .context("DAG_RUNTIME_REFERENCE_STORAGE_DECODE")?;
    Ok(ReferenceMetadata {
        hash,
        found: true,
        level: block.level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::{
        DagDposAuthorizationFacts, DagProposerExternalProposalFactsReport,
        DagProposerSessionBeginInput, SortitionRuntimeParams,
    };
    use crate::ffi::{BridgeDagStorageQueries, BridgeStorage, BridgeTransactionStorageQueries};
    use crate::storage::{
        create_dag_storage_queries, create_storage, create_transaction_storage_queries,
    };
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_consensus::dag;
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use rustaxa_vdf::prover::CancellationToken;
    use rustaxa_vdf::sortition::{self, LegacySortitionParams};
    use rustaxa_vdf::vrf::public_key_from_secret;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after UNIX_EPOCH")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, now_ns, id))
    }

    fn transaction_queries(storage: &BridgeStorage) -> Box<BridgeTransactionStorageQueries> {
        create_transaction_storage_queries(storage)
    }

    fn dag_queries(storage: &BridgeStorage) -> Box<BridgeDagStorageQueries> {
        create_dag_storage_queries(storage)
    }

    fn dag_block_with_vdf_payload(vdf_payload: Vec<u8>) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&&[0u8; 32][..]);
        block.append(&1u64);
        block.append(&0u64);
        block.append(&vdf_payload);
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    fn signed_dag_block_rlp(seed: u8, level: u64, gas_estimation: u64) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let mut block = rustaxa_types::dag::DagBlock {
            pivot: H256::from([1u8; 32]),
            level,
            timestamp: 123,
            vdf: vec![1, 2, 3],
            tips: vec![],
            transactions: vec![H256::from([9u8; 32])],
            signature: [0; 65],
            gas_estimation,
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .expect("sign dag block");
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();

        let mut stream = RlpStream::new_list(8);
        stream.append(&block.pivot);
        stream.append(&block.level);
        stream.append(&block.timestamp);
        stream.append(&block.vdf);
        stream.append_list(&block.tips);
        stream.append_list(&block.transactions);
        stream.append(&block.signature.as_ref());
        stream.append(&block.gas_estimation);
        stream.out().to_vec()
    }

    fn dag_block_with_vdf_payload_and_transaction_hashes(
        vdf_payload: Vec<u8>,
        transaction_hashes: &[DagTransactionHash],
    ) -> Vec<u8> {
        dag_block_with_level_and_transaction_hashes(1, vdf_payload, transaction_hashes)
    }

    fn dag_block_with_level_and_transaction_hashes(
        level: u64,
        vdf_payload: Vec<u8>,
        transaction_hashes: &[DagTransactionHash],
    ) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&&[0u8; 32][..]);
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf_payload);
        block.begin_list(0);
        block.begin_list(transaction_hashes.len());
        for hash in transaction_hashes {
            block.append(&&hash.hash[..]);
        }
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    fn tx_hash(byte: u8) -> DagTransactionHash {
        DagTransactionHash { hash: [byte; 32] }
    }

    fn signed_pbft_block(period: u64, timestamp: u64) -> Vec<u8> {
        signed_pbft_block_with_pivot(period, timestamp, H256::from_low_u64_be(11))
    }

    fn signed_pbft_block_with_pivot(period: u64, timestamp: u64, pivot: H256) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(10));
        block.append(&pivot);
        block.append(&H256::from_low_u64_be(12));
        block.append(&H256::from_low_u64_be(13));
        block.append(&period);
        block.append(&timestamp);
        block.begin_list(0);
        block.append(&vec![0u8; 65]);
        block.out().to_vec()
    }

    fn period_data_with_pbft_block(pbft_block: &[u8]) -> Vec<u8> {
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(pbft_block, 1);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.begin_list(0);
        period_data.out().to_vec()
    }

    fn dag_block_with_pivot_level_and_difficulty(
        pivot: H256,
        level: u64,
        difficulty: u16,
    ) -> Vec<u8> {
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);
        vdf_payload.append(&difficulty);

        let mut block = RlpStream::new_list(8);
        block.append(&pivot);
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf_payload.out().to_vec());
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    const SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    #[test]
    fn dag_manager_runtime_persists_and_loads_blocks() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_storage");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            let hash = [7u8; 32];
            let block_rlp = vec![0xAA, 0xBB, 0xCC];

            runtime
                .dag_manager_runtime_save_block(&hash, 11, 2, block_rlp.clone())
                .expect("save should succeed");

            assert!(runtime
                .dag_manager_runtime_is_block_known(&hash)
                .expect("known lookup should succeed"));

            let loaded = runtime
                .dag_manager_runtime_load_block(&hash)
                .expect("load should succeed");
            assert!(loaded.found);
            assert_eq!(loaded.block_rlp, block_rlp);

            let counters = runtime
                .dag_manager_runtime_persistence_counters()
                .expect("counter lookup should succeed");
            assert_eq!(counters.dag_blocks, 1);
            assert_eq!(counters.dag_edges, 3);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_plans_add_block_from_runtime_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_add_block_plan");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");

            let live_only = runtime
                .dag_manager_runtime_plan_add_block(DagAddBlockRuntimeInput {
                    save: true,
                    proposed: true,
                    block_hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    block_level: 2,
                })
                .expect("live-only persistence plan");
            assert!(live_only.accepted);
            assert!(!live_only.duplicate);
            assert!(live_only.persist_block);
            assert!(!live_only.add_to_graph);
            assert!(!live_only.emit_verified);
            assert!(!live_only.gossip);

            let missing = runtime
                .dag_manager_runtime_plan_add_block(DagAddBlockRuntimeInput {
                    save: true,
                    proposed: false,
                    block_hash: [3u8; 32],
                    pivot: [9u8; 32],
                    tips: vec![DagHash { hash: [8u8; 32] }],
                    block_level: 3,
                })
                .expect("missing-reference plan");
            assert!(!missing.accepted);
            assert_eq!(
                missing
                    .missing_references
                    .iter()
                    .map(|hash| hash.hash)
                    .collect::<Vec<_>>(),
                vec![[9u8; 32], [8u8; 32]]
            );

            let accepted = runtime
                .dag_manager_runtime_plan_add_block(DagAddBlockRuntimeInput {
                    save: true,
                    proposed: false,
                    block_hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![],
                    block_level: 3,
                })
                .expect("accepted plan");
            assert!(accepted.accepted);
            assert!(accepted.persist_transactions);
            assert!(accepted.persist_block);
            assert!(accepted.add_to_graph);
            assert!(accepted.emit_verified);
            assert!(accepted.gossip);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_plans_proposal_tip_selection_from_storage_tips() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposal_tip_selection");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_save_block(
                    &[10u8; 32],
                    3,
                    0,
                    signed_dag_block_rlp(0x61, 3, 100),
                )
                .expect("save lower tip");
            runtime
                .dag_manager_runtime_save_block(
                    &[20u8; 32],
                    5,
                    0,
                    signed_dag_block_rlp(0x62, 5, 100),
                )
                .expect("save higher tip");

            let plan = runtime
                .dag_manager_runtime_plan_proposal_tip_selection(
                    DagProposerStorageTipSelectionInput {
                        frontier_tips: vec![
                            DagHash { hash: [10u8; 32] },
                            DagHash { hash: [20u8; 32] },
                            DagHash { hash: [30u8; 32] },
                        ],
                        gas_limit: 150,
                        max_tips: 16,
                    },
                )
                .expect("plan");

            assert_eq!(plan.selected_tips.len(), 1);
            assert_eq!(plan.selected_tips[0].hash, [20u8; 32]);
            assert_eq!(plan.skipped_missing_tips, 1);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_ensures_proposal_period_for_mismatched_lookup() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposal_mapping");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            assert!(runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(200, 5)
                .expect("initial mapping write should succeed"));

            // Level 100 resolves to period 5 via the later (200 -> 5) mapping.
            let before = dag_queries(&storage)
                .get_proposal_period_for_dag_level(100)
                .expect("lookup should succeed");
            assert!(before.found);
            assert_eq!(before.period, 5);

            // Ensure path must still write because the resolved value mismatches
            // the expected period for this level.
            assert!(runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
                .expect("mismatch correction should succeed"));

            let after = dag_queries(&storage)
                .get_proposal_period_for_dag_level(100)
                .expect("lookup should succeed");
            assert!(after.found);
            assert_eq!(after.period, 0);

            assert!(!runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(100, 0)
                .expect("idempotent ensure should succeed"));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_period_block_hash_uses_rust_period_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_period_hash");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            let missing = runtime
                .dag_manager_runtime_period_block_hash(7)
                .expect("missing period lookup should succeed");
            assert!(!missing.found);
            assert_eq!(missing.hash, [0; 32]);

            let pbft_block = signed_pbft_block(7, 99);
            let expected_hash: [u8; 32] =
                PbftBlockLink::try_from(SignedPbftBlockRlp::new(&pbft_block))
                    .expect("test PBFT block should decode")
                    .block_hash
                    .into();
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let found = runtime
                .dag_manager_runtime_period_block_hash(7)
                .expect("period hash lookup should succeed");
            assert!(found.found);
            assert_eq!(found.hash, expected_hash);

            storage
                .0
                .period()
                .write(8, &vec![0x80])
                .expect("corrupt period data save should succeed");
            assert!(runtime.dag_manager_runtime_period_block_hash(8).is_err());
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    fn proposer_session_begin_input(vrf_key: [u8; 32]) -> DagProposerSessionBeginInput {
        DagProposerSessionBeginInput {
            transaction_pool_size: 1,
            non_finalized_transaction_count: 0,
            max_non_finalized_transactions: 100,
            dag_expiry_level_limit: 100,
            wallet_vrf_public_key: vrf_key,
            wallet_vrf_secret: SECRET_KEY,
            proposer_address: proposer_address_for_seed(0x44),
            max_non_finalized_dag_blocks: 100,
            max_non_finalized_dag_blocks_low_difficulty: 50,
            max_retry_count: 20,
            proposal_weight_limit: 1_000,
            total_transaction_shards: 4,
            node_transaction_shard: 2,
            shard_period_interval: 10,
            pbft_gas_limit: 10_000,
            dag_gas_limit: 1_000,
            max_tips: 16,
        }
    }

    fn proposer_external_facts(vrf_key: [u8; 32]) -> DagProposerExternalProposalFactsReport {
        DagProposerExternalProposalFactsReport {
            last_finalized_period: 7,
            authorization_facts: DagDposAuthorizationFacts {
                vrf_key_found: true,
                vrf_key: vrf_key.to_vec(),
                sender_eligible_vote_count: 10,
                vdf_sortition_max_vote_count: 20,
                eligibility_status: dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
            sortition_params: SortitionRuntimeParams {
                threshold_upper: u16::MAX,
                difficulty_min: 3,
                difficulty_max: 3,
                difficulty_stale: 9,
                lambda_bound: 128,
            },
        }
    }

    fn proposer_address_for_seed(seed: u8) -> [u8; 20] {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let mut public_key_hash = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&encoded.as_bytes()[1..]);
        hasher.finalize(&mut public_key_hash);
        public_key_hash[12..]
            .try_into()
            .expect("address slice has fixed length")
    }

    fn sign_proposer_hash_with_seed(signing_hash: [u8; 32], seed: u8) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&signing_hash)
            .expect("sign proposer intent");
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(recovery_id.to_byte());
        bytes
    }

    fn sign_proposer_hash(signing_hash: [u8; 32]) -> Vec<u8> {
        sign_proposer_hash_with_seed(signing_hash, 0x44)
    }

    fn begin_proposer_vdf_session(
        runtime: &mut BridgeDagManagerRuntime,
        vrf_key: [u8; 32],
        transaction_hash: [u8; 32],
    ) -> u64 {
        let session_id = dag_manager_runtime_begin_proposer_session(
            runtime,
            proposer_session_begin_input(vrf_key),
        )
        .expect("session should open");
        assert_eq!(
            dag_manager_runtime_proposer_session_report_external_facts(
                runtime,
                session_id,
                proposer_external_facts(vrf_key),
            )
            .expect("external facts should succeed")
            .action,
            DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
        );
        assert_eq!(
            dag_manager_runtime_proposer_session_report_transactions(
                runtime,
                session_id,
                DagProposerTransactionPackReport {
                    network_throttled: false,
                    transaction_hashes: vec![DagHash {
                        hash: transaction_hash,
                    }],
                    transaction_gas_estimations: vec![100],
                },
            )
            .action,
            DAG_PROPOSER_SESSION_ACTION_START_VDF
        );
        session_id
    }

    #[test]
    fn dag_proposer_worker_command_plans_attempts_and_backoff() {
        let attempt = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: false,
            attempt_returned_proposed: false,
        });
        assert!(attempt.attempt_proposal);
        assert!(!attempt.sleep_after_tick);

        let throttle = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: true,
            packet_queue_over_limit: false,
            has_attempt_result: false,
            attempt_returned_proposed: false,
        });
        assert!(!throttle.attempt_proposal);
        assert!(throttle.sleep_after_tick);
        assert_eq!(throttle.sleep_ms, 100);

        let no_block = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: true,
            attempt_returned_proposed: false,
        });
        assert!(!no_block.attempt_proposal);
        assert!(no_block.sleep_after_tick);
        assert_eq!(no_block.sleep_ms, 100);

        let proposed = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: true,
            attempt_returned_proposed: true,
        });
        assert!(!proposed.attempt_proposal);
        assert!(!proposed.sleep_after_tick);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_orders_executor_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_session");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 7)
                .expect("proposal-period mapping should save");
            let pbft_block = signed_pbft_block(7, 99);
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");

            let first = dag_manager_runtime_proposer_session_next(&mut runtime, session_id);
            assert_eq!(first.status, DAG_PROPOSER_SESSION_STATUS_ACTIVE);
            assert_eq!(
                first.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(first.transaction_request.proposal_period, 7);

            let pack = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                session_id,
                proposer_external_facts(vrf_key),
            )
            .expect("external facts should be accepted");
            assert_eq!(pack.action, DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS);
            assert_eq!(first.vdf_poll_interval_ms, 100);
            assert_eq!(first.stale_proof_sleep_ms, 1_000);
            assert!(!pack.vrf_input.is_empty());

            let start_vdf = dag_manager_runtime_proposer_session_report_transactions(
                &mut runtime,
                session_id,
                DagProposerTransactionPackReport {
                    network_throttled: false,
                    transaction_hashes: vec![DagHash { hash: [2u8; 32] }],
                    transaction_gas_estimations: vec![100],
                },
            );
            assert_eq!(start_vdf.action, DAG_PROPOSER_SESSION_ACTION_START_VDF);
            assert_eq!(start_vdf.selected_transaction_hashes.len(), 1);
            assert_eq!(
                start_vdf.vdf_message,
                dag_vdf_message(&[1u8; 32], vec![DagHash { hash: [2u8; 32] }])
            );

            let still_waiting =
                dag_manager_runtime_proposer_session_poll_vdf(&mut runtime, session_id);
            assert_eq!(still_waiting.action, DAG_PROPOSER_SESSION_ACTION_START_VDF);

            let build = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("block construction should succeed");
            assert_eq!(build.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            assert_ne!(build.signing_hash, [0; 32]);

            let add = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                session_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash(build.signing_hash),
                },
            )
            .expect("signed intent should finalize");
            assert_eq!(add.action, DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK);
            assert!(!add.signed_block.block_rlp.is_empty());
            assert_ne!(add.signed_block.block_hash, [0; 32]);
            let decoded = rustaxa_types::dag::DagBlock::try_from(
                rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&add.signed_block.block_rlp),
            )
            .expect("canonical signed block should decode");
            assert_eq!(decoded.pivot, H256::from([1u8; 32]));
            assert_eq!(decoded.level, 1);
            assert_eq!(decoded.vdf, vec![0xC0]);
            assert_eq!(decoded.transactions, vec![H256::from([2u8; 32])]);
            let mut expected_hash = [0u8; 32];
            let mut hasher = Keccak::v256();
            hasher.update(&add.signed_block.block_rlp);
            hasher.finalize(&mut expected_hash);
            assert_eq!(add.signed_block.block_hash, expected_hash);

            let complete = dag_manager_runtime_proposer_session_report_add_block(
                &mut runtime,
                session_id,
                DagProposerAddBlockReport {
                    accepted: true,
                    duplicate: false,
                    expired: false,
                    missing_references: Vec::new(),
                },
            );
            assert_eq!(complete.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(complete.action, DAG_PROPOSER_SESSION_ACTION_NONE);
            assert!(complete.return_value);
            assert!(complete.record_proposed_block);
            assert!(complete.update_retry_state);
            assert_eq!(complete.next_last_propose_level, 1);
            assert_eq!(complete.next_retry_count, 0);
            let retry_state = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("terminal step should persist retry state");
            assert_eq!(retry_state.last_propose_level, 1);
            assert_eq!(retry_state.retry_count, 0);
            assert_eq!(retry_state.max_retry_count, 20);

            let after_complete =
                dag_manager_runtime_proposer_session_next(&mut runtime, session_id);
            assert_eq!(
                after_complete.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_sessions_are_independent() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_keyed_sessions");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 7)
                .expect("first proposal-period mapping should save");
            let pbft_block = signed_pbft_block(7, 99);
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&pbft_block))
                .expect("period data save should succeed");

            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let first_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("first session should open");
            let second_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("second session should open");
            assert_ne!(first_id, second_id);

            let second_first_step =
                dag_manager_runtime_proposer_session_next(&mut runtime, second_id);
            assert_eq!(second_first_step.status, DAG_PROPOSER_SESSION_STATUS_ACTIVE);
            assert_eq!(
                second_first_step.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(second_first_step.proposal_level, 1);

            let first_first_step =
                dag_manager_runtime_proposer_session_next(&mut runtime, first_id);
            assert_eq!(first_first_step.status, DAG_PROPOSER_SESSION_STATUS_ACTIVE);
            assert_eq!(
                first_first_step.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(first_first_step.proposal_level, 1);

            let second_pack = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                second_id,
                proposer_external_facts(vrf_key),
            )
            .expect("second report should succeed");
            assert_eq!(
                second_pack.action,
                DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
            );

            let second_start_vdf = dag_manager_runtime_proposer_session_report_transactions(
                &mut runtime,
                second_id,
                DagProposerTransactionPackReport {
                    network_throttled: false,
                    transaction_hashes: vec![DagHash { hash: [4u8; 32] }],
                    transaction_gas_estimations: vec![200],
                },
            );
            assert_eq!(
                second_start_vdf.action,
                DAG_PROPOSER_SESSION_ACTION_START_VDF
            );
            assert_eq!(second_start_vdf.proposal_level, 1);
            assert_eq!(
                second_start_vdf.vdf_message,
                dag_vdf_message(&[1u8; 32], vec![DagHash { hash: [4u8; 32] }])
            );

            let first_still_waiting =
                dag_manager_runtime_proposer_session_next(&mut runtime, first_id);
            assert_eq!(
                first_still_waiting.action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );
            assert_eq!(first_still_waiting.proposal_level, 1);

            let first_pack = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                first_id,
                proposer_external_facts(vrf_key),
            )
            .expect("first report should succeed");
            assert_eq!(
                first_pack.action,
                DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
            );

            let first_start_vdf = dag_manager_runtime_proposer_session_report_transactions(
                &mut runtime,
                first_id,
                DagProposerTransactionPackReport {
                    network_throttled: false,
                    transaction_hashes: vec![DagHash { hash: [2u8; 32] }],
                    transaction_gas_estimations: vec![100],
                },
            );
            assert_eq!(
                first_start_vdf.action,
                DAG_PROPOSER_SESSION_ACTION_START_VDF
            );
            assert_eq!(first_start_vdf.proposal_level, 1);
            assert_eq!(
                first_start_vdf.vdf_message,
                dag_vdf_message(&[1u8; 32], vec![DagHash { hash: [2u8; 32] }])
            );

            let second_sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                second_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC1, 0x02],
                },
            )
            .expect("second intent should build");
            let first_sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                first_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC1, 0x01],
                },
            )
            .expect("first intent should build");
            assert_eq!(second_sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            assert_eq!(first_sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            assert_ne!(second_sign.signing_hash, first_sign.signing_hash);
            assert_eq!(
                runtime.proposer_sessions[&second_id]
                    .unsigned_intent
                    .as_ref()
                    .expect("second intent")
                    .transaction_hashes,
                vec![H256::from([4u8; 32])]
            );
            assert_eq!(
                runtime.proposer_sessions[&first_id]
                    .unsigned_intent
                    .as_ref()
                    .expect("first intent")
                    .transaction_hashes,
                vec![H256::from([2u8; 32])]
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_handles_missing_period_and_invalid_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_invalid");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

            let missing_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("missing-period session should open");
            let missing = dag_manager_runtime_proposer_session_next(&mut runtime, missing_id);
            assert_eq!(missing.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                missing.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD
            );

            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let invalid_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");
            let invalid = dag_manager_runtime_proposer_session_report_transactions(
                &mut runtime,
                invalid_id,
                DagProposerTransactionPackReport {
                    network_throttled: false,
                    transaction_hashes: Vec::new(),
                    transaction_gas_estimations: Vec::new(),
                },
            );
            assert_eq!(invalid.status, DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT);
            assert_eq!(
                invalid.error_code,
                "DAG_PROPOSER_SESSION_UNEXPECTED_TRANSACTION_REPORT"
            );

            let after_invalid = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                invalid_id,
                proposer_external_facts(vrf_key),
            )
            .expect("unknown-session report should return a step");
            assert_eq!(
                after_invalid.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_abort_proposer_session_is_idempotent() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_abort");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");
            assert!(runtime.proposer_sessions.contains_key(&session_id));

            assert!(dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                u64::MAX
            ));
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_external_facts_error_removes_proposer_session() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_report_error");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 7)
                .expect("proposal-period mapping should save");
            storage
                .0
                .period()
                .write(7, &period_data_with_pbft_block(&signed_pbft_block(7, 99)))
                .expect("valid period data should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");
            assert!(runtime.proposer_sessions.contains_key(&session_id));

            storage
                .0
                .period()
                .write(7, &vec![0x80])
                .expect("corrupt period data should save");
            assert!(dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                session_id,
                proposer_external_facts(vrf_key),
            )
            .is_err());
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_stale_observation_before_retry_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_stale_observation");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");
            assert_eq!(
                dag_manager_runtime_proposer_session_next(&mut runtime, session_id).action,
                DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
            );

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");
            let stale = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                session_id,
                proposer_external_facts(vrf_key),
            )
            .expect("stale report should return a terminal step");
            assert_eq!(stale.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                stale.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert_eq!(stale.error_code, "DAG_PROPOSER_SESSION_STALE_OBSERVATION");
            assert!(!runtime.proposer_retry_states.contains_key(&vrf_key));
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_stale_observation_after_vdf() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_stale_after_vdf");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");
            let stale = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("stale observation should return a terminal step");
            assert_eq!(stale.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(
                stale.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert!(!stale.update_retry_state);
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_cleans_up_malformed_signature() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_bad_signature");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");
            assert_eq!(sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);

            let error = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                session_id,
                DagProposerSigningReport {
                    signature: vec![0; 65],
                },
            )
            .err()
            .expect("structurally invalid signature must fail recovery");
            assert!(error
                .to_string()
                .contains("DAG_PROPOSER_SIGNATURE_RECOVERY"));
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_valid_wrong_key_signature() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_wrong_signer");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");

            let error = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                session_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash_with_seed(sign.signing_hash, 0x45),
                },
            )
            .err()
            .expect("wrong-key signature must be rejected");
            assert!(error
                .to_string()
                .contains("DAG_PROPOSER_SIGNATURE_PROPOSER_MISMATCH"));
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            assert!(!dag_manager_runtime_abort_proposer_session(
                &mut runtime,
                session_id
            ));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_cleans_up_corrupt_tip_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_corrupt_tip");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            for block in [
                DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                },
                DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 80,
                },
            ] {
                runtime
                    .dag_manager_runtime_add_block(block)
                    .expect("graph branch should add");
            }
            let frontier = runtime.state.proposer_frontier_facts();
            assert!(!frontier.frontier.tips.is_empty());
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(frontier.propose_level, 0)
                .expect("bootstrap proposal-period mapping should save");
            let corrupt_tip = frontier.frontier.tips[0];
            runtime
                .dag_manager_runtime_save_block(&corrupt_tip.0, 2, 0, vec![0x80])
                .expect("corrupt canonical row should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [4u8; 32]);

            assert!(dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .is_err());
            assert!(!runtime.proposer_sessions.contains_key(&session_id));
            let retry = runtime
                .proposer_retry_states
                .get(&vrf_key)
                .expect("external facts initialize retry state");
            assert_eq!(retry.last_propose_level, 0);
            assert_eq!(retry.retry_count, 0);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_rejects_duplicate_and_out_of_order_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_report_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

            let out_of_order_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [2u8; 32]);
            let out_of_order = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                out_of_order_id,
                DagProposerSigningReport {
                    signature: vec![0; 65],
                },
            )
            .expect("out-of-order report should return a step");
            assert_eq!(
                out_of_order.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );

            let duplicate_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [3u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                duplicate_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");
            assert_eq!(sign.action, DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK);
            let duplicate = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                duplicate_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("duplicate report should return a step");
            assert_eq!(duplicate.status, DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT);

            let duplicate_signing_id = begin_proposer_vdf_session(&mut runtime, vrf_key, [4u8; 32]);
            let sign = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                duplicate_signing_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("construction should succeed");
            let add = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                duplicate_signing_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash(sign.signing_hash),
                },
            )
            .expect("signing should succeed");
            assert_eq!(add.action, DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK);
            let duplicate_signing = dag_manager_runtime_proposer_session_report_signing(
                &mut runtime,
                duplicate_signing_id,
                DagProposerSigningReport {
                    signature: sign_proposer_hash(sign.signing_hash),
                },
            )
            .expect("duplicate signing should return a step");
            assert_eq!(
                duplicate_signing.status,
                DAG_PROPOSER_SESSION_STATUS_INVALID_REPORT
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_uses_runtime_frontier_for_vdf_cancel() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_vdf_cancel");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            let session_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");
            let pack = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                session_id,
                proposer_external_facts(vrf_key),
            )
            .expect("external facts should be accepted");
            assert_eq!(pack.action, DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS);
            let start = dag_manager_runtime_proposer_session_report_transactions(
                &mut runtime,
                session_id,
                DagProposerTransactionPackReport {
                    network_throttled: false,
                    transaction_hashes: vec![DagHash { hash: [2u8; 32] }],
                    transaction_gas_estimations: vec![100],
                },
            );
            assert_eq!(start.action, DAG_PROPOSER_SESSION_ACTION_START_VDF);
            runtime
                .proposer_sessions
                .get_mut(&session_id)
                .expect("session should remain active")
                .minimum_vdf_difficulty = start.vdf_difficulty.saturating_sub(1);

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");
            let cancelled = dag_manager_runtime_proposer_session_poll_vdf(&mut runtime, session_id);
            assert_eq!(cancelled.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(cancelled.action, DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_proposer_session_resumes_stale_proof_from_runtime_frontier() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_proposer_stale_resume");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(1, 0)
                .expect("bootstrap proposal-period mapping should save");
            let vrf_key =
                public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");
            runtime.proposer_retry_states.insert(
                vrf_key,
                DagProposerRetryState {
                    last_propose_level: 1,
                    retry_count: 20,
                    max_retry_count: 20,
                },
            );
            let session_id = dag_manager_runtime_begin_proposer_session(
                &mut runtime,
                proposer_session_begin_input(vrf_key),
            )
            .expect("session should open");
            let mut facts = proposer_external_facts(vrf_key);
            facts.sortition_params.difficulty_min = 9;
            facts.sortition_params.difficulty_max = 9;
            facts.sortition_params.difficulty_stale = 9;
            let pack = dag_manager_runtime_proposer_session_report_external_facts(
                &mut runtime,
                session_id,
                facts,
            )
            .expect("external facts should be accepted");
            assert!(pack.vdf_stale);
            assert_eq!(pack.action, DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS);
            assert_eq!(
                dag_manager_runtime_proposer_session_report_transactions(
                    &mut runtime,
                    session_id,
                    DagProposerTransactionPackReport {
                        network_throttled: false,
                        transaction_hashes: vec![DagHash { hash: [2u8; 32] }],
                        transaction_gas_estimations: vec![100],
                    },
                )
                .action,
                DAG_PROPOSER_SESSION_ACTION_START_VDF
            );
            let sleep = dag_manager_runtime_proposer_session_report_vdf_proof(
                &mut runtime,
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp: vec![0xC0],
                },
            )
            .expect("stale proof should request sleep");
            assert_eq!(sleep.action, DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP);

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: Vec::new(),
                    level: 2,
                    difficulty: 100,
                })
                .expect("frontier mutation should succeed");

            let resumed =
                dag_manager_runtime_proposer_session_resume_stale_proof(&mut runtime, session_id)
                    .expect("stale resume should produce a terminal step");
            assert_eq!(resumed.status, DAG_PROPOSER_SESSION_STATUS_COMPLETE);
            assert_eq!(resumed.action, DAG_PROPOSER_SESSION_ACTION_NONE);
            assert_eq!(
                resumed.reason_code,
                rustaxa_consensus::dag::DAG_PROPOSER_REASON_STALE_OBSERVATION
            );
            assert!(!resumed.update_retry_state);
            assert_eq!(resumed.proposal_level, 1);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_verify_block_session_orders_live_reports() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_verify_session");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");
            runtime
                .dag_manager_runtime_ensure_proposal_period_mapping(5, 7)
                .expect("mapping write should succeed");

            dag_manager_runtime_begin_verify_block_session(
                &mut runtime,
                DagVerifyBlockSessionInput {
                    block_level: 5,
                    pivot: [1u8; 32],
                    tips: vec![],
                    block_transaction_hashes: vec![
                        DagTransactionHash { hash: [2u8; 32] },
                        DagTransactionHash { hash: [3u8; 32] },
                    ],
                    supplied_transaction_hashes: vec![DagTransactionHash { hash: [3u8; 32] }],
                },
            )
            .expect("session should initialize");

            let first = dag_manager_runtime_verify_block_session_next(&mut runtime);
            assert_eq!(first.status, DAG_VERIFY_SESSION_STATUS_ACTIVE);
            assert_eq!(first.action, DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY);
            assert_eq!(first.proposal_period, 7);
            assert_eq!(first.query_hashes[0].hash, [2u8; 32]);

            let auth = dag_manager_runtime_verify_block_session_report_transactions(
                &mut runtime,
                DagVerifyBlockTransactionReport {
                    resolved_transactions: 2,
                },
            );
            assert_eq!(auth.action, DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS);

            let vdf = dag_manager_runtime_verify_block_session_report_authorization(
                &mut runtime,
                DagVerifyBlockAuthorizationReport {
                    vrf_key_found: true,
                    sender_eligible_vote_count: 11,
                    vdf_sortition_max_vote_count: 33,
                    eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
                },
            );
            assert_eq!(vdf.action, DAG_VERIFY_SESSION_ACTION_VDF_SORTITION);
            assert_eq!(vdf.vote_count, 11);
            assert_eq!(vdf.max_vote_count, 33);

            let gas = dag_manager_runtime_verify_block_session_report_vdf(
                &mut runtime,
                DagVerifyBlockVdfReport {
                    vdf_status: rustaxa_consensus::dag::DAG_VERIFY_VDF_STATUS_VALID,
                },
            );
            assert_eq!(gas.action, DAG_VERIFY_SESSION_ACTION_GAS);

            let complete = dag_manager_runtime_verify_block_session_report_gas(
                &mut runtime,
                DagVerifyBlockGasReport {
                    block_gas_estimation: 10,
                    estimated_transactions_weight: 10,
                    dag_gas_limit: 20,
                    pbft_gas_limit: 100,
                    tip_gas_estimations: vec![],
                },
            );
            assert!(complete.complete);
            assert_eq!(complete.status, DAG_VERIFY_SESSION_STATUS_COMPLETE);
            assert_eq!(complete.reject_code, 0);

            dag_manager_runtime_begin_verify_block_session(
                &mut runtime,
                DagVerifyBlockSessionInput {
                    block_level: 5,
                    pivot: [1u8; 32],
                    tips: vec![],
                    block_transaction_hashes: vec![DagTransactionHash { hash: [4u8; 32] }],
                    supplied_transaction_hashes: vec![],
                },
            )
            .expect("missing session should initialize");
            let _ = dag_manager_runtime_verify_block_session_next(&mut runtime);
            let missing = dag_manager_runtime_verify_block_session_report_transactions(
                &mut runtime,
                DagVerifyBlockTransactionReport {
                    resolved_transactions: 0,
                },
            );
            assert!(missing.complete);
            assert_eq!(
                missing.reject_code,
                rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_set_finalized_order_updates_graph_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_set_finalized_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block 2");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![DagHash { hash: [1u8; 32] }],
                    level: 3,
                    difficulty: 80,
                })
                .expect("add block 3");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![DagHash { hash: [3u8; 32] }],
                    level: 4,
                    difficulty: 60,
                })
                .expect("add block 4");

            let removed = runtime
                .dag_manager_runtime_set_finalized_order(
                    [4u8; 32],
                    4,
                    1,
                    vec![
                        DagHash { hash: [2u8; 32] },
                        DagHash { hash: [3u8; 32] },
                        DagHash { hash: [4u8; 32] },
                    ],
                )
                .expect("set finalized order should succeed");
            assert_eq!(removed.finalized_count, 3);
            assert!(removed.counter_update_hashes.is_empty());
            assert!(removed.expired_hashes.is_empty());
            assert!(removed.remaining_hashes.is_empty());

            let anchors = runtime.dag_manager_runtime_anchors();
            assert_eq!(anchors.old_anchor, [1u8; 32]);
            assert_eq!(anchors.anchor, [4u8; 32]);
            assert_eq!(runtime.dag_manager_runtime_latest_period(), 1);

            let non_finalized = runtime.dag_manager_runtime_non_finalized_blocks();
            assert!(non_finalized.is_empty());
            assert_eq!(
                runtime.dag_manager_runtime_non_finalized_min_difficulty(),
                u32::MAX
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_set_finalized_order_reports_expiry_plan() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_finalized_order_expiry");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 2, &storage)
                .expect("runtime should initialize");

            for block in [
                DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                },
                DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                },
                DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                },
                DagManagerBlock {
                    hash: [5u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 1,
                    difficulty: 70,
                },
                DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [5u8; 32],
                    tips: vec![],
                    level: 6,
                    difficulty: 60,
                },
            ] {
                runtime
                    .dag_manager_runtime_add_block(block)
                    .expect("add block");
            }

            let plan = runtime
                .dag_manager_runtime_set_finalized_order(
                    [4u8; 32],
                    4,
                    1,
                    vec![
                        DagHash { hash: [2u8; 32] },
                        DagHash { hash: [3u8; 32] },
                        DagHash { hash: [4u8; 32] },
                    ],
                )
                .expect("set finalized order should succeed");

            assert_eq!(
                plan.expired_hashes
                    .iter()
                    .map(|hash| hash.hash)
                    .collect::<Vec<_>>(),
                vec![[5u8; 32], [6u8; 32]]
            );
            assert!(plan.remaining_hashes.is_empty());
            assert_eq!(runtime.dag_manager_runtime_dag_expiry_level(), 2);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_select_non_finalized_hashes_excludes_known_hashes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_select_hashes");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 85,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                })
                .expect("add block");

            let selected = runtime.dag_manager_runtime_select_non_finalized_hashes(vec![
                DagHash { hash: [2u8; 32] },
                DagHash { hash: [9u8; 32] },
                DagHash { hash: [2u8; 32] },
            ]);
            let selected = selected.iter().map(|hash| hash.hash).collect::<Vec<_>>();
            assert_eq!(selected, vec![[3u8; 32], [4u8; 32], [6u8; 32]]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_empty_period_preserves_anchors() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_empty_period");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");

            let finalized_count = runtime
                .dag_manager_runtime_set_finalized_order([0u8; 32], 0, 1, vec![])
                .expect("empty period should advance");
            assert_eq!(finalized_count.finalized_count, 0);

            let anchors = runtime.dag_manager_runtime_anchors();
            assert_eq!(anchors.old_anchor, [0u8; 32]);
            assert_eq!(anchors.anchor, [1u8; 32]);
            assert_eq!(runtime.dag_manager_runtime_latest_period(), 1);
            assert_eq!(
                runtime
                    .dag_manager_runtime_non_finalized_blocks_size()
                    .blocks,
                1
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_non_finalized_sync_snapshot_includes_period_and_selected_hashes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_sync_snapshot");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 85,
                })
                .expect("add block");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                })
                .expect("add block");

            let snapshot = runtime.dag_manager_runtime_non_finalized_sync_snapshot(vec![
                DagHash { hash: [2u8; 32] },
                DagHash { hash: [9u8; 32] },
                DagHash { hash: [2u8; 32] },
            ]);
            let selected = snapshot
                .selected_hashes
                .into_iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>();

            assert_eq!(snapshot.period, 0);
            assert_eq!(selected, vec![[3u8; 32], [4u8; 32], [6u8; 32]]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_non_finalized_sync_payload_uses_storage_and_dedupes_transactions() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_sync_payload");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [2u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 2,
                    difficulty: 100,
                })
                .expect("add block 2");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [2u8; 32],
                    tips: vec![DagHash { hash: [2u8; 32] }],
                    level: 3,
                    difficulty: 90,
                })
                .expect("add block 3");
            runtime
                .dag_manager_runtime_add_block(DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![DagHash { hash: [3u8; 32] }],
                    level: 4,
                    difficulty: 80,
                })
                .expect("add block 4");

            let tx_block_3 = dag_block_with_vdf_payload_and_transaction_hashes(
                vec![0x11],
                &[tx_hash(1), tx_hash(2)],
            );
            let tx_block_4 = dag_block_with_vdf_payload_and_transaction_hashes(
                vec![0x22],
                &[tx_hash(2), tx_hash(4)],
            );

            runtime
                .dag_manager_runtime_save_block(&[3u8; 32], 3, 2, tx_block_3.clone())
                .expect("persist block 3");
            runtime
                .dag_manager_runtime_save_block(&[4u8; 32], 4, 2, tx_block_4.clone())
                .expect("persist block 4");

            storage
                .0
                .transaction()
                .write(H256::from([1u8; 32]), &[0xA1, 0x01])
                .expect("persist pending transaction 1");
            storage
                .0
                .transaction()
                .write(H256::from([2u8; 32]), &[0xA2, 0x02])
                .expect("persist pending transaction 2");
            storage
                .0
                .transaction()
                .write(H256::from([3u8; 32]), &[0xA3, 0x03])
                .expect("persist pending transaction 3");

            let payload = runtime
                .dag_manager_runtime_non_finalized_sync_payload(vec![DagHash { hash: [2u8; 32] }])
                .expect("sync payload should materialize");

            assert_eq!(payload.period, 0);
            assert_eq!(payload.blocks.len(), 2);
            assert_eq!(payload.blocks[0].hash, [3u8; 32]);
            assert_eq!(payload.blocks[0].block_rlp, tx_block_3);
            assert_eq!(payload.blocks[1].hash, [4u8; 32]);
            assert_eq!(payload.blocks[1].block_rlp, tx_block_4);

            assert_eq!(payload.transactions.len(), 3);
            assert_eq!(payload.transactions[0].hash, tx_hash(1).hash);
            assert!(payload.transactions[0].found);
            assert_eq!(payload.transactions[0].tx_rlp, vec![0xA1, 0x01]);
            assert_eq!(payload.transactions[1].hash, tx_hash(2).hash);
            assert!(payload.transactions[1].found);
            assert_eq!(payload.transactions[1].tx_rlp, vec![0xA2, 0x02]);
            assert_eq!(payload.transactions[2].hash, tx_hash(4).hash);
            assert!(!payload.transactions[2].found);
            assert!(payload.transactions[2].tx_rlp.is_empty());
            assert!(!payload.transactions[2].finalized);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_finalization_cleanup_payload_returns_storage_backed_side_effects() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_finalization_payload");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("runtime should initialize");

            runtime
                .dag_manager_runtime_save_block(
                    &[8u8; 32],
                    1,
                    0,
                    dag_block_with_vdf_payload(vec![0x88]),
                )
                .expect("persist finalized block needing counter update");
            runtime
                .dag_manager_runtime_save_block(
                    &[3u8; 32],
                    3,
                    3,
                    dag_block_with_vdf_payload_and_transaction_hashes(
                        vec![0x11],
                        &[tx_hash(1), tx_hash(2), tx_hash(1)],
                    ),
                )
                .expect("persist expired block a");
            runtime
                .dag_manager_runtime_save_block(
                    &[4u8; 32],
                    4,
                    1,
                    dag_block_with_vdf_payload_and_transaction_hashes(vec![0x22], &[tx_hash(3)]),
                )
                .expect("persist expired block b");
            runtime
                .dag_manager_runtime_save_block(
                    &[6u8; 32],
                    6,
                    1,
                    dag_block_with_vdf_payload_and_transaction_hashes(vec![0x33], &[tx_hash(3)]),
                )
                .expect("persist remaining block");

            storage
                .0
                .transaction()
                .write_location(H256::from([2u8; 32]), 7, 0, false)
                .expect("mark tx2 as finalized");

            let payload = runtime
                .dag_manager_runtime_finalization_cleanup_payload(DagManagerFinalizationPlan {
                    finalized_count: 2,
                    counter_update_hashes: vec![DagHash { hash: [8u8; 32] }],
                    expired_hashes: vec![DagHash { hash: [3u8; 32] }, DagHash { hash: [4u8; 32] }],
                    remaining_hashes: vec![DagHash { hash: [6u8; 32] }],
                })
                .expect("finalization cleanup payload should compute");

            assert_eq!(payload.counter_updates.len(), 1);
            assert_eq!(payload.counter_updates[0].hash, [8u8; 32]);
            assert_eq!(payload.counter_updates[0].level, 1);
            assert_eq!(payload.counter_updates[0].tips_count, 0);

            assert_eq!(payload.expired_hashes.len(), 2);
            assert_eq!(payload.expired_hashes[0].hash, [3u8; 32]);
            assert_eq!(payload.expired_hashes[1].hash, [4u8; 32]);

            assert_eq!(payload.remove_transaction_hashes.len(), 1);
            assert_eq!(payload.remove_transaction_hashes[0].hash, [1u8; 32]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_restore_from_storage_rebuilds_graph_without_legacy_snapshot() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_restore_from_storage");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let seed_runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("seed runtime should initialize");

            let anchor_rlp = dag_block_with_pivot_level_and_difficulty(H256::from([1u8; 32]), 3, 3);
            let anchor_facts =
                dag_manager_block_from_rlp(anchor_rlp.clone()).expect("anchor block facts");
            let anchor_hash = H256::from(anchor_facts.hash);
            let live_rlp = dag_block_with_pivot_level_and_difficulty(anchor_hash, 4, 4);
            let live_facts =
                dag_manager_block_from_rlp(live_rlp.clone()).expect("live block facts");

            let pbft_block = signed_pbft_block_with_pivot(1, 123, anchor_hash);
            let pbft_link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&pbft_block))
                .expect("pbft block link");
            storage
                .0
                .period()
                .write(1, &period_data_with_pbft_block(&pbft_block))
                .expect("persist period data");
            storage
                .0
                .period()
                .write_pbft_period(pbft_link.block_hash, 1)
                .expect("persist pbft period index");
            storage
                .0
                .pbft()
                .write_head(
                    H256::zero(),
                    format!(
                        r#"{{"head_hash":"0x{:064x}","size":1,"non_empty_size":1,"last_pbft_block_hash":"0x{:064x}"}}"#,
                        0, pbft_link.block_hash
                    )
                    .as_bytes(),
                )
                .expect("persist pbft head");

            seed_runtime
                .dag_manager_runtime_save_block(
                    &anchor_facts.hash,
                    anchor_facts.level,
                    anchor_facts.tips.len() as u64,
                    anchor_rlp,
                )
                .expect("persist anchor block");
            seed_runtime
                .dag_manager_runtime_save_block(
                    &live_facts.hash,
                    live_facts.level,
                    live_facts.tips.len() as u64,
                    live_rlp,
                )
                .expect("persist non-finalized block");

            let mut restored = create_dag_manager_runtime_from_storage(&[1u8; 32], 32, &storage)
                .expect("restored runtime should initialize");
            restored
                .dag_manager_runtime_restore_from_storage()
                .expect("restore from storage should succeed");

            assert_eq!(restored.dag_manager_runtime_latest_period(), 1);
            assert_eq!(
                restored.dag_manager_runtime_anchors().anchor,
                anchor_facts.hash
            );
            assert!(restored
                .dag_manager_runtime_is_block_known(&live_facts.hash)
                .expect("knownness should query Rust storage"));
            assert_eq!(restored.dag_manager_runtime_max_level(), 4);
            assert_eq!(
                restored.dag_manager_runtime_non_finalized_min_difficulty(),
                3
            );
            assert_eq!(
                restored
                    .dag_manager_runtime_non_finalized_blocks_size()
                    .blocks,
                2
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_apply_finalized_order_writes_storage_and_commits_state() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_apply_finalized_order");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 1, &storage)
                .expect("runtime should initialize");

            for block in [
                DagManagerBlock {
                    hash: [3u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 3,
                    difficulty: 90,
                },
                DagManagerBlock {
                    hash: [4u8; 32],
                    pivot: [3u8; 32],
                    tips: vec![],
                    level: 4,
                    difficulty: 80,
                },
                DagManagerBlock {
                    hash: [6u8; 32],
                    pivot: [1u8; 32],
                    tips: vec![],
                    level: 6,
                    difficulty: 70,
                },
            ] {
                runtime
                    .dag_manager_runtime_add_block(block)
                    .expect("add non-finalized block");
            }

            runtime
                .dag_manager_runtime_save_block(
                    &[8u8; 32],
                    5,
                    0,
                    dag_block_with_level_and_transaction_hashes(5, vec![0x88], &[]),
                )
                .expect("persist finalized anchor block");
            runtime
                .dag_manager_runtime_save_block(
                    &[3u8; 32],
                    3,
                    0,
                    dag_block_with_level_and_transaction_hashes(
                        3,
                        vec![0x11],
                        &[tx_hash(1), tx_hash(2)],
                    ),
                )
                .expect("persist expired block a");
            runtime
                .dag_manager_runtime_save_block(
                    &[4u8; 32],
                    4,
                    0,
                    dag_block_with_level_and_transaction_hashes(4, vec![0x22], &[tx_hash(3)]),
                )
                .expect("persist expired dependent block");
            runtime
                .dag_manager_runtime_save_block(
                    &[6u8; 32],
                    6,
                    0,
                    dag_block_with_level_and_transaction_hashes(6, vec![0x33], &[tx_hash(3)]),
                )
                .expect("persist remaining block");

            storage
                .0
                .transaction()
                .write_location(H256::from([2u8; 32]), 7, 0, false)
                .expect("mark tx2 as finalized");
            storage
                .0
                .transaction()
                .write(H256::from([1u8; 32]), &[0xA1])
                .expect("persist expired pending tx1");
            storage
                .0
                .transaction()
                .write(H256::from([2u8; 32]), &[0xA2])
                .expect("persist finalized pending tx2");
            storage
                .0
                .transaction()
                .write(H256::from([3u8; 32]), &[0xA3])
                .expect("persist retained pending tx3");

            let payload = runtime
                .dag_manager_runtime_apply_finalized_order(
                    [8u8; 32],
                    1,
                    vec![DagHash { hash: [8u8; 32] }],
                )
                .expect("apply finalized order");

            assert_eq!(payload.finalized_count, 1);
            assert_eq!(payload.expired_hashes.len(), 2);
            assert_eq!(payload.expired_hashes[0].hash, [3u8; 32]);
            assert_eq!(payload.expired_hashes[1].hash, [4u8; 32]);
            assert_eq!(payload.remove_transaction_hashes.len(), 1);
            assert_eq!(payload.remove_transaction_hashes[0].hash, [1u8; 32]);
            assert_eq!(
                transaction_queries(&storage)
                    .get_transaction(&[1u8; 32])
                    .expect("load removed pending tx1"),
                Vec::<u8>::new()
            );
            assert_eq!(
                transaction_queries(&storage)
                    .get_transaction(&[2u8; 32])
                    .expect("load finalized pending tx2"),
                vec![0xA2]
            );
            assert_eq!(
                transaction_queries(&storage)
                    .get_transaction(&[3u8; 32])
                    .expect("load retained pending tx3"),
                vec![0xA3]
            );

            assert_eq!(runtime.dag_manager_runtime_latest_period(), 1);
            assert_eq!(runtime.dag_manager_runtime_anchors().anchor, [8u8; 32]);
            assert!(
                !runtime
                    .dag_manager_runtime_load_block(&[3u8; 32])
                    .expect("load removed block")
                    .found
            );
            assert!(
                !runtime
                    .dag_manager_runtime_load_block(&[4u8; 32])
                    .expect("load removed dependent block")
                    .found
            );
            assert!(
                runtime
                    .dag_manager_runtime_load_block(&[6u8; 32])
                    .expect("load remaining block")
                    .found
            );

            let counters = runtime
                .dag_manager_runtime_persistence_counters()
                .expect("load counters");
            assert_eq!(counters.dag_blocks, 5);
            assert_eq!(counters.dag_edges, 5);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_manager_runtime_apply_finalized_order_requires_anchor_in_storage_before_state_commit() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_dag_runtime_apply_missing_anchor");

        {
            let storage = create_storage(temp_dir.to_str().expect("utf-8 path"))
                .expect("storage should initialize");
            let mut runtime = create_dag_manager_runtime_from_storage(&[1u8; 32], 1, &storage)
                .expect("runtime should initialize");

            let err = match runtime.dag_manager_runtime_apply_finalized_order(
                [8u8; 32],
                1,
                vec![DagHash { hash: [8u8; 32] }],
            ) {
                Ok(_) => panic!("missing anchor should fail"),
                Err(err) => err,
            };

            assert!(err
                .to_string()
                .contains("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK"));
            assert_eq!(runtime.dag_manager_runtime_latest_period(), 0);
            assert_eq!(runtime.dag_manager_runtime_anchors().anchor, [1u8; 32]);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn dag_verify_vdf_sortition_from_block_constructs_and_verifies_embedded_inputs() {
        let sortition_input = LegacySortitionParams {
            vrf_threshold_upper: 0x5ff,
            vdf_difficulty_min: 5,
            vdf_difficulty_max: 10,
            vdf_difficulty_stale: 9,
            vdf_lambda_bound: 64,
        };
        let proposal_period_hash = [9u8; 32];
        let block_level = 1;
        let vrf_input = dag::construct_dag_vrf_input(block_level, H256::from(proposal_period_hash));
        let block_rlp = dag_block_with_vdf_payload(vec![0; 0]);
        let vdf_input = dag::construct_dag_vdf_message_from_block_rlp(&block_rlp)
            .expect("VDF input should build");

        let proof = sortition::prove_legacy_vdf_sortition(
            sortition_input,
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .expect("proof generation should succeed");

        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&&proof.vrf_proof[..]);
        vdf_payload.append(&proof.vdf_proof);
        vdf_payload.append(&proof.vdf_output);
        vdf_payload.append(&proof.difficulty);

        let block_rlp = dag_block_with_vdf_payload(vdf_payload.out().to_vec());
        let vrf_public_key =
            public_key_from_secret(&SECRET_KEY).expect("VRF public key should derive");

        let result = dag_verify_vdf_sortition_from_block(DagVerifyVdfSortitionFromBlockInput {
            block_rlp,
            block_level,
            proposal_period_hash,
            sortition_params: SortitionRuntimeParams {
                threshold_upper: 0x5ff,
                difficulty_min: 5,
                difficulty_max: 10,
                difficulty_stale: 9,
                lambda_bound: 64,
            },
            vrf_public_key,
            sender_eligible_vote_count: 1,
            vdf_sortition_max_vote_count: 1,
        })
        .expect("embedded bridge verification should succeed");

        assert_eq!(result.vdf_status, dag::DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(result.difficulty, result.expected_difficulty);
    }

    #[test]
    fn dag_vdf_message_bridge_uses_legacy_pivot_and_transaction_rlp() {
        let pivot = [0x11_u8; 32];
        let tx_hashes = vec![
            DagHash {
                hash: [0x22_u8; 32],
            },
            DagHash {
                hash: [0x33_u8; 32],
            },
        ];

        let mut expected = RlpStream::new();
        expected.append(&H256::from(pivot));
        expected.append(&H256::from(tx_hashes[0].hash));
        expected.append(&H256::from(tx_hashes[1].hash));

        assert_eq!(dag_vdf_message(&pivot, tx_hashes), expected.out().to_vec());
    }

    #[test]
    fn dag_manager_block_from_rlp_bridge_decodes_hash_level_tips_and_difficulty() {
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);
        vdf_payload.append(&7u16);
        let block_rlp = dag_block_with_level_and_transaction_hashes(
            9,
            vdf_payload.out().to_vec(),
            &[DagTransactionHash { hash: [0x44; 32] }],
        );

        let facts = dag_manager_block_from_rlp(block_rlp).expect("manager facts");

        assert_ne!(facts.hash, [0; 32]);
        assert_eq!(facts.pivot, [0u8; 32]);
        assert_eq!(facts.level, 9);
        assert_eq!(facts.tips.len(), 0);
        assert_eq!(facts.difficulty, 7);
    }
}
