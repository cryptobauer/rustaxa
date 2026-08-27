//! CXX bridge for the Rust-owned public consensus query facade.
//!
//! The bridge exposes read-only DTOs for RPC, GraphQL, plugins, debug tools, and
//! CLI callers. It adapts the shared Rust storage owner into
//! `ConsensusQueryApi` calls without handing public layers manager pointers,
//! storage iterators, or mutable compatibility sidecars.

use crate::ffi::{rustaxa_ffi, BridgeApp, BridgeConsensusQueryApi};

fn query_hash_lookup_to_ffi(lookup: rustaxa_consensus::QueryHashLookup) -> rustaxa_ffi::HashLookup {
    rustaxa_ffi::HashLookup {
        found: lookup.found,
        hash: lookup.hash,
    }
}

fn query_number_lookup_to_ffi(
    lookup: rustaxa_consensus::QueryNumberLookup,
) -> rustaxa_ffi::FinalChainBlockNumberLookup {
    rustaxa_ffi::FinalChainBlockNumberLookup {
        found: lookup.found,
        value: lookup.value,
    }
}

fn period_lambda_to_ffi(lambda: rustaxa_consensus::QueryPeriodLambda) -> rustaxa_ffi::PeriodLambda {
    rustaxa_ffi::PeriodLambda {
        found: lambda.found,
        value: lambda.value,
    }
}

fn chain_stats_view_to_ffi(view: rustaxa_consensus::ChainStatsView) -> rustaxa_ffi::ChainStatsView {
    rustaxa_ffi::ChainStatsView {
        pbft_period: view.pbft_period,
        non_empty_pbft_periods: view.non_empty_pbft_periods,
        dag_blocks_count: view.dag_blocks_count,
        transactions_count: view.transactions_count,
        dag_blocks_executed: view.dag_blocks_executed,
        transactions_executed: view.transactions_executed,
    }
}

fn consensus_status_view_to_ffi(
    view: rustaxa_consensus::ConsensusStatusView,
) -> rustaxa_ffi::ConsensusStatusView {
    rustaxa_ffi::ConsensusStatusView {
        final_block_number: view.final_block_number,
        latest_dag_level: view.latest_dag_level,
        latest_dag_period_found: view.latest_dag_period_found,
        latest_dag_period: view.latest_dag_period,
    }
}

fn live_dag_status_view_to_ffi(
    status: rustaxa_consensus::DagRuntimeStatus,
) -> rustaxa_ffi::LiveDagStatusView {
    rustaxa_ffi::LiveDagStatusView {
        vertex_count: status.vertex_count,
        edge_count: status.edge_count,
        max_level: status.max_level,
        period: status.period,
        old_anchor: status.anchors.old.into(),
        current_anchor: status.anchors.current.into(),
        expiry_level: status.expiry_level,
        non_finalized_levels: status.non_finalized_levels,
        non_finalized_blocks: status.non_finalized_blocks,
    }
}

fn live_transaction_status_view_to_ffi(
    status: rustaxa_consensus::TransactionPoolStatus,
) -> rustaxa_ffi::LiveTransactionStatusView {
    rustaxa_ffi::LiveTransactionStatusView {
        transaction_count: status.transaction_count,
        queue_size: status.queue_size,
        non_finalized_size: status.non_finalized_size,
        gas_price_bid: status.gas_price_bid,
        transactions_dropped: status.transactions_dropped,
        non_proposable_over_limit: status.non_proposable_over_limit,
    }
}

fn sortition_params_change_view_to_ffi(
    view: rustaxa_consensus::SortitionParamsChangeView,
) -> rustaxa_ffi::SortitionParamsChangeView {
    rustaxa_ffi::SortitionParamsChangeView {
        found: view.found,
        period: view.period,
        interval_efficiency: view.interval_efficiency,
        threshold_upper: view.threshold_upper,
        threshold_upper_min: view.threshold_upper_min,
    }
}

fn final_chain_block_view_to_ffi(
    view: rustaxa_consensus::FinalChainBlockView,
) -> rustaxa_ffi::FinalChainBlockView {
    rustaxa_ffi::FinalChainBlockView {
        found: view.found,
        number: view.number,
        hash: view.hash,
        parent_hash: view.parent_hash,
        author: view.author,
        state_root: view.state_root,
        transactions_root: view.transactions_root,
        receipts_root: view.receipts_root,
        log_bloom: view.log_bloom,
        gas_used: view.gas_used,
        total_reward: view.total_reward,
        stored_header_rlp: view.stored_header_rlp,
        has_pbft_hash: view.has_pbft_hash,
        pbft_block_hash: view.pbft_block_hash,
    }
}

