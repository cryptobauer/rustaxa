//! Public query facade for Rust-owned consensus read models.
//!
//! The facade is the narrow read-only API that RPC, GraphQL, plugins, debug
//! tools, and CLI code should call instead of reaching into consensus managers,
//! mutable sidecars, or generic storage iterators. It owns only a cloned Rust
//! storage handle and mutates no state; callers receive stable DTOs plus
//! canonical bytes when compatibility materializers still need legacy encodings.

use anyhow::{Context, Result};
use ethereum_types::{H160, H256};
use rlp::Rlp;
use rustaxa_storage::Storage;
use rustaxa_types::PbftBlockMetadata;
use rustaxa_types::codec::rlp::dag::{DagBlockRlp, FinalizedDagBlockBundleRlp};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::dag::DagBlock;
use rustaxa_types::final_chain::StoredFinalChainBlockHeader;
use rustaxa_types::pillar::{PillarBlockData, RawPillarBlockData};
use rustaxa_vdf::vdf_sortition::decode_vdf_sortition_payload;
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;
const DAG_BLOCKS_POS_IN_PERIOD_DATA: usize = 2;
const PBFT_PREV_HASH_POS: usize = 0;
const PBFT_PIVOT_HASH_POS: usize = 1;
const PBFT_ORDER_HASH_POS: usize = 2;
const PBFT_FINAL_CHAIN_HASH_POS: usize = 3;
const PBFT_PERIOD_POS: usize = 4;
const PBFT_TIMESTAMP_POS: usize = 5;
const PBFT_REWARD_VOTES_POS: usize = 6;
const PBFT_EXTRA_DATA_POS: usize = 7;
const PBFT_SIGNATURE_WITH_EXTRA_POS: usize = 8;
const PBFT_SIGNATURE_WITHOUT_EXTRA_POS: usize = 7;
const PBFT_EXTRA_DATA_FIELDS: usize = 6;
const PBFT_EXTRA_DATA_MAX_SIZE: usize = 1024;
const PILLAR_VOTES_POS_IN_PERIOD_DATA: usize = 4;

pub use crate::transaction_storage::{
    STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR, STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM,
    STORED_TRANSACTION_SOURCE_MISSING, STORED_TRANSACTION_SOURCE_PENDING,
};
use crate::transaction_storage::{StoredTransactionLookupRequest, load_stored_transactions};

/// Hash lookup result returned by public query facade methods.
///
/// `found` is false when the requested durable row does not exist. When
/// `found` is true, `hash` contains the canonical 32-byte object hash in
/// Taraxa/Ethereum byte order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryHashLookup {
    pub found: bool,
    pub hash: [u8; 32],
}

/// Stable public view of a finalized FinalChain block.
///
/// The view combines Rust FinalChain lookup rows with the PBFT period-data hash
/// index used by public block formatters. It deliberately exposes plain values
/// and canonical stored-header bytes rather than manager pointers, storage
/// iterators, or mutable compatibility sidecars.
///
/// `found` is false when either the block header or block hash row is absent.
/// The remaining fields are defaulted in that case. For genesis, or for
/// temporary compatibility data that lacks period-data rows, `has_pbft_hash` is
/// false and `pbft_block_hash` is zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainBlockView {
    pub found: bool,
    pub number: u64,
    pub hash: [u8; 32],
    pub parent_hash: [u8; 32],
    pub author: [u8; 20],
    pub state_root: [u8; 32],
    pub transactions_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub log_bloom: Vec<u8>,
    pub gas_used: u64,
    pub total_reward: [u8; 32],
    pub stored_header_rlp: Vec<u8>,
    pub has_pbft_hash: bool,
    pub pbft_block_hash: [u8; 32],
}

/// Stable public view of one DAG block.
///
/// The view is loaded from Rust DAG storage and contains the base facts public
/// RPC/GraphQL formatters need for DAG block JSON without exposing a live DAG
/// manager or C++ block object. `finalized_period_found` distinguishes
/// non-finalized blocks from finalized blocks whose period/position index has
/// already been written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DagBlockView {
    pub found: bool,
    pub pivot: [u8; 32],
    pub level: u64,
    pub tips: Vec<[u8; 32]>,
    pub transactions: Vec<[u8; 32]>,
    pub trx_estimations: u64,
    pub signature: Vec<u8>,
    pub hash: [u8; 32],
    pub sender: [u8; 20],
    pub timestamp: u64,
    pub finalized_period_found: bool,
    pub finalized_period: u64,
    pub finalized_position: u32,
    pub has_vdf: bool,
    pub vdf_proof: Vec<u8>,
    pub vdf_sol1: Vec<u8>,
    pub vdf_sol2: Vec<u8>,
    pub vdf_difficulty: u16,
}

/// Stable public view of a transaction payload resolved by transaction hash.
///
/// The view carries canonical transaction RLP plus a source classification so
/// public C++ adapters can materialize regular or system transaction objects at
/// the formatting edge without calling `TransactionManager`. `found` is false
/// when the hash is absent from both pending and finalized transaction storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionView {
    pub found: bool,
    pub hash: [u8; 32],
    pub source: u8,
    pub location_found: bool,
    pub block_number: u64,
    pub transaction_index: u32,
    pub is_system: bool,
    pub block_hash_found: bool,
    pub block_hash: [u8; 32],
    pub transaction_rlp: Vec<u8>,
}

/// Stable public view of one transaction receipt resolved by transaction hash.
///
/// The view is built from Rust-owned transaction location, FinalChain block
/// hash, transaction payload, and receipt indexes. The receipt remains canonical
/// RLP so C++ public adapters can keep existing JSON formatting while no longer
/// reading `FinalChain` directly for single-receipt lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionReceiptView {
    pub found: bool,
    pub transaction_hash: [u8; 32],
    pub transaction_source: u8,
    pub transaction_rlp: Vec<u8>,
    pub receipt_rlp: Vec<u8>,
    pub block_number: u64,
    pub transaction_index: u32,
    pub is_system: bool,
    pub block_hash_found: bool,
    pub block_hash: [u8; 32],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TransactionLocationView {
    period: u64,
    position: u32,
    is_system: bool,
    block_hash: Option<[u8; 32]>,
}

/// Stable public view of optional PBFT block extra data.
///
/// `found` is false when the signed PBFT block has no compatible extra-data
/// payload. When true, version fields and optional pillar-block hash mirror the
/// legacy PBFT JSON shape without exposing a C++ `PbftBlock`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PbftBlockExtraDataView {
    pub found: bool,
    pub major_version: u16,
    pub minor_version: u16,
    pub patch_version: u16,
    pub net_version: u16,
    pub node_implementation: String,
    pub has_pillar_block_hash: bool,
    pub pillar_block_hash: [u8; 32],
}

