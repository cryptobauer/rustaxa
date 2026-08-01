use crate::ffi::rustaxa_ffi::*;
use crate::ffi::{BridgeFinalChain, BridgePbftService, BridgeStorage};
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
    DagGraphView, DagProposerPackPrepareRequest, DagProposerPackStep, DagTransactionService,
    DagTransactionServiceConfig,
    DagVerifyBlockTransactionCompletionReport as NativeDagVerifyBlockTransactionCompletionReport,
    DagVerifyBlockVdfRequest as NativeDagVerifyBlockVdfRequest,
};
use rustaxa_consensus::pbft_manager::{
    PbftFinalizationExecutorBoundary, PbftFinalizationExecutorStartRequest,
};
use rustaxa_consensus::transaction_packing_service::{
    TransactionPackingEstimate, TransactionPackingSelection,
};
use rustaxa_consensus::transaction_service::{
    finalized_status_facts_from_transaction_list_rlp, DagTransactionSaveInput,
    TransactionServiceAccountNonceFact, TransactionServiceCompatibilityPackFinalized,
    TransactionServiceCompatibilityPackPrepared, TransactionServiceCompatibilityPackRequest,
    TransactionServiceConfig, TransactionServiceEstimateRequest,
    TransactionServiceFinalizedFilterRequest, TransactionServiceFinalizedStatusFact,
    TransactionServiceGasEstimationResult, TransactionServicePackEstimate,
    TransactionServicePayload, TransactionServiceTransactionView,
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
    /// Starts or resumes PBFT finalization against the privately owned DAG root.
    ///
    /// The PBFT service owns executor serialization and the supplied request;
    /// this adapter only composes it with the DAG/transaction sibling without
    /// exposing that sibling to another bridge module. Native locks and guards
    /// remain inside the two application services, and errors propagate without
    /// publishing a partial executor boundary.
    pub(crate) fn start_finalization_executor(
        &self,
        runtime: &BridgePbftService,
        request: PbftFinalizationExecutorStartRequest,
    ) -> anyhow::Result<PbftFinalizationExecutorBoundary> {
        runtime.0.start_finalization_executor(&self.root, request)
    }

    /// Advances one cursor-bound PBFT finalization action against private state.
    ///
    /// The supplied action and leaf facts are interpreted by the native PBFT
    /// service. DAG, sortition, or transaction mutation is reached only through
    /// this task adapter; the native root, its locks, and its guards cannot
    /// escape into the PBFT bridge. Cursor, action, storage, and leaf failures
    /// are returned unchanged in the native executor result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance_finalization_action(
        &self,
        runtime: &BridgePbftService,
        cursor: u32,
        action: u8,
        last_block: u64,
        request_period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
    ) -> anyhow::Result<PbftFinalizationExecutorBoundary> {
        runtime.0.advance_finalization_action(
            &self.root,
            cursor,
            action,
            last_block,
            request_period,
            retention_window,
            account_nonce_facts,
        )
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

/// Applies finalized status from a canonical RLP transaction list.
///
/// This is the retained non-PBFT compatibility API for partially populated
/// legacy `PeriodData` objects. C++ supplies one opaque transaction-list RLP
/// plus external-EVM account nonce facts; Rust derives hashes and owns all
/// mutation. No per-transaction fact or mutation report crosses CXX.
pub fn service_update_finalized_transactions_status_from_transaction_list(
    service: &BridgeDagTransactionService,
    period: u64,
    retention_window: u64,
    account_nonce_facts: Vec<TransactionQueueAccountNonceFact>,
    transaction_list_rlp: Vec<u8>,
) -> Result<()> {
    let facts: Vec<TransactionServiceFinalizedStatusFact> =
        finalized_status_facts_from_transaction_list_rlp(&transaction_list_rlp)?;
    service.root.transaction_update_finalized_status(
        period,
        retention_window,
        bridge_to_service_account_nonce_facts(account_nonce_facts),
        facts,
    )?;
    Ok(())
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
    service
        .root
        .report_verify_block_authorization_with_final_chain(&final_chain.0)
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
    service
        .root
        .report_proposer_final_chain_facts_with_final_chain(session_id, &final_chain.0)
        .map(proposer_session_step_to_bridge)
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
