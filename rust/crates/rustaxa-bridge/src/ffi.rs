use crate::dag::*;
use crate::final_chain::*;
use crate::pbft_chain::*;
use crate::period_data_queue::*;
use crate::proposed_blocks::*;
use crate::sortition::*;
use crate::storage::*;
use crate::vdf::*;
use crate::verified_votes::*;
use rustaxa_consensus::dag::{DagGraph, DagManagerState};
use rustaxa_consensus::pbft_chain::PbftChain;
use rustaxa_consensus::period_data_queue::PeriodDataQueue;
use rustaxa_consensus::proposed_blocks::ProposedBlocks;
use rustaxa_consensus::sortition::SortitionParamsManager;
use rustaxa_consensus::verified_votes::VerifiedVotes;
use rustaxa_consensus::FinalChain;
use rustaxa_storage::Storage;
use rustaxa_storage::StorageWriteBatch;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex;

pub struct BridgeStorage(
    pub Arc<Storage>,
    pub Mutex<HashMap<u64, StorageWriteBatch>>,
    pub AtomicU64,
);

pub struct BridgeFinalChain(pub FinalChain);

pub struct BridgeDagGraph(pub DagGraph);

/// Storage-free DagManager state wrapper used for in-memory DAG graph/index
/// logic only. Persistence is intentionally handled by `BridgeDagManagerRuntime`.
pub struct BridgeDagManagerState(pub DagManagerState);

/// DagManager runtime wrapper coupling deterministic in-memory state with the
/// shared Rust storage handle used for direct DAG persistence and reads.
pub struct BridgeDagManagerRuntime {
    pub state: DagManagerState,
    pub storage: Arc<Storage>,
}

pub struct BridgePbftChain(pub PbftChain);

pub struct BridgeProposedBlocks(pub ProposedBlocks);

pub struct BridgePeriodDataQueue(pub PeriodDataQueue);

pub struct BridgeVerifiedVotes(pub VerifiedVotes);

pub struct BridgeSortitionParamsManager(pub SortitionParamsManager);

#[cxx::bridge(namespace = "rustaxa")]
pub mod rustaxa_ffi {
    struct BlockPeriod {
        period: u64,
        position: u32,
    }

    struct BlockPeriodLookup {
        found: bool,
        period: u64,
        position: u32,
    }

    struct BlockRlp {
        data: Vec<u8>,
    }

    /// Optional DAG block payload lookup result.
    struct DagBlockLookup {
        found: bool,
        block_rlp: Vec<u8>,
    }

    /// Persisted DAG block/edge counters loaded from storage status fields.
    struct DagPersistenceCounters {
        dag_blocks: u64,
        dag_edges: u64,
    }

    struct LevelBlocks {
        level: u64,
        blocks: Vec<BlockRlp>,
    }

    struct PeriodLookup {
        found: bool,
        period: u64,
    }

    struct PeriodLambda {
        found: bool,
        value: u32,
    }

    struct PeriodRlp {
        period: u64,
        data: Vec<u8>,
    }

    struct TxRlp {
        data: Vec<u8>,
    }

    struct HashPeriod {
        hash: [u8; 32],
        period: u64,
    }

    struct VoteRlp {
        data: Vec<u8>,
    }

    struct PbftChainHeadPayload {
        head_hash: [u8; 32],
        size: u64,
        non_empty_size: u64,
        last_pbft_block_hash: [u8; 32],
        last_non_null_anchor_hash: [u8; 32],
    }

    struct PbftBlockValidationResult {
        ok: bool,
        code: u8,
        expected_period: u64,
        actual_period: u64,
        expected_prev_hash: [u8; 32],
        actual_prev_hash: [u8; 32],
    }

    struct ProposedBlockLookup {
        found: bool,
        is_valid: bool,
        block_rlp: Vec<u8>,
    }

    struct ProposedBlockPeriodHashes {
        period: u64,
        block_hashes: Vec<DagHash>,
    }

    struct ProposedBlockSnapshotEntry {
        period: u64,
        block_hash: [u8; 32],
        block_rlp: Vec<u8>,
        is_valid: bool,
    }

    struct PeriodDataQueueEntryRef {
        entry_id: u64,
        period: u64,
    }

    struct PeriodDataQueuePushOutcome {
        accepted: bool,
        clear_existing: bool,
        expected_next_period: u64,
        actual_period: u64,
        current_period: u64,
        effective_size: usize,
    }

    struct PeriodDataQueuePopPlan {
        entry_id: u64,
        use_last_block_cert_votes: bool,
        next_entry_id: u64,
        current_period: u64,
        effective_size: usize,
    }

    struct PeriodDataQueueLastEntryLookup {
        found: bool,
        entry_id: u64,
        period: u64,
    }

    struct VerifiedVotePayload {
        vote_hash: [u8; 32],
        block_hash: [u8; 32],
        voter: [u8; 20],
        period: u64,
        round: u64,
        step: u64,
        vote_type: u8,
        weight: u64,
    }

    struct UniqueVoterCheckOutcome {
        is_unique: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
    }