/// Stable public view of a finalized PBFT schedule block.
///
/// The view is decoded from stored `PeriodData` and includes the PBFT block
/// facts and finalized DAG block order required by
/// `taraxa_getScheduleBlockByPeriod`. It does not materialize C++ PBFT/DAG
/// objects or expose storage iterators. `found` is false when no period data is
/// stored for the requested period.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PbftScheduleBlockView {
    pub found: bool,
    pub prev_block_hash: [u8; 32],
    pub dag_block_hash_as_pivot: [u8; 32],
    pub order_hash: [u8; 32],
    pub final_chain_hash: [u8; 32],
    pub period: u64,
    pub timestamp: u64,
    pub block_hash: [u8; 32],
    pub signature: Vec<u8>,
    pub beneficiary: [u8; 20],
    pub reward_votes: Vec<[u8; 32]>,
    pub has_extra_data: bool,
    pub extra_data: PbftBlockExtraDataView,
    pub dag_blocks_order: Vec<[u8; 32]>,
}

/// Stable public view of PBFT node-version facts.
///
/// The view is decoded from a finalized PBFT block embedded in `PeriodData`.
/// It carries only the author and semantic-version fields needed by
/// `taraxa_getNodeVersions`; the caller owns scan aggregation and JSON
/// formatting. `found` is false when no period data exists for the requested
/// period.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PbftNodeVersionView {
    pub found: bool,
    pub beneficiary: [u8; 20],
    pub major_version: u16,
    pub minor_version: u16,
    pub patch_version: u16,
}

/// Public/query view for one pillar validator vote-count delta.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PillarBlockViewVoteCountChange {
    pub address: [u8; 20],
    pub vote_count_change: i32,
}

/// Public/query view for one compact pillar-vote signature.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PillarBlockViewSignature {
    pub r: [u8; 32],
    pub vs: [u8; 32],
}

/// Stable public view of stored pillar block data.
///
/// The view combines the finalized pillar block stored for `pbft_period` with
/// the optimized pillar-vote bundle stored in the following period data row. It
/// carries only the fields required by `taraxa_getPillarBlockData` and avoids
/// exposing storage handles, raw period data, or C++ pillar objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PillarBlockDataView {
    pub found: bool,
    pub pbft_period: u64,
    pub state_root: [u8; 32],
    pub previous_pillar_block_hash: [u8; 32],
    pub bridge_root: [u8; 32],
    pub epoch: u64,
    pub validator_vote_count_changes: Vec<PillarBlockViewVoteCountChange>,
    pub block_hash: [u8; 32],
    pub signatures: Vec<PillarBlockViewSignature>,
}

/// Read-only public query facade over Rust consensus storage.
///
/// The API owns only a cloned Rust storage handle. It does not own RPC/GraphQL
/// formatting, transaction expansion, account/state reads, network sync
/// execution, or plugin lifecycle. Those callers may format returned DTOs into
/// legacy JSON or objects while this facade remains the single consensus read
/// boundary.
pub struct ConsensusQueryApi {
    storage: Arc<Storage>,
}

