//! CXX bridge for the Rust-owned public consensus query facade.
//!
//! The bridge exposes read-only DTOs for RPC, GraphQL, plugins, debug tools, and
//! CLI callers. It adapts the shared Rust storage owner into
//! `ConsensusQueryApi` calls without handing public layers manager pointers,
//! storage iterators, or mutable compatibility sidecars.

use crate::ffi::{rustaxa_ffi, BridgeConsensusQueryApi, BridgeStorage};

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

fn pbft_hashes_to_ffi(hashes: Vec<[u8; 32]>) -> Vec<rustaxa_ffi::PbftFinalizationHash> {
    hashes
        .into_iter()
        .map(|hash| rustaxa_ffi::PbftFinalizationHash { hash })
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

/// Creates a stateless public consensus query facade.
pub fn create_consensus_query_api(storage: &BridgeStorage) -> Box<BridgeConsensusQueryApi> {
    Box::new(BridgeConsensusQueryApi(
        rustaxa_consensus::ConsensusQueryApi::new(storage.0.clone()),
    ))
}

impl BridgeConsensusQueryApi {
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

    /// Returns storage-backed public chain statistics.
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_consensus::sortition::{SortitionParamsChange, THRESHOLD_UPPER_MIN_VALUE};
    use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlpOwned;
    use rustaxa_types::final_chain::StoredFinalChainBlockHeader;
    use rustaxa_types::pillar::{
        encode_optimized_pillar_votes_bundle_rlp, PillarBlock, PillarVote, ValidatorVoteCountChange,
    };

    #[allow(clippy::too_many_arguments)]
    fn seed_final_chain_conformance_lookup_rows(
        storage: &BridgeStorage,
        meta_key: u32,
        meta_value: Vec<u8>,
        block_number: u64,
        block_hash: H256,
        block_header_rlp: Vec<u8>,
        receipt_hash: H256,
        receipt_rlp: Vec<u8>,
        blooms_chunk: H256,
        blooms_rlp: Vec<u8>,
        receipt_period: u64,
        receipts_rlp: Vec<u8>,
    ) {
        storage
            .0
            .final_chain()
            .write_conformance_lookup_rows(
                meta_key,
                &meta_value,
                block_number,
                block_hash,
                &block_header_rlp,
                receipt_hash,
                &receipt_rlp,
                blooms_chunk,
                &blooms_rlp,
                receipt_period,
                &receipts_rlp,
            )
            .expect("final-chain conformance rows should seed");
    }

    fn period_data_rlp(pbft_block_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(pbft_block_rlp, 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn period_data_rlp_with_dag_bundle(pbft_block_rlp: &[u8]) -> Vec<u8> {
        let mut dag_bundle = RlpStream::new_list(3);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(0);

        let mut stream = RlpStream::new_list(5);
        stream.append_raw(pbft_block_rlp, 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&dag_bundle.out(), 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn canonical_pbft_vote_rlp(
        period: u64,
        round: u64,
        step: u64,
        block_hash: H256,
        proof: &[u8],
        signature: &[u8],
    ) -> Vec<u8> {
        let mut sortition = RlpStream::new_list(4);
        sortition.append(&period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&proof);
        let sortition_rlp = sortition.out().to_vec();

        let mut vote = RlpStream::new_list(3);
        vote.append(&block_hash);
        vote.append(&sortition_rlp);
        vote.append(&signature);
        vote.out().to_vec()
    }

    fn optimized_vote_rlp(proof: &[u8], signature: &[u8]) -> Vec<u8> {
        let mut vote = RlpStream::new_list(2);
        vote.append(&proof);
        vote.append(&signature);
        vote.out().to_vec()
    }

    fn period_data_with_cert_votes_rlp(
        block_hash: H256,
        certified_period: u64,
        round: u64,
        step: u64,
        optimized_vote_rlps: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut votes = RlpStream::new_list(optimized_vote_rlps.len());
        for vote_rlp in optimized_vote_rlps {
            votes.append_raw(vote_rlp, 1);
        }

        let mut votes_bundle = RlpStream::new_list(5);
        votes_bundle.append(&block_hash);
        votes_bundle.append(&certified_period);
        votes_bundle.append(&round);
        votes_bundle.append(&step);
        votes_bundle.append_raw(&votes.out(), 1);

        let mut stream = RlpStream::new_list(5);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&votes_bundle.out(), 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn period_data_with_pillar_votes_rlp(votes_bundle_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(votes_bundle_rlp, 1);
        stream.out().to_vec()
    }

    fn period_data_with_transactions_rlp(transaction_rlps: &[Vec<u8>]) -> Vec<u8> {
        let mut transactions = RlpStream::new_list(transaction_rlps.len());
        for transaction_rlp in transaction_rlps {
            transactions.append_raw(transaction_rlp, 1);
        }

        let mut stream = RlpStream::new_list(5);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&transactions.out(), 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn receipt_list_rlp(receipt_rlps: &[Vec<u8>]) -> Vec<u8> {
        let mut receipts = RlpStream::new_list(receipt_rlps.len());
        for receipt_rlp in receipt_rlps {
            receipts.append_raw(receipt_rlp, 1);
        }
        receipts.out().to_vec()
    }

    fn signature(value: u8) -> [u8; 65] {
        let mut signature = [value; 65];
        signature[64] = value & 1;
        signature
    }

    fn signed_pbft_block_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let mut extra_data = RlpStream::new_list(6);
        extra_data.append(&1u16);
        extra_data.append(&2u16);
        extra_data.append(&3u16);
        extra_data.append(&4u16);
        extra_data.append(&"rustaxa-test".to_string());
        extra_data.append(&H256::from_low_u64_be(55).as_bytes());
        let extra_data_bytes = extra_data.out().to_vec();

        let mut unsigned = RlpStream::new_list(8);
        unsigned.append(&H256::from_low_u64_be(10));
        unsigned.append(&H256::from_low_u64_be(11));
        unsigned.append(&H256::from_low_u64_be(12));
        unsigned.append(&H256::from_low_u64_be(13));
        unsigned.append(&7u64);
        unsigned.append(&99u64);
        unsigned.append_list(&[H256::from_low_u64_be(20), H256::from_low_u64_be(21)]);
        unsigned.append(&extra_data_bytes);
        let message_hash = keccak256(&unsigned.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed = RlpStream::new_list(9);
        signed.append(&H256::from_low_u64_be(10));
        signed.append(&H256::from_low_u64_be(11));
        signed.append(&H256::from_low_u64_be(12));
        signed.append(&H256::from_low_u64_be(13));
        signed.append(&7u64);
        signed.append(&99u64);
        signed.append_list(&[H256::from_low_u64_be(20), H256::from_low_u64_be(21)]);
        signed.append(&extra_data_bytes);
        signed.append(&signature_bytes);
        signed.out().to_vec()
    }

    fn dag_block_rlp() -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11; 80]);
        vdf.append(&vec![0x22, 0x23]);
        vdf.append(&vec![0x33, 0x34]);
        vdf.append(&7u16);
        let signing_key = SigningKey::from_slice(&[0x43; 32]).unwrap();
        let mut block = rustaxa_types::dag::DagBlock {
            pivot: H256::from_low_u64_be(1),
            level: 5,
            timestamp: 123,
            vdf: vdf.out().to_vec(),
            tips: vec![H256::from_low_u64_be(2)],
            transactions: vec![H256::from_low_u64_be(3), H256::from_low_u64_be(4)],
            signature: [0; 65],
            gas_estimation: 987,
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .unwrap();
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();

        let mut stream = RlpStream::new_list(8);
        stream.append(&block.pivot);
        stream.append(&block.level);
        stream.append(&block.timestamp);
        stream.append(&block.vdf);
        stream.append_list(&block.tips);
        stream.append_list(&block.transactions);
        stream.append(&block.signature.to_vec());
        stream.append(&block.gas_estimation);
        stream.out().to_vec()
    }

    fn signed_finalized_dag_bundle_rlp() -> (Vec<u8>, Vec<u8>) {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11; 80]);
        vdf.append(&vec![0x22, 0x23]);
        vdf.append(&vec![0x33, 0x34]);
        vdf.append(&7u16);
        let transactions = vec![H256::from_low_u64_be(3), H256::from_low_u64_be(4)];
        let signing_key = SigningKey::from_slice(&[0x42; 32]).unwrap();
        let mut block = rustaxa_types::dag::DagBlock {
            pivot: H256::from_low_u64_be(1),
            level: 5,
            timestamp: 123,
            vdf: vdf.out().to_vec(),
            tips: vec![H256::from_low_u64_be(2)],
            transactions: transactions.clone(),
            signature: [0; 65],
            gas_estimation: 987,
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .unwrap();
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();

        let mut compact_block = RlpStream::new_list(7);
        compact_block.append(&block.pivot);
        compact_block.append(&block.level);
        compact_block.append(&block.timestamp);
        compact_block.append(&block.vdf);
        compact_block.append_list(&block.tips);
        compact_block.append(&block.signature.to_vec());
        compact_block.append(&block.gas_estimation);

        let mut bundle = RlpStream::new_list(3);
        bundle.begin_list(transactions.len());
        for transaction in &transactions {
            bundle.append(transaction);
        }
        bundle.begin_list(1);
        bundle.begin_list(transactions.len());
        for idx in 0..transactions.len() {
            bundle.append(&idx);
        }
        bundle.begin_list(1);
        bundle.append_raw(&compact_block.out(), 1);

        let mut canonical_block = RlpStream::new_list(8);
        canonical_block.append(&block.pivot);
        canonical_block.append(&block.level);
        canonical_block.append(&block.timestamp);
        canonical_block.append(&block.vdf);
        canonical_block.append_list(&block.tips);
        canonical_block.append_list(&block.transactions);
        canonical_block.append(&block.signature.to_vec());
        canonical_block.append(&block.gas_estimation);

        (bundle.out().to_vec(), canonical_block.out().to_vec())
    }

    fn period_data_with_dag_bundle_rlp(dag_bundle_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(dag_bundle_rlp, 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn keccak256(data: &[u8]) -> H256 {
        let mut hasher = tiny_keccak::Keccak::v256();
        tiny_keccak::Hasher::update(&mut hasher, data);
        let mut out = [0u8; 32];
        tiny_keccak::Hasher::finalize(hasher, &mut out);
        H256::from(out)
    }

    fn stored_header_rlp() -> Vec<u8> {
        StoredBlockHeaderRlpOwned::from(&StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(10),
            state_root: H256::from_low_u64_be(11),
            transactions_root: H256::from_low_u64_be(12),
            receipts_root: H256::from_low_u64_be(13),
            log_bloom: [0xBB; 256].into(),
            gas_used: 99.into(),
            total_reward: rustaxa_types::DposTokenAmount::from(U256::from(100u64)),
        })
        .into_vec()
    }

    #[test]
    fn bridge_consensus_query_api_reads_public_block_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block_hash = H256::from_low_u64_be(88);
        let pbft_block_rlp = vec![0xC2, 0x03, 0x04];
        let query_bloom = {
            let mut bloom = [0u8; 256];
            bloom[255] = 0x80;
            bloom
        };
        let mut root_chunk = rustaxa_storage::zero_final_chain_log_bloom_chunk();
        root_chunk[0] = query_bloom.into();
        let mut leaf_chunk = rustaxa_storage::zero_final_chain_log_bloom_chunk();
        leaf_chunk[15] = query_bloom.into();

        storage
            .0
            .period()
            .write(15, &period_data_rlp(&pbft_block_rlp))
            .unwrap();
        seed_final_chain_conformance_lookup_rows(
            &storage,
            1,
            15u64.to_le_bytes().to_vec(),
            15,
            block_hash,
            stored_header_rlp(),
            H256::from_low_u64_be(99),
            vec![],
            rustaxa_storage::final_chain_log_bloom_chunk_id(1, 0).unwrap(),
            rustaxa_storage::encode_final_chain_log_bloom_chunk(&root_chunk),
            15,
            vec![],
        );
        seed_final_chain_conformance_lookup_rows(
            &storage,
            1,
            15u64.to_le_bytes().to_vec(),
            15,
            block_hash,
            stored_header_rlp(),
            H256::from_low_u64_be(99),
            vec![],
            rustaxa_storage::final_chain_log_bloom_chunk_id(0, 0).unwrap(),
            rustaxa_storage::encode_final_chain_log_bloom_chunk(&leaf_chunk),
            15,
            vec![],
        );

        let view = api.consensus_query_final_chain_block_by_number(15).unwrap();
        assert!(view.found);
        assert_eq!(view.number, 15);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.parent_hash, H256::from_low_u64_be(10).0);
        assert_eq!(view.state_root, H256::from_low_u64_be(11).0);
        assert_eq!(view.gas_used, 99);
        assert_eq!(view.total_reward, U256::from(100u64).to_big_endian());
        assert_eq!(view.log_bloom.len(), 256);
        assert_eq!(view.log_bloom, vec![0xBB; 256]);
        assert!(view.has_pbft_hash);

        let lookup = api.consensus_query_pbft_block_hash_by_period(15).unwrap();
        assert!(lookup.found);
        assert_eq!(lookup.hash, view.pbft_block_hash);
        let number_lookup = api
            .consensus_query_final_chain_block_number_by_hash(&block_hash.0)
            .unwrap();
        assert!(number_lookup.found);
        assert_eq!(number_lookup.value, 15);
        assert!(
            !api.consensus_query_final_chain_block_number_by_hash(&[0x99; 32])
                .unwrap()
                .found
        );
        assert_eq!(
            api.consensus_query_final_chain_last_block_number().unwrap(),
            15
        );
        storage.0.metadata().write_period_lambda(15, 1234).unwrap();
        let sortition_change = SortitionParamsChange {
            period: 12,
            interval_efficiency: 4_200,
            threshold_upper: 1_234,
        };
        storage
            .0
            .metadata()
            .write_sortition_params_change(12, &sortition_change.to_rlp_bytes())
            .unwrap();
        let status_dag_block_rlp = dag_block_rlp();
        let status_dag_block_hash = keccak256(&status_dag_block_rlp);
        storage
            .0
            .dag()
            .write(
                H256::from(status_dag_block_hash.0),
                5,
                1,
                &status_dag_block_rlp,
            )
            .unwrap();
        storage
            .0
            .dag()
            .write_proposal_period_at_level(5, 12)
            .unwrap();
        storage
            .0
            .metadata()
            .write_status_field(rustaxa_storage::StatusField::ExecutedBlkCount as u8, 21)
            .unwrap();
        storage
            .0
            .metadata()
            .write_status_field(rustaxa_storage::StatusField::ExecutedTrxCount as u8, 34)
            .unwrap();
        storage
            .0
            .metadata()
            .write_status_field(rustaxa_storage::StatusField::DagBlkCount as u8, 55)
            .unwrap();
        storage
            .0
            .metadata()
            .write_status_field(rustaxa_storage::StatusField::TrxCount as u8, 89)
            .unwrap();
        let period_lambda = api.consensus_query_period_lambda_by_period(15).unwrap();
        assert!(period_lambda.found);
        assert_eq!(period_lambda.value, 1234);
        assert!(
            !api.consensus_query_period_lambda_by_period(16)
                .unwrap()
                .found
        );
        let proposal_period = api
            .consensus_query_proposal_period_for_dag_level(5)
            .unwrap();
        assert!(proposal_period.found);
        assert_eq!(proposal_period.value, 12);
        assert!(
            !api.consensus_query_proposal_period_for_dag_level(6)
                .unwrap()
                .found
        );
        let exact_sortition_change_view = api
            .consensus_query_sortition_params_change_by_period(12)
            .unwrap();
        assert!(exact_sortition_change_view.found);
        assert_eq!(exact_sortition_change_view.period, 12);
        assert_eq!(exact_sortition_change_view.interval_efficiency, 4_200);
        assert_eq!(exact_sortition_change_view.threshold_upper, 1_234);
        assert_eq!(
            exact_sortition_change_view.threshold_upper_min,
            THRESHOLD_UPPER_MIN_VALUE
        );
        let sortition_change_view = api
            .consensus_query_sortition_params_change_by_period(15)
            .unwrap();
        assert!(sortition_change_view.found);
        assert_eq!(sortition_change_view.period, 12);
        assert_eq!(sortition_change_view.interval_efficiency, 4_200);
        assert_eq!(sortition_change_view.threshold_upper, 1_234);
        assert_eq!(
            sortition_change_view.threshold_upper_min,
            THRESHOLD_UPPER_MIN_VALUE
        );
        assert!(
            !api.consensus_query_sortition_params_change_by_period(11)
                .unwrap()
                .found
        );
        storage
            .0
            .metadata()
            .write_sortition_params_change(16, &[0xC1])
            .unwrap();
        assert!(api
            .consensus_query_sortition_params_change_by_period(16)
            .is_err());
        let chain_stats = api.consensus_query_chain_stats().unwrap();
        assert_eq!(chain_stats.pbft_period, 15);
        assert_eq!(chain_stats.dag_blocks_count, 55);
        assert_eq!(chain_stats.transactions_count, 89);
        assert_eq!(chain_stats.dag_blocks_executed, 21);
        assert_eq!(chain_stats.transactions_executed, 34);
        let status = api.consensus_query_status().unwrap();
        assert_eq!(status.final_block_number, 15);
        assert_eq!(status.latest_dag_level, 5);
        assert!(status.latest_dag_period_found);
        assert_eq!(status.latest_dag_period, 12);
        assert_eq!(
            api.consensus_query_final_chain_blocks_with_bloom(&query_bloom, 1, 15)
                .unwrap(),
            vec![15]
        );
        assert!(api
            .consensus_query_final_chain_blocks_with_bloom(&[0x11; 256], 1, 15)
            .unwrap()
            .is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_pbft_schedule_block_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_schedule_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key);

        storage
            .0
            .period()
            .write(7, &period_data_rlp_with_dag_bundle(&pbft_block))
            .unwrap();

        let view = api
            .consensus_query_pbft_schedule_block_by_period(7)
            .unwrap();
        assert!(view.found);
        assert_eq!(view.prev_block_hash, H256::from_low_u64_be(10).0);
        assert_eq!(view.dag_block_hash_as_pivot, H256::from_low_u64_be(11).0);
        assert_eq!(view.period, 7);
        assert_eq!(view.timestamp, 99);
        assert_eq!(view.block_hash, keccak256(&pbft_block).0);
        assert_eq!(view.signature.len(), 65);
        assert_eq!(view.reward_votes.len(), 2);
        assert_eq!(view.reward_votes[0].hash, H256::from_low_u64_be(20).0);
        assert!(view.has_extra_data);
        assert_eq!(view.extra_data.major_version, 1);
        assert_eq!(view.extra_data.node_implementation, "rustaxa-test");
        assert!(view.dag_blocks_order.is_empty());

        assert!(
            !api.consensus_query_pbft_schedule_block_by_period(8)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_pbft_node_version_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_node_version_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key);

        storage
            .0
            .period()
            .write(7, &period_data_rlp(&pbft_block))
            .unwrap();

        let view = api.consensus_query_pbft_node_version_by_period(7).unwrap();
        assert!(view.found);
        assert_eq!(
            view.beneficiary,
            rustaxa_types::PbftBlockMetadata::try_from(
                rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(&pbft_block)
            )
            .unwrap()
            .author
            .0
        );
        assert_eq!(view.major_version, 1);
        assert_eq!(view.minor_version, 2);
        assert_eq!(view.patch_version, 3);

        assert!(
            !api.consensus_query_pbft_node_version_by_period(8)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_pbft_cert_vote_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_cert_votes_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block_hash = H256::from_low_u64_be(42);
        let proof = vec![0x77; 80];
        let signature = vec![0x11; 65];
        let optimized_vote = optimized_vote_rlp(&proof, &signature);
        let expected_vote = canonical_pbft_vote_rlp(12, 3, 3, block_hash, &proof, &signature);

        storage
            .0
            .period()
            .write(
                13,
                &period_data_with_cert_votes_rlp(
                    block_hash,
                    12,
                    3,
                    3,
                    std::slice::from_ref(&optimized_vote),
                ),
            )
            .unwrap();

        let view = api
            .consensus_query_pbft_previous_block_cert_votes_by_period(13)
            .unwrap();
        assert!(view.found);
        assert_eq!(view.period, 13);
        assert_eq!(view.certified_period, 12);
        assert_eq!(view.round, 3);
        assert_eq!(view.step, 3);
        assert_eq!(view.block_hash, block_hash.0);
        assert_eq!(view.votes.len(), 1);
        assert_eq!(view.votes[0].vote_rlp, expected_vote);
        assert!(
            !api.consensus_query_pbft_previous_block_cert_votes_by_period(14)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_pillar_block_data_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_pillar_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block = PillarBlock {
            period: 10,
            state_root: H256::from_low_u64_be(0x10),
            previous_pillar_block_hash: H256::from_low_u64_be(0x11),
            bridge_root: H256::from_low_u64_be(0x12),
            epoch: 13,
            validator_vote_count_changes: vec![ValidatorVoteCountChange {
                address: H160::from([0x14; 20]),
                vote_count_change: -7,
            }],
        };
        let vote = PillarVote {
            period: 11,
            block_hash: block.hash(),
            signature: signature(0x21),
        };
        let votes_bundle_rlp =
            encode_optimized_pillar_votes_bundle_rlp(std::slice::from_ref(&vote)).unwrap();

        storage
            .pillar_chain_storage_apply_finalized_block(10, block.encode_rlp())
            .unwrap();
        storage
            .0
            .period()
            .write(11, &period_data_with_pillar_votes_rlp(&votes_bundle_rlp))
            .unwrap();

        let view = api.consensus_query_pillar_block_data_by_period(10).unwrap();
        assert!(view.found);
        assert_eq!(view.pbft_period, 10);
        assert_eq!(view.state_root, H256::from_low_u64_be(0x10).0);
        assert_eq!(
            view.previous_pillar_block_hash,
            H256::from_low_u64_be(0x11).0
        );
        assert_eq!(view.bridge_root, H256::from_low_u64_be(0x12).0);
        assert_eq!(view.epoch, 13);
        assert_eq!(view.block_hash, <[u8; 32]>::from(block.hash()));
        assert_eq!(view.validator_vote_count_changes.len(), 1);
        assert_eq!(
            view.validator_vote_count_changes[0].address,
            H160::from([0x14; 20]).0
        );
        assert_eq!(view.validator_vote_count_changes[0].vote_count_change, -7);
        assert_eq!(view.signatures.len(), 1);
        let (r, vs) = vote.compact_signature_words();
        assert_eq!(view.signatures[0].r, r);
        assert_eq!(view.signatures[0].vs, vs);

        assert!(
            !api.consensus_query_pillar_block_data_by_period(12)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_dag_block_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_dag_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block_rlp = dag_block_rlp();
        let block_hash = keccak256(&block_rlp);

        storage
            .0
            .dag()
            .write(H256::from(block_hash.0), 5, 1, &block_rlp)
            .unwrap();
        storage
            .0
            .dag()
            .write_period(H256::from(block_hash.0), 9, 2)
            .unwrap();

        let view = api
            .consensus_query_dag_block_by_hash(&block_hash.0)
            .unwrap();
        assert!(view.found);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.pivot, H256::from_low_u64_be(1).0);
        assert_eq!(view.level, 5);
        assert_eq!(view.transactions.len(), 2);
        assert_eq!(view.block_rlp, block_rlp);
        assert!(view.finalized_period_found);
        assert_eq!(view.finalized_period, 9);
        assert_eq!(view.vdf_proof, vec![0x11; 80]);
        assert_eq!(view.vdf_sol1, vec![0x22, 0x23]);
        assert_eq!(view.vdf_sol2, vec![0x33, 0x34]);
        assert_eq!(view.vdf_difficulty, 7);

        let level_views = api.consensus_query_dag_blocks_by_level(5, 1).unwrap();
        assert_eq!(level_views.len(), 1);
        assert_eq!(level_views[0].hash, block_hash.0);
        assert_eq!(level_views[0].block_rlp, block_rlp);

        let (dag_bundle, canonical_block) = signed_finalized_dag_bundle_rlp();
        storage
            .0
            .period()
            .write(7, &period_data_with_dag_bundle_rlp(&dag_bundle))
            .unwrap();
        let finalized_views = api
            .consensus_query_finalized_dag_blocks_by_period(7)
            .unwrap();
        assert_eq!(finalized_views.len(), 1);
        assert_eq!(finalized_views[0].hash, keccak256(&canonical_block).0);
        assert_eq!(finalized_views[0].block_rlp, canonical_block);
        assert_eq!(finalized_views[0].finalized_period, 7);
        assert_eq!(finalized_views[0].finalized_position, 0);
        assert_eq!(finalized_views[0].transactions.len(), 2);
        assert_eq!(finalized_views[0].vdf_difficulty, 7);
        assert!(api
            .consensus_query_finalized_dag_blocks_by_period(8)
            .unwrap()
            .is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_transaction_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_transaction_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let pending_hash = H256::from_low_u64_be(1);
        let finalized_hash = H256::from_low_u64_be(2);
        let missing_hash = H256::from_low_u64_be(3);
        let system_hash = H256::from_low_u64_be(4);

        storage
            .0
            .transaction()
            .write(pending_hash, &[0x11])
            .unwrap();
        storage
            .0
            .transaction()
            .write_location(finalized_hash, 8, 0, false)
            .unwrap();
        storage
            .0
            .period()
            .write(8, &period_data_with_transactions_rlp(&[vec![0x22]]))
            .unwrap();
        storage
            .0
            .transaction()
            .write_location(system_hash, 9, 0, true)
            .unwrap();
        storage
            .0
            .transaction()
            .write_system(system_hash, &[0x44])
            .unwrap();

        let pending = api
            .consensus_query_transaction_by_hash(&pending_hash.0)
            .unwrap();
        assert!(pending.found);
        assert_eq!(pending.hash, pending_hash.0);
        assert_eq!(
            pending.source,
            rustaxa_consensus::STORED_TRANSACTION_SOURCE_PENDING
        );
        assert_eq!(pending.transaction_rlp, vec![0x11]);

        let finalized = api
            .consensus_query_transaction_by_hash(&finalized_hash.0)
            .unwrap();
        assert!(finalized.found);
        assert_eq!(
            finalized.source,
            rustaxa_consensus::STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        );
        assert!(finalized.location_found);
        assert_eq!(finalized.block_number, 8);
        assert_eq!(finalized.transaction_index, 0);
        assert!(!finalized.is_system);
        assert!(!finalized.block_hash_found);
        assert_eq!(finalized.transaction_rlp, vec![0x22]);

        let system = api
            .consensus_query_transaction_by_hash(&system_hash.0)
            .unwrap();
        assert!(system.found);
        assert_eq!(
            system.source,
            rustaxa_consensus::STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM
        );
        assert!(system.location_found);
        assert_eq!(system.block_number, 9);
        assert_eq!(system.transaction_index, 0);
        assert!(system.is_system);
        assert_eq!(system.transaction_rlp, vec![0x44]);

        let missing = api
            .consensus_query_transaction_by_hash(&missing_hash.0)
            .unwrap();
        assert!(!missing.found);
        assert_eq!(
            missing.source,
            rustaxa_consensus::STORED_TRANSACTION_SOURCE_MISSING
        );
        assert!(missing.transaction_rlp.is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_indexed_transaction_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_indexed_transaction_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block_hash = H256::from_low_u64_be(0x24);
        let first_rlp = vec![0x22];
        let second_rlp = vec![0x33];

        storage
            .0
            .period()
            .write(
                12,
                &period_data_with_transactions_rlp(&[first_rlp.clone(), second_rlp.clone()]),
            )
            .unwrap();
        seed_final_chain_conformance_lookup_rows(
            &storage,
            0,
            b"meta".to_vec(),
            12,
            block_hash,
            vec![0xC0],
            H256::zero(),
            vec![0xC0],
            H256::zero(),
            vec![0xC0],
            12,
            receipt_list_rlp(&[]),
        );

        let by_number = api
            .consensus_query_transaction_by_block_number_and_index(12, 1)
            .unwrap();
        assert!(by_number.found);
        assert_eq!(by_number.hash, keccak256(&second_rlp).0);
        assert_eq!(
            by_number.source,
            rustaxa_consensus::STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        );
        assert!(by_number.location_found);
        assert_eq!(by_number.block_number, 12);
        assert_eq!(by_number.transaction_index, 1);
        assert!(by_number.block_hash_found);
        assert_eq!(by_number.block_hash, block_hash.0);
        assert_eq!(by_number.transaction_rlp, second_rlp);

        let by_hash = api
            .consensus_query_transaction_by_block_hash_and_index(&block_hash.0, 0)
            .unwrap();
        assert!(by_hash.found);
        assert_eq!(by_hash.hash, keccak256(&first_rlp).0);
        assert_eq!(by_hash.transaction_rlp, first_rlp);
        assert_eq!(
            api.consensus_query_transaction_count_by_block_number(12)
                .unwrap(),
            2
        );
        assert_eq!(
            api.consensus_query_transaction_count_by_block_hash(&block_hash.0)
                .unwrap(),
            2
        );
        assert_eq!(
            api.consensus_query_transaction_count_by_block_number(99)
                .unwrap(),
            0
        );
        assert_eq!(
            api.consensus_query_transaction_count_by_block_hash(&[0x99; 32])
                .unwrap(),
            0
        );
        assert!(
            !api.consensus_query_transaction_by_block_number_and_index(12, 2)
                .unwrap()
                .found
        );
        assert!(
            !api.consensus_query_transaction_by_block_hash_and_index(&[0x99; 32], 0)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_transaction_receipt_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_transaction_receipt_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let trx_hash = H256::from_low_u64_be(0x21);
        let block_hash = H256::from_low_u64_be(0x24);
        let trx_rlp = vec![0x31];
        let receipt_rlp = vec![0x41];

        storage
            .0
            .transaction()
            .write_location(trx_hash, 12, 0, false)
            .unwrap();
        storage
            .0
            .period()
            .write(
                12,
                &period_data_with_transactions_rlp(std::slice::from_ref(&trx_rlp)),
            )
            .unwrap();
        seed_final_chain_conformance_lookup_rows(
            &storage,
            0,
            b"meta".to_vec(),
            12,
            block_hash,
            vec![0xC0],
            trx_hash,
            receipt_rlp.clone(),
            H256::zero(),
            vec![0xC0],
            12,
            receipt_list_rlp(std::slice::from_ref(&receipt_rlp)),
        );

        let view = api
            .consensus_query_transaction_receipt_by_hash(&trx_hash.0)
            .unwrap();
        assert!(view.found);
        assert_eq!(view.transaction_hash, trx_hash.0);
        assert_eq!(
            view.transaction_source,
            rustaxa_consensus::STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        );
        assert_eq!(view.transaction_rlp, trx_rlp);
        assert_eq!(view.receipt_rlp, receipt_rlp);
        assert_eq!(view.block_number, 12);
        assert_eq!(view.transaction_index, 0);
        assert!(!view.is_system);
        assert!(view.block_hash_found);
        assert_eq!(view.block_hash, block_hash.0);

        let block_receipts = api
            .consensus_query_transaction_receipts_by_block_number(12)
            .unwrap();
        assert_eq!(block_receipts.len(), 1);
        assert_eq!(block_receipts[0].transaction_hash, keccak256(&trx_rlp).0);
        assert_eq!(block_receipts[0].transaction_rlp, trx_rlp);
        assert_eq!(block_receipts[0].receipt_rlp, receipt_rlp);
        assert_eq!(block_receipts[0].block_number, 12);
        assert_eq!(block_receipts[0].transaction_index, 0);
        assert_eq!(block_receipts[0].block_hash, block_hash.0);
        assert!(api
            .consensus_query_transaction_receipts_by_block_number(99)
            .unwrap()
            .is_empty());
        assert!(
            !api.consensus_query_transaction_receipt_by_hash(&[0x99; 32])
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
