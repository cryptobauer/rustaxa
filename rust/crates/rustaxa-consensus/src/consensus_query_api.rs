//! Public query facade for Rust-owned consensus read models.
//!
//! The facade is the narrow read-only API that RPC, GraphQL, plugins, debug
//! tools, and CLI code should call instead of reaching into consensus managers,
//! mutable sidecars, or generic storage iterators. It owns cloned references to
//! the shared storage, PBFT, and FinalChain siblings and mutates no state;
//! callers receive stable DTOs plus canonical bytes when compatibility
//! materializers still need legacy encodings.

use anyhow::{Context, Result};
use ethereum_types::{H160, H256};
use rlp::{Rlp, RlpStream};
use rustaxa_storage::{
    FINAL_CHAIN_BLOOM_INDEX_LEVELS, FINAL_CHAIN_BLOOM_INDEX_SIZE, FinalChainLogBloom, StatusField,
    Storage, decode_final_chain_log_bloom_chunk, final_chain_log_bloom_chunk_id,
};
use rustaxa_types::codec::rlp::dag::{DagBlockRlp, FinalizedDagBlockBundleRlp};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::dag::DagBlock;
use rustaxa_types::final_chain::StoredFinalChainBlockHeader;
use rustaxa_types::pillar::{PillarBlockData, RawPillarBlockData};
use rustaxa_types::{DposValidatorStake, PbftBlockMetadata};
use rustaxa_vdf::vdf_sortition::decode_vdf_sortition_payload;
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

use crate::dag_transaction_service::{
    DagNonFinalizedIndex, DagRuntimeStatus, DagTransactionService, TransactionPoolStatus,
};
use crate::final_chain::FinalChain;
use crate::network_api::NetworkPbftSyncSnapshot;
use crate::pbft_service::PbftService;
use crate::sortition::{SortitionParamsChange, THRESHOLD_UPPER_MIN_VALUE};
use crate::verified_votes::PbftVoteType;

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
const CERT_VOTES_POS_IN_PERIOD_DATA: usize = 1;
const PILLAR_VOTES_POS_IN_PERIOD_DATA: usize = 4;
const PBFT_CERT_VOTE_STEP: u64 = 3;
const PBFT_VOTES_BUNDLE_FIELDS: usize = 5;
const PBFT_VOTES_BUNDLE_BLOCK_HASH_POS: usize = 0;
const PBFT_VOTES_BUNDLE_PERIOD_POS: usize = 1;
const PBFT_VOTES_BUNDLE_ROUND_POS: usize = 2;
const PBFT_VOTES_BUNDLE_STEP_POS: usize = 3;
const PBFT_VOTES_BUNDLE_VOTES_POS: usize = 4;
const PBFT_OPTIMIZED_VOTE_PROOF_POS: usize = 0;
const PBFT_OPTIMIZED_VOTE_SIGNATURE_POS: usize = 1;
const FINAL_CHAIN_META_LAST_NUMBER: u32 = 1;

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

/// Number lookup result returned by public query facade methods.
///
/// `found` is false when the requested durable row does not exist. When
/// `found` is true, `value` contains the canonical unsigned block/period number
/// decoded from Rust-owned storage bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryNumberLookup {
    pub found: bool,
    pub value: u64,
}

/// Optional dynamic-lambda lookup returned by public query facade methods.
///
/// `found` is false when no exact period-lambda row exists. When `found` is
/// true, `value` carries the persisted lambda in milliseconds. The query facade
/// exposes this as a dedicated public read so RPC callers do not reach into
/// generic metadata storage for consensus-owned dynamic-lambda facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryPeriodLambda {
    pub found: bool,
    pub value: u32,
}

/// Storage-backed public chain statistics view.
///
/// The view contains the finalized period and status counters exposed by
/// public status/statistics endpoints. Values default to zero when the
/// corresponding storage row is absent, matching the Rust metadata repository
/// and legacy genesis behavior. `dag_blocks_count` and `transactions_count`
/// are persisted DAG/transaction-manager counters for compatibility status
/// routes; they are not live mempool, peer, or sync-progress facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChainStatsView {
    pub pbft_period: u64,
    pub non_empty_pbft_periods: u64,
    pub dag_blocks_count: u64,
    pub transactions_count: u64,
    pub dag_blocks_executed: u64,
    pub transactions_executed: u64,
}

/// Coherent live PBFT progress exposed to public and transport readers.
///
/// Both counters are sampled from one application-owned chain head. No hashes,
/// mutation hooks, or storage handles cross the client boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PbftProgressView {
    pub finalized_period: u64,
    pub non_empty_finalized_periods: u64,
}

/// Storage-backed public consensus status view.
///
/// This view is the read-only status DTO for public query clients that need
/// consensus-owned finalized and DAG index facts without holding live manager
/// pointers. `latest_dag_period_found` is false when the DAG level index exists
/// but no proposal-period mapping has been persisted for that level, which can
/// happen during genesis, import, or repair windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsensusStatusView {
    pub final_block_number: u64,
    pub latest_dag_level: u64,
    pub latest_dag_period_found: bool,
    pub latest_dag_period: u64,
}

/// Storage-backed public view of the sortition params change active for a period.
///
/// The view is intentionally narrower than the full sortition manager state:
/// it only exposes the compatibility fields returned by the Test RPC
/// `get_sortition_change`. `found` is false when no params-change row exists at
/// or before the requested period. Malformed storage bytes are returned as query
/// errors so public adapters do not silently preserve corrupt consensus config.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SortitionParamsChangeView {
    pub found: bool,
    pub period: u64,
    pub interval_efficiency: u16,
    pub threshold_upper: u16,
    pub threshold_upper_min: u16,
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
    /// Canonical fully materialized legacy header RLP when a live application root is attached.
    pub header_rlp: Vec<u8>,
    pub stored_header_rlp: Vec<u8>,
    pub has_pbft_hash: bool,
    pub pbft_block_hash: [u8; 32],
}

/// Stable public view of one DAG block.
///
/// The view is loaded from Rust DAG storage and contains the base facts public
/// RPC/GraphQL formatters need for DAG block JSON without exposing a live DAG
/// manager or C++ block object. `block_rlp` preserves the canonical storage
/// bytes for compatibility clients that still have to materialize a legacy C++
/// `DagBlock` at the public API edge. `finalized_period_found` distinguishes
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
    pub block_rlp: Vec<u8>,
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