impl ConsensusQueryApi {
    /// Creates a public query facade over a shared Rust storage owner.
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    /// Returns the canonical PBFT block hash for a finalized period.
    ///
    /// The hash is derived from the signed PBFT block embedded at item 0 of the
    /// stored `PeriodData` RLP, matching the legacy public block hash contract.
    /// Missing period data returns `found == false`; malformed period data is an
    /// error so public formatters can preserve their existing invalid-params
    /// behavior.
    pub fn pbft_block_hash_by_period(&self, period: u64) -> Result<QueryHashLookup> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(QueryHashLookup::default());
        }
        let period_rlp = Rlp::new(&period_data);
        Ok(QueryHashLookup {
            found: true,
            hash: keccak256(
                period_rlp
                    .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)
                    .context("CONSENSUS_QUERY_PERIOD_DATA_PBFT_BLOCK")?
                    .as_raw(),
            )
            .into(),
        })
    }

    /// Returns a finalized PBFT schedule-block view by period.
    ///
    /// The query decodes the signed PBFT block and finalized DAG block bundle
    /// from Rust-owned period storage. Missing period data returns
    /// `found == false`; malformed PBFT or DAG bundle bytes are errors so
    /// public adapters can preserve their existing invalid-params behavior.
    pub fn pbft_schedule_block_by_period(&self, period: u64) -> Result<PbftScheduleBlockView> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(PbftScheduleBlockView::default());
        }
        pbft_schedule_block_view_from_period_data(&period_data)
    }

    /// Returns PBFT author and semantic-version facts by finalized period.
    ///
    /// The query decodes the signed PBFT block and extra-data payload from
    /// Rust-owned period storage. Missing period data returns `found == false`;
    /// malformed PBFT bytes, unrecoverable author, or missing/invalid
    /// extra-data is an error so public adapters keep their existing
    /// invalid-params behavior instead of silently omitting a validator.
    pub fn pbft_node_version_by_period(&self, period: u64) -> Result<PbftNodeVersionView> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(PbftNodeVersionView::default());
        }
        pbft_node_version_view_from_period_data(&period_data)
    }

    /// Returns a finalized FinalChain block view by block number.
    ///
    /// The query reads only Rust-owned FinalChain and period storage rows. It
    /// does not materialize C++ `BlockHeader`/`PbftBlock` objects and it does
    /// not expand transactions or receipts. Callers that need legacy encodings
    /// can use `stored_header_rlp` while migrating to the typed fields.
    pub fn final_chain_block_by_number(&self, number: u64) -> Result<FinalChainBlockView> {
        let Some(stored_header_rlp) = self.storage.final_chain().block_header_raw(number)? else {
            return Ok(FinalChainBlockView::default());
        };
        let Some(hash_bytes) = self.storage.final_chain().block_hash_by_number(number)? else {
            return Ok(FinalChainBlockView::default());
        };
        let block_hash = h256_bytes(&hash_bytes).context("CONSENSUS_QUERY_FINAL_CHAIN_HASH")?;
        let stored_header =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&stored_header_rlp))
                .context("CONSENSUS_QUERY_FINAL_CHAIN_HEADER")?;
        let pbft_hash = self.pbft_block_hash_by_period(number)?;

        Ok(FinalChainBlockView {
            found: true,
            number,
            hash: block_hash.into(),
            parent_hash: stored_header.parent_hash.into(),
            author: H160::zero().into(),
            state_root: stored_header.state_root.into(),
            transactions_root: stored_header.transactions_root.into(),
            receipts_root: stored_header.receipts_root.into(),
            log_bloom: stored_header.log_bloom,
            gas_used: stored_header.gas_used,
            total_reward: stored_header.total_reward.to_big_endian(),
            stored_header_rlp,
            has_pbft_hash: pbft_hash.found,
            pbft_block_hash: pbft_hash.hash,
        })
    }

    /// Returns stored pillar block data for a finalized pillar period.
    ///
    /// Missing pillar block bytes, missing following period data, or an empty
    /// pillar-vote bundle return `found == false`. Malformed pillar block or
    /// vote bytes are returned as errors so public adapters can preserve their
    /// existing invalid-params behavior.
    pub fn pillar_block_data_by_period(&self, period: u64) -> Result<PillarBlockDataView> {
        let Some(pillar_block_rlp) = self.storage.pillar().rlp(period)? else {
            return Ok(PillarBlockDataView::default());
        };
        let votes_period = period
            .checked_add(1)
            .context("CONSENSUS_QUERY_PILLAR_VOTES_PERIOD_OVERFLOW")?;
        let period_data = self
            .storage
            .period()
            .data_raw(votes_period)
            .context("CONSENSUS_QUERY_PILLAR_PERIOD_DATA")?;
        if period_data.is_empty() {
            return Ok(PillarBlockDataView::default());
        }

        let period_rlp = Rlp::new(&period_data);
        if period_rlp.item_count()? <= PILLAR_VOTES_POS_IN_PERIOD_DATA {
            return Ok(PillarBlockDataView::default());
        }
        let votes = period_rlp
            .at(PILLAR_VOTES_POS_IN_PERIOD_DATA)
            .context("CONSENSUS_QUERY_PILLAR_VOTES")?;
        if votes.item_count()? == 0 {
            return Ok(PillarBlockDataView::default());
        }

        let block_data_rlp = RawPillarBlockData {
            pillar_block_rlp,
            pillar_votes_bundle_rlp: votes.as_raw().to_vec(),
        }
        .encode_rlp()
        .context("CONSENSUS_QUERY_PILLAR_BLOCK_DATA_RLP")?;
        let block_data = PillarBlockData::decode_rlp(&block_data_rlp)
            .context("CONSENSUS_QUERY_PILLAR_BLOCK_DATA_DECODE")?;

        Ok(pillar_block_data_view(block_data))
    }

    /// Returns a public DAG block view by block hash.
    ///
    /// The query reads canonical DAG block bytes and finalized period metadata
    /// from Rust storage. It does not expand transaction hashes into
    /// transaction objects and it does not consult a live `DagManager` or
    /// `PbftManager`. Missing DAG block bytes return `found == false`;
    /// malformed block or VDF bytes are returned as errors for public adapters
    /// to map to their existing invalid-params behavior.
    pub fn dag_block_by_hash(&self, hash: [u8; 32]) -> Result<DagBlockView> {
        let requested_hash = H256::from(hash);
        let Some(block_rlp) = self.storage.dag().by_hash_rlp_optional(requested_hash)? else {
            return Ok(DagBlockView::default());
        };
        let block = DagBlock::try_from(DagBlockRlp::new(&block_rlp))
            .context("CONSENSUS_QUERY_DAG_BLOCK_DECODE")?;
        let finalized = self.storage.dag().period_optional(requested_hash)?;
        let sender = block
            .recover_sender()
            .context("CONSENSUS_QUERY_DAG_BLOCK_SENDER")?;
        let (has_vdf, vdf_proof, vdf_sol1, vdf_sol2, vdf_difficulty) = if block.level > 0 {
            let vdf = decode_vdf_sortition_payload(&block.vdf)
                .context("CONSENSUS_QUERY_DAG_BLOCK_VDF_DECODE")?;
            (
                true,
                vdf.vrf_proof.to_vec(),
                vdf.vdf_solution_proof,
                vdf.vdf_solution_output,
                vdf.difficulty,
            )
        } else {
            (false, Vec::new(), Vec::new(), Vec::new(), 0)
        };

        Ok(DagBlockView {
            found: true,
            pivot: block.pivot.into(),
            level: block.level,
            tips: block.tips.into_iter().map(Into::into).collect(),
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            trx_estimations: block.gas_estimation,
            signature: block.signature.to_vec(),
            hash: keccak256(&block_rlp).into(),
            sender: sender.into(),
            timestamp: block.timestamp,
            finalized_period_found: finalized.is_some(),
            finalized_period: finalized.map(|(period, _)| period).unwrap_or_default(),
            finalized_position: finalized.map(|(_, position)| position).unwrap_or_default(),
            has_vdf,
            vdf_proof,
            vdf_sol1,
            vdf_sol2,
            vdf_difficulty,
        })
    }

    /// Returns public DAG block views for a contiguous level window.
    ///
    /// The query reads level indexes and DAG block bytes from Rust storage and
    /// materializes the same stable DTO as [`Self::dag_block_by_hash`]. Missing
    /// block payloads referenced by a level index are skipped, matching the
    /// legacy storage query behavior during transitional repair windows.
    pub fn dag_blocks_by_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<DagBlockView>> {
        let hashes = self
            .storage
            .dag()
            .hashes_at_level_range(level, number_of_levels)?;
        let mut views = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let view = self.dag_block_by_hash(hash.into())?;
            if view.found {
                views.push(view);
            }
        }
        Ok(views)
    }

    /// Returns finalized DAG block views embedded in one PBFT period.
    ///
    /// The query reads Rust-owned period data, reconstructs canonical DAG block
    /// bytes from the compact finalized DAG bundle, and returns public DAG
    /// views in bundle order. Missing period data returns an empty vector,
    /// matching the legacy debug/GraphQL query behavior. Malformed period data
    /// or DAG bundle bytes are returned as errors for public adapters to map to
    /// their existing invalid-params behavior.
    pub fn finalized_dag_blocks_by_period(&self, period: u64) -> Result<Vec<DagBlockView>> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(Vec::new());
        }
        let period_rlp = Rlp::new(&period_data);
        finalized_dag_block_views(
            period_rlp
                .at(DAG_BLOCKS_POS_IN_PERIOD_DATA)
                .context("CONSENSUS_QUERY_PERIOD_DAG_BUNDLE")?
                .as_raw(),
            period,
        )
    }

    fn transaction_location_by_hash(&self, hash: H256) -> Result<Option<TransactionLocationView>> {
        let Some(location_rlp) = self.storage.transaction().location_rlp(hash)? else {
            return Ok(None);
        };
        let location = Rlp::new(&location_rlp);
        let period = location
            .val_at::<u64>(0)
            .context("CONSENSUS_QUERY_TRANSACTION_LOCATION_PERIOD")?;
        let position = location
            .val_at::<u32>(1)
            .context("CONSENSUS_QUERY_TRANSACTION_LOCATION_POSITION")?;
        let is_system = location
            .item_count()
            .context("CONSENSUS_QUERY_TRANSACTION_LOCATION_SHAPE")?
            == 3
            && location
                .val_at::<bool>(2)
                .context("CONSENSUS_QUERY_TRANSACTION_LOCATION_SYSTEM_FLAG")?;
        let block_hash = self
            .storage
            .final_chain()
            .block_hash_by_number(period)?
            .map(|bytes| h256_bytes(&bytes).map(Into::into))
            .transpose()
            .context("CONSENSUS_QUERY_TRANSACTION_BLOCK_HASH")?;
        Ok(Some(TransactionLocationView {
            period,
            position,
            is_system,
            block_hash,
        }))
    }

    fn receipt_by_period_position(&self, period: u64, position: u32) -> Result<Option<Vec<u8>>> {
        let receipts_rlp = self.storage.period().receipt(period)?;
        if receipts_rlp.is_empty() {
            return Ok(None);
        }
        let receipts = Rlp::new(&receipts_rlp);
        if position as usize >= receipts.item_count()? {
            return Ok(None);
        }
        Ok(Some(receipts.at(position as usize)?.as_raw().to_vec()))
    }

    /// Returns a public transaction view by transaction hash.
    ///
    /// The query uses the Rust-owned transaction storage lookup rules shared
    /// with `TransactionManager`: pending non-finalized payloads take
    /// precedence, finalized regular transactions are loaded from period data,
    /// and finalized system transactions are loaded from system-transaction
    /// storage. The method returns canonical bytes and a source code only; it
    /// does not materialize legacy C++ transaction objects or read receipts.
    pub fn transaction_by_hash(&self, hash: [u8; 32]) -> Result<TransactionView> {
        let requested_hash = H256::from(hash);
        let mut lookups = load_stored_transactions(
            &self.storage,
            vec![StoredTransactionLookupRequest {
                input_index: 0,
                hash: requested_hash,
            }],
        )
        .context("CONSENSUS_QUERY_TRANSACTION_LOOKUP")?;
        let Some(lookup) = lookups.pop() else {
            return Ok(TransactionView::default());
        };
        if lookup.input_index != 0 || lookup.hash != requested_hash {
            anyhow::bail!("CONSENSUS_QUERY_TRANSACTION_LOOKUP_MISMATCH");
        }
        let location = self.transaction_location_by_hash(requested_hash)?;
        let (
            location_found,
            block_number,
            transaction_index,
            is_system,
            block_hash_found,
            block_hash,
        ) = match location {
            Some(location) => (
                true,
                location.period,
                location.position,
                location.is_system,
                location.block_hash.is_some(),
                location.block_hash.unwrap_or_default(),
            ),
            None => (false, 0, 0, false, false, [0; 32]),
        };

        Ok(TransactionView {
            found: lookup.found,
            hash,
            source: lookup.source,
            location_found,
            block_number,
            transaction_index,
            is_system,
            block_hash_found,
            block_hash,
            transaction_rlp: lookup.tx_rlp,
        })
    }

    /// Returns a public transaction view by finalized block number and index.
    ///
    /// Unknown blocks and out-of-range indexes return `found == false`,
    /// matching ETH RPC null-result behavior. The query reads Rust-owned
    /// FinalChain block-hash and period-data rows, computes the canonical
    /// transaction hash from the stored transaction bytes, and returns the same
    /// location-aware DTO as hash lookup without materializing C++ objects.
    pub fn transaction_by_block_number_and_index(
        &self,
        block_number: u64,
        transaction_index: u64,
    ) -> Result<TransactionView> {
        let Some(block_hash_bytes) = self
            .storage
            .final_chain()
            .block_hash_by_number(block_number)?
        else {
            return Ok(TransactionView::default());
        };
        let block_hash = h256_bytes(&block_hash_bytes)
            .context("CONSENSUS_QUERY_TRANSACTION_INDEX_BLOCK_HASH")?;
        if transaction_index > u64::from(u32::MAX) {
            return Ok(TransactionView::default());
        }
        let transaction_index = transaction_index as u32;
        if u64::from(transaction_index) >= self.storage.transaction().count(block_number)? {
            return Ok(TransactionView::default());
        }
        let Some(transaction_rlp) = self
            .storage
            .transaction()
            .by_period_position_rlp(block_number, transaction_index)
            .context("CONSENSUS_QUERY_TRANSACTION_INDEX_PAYLOAD")?
        else {
            return Ok(TransactionView::default());
        };
        let hash: [u8; 32] = keccak256(&transaction_rlp).into();

        Ok(TransactionView {
            found: true,
            hash,
            source: STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR,
            location_found: true,
            block_number,
            transaction_index,
            is_system: false,
            block_hash_found: true,
            block_hash: block_hash.into(),
            transaction_rlp,
        })
    }

    /// Returns a public transaction view by finalized block hash and index.
    ///
    /// The block-hash index is resolved inside the query facade so ETH RPC
    /// callers do not need to ask `FinalChain` for block-number translation in
    /// Rust mode. Missing hash rows, inconsistent hash/number indexes, and
    /// out-of-range transaction indexes return `found == false`.
    pub fn transaction_by_block_hash_and_index(
        &self,
        block_hash: [u8; 32],
        transaction_index: u64,
    ) -> Result<TransactionView> {
        let Some(block_number_bytes) = self
            .storage
            .final_chain()
            .block_number_by_hash(H256::from(block_hash))?
        else {
            return Ok(TransactionView::default());
        };
        let block_number = decode_u64_le(
            &block_number_bytes,
            "CONSENSUS_QUERY_TRANSACTION_INDEX_BLOCK_NUMBER",
        )?;
        let view = self.transaction_by_block_number_and_index(block_number, transaction_index)?;
        if view.found && view.block_hash != block_hash {
            return Ok(TransactionView::default());
        }
        Ok(view)
    }

    /// Returns a public transaction receipt view by transaction hash.
    ///
    /// Missing transaction location, transaction payload, or receipt bytes
    /// return `found == false`. The receipt bytes are loaded through the
    /// finalized period receipt list when available and fall back to the legacy
    /// receipt-by-transaction-hash row, matching the C++ FinalChain lookup
    /// contract. Malformed transaction-location or block-hash rows are returned
    /// as errors with stable context labels.
    pub fn transaction_receipt_by_hash(&self, hash: [u8; 32]) -> Result<TransactionReceiptView> {
        let transaction = self.transaction_by_hash(hash)?;
        if !transaction.found || !transaction.location_found {
            return Ok(TransactionReceiptView::default());
        }

        let requested_hash = H256::from(hash);
        let receipt_rlp = match self
            .receipt_by_period_position(transaction.block_number, transaction.transaction_index)
            .context("CONSENSUS_QUERY_TRANSACTION_RECEIPT_BY_PERIOD")?
        {
            Some(receipt_rlp) => Some(receipt_rlp),
            None => self
                .storage
                .final_chain()
                .receipt_by_trx_hash(requested_hash)
                .context("CONSENSUS_QUERY_TRANSACTION_RECEIPT_BY_HASH")?,
        };
        let Some(receipt_rlp) = receipt_rlp else {
            return Ok(TransactionReceiptView::default());
        };

        Ok(TransactionReceiptView {
            found: true,
            transaction_hash: hash,
            transaction_source: transaction.source,
            transaction_rlp: transaction.transaction_rlp,
            receipt_rlp,
            block_number: transaction.block_number,
            transaction_index: transaction.transaction_index,
            is_system: transaction.is_system,
            block_hash_found: transaction.block_hash_found,
            block_hash: transaction.block_hash,
        })
    }
}