    struct UniqueVoterInsertOutcome {
        accepted: bool,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        used_secondary_slot: bool,
        duplicate_vote_hash: bool,
    }

    struct VotedValueInsertOutcome {
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
    }

    struct AtomicVoteInsertOutcome {
        inserted: bool,
        total_weight: u64,
        votes_count: usize,
        conflict_found: bool,
        conflicting_vote_hash: [u8; 32],
        used_secondary_slot: bool,
        duplicate_vote_hash: bool,
    }

    struct ThresholdDecisionOutcome {
        t_plus_one_reached: bool,
        network_t_plus_one_step_updated: bool,
        two_t_plus_one_reached: bool,
        two_t_plus_one_kind_found: bool,
        two_t_plus_one_kind: u8,
        two_t_plus_one_round_found: bool,
        two_t_plus_one_inserted: bool,
    }

    struct TwoTPlusOneInsertOutcome {
        round_found: bool,
        inserted: bool,
    }

    struct DetermineNewRoundOutcome {
        found: bool,
        new_round: u64,
        source_round: u64,
        source_kind: u8,
        block_hash: [u8; 32],
        step: u64,
    }

    struct TwoTPlusOneVotedBlockLookup {
        found: bool,
        block_hash: [u8; 32],
        step: u64,
    }

    struct TwoTPlusOneVotesLookup {
        found: bool,
        block_hash: [u8; 32],
        step: u64,
        vote_hashes: Vec<DagHash>,
    }

    struct NetworkTPlusOneStepLookup {
        found: bool,
        step: u64,
    }

    struct TwoTPlusOneSnapshotEntry {
        period: u64,
        round: u64,
        kind: u8,
        block_hash: [u8; 32],
        step: u64,
    }

    struct RoundMarkerSnapshot {
        period: u64,
        round: u64,
        network_t_plus_one_step: u64,
    }

    struct FinalChainBlockNumberLookup {
        found: bool,
        value: u64,
    }

    struct GenesisAccount {
        address: [u8; 20],
        balance: Vec<u8>,
    }

    struct GenesisValidator {
        address: [u8; 20],
        owner: [u8; 20],
        vrf_key: [u8; 32],
        commission: u16,
        description: String,
        endpoint: String,
        total_stake: Vec<u8>,
    }

    struct GenesisDposConfig {
        eligibility_balance_threshold: Vec<u8>,
        vote_eligibility_balance_step: Vec<u8>,
        validator_maximum_stake: Vec<u8>,
    }

    struct AccountLookup {
        found: bool,
        nonce: u64,
        balance: Vec<u8>,
        storage_root_hash: [u8; 32],
        code_hash: [u8; 32],
        code_size: u64,
    }

    struct DposValidatorStake {
        address: [u8; 20],
        stake: Vec<u8>,
    }

    struct DposValidatorVoteCount {
        address: [u8; 20],
        vote_count: u64,
    }

    struct FinalChainCall {
        block_number: u64,
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        input: Vec<u8>,
    }

    struct FinalChainCallOutcome {
        code_retval: Vec<u8>,
        gas_used: u64,
        code_err: String,
        consensus_err: String,
    }

    struct FinalizationOutcome {
        block_header_rlp: Vec<u8>,
        receipts: Vec<ReceiptRlp>,
    }

    struct FinalizationTransaction {
        hash: [u8; 32],
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        nonce: u64,
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        data: Vec<u8>,
        rlp: Vec<u8>,
    }

    struct ReceiptRlp {
        data: Vec<u8>,
    }

    struct DagHash {
        hash: [u8; 32],
    }

    struct FinalizationDagBlock {
        author: [u8; 20],
        transaction_hashes: Vec<DagHash>,
    }

    struct DagLevelHashes {
        level: u64,
        hashes: Vec<DagHash>,
    }

    struct DagOrder {
        found: bool,
        hashes: Vec<DagHash>,
    }

    struct DagFrontier {
        pivot: [u8; 32],
        tips: Vec<DagHash>,
    }

    struct DagReferenceMetadata {
        hash: [u8; 32],
        found: bool,
        level: u64,
    }

    struct DagPivotTipsValidation {
        ok: bool,
        expected_level: u64,
        level_matches: bool,
        missing_references: Vec<DagHash>,
    }

    /// C++-originated payload for Rust DAG block verification prechecks.
    struct DagVerifyPrecheckBlock {
        level: u64,
        pivot: [u8; 32],
        tips: Vec<DagHash>,
    }

    /// Rust DAG block verification precheck decision.
    struct DagVerifyPrecheckResult {
        continue_validation: bool,
        reject_code: u32,
        proposal_period_found: bool,
        proposal_period: u64,
    }

    /// Per-tip gas metadata for Rust DAG verification gas decisions.
    struct DagTipGas {
        found: bool,
        gas_estimation: u64,
    }

    /// C++-originated payload for Rust transaction availability decisions.
    struct DagVerifyTransactionAvailabilityInput {
        expected_transactions: u64,
        resolved_transactions: u64,
    }