fn dag_hashes_to_ffi(hashes: Vec<[u8; 32]>) -> Vec<rustaxa_ffi::DagHash> {
    hashes
        .into_iter()
        .map(|hash| rustaxa_ffi::DagHash { hash })
        .collect()
}

fn pbft_hashes_to_ffi(hashes: Vec<[u8; 32]>) -> Vec<rustaxa_ffi::DagHash> {
    hashes
        .into_iter()
        .map(|hash| rustaxa_ffi::DagHash { hash })
        .collect()
}

fn pbft_extra_data_view_to_ffi(
    view: rustaxa_consensus::PbftBlockExtraDataView,
) -> rustaxa_ffi::PbftBlockExtraDataView {
    rustaxa_ffi::PbftBlockExtraDataView {
        found: view.found,
        major_version: view.major_version,
        minor_version: view.minor_version,
        patch_version: view.patch_version,
        net_version: view.net_version,
        node_implementation: view.node_implementation,
        has_pillar_block_hash: view.has_pillar_block_hash,
        pillar_block_hash: view.pillar_block_hash,
    }
}

fn pbft_schedule_block_view_to_ffi(
    view: rustaxa_consensus::PbftScheduleBlockView,
) -> rustaxa_ffi::PbftScheduleBlockView {
    rustaxa_ffi::PbftScheduleBlockView {
        found: view.found,
        prev_block_hash: view.prev_block_hash,
        dag_block_hash_as_pivot: view.dag_block_hash_as_pivot,
        order_hash: view.order_hash,
        final_chain_hash: view.final_chain_hash,
        period: view.period,
        timestamp: view.timestamp,
        block_hash: view.block_hash,
        signature: view.signature,
        beneficiary: view.beneficiary,
        reward_votes: pbft_hashes_to_ffi(view.reward_votes),
        has_extra_data: view.has_extra_data,
        extra_data: pbft_extra_data_view_to_ffi(view.extra_data),
        dag_blocks_order: pbft_hashes_to_ffi(view.dag_blocks_order),
    }
}

fn pbft_node_version_view_to_ffi(
    view: rustaxa_consensus::PbftNodeVersionView,
) -> rustaxa_ffi::PbftNodeVersionView {
    rustaxa_ffi::PbftNodeVersionView {
        found: view.found,
        beneficiary: view.beneficiary,
        major_version: view.major_version,
        minor_version: view.minor_version,
        patch_version: view.patch_version,
    }
}

fn pbft_cert_vote_rlp_to_ffi(
    vote: rustaxa_consensus::PbftCertVoteRlp,
) -> rustaxa_ffi::PbftCertVoteRlp {
    rustaxa_ffi::PbftCertVoteRlp {
        vote_rlp: vote.vote_rlp,
    }
}

fn pbft_period_cert_votes_view_to_ffi(
    view: rustaxa_consensus::PbftPeriodCertVotesView,
) -> rustaxa_ffi::PbftPeriodCertVotesView {
    rustaxa_ffi::PbftPeriodCertVotesView {
        found: view.found,
        period: view.period,
        certified_period: view.certified_period,
        round: view.round,
        step: view.step,
        block_hash: view.block_hash,
        votes: view
            .votes
            .into_iter()
            .map(pbft_cert_vote_rlp_to_ffi)
            .collect(),
    }
}

fn pillar_vote_count_change_to_ffi(
    change: rustaxa_consensus::PillarBlockViewVoteCountChange,
) -> rustaxa_ffi::PillarValidatorVoteCountChange {
    rustaxa_ffi::PillarValidatorVoteCountChange {
        address: change.address,
        vote_count_change: change.vote_count_change,
    }
}

fn pillar_signature_to_ffi(
    signature: rustaxa_consensus::PillarBlockViewSignature,
) -> rustaxa_ffi::PillarBlockViewSignature {
    rustaxa_ffi::PillarBlockViewSignature {
        r: signature.r,
        vs: signature.vs,
    }
}