fn h256_bytes(bytes: &[u8]) -> Result<H256> {
    let array: [u8; 32] = bytes
        .try_into()
        .with_context(|| format!("expected 32-byte hash, got {}", bytes.len()))?;
    Ok(H256::from(array))
}

fn decode_u64_le(bytes: &[u8], context: &'static str) -> Result<u64> {
    let array: [u8; 8] = bytes
        .try_into()
        .with_context(|| format!("{context}: expected 8 bytes, got {}", bytes.len()))?;
    Ok(u64::from_le_bytes(array))
}

fn keccak256(data: &[u8]) -> H256 {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    H256::from(out)
}

fn pbft_schedule_block_view_from_period_data(period_data: &[u8]) -> Result<PbftScheduleBlockView> {
    let period_rlp = Rlp::new(period_data);
    let pbft_block = period_rlp
        .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)
        .context("CONSENSUS_QUERY_SCHEDULE_PBFT_BLOCK")?;
    let pbft_block_rlp = pbft_block.as_raw();
    let item_count = pbft_block
        .item_count()
        .context("CONSENSUS_QUERY_SCHEDULE_PBFT_ITEM_COUNT")?;
    anyhow::ensure!(
        item_count == 8 || item_count == 9,
        "CONSENSUS_QUERY_SCHEDULE_INVALID_PBFT_FIELD_COUNT"
    );

    let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(pbft_block_rlp))
        .context("CONSENSUS_QUERY_SCHEDULE_PBFT_METADATA")?;
    let has_extra_data = item_count == 9;
    let extra_data = if has_extra_data {
        decode_pbft_extra_data(
            pbft_block
                .at(PBFT_EXTRA_DATA_POS)
                .context("CONSENSUS_QUERY_SCHEDULE_EXTRA_DATA")?
                .data()
                .context("CONSENSUS_QUERY_SCHEDULE_EXTRA_DATA_BYTES")?,
        )?
    } else {
        PbftBlockExtraDataView::default()
    };
    let signature_pos = if has_extra_data {
        PBFT_SIGNATURE_WITH_EXTRA_POS
    } else {
        PBFT_SIGNATURE_WITHOUT_EXTRA_POS
    };
    let signature = pbft_block
        .at(signature_pos)
        .context("CONSENSUS_QUERY_SCHEDULE_SIGNATURE")?
        .data()
        .context("CONSENSUS_QUERY_SCHEDULE_SIGNATURE_BYTES")?
        .to_vec();
    anyhow::ensure!(
        signature.len() == 65,
        "CONSENSUS_QUERY_SCHEDULE_INVALID_SIGNATURE_LENGTH"
    );

    let dag_blocks_order = finalized_dag_hashes(
        period_rlp
            .at(DAG_BLOCKS_POS_IN_PERIOD_DATA)
            .context("CONSENSUS_QUERY_SCHEDULE_DAG_BUNDLE")?
            .as_raw(),
    )?;

    Ok(PbftScheduleBlockView {
        found: true,
        prev_block_hash: pbft_block
            .val_at::<H256>(PBFT_PREV_HASH_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_PREV_HASH")?
            .into(),
        dag_block_hash_as_pivot: pbft_block
            .val_at::<H256>(PBFT_PIVOT_HASH_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_PIVOT_HASH")?
            .into(),
        order_hash: pbft_block
            .val_at::<H256>(PBFT_ORDER_HASH_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_ORDER_HASH")?
            .into(),
        final_chain_hash: pbft_block
            .val_at::<H256>(PBFT_FINAL_CHAIN_HASH_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_FINAL_CHAIN_HASH")?
            .into(),
        period: pbft_block
            .val_at(PBFT_PERIOD_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_PERIOD")?,
        timestamp: pbft_block
            .val_at(PBFT_TIMESTAMP_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_TIMESTAMP")?,
        block_hash: keccak256(pbft_block_rlp).into(),
        signature,
        beneficiary: metadata.author.into(),
        reward_votes: pbft_block
            .list_at::<H256>(PBFT_REWARD_VOTES_POS)
            .context("CONSENSUS_QUERY_SCHEDULE_REWARD_VOTES")?
            .into_iter()
            .map(Into::into)
            .collect(),
        has_extra_data: extra_data.found,
        extra_data,
        dag_blocks_order,
    })
}

fn pbft_node_version_view_from_period_data(period_data: &[u8]) -> Result<PbftNodeVersionView> {
    let period_rlp = Rlp::new(period_data);
    let pbft_block = period_rlp
        .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)
        .context("CONSENSUS_QUERY_NODE_VERSION_PBFT_BLOCK")?;
    let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(pbft_block.as_raw()))
        .context("CONSENSUS_QUERY_NODE_VERSION_PBFT_METADATA")?;
    let extra_data = if pbft_block
        .item_count()
        .context("CONSENSUS_QUERY_NODE_VERSION_PBFT_ITEM_COUNT")?
        == 9
    {
        decode_pbft_extra_data(
            pbft_block
                .at(PBFT_EXTRA_DATA_POS)
                .context("CONSENSUS_QUERY_NODE_VERSION_EXTRA_DATA")?
                .data()
                .context("CONSENSUS_QUERY_NODE_VERSION_EXTRA_DATA_BYTES")?,
        )?
    } else {
        PbftBlockExtraDataView::default()
    };
    anyhow::ensure!(
        extra_data.found,
        "CONSENSUS_QUERY_NODE_VERSION_EXTRA_DATA_MISSING"
    );

    Ok(PbftNodeVersionView {
        found: true,
        beneficiary: metadata.author.into(),
        major_version: extra_data.major_version,
        minor_version: extra_data.minor_version,
        patch_version: extra_data.patch_version,
    })
}

fn decode_pbft_extra_data(bytes: &[u8]) -> Result<PbftBlockExtraDataView> {
    anyhow::ensure!(
        bytes.len() <= PBFT_EXTRA_DATA_MAX_SIZE,
        "CONSENSUS_QUERY_PBFT_EXTRA_DATA_TOO_LARGE"
    );
    let rlp = Rlp::new(bytes);
    if rlp.item_count().ok() != Some(PBFT_EXTRA_DATA_FIELDS) {
        return Ok(PbftBlockExtraDataView::default());
    }
    let major_version = match rlp.val_at(0) {
        Ok(value) => value,
        Err(_) => return Ok(PbftBlockExtraDataView::default()),
    };
    let minor_version = match rlp.val_at(1) {
        Ok(value) => value,
        Err(_) => return Ok(PbftBlockExtraDataView::default()),
    };
    let patch_version = match rlp.val_at(2) {
        Ok(value) => value,
        Err(_) => return Ok(PbftBlockExtraDataView::default()),
    };
    let net_version = match rlp.val_at(3) {
        Ok(value) => value,
        Err(_) => return Ok(PbftBlockExtraDataView::default()),
    };
    let node_implementation = match rlp.val_at(4) {
        Ok(value) => value,
        Err(_) => return Ok(PbftBlockExtraDataView::default()),
    };
    let pillar_block_hash = match rlp.at(5).and_then(|value| value.data()) {
        Ok([]) => None,
        Ok(data) if data.len() == 32 => Some(H256::from_slice(data)),
        Ok(_) => return Ok(PbftBlockExtraDataView::default()),
        Err(_) => return Ok(PbftBlockExtraDataView::default()),
    };

    Ok(PbftBlockExtraDataView {
        found: true,
        major_version,
        minor_version,
        patch_version,
        net_version,
        node_implementation,
        has_pillar_block_hash: pillar_block_hash.is_some(),
        pillar_block_hash: pillar_block_hash.unwrap_or_default().into(),
    })
}

fn finalized_dag_hashes(dag_bundle_rlp: &[u8]) -> Result<Vec<[u8; 32]>> {
    let bundle = Rlp::new(dag_bundle_rlp);
    if dag_bundle_is_empty(&bundle)? {
        return Ok(Vec::new());
    }
    anyhow::ensure!(
        bundle.item_count()? == 3,
        "CONSENSUS_QUERY_INVALID_FINALIZED_DAG_BUNDLE_FIELD_COUNT"
    );
    let compact_blocks = bundle
        .at(2)
        .context("CONSENSUS_QUERY_FINALIZED_DAG_COMPACT_BLOCKS")?;
    let finalized_bundle = FinalizedDagBlockBundleRlp::new(dag_bundle_rlp);
    let mut out = Vec::with_capacity(compact_blocks.item_count()?);
    for position in 0..compact_blocks.item_count()? {
        let canonical = finalized_bundle
            .canonical_block_rlp(position)
            .context("CONSENSUS_QUERY_FINALIZED_DAG_CANONICAL_BLOCK")?;
        out.push(keccak256(&canonical).into());
    }
    Ok(out)
}

fn finalized_dag_block_views(dag_bundle_rlp: &[u8], period: u64) -> Result<Vec<DagBlockView>> {
    let bundle = Rlp::new(dag_bundle_rlp);
    if dag_bundle_is_empty(&bundle)? {
        return Ok(Vec::new());
    }
    anyhow::ensure!(
        bundle.item_count()? == 3,
        "CONSENSUS_QUERY_INVALID_FINALIZED_DAG_BUNDLE_FIELD_COUNT"
    );
    let compact_blocks = bundle
        .at(2)
        .context("CONSENSUS_QUERY_FINALIZED_DAG_COMPACT_BLOCKS")?;
    let finalized_bundle = FinalizedDagBlockBundleRlp::new(dag_bundle_rlp);
    let mut out = Vec::with_capacity(compact_blocks.item_count()?);
    for position in 0..compact_blocks.item_count()? {
        let canonical = finalized_bundle
            .canonical_block_rlp(position)
            .context("CONSENSUS_QUERY_FINALIZED_DAG_CANONICAL_BLOCK")?;
        out.push(finalized_dag_block_view_from_canonical_rlp(
            &canonical,
            period,
            position as u32,
        )?);
    }
    Ok(out)
}

fn finalized_dag_block_view_from_canonical_rlp(
    block_rlp: &[u8],
    period: u64,
    position: u32,
) -> Result<DagBlockView> {
    let block = DagBlock::try_from(DagBlockRlp::new(block_rlp))
        .context("CONSENSUS_QUERY_FINALIZED_DAG_BLOCK_DECODE")?;
    let sender = block
        .recover_sender()
        .context("CONSENSUS_QUERY_FINALIZED_DAG_BLOCK_SENDER")?;
    let (has_vdf, vdf_proof, vdf_sol1, vdf_sol2, vdf_difficulty) = if block.level > 0 {
        let vdf = decode_vdf_sortition_payload(&block.vdf)
            .context("CONSENSUS_QUERY_FINALIZED_DAG_BLOCK_VDF_DECODE")?;
        (
            true,
            vdf.vrf_proof.to_vec(),
            vdf.vdf_solution_proof,
            vdf.vdf_solution_output,
            vdf.difficulty,
        )
    } else {
        (false, Vec::new(), Vec::new(), Vec::new(), 0)
    };

    Ok(DagBlockView {
        found: true,
        pivot: block.pivot.into(),
        level: block.level,
        tips: block.tips.into_iter().map(Into::into).collect(),
        transactions: block.transactions.into_iter().map(Into::into).collect(),
        trx_estimations: block.gas_estimation,
        signature: block.signature.to_vec(),
        hash: keccak256(block_rlp).into(),
        sender: sender.into(),
        timestamp: block.timestamp,
        finalized_period_found: true,
        finalized_period: period,
        finalized_position: position,
        has_vdf,
        vdf_proof,
        vdf_sol1,
        vdf_sol2,
        vdf_difficulty,
    })
}

fn dag_bundle_is_empty(bundle: &Rlp<'_>) -> Result<bool> {
    if bundle.is_list() {
        return Ok(false);
    }
    Ok(bundle.data()?.is_empty())
}

fn pillar_block_data_view(block_data: PillarBlockData) -> PillarBlockDataView {
    let block_hash = block_data.pillar_block.hash();
    PillarBlockDataView {
        found: true,
        pbft_period: block_data.pillar_block.period,
        state_root: block_data.pillar_block.state_root.into(),
        previous_pillar_block_hash: block_data.pillar_block.previous_pillar_block_hash.into(),
        bridge_root: block_data.pillar_block.bridge_root.into(),
        epoch: block_data.pillar_block.epoch,
        validator_vote_count_changes: block_data
            .pillar_block
            .validator_vote_count_changes
            .into_iter()
            .map(|change| PillarBlockViewVoteCountChange {
                address: change.address.into(),
                vote_count_change: change.vote_count_change,
            })
            .collect(),
        block_hash: block_hash.into(),
        signatures: block_data
            .pillar_votes
            .into_iter()
            .map(|vote| {
                let (r, vs) = vote.compact_signature_words();
                PillarBlockViewSignature { r, vs }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::U256;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::Config;
    use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlpOwned;
    use rustaxa_types::pillar::{
        PillarBlock, PillarVote, ValidatorVoteCountChange, encode_optimized_pillar_votes_bundle_rlp,
    };
    use std::sync::Arc;

    fn test_storage(name: &str) -> (std::path::PathBuf, Arc<Storage>) {
        let path = std::env::temp_dir().join(format!(
            "rustaxa_consensus_query_api_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        (path, storage)
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
        let mut block = DagBlock {
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
        let mut block = DagBlock {
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

    fn stored_header_rlp() -> Vec<u8> {
        StoredBlockHeaderRlpOwned::from(&StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(1),
            state_root: H256::from_low_u64_be(2),
            transactions_root: H256::from_low_u64_be(3),
            receipts_root: H256::from_low_u64_be(4),
            log_bloom: vec![0xAA; 256],
            gas_used: 55,
            total_reward: U256::from(66u64),
        })
        .into_vec()
    }

    #[test]
    fn query_api_reads_final_chain_block_view_and_pbft_hash_from_storage() {
        let (path, storage) = test_storage("block_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_hash = H256::from_low_u64_be(77);
        let pbft_block_rlp = vec![0xC2, 0x01, 0x02];
        storage
            .period()
            .write(9, &period_data_rlp(&pbft_block_rlp))
            .unwrap();
        storage
            .final_chain()
            .write_block_header(9, block_hash, &stored_header_rlp(), &[])
            .unwrap();

        let view = api.final_chain_block_by_number(9).unwrap();
        assert!(view.found);
        assert_eq!(view.number, 9);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.parent_hash, H256::from_low_u64_be(1).0);
        assert_eq!(view.state_root, H256::from_low_u64_be(2).0);
        assert_eq!(view.transactions_root, H256::from_low_u64_be(3).0);
        assert_eq!(view.receipts_root, H256::from_low_u64_be(4).0);
        assert_eq!(view.log_bloom, vec![0xAA; 256]);
        assert_eq!(view.gas_used, 55);
        assert_eq!(view.total_reward, U256::from(66u64).to_big_endian());
        assert!(!view.stored_header_rlp.is_empty());
        assert!(view.has_pbft_hash);
        assert_eq!(view.pbft_block_hash, keccak256(&pbft_block_rlp).0);

        let lookup = api.pbft_block_hash_by_period(9).unwrap();
        assert!(lookup.found);
        assert_eq!(lookup.hash, view.pbft_block_hash);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_returns_not_found_for_missing_public_rows() {
        let (path, storage) = test_storage("missing");
        let api = ConsensusQueryApi::new(storage.clone());

        assert!(!api.final_chain_block_by_number(44).unwrap().found);
        assert!(!api.pbft_block_hash_by_period(44).unwrap().found);
        assert!(!api.pbft_schedule_block_by_period(44).unwrap().found);
        assert!(!api.pbft_node_version_by_period(44).unwrap().found);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_pbft_schedule_block_view_from_period_data() {
        let (path, storage) = test_storage("schedule_block_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key);
        storage
            .period()
            .write(7, &period_data_rlp_with_dag_bundle(&pbft_block))
            .unwrap();

        let view = api.pbft_schedule_block_by_period(7).unwrap();

        assert!(view.found);
        assert_eq!(view.prev_block_hash, H256::from_low_u64_be(10).0);
        assert_eq!(view.dag_block_hash_as_pivot, H256::from_low_u64_be(11).0);
        assert_eq!(view.order_hash, H256::from_low_u64_be(12).0);
        assert_eq!(view.final_chain_hash, H256::from_low_u64_be(13).0);
        assert_eq!(view.period, 7);
        assert_eq!(view.timestamp, 99);
        assert_eq!(view.block_hash, keccak256(&pbft_block).0);
        assert_eq!(view.signature.len(), 65);
        assert_eq!(
            view.beneficiary,
            PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&pbft_block))
                .unwrap()
                .author
                .0
        );
        assert_eq!(
            view.reward_votes,
            vec![H256::from_low_u64_be(20).0, H256::from_low_u64_be(21).0]
        );
        assert!(view.has_extra_data);
        assert_eq!(view.extra_data.major_version, 1);
        assert_eq!(view.extra_data.minor_version, 2);
        assert_eq!(view.extra_data.patch_version, 3);
        assert_eq!(view.extra_data.net_version, 4);
        assert_eq!(view.extra_data.node_implementation, "rustaxa-test");
        assert!(view.extra_data.has_pillar_block_hash);
        assert_eq!(
            view.extra_data.pillar_block_hash,
            H256::from_low_u64_be(55).0
        );
        assert!(view.dag_blocks_order.is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_pbft_node_version_view_from_period_data() {
        let (path, storage) = test_storage("node_version_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key);
        storage
            .period()
            .write(7, &period_data_rlp(&pbft_block))
            .unwrap();

        let view = api.pbft_node_version_by_period(7).unwrap();

        assert!(view.found);
        assert_eq!(
            view.beneficiary,
            PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&pbft_block))
                .unwrap()
                .author
                .0
        );
        assert_eq!(view.major_version, 1);
        assert_eq!(view.minor_version, 2);
        assert_eq!(view.patch_version, 3);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_pillar_block_data_view_from_storage() {
        let (path, storage) = test_storage("pillar_block_data_view");
        let api = ConsensusQueryApi::new(storage.clone());
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

        storage.pillar().write(10, &block.encode_rlp()).unwrap();
        storage
            .period()
            .write(11, &period_data_with_pillar_votes_rlp(&votes_bundle_rlp))
            .unwrap();

        let view = api.pillar_block_data_by_period(10).unwrap();
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

        assert!(!api.pillar_block_data_by_period(12).unwrap().found);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_dag_block_view_and_finalized_period_from_storage() {
        let (path, storage) = test_storage("dag_block_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_rlp = dag_block_rlp();
        let block_hash = keccak256(&block_rlp);
        storage.dag().write(block_hash, 5, 1, &block_rlp).unwrap();
        storage.dag().write_period(block_hash, 9, 2).unwrap();

        let view = api.dag_block_by_hash(block_hash.0).unwrap();
        assert!(view.found);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.pivot, H256::from_low_u64_be(1).0);
        assert_eq!(view.level, 5);
        assert_eq!(view.timestamp, 123);
        assert_eq!(view.tips, vec![H256::from_low_u64_be(2).0]);
        assert_eq!(
            view.transactions,
            vec![H256::from_low_u64_be(3).0, H256::from_low_u64_be(4).0]
        );
        assert_eq!(view.trx_estimations, 987);
        assert_eq!(view.signature.len(), 65);
        assert!(view.finalized_period_found);
        assert_eq!(view.finalized_period, 9);
        assert_eq!(view.vdf_proof, vec![0x11; 80]);
        assert_eq!(view.vdf_sol1, vec![0x22, 0x23]);
        assert_eq!(view.vdf_sol2, vec![0x33, 0x34]);
        assert_eq!(view.vdf_difficulty, 7);

        let level_views = api.dag_blocks_by_level(5, 1).unwrap();
        assert_eq!(level_views.len(), 1);
        assert_eq!(level_views[0].hash, block_hash.0);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_transaction_view_from_storage() {
        let (path, storage) = test_storage("transaction_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let pending_hash = H256::from_low_u64_be(1);
        let finalized_hash = H256::from_low_u64_be(2);
        let missing_hash = H256::from_low_u64_be(3);
        let system_hash = H256::from_low_u64_be(4);
        storage
            .transaction()
            .write(pending_hash, &[0x11])
            .expect("pending transaction should persist");
        storage
            .transaction()
            .write_location(finalized_hash, 8, 0, false)
            .expect("finalized transaction location should persist");
        storage
            .period()
            .write(8, &period_data_with_transactions_rlp(&[vec![0x22]]))
            .expect("period transaction payload should persist");
        storage
            .transaction()
            .write_location(system_hash, 9, 0, true)
            .expect("system transaction location should persist");
        storage
            .transaction()
            .write_system(system_hash, &[0x44])
            .expect("system transaction payload should persist");

        let pending = api.transaction_by_hash(pending_hash.0).unwrap();
        assert!(pending.found);
        assert_eq!(pending.hash, pending_hash.0);
        assert_eq!(pending.source, STORED_TRANSACTION_SOURCE_PENDING);
        assert_eq!(pending.transaction_rlp, vec![0x11]);

        let finalized = api.transaction_by_hash(finalized_hash.0).unwrap();
        assert!(finalized.found);
        assert_eq!(
            finalized.source,
            STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        );
        assert!(finalized.location_found);
        assert_eq!(finalized.block_number, 8);
        assert_eq!(finalized.transaction_index, 0);
        assert!(!finalized.is_system);
        assert!(!finalized.block_hash_found);
        assert_eq!(finalized.transaction_rlp, vec![0x22]);

        let system = api.transaction_by_hash(system_hash.0).unwrap();
        assert!(system.found);
        assert_eq!(system.source, STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM);
        assert!(system.location_found);
        assert_eq!(system.block_number, 9);
        assert_eq!(system.transaction_index, 0);
        assert!(system.is_system);
        assert_eq!(system.transaction_rlp, vec![0x44]);

        let missing = api.transaction_by_hash(missing_hash.0).unwrap();
        assert!(!missing.found);
        assert_eq!(missing.source, STORED_TRANSACTION_SOURCE_MISSING);
        assert!(missing.transaction_rlp.is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_indexed_transaction_view_from_storage() {
        let (path, storage) = test_storage("indexed_transaction_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_hash = H256::from_low_u64_be(0x24);
        let first_rlp = vec![0x22];
        let second_rlp = vec![0x33];

        storage
            .period()
            .write(
                12,
                &period_data_with_transactions_rlp(&[first_rlp.clone(), second_rlp.clone()]),
            )
            .expect("period transaction payloads should persist");
        storage
            .final_chain()
            .write_conformance_lookup_rows(
                0,
                b"meta",
                12,
                block_hash,
                &[0xC0],
                H256::zero(),
                &[0xC0],
                H256::zero(),
                &[0xC0],
                12,
                &receipt_list_rlp(&[]),
            )
            .expect("final-chain lookup rows should persist");

        let by_number = api
            .transaction_by_block_number_and_index(12, 1)
            .expect("number-index query should succeed");
        assert!(by_number.found);
        assert_eq!(by_number.hash, keccak256(&second_rlp).0);
        assert_eq!(
            by_number.source,
            STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        );
        assert!(by_number.location_found);
        assert_eq!(by_number.block_number, 12);
        assert_eq!(by_number.transaction_index, 1);
        assert!(!by_number.is_system);
        assert!(by_number.block_hash_found);
        assert_eq!(by_number.block_hash, block_hash.0);
        assert_eq!(by_number.transaction_rlp, second_rlp);

        let by_hash = api
            .transaction_by_block_hash_and_index(block_hash.0, 0)
            .expect("hash-index query should succeed");
        assert!(by_hash.found);
        assert_eq!(by_hash.hash, keccak256(&first_rlp).0);
        assert_eq!(by_hash.transaction_rlp, first_rlp);

        assert!(
            !api.transaction_by_block_number_and_index(12, 2)
                .unwrap()
                .found
        );
        assert!(
            !api.transaction_by_block_number_and_index(99, 0)
                .unwrap()
                .found
        );
        assert!(
            !api.transaction_by_block_hash_and_index([0x99; 32], 0)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_transaction_receipt_view_from_storage() {
        let (path, storage) = test_storage("transaction_receipt_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let trx_hash = H256::from_low_u64_be(0x21);
        let fallback_hash = H256::from_low_u64_be(0x22);
        let missing_hash = H256::from_low_u64_be(0x23);
        let block_hash = H256::from_low_u64_be(0x24);
        let fallback_block_hash = H256::from_low_u64_be(0x25);
        let trx_rlp = vec![0x31];
        let fallback_trx_rlp = vec![0x32];
        let receipt_rlp = vec![0x41];
        let fallback_receipt_rlp = vec![0x42];

        storage
            .transaction()
            .write_location(trx_hash, 12, 0, false)
            .unwrap();
        storage
            .period()
            .write(
                12,
                &period_data_with_transactions_rlp(std::slice::from_ref(&trx_rlp)),
            )
            .unwrap();
        storage
            .final_chain()
            .write_conformance_lookup_rows(
                0,
                b"meta",
                12,
                block_hash,
                &[0xC0],
                trx_hash,
                &receipt_rlp,
                H256::zero(),
                &[0xC0],
                12,
                &receipt_list_rlp(std::slice::from_ref(&receipt_rlp)),
            )
            .unwrap();

        storage
            .transaction()
            .write_location(fallback_hash, 13, 0, false)
            .unwrap();
        storage
            .period()
            .write(
                13,
                &period_data_with_transactions_rlp(std::slice::from_ref(&fallback_trx_rlp)),
            )
            .unwrap();
        storage
            .final_chain()
            .write_conformance_lookup_rows(
                1,
                b"meta",
                13,
                fallback_block_hash,
                &[0xC0],
                fallback_hash,
                &fallback_receipt_rlp,
                H256::zero(),
                &[0xC0],
                13,
                &receipt_list_rlp(&[]),
            )
            .unwrap();

        let view = api.transaction_receipt_by_hash(trx_hash.0).unwrap();
        assert!(view.found);
        assert_eq!(view.transaction_hash, trx_hash.0);
        assert_eq!(
            view.transaction_source,
            STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        );
        assert_eq!(view.transaction_rlp, trx_rlp);
        assert_eq!(view.receipt_rlp, receipt_rlp);
        assert_eq!(view.block_number, 12);
        assert_eq!(view.transaction_index, 0);
        assert!(view.block_hash_found);
        assert_eq!(view.block_hash, block_hash.0);

        let fallback = api.transaction_receipt_by_hash(fallback_hash.0).unwrap();
        assert!(fallback.found);
        assert_eq!(fallback.receipt_rlp, fallback_receipt_rlp);
        assert_eq!(fallback.block_hash, fallback_block_hash.0);

        assert!(
            !api.transaction_receipt_by_hash(missing_hash.0)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_finalized_dag_block_views_from_period_data() {
        let (path, storage) = test_storage("finalized_dag_blocks_by_period");
        let api = ConsensusQueryApi::new(storage.clone());
        let (dag_bundle, canonical_block) = signed_finalized_dag_bundle_rlp();
        storage
            .period()
            .write(7, &period_data_with_dag_bundle_rlp(&dag_bundle))
            .unwrap();

        let views = api.finalized_dag_blocks_by_period(7).unwrap();

        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert!(view.found);
        assert_eq!(view.hash, keccak256(&canonical_block).0);
        assert_eq!(view.pivot, H256::from_low_u64_be(1).0);
        assert_eq!(view.level, 5);
        assert_eq!(view.timestamp, 123);
        assert_eq!(view.tips, vec![H256::from_low_u64_be(2).0]);
        assert_eq!(
            view.transactions,
            vec![H256::from_low_u64_be(3).0, H256::from_low_u64_be(4).0]
        );
        assert_eq!(view.trx_estimations, 987);
        assert!(view.finalized_period_found);
        assert_eq!(view.finalized_period, 7);
        assert_eq!(view.finalized_position, 0);
        assert_eq!(view.vdf_proof, vec![0x11; 80]);
        assert_eq!(view.vdf_sol1, vec![0x22, 0x23]);
        assert_eq!(view.vdf_sol2, vec![0x33, 0x34]);
        assert_eq!(view.vdf_difficulty, 7);
        assert!(api.finalized_dag_blocks_by_period(8).unwrap().is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