    /// Rust transaction availability decision.
    struct DagVerifyTransactionAvailabilityResult {
        continue_validation: bool,
        reject_code: u32,
    }

    /// C++-originated payload for Rust VDF verification preparation.
    struct DagVerifyVdfPrepareInput {
        vrf_key_found: bool,
        eligible_vote_count: u64,
        vdf_max_vote_count: u64,
    }

    /// Rust VDF verification preparation result.
    struct DagVerifyVdfPrepareResult {
        continue_validation: bool,
        reject_code: u32,
        reason_code: u32,
        vote_count: u64,
        max_vote_count: u64,
    }

    /// C++-originated payload for Rust authorization decisions.
    struct DagVerifyAuthorizationInput {
        vdf_valid: bool,
        dpos_snapshot_available: bool,
        dpos_eligible: bool,
    }

    /// Rust authorization decision.
    struct DagVerifyAuthorizationResult {
        continue_validation: bool,
        reject_code: u32,
        reason_code: u32,
    }

    /// C++-originated payload for Rust DAG VDF sortition verification.
    struct DagVerifyVdfSortitionInput {
        block_rlp: Vec<u8>,
        vdf_input: Vec<u8>,
        sortition_params: SortitionRuntimeParams,
        vrf_output: Vec<u8>,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
    }

    /// Rust DAG VDF sortition verification result.
    struct DagVerifyVdfSortitionResult {
        vdf_status: u8,
        difficulty: u16,
        expected_difficulty: u16,
    }

    /// C++-originated VDF and DPoS facts for Rust authorization decisions.
    struct DagVerifyVdfDposFacts {
        vrf_key_found: bool,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
        vdf_status: u8,
        dpos_status: u8,
    }

    /// Rust-collected DPoS and VRF facts for DAG authorization.
    struct DagDposAuthorizationFacts {
        vrf_key_found: bool,
        vrf_key: Vec<u8>,
        sender_eligible_vote_count: u64,
        vdf_sortition_max_vote_count: u64,
        eligibility_status: u8,
    }

    /// Rust VDF and DPoS authorization decision.
    struct DagVerifyVdfDposDecision {
        continue_validation: bool,
        reject_code: u32,
        reason_code: u32,
        vote_count: u64,
        max_vote_count: u64,
    }

    /// C++-originated payload for Rust gas verification decisions.
    struct DagVerifyGasInput {
        block_gas_estimation: u64,
        estimated_transactions_weight: u64,
        dag_gas_limit: u64,
        pbft_gas_limit: u64,
        tip_gas_estimations: Vec<DagTipGas>,
    }

    /// Rust gas verification decision.
    struct DagVerifyGasResult {
        continue_validation: bool,
        reject_code: u32,
    }

    struct DagManagerBlock {
        hash: [u8; 32],
        pivot: [u8; 32],
        tips: Vec<DagHash>,
        level: u64,
        difficulty: u32,
    }

    struct DagManagerSnapshot {
        old_anchor: [u8; 32],
        anchor: [u8; 32],
        anchor_level: u64,
        period: u64,
        max_level: u64,
        dag_expiry_level: u64,
        non_finalized_min_difficulty: u32,
        non_finalized_blocks: Vec<DagManagerBlock>,
    }

    struct DagManagerAnchors {
        old_anchor: [u8; 32],
        anchor: [u8; 32],
    }

    struct DagManagerNonFinalizedSize {
        levels: u64,
        blocks: u64,
    }

    struct SortitionRuntimeConfig {
        threshold_upper: u16,
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
        changes_count_for_average: u16,
        dag_efficiency_target_low: u16,
        dag_efficiency_target_high: u16,
        changing_interval: u16,
        computation_interval: u16,
    }

    struct SortitionRuntimeParams {
        threshold_upper: u16,
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
    }

    struct SortitionParamsChangePayload {
        period: u64,
        interval_efficiency: u16,
        threshold_upper: u16,
    }

    struct SortitionParamsChangeResult {
        changed: bool,
        period: u64,
        interval_efficiency: u16,
        threshold_upper: u16,
    }

    struct SortitionEfficiencyResult {
        ok: bool,
        value: u16,
        error: String,
    }