/// Stable public view of one transaction receipt resolved by transaction lookup.
///
/// The view is built from Rust-owned transaction location, FinalChain block
/// hash, transaction payload, and receipt indexes. The receipt remains canonical
/// RLP so C++ public adapters can keep existing JSON formatting while no longer
/// reading `FinalChain` directly for receipt lookup.
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

/// Canonical PBFT vote bytes decoded from a previous-block cert-vote bundle.
///
/// `vote_rlp` is reconstructed as legacy `PbftVote::rlp(true, false)` from the
/// optimized vote entry stored in `PeriodData`, using the bundle-level block
/// hash, period, round, and step. Public adapters may materialize this at the
/// formatting edge while the storage/query authority remains Rust-owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PbftCertVoteRlp {
    pub vote_rlp: Vec<u8>,
}

/// Stable public view of previous-block PBFT cert votes for a finalized period.
///
/// The view is decoded from the cert-vote bundle embedded in finalized
/// `PeriodData`. It returns only bundle identity and canonical vote bytes so
/// debug/public adapters can keep legacy JSON formatting and live validation at
/// the edge without reaching into `DbStorage` for consensus-owned period data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PbftPeriodCertVotesView {
    pub found: bool,
    pub period: u64,
    pub certified_period: u64,
    pub round: u64,
    pub step: u64,
    pub block_hash: [u8; 32],
    pub votes: Vec<PbftCertVoteRlp>,
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
    pub epoch: [u8; 32],
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
    live_pbft: Option<Arc<PbftService>>,
    live_final_chain: Option<Arc<FinalChain>>,
    live_dag_transaction: Option<Arc<DagTransactionService>>,
}