fn pillar_block_data_view_to_ffi(
    view: rustaxa_consensus::PillarBlockDataView,
) -> rustaxa_ffi::PillarBlockDataView {
    rustaxa_ffi::PillarBlockDataView {
        found: view.found,
        pbft_period: view.pbft_period,
        state_root: view.state_root,
        previous_pillar_block_hash: view.previous_pillar_block_hash,
        bridge_root: view.bridge_root,
        epoch: view.epoch,
        validator_vote_count_changes: view
            .validator_vote_count_changes
            .into_iter()
            .map(pillar_vote_count_change_to_ffi)
            .collect(),
        block_hash: view.block_hash,
        signatures: view
            .signatures
            .into_iter()
            .map(pillar_signature_to_ffi)
            .collect(),
    }
}

fn dag_block_view_to_ffi(view: rustaxa_consensus::DagBlockView) -> rustaxa_ffi::DagBlockPublicView {
    rustaxa_ffi::DagBlockPublicView {
        found: view.found,
        pivot: view.pivot,
        level: view.level,
        tips: dag_hashes_to_ffi(view.tips),
        transactions: dag_hashes_to_ffi(view.transactions),
        trx_estimations: view.trx_estimations,
        signature: view.signature,
        block_rlp: view.block_rlp,
        hash: view.hash,
        sender: view.sender,
        timestamp: view.timestamp,
        finalized_period_found: view.finalized_period_found,
        finalized_period: view.finalized_period,
        finalized_position: view.finalized_position,
        has_vdf: view.has_vdf,
        vdf_proof: view.vdf_proof,
        vdf_sol1: view.vdf_sol1,
        vdf_sol2: view.vdf_sol2,
        vdf_difficulty: view.vdf_difficulty,
    }
}

fn transaction_view_to_ffi(
    view: rustaxa_consensus::TransactionView,
) -> rustaxa_ffi::TransactionPublicView {
    rustaxa_ffi::TransactionPublicView {
        found: view.found,
        hash: view.hash,
        source: view.source,
        location_found: view.location_found,
        block_number: view.block_number,
        transaction_index: view.transaction_index,
        is_system: view.is_system,
        block_hash_found: view.block_hash_found,
        block_hash: view.block_hash,
        transaction_rlp: view.transaction_rlp,
    }
}

fn transaction_receipt_view_to_ffi(
    view: rustaxa_consensus::TransactionReceiptView,
) -> rustaxa_ffi::TransactionReceiptPublicView {
    rustaxa_ffi::TransactionReceiptPublicView {
        found: view.found,
        transaction_hash: view.transaction_hash,
        transaction_source: view.transaction_source,
        transaction_rlp: view.transaction_rlp,
        receipt_rlp: view.receipt_rlp,
        block_number: view.block_number,
        transaction_index: view.transaction_index,
        is_system: view.is_system,
        block_hash_found: view.block_hash_found,
        block_hash: view.block_hash,
    }
}

/// Creates a stateless public consensus query facade over application-owned storage.
pub fn create_consensus_query_api(runtime: &BridgeApp) -> Box<BridgeConsensusQueryApi> {
    Box::new(BridgeConsensusQueryApi(
        runtime.0.consensus_query_api_for_bridge(),
    ))
}

impl BridgeConsensusQueryApi {
    /// Returns compact live DAG graph and non-finalized pressure facts.
    pub fn consensus_query_live_dag_status(
        &self,
    ) -> Result<rustaxa_ffi::LiveDagStatusView, anyhow::Error> {
        Ok(live_dag_status_view_to_ffi(self.0.dag_live_status()?))
    }

    /// Returns compact live transaction queue and pressure facts.
    pub fn consensus_query_live_transaction_status(
        &self,
    ) -> Result<rustaxa_ffi::LiveTransactionStatusView, anyhow::Error> {
        Ok(live_transaction_status_view_to_ffi(
            self.0.transaction_pool_status()?,
        ))
    }

    /// Returns the live application-owned verified-vote count for public clients.
    pub fn consensus_query_verified_vote_count(&self) -> Result<u64, anyhow::Error> {
        self.0.verified_vote_count()
    }