    extern "Rust" {
        type WesolowskiVdf;
        type CancellationToken;
        type Solution;

        pub fn make_vdf(
            lambda: u32,
            time_bits: u32,
            input: &[u8],
            modulus: &[u8],
        ) -> Box<WesolowskiVdf>;

        pub fn make_solution(proof: &[u8], output: &[u8]) -> Box<Solution>;

        pub fn make_cancellation_token() -> Box<CancellationToken>;
        pub unsafe fn make_cancellation_token_with_atomic(
            atomic_ptr: *const bool,
        ) -> Box<CancellationToken>;
        pub fn cancellation_token_cancel(token: &CancellationToken);

        pub fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution>;
        pub fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool;

        pub fn solution_get_proof(solution: &Solution) -> &[u8];
        pub fn solution_get_output(solution: &Solution) -> &[u8];

        // Consensus DAG

        type BridgeDagGraph;

        pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph>;
        pub fn dag_vertex_count(self: &BridgeDagGraph) -> usize;
        pub fn dag_edge_count(self: &BridgeDagGraph) -> usize;
        pub fn dag_has_vertex(self: &BridgeDagGraph, vertex: &[u8; 32]) -> bool;
        pub fn dag_add_vertex_edges(
            self: &mut BridgeDagGraph,
            new_vertex: &[u8; 32],
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> bool;
        pub fn dag_leaves(self: &BridgeDagGraph) -> Vec<DagHash>;
        pub fn dag_ghost_path(self: &BridgeDagGraph, root: &[u8; 32]) -> Vec<DagHash>;
        pub fn dag_compute_order(
            self: &BridgeDagGraph,
            anchor: &[u8; 32],
            non_finalized_blocks: Vec<DagLevelHashes>,
        ) -> DagOrder;
        pub fn dag_derive_frontier(ghost_path: Vec<DagHash>, leaves: Vec<DagHash>) -> DagFrontier;
        pub fn dag_validate_pivot_tips_metadata(
            block_level: u64,
            pivot: DagReferenceMetadata,
            tips: Vec<DagReferenceMetadata>,
        ) -> DagPivotTipsValidation;
        pub fn dag_clear(self: &mut BridgeDagGraph);
        pub fn dag_graphviz_dot(self: &BridgeDagGraph) -> String;

        type BridgeDagManagerState;

        pub fn create_dag_manager_state(
            genesis: &[u8; 32],
            dag_expiry_limit: u32,
        ) -> Result<Box<BridgeDagManagerState>>;
        pub fn dag_manager_rebuild(
            self: &mut BridgeDagManagerState,
            snapshot: DagManagerSnapshot,
        ) -> Result<()>;
        pub fn dag_manager_add_block(
            self: &mut BridgeDagManagerState,
            block: DagManagerBlock,
        ) -> Result<()>;
        pub fn dag_manager_validate_pivot_tips(
            self: &BridgeDagManagerState,
            block_level: u64,
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> DagPivotTipsValidation;
        pub fn dag_manager_compute_order(
            self: &BridgeDagManagerState,
            anchor: &[u8; 32],
        ) -> DagOrder;
        pub fn dag_manager_frontier(self: &BridgeDagManagerState) -> DagFrontier;
        pub fn dag_manager_ghost_path(
            self: &BridgeDagManagerState,
            source: &[u8; 32],
        ) -> Vec<DagHash>;
        pub fn dag_manager_anchor_ghost_path(self: &BridgeDagManagerState) -> Vec<DagHash>;
        pub fn dag_manager_graphviz_dot(self: &BridgeDagManagerState, pivot_tree: bool) -> String;
        pub fn dag_manager_vertex_count(self: &BridgeDagManagerState) -> usize;
        pub fn dag_manager_edge_count(self: &BridgeDagManagerState) -> usize;
        pub fn dag_manager_max_level(self: &BridgeDagManagerState) -> u64;
        pub fn dag_manager_latest_period(self: &BridgeDagManagerState) -> u64;
        pub fn dag_manager_anchors(self: &BridgeDagManagerState) -> DagManagerAnchors;
        pub fn dag_manager_dag_expiry_limit(self: &BridgeDagManagerState) -> u32;
        pub fn dag_manager_dag_expiry_level(self: &BridgeDagManagerState) -> u64;
        pub fn dag_manager_non_finalized_blocks(
            self: &BridgeDagManagerState,
        ) -> Vec<DagLevelHashes>;
        pub fn dag_manager_non_finalized_blocks_size(
            self: &BridgeDagManagerState,
        ) -> DagManagerNonFinalizedSize;
        pub fn dag_manager_non_finalized_min_difficulty(self: &BridgeDagManagerState) -> u32;

        type BridgeDagManagerRuntime;

        pub fn create_dag_manager_runtime_from_storage(
            genesis: &[u8; 32],
            dag_expiry_limit: u32,
            storage: &BridgeStorage,
        ) -> Result<Box<BridgeDagManagerRuntime>>;
        pub fn dag_manager_runtime_rebuild(
            self: &mut BridgeDagManagerRuntime,
            snapshot: DagManagerSnapshot,
        ) -> Result<()>;
        pub fn dag_manager_runtime_add_block(
            self: &mut BridgeDagManagerRuntime,
            block: DagManagerBlock,
        ) -> Result<()>;
        pub fn dag_manager_runtime_compute_order(
            self: &BridgeDagManagerRuntime,
            anchor: &[u8; 32],
        ) -> DagOrder;
        pub fn dag_manager_runtime_frontier(self: &BridgeDagManagerRuntime) -> DagFrontier;
        pub fn dag_manager_runtime_ghost_path(
            self: &BridgeDagManagerRuntime,
            source: &[u8; 32],
        ) -> Vec<DagHash>;
        pub fn dag_manager_runtime_anchor_ghost_path(
            self: &BridgeDagManagerRuntime,
        ) -> Vec<DagHash>;
        pub fn dag_manager_runtime_graphviz_dot(
            self: &BridgeDagManagerRuntime,
            pivot_tree: bool,
        ) -> String;
        pub fn dag_manager_runtime_vertex_count(self: &BridgeDagManagerRuntime) -> usize;
        pub fn dag_manager_runtime_edge_count(self: &BridgeDagManagerRuntime) -> usize;
        pub fn dag_manager_runtime_max_level(self: &BridgeDagManagerRuntime) -> u64;
        pub fn dag_manager_runtime_latest_period(self: &BridgeDagManagerRuntime) -> u64;
        pub fn dag_manager_runtime_anchors(self: &BridgeDagManagerRuntime) -> DagManagerAnchors;
        pub fn dag_manager_runtime_dag_expiry_limit(self: &BridgeDagManagerRuntime) -> u32;
        pub fn dag_manager_runtime_dag_expiry_level(self: &BridgeDagManagerRuntime) -> u64;
        pub fn dag_manager_runtime_non_finalized_blocks(
            self: &BridgeDagManagerRuntime,
        ) -> Vec<DagLevelHashes>;
        pub fn dag_manager_runtime_non_finalized_blocks_size(
            self: &BridgeDagManagerRuntime,
        ) -> DagManagerNonFinalizedSize;
        pub fn dag_manager_runtime_non_finalized_min_difficulty(
            self: &BridgeDagManagerRuntime,
        ) -> u32;
        pub fn dag_manager_runtime_block_exists(
            self: &BridgeDagManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<bool>;
        pub fn dag_manager_runtime_load_block(
            self: &BridgeDagManagerRuntime,
            hash: &[u8; 32],
        ) -> Result<DagBlockLookup>;
        pub fn dag_manager_runtime_save_block(
            self: &BridgeDagManagerRuntime,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn dag_manager_runtime_ensure_proposal_period_mapping(
            self: &BridgeDagManagerRuntime,
            level: u64,
            period: u64,
        ) -> Result<bool>;
        pub fn dag_manager_runtime_persistence_counters(
            self: &BridgeDagManagerRuntime,
        ) -> Result<DagPersistenceCounters>;
        pub fn dag_manager_runtime_verify_precheck(
            self: &BridgeDagManagerRuntime,
            block: DagVerifyPrecheckBlock,
        ) -> Result<DagVerifyPrecheckResult>;
        pub fn dag_verify_transaction_availability(
            input: DagVerifyTransactionAvailabilityInput,
        ) -> DagVerifyTransactionAvailabilityResult;
        pub fn dag_verify_vdf_prepare(input: DagVerifyVdfPrepareInput)
            -> DagVerifyVdfPrepareResult;
        pub fn dag_vdf_vrf_proof(block_rlp: Vec<u8>) -> Result<Vec<u8>>;
        pub fn dag_verify_vdf_sortition(
            input: DagVerifyVdfSortitionInput,
        ) -> Result<DagVerifyVdfSortitionResult>;
        pub fn dag_verify_authorization(
            input: DagVerifyAuthorizationInput,
        ) -> DagVerifyAuthorizationResult;
        pub fn dag_decide_vdf_dpos_authorization(
            facts: DagVerifyVdfDposFacts,
        ) -> DagVerifyVdfDposDecision;
        pub fn dag_verify_gas(input: DagVerifyGasInput) -> Result<DagVerifyGasResult>;

        // Consensus PBFT chain

        type BridgePbftChain;

        pub fn create_pbft_chain(head: PbftChainHeadPayload) -> Result<Box<BridgePbftChain>>;
        pub fn pbft_chain_head(self: &BridgePbftChain) -> PbftChainHeadPayload;
        pub fn pbft_chain_project_update(
            self: &BridgePbftChain,
            block_hash: &[u8; 32],
            anchor_hash: &[u8; 32],
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_project_legacy_json_head(
            self: &BridgePbftChain,
            block_hash: &[u8; 32],
            increments_non_empty_size: bool,
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_update(
            self: &mut BridgePbftChain,
            block_hash: &[u8; 32],
            anchor_hash: &[u8; 32],
        ) -> Result<PbftChainHeadPayload>;
        pub fn pbft_chain_validate_block(
            self: &BridgePbftChain,
            period: u64,
            prev_hash: &[u8; 32],
        ) -> PbftBlockValidationResult;

        // Consensus proposed PBFT blocks

        type BridgeProposedBlocks;

        pub fn create_proposed_blocks_index() -> Box<BridgeProposedBlocks>;
        pub fn proposed_blocks_push(
            self: &mut BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> bool;
        pub fn proposed_blocks_mark_valid(
            self: &mut BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> Result<()>;
        pub fn proposed_blocks_get(
            self: &BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> ProposedBlockLookup;
        pub fn proposed_blocks_contains(
            self: &BridgeProposedBlocks,
            period: u64,
            block_hash: &[u8; 32],
        ) -> bool;
        pub fn proposed_blocks_cleanup_candidates(
            self: &BridgeProposedBlocks,
            period: u64,
        ) -> Vec<ProposedBlockPeriodHashes>;
        pub fn proposed_blocks_remove_period(self: &mut BridgeProposedBlocks, period: u64);
        pub fn proposed_blocks_old_blocks_message(
            self: &BridgeProposedBlocks,
            current_period: u64,
        ) -> String;
        pub fn proposed_blocks_snapshot_entries(
            self: &BridgeProposedBlocks,
        ) -> Vec<ProposedBlockSnapshotEntry>;
        pub fn proposed_blocks_snapshot(
            self: &BridgeProposedBlocks,
        ) -> Vec<ProposedBlockPeriodHashes>;

        // Consensus period-data queue

        type BridgePeriodDataQueue;

        pub fn create_period_data_queue() -> Box<BridgePeriodDataQueue>;
        pub fn period_data_queue_period(self: &BridgePeriodDataQueue) -> u64;
        pub fn period_data_queue_size(self: &BridgePeriodDataQueue) -> usize;
        pub fn period_data_queue_empty(self: &BridgePeriodDataQueue) -> bool;
        pub fn period_data_queue_clear(self: &mut BridgePeriodDataQueue);
        pub fn period_data_queue_push(
            self: &mut BridgePeriodDataQueue,
            entry_id: u64,
            period: u64,
            max_pbft_size: u64,
            current_block_cert_votes_count: usize,
        ) -> Result<PeriodDataQueuePushOutcome>;
        pub fn period_data_queue_pop(
            self: &mut BridgePeriodDataQueue,
        ) -> Result<PeriodDataQueuePopPlan>;
        pub fn period_data_queue_last_entry(
            self: &BridgePeriodDataQueue,
        ) -> PeriodDataQueueLastEntryLookup;
        pub fn period_data_queue_clean_old_data(
            self: &mut BridgePeriodDataQueue,
            period: u64,
        ) -> Vec<PeriodDataQueueEntryRef>;

        // Consensus verified votes

        type BridgeVerifiedVotes;

        pub fn create_verified_votes_index() -> Box<BridgeVerifiedVotes>;
        pub fn verified_votes_size(self: &BridgeVerifiedVotes) -> u64;
        pub fn verified_votes_check_unique_voter(
            self: &BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<UniqueVoterCheckOutcome>;
        pub fn verified_votes_insert_unique_voter(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<UniqueVoterInsertOutcome>;
        pub fn verified_votes_insert_voted_value(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<VotedValueInsertOutcome>;
        pub fn verified_votes_insert_vote_atomic(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
        ) -> Result<AtomicVoteInsertOutcome>;
        pub fn verified_votes_apply_threshold_decision(
            self: &mut BridgeVerifiedVotes,
            vote: VerifiedVotePayload,
            total_weight: u64,
            two_t_plus_one_threshold: u64,
        ) -> Result<ThresholdDecisionOutcome>;
        pub fn verified_votes_vote_in_verified_map(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            step: u64,
            block_hash: &[u8; 32],
            vote_hash: &[u8; 32],
        ) -> bool;
        pub fn verified_votes_set_network_t_plus_one_step(
            self: &mut BridgeVerifiedVotes,
            period: u64,
            round: u64,
            step: u64,
        ) -> bool;
        pub fn verified_votes_get_network_t_plus_one_step(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
        ) -> NetworkTPlusOneStepLookup;
        pub fn verified_votes_determine_new_round(
            self: &BridgeVerifiedVotes,
            period: u64,
            current_round: u64,
        ) -> DetermineNewRoundOutcome;
        pub fn verified_votes_insert_two_t_plus_one_voted_block(
            self: &mut BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
            block_hash: &[u8; 32],
            step: u64,
        ) -> Result<TwoTPlusOneInsertOutcome>;
        pub fn verified_votes_get_two_t_plus_one_voted_block(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotedBlockLookup>;
        pub fn verified_votes_get_two_t_plus_one_voted_block_votes(
            self: &BridgeVerifiedVotes,
            period: u64,
            round: u64,
            kind: u8,
        ) -> Result<TwoTPlusOneVotesLookup>;
        pub fn verified_votes_cleanup_votes_by_period(
            self: &mut BridgeVerifiedVotes,
            pbft_period: u64,
        );
        pub fn verified_votes_snapshot_votes(
            self: &BridgeVerifiedVotes,
        ) -> Vec<VerifiedVotePayload>;
        pub fn verified_votes_snapshot_two_t_plus_one(
            self: &BridgeVerifiedVotes,
        ) -> Vec<TwoTPlusOneSnapshotEntry>;
        pub fn verified_votes_snapshot_round_markers(
            self: &BridgeVerifiedVotes,
        ) -> Vec<RoundMarkerSnapshot>;

        // Consensus sortition

        type BridgeSortitionParamsManager;

        pub fn create_sortition_params_manager(
            config: SortitionRuntimeConfig,
            params_changes: Vec<SortitionParamsChangePayload>,
        ) -> Result<Box<BridgeSortitionParamsManager>>;
        pub fn sortition_current_params(
            self: &BridgeSortitionParamsManager,
        ) -> SortitionRuntimeParams;
        pub fn sortition_params_for_period(
            self: &BridgeSortitionParamsManager,
            found: bool,
            change: SortitionParamsChangePayload,
        ) -> SortitionRuntimeParams;
        pub fn sortition_restore_finalized_period(
            self: &mut BridgeSortitionParamsManager,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
        ) -> Result<()>;
        pub fn sortition_record_finalized_period(
            self: &mut BridgeSortitionParamsManager,
            period: u64,
            has_pivot: bool,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
            non_empty_pbft_chain_size: u64,
        ) -> Result<SortitionParamsChangeResult>;
        pub fn sortition_average_dag_efficiency(self: &BridgeSortitionParamsManager)
            -> Result<u16>;
        pub fn sortition_params_changes(
            self: &BridgeSortitionParamsManager,
        ) -> Vec<SortitionParamsChangePayload>;
        pub fn sortition_calculate_dag_efficiency(
            self: &BridgeSortitionParamsManager,
            unique_transactions: u64,
            total_dag_transaction_refs: u64,
        ) -> SortitionEfficiencyResult;

        // Storage

        type BridgeStorage;

        pub fn create_storage(path: &str) -> Result<Box<BridgeStorage>>;
        pub fn create_write_batch(self: &BridgeStorage) -> Result<u64>;
        pub fn batch_put(
            self: &BridgeStorage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
            value: Vec<u8>,
        ) -> Result<()>;
        pub fn batch_delete(
            self: &BridgeStorage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
        ) -> Result<()>;
        pub fn commit_write_batch(self: &BridgeStorage, batch_id: u64, sync: bool) -> Result<()>;
        pub fn drop_write_batch(self: &BridgeStorage, batch_id: u64) -> Result<()>;

        pub fn dag_block_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_dag_block_period(self: &BridgeStorage, hash: &[u8; 32]) -> Result<BlockPeriod>;
        pub fn get_dag_block_period_lookup(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn get_last_blocks_level(self: &BridgeStorage) -> Result<u64>;
        pub fn get_blocks_by_level(self: &BridgeStorage, level: u64) -> Result<Vec<u8>>;
        pub fn get_dag_blocks_at_level(
            self: &BridgeStorage,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<BlockRlp>>;
        pub fn get_nonfinalized_dag_blocks(self: &BridgeStorage) -> Result<Vec<LevelBlocks>>;
        pub fn get_proposal_period_for_dag_level(
            self: &BridgeStorage,
            level: u64,
        ) -> Result<PeriodLookup>;
        pub fn save_dag_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn update_dag_block_counter(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
        ) -> Result<()>;
        pub fn remove_dag_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_proposal_period_dag_levels_map(
            self: &BridgeStorage,
            level: u64,
            period: u64,
        ) -> Result<()>;
        pub fn save_dag_block_period(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
        ) -> Result<()>;

        pub fn get_period_data_raw(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_period_from_pbft_hash(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<PeriodLookup>;
        pub fn get_block_receipt(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_final_chain_meta_value(self: &BridgeStorage, key: u32) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_header(
            self: &BridgeStorage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_hash_by_number(
            self: &BridgeStorage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_number_by_hash(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_log_blooms_chunk(
            self: &BridgeStorage,
            chunk_id: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_receipt_by_trx_hash(
            self: &BridgeStorage,
            trx_hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn save_period_data(
            self: &BridgeStorage,
            period: u64,
            period_data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_block_period(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
        ) -> Result<()>;
        pub fn get_pillar_block(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_latest_pillar_block(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_own_pillar_block_vote(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_current_pillar_block_data(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn save_pillar_block(
            self: &BridgeStorage,
            period: u64,
            pillar_block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_own_pillar_block_vote(self: &BridgeStorage, vote_rlp: Vec<u8>) -> Result<()>;
        pub fn save_current_pillar_block_data(
            self: &BridgeStorage,
            data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_genesis_hash(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn set_genesis_hash(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn get_last_sortition_params(self: &BridgeStorage, count: u64)
            -> Result<Vec<BlockRlp>>;
        pub fn get_params_change_for_period(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_status_field(self: &BridgeStorage, field: u8) -> Result<u64>;
        pub fn save_status_field(self: &BridgeStorage, field: u8, value: u64) -> Result<()>;
        pub fn save_sortition_params_change(
            self: &BridgeStorage,
            period: u64,
            params_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_period_lambda(
            self: &BridgeStorage,
            period: u64,
            find_closest: bool,
        ) -> Result<PeriodLambda>;
        pub fn save_period_lambda(
            self: &BridgeStorage,
            period: u64,
            period_lambda: u32,
        ) -> Result<()>;
        pub fn get_rounds_count_dynamic_lambda(self: &BridgeStorage) -> Result<u32>;
        pub fn save_rounds_count_dynamic_lambda(
            self: &BridgeStorage,
            rounds_count: u32,
        ) -> Result<()>;
        pub fn get_blocks_rewards_stats(self: &BridgeStorage) -> Result<Vec<PeriodRlp>>;
        pub fn save_block_rewards_stats(
            self: &BridgeStorage,
            period: u64,
            stats_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn clear_block_rewards_stats(self: &BridgeStorage) -> Result<()>;

        pub fn pbft_block_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_pbft_mgr_field(self: &BridgeStorage, field: u8) -> Result<u32>;
        pub fn get_pbft_mgr_status(self: &BridgeStorage, field: u8) -> Result<bool>;
        pub fn get_cert_voted_block_in_round(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_proposed_pbft_blocks(self: &BridgeStorage) -> Result<Vec<BlockRlp>>;
        pub fn get_pbft_head(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_own_verified_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn get_all_two_t_plus_one_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn get_reward_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn save_cert_voted_block_in_round(
            self: &BridgeStorage,
            round: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_proposed_pbft_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_mgr_field(self: &BridgeStorage, field: u8, value: u32) -> Result<()>;
        pub fn save_pbft_mgr_status(self: &BridgeStorage, field: u8, value: bool) -> Result<()>;
        pub fn save_pbft_head(self: &BridgeStorage, hash: &[u8; 32], head: Vec<u8>) -> Result<()>;
        pub fn save_own_verified_vote(
            self: &BridgeStorage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn remove_cert_voted_block_in_round(self: &BridgeStorage) -> Result<()>;
        pub fn remove_proposed_pbft_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_own_verified_vote(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_extra_reward_vote(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn replace_two_t_plus_one_votes(
            self: &BridgeStorage,
            vote_type: u8,
            votes_bundle_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_extra_reward_vote(
            self: &BridgeStorage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;

        pub fn transaction_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn transaction_finalized(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_transaction_location(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction_by_period_position(
            self: &BridgeStorage,
            period: u64,
            position: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeStorage, period: u64) -> Result<u64>;
        pub fn get_system_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_all_nonfinalized_transactions(self: &BridgeStorage) -> Result<Vec<TxRlp>>;
        pub fn get_all_transaction_period(self: &BridgeStorage) -> Result<Vec<HashPeriod>>;
        pub fn get_period_system_transactions_hashes(
            self: &BridgeStorage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn save_transaction(
            self: &BridgeStorage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn remove_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_transaction_location(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
            is_system: bool,
        ) -> Result<()>;
        pub fn save_system_transaction(
            self: &BridgeStorage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_period_system_transactions_hashes(
            self: &BridgeStorage,
            period: u64,
            hashes_rlp: Vec<u8>,
        ) -> Result<()>;

        // FinalChain

        type BridgeFinalChain;

        pub fn create_final_chain(
            storage: &BridgeStorage,
            block_gas_limit: u64,
            genesis_timestamp: u64,
            genesis_accounts: Vec<GenesisAccount>,
            genesis_validators: Vec<GenesisValidator>,
            genesis_dpos_config: GenesisDposConfig,
        ) -> Result<Box<BridgeFinalChain>>;

        pub fn get_last_block_number(self: &BridgeFinalChain) -> Result<u64>;
        pub fn get_block_number(
            self: &BridgeFinalChain,
            hash: &[u8; 32],
        ) -> Result<FinalChainBlockNumberLookup>;
        pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
        pub fn get_block_header(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
        pub fn get_transaction_location(
            self: &BridgeFinalChain,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeFinalChain, period: u64) -> Result<u64>;
        pub fn get_account(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<AccountLookup>;
        pub fn get_dpos_eligible_vote_count(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<u64>;
        pub fn get_dpos_eligible_total_vote_count(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<u64>;
        pub fn get_dpos_is_eligible(
            self: &BridgeFinalChain,
            block_number: u64,
            address: &[u8; 20],
        ) -> Result<bool>;
        pub fn get_dag_dpos_authorization_facts(
            self: &BridgeFinalChain,
            block_number: u64,
            sender: &[u8; 20],
            vdf_sortition_max_vote_count: u64,
            use_total_vote_count_for_vdf_sortition: bool,
        ) -> Result<DagDposAuthorizationFacts>;
        pub fn get_dpos_validators_total_stakes(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<Vec<DposValidatorStake>>;
        pub fn get_dpos_validators_eligible_vote_counts(
            self: &BridgeFinalChain,
            block_number: u64,
        ) -> Result<Vec<DposValidatorVoteCount>>;
        pub fn get_vrf_key(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<Vec<u8>>;
        pub fn estimate_call_gas(self: &BridgeFinalChain, gas_limit: u64) -> Result<u64>;
        pub fn call(
            self: &BridgeFinalChain,
            request: FinalChainCall,
        ) -> Result<FinalChainCallOutcome>;
        pub fn finalize_block(
            self: &BridgeFinalChain,
            pbft_block_rlp: Vec<u8>,
            transactions: Vec<FinalizationTransaction>,
            finalized_dag_blocks: Vec<FinalizationDagBlock>,
        ) -> Result<FinalizationOutcome>;
        pub fn get_transaction_rlps(self: &BridgeFinalChain, period: u64) -> Result<Vec<TxRlp>>;
        pub fn get_transaction_receipt(
            self: &BridgeFinalChain,
            period: u64,
            position: u64,
        ) -> Result<Vec<u8>>;
    }
}
