use crate::ffi::rustaxa_ffi::*;
use crate::ffi::{BridgeFinalChain, BridgeStorage};
use crate::transaction_manager::{
    bridge_to_service_account_nonce_facts, bridge_to_service_final_chain_admission_fact,
    bridge_to_service_gas_estimation_request, bridge_to_service_queue_entry,
    bridge_to_service_transaction_view_requests, bridge_to_service_validated_admission_fact,
    consensus_verify_transaction_fact_from_ffi_fact, domain_gas_pricer_config,
    service_public_admission_to_bridge, service_to_bridge_gas_estimation_plan,
    service_to_bridge_transaction_view, service_to_bridge_transaction_view_plan,
    service_transaction_groups_to_bridge,
};
use anyhow::{Context, Result};
use ethereum_types::{H256, U256};
use rustaxa_consensus::dag_service::{
    DagProposerAddBlockReport as NativeDagProposerAddBlockReport,
    DagProposerSessionAction as NativeDagProposerSessionAction,
    DagProposerSessionBeginInput as NativeDagProposerSessionBeginInput,
    DagProposerSessionStep as NativeDagProposerSessionStep,
    DagProposerSigningReport as NativeDagProposerSigningReport,
    DagProposerVdfProofReport as NativeDagProposerVdfProofReport, DagServiceConfig,
    DagVerifyBlockGasReport as NativeDagVerifyBlockGasReport,
    DagVerifyBlockSessionInput as NativeDagVerifyBlockSessionInput,
    DagVerifyBlockSessionStep as NativeDagVerifyBlockSessionStep,
};
use rustaxa_consensus::dag_transaction_service::{
    DagAddBlockAccountNonceFact as NativeDagAddBlockAccountNonceFact,
    DagAddBlockCompletion as NativeDagAddBlockCompletion,
    DagAddBlockPrepareRequest as NativeDagAddBlockPrepareRequest,
    DagAddBlockTransactionPayload as NativeDagAddBlockTransactionPayload, DagGhostPathRoot,
    DagGraphView, DagProposerFinalChainFacts as NativeDagProposerFinalChainFacts,
    DagProposerFinalChainRequestOrStep as NativeDagProposerFinalChainRequestOrStep,
    DagProposerPackPrepareRequest, DagProposerPackStep, DagTransactionService,
    DagTransactionServiceConfig,
    DagVerifyBlockAuthorizationRequestOrStep as NativeDagVerifyBlockAuthorizationRequestOrStep,
    DagVerifyBlockTransactionCompletionReport as NativeDagVerifyBlockTransactionCompletionReport,
    DagVerifyBlockVdfRequest as NativeDagVerifyBlockVdfRequest,
};
use rustaxa_consensus::sortition::SortitionServiceGuard;
use rustaxa_consensus::transaction_packing_service::{
    TransactionPackingEstimate, TransactionPackingSelection,
};
#[cfg(test)]
use rustaxa_consensus::transaction_service::TransactionServiceGuard;
use rustaxa_consensus::transaction_service::{
    DagTransactionSaveInput, TransactionServiceAccountNonceFact,
    TransactionServiceCompatibilityPackFinalized, TransactionServiceCompatibilityPackPrepared,
    TransactionServiceCompatibilityPackRequest, TransactionServiceConfig,
    TransactionServiceEstimateRequest, TransactionServiceFinalizedFilterRequest,
    TransactionServiceFinalizedStatusFact, TransactionServiceGasEstimationResult,
    TransactionServicePackEstimate, TransactionServicePayload, TransactionServiceTransactionView,
    TransactionServiceVerifyNotFinalizedFact,
};

/// CXX wrapper over the native DAG application root.
///
/// [`DagTransactionService`] owns sibling construction, restoration, lifetime,
/// and lock order. This bridge type retains only CXX conversion and temporary
/// FFI-shaped task adapters. Proposer and verification fact collection releases
/// every native guard for external FinalChain, EVM, VDF, signing, network, and
/// callback work, then reacquires through the native root and revalidates the
/// exact cursor before applying results.
pub struct BridgeDagTransactionService {
    root: DagTransactionService,
}

impl BridgeDagTransactionService {
    #[cfg(test)]
    fn transaction(&self) -> TransactionServiceGuard<'_> {
        self.root
            .lock_transaction()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    #[cfg(test)]
    fn insert_test_queue_transaction(&self, input: TransactionQueueInsertInput) -> Result<()> {
        let proposable = input.proposable;
        self.root
            .lock_transaction()?
            .queue
            .insert(bridge_to_service_queue_entry(&input), proposable)?;
        Ok(())
    }

    /// Acquires the native sortition owner after any required DAG lock.
    ///
    /// The temporary guard preserves the canonical DAG-then-sortition lock
    /// order for coupled cursor revalidation. It must not cross an external
    /// executor call.
    pub(crate) fn sortition(&self) -> Result<SortitionServiceGuard<'_>> {
        self.root.lock_sortition()
    }
}

/// Converts CXX configuration and constructs the native application root.
///
/// All restoration, shared storage ownership, error ordering, and publication
/// are owned by [`DagTransactionService`]. This wrapper is returned only for the
/// remaining named C++ DAG, transaction, sortition, gas, and PBFT clients.
pub fn create_dag_transaction_service_from_storage(
    storage: &BridgeStorage,
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
    max_levels_per_period: u64,
    sortition_config: SortitionRuntimeConfig,
    transaction_queue_config: TransactionQueueConfig,
    gas_pricer_config: GasPricerConfig,
    proposal_dag_gas_limit: u64,
) -> Result<Box<BridgeDagTransactionService>> {
    let root = DagTransactionService::restore(
        storage.0.clone(),
        DagTransactionServiceConfig {
            transaction: TransactionServiceConfig {
                queue_max_size: transaction_queue_config.max_size,
                gas_pricer_config: domain_gas_pricer_config(gas_pricer_config),
                proposal_dag_gas_limit,
            },
            dag: DagServiceConfig {
                genesis_hash: H256::from(*genesis),
                dag_expiry_limit,
                max_levels_per_period,
            },
            sortition: sortition_config.into(),
        },
    )?;
    Ok(Box::new(BridgeDagTransactionService { root }))
}

impl BridgeDagTransactionService {
    pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
        &self,
        fact: TransactionManagerValidatedInsertRuntimeFact,
        final_chain_fact: TransactionManagerFinalChainAdmissionFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerAdmissionCommandReport> {
        Ok(crate::transaction_manager::service_admission_to_bridge(
            self.root.transaction_execute_admission(
                bridge_to_service_validated_admission_fact(fact),
                bridge_to_service_final_chain_admission_fact(final_chain_fact),
                bridge_to_service_queue_entry(&input),
            )?,
        ))
    }

    pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(
        &self,
        verify_fact: TransactionManagerVerifyTransactionFact,
        admission_fact: TransactionManagerValidatedInsertRuntimeFact,
        final_chain_fact: TransactionManagerFinalChainAdmissionFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerPublicAdmissionCommandReport> {
        Ok(service_public_admission_to_bridge(
            self.root.transaction_execute_public_admission(
                consensus_verify_transaction_fact_from_ffi_fact(verify_fact),
                bridge_to_service_validated_admission_fact(admission_fact),
                bridge_to_service_final_chain_admission_fact(final_chain_fact),
                bridge_to_service_queue_entry(&input),
            )?,
        ))
    }

    pub fn transaction_manager_runtime_gas_price_update(&self, gas_prices: Vec<GasPricerGasPrice>) {
        self.root
            .transaction_update_gas_prices(
                gas_prices
                    .into_iter()
                    .map(|gas_price| U256::from_big_endian(&gas_price.price))
                    .collect(),
            )
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED");
    }

    #[allow(clippy::too_many_arguments)]
    pub fn transaction_manager_runtime_pack_prepare_sharded(
        &self,
        weight_limit: u64,
        min_transaction_gas: u64,
        proposal_period: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
        total_shards: u16,
        node_shard: u16,
        shard_period_interval: u64,
    ) -> Result<TransactionPackPreparedPlan> {
        Ok(service_pack_prepared_to_bridge(
            self.root.transaction_prepare_compatibility_pack(
                TransactionServiceCompatibilityPackRequest {
                    weight_limit,
                    min_transaction_gas,
                    proposal_period,
                    estimate_gas_limit,
                    last_block_number,
                    total_shards,
                    node_shard,
                    shard_period_interval,
                },
            )?,
        ))
    }

    pub fn transaction_manager_runtime_pack_finalize_with_estimates(
        &self,
        inputs: Vec<TransactionPackSessionEstimateInput>,
    ) -> Result<TransactionPackSessionStep> {
        Ok(service_pack_finalized_to_bridge(
            self.root.transaction_finalize_compatibility_pack(
                inputs
                    .into_iter()
                    .map(|input| TransactionServicePackEstimate {
                        hash: H256::from(input.hash),
                        gas_used: input.gas_used,
                        last_block_number: input.last_block_number,
                        result_rlp: input.result_rlp,
                    })
                    .collect(),
            )?,
        ))
    }

    pub fn transaction_manager_runtime_pack_abort(&self) -> bool {
        match self.root.transaction_abort_compatibility_pack() {
            Ok(aborted) => aborted,
            Err(error)
                if error
                    .to_string()
                    .contains("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED") =>
            {
                panic!("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
            }
            Err(error) => panic!("TM_RUNTIME_PACKING_LOCK_POISONED: {error}"),
        }
    }

    pub fn transaction_manager_runtime_store_gas_estimation(
        &self,
        result: TransactionManagerGasEstimationResult,
    ) -> Result<bool> {
        self.root
            .transaction_store_gas_estimation(TransactionServiceGasEstimationResult {
                hash: H256::from(result.hash),
                proposal_period: result.proposal_period,
                gas_used: result.gas_used,
                result_rlp: result.result_rlp,
            })
    }

    pub fn transaction_manager_runtime_initialize_recently_finalized_payloads(
        &self,
        period: u64,
        payloads: Vec<TransactionManagerSidecarInsertInput>,
    ) -> Result<()> {
        self.root.transaction_initialize_recently_finalized(
            period,
            payloads
                .into_iter()
                .map(|payload| TransactionServicePayload {
                    hash: H256::from(payload.hash),
                    transaction_rlp: payload.trx_rlp,
                })
                .collect(),
        )
    }

    pub fn transaction_manager_runtime_remove_non_finalized(
        &self,
        requests: Vec<TransactionManagerSidecarLookupRequest>,
    ) -> Result<u64> {
        self.root.transaction_remove_non_finalized(
            requests
                .into_iter()
                .map(|request| H256::from(request.hash))
                .collect(),
        )
    }

    pub fn transaction_manager_runtime_queue_block_finalized(
        &self,
        block_number: u64,
    ) -> Vec<TransactionQueueHash> {
        self.root
            .transaction_queue_block_finalized(block_number)
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
            .into_iter()
            .map(|hash| TransactionQueueHash { hash: hash.0 })
            .collect()
    }

    pub fn transaction_manager_runtime_gas_price_bid(&self) -> [u8; 32] {
        self.root
            .transaction_gas_price_bid()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_plan_gas_estimation(
        &self,
        fact: TransactionManagerGasEstimationFact,
    ) -> Result<TransactionManagerGasEstimationPlan> {
        Ok(service_to_bridge_gas_estimation_plan(
            self.root
                .transaction_plan_gas_estimation(bridge_to_service_gas_estimation_request(fact))?,
        ))
    }

    pub fn transaction_manager_runtime_transaction_count(&self) -> u64 {
        self.root
            .transaction_count()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_is_transaction_known_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<bool> {
        self.root.transaction_is_known(*hash)
    }

    pub fn transaction_manager_runtime_non_finalized_size(&self) -> usize {
        self.root
            .transaction_non_finalized_size()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_queue_lookup_transaction_views(
        &self,
        requests: Vec<TransactionManagerTransactionViewRequest>,
    ) -> Result<Vec<TransactionManagerTransactionView>> {
        Ok(self
            .root
            .transaction_queue_views(bridge_to_service_transaction_view_requests(requests))?
            .into_iter()
            .map(service_to_bridge_transaction_view)
            .collect())
    }

    pub fn transaction_manager_runtime_queue_all_transaction_groups(
        &self,
    ) -> Vec<TransactionQueueTransactionGroup> {
        service_transaction_groups_to_bridge(
            self.root
                .transaction_queue_groups()
                .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED"),
        )
    }

    pub fn transaction_manager_runtime_queue_size(&self) -> usize {
        self.root
            .transaction_queue_size()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_queue_proposable_accounts(
        &self,
    ) -> Vec<TransactionQueueProposableAccountFact> {
        self.root
            .transaction_queue_proposable_accounts()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
            .into_iter()
            .map(|sender| TransactionQueueProposableAccountFact { sender: sender.0 })
            .collect()
    }

    pub fn transaction_manager_runtime_queue_transactions_dropped(&self) -> bool {
        self.root
            .transaction_queue_dropped()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_queue_non_proposable_over_limit(&self) -> bool {
        self.root
            .transaction_queue_non_proposable_over_limit()
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(
        &self,
        limit: u64,
    ) -> [u8; 32] {
        self.root
            .transaction_queue_min_gas_price(limit)
            .expect("DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED")
    }

    pub fn transaction_manager_runtime_lookup_non_finalized_transaction_views(
        &self,
        requests: Vec<TransactionManagerTransactionViewRequest>,
    ) -> Result<Vec<TransactionManagerTransactionView>> {
        Ok(self
            .root
            .transaction_non_finalized_views(bridge_to_service_transaction_view_requests(requests))?
            .into_iter()
            .map(service_to_bridge_transaction_view)
            .collect())
    }

    pub fn transaction_manager_runtime_lookup_transaction_views(
        &self,
        requests: Vec<TransactionManagerTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionManagerTransactionViewPlan> {
        Ok(service_to_bridge_transaction_view_plan(
            self.root.transaction_views(
                bridge_to_service_transaction_view_requests(requests),
                max_count,
            )?,
        ))
    }

    pub fn transaction_manager_runtime_lookup_proposal_transaction_views_with_account_nonce_facts(
        &self,
        proposal_period: u64,
        requests: Vec<TransactionManagerTransactionViewRequest>,
        account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
        max_count: u64,
    ) -> Result<TransactionManagerTransactionViewPlan> {
        Ok(service_to_bridge_transaction_view_plan(
            self.root.proposal_transaction_views(
                proposal_period,
                bridge_to_service_transaction_view_requests(requests),
                bridge_to_service_account_nonce_facts(account_nonce_facts),
                max_count,
            )?,
        ))
    }

    pub fn dag_transaction_service_proposer_pack_prepare(
        &self,
        session_id: u64,
        network_throttled: bool,
        min_transaction_gas: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
    ) -> Result<DagProposerSessionStep> {
        dag_transaction_service_proposer_pack_prepare(
            self,
            session_id,
            network_throttled,
            min_transaction_gas,
            estimate_gas_limit,
            last_block_number,
        )
    }

    pub fn dag_transaction_service_proposer_pack_finalize(
        &self,
        session_id: u64,
        estimates: Vec<TransactionPackSessionEstimateInput>,
    ) -> Result<DagProposerSessionStep> {
        dag_transaction_service_proposer_pack_finalize(self, session_id, estimates)
    }

    pub fn dag_transaction_service_proposer_pack_abort(&self, session_id: u64) -> Result<bool> {
        dag_transaction_service_proposer_pack_abort(self, session_id)
    }

    /// Commits finalized DAG cleanup, then clears matching private transaction sidecars.
    ///
    /// Both runtimes are locked DAG then transaction. Transaction state remains untouched when the fallible DAG/storage
    /// phase fails. After a successful DAG commit, sidecar removal is infallible because storage deletion already
    /// occurred inside the DAG runtime. Only finalized count and expired DAG hashes cross CXX.
    pub fn dag_manager_runtime_apply_finalized_order(
        &self,
        new_anchor: [u8; 32],
        new_period: u64,
        finalized_order: Vec<DagHash>,
    ) -> Result<DagManagerFinalizationApplyPayload> {
        let report = self.root.apply_finalized_order(
            H256::from(new_anchor),
            new_period,
            finalized_order
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        )?;
        Ok(DagManagerFinalizationApplyPayload {
            finalized_count: u64::try_from(report.finalized_count)
                .context("DAG_RUNTIME_FINALIZATION_COUNT_OVERFLOW")?,
            expired_hashes: report
                .expired_hashes
                .into_iter()
                .map(|hash| DagHash { hash: hash.0 })
                .collect(),
        })
    }

    pub fn dag_manager_runtime_validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> Result<DagPivotTipsValidation> {
        let validation = self.root.dag_validate_references(
            block_level,
            H256::from(*pivot),
            tips.into_iter().map(|hash| H256::from(hash.hash)).collect(),
        )?;
        Ok(DagPivotTipsValidation {
            ok: validation.ok,
            expected_level: validation.expected_level,
            level_matches: validation.level_matches,
            missing_references: to_dag_hashes(validation.missing_references),
        })
    }

    pub fn dag_manager_runtime_non_finalized_sync_payload(
        &self,
        known_hashes: Vec<DagHash>,
    ) -> Result<DagManagerNonFinalizedSyncPayload> {
        let payload = self.root.dag_non_finalized_sync(
            known_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        )?;
        Ok(DagManagerNonFinalizedSyncPayload {
            period: payload.period,
            blocks: payload
                .storage
                .blocks
                .into_iter()
                .map(|block| DagSyncBlockRlp {
                    hash: block.hash.into(),
                    block_rlp: block.block_rlp,
                })
                .collect(),
            transactions: payload
                .storage
                .transactions
                .into_iter()
                .map(|lookup| DagTransactionRlpLookup {
                    hash: lookup.hash.into(),
                    found: lookup.found,
                    finalized: lookup.finalized,
                    tx_rlp: lookup.tx_rlp,
                })
                .collect(),
        })
    }

    pub fn dag_manager_runtime_compute_order(&self, anchor: &[u8; 32]) -> Result<DagOrder> {
        match self.root.dag_order(H256::from(*anchor))? {
            Some(hashes) => Ok(DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            }),
            None => Ok(DagOrder {
                found: false,
                hashes: Vec::new(),
            }),
        }
    }

    pub fn dag_manager_runtime_frontier(&self) -> Result<DagFrontier> {
        let frontier = self.root.dag_frontier()?;
        Ok(DagFrontier {
            pivot: frontier.pivot.into(),
            tips: to_dag_hashes(frontier.tips),
        })
    }

    pub fn dag_manager_runtime_ghost_path(&self, source: &[u8; 32]) -> Result<Vec<DagHash>> {
        let path = self
            .root
            .dag_ghost_path(DagGhostPathRoot::Block(H256::from(*source)))?;
        Ok(to_dag_hashes(path))
    }

    pub fn dag_manager_runtime_anchor_ghost_path(&self) -> Result<Vec<DagHash>> {
        let path = self.root.dag_ghost_path(DagGhostPathRoot::CurrentAnchor)?;
        Ok(to_dag_hashes(path))
    }

    pub fn dag_manager_runtime_graphviz_dot(&self, pivot_tree: bool) -> Result<String> {
        self.root.dag_graphviz(if pivot_tree {
            DagGraphView::PivotTree
        } else {
            DagGraphView::Complete
        })
    }

    pub fn dag_manager_runtime_vertex_count(&self) -> Result<usize> {
        usize::try_from(self.root.dag_runtime_status()?.vertex_count)
            .context("DAG_RUNTIME_VERTEX_COUNT_PLATFORM_OVERFLOW")
    }

    pub fn dag_manager_runtime_edge_count(&self) -> Result<usize> {
        usize::try_from(self.root.dag_runtime_status()?.edge_count)
            .context("DAG_RUNTIME_EDGE_COUNT_PLATFORM_OVERFLOW")
    }

    pub fn dag_manager_runtime_max_level(&self) -> Result<u64> {
        Ok(self.root.dag_runtime_status()?.max_level)
    }

    pub fn dag_manager_runtime_latest_period(&self) -> Result<u64> {
        Ok(self.root.dag_runtime_status()?.period)
    }

    pub fn dag_manager_runtime_anchors(&self) -> Result<DagManagerAnchors> {
        let anchors = self.root.dag_runtime_status()?.anchors;
        Ok(DagManagerAnchors {
            old_anchor: anchors.old.into(),
            anchor: anchors.current.into(),
        })
    }

    pub fn dag_manager_runtime_dag_expiry_limit(&self) -> Result<u32> {
        Ok(self.root.dag_runtime_status()?.expiry_limit)
    }

    pub fn dag_manager_runtime_dag_expiry_level(&self) -> Result<u64> {
        Ok(self.root.dag_runtime_status()?.expiry_level)
    }

    pub fn dag_manager_runtime_non_finalized_blocks(&self) -> Result<Vec<DagLevelHashes>> {
        let blocks = self
            .root
            .dag_non_finalized_index()?
            .levels
            .into_iter()
            .map(|level| DagLevelHashes {
                level: level.level,
                hashes: to_dag_hashes(level.hashes),
            })
            .collect();
        Ok(blocks)
    }

    pub fn dag_manager_runtime_non_finalized_blocks_size(
        &self,
    ) -> Result<DagManagerNonFinalizedSize> {
        let summary = self.root.dag_non_finalized_summary()?;
        Ok(DagManagerNonFinalizedSize {
            levels: summary.levels,
            blocks: summary.blocks,
        })
    }

    pub fn dag_manager_runtime_non_finalized_min_difficulty(&self) -> Result<u32> {
        Ok(self.root.dag_non_finalized_summary()?.min_difficulty)
    }

    pub fn dag_manager_runtime_is_block_known(&self, hash: &[u8; 32]) -> Result<bool> {
        self.root.dag_is_block_known(H256::from(*hash))
    }

    pub fn dag_manager_runtime_load_block(&self, hash: &[u8; 32]) -> Result<DagBlockLookup> {
        let lookup = self.root.dag_load_block(H256::from(*hash))?;
        Ok(DagBlockLookup {
            found: lookup.found,
            block_rlp: lookup.block_rlp,
        })
    }

    pub fn dag_manager_runtime_plan_proposal_tip_selection(
        &self,
        input: DagProposerStorageTipSelectionInput,
    ) -> Result<DagProposerTipSelectionPlan> {
        let plan = self.root.dag_select_proposer_tips(
            rustaxa_consensus::dag::DagProposerStorageTipSelectionInput {
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

    pub fn dag_manager_runtime_period_block_hash(&self, period: u64) -> Result<HashLookup> {
        let lookup = self.root.dag_period_block_hash(period)?;
        Ok(HashLookup {
            found: lookup.found,
            hash: lookup.hash.into(),
        })
    }

    pub fn dag_manager_runtime_persistence_counters(&self) -> Result<DagPersistenceCounters> {
        let counters = self.root.dag_persistence_counters()?;
        Ok(DagPersistenceCounters {
            dag_blocks: counters.dag_blocks,
            dag_edges: counters.dag_edges,
        })
    }

    /// Prepares one canonical add-block transition without mutating DAG,
    /// transaction, or storage state.
    pub fn dag_transaction_service_prepare_add_block(
        &self,
        input: DagAddBlockPrepareInput,
    ) -> Result<DagAddBlockPreparation> {
        let preparation = self
            .root
            .prepare_add_block(NativeDagAddBlockPrepareRequest {
                expected_hash: H256::from(input.expected_block_hash),
                block_rlp: input.block_rlp,
                validate_hash: input.validate_block_hash,
                save: input.save,
                proposed: input.proposed,
                transactions: input
                    .transactions
                    .into_iter()
                    .map(|payload| NativeDagAddBlockTransactionPayload {
                        hash: H256::from(payload.hash),
                        transaction_rlp: payload.trx_rlp,
                    })
                    .collect(),
            })?;
        Ok(DagAddBlockPreparation {
            cursor_id: preparation.cursor_id,
            block_level: preparation.block_level,
            accepted: preparation.accepted,
            duplicate: preparation.duplicate,
            expired: preparation.expired,
            missing_references: preparation
                .missing_references
                .into_iter()
                .map(|hash| DagHash { hash: hash.0 })
                .collect(),
            account_requests: preparation
                .account_requests
                .into_iter()
                .map(|request| DagAddBlockAccountRequest {
                    input_index: request.input_index,
                    sender: request.sender.0,
                })
                .collect(),
        })
    }

    /// Completes one prepared add-block transition through a single durable
    /// storage batch, then publishes prevalidated live state.
    pub fn dag_transaction_service_complete_add_block(
        &self,
        input: DagAddBlockCompletionInput,
    ) -> Result<DagAddBlockCommitReport> {
        let report = self.root.complete_add_block(NativeDagAddBlockCompletion {
            cursor_id: input.cursor_id,
            account_nonce_facts: input
                .account_nonce_facts
                .into_iter()
                .map(|fact| NativeDagAddBlockAccountNonceFact {
                    input_index: fact.input_index,
                    account_nonce: U256::from_big_endian(&fact.account_nonce),
                })
                .collect(),
        })?;
        Ok(DagAddBlockCommitReport {
            accepted: report.accepted,
            emit_verified: report.emit_verified,
            gossip: report.gossip,
            proposed: report.proposed,
            queue_erased: report
                .queue_erased
                .into_iter()
                .map(|hash| TransactionManagerHashCommand { hash: hash.0 })
                .collect(),
            counters: DagPersistenceCounters {
                dag_blocks: report.counters.dag_blocks,
                dag_edges: report.counters.dag_edges,
            },
        })
    }

    /// Idempotently aborts the matching add-block cursor without affecting a
    /// newer or unrelated preparation.
    pub fn dag_transaction_service_abort_add_block(&self, cursor_id: u64) -> Result<bool> {
        self.root.abort_add_block(cursor_id)
    }
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
        .collect()
}

pub fn service_save_transactions_from_dag_block_command_report_with_runtime(
    service: &BridgeDagTransactionService,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<TransactionManagerDagSaveCommandReport> {
    let outcome = service.root.transaction_save_dag_transactions(
        facts
            .into_iter()
            .map(|fact| DagTransactionSaveInput {
                input_index: fact.input_index,
                hash: H256::from(fact.hash),
                transaction_rlp: fact.trx_rlp,
                transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
                sender_account_nonce: U256::from_big_endian(&fact.sender_account_nonce),
            })
            .collect(),
    )?;
    Ok(crate::transaction_manager::dag_save_command_report(
        &outcome,
    ))
}

pub fn service_update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
    service: &BridgeDagTransactionService,
    period: u64,
    retention_window: u64,
    account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerFinalizedStatusCommandReport> {
    let report = service.root.transaction_update_finalized_status(
        period,
        retention_window,
        bridge_to_service_account_nonce_facts(account_nonce_facts),
        facts
            .into_iter()
            .map(|fact| TransactionServiceFinalizedStatusFact {
                input_index: fact.input_index,
                hash: H256::from(fact.hash),
                tx_rlp: fact.trx_rlp,
            })
            .collect(),
    )?;
    Ok(TransactionManagerFinalizedStatusCommandReport {
        removed_non_finalized: report
            .removed_non_finalized
            .into_iter()
            .map(|hash| TransactionManagerHashCommand { hash: hash.0 })
            .collect(),
        queue_erased: report
            .queue_erased
            .into_iter()
            .map(|hash| TransactionManagerHashCommand { hash: hash.0 })
            .collect(),
        finalized_account_purged: report
            .finalized_account_purged
            .into_iter()
            .map(|hash| TransactionManagerHashCommand { hash: hash.0 })
            .collect(),
        accepted_count: report.accepted_count,
        purge_transaction_queue: false,
    })
}

pub fn service_transaction_manager_filter_non_finalized_with_runtime(
    service: &BridgeDagTransactionService,
    requests: Vec<TransactionManagerSidecarLookupRequest>,
) -> Result<FinalizedTransactionFilterPlan> {
    Ok(FinalizedTransactionFilterPlan {
        not_finalized: service
            .root
            .transaction_filter_non_finalized(
                requests
                    .into_iter()
                    .map(|request| TransactionServiceFinalizedFilterRequest {
                        input_index: request.input_index,
                        hash: H256::from(request.hash),
                    })
                    .collect(),
            )?
            .not_finalized
            .into_iter()
            .map(|action| TransactionManagerFilterAction {
                input_index: action.input_index,
                hash: action.hash.0,
            })
            .collect(),
    })
}

pub fn service_transaction_manager_verify_not_finalized_with_runtime(
    service: &BridgeDagTransactionService,
    facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
    let outcome = service.root.transaction_verify_not_finalized(
        facts
            .into_iter()
            .map(|fact| TransactionServiceVerifyNotFinalizedFact {
                input_index: fact.input_index,
                hash: H256::from(fact.hash),
                transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
                sender_account_nonce: U256::from_big_endian(&fact.sender_account_nonce),
            })
            .collect(),
    )?;
    Ok(TransactionManagerVerifyNotFinalizedOutcome {
        is_finalized: outcome.is_finalized,
        input_index: outcome.input_index,
        hash: outcome.hash.0,
        source: outcome.source,
    })
}

pub fn service_transaction_manager_recover_nonfinalized_with_runtime(
    service: &BridgeDagTransactionService,
) -> Result<()> {
    service.root.transaction_recover_non_finalized().map(|_| ())
}

/// Prepares one DAG-owned transaction pack without exposing private pack configuration.
///
/// The service validates the DAG cursor before touching transaction state and locks DAG then transaction. A throttled
/// request never opens a transaction cursor. Otherwise Rust returns either EVM-only estimate candidates while retaining
/// both owner-bound cursors, or directly advances the DAG cursor when declared gas/cache facts complete packing. No lock
/// guard survives this call, so C++ may perform EVM callbacks after it returns. Any failure aborts both matching cursors.
pub fn dag_transaction_service_proposer_pack_prepare(
    service: &BridgeDagTransactionService,
    session_id: u64,
    network_throttled: bool,
    min_transaction_gas: u64,
    estimate_gas_limit: u64,
    last_block_number: u64,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .prepare_proposer_pack(DagProposerPackPrepareRequest {
            session_id,
            network_throttled,
            min_transaction_gas,
            estimate_gas_limit,
            last_block_number,
        })
        .map(proposer_pack_step_to_bridge)
}

/// Finalizes an owner-bound proposer pack after the unlocked EVM executor interval.
///
/// The bridge converts the plain estimator report once. Native code revalidates
/// and advances both owner-bound cursors under DAG-then-transaction locking.
pub fn dag_transaction_service_proposer_pack_finalize(
    service: &BridgeDagTransactionService,
    session_id: u64,
    estimates: Vec<TransactionPackSessionEstimateInput>,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .finalize_proposer_pack(
            session_id,
            estimates
                .into_iter()
                .map(|input| TransactionPackingEstimate {
                    hash: H256::from(input.hash),
                    gas_used: input.gas_used,
                    last_block_number: input.last_block_number,
                    result_rlp: input.result_rlp,
                })
                .collect(),
        )
        .map(proposer_pack_step_to_bridge)
}

/// Idempotently aborts the matching native transaction and DAG proposer cursors.
pub fn dag_transaction_service_proposer_pack_abort(
    service: &BridgeDagTransactionService,
    session_id: u64,
) -> Result<bool> {
    service.root.abort_proposer_pack(session_id)
}

fn proposer_pack_step_to_bridge(step: DagProposerPackStep) -> DagProposerSessionStep {
    let mut session = proposer_session_step_to_bridge(step.session);
    session.transaction_estimate_requests = step
        .estimate_requests
        .into_iter()
        .map(pack_candidate_from_request)
        .collect();
    session
}

fn proposer_session_step_to_bridge(step: NativeDagProposerSessionStep) -> DagProposerSessionStep {
    let action = match step.action {
        NativeDagProposerSessionAction::CollectFinalChainFacts => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS
        }
        NativeDagProposerSessionAction::PackTransactions => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS
        }
        NativeDagProposerSessionAction::StartVdf => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_START_VDF
        }
        NativeDagProposerSessionAction::CancelVdf => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF
        }
        NativeDagProposerSessionAction::StaleProofSleep => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP
        }
        NativeDagProposerSessionAction::SignBlock => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK
        }
        NativeDagProposerSessionAction::AddBlock => {
            crate::dag::DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK
        }
        NativeDagProposerSessionAction::Complete => crate::dag::DAG_PROPOSER_SESSION_ACTION_NONE,
    };
    let sortition_params = if matches!(step.action, NativeDagProposerSessionAction::StartVdf) {
        step.sortition_params
    } else {
        crate::dag::empty_sortition_params()
    };
    DagProposerSessionStep {
        status: step.status,
        action,
        reason_code: step.reason_code,
        return_value: step.return_value,
        update_retry_state: step.update_retry_state,
        next_last_propose_level: step.next_last_propose_level,
        next_retry_count: step.next_retry_count,
        frontier_pivot: step.frontier_pivot.0,
        proposal_level: step.proposal_level,
        proposal_period: step.proposal_period,
        last_finalized_period: step.last_finalized_period,
        vrf_input: step.vrf_input,
        vote_count: step.vote_count,
        max_vote_count: step.max_vote_count,
        vdf_difficulty: step.vdf_difficulty,
        vdf_sortition_params: crate::dag::legacy_sortition_params(sortition_params),
        vdf_stale: step.vdf_stale,
        old_proposal: step.old_proposal,
        vdf_message: step.vdf_message,
        selected_transaction_hashes: step
            .selected_transaction_hashes
            .into_iter()
            .map(|hash| DagHash { hash: hash.0 })
            .collect(),
        transaction_estimate_requests: Vec::new(),
        selected_transactions: step
            .selected_transactions
            .into_iter()
            .map(selected_transaction_to_bridge)
            .collect(),
        signing_hash: step.signing_hash.0,
        signed_block: step.signed_intent.map_or(
            DagProposerSignedBlockIntent {
                block_rlp: Vec::new(),
                block_hash: [0; 32],
            },
            |intent| DagProposerSignedBlockIntent {
                block_rlp: intent.block_rlp,
                block_hash: intent.block_hash.0,
            },
        ),
        record_proposed_block: step.record_proposed_block,
        vdf_poll_interval_ms: rustaxa_consensus::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
        stale_proof_sleep_ms: rustaxa_consensus::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
        error_code: step.error_code,
    }
}

fn pack_candidate_from_request(
    request: TransactionServiceEstimateRequest,
) -> TransactionPackSessionCandidate {
    TransactionPackSessionCandidate {
        found: true,
        hash: request.hash.0,
        declared_gas: request.declared_gas,
        sender: request.sender.0,
        nonce: request.nonce.to_big_endian(),
        gas_price: request.gas_price.to_big_endian(),
        gas: request.gas,
        receiver_found: request.receiver.is_some(),
        receiver: request.receiver.unwrap_or_default().0,
        value: request.value.to_big_endian(),
        data: request.data,
    }
}

fn selected_transaction_to_bridge(
    selected: TransactionPackingSelection,
) -> TransactionPackSelectedTransaction {
    TransactionPackSelectedTransaction {
        hash: selected.hash.0,
        gas_used: selected.gas_used,
        tx_rlp: selected.transaction_rlp,
    }
}

fn empty_pack_candidate() -> TransactionPackSessionCandidate {
    TransactionPackSessionCandidate {
        found: false,
        hash: [0; 32],
        declared_gas: 0,
        sender: [0; 20],
        nonce: [0; 32],
        gas_price: [0; 32],
        gas: 0,
        receiver_found: false,
        receiver: [0; 20],
        value: [0; 32],
        data: Vec::new(),
    }
}

fn service_pack_prepared_to_bridge(
    prepared: TransactionServiceCompatibilityPackPrepared,
) -> TransactionPackPreparedPlan {
    TransactionPackPreparedPlan {
        request_estimates: prepared
            .request_estimates
            .into_iter()
            .map(pack_candidate_from_request)
            .collect(),
        selected_transactions: prepared
            .selected_transactions
            .into_iter()
            .map(selected_transaction_to_bridge)
            .collect(),
        demoted_hashes: prepared
            .demoted_hashes
            .into_iter()
            .map(|hash| TransactionQueueHash { hash: hash.0 })
            .collect(),
        stopped: prepared.stopped,
    }
}

fn service_pack_finalized_to_bridge(
    finalized: TransactionServiceCompatibilityPackFinalized,
) -> TransactionPackSessionStep {
    TransactionPackSessionStep {
        request_estimate: false,
        candidate: empty_pack_candidate(),
        selected_transactions: finalized
            .selected_transactions
            .into_iter()
            .map(selected_transaction_to_bridge)
            .collect(),
        demoted_hashes: finalized
            .demoted_hashes
            .into_iter()
            .map(|hash| TransactionQueueHash { hash: hash.0 })
            .collect(),
        stopped: finalized.stopped,
    }
}

pub fn service_dag_manager_runtime_begin_verify_block_session(
    service: &BridgeDagTransactionService,
    input: DagVerifyBlockSessionInput,
) -> Result<()> {
    service
        .root
        .begin_verify_block_session(NativeDagVerifyBlockSessionInput {
            block_hash: input.block_hash,
            block_level: input.block_level,
            pivot: input.pivot,
            tips: input
                .tips
                .into_iter()
                .map(|tip| H256::from(tip.hash))
                .collect(),
            block_transaction_hashes: input
                .block_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            supplied_transaction_hashes: input
                .supplied_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            block_rlp: input.block_rlp,
        })
}

pub fn service_dag_manager_runtime_verify_block_session_next(
    service: &BridgeDagTransactionService,
) -> Result<DagVerifyBlockSessionStep> {
    service
        .root
        .next_verify_block_session()
        .map(native_verify_block_step_to_bridge)
}

/// Prepares the active DAG verification transaction query without advancing it.
///
/// Locks are acquired DAG then transaction. Query hashes and proposal period
/// remain Rust-private. The returned cursor identity and proposal period bind a
/// later completion to this exact session after C++ has materialized and
/// hash-validated every returned payload.
pub fn service_dag_manager_runtime_verify_block_session_prepare_transactions(
    service: &BridgeDagTransactionService,
) -> Result<DagVerifyBlockTransactionPreparation> {
    let plan = service.root.prepare_verify_block_transactions()?;
    Ok(DagVerifyBlockTransactionPreparation {
        cursor_id: plan.cursor_id,
        proposal_period: plan.proposal_period,
        transactions: plan
            .transactions
            .into_iter()
            .map(native_transaction_view_to_bridge)
            .collect(),
    })
}

/// Completes prepared transaction availability after C++ materialization.
///
/// Cursor and proposal-period identity are checked before TransactionManager
/// lookup. Finalized-storage senders require explicit account facts, and all
/// proposal-period filtering completes before the DAG action advances. Any
/// identity or lookup error leaves the session unchanged.
pub fn service_dag_manager_runtime_verify_block_session_complete_transactions(
    service: &BridgeDagTransactionService,
    report: DagVerifyBlockTransactionCompletionReport,
) -> Result<DagVerifyBlockSessionStep> {
    service
        .root
        .complete_verify_block_transactions(NativeDagVerifyBlockTransactionCompletionReport {
            cursor_id: report.cursor_id,
            proposal_period: report.proposal_period,
            account_nonce_facts: report
                .account_nonce_facts
                .into_iter()
                .map(|fact| TransactionServiceAccountNonceFact {
                    sender: fact.sender,
                    account_found: fact.account_found,
                    account_nonce: fact.account_nonce,
                })
                .collect(),
        })
        .map(native_verify_block_step_to_bridge)
}

fn native_transaction_view_to_bridge(
    view: TransactionServiceTransactionView,
) -> TransactionManagerTransactionView {
    TransactionManagerTransactionView {
        input_index: view.input_index,
        hash: view.hash,
        found: view.found,
        source: view.source,
        old_finalized: view.old_finalized,
        tx_rlp: view.tx_rlp,
    }
}

fn native_verify_block_step_to_bridge(
    step: NativeDagVerifyBlockSessionStep,
) -> DagVerifyBlockSessionStep {
    DagVerifyBlockSessionStep {
        cursor_id: step.cursor_id,
        status: step.status,
        action: step.action,
        complete: step.complete,
        reject_code: step.reject_code,
        proposal_period: step.proposal_period,
        vote_count: step.vote_count,
        max_vote_count: step.max_vote_count,
        error_code: step.error_code,
    }
}
/// Collects FinalChain DPoS/VRF facts for the exact active DAG verification cursor.
///
/// The cursor is snapshotted under the DAG lock, the FinalChain query runs with
/// every service lock released, and the unchanged cursor is revalidated before
/// facts advance it. Infrastructure failures remove only the matching cursor.
pub fn service_dag_manager_runtime_verify_block_session_report_authorization(
    service: &BridgeDagTransactionService,
    final_chain: &BridgeFinalChain,
) -> Result<DagVerifyBlockSessionStep> {
    service_dag_manager_runtime_verify_block_session_report_authorization_with_hook(
        service,
        final_chain,
        || {},
    )
}

fn service_dag_manager_runtime_verify_block_session_report_authorization_with_hook(
    service: &BridgeDagTransactionService,
    final_chain: &BridgeFinalChain,
    between_query_and_apply: impl FnOnce(),
) -> Result<DagVerifyBlockSessionStep> {
    let request = match service.root.prepare_verify_block_authorization()? {
        NativeDagVerifyBlockAuthorizationRequestOrStep::Request(request) => request,
        NativeDagVerifyBlockAuthorizationRequestOrStep::Step(step) => {
            return Ok(native_verify_block_step_to_bridge(step));
        }
    };

    let facts_result = final_chain
        .0
        .dag_dpos_authorization_facts(request.proposal_period.into(), request.sender.0);
    let facts = match facts_result {
        Ok(facts) => facts,
        Err(error) => {
            service.root.abort_verify_block_authorization(&request)?;
            return Err(error);
        }
    };

    between_query_and_apply();

    service
        .root
        .complete_verify_block_authorization(&request, facts)
        .map(native_verify_block_step_to_bridge)
}

pub fn service_dag_manager_runtime_verify_block_session_report_gas(
    service: &BridgeDagTransactionService,
    report: DagVerifyBlockGasReport,
) -> Result<DagVerifyBlockSessionStep> {
    service
        .root
        .report_verify_block_gas(NativeDagVerifyBlockGasReport {
            block_gas_estimation: report.block_gas_estimation,
            estimated_transactions_weight: report.estimated_transactions_weight,
            dag_gas_limit: report.dag_gas_limit,
            pbft_gas_limit: report.pbft_gas_limit,
        })
        .map(native_verify_block_step_to_bridge)
}

/// Executes cursor-bound DAG VDF verification across isolated DAG and
/// sortition lock intervals.
pub fn service_dag_transaction_service_verify_block_session_vdf(
    service: &BridgeDagTransactionService,
    request: DagVerifyBlockVdfRequest,
) -> Result<DagVerifyBlockSessionStep> {
    service
        .root
        .verify_block_vdf(NativeDagVerifyBlockVdfRequest {
            cursor_id: request.cursor_id,
            block_rlp: request.block_rlp,
            block_level: request.block_level,
            proposal_period_hash: H256::from(request.proposal_period_hash),
        })
        .map(native_verify_block_step_to_bridge)
}
/// Opens a proposer session with transaction pressure derived from the sibling
/// Rust TransactionManager while holding the universal DAG-then-transaction lock order.
///
/// CXX supplies wallet and configuration inputs only. Queue and non-finalized
/// sidecar counts are captured with session creation and retained by the DAG
/// cursor for threshold and skip decisions.
pub fn service_dag_manager_runtime_begin_proposer_session(
    service: &BridgeDagTransactionService,
    input: DagProposerSessionBeginInput,
) -> Result<u64> {
    service
        .root
        .begin_proposer_session(NativeDagProposerSessionBeginInput {
            max_non_finalized_transactions: input.max_non_finalized_transactions,
            dag_expiry_level_limit: input.dag_expiry_level_limit,
            wallet_vrf_public_key: input.wallet_vrf_public_key,
            wallet_vrf_secret: input.wallet_vrf_secret,
            proposer_address: input.proposer_address,
            max_non_finalized_dag_blocks: input.max_non_finalized_dag_blocks,
            max_non_finalized_dag_blocks_low_difficulty: input
                .max_non_finalized_dag_blocks_low_difficulty,
            max_retry_count: input.max_retry_count,
            proposal_weight_limit: input.proposal_weight_limit,
            total_transaction_shards: input.total_transaction_shards,
            node_transaction_shard: input.node_transaction_shard,
            shard_period_interval: input.shard_period_interval,
            pbft_gas_limit: input.pbft_gas_limit,
            dag_gas_limit: input.dag_gas_limit,
            max_tips: input.max_tips,
        })
}

pub fn service_dag_manager_runtime_abort_proposer_session(
    service: &BridgeDagTransactionService,
    session_id: u64,
) -> Result<bool> {
    service.root.abort_proposer_session(session_id)
}

pub fn service_dag_manager_runtime_proposer_session_next(
    service: &BridgeDagTransactionService,
    session_id: u64,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .next_proposer_session(session_id)
        .map(proposer_session_step_to_bridge)
}
/// Composes FinalChain facts with exact historical sortition parameters.
///
/// The first DAG lock validates and snapshots the keyed proposer cursor. The
/// first indexed sortition lookup runs under the sortition lock alone. Rust
/// then reacquires DAG followed by sortition, revalidates the exact cursor,
/// repeats the indexed lookup, compares every parameter field, and privately
/// plans/advances while both values remain protected. Parameter drift returns
/// `DAG_PROPOSER_SESSION_SORTITION_PARAMS_STALE_RETRY` without advancement;
/// operational lookup failures remove only the exact snapshotted session.
pub fn service_dag_manager_runtime_proposer_session_report_final_chain_facts(
    service: &BridgeDagTransactionService,
    session_id: u64,
    final_chain: &BridgeFinalChain,
) -> Result<DagProposerSessionStep> {
    service_dag_manager_runtime_proposer_session_report_final_chain_facts_with_hook(
        service,
        session_id,
        final_chain,
        || {},
    )
}

fn service_dag_manager_runtime_proposer_session_report_final_chain_facts_with_hook(
    service: &BridgeDagTransactionService,
    session_id: u64,
    final_chain: &BridgeFinalChain,
    between_lookups: impl FnOnce(),
) -> Result<DagProposerSessionStep> {
    let request = match service
        .root
        .prepare_proposer_final_chain_facts(session_id)?
    {
        NativeDagProposerFinalChainRequestOrStep::Request(request) => request,
        NativeDagProposerFinalChainRequestOrStep::Step(step) => {
            return Ok(proposer_session_step_to_bridge(*step));
        }
    };
    let (last_finalized_period, authorization_facts) =
        match proposer_final_chain_facts_from_final_chain(
            final_chain,
            request.proposal_period_found,
            request.proposal_period,
            request.proposer_address,
        ) {
            Ok(facts) => facts,
            Err(error) => {
                service.root.abort_proposer_final_chain_facts(&request)?;
                return Err(error);
            }
        };
    between_lookups();
    service
        .root
        .complete_proposer_final_chain_facts(
            &request,
            NativeDagProposerFinalChainFacts {
                last_finalized_period,
                authorization_facts,
            },
        )
        .map(proposer_session_step_to_bridge)
}

fn proposer_final_chain_facts_from_final_chain(
    final_chain: &BridgeFinalChain,
    proposal_period_found: bool,
    proposal_period: u64,
    proposer_address: [u8; 20],
) -> Result<(u64, rustaxa_consensus::dag::DagDposAuthorizationFacts)> {
    let last_finalized_period = final_chain.0.last_block_number()?;
    let authorization_facts = if proposal_period_found {
        final_chain
            .0
            .dag_dpos_authorization_facts(proposal_period.into(), proposer_address)?
    } else {
        rustaxa_consensus::dag::DagDposAuthorizationFacts {
            vrf_key: None,
            vrf_key_found: false,
            sender_eligible_vote_count: 0,
            vdf_sortition_max_vote_count: 0,
            eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
        }
    };
    Ok((last_finalized_period, authorization_facts))
}

pub fn service_dag_manager_runtime_proposer_session_poll_vdf(
    service: &BridgeDagTransactionService,
    session_id: u64,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .poll_proposer_vdf(session_id)
        .map(proposer_session_step_to_bridge)
}

pub fn service_dag_manager_runtime_proposer_session_report_vdf_proof(
    service: &BridgeDagTransactionService,
    session_id: u64,
    report: DagProposerVdfProofReport,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .report_proposer_vdf_proof(
            session_id,
            NativeDagProposerVdfProofReport {
                proof_ok: report.proof_ok,
                vdf_rlp: report.vdf_rlp,
            },
        )
        .map(proposer_session_step_to_bridge)
}

pub fn service_dag_manager_runtime_proposer_session_resume_stale_proof(
    service: &BridgeDagTransactionService,
    session_id: u64,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .resume_proposer_stale_proof(session_id)
        .map(proposer_session_step_to_bridge)
}

pub fn service_dag_manager_runtime_proposer_session_report_signing(
    service: &BridgeDagTransactionService,
    session_id: u64,
    report: DagProposerSigningReport,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .report_proposer_signing(
            session_id,
            NativeDagProposerSigningReport {
                signature: report.signature,
            },
        )
        .map(proposer_session_step_to_bridge)
}

pub fn service_dag_manager_runtime_proposer_session_report_add_block(
    service: &BridgeDagTransactionService,
    session_id: u64,
    report: DagProposerAddBlockReport,
) -> Result<DagProposerSessionStep> {
    service
        .root
        .report_proposer_add_block(
            session_id,
            NativeDagProposerAddBlockReport {
                accepted: report.accepted,
                duplicate: report.duplicate,
                expired: report.expired,
                missing_references: report
                    .missing_references
                    .into_iter()
                    .map(|hash| H256::from(hash.hash))
                    .collect(),
            },
        )
        .map(proposer_session_step_to_bridge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi;
    use crate::final_chain::create_final_chain_with_rewards_config;
    use crate::storage::create_storage;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_consensus::sortition::SortitionParamsChange;
    use rustaxa_types::LegacyTransactionEnvelope;
    use rustaxa_vdf::prover::CancellationToken;
    use rustaxa_vdf::sortition::{self, LegacySortitionParams};
    use rustaxa_vdf::vrf::public_key_from_secret;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use tiny_keccak::{Hasher, Keccak};

    const DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY: u8 = 1;
    const DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS: u8 = 2;
    const DAG_VERIFY_SESSION_ACTION_VDF_SORTITION: u8 = 3;
    const DAG_VERIFY_SESSION_ACTION_GAS: u8 = 4;

    const SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn u256_be(value: u64) -> Vec<u8> {
        U256::from(value).to_big_endian().to_vec()
    }

    fn queue_config() -> TransactionQueueConfig {
        TransactionQueueConfig { max_size: 100 }
    }
    fn sortition_config() -> SortitionRuntimeConfig {
        SortitionRuntimeConfig {
            threshold_upper: 1_000,
            difficulty_min: 1,
            difficulty_max: 10,
            difficulty_stale: 3,
            lambda_bound: 100,
            changes_count_for_average: 4,
            dag_efficiency_target_low: 4_800,
            dag_efficiency_target_high: 5_200,
            changing_interval: 1,
            computation_interval: 1,
        }
    }
    fn gas_config() -> GasPricerConfig {
        GasPricerConfig {
            percentile: 50,
            minimum_price: [0; 32],
            history_blocks: 10,
            is_light_node: false,
            blocks_gas_pricer: false,
        }
    }

    fn keccak256(bytes: &[u8]) -> H256 {
        let mut out = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(bytes);
        hasher.finalize(&mut out);
        H256::from(out)
    }

    fn address_from_signing_key(signing_key: &SigningKey) -> [u8; 20] {
        let point = signing_key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&point.as_bytes()[1..]);
        hash.as_bytes()[12..].try_into().unwrap()
    }

    fn signed_legacy_transaction_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let chain_id = 2999u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&U256::from(1));
        unsigned.append(&U256::from(2));
        unsigned.append(&21_000u64);
        unsigned.append(&H160::from([0x44; 20]));
        unsigned.append(&U256::from(3));
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .unwrap();
        let signature = signature.to_bytes();
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(1));
        stream.append(&U256::from(2));
        stream.append(&21_000u64);
        stream.append(&H160::from([0x44; 20]));
        stream.append(&U256::from(3));
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(
            chain_id * 2 + 35 + u64::from(recovery_id.to_byte()),
        ));
        stream.append(&U256::from_big_endian(&signature[..32]));
        stream.append(&U256::from_big_endian(&signature[32..]));
        stream.out().to_vec()
    }

    fn append_pbft_block_fields(stream: &mut RlpStream, period: u64) {
        stream.append(&H256::from_low_u64_be(10));
        stream.append(&H256::from_low_u64_be(11));
        stream.append(&H256::from_low_u64_be(12));
        stream.append(&H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&1_000u64);
        stream.begin_list(0);
    }

    fn signed_pbft_block(signing_key: &SigningKey, period: u64) -> Vec<u8> {
        let mut unsigned = RlpStream::new_list(7);
        append_pbft_block_fields(&mut unsigned, period);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());
        let mut signed = RlpStream::new_list(8);
        append_pbft_block_fields(&mut signed, period);
        signed.append(&signature_bytes);
        signed.out().to_vec()
    }

    fn period_data_rlp(pbft_block: &[u8], transaction_rlp: &[u8]) -> Vec<u8> {
        let mut transactions = RlpStream::new_list(1);
        transactions.append_raw(transaction_rlp, 1);
        let mut finalized_dag_bundle = RlpStream::new_list(3);
        finalized_dag_bundle.begin_list(0);
        finalized_dag_bundle.begin_list(1);
        finalized_dag_bundle.begin_list(1);
        finalized_dag_bundle.append(&0usize);
        finalized_dag_bundle.begin_list(0);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(pbft_block, 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&finalized_dag_bundle.out(), 1);
        period_data.append_raw(&transactions.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.out().to_vec()
    }

    fn composed_add_block_rlp(pivot: [u8; 32], level: u64, transactions: &[[u8; 32]]) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11u8; 80]);
        vdf.append(&vec![0x22u8]);
        vdf.append(&vec![0x33u8]);
        vdf.append(&1u16);
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from(pivot));
        block.append(&level);
        block.append(&0u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(transactions.len());
        for hash in transactions {
            block.append(&H256::from(*hash));
        }
        block.append(&&[0u8; 65][..]);
        block.append(&0u64);
        block.out().to_vec()
    }

    fn add_block_prepare_input(
        block_rlp: Vec<u8>,
        save: bool,
        transactions: Vec<DagAddBlockTransactionPayload>,
    ) -> DagAddBlockPrepareInput {
        DagAddBlockPrepareInput {
            expected_block_hash: keccak256(&block_rlp).0,
            validate_block_hash: true,
            block_rlp,
            save,
            proposed: true,
            transactions,
        }
    }

    fn sign_hash(hash: [u8; 32]) -> Vec<u8> {
        let key = SigningKey::from_slice(&[0x44; 32]).unwrap();
        let (signature, recovery_id) = key.sign_prehash_recoverable(&hash).unwrap();
        let mut bytes = signature.to_bytes().to_vec();
        bytes.push(recovery_id.to_byte());
        bytes
    }

    fn proposer_begin_input() -> DagProposerSessionBeginInput {
        let vrf_key = public_key_from_secret(&SECRET_KEY).expect("VRF key");
        DagProposerSessionBeginInput {
            max_non_finalized_transactions: 100,
            dag_expiry_level_limit: 100,
            wallet_vrf_public_key: vrf_key,
            wallet_vrf_secret: SECRET_KEY,
            proposer_address: address_from_signing_key(
                &SigningKey::from_slice(&[0x44; 32]).unwrap(),
            ),
            max_non_finalized_dag_blocks: 100,
            max_non_finalized_dag_blocks_low_difficulty: 50,
            max_retry_count: 20,
            proposal_weight_limit: 100_000,
            total_transaction_shards: 1,
            node_transaction_shard: 0,
            shard_period_interval: 10,
            pbft_gas_limit: 1_000_000,
            dag_gas_limit: 100_000,
            max_tips: 16,
        }
    }

    fn open_proposer_pack(service: &BridgeDagTransactionService) -> u64 {
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(service, proposer_begin_input())
                .expect("proposer session");
        let request = match service
            .root
            .prepare_proposer_final_chain_facts(session_id)
            .expect("proposer FinalChain preparation")
        {
            NativeDagProposerFinalChainRequestOrStep::Request(request) => request,
            NativeDagProposerFinalChainRequestOrStep::Step(_) => {
                panic!("proposer cursor should request FinalChain facts")
            }
        };
        let step = service
            .root
            .complete_proposer_final_chain_facts(
                &request,
                NativeDagProposerFinalChainFacts {
                    last_finalized_period: 0,
                    authorization_facts: rustaxa_consensus::dag::DagDposAuthorizationFacts {
                        vrf_key: Some(public_key_from_secret(&SECRET_KEY).unwrap()),
                        vrf_key_found: true,
                        sender_eligible_vote_count: 1,
                        vdf_sortition_max_vote_count: 1,
                        eligibility_status: rustaxa_consensus::dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
                    },
                },
            )
            .expect("proposer FinalChain completion");
        assert_eq!(
            step.action,
            NativeDagProposerSessionAction::PackTransactions
        );
        session_id
    }

    fn report_proposer_final_chain_facts(
        service: &BridgeDagTransactionService,
        final_chain: &BridgeFinalChain,
        session_id: u64,
    ) -> DagProposerSessionStep {
        service_dag_manager_runtime_proposer_session_report_final_chain_facts(
            service,
            session_id,
            final_chain,
        )
        .expect("final-chain facts")
    }

    fn proposer_address() -> [u8; 20] {
        address_from_signing_key(&SigningKey::from_slice(&[0x44; 32]).unwrap())
    }

    fn make_proposer_final_chain(
        storage: &BridgeStorage,
        proposer_address: [u8; 20],
    ) -> Box<BridgeFinalChain> {
        let proposer_vrf_key = public_key_from_secret(&SECRET_KEY).expect("proposer vrf key");
        create_final_chain_with_rewards_config(
            storage,
            0,
            0,
            vec![],
            vec![rustaxa_ffi::GenesisValidator {
                address: proposer_address,
                owner: [0u8; 20],
                vrf_key: proposer_vrf_key,
                commission: 0,
                description: "".to_string(),
                endpoint: "".to_string(),
                total_stake: u256_be(10_000),
                delegations: vec![rustaxa_ffi::GenesisDelegation {
                    delegator: proposer_address,
                    stake: u256_be(10_000),
                }],
            }],
            rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: u256_be(1_000),
                vote_eligibility_balance_step: u256_be(1_000),
                validator_maximum_stake: u256_be(30_000),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
            rustaxa_ffi::FinalChainRewardsConfig {
                committee_size: 0,
                magnolia_period: 0,
                phalaenopsis_period: u64::MAX,
                aspen_part_one_period: u64::MAX,
                fix_claim_all_block_num: u64::MAX,
                fix_redelegate_block_num: u64::MAX,
                aspen_part_two_period: 0,
                max_block_author_reward_percent: 0,
                dag_proposers_reward_percent: 0,
                yield_percentage: 0,
                dpos_blocks_per_year: 0,
                dpos_delegation_locking_period: 0,
                cornus_period: 0,
                cornus_delegation_locking_period: 0,
                genesis_balance_sum: Vec::new(),
                aspen_max_supply: Vec::new(),
                aspen_generated_rewards: Vec::new(),
                cacti_period: 0,
                cacti_delegation_locking_period: 0,
                magnolia_jail_time: 0,
                cacti_jail_time: 0,
                frequency_rules: Vec::new(),
                redelegations: Vec::new(),
            },
        )
        .expect("proposer final chain")
    }

    fn assert_zero_legacy_sortition_params(
        params: &crate::ffi::rustaxa_ffi::LegacySortitionParams,
    ) {
        assert_eq!(params.vrf_threshold_upper, 0);
        assert_eq!(params.vdf_difficulty_min, 0);
        assert_eq!(params.vdf_difficulty_max, 0);
        assert_eq!(params.vdf_difficulty_stale, 0);
        assert_eq!(params.vdf_lambda_bound, 0);
    }

    fn insert_test_queue_transaction(
        service: &BridgeDagTransactionService,
        secret: u8,
    ) -> LegacyTransactionEnvelope {
        let key = SigningKey::from_slice(&[secret; 32]).unwrap();
        let tx_rlp = signed_legacy_transaction_rlp(&key);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        service
            .insert_test_queue_transaction(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: address_from_signing_key(&key),
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .unwrap();
        envelope
    }

    fn begin_verify_block_session(
        service: &BridgeDagTransactionService,
        block_hashes: &[[u8; 32]],
        supplied_hashes: &[[u8; 32]],
    ) {
        service_dag_manager_runtime_begin_verify_block_session(
            service,
            DagVerifyBlockSessionInput {
                block_hash: [0u8; 32],
                block_level: 1,
                pivot: [1; 32],
                tips: Vec::new(),
                block_transaction_hashes: block_hashes
                    .iter()
                    .map(|hash| DagTransactionHash { hash: *hash })
                    .collect(),
                supplied_transaction_hashes: supplied_hashes
                    .iter()
                    .map(|hash| DagTransactionHash { hash: *hash })
                    .collect(),
                block_rlp: Vec::new(),
            },
        )
        .expect("verify-block session should begin");
    }

    fn vdf_test_block(vdf_payload: Vec<u8>, signature_byte: u8) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::from([1u8; 32]));
        block.append(&1u64);
        block.append(&0u64);
        block.append(&vdf_payload);
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[signature_byte; 65][..]);
        block.append(&0u64);
        block.out().to_vec()
    }

    fn sign_vdf_test_block(block_rlp: &[u8]) -> Vec<u8> {
        sign_vdf_test_block_with_seed(block_rlp, 0x44)
    }

    fn sign_vdf_test_block_with_seed(block_rlp: &[u8], seed: u8) -> Vec<u8> {
        let mut block = rustaxa_types::dag::DagBlock::try_from(
            rustaxa_types::codec::rlp::dag::DagBlockRlp::new(block_rlp),
        )
        .expect("test DAG block should decode");
        let key = SigningKey::from_slice(&[seed; 32]).expect("test DAG signing key");
        let (signature, recovery_id) = key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .expect("test DAG block should sign");
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();

        let mut encoded = RlpStream::new_list(8);
        encoded.append(&block.pivot);
        encoded.append(&block.level);
        encoded.append(&block.timestamp);
        encoded.append(&block.vdf);
        encoded.append_list(&block.tips);
        encoded.append_list(&block.transactions);
        encoded.append(&block.signature.as_ref());
        encoded.append(&block.gas_estimation);
        encoded.out().to_vec()
    }

    fn valid_vdf_request(
        threshold_upper: u16,
        proposal_period_hash: [u8; 32],
    ) -> DagVerifyBlockVdfRequest {
        valid_vdf_request_with_votes(threshold_upper, proposal_period_hash, 1, 1)
    }

    fn valid_vdf_request_with_votes(
        threshold_upper: u16,
        proposal_period_hash: [u8; 32],
        vote_count: u64,
        max_vote_count: u64,
    ) -> DagVerifyBlockVdfRequest {
        let placeholder = vdf_test_block(Vec::new(), 0);
        let vrf_input =
            rustaxa_consensus::dag::construct_dag_vrf_input(1, H256::from(proposal_period_hash));
        let vdf_input =
            rustaxa_consensus::dag::construct_dag_vdf_message_from_block_rlp(&placeholder)
                .expect("VDF message should build");
        let proof = sortition::prove_legacy_vdf_sortition(
            LegacySortitionParams {
                vrf_threshold_upper: threshold_upper,
                vdf_difficulty_min: 1,
                vdf_difficulty_max: 10,
                vdf_difficulty_stale: 3,
                vdf_lambda_bound: 100,
            },
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            vote_count,
            max_vote_count,
            &CancellationToken::new(),
        )
        .expect("VDF proof should generate");
        let mut payload = RlpStream::new_list(4);
        payload.append(&&proof.vrf_proof[..]);
        payload.append(&proof.vdf_proof);
        payload.append(&proof.vdf_output);
        payload.append(&proof.difficulty);
        DagVerifyBlockVdfRequest {
            cursor_id: 0,
            block_rlp: sign_vdf_test_block(&vdf_test_block(payload.out().to_vec(), 0)),
            block_level: 1,
            proposal_period_hash,
        }
    }

    fn begin_vdf_action(
        service: &BridgeDagTransactionService,
        storage: &BridgeStorage,
        block_rlp: &[u8],
    ) -> DagVerifyBlockSessionStep {
        service_dag_manager_runtime_begin_verify_block_session(
            service,
            DagVerifyBlockSessionInput {
                block_hash: keccak256(block_rlp).0,
                block_level: 1,
                pivot: [1; 32],
                tips: Vec::new(),
                block_transaction_hashes: Vec::new(),
                supplied_transaction_hashes: Vec::new(),
                block_rlp: block_rlp.to_vec(),
            },
        )
        .expect("verify session should begin");
        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(service)
                .expect("empty transaction query should prepare");
        let authorization = service_dag_manager_runtime_verify_block_session_complete_transactions(
            service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .expect("empty transaction query should complete");
        assert_eq!(
            authorization.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );
        let final_chain = make_proposer_final_chain(storage, proposer_address());
        service_dag_manager_runtime_verify_block_session_report_authorization(service, &final_chain)
            .expect("authorization should advance to VDF")
    }

    fn begin_verify_authorization_action(
        service: &BridgeDagTransactionService,
        block_rlp: &[u8],
    ) -> DagVerifyBlockSessionStep {
        service_dag_manager_runtime_begin_verify_block_session(
            service,
            DagVerifyBlockSessionInput {
                block_hash: keccak256(block_rlp).0,
                block_level: 1,
                pivot: [1; 32],
                tips: Vec::new(),
                block_transaction_hashes: Vec::new(),
                supplied_transaction_hashes: Vec::new(),
                block_rlp: block_rlp.to_vec(),
            },
        )
        .expect("verify session should begin");
        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(service)
                .expect("empty transaction query should prepare");
        service_dag_manager_runtime_verify_block_session_complete_transactions(
            service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .expect("empty transaction query should complete")
    }

    #[test]
    fn verify_authorization_composes_final_chain_and_keeps_vrf_key_private() {
        let dir = unique_temp_dir("rustaxa_dag_verify_composed_authorization");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        let mut request = valid_vdf_request_with_votes(1_000, [0xA7; 32], 10, 30);
        request.block_rlp = sign_vdf_test_block(&request.block_rlp);
        let authorization = begin_verify_authorization_action(&service, &request.block_rlp);
        assert_eq!(
            authorization.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );

        let vdf = service_dag_manager_runtime_verify_block_session_report_authorization(
            &service,
            &final_chain,
        )
        .expect("Rust FinalChain authorization should advance the cursor");
        assert_eq!(vdf.action, DAG_VERIFY_SESSION_ACTION_VDF_SORTITION);
        assert_eq!(vdf.vote_count, 10);
        assert_eq!(vdf.max_vote_count, 30);

        let gas = service_dag_transaction_service_verify_block_session_vdf(
            &service,
            DagVerifyBlockVdfRequest {
                cursor_id: vdf.cursor_id,
                ..request
            },
        )
        .expect("VDF verification should use the cursor-private VRF key");
        assert_eq!(gas.action, DAG_VERIFY_SESSION_ACTION_GAS);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_authorization_rejects_wrong_action_and_stale_replacement() {
        let dir = unique_temp_dir("rustaxa_dag_verify_authorization_stale");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        let block_rlp = sign_vdf_test_block(&vdf_test_block(Vec::new(), 0));
        begin_verify_block_session(&service, &[], &[]);
        let wrong = service_dag_manager_runtime_verify_block_session_report_authorization(
            &service,
            &final_chain,
        )
        .expect("wrong action should use the stable invalid-step carrier");
        assert_eq!(wrong.status, 2);
        assert!(wrong
            .error_code
            .contains("DAG_VERIFY_SESSION_UNEXPECTED_AUTHORIZATION_REPORT"));

        begin_verify_authorization_action(&service, &block_rlp);
        let stale =
            service_dag_manager_runtime_verify_block_session_report_authorization_with_hook(
                &service,
                &final_chain,
                || {
                    begin_verify_block_session(&service, &[], &[]);
                },
            )
            .err()
            .expect("replacement cursor must reject stale FinalChain facts");
        assert!(stale
            .to_string()
            .contains("DAG_VERIFY_SESSION_AUTHORIZATION_CURSOR_MISMATCH"));
        let replacement = service_dag_manager_runtime_verify_block_session_next(&service).unwrap();
        assert_eq!(
            replacement.action,
            DAG_VERIFY_SESSION_ACTION_TRANSACTION_QUERY
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_authorization_preserves_snapshot_and_missing_key_rejections() {
        let dir = unique_temp_dir("rustaxa_dag_verify_authorization_rejections");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        let unknown_sender_block =
            sign_vdf_test_block_with_seed(&vdf_test_block(Vec::new(), 0), 0x45);
        begin_verify_authorization_action(&service, &unknown_sender_block);
        let missing_key = service_dag_manager_runtime_verify_block_session_report_authorization(
            &service,
            &final_chain,
        )
        .expect("missing validator VRF key is a consensus rejection");
        assert!(missing_key.complete);
        assert_eq!(
            missing_key.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );

        storage
            .0
            .dag()
            .write_proposal_period_at_level(1, 1)
            .unwrap();
        let proposer_block = sign_vdf_test_block(&vdf_test_block(Vec::new(), 0));
        begin_verify_authorization_action(&service, &proposer_block);
        let unavailable = service_dag_manager_runtime_verify_block_session_report_authorization(
            &service,
            &final_chain,
        )
        .expect("missing historical snapshot is a consensus rejection");
        assert!(unavailable.complete);
        assert_eq!(
            unavailable.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FUTURE_BLOCK
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_authorization_decode_failure_removes_the_owned_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_verify_authorization_decode_failure");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        begin_verify_authorization_action(&service, &[0x80]);
        let decode_error = service_dag_manager_runtime_verify_block_session_report_authorization(
            &service,
            &final_chain,
        )
        .err()
        .expect("malformed retained block bytes are an infrastructure error");
        assert!(decode_error
            .to_string()
            .contains("DAG_VERIFY_SESSION_AUTHORIZATION_BLOCK_DECODE"));
        let removed = service_dag_manager_runtime_verify_block_session_next(&service).unwrap();
        assert_eq!(removed.status, 2);
        assert!(removed
            .error_code
            .contains("DAG_VERIFY_SESSION_NOT_STARTED"));

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_final_chain_facts_use_period_params_and_expose_them_only_for_start_vdf() {
        let dir = unique_temp_dir("rustaxa_dag_proposer_period_sortition");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        insert_test_queue_transaction(&service, 0x71);
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let collect =
            service_dag_manager_runtime_proposer_session_next(&service, session_id).unwrap();
        storage
            .0
            .metadata()
            .write_sortition_params_change(
                collect.proposal_period,
                &SortitionParamsChange {
                    period: collect.proposal_period,
                    interval_efficiency: 5_000,
                    threshold_upper: u16::MAX,
                }
                .to_rlp_bytes(),
            )
            .unwrap();

        let pack = service_dag_manager_runtime_proposer_session_report_final_chain_facts(
            &service,
            session_id,
            &final_chain,
        )
        .unwrap();
        assert_eq!(pack.action, 1);
        assert_zero_legacy_sortition_params(&pack.vdf_sortition_params);
        let prepare = service
            .dag_transaction_service_proposer_pack_prepare(session_id, false, 21_000, 0, 10)
            .expect("proposer pack should prepare an EVM estimate");
        let start_vdf = service
            .dag_transaction_service_proposer_pack_finalize(
                session_id,
                prepare
                    .transaction_estimate_requests
                    .iter()
                    .map(|estimate| TransactionPackSessionEstimateInput {
                        hash: estimate.hash,
                        gas_used: 21_000,
                        last_block_number: 10,
                        result_rlp: vec![0xC0],
                    })
                    .collect(),
            )
            .expect("proposer pack should finalize");
        assert_eq!(start_vdf.action, 2);
        assert_eq!(start_vdf.vdf_sortition_params.vrf_threshold_upper, u16::MAX);
        assert_eq!(start_vdf.vdf_sortition_params.vdf_difficulty_min, 1);
        assert_eq!(start_vdf.vdf_sortition_params.vdf_difficulty_max, 10);
        assert_eq!(start_vdf.vdf_sortition_params.vdf_difficulty_stale, 3);
        assert_eq!(start_vdf.vdf_sortition_params.vdf_lambda_bound, 100);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_final_chain_facts_no_vdf_branch_keeps_params_private() {
        let dir = unique_temp_dir("rustaxa_dag_proposer_no_vdf_params");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let terminal = service_dag_manager_runtime_proposer_session_report_final_chain_facts(
            &service,
            session_id,
            &final_chain,
        )
        .unwrap();
        assert_ne!(terminal.action, 2);
        assert_zero_legacy_sortition_params(&terminal.vdf_sortition_params);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_final_chain_facts_reject_full_stale_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_proposer_stale_facts");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        insert_test_queue_transaction(&service, 0x73);
        let stale_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let stale =
            service_dag_manager_runtime_proposer_session_report_final_chain_facts_with_hook(
                &service,
                stale_id,
                &final_chain,
                || {
                    assert!(service.root.abort_proposer_session(stale_id).unwrap());
                },
            )
            .err()
            .expect("removed cursor must fail revalidation");
        assert!(stale
            .to_string()
            .contains("DAG_PROPOSER_SESSION_STALE_CURSOR"));

        let wrong_stage_id = open_proposer_pack(&service);
        let wrong_stage = service_dag_manager_runtime_proposer_session_report_final_chain_facts(
            &service,
            wrong_stage_id,
            &final_chain,
        )
        .expect("wrong-stage reports use the stable invalid-step carrier");
        assert_eq!(wrong_stage.status, 2);
        assert!(wrong_stage
            .error_code
            .contains("DAG_PROPOSER_SESSION_UNEXPECTED_FINAL_CHAIN_FACTS_REPORT"));

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_final_chain_facts_detect_param_drift_without_advancing() {
        let dir = unique_temp_dir("rustaxa_dag_proposer_sortition_drift");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        insert_test_queue_transaction(&service, 0x74);
        let session_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let period = service_dag_manager_runtime_proposer_session_next(&service, session_id)
            .unwrap()
            .proposal_period;
        let stale =
            service_dag_manager_runtime_proposer_session_report_final_chain_facts_with_hook(
                &service,
                session_id,
                &final_chain,
                || {
                    storage
                        .0
                        .metadata()
                        .write_sortition_params_change(
                            period,
                            &SortitionParamsChange {
                                period,
                                interval_efficiency: 5_000,
                                threshold_upper: 1_234,
                            }
                            .to_rlp_bytes(),
                        )
                        .unwrap();
                },
            )
            .err()
            .expect("changed exact params must request retry");
        assert!(stale
            .to_string()
            .contains("DAG_PROPOSER_SESSION_SORTITION_PARAMS_STALE_RETRY"));
        assert_eq!(
            service_dag_manager_runtime_proposer_session_next(&service, session_id)
                .unwrap()
                .action,
            7
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_sortition_lookup_failure_cleans_only_the_owned_session() {
        let dir = unique_temp_dir("rustaxa_dag_proposer_sortition_failure_cleanup");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        let corrupt =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let period = service_dag_manager_runtime_proposer_session_next(&service, corrupt)
            .unwrap()
            .proposal_period;
        storage
            .0
            .metadata()
            .write_sortition_params_change(period, &[0x80])
            .unwrap();
        assert!(
            service_dag_manager_runtime_proposer_session_report_final_chain_facts(
                &service,
                corrupt,
                &final_chain,
            )
            .is_err()
        );
        assert!(!service_dag_manager_runtime_abort_proposer_session(&service, corrupt).unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_final_chain_read_failure_cleans_only_the_matching_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_proposer_final_chain_failure_cleanup");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());
        let first =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let second =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();

        storage
            .0
            .final_chain()
            .write_conformance_lookup_rows(
                1,
                &[1],
                99,
                H256::zero(),
                &[],
                H256::zero(),
                &[],
                H256::zero(),
                &[],
                99,
                &[],
            )
            .unwrap();
        let read_error = service_dag_manager_runtime_proposer_session_report_final_chain_facts(
            &service,
            first,
            &final_chain,
        )
        .err()
        .expect("invalid LAST_NUMBER must fail after sortition lookup");
        assert!(read_error
            .to_string()
            .contains("final_chain_meta/LAST_NUMBER"));

        let first_after =
            service_dag_manager_runtime_proposer_session_next(&service, first).unwrap();
        assert_eq!(first_after.status, 2);
        assert!(first_after
            .error_code
            .contains("DAG_PROPOSER_SESSION_NOT_STARTED"));
        let second_after =
            service_dag_manager_runtime_proposer_session_next(&service, second).unwrap();
        assert_eq!(second_after.status, 0);
        assert_eq!(second_after.action, 7);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cursor_bound_vdf_uses_historical_params_and_advances_valid_proof_once() {
        let dir = unique_temp_dir("rustaxa_dag_vdf_historical_params");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let proposal_period_hash = [0x91; 32];
        let mut request = valid_vdf_request(1_234, proposal_period_hash);
        let step = begin_vdf_action(&service, &storage, &request.block_rlp);
        request.cursor_id = step.cursor_id;
        storage
            .0
            .metadata()
            .write_sortition_params_change(
                step.proposal_period,
                &SortitionParamsChange {
                    period: step.proposal_period,
                    interval_efficiency: 5_000,
                    threshold_upper: 1_234,
                }
                .to_rlp_bytes(),
            )
            .unwrap();
        assert_eq!(
            service
                .sortition()
                .unwrap()
                .current_params()
                .vrf
                .threshold_upper,
            1_000
        );

        let gas = service_dag_transaction_service_verify_block_session_vdf(&service, request)
            .expect("historical parameters should verify the proof");
        assert_eq!(gas.action, DAG_VERIFY_SESSION_ACTION_GAS);
        assert_eq!(gas.cursor_id, step.cursor_id);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cursor_bound_vdf_classifies_malformed_proof_as_invalid() {
        let dir = unique_temp_dir("rustaxa_dag_vdf_invalid_proof");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let block_rlp = vdf_test_block(vec![0x80], 0);
        let signed_block_rlp = sign_vdf_test_block(&block_rlp);
        let step = begin_vdf_action(&service, &storage, &signed_block_rlp);
        let invalid = service_dag_transaction_service_verify_block_session_vdf(
            &service,
            DagVerifyBlockVdfRequest {
                cursor_id: step.cursor_id,
                block_rlp: signed_block_rlp,
                block_level: 1,
                proposal_period_hash: [0x92; 32],
            },
        )
        .expect("malformed proof should be a deterministic consensus rejection");
        assert!(invalid.complete);
        assert_eq!(
            invalid.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cursor_bound_vdf_rejects_wrong_action_without_advancing() {
        let dir = unique_temp_dir("rustaxa_dag_vdf_capability_and_action");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let request = valid_vdf_request(1_000, [0x93; 32]);
        service_dag_manager_runtime_begin_verify_block_session(
            &service,
            DagVerifyBlockSessionInput {
                block_hash: keccak256(&request.block_rlp).0,
                block_level: 1,
                pivot: [1; 32],
                tips: Vec::new(),
                block_transaction_hashes: Vec::new(),
                supplied_transaction_hashes: Vec::new(),
                block_rlp: request.block_rlp.clone(),
            },
        )
        .unwrap();
        let action = service_dag_manager_runtime_verify_block_session_next(&service).unwrap();
        let wrong_action = service_dag_transaction_service_verify_block_session_vdf(
            &service,
            DagVerifyBlockVdfRequest {
                cursor_id: action.cursor_id,
                ..request
            },
        )
        .err()
        .expect("wrong action should fail");
        assert!(wrong_action
            .to_string()
            .contains("DAG_VERIFY_SESSION_UNEXPECTED_VDF_ACTION"));
        assert_eq!(
            service_dag_manager_runtime_verify_block_session_next(&service)
                .unwrap()
                .action,
            1
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cursor_bound_vdf_rejects_full_block_mismatch() {
        let dir = unique_temp_dir("rustaxa_dag_vdf_block_mismatch");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let mut request = valid_vdf_request(1_000, [0x95; 32]);
        let vdf = begin_vdf_action(&service, &storage, &request.block_rlp);
        request.cursor_id = vdf.cursor_id;
        let mut mismatched = request;
        mismatched.block_rlp = vdf_test_block(Vec::new(), 1);
        let mismatch =
            service_dag_transaction_service_verify_block_session_vdf(&service, mismatched)
                .err()
                .expect("mismatched block should fail");
        assert!(mismatch
            .to_string()
            .contains("DAG_VERIFY_SESSION_VDF_REQUEST_FINGERPRINT_MISMATCH"));

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_session_start_derives_queue_and_sidecar_pressure_for_skip_thresholds() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_proposer_pressure");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let final_chain = make_proposer_final_chain(&storage, proposer_address());

        // The caller-shaped input contains configuration only; the empty sibling
        // runtime drives the legacy empty-pool skip.
        let empty_id =
            service_dag_manager_runtime_begin_proposer_session(&service, proposer_begin_input())
                .unwrap();
        let empty = report_proposer_final_chain_facts(&service, &final_chain, empty_id);
        assert_eq!(empty.status, 1);
        assert_eq!(
            empty.reason_code,
            rustaxa_consensus::dag::DAG_PROPOSER_REASON_TRANSACTION_POOL_EMPTY
        );

        insert_test_queue_transaction(&service, 0x51);
        let sidecar_rlp =
            signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x52; 32]).unwrap());
        service
            .transaction()
            .sidecar
            .insert_non_finalized(keccak256(&sidecar_rlp), sidecar_rlp)
            .unwrap();

        let mut limited_input = proposer_begin_input();
        limited_input.max_non_finalized_transactions = 0;
        let limited_id =
            service_dag_manager_runtime_begin_proposer_session(&service, limited_input).unwrap();
        let limited = report_proposer_final_chain_facts(&service, &final_chain, limited_id);
        assert_eq!(limited.status, 1);
        assert_eq!(
            limited.reason_code,
            rustaxa_consensus::dag::DAG_PROPOSER_REASON_NON_FINALIZED_TRANSACTION_LIMIT
        );

        let ready_id = open_proposer_pack(&service);
        assert!(service
            .dag_transaction_service_proposer_pack_abort(ready_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_commits_transactions_block_graph_and_restart_state() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let key = SigningKey::from_slice(&[0x61; 32]).unwrap();
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let transaction = LegacyTransactionEnvelope::decode(&transaction_rlp).unwrap();
        insert_test_queue_transaction(&service, 0x61);
        let block_rlp = composed_add_block_rlp([1; 32], 1, &[transaction.hash.0]);
        let block_hash = keccak256(&block_rlp);
        let preparation = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                block_rlp.clone(),
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: transaction.hash.0,
                    trx_rlp: transaction_rlp.clone(),
                }],
            ))
            .unwrap();
        assert!(preparation.accepted);
        assert_eq!(preparation.account_requests.len(), 1);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 1);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        let missing_fact = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .err()
            .expect("missing nonce fact must fail before the shared commit");
        assert!(missing_fact
            .to_string()
            .contains("DAG_ADD_BLOCK_ACCOUNT_NONCE_FACT_COUNT_MISMATCH"));
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 1);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);
        assert!(
            !service
                .dag_manager_runtime_load_block(&block_hash.0)
                .unwrap()
                .found
        );
        assert!(storage
            .0
            .transaction()
            .rlp(transaction.hash)
            .unwrap()
            .is_none());

        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                    input_index: 0,
                    account_nonce: U256::zero().to_big_endian(),
                }],
            })
            .unwrap();
        assert!(report.accepted);
        assert!(report.emit_verified);
        assert!(report.gossip);
        assert_eq!(report.queue_erased.len(), 1);
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 1);
        assert!(service
            .transaction()
            .sidecar
            .contains_non_finalized(transaction.hash));
        assert_eq!(
            storage.0.transaction().rlp(transaction.hash).unwrap(),
            Some(transaction_rlp)
        );
        assert_eq!(
            service
                .dag_manager_runtime_load_block(&block_hash.0)
                .unwrap()
                .block_rlp,
            block_rlp
        );

        drop(service);
        let restored = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(restored.dag_manager_runtime_vertex_count().unwrap(), 2);
        assert_eq!(restored.transaction_manager_runtime_transaction_count(), 1);

        let duplicate = restored
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                block_rlp,
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: [0; 32],
                    trx_rlp: Vec::new(),
                }],
            ))
            .unwrap();
        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.cursor_id, 0);

        drop(restored);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_terminal_and_save_false_paths_do_not_persist_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_terminal");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let missing_rlp = composed_add_block_rlp([0x77; 32], 1, &[[0xAA; 32]]);
        let missing = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                missing_rlp,
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: [0; 32],
                    trx_rlp: Vec::new(),
                }],
            ))
            .unwrap();
        assert!(!missing.accepted);
        assert!(!missing.missing_references.is_empty());
        assert_eq!(missing.cursor_id, 0);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        let no_save_rlp = composed_add_block_rlp([1; 32], 1, &[[0xBB; 32]]);
        let no_save_hash = keccak256(&no_save_rlp);
        let object_hash = [0xBC; 32];
        let mut no_save_input = add_block_prepare_input(no_save_rlp, false, Vec::new());
        no_save_input.expected_block_hash = object_hash;
        no_save_input.validate_block_hash = false;
        let no_save = service
            .dag_transaction_service_prepare_add_block(no_save_input)
            .unwrap();
        assert!(no_save.accepted);
        assert!(no_save.account_requests.is_empty());
        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: no_save.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();
        assert!(!report.emit_verified);
        assert!(!report.gossip);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);
        assert!(service
            .dag_manager_runtime_is_block_known(&object_hash)
            .unwrap());
        assert!(!service
            .dag_manager_runtime_is_block_known(&no_save_hash.0)
            .unwrap());
        assert!(
            !service
                .dag_manager_runtime_load_block(&no_save_hash.0)
                .unwrap()
                .found
        );
        assert_eq!(report.counters.dag_blocks, 0);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_object_compatibility_persists_only_supplied_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_object");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let block_rlp = composed_add_block_rlp([1; 32], 1, &[[0xBD; 32]]);
        let canonical_hash = keccak256(&block_rlp);
        let object_hash = [0xBE; 32];
        let mut input = add_block_prepare_input(block_rlp.clone(), true, Vec::new());
        input.expected_block_hash = object_hash;
        input.validate_block_hash = false;

        let preparation = service
            .dag_transaction_service_prepare_add_block(input)
            .unwrap();
        assert!(preparation.accepted);
        assert!(preparation.account_requests.is_empty());
        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();

        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);
        assert_eq!(
            service
                .dag_manager_runtime_load_block(&object_hash)
                .unwrap()
                .block_rlp,
            block_rlp
        );
        assert!(
            !service
                .dag_manager_runtime_load_block(&canonical_hash.0)
                .unwrap()
                .found
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_preserves_finalized_nonce_filtering() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_finalized");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let key = SigningKey::from_slice(&[0x62; 32]).unwrap();
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let transaction = LegacyTransactionEnvelope::decode(&transaction_rlp).unwrap();
        let pbft_block = signed_pbft_block(&SigningKey::from_slice(&[0x63; 32]).unwrap(), 1);
        storage
            .0
            .transaction()
            .write_location(transaction.hash, 1, 0, false)
            .unwrap();
        storage
            .0
            .period()
            .write(1, &period_data_rlp(&pbft_block, &transaction_rlp))
            .unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let block_rlp = composed_add_block_rlp([1; 32], 1, &[transaction.hash.0]);
        let preparation = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                block_rlp,
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: transaction.hash.0,
                    trx_rlp: transaction_rlp,
                }],
            ))
            .unwrap();
        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: preparation.cursor_id,
                account_nonce_facts: vec![DagAddBlockAccountNonceFact {
                    input_index: 0,
                    account_nonce: U256::from(2u64).to_big_endian(),
                }],
            })
            .unwrap();
        assert!(report.accepted);
        assert!(report.queue_erased.is_empty());
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);
        assert!(!service
            .transaction()
            .sidecar
            .contains_non_finalized(transaction.hash));
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_active_cursor_survives_second_terminal_and_malformed_prepares() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_stale");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let first_rlp = composed_add_block_rlp([1; 32], 1, &[]);
        let first = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                first_rlp,
                true,
                Vec::new(),
            ))
            .unwrap();
        let second = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[[0xDD; 32]]),
                false,
                Vec::new(),
            ))
            .err()
            .expect("a second accepted prepare must not replace the active cursor");
        assert!(second
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));

        let terminal = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([0x77; 32], 1, &[]),
                true,
                Vec::new(),
            ))
            .err()
            .expect("a terminal second prepare must leave the active cursor intact");
        assert!(terminal
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));

        let malformed = service
            .dag_transaction_service_prepare_add_block(DagAddBlockPrepareInput {
                expected_block_hash: [0; 32],
                validate_block_hash: true,
                block_rlp: vec![0x80],
                save: true,
                proposed: false,
                transactions: Vec::new(),
            })
            .err()
            .expect("malformed second prepare must leave the active cursor intact");
        assert!(malformed
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));

        let report = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();
        assert_eq!(report.counters.dag_blocks, 1);
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);
        let retry = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .err()
            .expect("a committed cursor must already be gone");
        assert!(retry
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_NOT_STARTED"));

        let malformed_without_active = service
            .dag_transaction_service_prepare_add_block(DagAddBlockPrepareInput {
                expected_block_hash: [0; 32],
                validate_block_hash: true,
                block_rlp: vec![0x80],
                save: true,
                proposed: false,
                transactions: Vec::new(),
            })
            .err()
            .expect("malformed input must be decoded once no cursor is active");
        assert!(malformed_without_active
            .to_string()
            .contains("DAG_ADD_BLOCK_PREPARE_DECODE"));
        let malformed_transaction = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[[0xCC; 32]]),
                true,
                vec![DagAddBlockTransactionPayload {
                    hash: [0xCC; 32],
                    trx_rlp: vec![0x80],
                }],
            ))
            .err()
            .expect("malformed transaction must reject accepted preparation");
        assert!(malformed_transaction
            .to_string()
            .contains("DAG_ADD_BLOCK_PREPARE_TRANSACTION_DECODE"));
        assert_eq!(service.transaction_manager_runtime_transaction_count(), 0);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_abort_is_matching_stale_safe_and_idempotent() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_abort");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let first = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[]),
                true,
                Vec::new(),
            ))
            .unwrap();
        assert!(!service
            .dag_transaction_service_abort_add_block(first.cursor_id + 1)
            .unwrap());
        assert!(service
            .dag_transaction_service_abort_add_block(first.cursor_id)
            .unwrap());
        assert!(!service
            .dag_transaction_service_abort_add_block(first.cursor_id)
            .unwrap());

        let second = service
            .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                composed_add_block_rlp([1; 32], 1, &[]),
                false,
                Vec::new(),
            ))
            .unwrap();
        assert_ne!(first.cursor_id, second.cursor_id);
        assert!(!service
            .dag_transaction_service_abort_add_block(first.cursor_id)
            .unwrap());
        let stale = service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: first.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .err()
            .expect("an old cursor must not consume the active cursor");
        assert!(stale
            .to_string()
            .contains("DAG_ADD_BLOCK_SESSION_CURSOR_MISMATCH"));
        assert!(service
            .dag_transaction_service_abort_add_block(second.cursor_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn composed_add_block_concurrent_prepares_publish_exactly_one_cursor() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_composed_add_concurrent");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = Arc::new(
            *create_dag_transaction_service_from_storage(
                &storage,
                &[1; 32],
                32,
                100,
                sortition_config(),
                queue_config(),
                gas_config(),
                u64::MAX,
            )
            .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(3));
        let handles = [[0xD1; 32], [0xD2; 32]].map(|transaction_hash| {
            let service = Arc::clone(&service);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                service
                    .dag_transaction_service_prepare_add_block(add_block_prepare_input(
                        composed_add_block_rlp([1; 32], 1, &[transaction_hash]),
                        false,
                        Vec::new(),
                    ))
                    .map_err(|error| error.to_string())
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().unwrap());
        let winner = results
            .iter()
            .find_map(|result| result.as_ref().ok())
            .expect("one prepare must acquire the cursor");
        let loser = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one prepare must observe the active cursor");
        assert!(loser.contains("DAG_ADD_BLOCK_SESSION_ALREADY_ACTIVE"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);

        service
            .dag_transaction_service_complete_add_block(DagAddBlockCompletionInput {
                cursor_id: winner.cursor_id,
                account_nonce_facts: Vec::new(),
            })
            .unwrap();
        assert_eq!(service.dag_manager_runtime_vertex_count().unwrap(), 2);

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_prepare_does_not_advance_and_completion_succeeds() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_all_supplied");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let supplied = [7; 32];
        begin_verify_block_session(&service, &[supplied, supplied], &[supplied]);

        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert!(preparation.transactions.is_empty());
        let unadvanced = service_dag_manager_runtime_verify_block_session_next(&service).unwrap();
        assert_eq!(unadvanced.action, 1);
        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            completed.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_prepare_returns_runtime_views_in_canonical_query_order() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_mixed");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let supplied = [8; 32];
        let key = SigningKey::from_slice(&[0x49; 32]).unwrap();
        let queued_rlp = signed_legacy_transaction_rlp(&key);
        let queued = LegacyTransactionEnvelope::decode(&queued_rlp).unwrap();
        service
            .insert_test_queue_transaction(TransactionQueueInsertInput {
                hash: queued.hash.0,
                sender: address_from_signing_key(&key),
                nonce: queued.nonce.to_big_endian(),
                gas_price: queued.gas_price.to_big_endian(),
                gas: queued.gas,
                data_size: queued.data.len(),
                tx_rlp: queued_rlp.clone(),
                proposable: true,
                last_block_number: 0,
            })
            .unwrap();
        let sidecar_rlp =
            signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x4C; 32]).unwrap());
        let sidecar_hash = keccak256(&sidecar_rlp);
        service
            .transaction()
            .sidecar
            .insert_non_finalized(sidecar_hash, sidecar_rlp.clone())
            .unwrap();
        begin_verify_block_session(
            &service,
            &[supplied, queued.hash.0, sidecar_hash.0, queued.hash.0],
            &[supplied],
        );

        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert_eq!(preparation.transactions.len(), 2);
        assert_eq!(preparation.transactions[0].input_index, 0);
        assert_eq!(preparation.transactions[0].hash, queued.hash.0);
        assert_eq!(preparation.transactions[0].tx_rlp, queued_rlp);
        assert_eq!(preparation.transactions[1].input_index, 1);
        assert_eq!(preparation.transactions[1].hash, sidecar_hash.0);
        assert_eq!(preparation.transactions[1].tx_rlp, sidecar_rlp);
        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            completed.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_resolution_rejects_missing_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_missing");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        begin_verify_block_session(&service, &[[10; 32]], &[]);

        let preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert_eq!(preparation.transactions.len(), 1);
        assert!(!preparation.transactions[0].found);
        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: preparation.cursor_id,
                proposal_period: preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert!(completed.complete);
        assert_eq!(
            completed.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_completion_requires_nonce_facts_and_rejects_old_finalized_transactions() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_old_finalized");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let key = SigningKey::from_slice(&[0x4A; 32]).unwrap();
        let transaction_rlp = signed_legacy_transaction_rlp(&key);
        let transaction_hash = keccak256(&transaction_rlp);
        let pbft_block = signed_pbft_block(&SigningKey::from_slice(&[0x4B; 32]).unwrap(), 1);
        storage
            .0
            .transaction()
            .write_location(transaction_hash, 1, 0, false)
            .unwrap();
        storage
            .0
            .period()
            .write(1, &period_data_rlp(&pbft_block, &transaction_rlp))
            .unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        begin_verify_block_session(&service, &[transaction_hash.0], &[]);

        let missing_fact_preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        assert!(missing_fact_preparation.transactions[0].found);
        let missing_fact = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: missing_fact_preparation.cursor_id,
                proposal_period: missing_fact_preparation.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .err()
        .expect("missing finalized sender fact must reject completion");
        assert!(missing_fact
            .to_string()
            .contains("TM_PROPOSAL_FINALIZED_ACCOUNT_NONCE_FACT_MISSING"));
        assert_eq!(
            service_dag_manager_runtime_verify_block_session_next(&service)
                .unwrap()
                .action,
            1
        );

        begin_verify_block_session(&service, &[transaction_hash.0], &[]);
        let old_preparation =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        let old = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: old_preparation.cursor_id,
                proposal_period: old_preparation.proposal_period,
                account_nonce_facts: vec![TransactionQueueAccountNonceFact {
                    sender: address_from_signing_key(&key),
                    account_found: true,
                    account_nonce: U256::from(2u64).to_big_endian(),
                }],
            },
        )
        .unwrap();
        assert!(old.complete);
        assert_eq!(
            old.reject_code,
            rustaxa_consensus::dag::DAG_VERIFY_REJECT_MISSING_TRANSACTION
        );

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_block_completion_rejects_stale_period_and_action_misuse_without_advancing() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_verify_misuse");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();

        let not_started =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .err()
                .expect("missing session must reject preparation");
        assert!(not_started
            .to_string()
            .contains("DAG_VERIFY_SESSION_NOT_STARTED"));

        begin_verify_block_session(&service, &[[12; 32]], &[[12; 32]]);
        let stale = service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
            .unwrap();
        begin_verify_block_session(&service, &[[11; 32]], &[[11; 32]]);
        let current =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .unwrap();
        let stale_error = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: stale.cursor_id,
                proposal_period: stale.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .err()
        .expect("stale cursor must reject completion");
        assert!(stale_error
            .to_string()
            .contains("DAG_VERIFY_SESSION_TRANSACTION_CURSOR_MISMATCH"));
        assert_eq!(
            service_dag_manager_runtime_verify_block_session_next(&service)
                .unwrap()
                .action,
            1
        );

        let period_error = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: current.cursor_id,
                proposal_period: current.proposal_period + 1,
                account_nonce_facts: Vec::new(),
            },
        )
        .err()
        .expect("wrong proposal period must reject completion");
        assert!(period_error
            .to_string()
            .contains("DAG_VERIFY_SESSION_TRANSACTION_PERIOD_MISMATCH"));

        let completed = service_dag_manager_runtime_verify_block_session_complete_transactions(
            &service,
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: current.cursor_id,
                proposal_period: current.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            completed.action,
            DAG_VERIFY_SESSION_ACTION_AUTHORIZATION_FACTS
        );
        let wrong_action =
            service_dag_manager_runtime_verify_block_session_prepare_transactions(&service)
                .err()
                .expect("wrong action must reject preparation");
        assert!(wrong_action
            .to_string()
            .contains("DAG_VERIFY_SESSION_UNEXPECTED_TRANSACTION_COMPLETION"));

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proposer_pack_estimate_finalize_retains_payload_and_cache_only_reuses_it() {
        let dir = unique_temp_dir("rustaxa_dag_transaction_service_pack_estimate");
        let storage = create_storage(dir.to_str().unwrap()).unwrap();
        let service = create_dag_transaction_service_from_storage(
            &storage,
            &[1; 32],
            32,
            100,
            sortition_config(),
            queue_config(),
            gas_config(),
            u64::MAX,
        )
        .unwrap();
        let key = SigningKey::from_slice(&[0x47; 32]).unwrap();
        let tx_rlp = signed_legacy_transaction_rlp(&key);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        service
            .insert_test_queue_transaction(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: address_from_signing_key(&key),
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp: tx_rlp.clone(),
                proposable: true,
                last_block_number: 0,
            })
            .unwrap();
        let session_id = open_proposer_pack(&service);
        let prepare = service
            .dag_transaction_service_proposer_pack_prepare(session_id, false, 21_000, 0, 10)
            .expect("estimate prepare");
        assert_eq!(prepare.action, 1);
        assert_eq!(prepare.transaction_estimate_requests.len(), 1);
        assert!(prepare.selected_transactions.is_empty());
        let estimate = &prepare.transaction_estimate_requests[0];
        let start_vdf = service
            .dag_transaction_service_proposer_pack_finalize(
                session_id,
                vec![TransactionPackSessionEstimateInput {
                    hash: estimate.hash,
                    gas_used: 21_000,
                    last_block_number: 10,
                    result_rlp: vec![0xC0],
                }],
            )
            .expect("estimate finalize");
        assert_eq!(start_vdf.action, 2);
        assert!(start_vdf.transaction_estimate_requests.is_empty());
        assert!(start_vdf.selected_transactions.is_empty());

        let sign = service_dag_manager_runtime_proposer_session_report_vdf_proof(
            &service,
            session_id,
            DagProposerVdfProofReport {
                proof_ok: true,
                vdf_rlp: vec![0xC0],
            },
        )
        .expect("VDF proof");
        let add = service_dag_manager_runtime_proposer_session_report_signing(
            &service,
            session_id,
            DagProposerSigningReport {
                signature: sign_hash(sign.signing_hash),
            },
        )
        .expect("signing");
        assert_eq!(add.action, 6);
        assert_eq!(add.selected_transactions.len(), 1);
        assert_eq!(add.selected_transactions[0].hash, envelope.hash.0);
        assert_eq!(add.selected_transactions[0].gas_used, 21_000);
        assert_eq!(add.selected_transactions[0].tx_rlp, tx_rlp);
        service_dag_manager_runtime_proposer_session_report_add_block(
            &service,
            session_id,
            DagProposerAddBlockReport {
                accepted: true,
                duplicate: false,
                expired: false,
                missing_references: Vec::new(),
            },
        )
        .expect("add report");

        let cache_id = open_proposer_pack(&service);
        let cache_only = service
            .dag_transaction_service_proposer_pack_prepare(cache_id, false, 21_000, 0, 10)
            .expect("cache-only prepare");
        assert_eq!(cache_only.action, 2);
        assert!(cache_only.transaction_estimate_requests.is_empty());
        assert!(!service
            .transaction()
            .transaction_packing
            .is_active()
            .unwrap());
        assert!(service
            .dag_transaction_service_proposer_pack_abort(cache_id)
            .unwrap());

        drop(service);
        drop(storage);
        let _ = fs::remove_dir_all(dir);
    }
}