impl ConsensusQueryApi {
    /// Creates a public query facade over a shared Rust storage owner.
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            live_pbft: None,
            live_final_chain: None,
            live_dag_transaction: None,
        }
    }

    /// Creates the application-root query client over durable storage and the
    /// live PBFT owner.
    ///
    /// Only [`crate::ConsensusApplication`] constructs this form. Public
    /// clients can observe the current in-memory chain head without receiving
    /// a mutable PBFT service or a separately constructible chain facade.
    pub(crate) fn new_live(
        storage: Arc<Storage>,
        pbft: Arc<PbftService>,
        final_chain: Arc<FinalChain>,
        dag_transaction: Arc<DagTransactionService>,
    ) -> Self {
        Self {
            storage,
            live_pbft: Some(pbft),
            live_final_chain: Some(final_chain),
            live_dag_transaction: Some(dag_transaction),
        }
    }

    /// Returns a lock-coherent live DAG graph/head snapshot.
    ///
    /// Storage-only fixtures fail with a stable unavailable error. Production
    /// clients receive owned values and cannot access the graph or its mutex.
    pub fn dag_live_status(&self) -> Result<DagRuntimeStatus> {
        self.live_dag_transaction
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_DAG_UNAVAILABLE")?
            .dag_runtime_status()
    }

    /// Returns the complete live non-finalized DAG level index.
    pub fn dag_live_non_finalized_index(&self) -> Result<DagNonFinalizedIndex> {
        self.live_dag_transaction
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_DAG_UNAVAILABLE")?
            .dag_non_finalized_index()
    }

    /// Returns one lock-coherent public transaction-pool status snapshot.
    pub fn transaction_pool_status(&self) -> Result<TransactionPoolStatus> {
        self.live_dag_transaction
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_TRANSACTION_UNAVAILABLE")?
            .transaction_pool_status()
    }

    /// Returns a side-effect-free snapshot of the application-owned PBFT-sync lifecycle.
    ///
    /// `now_ms` must use the same monotonic clock domain as network lifecycle
    /// events. It is used only to derive elapsed durations; queries never
    /// expire, stop, or otherwise mutate a sync session. Storage-only fixtures
    /// fail with a stable unavailable error.
    pub fn pbft_sync_status(&self, now_ms: u64) -> Result<NetworkPbftSyncSnapshot> {
        self.live_pbft
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_NETWORK_UNAVAILABLE")?
            .network_service()
            .pbft_sync_status(now_ms)
    }

    /// Returns whether native live queue/sidecar state knows a transaction.
    pub fn live_transaction_is_known(&self, hash: [u8; 32]) -> Result<bool> {
        self.live_dag_transaction
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_TRANSACTION_UNAVAILABLE")?
            .transaction_is_known(hash)
    }

    /// Returns the number of votes in the live application-owned verified-vote index.
    ///
    /// Storage-only fixtures have no live vote owner and return
    /// `CONSENSUS_QUERY_LIVE_PBFT_UNAVAILABLE`. The count is sampled under the
    /// native verified-vote lock and never exposes the mutable index.
    pub fn verified_vote_count(&self) -> Result<u64> {
        self.live_pbft
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_PBFT_UNAVAILABLE")?
            .verified_votes_size()
    }

    /// Resolves the live PBFT quorum threshold for one period and vote kind.
    ///
    /// The query composes the application-owned PBFT and FinalChain siblings,
    /// including the native threshold cache and exact DPoS lookup. Unsupported
    /// vote kinds, future DPoS state, and infrastructure failures remain typed
    /// planner statuses rather than being converted to a misleading quorum.
    pub fn pbft_vote_threshold(
        &self,
        period: u64,
        vote_type: PbftVoteType,
    ) -> Result<crate::pbft_thresholds::PbftTwoTPlusOneThresholdPlan> {
        let pbft = self
            .live_pbft
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_PBFT_UNAVAILABLE")?;
        let final_chain = self
            .live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?;
        pbft.public_vote_threshold(final_chain.as_ref(), period, vote_type)
    }

    /// Returns the current application-owned PBFT chain head.
    ///
    /// Storage-only query fixtures do not have a live PBFT owner and receive a
    /// stable error. Production query clients are always constructed by the
    /// application root and therefore observe the same head as consensus tasks.
    pub fn pbft_progress(&self) -> Result<PbftProgressView> {
        let head = self
            .live_pbft
            .as_ref()
            .map(|pbft| pbft.pbft_chain_head())
            .context("CONSENSUS_QUERY_LIVE_PBFT_UNAVAILABLE")?;
        Ok(PbftProgressView {
            finalized_period: head.size,
            non_empty_finalized_periods: head.non_empty_size,
        })
    }

    /// Returns whether a finalized PBFT block hash has a durable period index.
    pub fn pbft_sync_block_exists(&self, block_hash: [u8; 32]) -> Result<bool> {
        crate::pbft_chain::pbft_block_exists_in_storage(&self.storage, H256::from(block_hash))
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

    /// Returns previous-block PBFT cert-vote bytes embedded in one finalized period.
    ///
    /// The query reads only Rust-owned period storage and reconstructs canonical
    /// full vote RLP payloads from the optimized cert-vote bundle. Missing
    /// period data or empty cert-vote bundles return `found == false`, matching
    /// legacy debug RPC behavior. Malformed period-data or non-cert vote-bundle
    /// shape is returned as an error so public adapters can preserve
    /// invalid-params handling.
    pub fn pbft_previous_block_cert_votes_by_period(
        &self,
        period: u64,
    ) -> Result<PbftPeriodCertVotesView> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(PbftPeriodCertVotesView::default());
        }
        let period_rlp = Rlp::new(&period_data);
        let votes_bundle_rlp = period_rlp
            .at(CERT_VOTES_POS_IN_PERIOD_DATA)
            .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES")?;
        pbft_period_cert_votes_view_from_bundle(period, &votes_bundle_rlp)
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
        let header_rlp = match &self.live_final_chain {
            Some(final_chain) => final_chain.block_header(number.into())?.unwrap_or_default(),
            None => Vec::new(),
        };

        Ok(FinalChainBlockView {
            found: true,
            number,
            hash: block_hash.into(),
            parent_hash: stored_header.parent_hash.into(),
            author: H160::zero().into(),
            state_root: stored_header.state_root.into(),
            transactions_root: stored_header.transactions_root.into(),
            receipts_root: stored_header.receipts_root.into(),
            log_bloom: stored_header.log_bloom.as_ref().to_vec(),
            gas_used: stored_header.gas_used.as_u64(),
            total_reward: stored_header.total_reward.to_fixed_be_bytes(),
            header_rlp,
            stored_header_rlp,
            has_pbft_hash: pbft_hash.found,
            pbft_block_hash: pbft_hash.hash,
        })
    }

    /// Returns the finalized FinalChain block number for `block_hash`.
    ///
    /// The lookup reads the Rust-owned hash-to-number index directly. Missing
    /// hashes return `found == false`; malformed number bytes are errors so
    /// public adapters surface storage inconsistency instead of falling back to
    /// `FinalChain` in Rust mode.
    pub fn final_chain_block_number_by_hash(
        &self,
        block_hash: [u8; 32],
    ) -> Result<QueryNumberLookup> {
        let Some(number_bytes) = self
            .storage
            .final_chain()
            .block_number_by_hash(H256::from(block_hash))?
        else {
            return Ok(QueryNumberLookup::default());
        };
        Ok(QueryNumberLookup {
            found: true,
            value: decode_u64_le(&number_bytes, "CONSENSUS_QUERY_FINAL_CHAIN_BLOCK_NUMBER")?,
        })
    }

    /// Returns the latest finalized FinalChain block number.
    ///
    /// The value is read directly from Rust-owned FinalChain metadata. Missing
    /// metadata returns zero, matching the Rust FinalChain runtime and legacy
    /// genesis behavior.
    pub fn final_chain_last_block_number(&self) -> Result<u64> {
        let Some(raw) = self
            .storage
            .final_chain()
            .meta_value(FINAL_CHAIN_META_LAST_NUMBER)?
        else {
            return Ok(0);
        };
        decode_u64_le(&raw, "CONSENSUS_QUERY_FINAL_CHAIN_LAST_NUMBER")
    }

    /// Returns one validator's eligible DPoS vote count at an exact finalized block.
    pub fn final_chain_dpos_eligible_vote_count(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<u64> {
        self.live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?
            .dpos_eligible_vote_count(block_number.into(), address)
    }

    /// Returns the total eligible DPoS vote count at an exact finalized block.
    pub fn final_chain_dpos_eligible_total_vote_count(&self, block_number: u64) -> Result<u64> {
        self.live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?
            .dpos_eligible_total_vote_count(block_number.into())
    }

    /// Returns validator stakes in canonical address order.
    pub fn final_chain_dpos_validators_total_stakes(
        &self,
        block_number: u64,
    ) -> Result<Vec<DposValidatorStake>> {
        self.live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?
            .dpos_validators_total_stakes(block_number.into())
    }

    /// Returns the total delegated amount as canonical unsigned big-endian bytes.
    pub fn final_chain_dpos_total_amount_delegated(&self, block_number: u64) -> Result<Vec<u8>> {
        self.live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?
            .dpos_total_amount_delegated(block_number.into())
    }

    /// Returns the native DPoS yield active at the requested finalized block.
    pub fn final_chain_dpos_yield(&self, block_number: u64) -> Result<u64> {
        self.live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?
            .dpos_yield(block_number.into())
    }

    /// Returns total supply as canonical unsigned big-endian bytes.
    pub fn final_chain_dpos_total_supply(&self, block_number: u64) -> Result<Vec<u8>> {
        self.live_final_chain
            .as_ref()
            .context("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")?
            .dpos_total_supply(block_number.into())
    }

    /// Returns the exact persisted dynamic lambda for a finalized period.
    ///
    /// This is the public-query route for `taraxa_getPeriodLambda`. It
    /// intentionally does not use closest-prior fallback because that RPC has
    /// historically reported only rows explicitly saved for the requested
    /// period.
    pub fn period_lambda_by_period(&self, period: u64) -> Result<QueryPeriodLambda> {
        Ok(
            match self.storage.metadata().period_lambda(period, false)? {
                Some(value) => QueryPeriodLambda { found: true, value },
                None => QueryPeriodLambda::default(),
            },
        )
    }

    /// Returns the finalized proposal period mapped to a DAG level.
    ///
    /// This is the public-query route for light-node history cleanup. It reads
    /// the Rust DAG index directly and returns an optional scalar instead of
    /// exposing `DbStorage` or generic DAG storage queries to plugin code.
    pub fn proposal_period_for_dag_level(&self, level: u64) -> Result<QueryNumberLookup> {
        Ok(match self.storage.dag().proposal_period_at_level(level)? {
            Some(value) => QueryNumberLookup { found: true, value },
            None => QueryNumberLookup::default(),
        })
    }

    /// Returns the public chain statistics view.
    ///
    /// This query keeps `taraxa_getChainStats` behind the public read facade
    /// without injecting `FinalChain` or `DbStorage` into RPC code. Production
    /// clients sample both PBFT counters from one live application
    /// head. Storage-only fixtures retain the persisted finalized-period value.
    /// Executed DAG block and transaction counters come from Rust status fields.
    pub fn chain_stats(&self) -> Result<ChainStatsView> {
        let (pbft_period, non_empty_pbft_periods) = match &self.live_pbft {
            Some(_) => {
                let progress = self.pbft_progress()?;
                (
                    progress.finalized_period,
                    progress.non_empty_finalized_periods,
                )
            }
            None => {
                let period = self.final_chain_last_block_number()?;
                (period, period)
            }
        };
        Ok(ChainStatsView {
            pbft_period,
            non_empty_pbft_periods,
            dag_blocks_count: self
                .storage
                .metadata()
                .status_field(StatusField::DagBlkCount as u8)?,
            transactions_count: self
                .storage
                .metadata()
                .status_field(StatusField::TrxCount as u8)?,
            dag_blocks_executed: self
                .storage
                .metadata()
                .status_field(StatusField::ExecutedBlkCount as u8)?,
            transactions_executed: self
                .storage
                .metadata()
                .status_field(StatusField::ExecutedTrxCount as u8)?,
        })
    }

    /// Returns the storage-backed public consensus status view.
    ///
    /// This query is deliberately narrower than live node status: it exposes
    /// finalized head and DAG index facts that are already persisted in
    /// Rust-owned storage. Peer progress, active syncing state, mempool size,
    /// and other live network/manager facts remain outside this storage query
    /// facade until they have dedicated runtime DTOs.
    pub fn consensus_status(&self) -> Result<ConsensusStatusView> {
        let latest_dag_level = self.storage.dag().last_level()?;
        let latest_dag_period = self
            .storage
            .dag()
            .proposal_period_at_level(latest_dag_level)?;
        Ok(ConsensusStatusView {
            final_block_number: self.final_chain_last_block_number()?,
            latest_dag_level,
            latest_dag_period_found: latest_dag_period.is_some(),
            latest_dag_period: latest_dag_period.unwrap_or_default(),
        })
    }

    /// Returns the sortition params change active at or before `period`.
    ///
    /// This is the public-query route for the Test RPC `get_sortition_change`.
    /// It reads the Rust metadata repository directly and decodes the canonical
    /// C++-compatible sortition change payload rather than exposing `DbStorage`
    /// or the broader sortition manager to public RPC code.
    pub fn sortition_params_change_by_period(
        &self,
        period: u64,
    ) -> Result<SortitionParamsChangeView> {
        let Some(raw_change) = self
            .storage
            .metadata()
            .params_change_for_period_rlp(period)?
        else {
            return Ok(SortitionParamsChangeView::default());
        };
        let change = SortitionParamsChange::from_rlp_bytes(&raw_change)
            .context("CONSENSUS_QUERY_SORTITION_PARAMS_CHANGE_DECODE")?;
        Ok(SortitionParamsChangeView {
            found: true,
            period: change.period,
            interval_efficiency: change.interval_efficiency,
            threshold_upper: change.threshold_upper,
            threshold_upper_min: THRESHOLD_UPPER_MIN_VALUE,
        })
    }

    /// Returns finalized block numbers whose indexed bloom contains `bloom`.
    ///
    /// The query follows the Rust FinalChain bloom-index traversal but keeps the
    /// public facade storage-only: missing chunks decode as zero chunks,
    /// malformed chunks are errors, and the range is inclusive.
    pub fn final_chain_blocks_with_bloom(
        &self,
        bloom: [u8; 256],
        from: u64,
        to: u64,
    ) -> Result<Vec<u64>> {
        if from > to {
            return Ok(Vec::new());
        }

        let root_level = FINAL_CHAIN_BLOOM_INDEX_LEVELS - 1;
        let root_units = final_chain_bloom_index_units(FINAL_CHAIN_BLOOM_INDEX_LEVELS)?;
        let first_index = from / root_units;
        let last_index = to / root_units + u64::from(!to.is_multiple_of(root_units));
        let mut result = Vec::new();
        for index in first_index..=last_index {
            self.final_chain_blocks_with_bloom_at(
                &bloom.into(),
                from,
                to,
                root_level,
                index,
                &mut result,
            )?;
        }
        Ok(result)
    }

    fn final_chain_blocks_with_bloom_at(
        &self,
        bloom: &FinalChainLogBloom,
        from: u64,
        to: u64,
        level: u64,
        index: u64,
        result: &mut Vec<u64>,
    ) -> Result<()> {
        let course_units = final_chain_bloom_index_units(level + 1)?;
        let fine_units = final_chain_bloom_index_units(level)?;
        let range_start = index
            .checked_mul(course_units)
            .ok_or_else(|| anyhow::anyhow!("CONSENSUS_QUERY_BLOOM_RANGE_OVERFLOW"))?;
        let range_end = index
            .checked_add(1)
            .and_then(|value| value.checked_mul(course_units))
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| anyhow::anyhow!("CONSENSUS_QUERY_BLOOM_RANGE_OVERFLOW"))?;
        if range_end < from || range_start > to {
            return Ok(());
        }

        let offset_begin = if from > range_start {
            (from - range_start) / fine_units
        } else {
            0
        };
        let offset_end = if to < range_end {
            (to - range_start) / fine_units + 1
        } else {
            FINAL_CHAIN_BLOOM_INDEX_SIZE as u64
        };
        let chunk_id = final_chain_log_bloom_chunk_id(level, index)
            .context("CONSENSUS_QUERY_BLOOM_CHUNK_ID")?;
        let raw = self.storage.final_chain().log_blooms_chunk_raw(chunk_id)?;
        let chunk = decode_final_chain_log_bloom_chunk(raw.as_deref())
            .context("CONSENSUS_QUERY_BLOOM_CHUNK")?;
        for offset in offset_begin..offset_end {
            let slot = usize::try_from(offset).context("CONSENSUS_QUERY_BLOOM_SLOT")?;
            if !log_bloom_contains(&chunk[slot], bloom) {
                continue;
            }
            let child_index = offset
                .checked_add(
                    index
                        .checked_mul(FINAL_CHAIN_BLOOM_INDEX_SIZE as u64)
                        .ok_or_else(|| {
                            anyhow::anyhow!("CONSENSUS_QUERY_BLOOM_CHILD_INDEX_OVERFLOW")
                        })?,
                )
                .ok_or_else(|| anyhow::anyhow!("CONSENSUS_QUERY_BLOOM_CHILD_INDEX_OVERFLOW"))?;
            if level > 0 {
                self.final_chain_blocks_with_bloom_at(
                    bloom,
                    from,
                    to,
                    level - 1,
                    child_index,
                    result,
                )?;
            } else if child_index >= from && child_index <= to {
                result.push(child_index);
            }
        }
        Ok(())
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
        let hash = keccak256(&block_rlp).into();

        Ok(DagBlockView {
            found: true,
            pivot: block.pivot.into(),
            level: block.level,
            tips: block.tips.into_iter().map(Into::into).collect(),
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            trx_estimations: block.gas_estimation,
            signature: block.signature.to_vec(),
            block_rlp,
            hash,
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
        let regular_count = self.storage.transaction().count(block_number)?;
        if u64::from(transaction_index) >= self.transaction_count_by_block_number(block_number)? {
            return Ok(TransactionView::default());
        }
        let (hash, source, is_system, transaction_rlp) =
            if u64::from(transaction_index) < regular_count {
                let Some(transaction_rlp) = self
                    .storage
                    .transaction()
                    .by_period_position_rlp(block_number, transaction_index)
                    .context("CONSENSUS_QUERY_TRANSACTION_INDEX_PAYLOAD")?
                else {
                    return Ok(TransactionView::default());
                };
                (
                    keccak256(&transaction_rlp).0,
                    STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR,
                    false,
                    transaction_rlp,
                )
            } else {
                let system_position = usize::try_from(u64::from(transaction_index) - regular_count)
                    .context("CONSENSUS_QUERY_SYSTEM_TRANSACTION_INDEX")?;
                let hash = *self
                    .system_transaction_hashes(block_number)?
                    .get(system_position)
                    .context("CONSENSUS_QUERY_SYSTEM_TRANSACTION_HASH_INDEX")?;
                let Some(transaction_rlp) = self
                    .storage
                    .transaction()
                    .system_rlp(hash)
                    .context("CONSENSUS_QUERY_SYSTEM_TRANSACTION_PAYLOAD")?
                else {
                    anyhow::bail!("CONSENSUS_QUERY_SYSTEM_TRANSACTION_MISSING");
                };
                anyhow::ensure!(
                    keccak256(&transaction_rlp) == hash,
                    "CONSENSUS_QUERY_SYSTEM_TRANSACTION_HASH_MISMATCH"
                );
                (
                    hash.0,
                    STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM,
                    true,
                    transaction_rlp,
                )
            };

        Ok(TransactionView {
            found: true,
            hash,
            source,
            location_found: true,
            block_number,
            transaction_index,
            is_system,
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

    /// Returns the finalized transaction count for a block number.
    ///
    /// Unknown blocks return zero, matching ETH RPC transaction-count behavior.
    /// The count includes regular period-data transactions followed by the
    /// persisted system-transaction hash list, without exposing a `FinalChain`
    /// object or transaction vector to public adapters.
    pub fn transaction_count_by_block_number(&self, block_number: u64) -> Result<u64> {
        if self
            .storage
            .final_chain()
            .block_hash_by_number(block_number)?
            .is_none()
        {
            return Ok(0);
        }
        let regular = self.storage.transaction().count(block_number)?;
        let system = self.system_transaction_hashes(block_number)?.len() as u64;
        regular
            .checked_add(system)
            .context("CONSENSUS_QUERY_TRANSACTION_COUNT_OVERFLOW")
    }

    fn system_transaction_hashes(&self, block_number: u64) -> Result<Vec<H256>> {
        let hashes_rlp = self
            .storage
            .transaction()
            .period_system_hashes_rlp(block_number)
            .context("CONSENSUS_QUERY_SYSTEM_TRANSACTION_HASHES")?;
        if hashes_rlp.is_empty() {
            return Ok(Vec::new());
        }
        let hashes = Rlp::new(&hashes_rlp);
        (0..hashes.item_count()?)
            .map(|index| {
                h256_bytes(
                    hashes
                        .at(index)
                        .context("CONSENSUS_QUERY_SYSTEM_TRANSACTION_HASH_INDEX")?
                        .data()
                        .context("CONSENSUS_QUERY_SYSTEM_TRANSACTION_HASH")?,
                )
            })
            .collect()
    }

    /// Returns the finalized transaction count for a block hash.
    ///
    /// Missing block-hash rows return zero. The block-number translation stays
    /// inside the query facade so ETH RPC callers do not read `FinalChain`
    /// indexes directly in Rust mode.
    pub fn transaction_count_by_block_hash(&self, block_hash: [u8; 32]) -> Result<u64> {
        let Some(block_number_bytes) = self
            .storage
            .final_chain()
            .block_number_by_hash(H256::from(block_hash))?
        else {
            return Ok(0);
        };
        let block_number = decode_u64_le(
            &block_number_bytes,
            "CONSENSUS_QUERY_TRANSACTION_COUNT_BLOCK_NUMBER",
        )?;
        self.transaction_count_by_block_number(block_number)
    }

    fn transaction_receipt_for_transaction(
        &self,
        transaction: TransactionView,
    ) -> Result<TransactionReceiptView> {
        if !transaction.found || !transaction.location_found {
            return Ok(TransactionReceiptView::default());
        }

        let transaction_hash = H256::from(transaction.hash);
        let receipt_rlp = match self
            .receipt_by_period_position(transaction.block_number, transaction.transaction_index)
            .context("CONSENSUS_QUERY_TRANSACTION_RECEIPT_BY_PERIOD")?
        {
            Some(receipt_rlp) => Some(receipt_rlp),
            None => self
                .storage
                .final_chain()
                .receipt_by_trx_hash(transaction_hash)
                .context("CONSENSUS_QUERY_TRANSACTION_RECEIPT_BY_HASH")?,
        };
        let Some(receipt_rlp) = receipt_rlp else {
            return Ok(TransactionReceiptView::default());
        };

        Ok(TransactionReceiptView {
            found: true,
            transaction_hash: transaction.hash,
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
        self.transaction_receipt_for_transaction(transaction)
    }

    /// Returns all public regular and system transaction receipt views for a finalized block number.
    ///
    /// Unknown blocks return an empty vector, matching `eth_getBlockReceipts`
    /// empty-array behavior. For known finalized blocks, the query resolves the
    /// Rust-owned transaction count, indexed transaction payloads, and receipt
    /// rows inside the facade so public RPC code does not call `FinalChain`
    /// directly for block receipt expansion. Missing transaction or receipt rows
    /// in a known block are reported as storage-consistency errors.
    pub fn transaction_receipts_by_block_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<TransactionReceiptView>> {
        let count = self.transaction_count_by_block_number(block_number)?;
        let mut receipts = Vec::with_capacity(count as usize);
        for transaction_index in 0..count {
            let transaction =
                self.transaction_by_block_number_and_index(block_number, transaction_index)?;
            if !transaction.found {
                anyhow::bail!("CONSENSUS_QUERY_BLOCK_RECEIPT_TRANSACTION_MISSING");
            }
            let receipt = self.transaction_receipt_for_transaction(transaction)?;
            if !receipt.found {
                anyhow::bail!("CONSENSUS_QUERY_BLOCK_RECEIPT_MISSING");
            }
            receipts.push(receipt);
        }
        Ok(receipts)
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

fn final_chain_bloom_index_units(level_count: u64) -> Result<u64> {
    let mut units = 1u64;
    for _ in 0..level_count {
        units = units
            .checked_mul(FINAL_CHAIN_BLOOM_INDEX_SIZE as u64)
            .ok_or_else(|| anyhow::anyhow!("CONSENSUS_QUERY_BLOOM_INDEX_UNIT_OVERFLOW"))?;
    }
    Ok(units)
}

fn log_bloom_contains(stored: &FinalChainLogBloom, query: &FinalChainLogBloom) -> bool {
    stored
        .as_ref()
        .iter()
        .zip(query.as_ref())
        .all(|(stored, query)| stored & query == *query)
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

fn pbft_period_cert_votes_view_from_bundle(
    requested_period: u64,
    votes_bundle_rlp: &Rlp<'_>,
) -> Result<PbftPeriodCertVotesView> {
    let item_count = votes_bundle_rlp
        .item_count()
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_BUNDLE_SHAPE")?;
    anyhow::ensure!(
        item_count == PBFT_VOTES_BUNDLE_FIELDS,
        "CONSENSUS_QUERY_PERIOD_CERT_VOTES_BUNDLE_SHAPE"
    );

    let block_hash = votes_bundle_rlp
        .val_at::<H256>(PBFT_VOTES_BUNDLE_BLOCK_HASH_POS)
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_BLOCK_HASH")?;
    let certified_period = votes_bundle_rlp
        .val_at::<u64>(PBFT_VOTES_BUNDLE_PERIOD_POS)
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_PERIOD")?;
    let round = votes_bundle_rlp
        .val_at::<u64>(PBFT_VOTES_BUNDLE_ROUND_POS)
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_ROUND")?;
    let step = votes_bundle_rlp
        .val_at::<u64>(PBFT_VOTES_BUNDLE_STEP_POS)
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_STEP")?;
    anyhow::ensure!(
        step == PBFT_CERT_VOTE_STEP,
        "CONSENSUS_QUERY_PERIOD_CERT_VOTES_NON_CERT_STEP"
    );

    let optimized_votes = votes_bundle_rlp
        .at(PBFT_VOTES_BUNDLE_VOTES_POS)
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_LIST")?;
    let vote_count = optimized_votes
        .item_count()
        .context("CONSENSUS_QUERY_PERIOD_CERT_VOTES_LIST_SHAPE")?;
    if vote_count == 0 {
        return Ok(PbftPeriodCertVotesView::default());
    }

    let mut votes = Vec::with_capacity(vote_count);
    for optimized_vote in optimized_votes.iter() {
        let proof = optimized_vote
            .at(PBFT_OPTIMIZED_VOTE_PROOF_POS)
            .context("CONSENSUS_QUERY_PERIOD_CERT_VOTE_PROOF")?
            .data()
            .context("CONSENSUS_QUERY_PERIOD_CERT_VOTE_PROOF_BYTES")?
            .to_vec();
        let signature = optimized_vote
            .at(PBFT_OPTIMIZED_VOTE_SIGNATURE_POS)
            .context("CONSENSUS_QUERY_PERIOD_CERT_VOTE_SIGNATURE")?
            .data()
            .context("CONSENSUS_QUERY_PERIOD_CERT_VOTE_SIGNATURE_BYTES")?
            .to_vec();

        let mut sortition = RlpStream::new_list(4);
        sortition.append(&certified_period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&proof);
        let sortition_rlp = sortition.out().to_vec();

        let mut vote = RlpStream::new_list(3);
        vote.append(&block_hash);
        vote.append(&sortition_rlp);
        vote.append(&signature);
        votes.push(PbftCertVoteRlp {
            vote_rlp: vote.out().to_vec(),
        });
    }

    Ok(PbftPeriodCertVotesView {
        found: true,
        period: requested_period,
        certified_period,
        round,
        step,
        block_hash: block_hash.into(),
        votes,
    })
}

/// Decodes previous-block certificate votes from canonical `PeriodData` bytes.
///
/// Startup recovery uses this crate-private projection to feed strict native
/// vote validation before FinalChain replay. Empty bundles return an empty
/// vector; malformed shapes fail before any replay effect is dispatched.
pub(crate) fn pbft_cert_votes_from_period_data_bytes(
    requested_period: u64,
    period_data_rlp: &[u8],
) -> Result<Vec<Vec<u8>>> {
    let period_data = Rlp::new(period_data_rlp);
    let bundle = period_data
        .at(CERT_VOTES_POS_IN_PERIOD_DATA)
        .context("CONSENSUS_STARTUP_PERIOD_CERT_VOTES")?;
    if bundle
        .item_count()
        .context("CONSENSUS_STARTUP_PERIOD_CERT_VOTES_SHAPE")?
        == 0
    {
        return Ok(Vec::new());
    }
    Ok(
        pbft_period_cert_votes_view_from_bundle(requested_period, &bundle)?
            .votes
            .into_iter()
            .map(|vote| vote.vote_rlp)
            .collect(),
    )
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
        block_rlp: block_rlp.to_vec(),
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
        epoch: block_data.pillar_block.epoch.to_big_endian(),
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

    fn hash_list_rlp(hashes: &[H256]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(hashes.len());
        for hash in hashes {
            stream.append(&hash.as_bytes());
        }
        stream.out().to_vec()
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
            log_bloom: [0xAA; 256].into(),
            gas_used: 55.into(),
            total_reward: rustaxa_types::DposTokenAmount::from(U256::from(66u64)),
        })
        .into_vec()
    }

    #[test]
    fn storage_only_query_rejects_live_final_chain_dpos_reads() {
        let (path, storage) = test_storage("dpos_requires_live_root");
        let api = ConsensusQueryApi::new(storage);

        let error = api
            .final_chain_dpos_eligible_total_vote_count(0)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("CONSENSUS_QUERY_LIVE_FINAL_CHAIN_UNAVAILABLE")
        );
        assert!(
            api.final_chain_dpos_eligible_vote_count(0, [0; 20])
                .is_err()
        );
        assert!(api.final_chain_dpos_validators_total_stakes(0).is_err());
        assert!(api.final_chain_dpos_total_amount_delegated(0).is_err());
        assert!(api.final_chain_dpos_yield(0).is_err());
        assert!(api.final_chain_dpos_total_supply(0).is_err());

        drop(api);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_reads_final_chain_block_view_and_pbft_hash_from_storage() {
        let (path, storage) = test_storage("block_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_hash = H256::from_low_u64_be(77);
        let pbft_block_rlp = vec![0xC2, 0x01, 0x02];
        let query_bloom = {
            let mut bloom = [0u8; 256];
            bloom[255] = 0x80;
            bloom
        };
        let mut root_chunk = rustaxa_storage::zero_final_chain_log_bloom_chunk();
        root_chunk[0] = query_bloom.into();
        let mut leaf_chunk = rustaxa_storage::zero_final_chain_log_bloom_chunk();
        leaf_chunk[9] = query_bloom.into();
        storage
            .period()
            .write(9, &period_data_rlp(&pbft_block_rlp))
            .unwrap();
        storage
            .final_chain()
            .write_block_header(9, block_hash, &stored_header_rlp(), &[])
            .unwrap();
        storage
            .final_chain()
            .write_conformance_lookup_rows(
                1,
                &9u64.to_le_bytes(),
                9,
                block_hash,
                &stored_header_rlp(),
                H256::zero(),
                &[0xC0],
                rustaxa_storage::final_chain_log_bloom_chunk_id(1, 0).unwrap(),
                &rustaxa_storage::encode_final_chain_log_bloom_chunk(&root_chunk),
                9,
                &[0xC0],
            )
            .unwrap();
        storage
            .final_chain()
            .write_conformance_lookup_rows(
                1,
                &9u64.to_le_bytes(),
                9,
                block_hash,
                &stored_header_rlp(),
                H256::zero(),
                &[0xC0],
                rustaxa_storage::final_chain_log_bloom_chunk_id(0, 0).unwrap(),
                &rustaxa_storage::encode_final_chain_log_bloom_chunk(&leaf_chunk),
                9,
                &[0xC0],
            )
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
        assert_eq!(api.final_chain_last_block_number().unwrap(), 9);
        storage.metadata().write_period_lambda(9, 1234).unwrap();
        let sortition_change = SortitionParamsChange {
            period: 8,
            interval_efficiency: 4_200,
            threshold_upper: 1_234,
        };
        storage
            .metadata()
            .write_sortition_params_change(8, &sortition_change.to_rlp_bytes())
            .unwrap();
        let status_dag_block_rlp = dag_block_rlp();
        let status_dag_block_hash = keccak256(&status_dag_block_rlp);
        storage
            .dag()
            .write(status_dag_block_hash, 5, 1, &status_dag_block_rlp)
            .unwrap();
        storage.dag().write_proposal_period_at_level(5, 8).unwrap();
        storage
            .metadata()
            .write_status_field(StatusField::ExecutedBlkCount as u8, 21)
            .unwrap();
        storage
            .metadata()
            .write_status_field(StatusField::ExecutedTrxCount as u8, 34)
            .unwrap();
        storage
            .metadata()
            .write_status_field(StatusField::DagBlkCount as u8, 55)
            .unwrap();
        storage
            .metadata()
            .write_status_field(StatusField::TrxCount as u8, 89)
            .unwrap();
        assert_eq!(
            api.period_lambda_by_period(9).unwrap(),
            QueryPeriodLambda {
                found: true,
                value: 1234
            }
        );
        assert!(!api.period_lambda_by_period(10).unwrap().found);
        assert_eq!(
            api.proposal_period_for_dag_level(5).unwrap(),
            QueryNumberLookup {
                found: true,
                value: 8
            }
        );
        assert!(!api.proposal_period_for_dag_level(6).unwrap().found);
        assert_eq!(
            api.sortition_params_change_by_period(8).unwrap(),
            SortitionParamsChangeView {
                found: true,
                period: 8,
                interval_efficiency: 4_200,
                threshold_upper: 1_234,
                threshold_upper_min: THRESHOLD_UPPER_MIN_VALUE,
            }
        );
        assert_eq!(
            api.sortition_params_change_by_period(9).unwrap(),
            SortitionParamsChangeView {
                found: true,
                period: 8,
                interval_efficiency: 4_200,
                threshold_upper: 1_234,
                threshold_upper_min: THRESHOLD_UPPER_MIN_VALUE,
            }
        );
        assert!(!api.sortition_params_change_by_period(7).unwrap().found);
        storage
            .metadata()
            .write_sortition_params_change(10, &[0xC1])
            .unwrap();
        assert!(api.sortition_params_change_by_period(10).is_err());
        assert_eq!(
            api.chain_stats().unwrap(),
            ChainStatsView {
                pbft_period: 9,
                non_empty_pbft_periods: 9,
                dag_blocks_count: 55,
                transactions_count: 89,
                dag_blocks_executed: 21,
                transactions_executed: 34
            }
        );
        assert_eq!(
            api.consensus_status().unwrap(),
            ConsensusStatusView {
                final_block_number: 9,
                latest_dag_level: 5,
                latest_dag_period_found: true,
                latest_dag_period: 8,
            }
        );
        assert_eq!(
            api.final_chain_block_number_by_hash(block_hash.into())
                .unwrap(),
            QueryNumberLookup {
                found: true,
                value: 9
            }
        );
        assert!(
            !api.final_chain_block_number_by_hash([0x99; 32])
                .unwrap()
                .found
        );
        assert_eq!(
            api.final_chain_blocks_with_bloom(query_bloom, 1, 9)
                .unwrap(),
            vec![9]
        );
        assert!(
            api.final_chain_blocks_with_bloom([0x11; 256], 1, 9)
                .unwrap()
                .is_empty()
        );

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
        assert!(!api.sortition_params_change_by_period(44).unwrap().found);

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
            epoch: 13.into(),
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
        assert_eq!(view.epoch, U256::from(13).to_big_endian());
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
        assert_eq!(view.block_rlp, block_rlp);
        assert!(view.finalized_period_found);
        assert_eq!(view.finalized_period, 9);
        assert_eq!(view.vdf_proof, vec![0x11; 80]);
        assert_eq!(view.vdf_sol1, vec![0x22, 0x23]);
        assert_eq!(view.vdf_sol2, vec![0x33, 0x34]);
        assert_eq!(view.vdf_difficulty, 7);

        let level_views = api.dag_blocks_by_level(5, 1).unwrap();
        assert_eq!(level_views.len(), 1);
        assert_eq!(level_views[0].hash, block_hash.0);
        assert_eq!(level_views[0].block_rlp, block_rlp);

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
        let system_rlp = vec![0x44];
        let system_hash = keccak256(&system_rlp);

        storage
            .period()
            .write(
                12,
                &period_data_with_transactions_rlp(&[first_rlp.clone(), second_rlp.clone()]),
            )
            .expect("period transaction payloads should persist");
        storage
            .transaction()
            .write_system(system_hash, &system_rlp)
            .expect("system transaction payload should persist");
        storage
            .transaction()
            .write_period_system_hashes(12, &hash_list_rlp(&[system_hash]))
            .expect("system transaction index should persist");
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
        let system = api
            .transaction_by_block_number_and_index(12, 2)
            .expect("system transaction index query should succeed");
        assert!(system.found);
        assert_eq!(system.hash, system_hash.0);
        assert_eq!(system.source, STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM);
        assert!(system.is_system);
        assert_eq!(system.transaction_rlp, system_rlp);
        assert_eq!(api.transaction_count_by_block_number(12).unwrap(), 3);
        assert_eq!(
            api.transaction_count_by_block_hash(block_hash.0).unwrap(),
            3
        );
        assert_eq!(api.transaction_count_by_block_number(99).unwrap(), 0);
        assert_eq!(api.transaction_count_by_block_hash([0x99; 32]).unwrap(), 0);

        assert!(
            !api.transaction_by_block_number_and_index(12, 3)
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
    fn query_api_system_only_index_is_canonical_and_corruption_fails_closed() {
        let (path, storage) = test_storage("system_only_transaction_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_hash = H256::from_low_u64_be(0x31);
        let system_rlp = vec![0x55];
        let system_hash = keccak256(&system_rlp);
        storage
            .period()
            .write(14, &period_data_with_transactions_rlp(&[]))
            .unwrap();
        storage
            .transaction()
            .write_system(system_hash, &system_rlp)
            .unwrap();
        storage
            .transaction()
            .write_period_system_hashes(14, &hash_list_rlp(&[system_hash]))
            .unwrap();
        storage
            .final_chain()
            .write_conformance_lookup_rows(
                0,
                b"meta",
                14,
                block_hash,
                &[0xC0],
                H256::zero(),
                &[0xC0],
                H256::zero(),
                &[0xC0],
                14,
                &receipt_list_rlp(&[]),
            )
            .unwrap();

        assert_eq!(api.transaction_count_by_block_number(14).unwrap(), 1);
        let view = api.transaction_by_block_number_and_index(14, 0).unwrap();
        assert!(view.found);
        assert!(view.is_system);
        assert_eq!(view.hash, system_hash.0);
        assert_eq!(view.transaction_rlp, system_rlp);

        storage
            .transaction()
            .write_period_system_hashes(14, &[0xC1, 0x01])
            .unwrap();
        assert!(api.transaction_count_by_block_number(14).is_err());

        let missing_hash = H256::from_low_u64_be(0x99);
        storage
            .transaction()
            .write_period_system_hashes(14, &hash_list_rlp(&[missing_hash]))
            .unwrap();
        assert!(api.transaction_by_block_number_and_index(14, 0).is_err());

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
        let system_rlp = vec![0x33];
        let system_hash = keccak256(&system_rlp);
        let fallback_trx_rlp = vec![0x32];
        let receipt_rlp = vec![0x41];
        let system_receipt_rlp = vec![0x43];
        let fallback_receipt_rlp = vec![0x42];

        storage
            .transaction()
            .write_location(trx_hash, 12, 0, false)
            .unwrap();
        storage
            .transaction()
            .write_location(system_hash, 12, 1, true)
            .unwrap();
        storage
            .transaction()
            .write_system(system_hash, &system_rlp)
            .unwrap();
        storage
            .transaction()
            .write_period_system_hashes(12, &hash_list_rlp(&[system_hash]))
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
                &receipt_list_rlp(&[receipt_rlp.clone(), system_receipt_rlp.clone()]),
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

        let block_receipts = api.transaction_receipts_by_block_number(12).unwrap();
        assert_eq!(block_receipts.len(), 2);
        assert_eq!(block_receipts[0].transaction_hash, keccak256(&trx_rlp).0);
        assert_eq!(block_receipts[0].transaction_rlp, trx_rlp);
        assert_eq!(block_receipts[0].receipt_rlp, receipt_rlp);
        assert_eq!(block_receipts[0].block_number, 12);
        assert_eq!(block_receipts[0].transaction_index, 0);
        assert_eq!(block_receipts[0].block_hash, block_hash.0);
        assert_eq!(block_receipts[1].transaction_hash, system_hash.0);
        assert_eq!(
            block_receipts[1].transaction_source,
            STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM
        );
        assert_eq!(block_receipts[1].transaction_rlp, system_rlp);
        assert_eq!(block_receipts[1].receipt_rlp, system_receipt_rlp);
        assert_eq!(block_receipts[1].transaction_index, 1);
        assert!(block_receipts[1].is_system);
        assert!(
            api.transaction_receipts_by_block_number(99)
                .unwrap()
                .is_empty()
        );

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

    #[test]
    fn query_api_reads_pbft_cert_vote_rlp_view_from_period_data() {
        let (path, storage) = test_storage("pbft_cert_votes_by_period");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_hash = H256::from_low_u64_be(42);
        let proof = vec![0x77; 80];
        let signature = vec![0x11; 65];
        let optimized_vote = optimized_vote_rlp(&proof, &signature);
        let vote_rlp = canonical_pbft_vote_rlp(12, 3, 3, block_hash, &proof, &signature);
        let period_data = period_data_with_cert_votes_rlp(
            block_hash,
            12,
            3,
            3,
            std::slice::from_ref(&optimized_vote),
        );
        storage.period().write(13, &period_data).unwrap();

        let view = api.pbft_previous_block_cert_votes_by_period(13).unwrap();
        assert!(view.found);
        assert_eq!(view.block_hash, block_hash.0);
        assert_eq!(view.period, 13);
        assert_eq!(view.certified_period, 12);
        assert_eq!(view.round, 3);
        assert_eq!(view.step, 3);
        assert_eq!(view.votes.len(), 1);
        assert_eq!(view.votes[0].vote_rlp, vote_rlp);
        assert_eq!(
            pbft_cert_votes_from_period_data_bytes(13, &period_data).unwrap(),
            vec![vote_rlp]
        );
        assert!(
            pbft_cert_votes_from_period_data_bytes(1, &period_data_rlp_with_dag_bundle(&[0xc0]))
                .unwrap()
                .is_empty()
        );
        assert!(
            !api.pbft_previous_block_cert_votes_by_period(14)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