    /// Resolves one public PBFT quorum through the application-owned PBFT and FinalChain siblings.
    ///
    /// `vote_type` uses the canonical legacy numeric vote-kind values. Invalid
    /// values fail at the boundary; valid requests retain the native planner's
    /// typed status and optional threshold fields.
    pub fn consensus_query_pbft_vote_threshold(
        &self,
        period: u64,
        vote_type: u8,
    ) -> Result<rustaxa_ffi::PbftTwoTPlusOneThresholdPlan, anyhow::Error> {
        let plan = self.0.pbft_vote_threshold(
            period,
            rustaxa_consensus::verified_votes::PbftVoteType::try_from(vote_type)?,
        )?;
        Ok(rustaxa_ffi::PbftTwoTPlusOneThresholdPlan {
            status: plan.status.as_u8(),
            error_code: plan.error_code.to_owned(),
            has_threshold: plan.has_threshold,
            threshold: plan.threshold,
        })
    }

    /// Returns durable finalized PBFT membership for transport readers.
    pub fn consensus_query_pbft_sync_block_exists(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<bool, anyhow::Error> {
        self.0.pbft_sync_block_exists(*block_hash)
    }

    /// Returns the canonical PBFT block hash for a finalized period.
    pub fn consensus_query_pbft_block_hash_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::HashLookup, anyhow::Error> {
        Ok(query_hash_lookup_to_ffi(
            self.0.pbft_block_hash_by_period(period)?,
        ))
    }

    /// Returns a stable FinalChain public block view by finalized block number.
    pub fn consensus_query_final_chain_block_by_number(
        &self,
        number: u64,
    ) -> Result<rustaxa_ffi::FinalChainBlockView, anyhow::Error> {
        Ok(final_chain_block_view_to_ffi(
            self.0.final_chain_block_by_number(number)?,
        ))
    }

    /// Returns the finalized FinalChain block number for a block hash.
    pub fn consensus_query_final_chain_block_number_by_hash(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::FinalChainBlockNumberLookup, anyhow::Error> {
        Ok(query_number_lookup_to_ffi(
            self.0.final_chain_block_number_by_hash(*block_hash)?,
        ))
    }

    /// Returns the latest finalized FinalChain block number.
    pub fn consensus_query_final_chain_last_block_number(&self) -> Result<u64, anyhow::Error> {
        self.0.final_chain_last_block_number()
    }

    /// Returns the exact persisted dynamic lambda for a finalized period.
    pub fn consensus_query_period_lambda_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PeriodLambda, anyhow::Error> {
        Ok(period_lambda_to_ffi(
            self.0.period_lambda_by_period(period)?,
        ))
    }

    /// Returns the finalized proposal period mapped to a DAG level.
    pub fn consensus_query_proposal_period_for_dag_level(
        &self,
        level: u64,
    ) -> Result<rustaxa_ffi::FinalChainBlockNumberLookup, anyhow::Error> {
        Ok(query_number_lookup_to_ffi(
            self.0.proposal_period_for_dag_level(level)?,
        ))
    }

    /// Returns live PBFT progress with persisted public chain statistics.
    pub fn consensus_query_chain_stats(
        &self,
    ) -> Result<rustaxa_ffi::ChainStatsView, anyhow::Error> {
        Ok(chain_stats_view_to_ffi(self.0.chain_stats()?))
    }

    /// Returns storage-backed finalized head and DAG index status facts.
    pub fn consensus_query_status(
        &self,
    ) -> Result<rustaxa_ffi::ConsensusStatusView, anyhow::Error> {
        Ok(consensus_status_view_to_ffi(self.0.consensus_status()?))
    }

    /// Returns the sortition params change active at or before a period.
    pub fn consensus_query_sortition_params_change_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::SortitionParamsChangeView, anyhow::Error> {
        Ok(sortition_params_change_view_to_ffi(
            self.0.sortition_params_change_by_period(period)?,
        ))
    }

    /// Returns finalized block numbers whose Rust FinalChain bloom index contains the query bloom.
    pub fn consensus_query_final_chain_blocks_with_bloom(
        &self,
        bloom: &[u8; 256],
        from: u64,
        to: u64,
    ) -> Result<Vec<u64>, anyhow::Error> {
        self.0.final_chain_blocks_with_bloom(*bloom, from, to)
    }

    /// Returns a stable PBFT schedule-block public view by finalized period.
    pub fn consensus_query_pbft_schedule_block_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PbftScheduleBlockView, anyhow::Error> {
        Ok(pbft_schedule_block_view_to_ffi(
            self.0.pbft_schedule_block_by_period(period)?,
        ))
    }

    /// Returns PBFT author and semantic-version facts by finalized period.
    pub fn consensus_query_pbft_node_version_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PbftNodeVersionView, anyhow::Error> {
        Ok(pbft_node_version_view_to_ffi(
            self.0.pbft_node_version_by_period(period)?,
        ))
    }

    /// Returns previous-block PBFT cert-vote bytes by finalized period.
    pub fn consensus_query_pbft_previous_block_cert_votes_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PbftPeriodCertVotesView, anyhow::Error> {
        Ok(pbft_period_cert_votes_view_to_ffi(
            self.0.pbft_previous_block_cert_votes_by_period(period)?,
        ))
    }

    /// Returns a stable pillar block-data public view by finalized pillar period.
    pub fn consensus_query_pillar_block_data_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PillarBlockDataView, anyhow::Error> {
        Ok(pillar_block_data_view_to_ffi(
            self.0.pillar_block_data_by_period(period)?,
        ))
    }

    /// Returns a stable DAG public block view by block hash.
    pub fn consensus_query_dag_block_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::DagBlockPublicView, anyhow::Error> {
        Ok(dag_block_view_to_ffi(self.0.dag_block_by_hash(*hash)?))
    }

    /// Returns stable DAG public block views for a contiguous level window.
    pub fn consensus_query_dag_blocks_by_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<rustaxa_ffi::DagBlockPublicView>, anyhow::Error> {
        Ok(self
            .0
            .dag_blocks_by_level(level, number_of_levels)?
            .into_iter()
            .map(dag_block_view_to_ffi)
            .collect())
    }

    /// Returns stable finalized DAG block views for one PBFT period.
    pub fn consensus_query_finalized_dag_blocks_by_period(
        &self,
        period: u64,
    ) -> Result<Vec<rustaxa_ffi::DagBlockPublicView>, anyhow::Error> {
        Ok(self
            .0
            .finalized_dag_blocks_by_period(period)?
            .into_iter()
            .map(dag_block_view_to_ffi)
            .collect())
    }

    /// Returns a stable public transaction payload view by transaction hash.
    pub fn consensus_query_transaction_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::TransactionPublicView, anyhow::Error> {
        Ok(transaction_view_to_ffi(self.0.transaction_by_hash(*hash)?))
    }

    /// Returns a stable public transaction payload view by finalized block number and index.
    pub fn consensus_query_transaction_by_block_number_and_index(
        &self,
        block_number: u64,
        transaction_index: u64,
    ) -> Result<rustaxa_ffi::TransactionPublicView, anyhow::Error> {
        Ok(transaction_view_to_ffi(
            self.0
                .transaction_by_block_number_and_index(block_number, transaction_index)?,
        ))
    }

    /// Returns a stable public transaction payload view by finalized block hash and index.
    pub fn consensus_query_transaction_by_block_hash_and_index(
        &self,
        block_hash: &[u8; 32],
        transaction_index: u64,
    ) -> Result<rustaxa_ffi::TransactionPublicView, anyhow::Error> {
        Ok(transaction_view_to_ffi(
            self.0
                .transaction_by_block_hash_and_index(*block_hash, transaction_index)?,
        ))
    }

    /// Returns the finalized transaction count for a public block-number query.
    pub fn consensus_query_transaction_count_by_block_number(
        &self,
        block_number: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0.transaction_count_by_block_number(block_number)
    }

    /// Returns the finalized transaction count for a public block-hash query.
    pub fn consensus_query_transaction_count_by_block_hash(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<u64, anyhow::Error> {
        self.0.transaction_count_by_block_hash(*block_hash)
    }

    /// Returns a stable public transaction receipt payload view by transaction hash.
    pub fn consensus_query_transaction_receipt_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::TransactionReceiptPublicView, anyhow::Error> {
        Ok(transaction_receipt_view_to_ffi(
            self.0.transaction_receipt_by_hash(*hash)?,
        ))
    }

    /// Returns stable public transaction receipt views for one finalized block number.
    pub fn consensus_query_transaction_receipts_by_block_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<rustaxa_ffi::TransactionReceiptPublicView>, anyhow::Error> {
        Ok(self
            .0
            .transaction_receipts_by_block_number(block_number)?
            .into_iter()
            .map(transaction_receipt_view_to_ffi)
            .collect())
    }
}
