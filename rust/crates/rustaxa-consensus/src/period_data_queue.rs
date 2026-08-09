//! Period-data sync queue metadata for PBFT rewrite mode.
//!
//! This module models the deterministic queue contract used while syncing PBFT
//! period data from peers. It owns canonical encoded period payloads, fixed peer
//! identities, compact validation facts, effective processable size, and
//! pop/cleanup decisions. C++ materializes legacy objects only after pop while
//! the remaining execution boundary is migrated.

use anyhow::{Context, Result, anyhow, bail, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_types::codec::rlp::dag::{DagBlockRlp, FinalizedDagBlockBundleRlp};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use rustaxa_types::{
    DagBlock, LegacyTransactionEnvelope, decode_optimized_pillar_votes_bundle_rlp,
};
use rustaxa_vdf::vdf_sortition::decode_vdf_sortition_payload;
use std::collections::{HashSet, VecDeque};
use tiny_keccak::{Hasher, Keccak};

use crate::pbft_vote_validation::inspect_canonical_pbft_vote;

/// Metadata for one queued period-data payload.
///
/// Inputs/outputs:
/// - `period_data_rlp`: canonical period-data RLP bytes for this entry.
/// - `source_peer_id`: 64-byte canonical peer id that provided this payload.
/// - `period`: PBFT period carried by that payload.
/// - `block_hash`: PBFT block hash carried by that payload.
/// - `prev_block_hash`: previous PBFT block hash carried by that payload.
/// - `pivot_hash`: pivot DAG block hash carried by that payload.
/// - `final_chain_hash`: final-chain hash carried by that payload's PBFT
///   block.
/// - `reward_vote_hashes`: reward-vote hashes referenced by that payload's
///   PBFT block.
/// - `pillar_vote_rlps`: canonical pillar-vote RLP payloads carried by the
///   synced period-data payload for Rust sync validation.
/// - `transaction_rlps`: canonical transaction payloads carried by the synced
///   period-data payload for finalization materialization.
/// - `previous_cert_vote_rlps`: canonical PBFT cert-vote payloads carried by
///   the synced period-data payload for the previous block.
/// - transaction hash lists: compact sync validation facts carried by the
///   payload.
/// - previous-cert-vote flags: compact vote sidecar facts used by sync
///   admission planning.
/// - `pillar_votes_present`: compact pillar sidecar presence used by sync
///   admission planning.
/// - extra-data flags: compact PBFT block extra-data facts used by sync
///   admission planning.
///
/// Invariants:
/// - entries are stored in insertion order and accepted only by PBFT sync
///   period rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueueEntryRef {
    pub period_data_rlp: Vec<u8>,
    pub source_peer_id: [u8; 64],
    pub period: u64,
    pub block_hash: H256,
    pub prev_block_hash: H256,
    pub pivot_hash: H256,
    pub final_chain_hash: H256,
    pub reward_vote_hashes: Vec<H256>,
    pub pillar_vote_rlps: Vec<Vec<u8>>,
    pub transaction_rlps: Vec<Vec<u8>>,
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
    pub dag_transaction_hashes: Vec<H256>,
    pub period_data_transaction_hashes: Vec<H256>,
    pub period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
    pub previous_cert_votes_present: bool,
    pub previous_cert_first_vote_has_weight: bool,
    pub pillar_votes_present: bool,
    pub extra_data_present: bool,
    pub extra_data_pillar_block_hash_present: bool,
}

/// Compact transaction identity retained for synced period-data transactions.
///
/// Inputs/outputs:
/// - `input_index`: original transaction-list index in the period data payload.
/// - `hash`: canonical transaction hash.
/// - `transaction_nonce`: declared transaction nonce as a 32-byte big-endian
///   U256 for CXX compatibility.
/// - `sender`: recovered transaction sender.
///
/// Invariants:
/// - Identities are ordered exactly like the period-data transaction list.
/// - Sender recovery and hash validation happen before this fact enters the
///   queue; malformed payloads must be rejected by the bridge/shim caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueueTransactionIdentity {
    pub input_index: u64,
    pub hash: H256,
    pub transaction_nonce: [u8; 32],
    pub sender: [u8; 20],
}

/// Complete native request for admitting one synced period-data payload.
///
/// `entry` carries the durable-domain payload facts, `max_pbft_size` is the
/// current PBFT-chain size used by admission arithmetic, and
/// `current_block_cert_vote_rlps` supplies the final-entry certificate source.
/// The request is consumed exactly once; rejected admission does not mutate
/// queue state, while arithmetic overflow is returned as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueuePushRequest {
    pub entry: PeriodDataQueueEntryRef,
    pub max_pbft_size: u64,
    pub current_block_cert_vote_rlps: Vec<Vec<u8>>,
}

/// Encoded queue-push input accepted from the temporary CXX sync executor.
///
/// Rust derives every deterministic block, DAG, transaction, pillar, and
/// presence fact from `period_data_rlp` before queue mutation. Previous and
/// current certificate votes remain explicit because the optimized legacy
/// period-data encoding does not retain vote weights and the current-block
/// certificate is not part of `PeriodData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPeriodDataQueuePushRequest {
    pub period_data_rlp: Vec<u8>,
    pub source_peer_id: [u8; 64],
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
    pub current_block_cert_vote_rlps: Vec<Vec<u8>>,
}

/// Fully validated encoded queue payload awaiting one native chain-head fact.
///
/// Payload decoding and signature recovery can complete outside the manager
/// lock. The manager then samples its composed PBFT-chain sibling and converts
/// this value into the mutation request under one serialization domain.
#[derive(Clone)]
pub(crate) struct DecodedPeriodDataQueuePush {
    pub(crate) entry: PeriodDataQueueEntryRef,
    pub(crate) current_block_cert_vote_rlps: Vec<Vec<u8>>,
}

/// Side-effect-free native view of one raw PBFT-sync packet.
///
/// The packet decoder retains the exact encoded `PeriodData` child, expands
/// optimized certificate bundles into canonical unweighted vote payloads, and
/// invokes the same strict decoder used immediately before queue mutation.
/// The calculated DAG order hash is derived from canonical reconstructed DAG
/// block bytes. No queue or verified-vote state is changed.
#[derive(Clone)]
pub(crate) struct DecodedPbftSyncPacketPrecheck {
    pub(crate) period_data: DecodedPeriodDataQueuePush,
    pub(crate) last_block: bool,
    pub(crate) max_dag_level: u64,
    pub(crate) current_cert_votes_present: bool,
    pub(crate) declared_order_hash: H256,
    pub(crate) calculated_order_hash: H256,
}

/// Decodes and validates the canonical outer PBFT-sync packet without mutation.
///
/// The wire shape must be exactly `[last_block, period_data,
/// current_cert_votes_or_empty]`. Both optimized certificate locations are
/// normalized to canonical three-field signed votes before the existing strict
/// period-data decoder is called. Malformed shape, signatures, or nested
/// payloads return a stable precheck/queue-prefixed error.
pub(crate) fn decode_pbft_sync_packet_precheck(
    packet_rlp: &[u8],
) -> Result<DecodedPbftSyncPacketPrecheck> {
    let packet = Rlp::new(packet_rlp);
    ensure_single_rlp_value(&packet, packet_rlp, "PBFT_SYNC_PACKET_OUTER")?;
    ensure_exact_list_items(&packet, 3, "PBFT_SYNC_PACKET_OUTER")?;
    let last_block: bool = packet.val_at(0).context("PBFT_SYNC_PACKET_LAST_BLOCK")?;
    let period_data = packet.at(1).context("PBFT_SYNC_PACKET_PERIOD_DATA")?;
    let period_data_rlp = period_data.as_raw().to_vec();

    let period = period_data
        .at(0)
        .context("PBFT_SYNC_PACKET_PERIOD_DATA_BLOCK")?
        .val_at::<u64>(4)
        .context("PBFT_SYNC_PACKET_PERIOD_DATA_PERIOD")?;
    let previous_cert_vote_rlps = if period > 1 {
        decode_optimized_cert_vote_bundle(
            &period_data
                .at(1)
                .context("PBFT_SYNC_PACKET_PREVIOUS_CERT_BUNDLE")?,
            "PBFT_SYNC_PACKET_PREVIOUS_CERT_BUNDLE",
        )?
    } else {
        let placeholder = period_data
            .at(1)
            .context("PBFT_SYNC_PACKET_PREVIOUS_CERT_PLACEHOLDER")?;
        ensure!(
            placeholder.as_raw() == [0x80],
            "PBFT_SYNC_PACKET_PREVIOUS_CERT_PLACEHOLDER"
        );
        Vec::new()
    };

    let current_bundle = packet
        .at(2)
        .context("PBFT_SYNC_PACKET_CURRENT_CERT_BUNDLE")?;
    let current_cert_votes_present = !current_bundle.is_empty();
    let current_block_cert_vote_rlps = if current_cert_votes_present {
        decode_optimized_cert_vote_bundle(&current_bundle, "PBFT_SYNC_PACKET_CURRENT_CERT_BUNDLE")?
    } else {
        ensure!(
            current_bundle
                .data()
                .map(|bytes| bytes.is_empty())
                .unwrap_or(false),
            "PBFT_SYNC_PACKET_CURRENT_CERT_BUNDLE"
        );
        Vec::new()
    };

    let decoded = decode_encoded_period_data_queue_push_with_vote_validation(
        EncodedPeriodDataQueuePushRequest {
            period_data_rlp,
            source_peer_id: [0; 64],
            previous_cert_vote_rlps,
            current_block_cert_vote_rlps,
        },
        false,
    )?;
    let block = period_data.at(0)?;
    let declared_order_hash = block.val_at(2).context("PBFT_SYNC_PACKET_ORDER_HASH")?;
    let calculated_order_hash = calculate_period_data_order_hash(&period_data)?;
    let max_dag_level = max_period_data_dag_level(&period_data)?;

    Ok(DecodedPbftSyncPacketPrecheck {
        period_data: decoded,
        last_block,
        max_dag_level,
        current_cert_votes_present,
        declared_order_hash,
        calculated_order_hash,
    })
}

fn max_period_data_dag_level(period_data: &Rlp<'_>) -> Result<u64> {
    let dag_bundle = period_data.at(2).context("PBFT_SYNC_PACKET_DAG_BUNDLE")?;
    if dag_bundle.is_empty() {
        return Ok(0);
    }
    let blocks = dag_bundle.at(2).context("PBFT_SYNC_PACKET_DAG_BLOCKS")?;
    let block_count = blocks.item_count().context("PBFT_SYNC_PACKET_DAG_BLOCKS")?;
    let bundle = FinalizedDagBlockBundleRlp::new(dag_bundle.as_raw());
    let mut max_level = 0;
    for position in 0..block_count {
        let canonical = bundle
            .canonical_block_rlp(position)
            .context("PBFT_SYNC_PACKET_DAG_BLOCK")?;
        let block = DagBlock::try_from(DagBlockRlp::new(&canonical))
            .context("PBFT_SYNC_PACKET_DAG_BLOCK")?;
        max_level = max_level.max(block.level);
    }
    Ok(max_level)
}

fn decode_optimized_cert_vote_bundle(bundle: &Rlp<'_>, field: &str) -> Result<Vec<Vec<u8>>> {
    ensure_exact_list_items(bundle, 5, field)?;
    let block_hash: H256 = bundle
        .val_at(0)
        .with_context(|| format!("{field}_BLOCK_HASH"))?;
    let period: u64 = bundle
        .val_at(1)
        .with_context(|| format!("{field}_PERIOD"))?;
    let round: u64 = bundle.val_at(2).with_context(|| format!("{field}_ROUND"))?;
    let step: u64 = bundle.val_at(3).with_context(|| format!("{field}_STEP"))?;
    let votes = bundle.at(4).with_context(|| format!("{field}_VOTES"))?;
    let vote_count = votes
        .item_count()
        .with_context(|| format!("{field}_VOTES"))?;
    ensure!(vote_count > 0, "{field}_VOTES_EMPTY");
    ensure_exact_list_items(&votes, vote_count, &format!("{field}_VOTES"))?;

    votes
        .iter()
        .enumerate()
        .map(|(index, optimized)| {
            ensure_exact_list_items(&optimized, 2, &format!("{field}_VOTE_{index}"))?;
            let proof = optimized
                .at(0)
                .and_then(|value| value.data())
                .with_context(|| format!("{field}_PROOF_{index}"))?;
            let signature = optimized
                .at(1)
                .and_then(|value| value.data())
                .with_context(|| format!("{field}_SIGNATURE_{index}"))?;
            ensure!(signature.len() == 65, "{field}_SIGNATURE_{index}");

            let mut sortition = RlpStream::new_list(4);
            sortition.append(&period);
            sortition.append(&round);
            sortition.append(&step);
            sortition.append(&proof);
            let mut vote = RlpStream::new_list(3);
            vote.append(&block_hash);
            vote.append(&sortition.out().as_ref());
            vote.append(&signature);
            Ok(vote.out().to_vec())
        })
        .collect()
}

fn calculate_period_data_order_hash(period_data: &Rlp<'_>) -> Result<H256> {
    let dag_bundle = period_data.at(2).context("PBFT_SYNC_PACKET_DAG_BUNDLE")?;
    if dag_bundle.is_empty() {
        return Ok(H256::zero());
    }
    let block_count = dag_bundle
        .at(2)
        .context("PBFT_SYNC_PACKET_DAG_BLOCKS")?
        .item_count()
        .context("PBFT_SYNC_PACKET_DAG_BLOCKS")?;
    let bundle = FinalizedDagBlockBundleRlp::new(dag_bundle.as_raw());
    let mut hashes = Vec::with_capacity(block_count);
    for position in 0..block_count {
        let canonical = bundle
            .canonical_block_rlp(position)
            .context("PBFT_SYNC_PACKET_DAG_BLOCK")?;
        hashes.push(keccak256(&canonical));
    }
    if hashes.is_empty() {
        return Ok(H256::zero());
    }
    let mut order = RlpStream::new_list(1);
    order.begin_list(hashes.len());
    for hash in hashes {
        order.append(&hash);
    }
    Ok(keccak256(&order.out()))
}

fn keccak256(bytes: &[u8]) -> H256 {
    let mut hasher = Keccak::v256();
    let mut output = [0_u8; 32];
    hasher.update(bytes);
    hasher.finalize(&mut output);
    output.into()
}

impl DecodedPeriodDataQueuePush {
    pub(crate) fn with_chain_size(self, max_pbft_size: u64) -> PeriodDataQueuePushRequest {
        PeriodDataQueuePushRequest {
            entry: self.entry,
            max_pbft_size,
            current_block_cert_vote_rlps: self.current_block_cert_vote_rlps,
        }
    }
}

/// Strictly decodes one encoded queue request into native queue facts.
///
/// All decoding and signature recovery completes before the returned request
/// can be passed to [`PeriodDataQueue::push`]. Malformed outer shapes, signed
/// blocks, finalized DAG indexes, transactions, pillar bundles, or vote
/// payloads return a stable queue-prefixed error without mutating queue state.
pub(crate) fn decode_encoded_period_data_queue_push(
    request: EncodedPeriodDataQueuePushRequest,
) -> Result<DecodedPeriodDataQueuePush> {
    decode_encoded_period_data_queue_push_with_vote_validation(request, true)
}

fn decode_encoded_period_data_queue_push_with_vote_validation(
    request: EncodedPeriodDataQueuePushRequest,
    validate_vote_signatures: bool,
) -> Result<DecodedPeriodDataQueuePush> {
    let period_data = Rlp::new(&request.period_data_rlp);
    ensure_single_rlp_value(
        &period_data,
        &request.period_data_rlp,
        "PBFT_PERIOD_DATA_QUEUE_OUTER",
    )?;
    let field_count = period_data
        .item_count()
        .context("PBFT_PERIOD_DATA_QUEUE_OUTER_SHAPE")?;
    ensure!(
        matches!(field_count, 4 | 5),
        "PBFT_PERIOD_DATA_QUEUE_OUTER_SHAPE"
    );
    ensure_exact_list_items(&period_data, field_count, "PBFT_PERIOD_DATA_QUEUE_OUTER")?;

    let block_rlp = period_data.at(0).context("PBFT_PERIOD_DATA_QUEUE_BLOCK")?;
    let block_field_count = block_rlp
        .item_count()
        .context("PBFT_PERIOD_DATA_QUEUE_BLOCK_SHAPE")?;
    ensure!(
        matches!(block_field_count, 8 | 9),
        "PBFT_PERIOD_DATA_QUEUE_BLOCK_SHAPE"
    );
    ensure_exact_list_items(
        &block_rlp,
        block_field_count,
        "PBFT_PERIOD_DATA_QUEUE_BLOCK",
    )?;
    ensure!(
        block_rlp
            .at(block_field_count - 1)
            .and_then(|signature| signature.data())
            .map(|signature| signature.len() == 65)
            .unwrap_or(false),
        "PBFT_PERIOD_DATA_QUEUE_BLOCK_SIGNATURE"
    );
    let block_bytes = block_rlp.as_raw();
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(block_bytes))
        .context("PBFT_PERIOD_DATA_QUEUE_BLOCK_DECODE")?;
    rustaxa_types::PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(block_bytes))
        .context("PBFT_PERIOD_DATA_QUEUE_BLOCK_SIGNATURE")?;
    let final_chain_hash: H256 = block_rlp
        .val_at(3)
        .context("PBFT_PERIOD_DATA_QUEUE_FINAL_CHAIN_HASH")?;
    let _: H256 = block_rlp
        .val_at(2)
        .context("PBFT_PERIOD_DATA_QUEUE_ORDER_HASH")?;
    let reward_votes = block_rlp
        .at(6)
        .context("PBFT_PERIOD_DATA_QUEUE_REWARD_VOTES")?;
    let reward_vote_count = reward_votes
        .item_count()
        .context("PBFT_PERIOD_DATA_QUEUE_REWARD_VOTES")?;
    ensure_exact_list_items(
        &reward_votes,
        reward_vote_count,
        "PBFT_PERIOD_DATA_QUEUE_REWARD_VOTES",
    )?;
    let reward_vote_hashes: Vec<H256> = block_rlp
        .list_at(6)
        .context("PBFT_PERIOD_DATA_QUEUE_REWARD_VOTES")?;
    ensure!(
        reward_vote_hashes
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            == reward_vote_hashes.len(),
        "PBFT_PERIOD_DATA_QUEUE_DUPLICATE_REWARD_VOTE"
    );

    let (extra_data_present, extra_data_pillar_block_hash_present) =
        decode_extra_data_presence(&block_rlp, block_field_count)?;
    let dag_transaction_hashes = decode_dag_transaction_hashes(
        &period_data
            .at(2)
            .context("PBFT_PERIOD_DATA_QUEUE_DAG_BUNDLE")?,
    )?;
    let decoded_transactions = decode_period_transactions(
        &period_data
            .at(3)
            .context("PBFT_PERIOD_DATA_QUEUE_TRANSACTIONS")?,
    )?;
    let pillar_vote_rlps = if field_count == 5 {
        let bundle = period_data
            .at(4)
            .context("PBFT_PERIOD_DATA_QUEUE_PILLAR_BUNDLE")?;
        ensure_exact_list_items(&bundle, 3, "PBFT_PERIOD_DATA_QUEUE_PILLAR_BUNDLE")?;
        let signatures = bundle
            .at(2)
            .context("PBFT_PERIOD_DATA_QUEUE_PILLAR_SIGNATURES")?;
        let signature_count = signatures
            .item_count()
            .context("PBFT_PERIOD_DATA_QUEUE_PILLAR_SIGNATURES")?;
        ensure_exact_list_items(
            &signatures,
            signature_count,
            "PBFT_PERIOD_DATA_QUEUE_PILLAR_SIGNATURES",
        )?;
        decode_optimized_pillar_votes_bundle_rlp(bundle.as_raw())
            .context("PBFT_PERIOD_DATA_QUEUE_PILLAR_BUNDLE")?
            .into_iter()
            .map(|vote| vote.encode_rlp())
            .collect()
    } else {
        Vec::new()
    };

    if link.period > 1 {
        validate_previous_cert_bundle(
            &period_data
                .at(1)
                .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_BUNDLE")?,
            &request.previous_cert_vote_rlps,
        )?;
    }
    if validate_vote_signatures {
        validate_cert_vote_rlps(&request.previous_cert_vote_rlps)?;
        validate_cert_vote_rlps(&request.current_block_cert_vote_rlps)?;
    }
    let previous_cert_votes_present = !request.previous_cert_vote_rlps.is_empty();
    let previous_cert_first_vote_has_weight = request
        .previous_cert_vote_rlps
        .first()
        .map(|vote| Rlp::new(vote).item_count() == Ok(4))
        .unwrap_or(false);

    Ok(DecodedPeriodDataQueuePush {
        entry: PeriodDataQueueEntryRef {
            period_data_rlp: request.period_data_rlp,
            source_peer_id: request.source_peer_id,
            period: link.period,
            block_hash: link.block_hash,
            prev_block_hash: link.prev_block_hash,
            pivot_hash: link.pivot_dag_block_hash,
            final_chain_hash,
            reward_vote_hashes,
            pillar_vote_rlps,
            transaction_rlps: decoded_transactions.rlps,
            previous_cert_vote_rlps: request.previous_cert_vote_rlps,
            dag_transaction_hashes,
            period_data_transaction_hashes: decoded_transactions.hashes,
            period_data_transaction_identities: decoded_transactions.identities,
            previous_cert_votes_present,
            previous_cert_first_vote_has_weight,
            pillar_votes_present: field_count == 5,
            extra_data_present,
            extra_data_pillar_block_hash_present,
        },
        current_block_cert_vote_rlps: request.current_block_cert_vote_rlps,
    })
}

fn decode_extra_data_presence(block: &Rlp<'_>, field_count: usize) -> Result<(bool, bool)> {
    if field_count != 9 {
        return Ok((false, false));
    }
    let bytes = block
        .at(7)
        .and_then(|value| value.data())
        .context("PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA")?;
    if bytes.len() > 1024 {
        bail!("PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA_TOO_LARGE");
    }
    let extra = Rlp::new(bytes);
    ensure_single_rlp_value(&extra, bytes, "PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA")?;
    ensure_exact_list_items(&extra, 6, "PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA")?;
    for index in 0..4 {
        extra
            .val_at::<u16>(index)
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA_VERSION_{index}"))?;
    }
    extra
        .at(4)
        .and_then(|value| value.data())
        .context("PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA_IMPLEMENTATION")?;
    let pillar_hash = extra
        .at(5)
        .and_then(|value| value.data())
        .context("PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA_PILLAR_HASH")?;
    ensure!(
        pillar_hash.is_empty() || pillar_hash.len() == 32,
        "PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA_PILLAR_HASH"
    );
    let pillar_present = pillar_hash.len() == 32;
    Ok((true, pillar_present))
}

fn decode_dag_transaction_hashes(bundle_rlp: &Rlp<'_>) -> Result<Vec<H256>> {
    if bundle_rlp.is_empty() {
        return Ok(Vec::new());
    }
    ensure!(
        bundle_rlp.item_count()? == 3,
        "PBFT_PERIOD_DATA_QUEUE_DAG_BUNDLE_SHAPE"
    );
    ensure_exact_list_items(bundle_rlp, 3, "PBFT_PERIOD_DATA_QUEUE_DAG_BUNDLE")?;
    validate_finalized_dag_bundle_shape(bundle_rlp)?;
    let block_count = bundle_rlp
        .at(2)
        .context("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCKS")?
        .item_count()?;
    let bundle = FinalizedDagBlockBundleRlp::new(bundle_rlp.as_raw());
    let mut hashes = Vec::new();
    for position in 0..block_count {
        let block = DagBlock::try_from(DagBlockRlp::new(
            &bundle
                .canonical_block_rlp(position)
                .context("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK")?,
        ))
        .context("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK")?;
        hashes.extend(block.transactions);
    }
    Ok(hashes)
}

struct DecodedPeriodTransactions {
    rlps: Vec<Vec<u8>>,
    hashes: Vec<H256>,
    identities: Vec<PeriodDataQueueTransactionIdentity>,
}

fn decode_period_transactions(transactions: &Rlp<'_>) -> Result<DecodedPeriodTransactions> {
    ensure!(
        transactions.is_list(),
        "PBFT_PERIOD_DATA_QUEUE_TRANSACTIONS_SHAPE"
    );
    let transaction_count = transactions
        .item_count()
        .context("PBFT_PERIOD_DATA_QUEUE_TRANSACTIONS_SHAPE")?;
    ensure_exact_list_items(
        transactions,
        transaction_count,
        "PBFT_PERIOD_DATA_QUEUE_TRANSACTIONS",
    )?;
    let mut rlps = Vec::new();
    let mut hashes = Vec::new();
    let mut identities = Vec::new();
    for (index, transaction) in transactions.iter().enumerate() {
        let bytes = transaction.as_raw().to_vec();
        ensure_single_rlp_value(
            &transaction,
            &bytes,
            &format!("PBFT_PERIOD_DATA_QUEUE_TRANSACTION_{index}"),
        )?;
        ensure_exact_list_items(
            &transaction,
            9,
            &format!("PBFT_PERIOD_DATA_QUEUE_TRANSACTION_{index}"),
        )?;
        let envelope = LegacyTransactionEnvelope::decode(&bytes)
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_TRANSACTION_{index}"))?;
        let sender = envelope
            .sender
            .ok_or_else(|| anyhow!("PBFT_PERIOD_DATA_QUEUE_TRANSACTION_SENDER_{index}"))?;
        let nonce = envelope.nonce.to_big_endian();
        rlps.push(bytes);
        hashes.push(envelope.hash);
        identities.push(PeriodDataQueueTransactionIdentity {
            input_index: index as u64,
            hash: envelope.hash,
            transaction_nonce: nonce,
            sender: sender.into(),
        });
    }
    Ok(DecodedPeriodTransactions {
        rlps,
        hashes,
        identities,
    })
}

fn validate_cert_vote_rlps(votes: &[Vec<u8>]) -> Result<()> {
    for (index, vote) in votes.iter().enumerate() {
        let vote_rlp = Rlp::new(vote);
        ensure_single_rlp_value(
            &vote_rlp,
            vote,
            &format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_{index}"),
        )?;
        let item_count = vote_rlp
            .item_count()
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_{index}"))?;
        ensure!(
            matches!(item_count, 3 | 4),
            "PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_SHAPE_{index}"
        );
        ensure_exact_list_items(
            &vote_rlp,
            item_count,
            &format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_{index}"),
        )?;
        let sortition_bytes = vote_rlp
            .at(1)
            .and_then(|value| value.data())
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_SORTITION_{index}"))?;
        let sortition = Rlp::new(sortition_bytes);
        ensure_single_rlp_value(
            &sortition,
            sortition_bytes,
            &format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_SORTITION_{index}"),
        )?;
        ensure_exact_list_items(
            &sortition,
            4,
            &format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_SORTITION_{index}"),
        )?;
        let inspection = inspect_canonical_pbft_vote(vote)
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_{index}"))?;
        ensure!(
            inspection.signature_valid,
            "PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_SIGNATURE_{index}"
        );
    }
    Ok(())
}

fn validate_previous_cert_bundle(bundle: &Rlp<'_>, votes: &[Vec<u8>]) -> Result<()> {
    ensure!(
        bundle.item_count()? == 5,
        "PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_BUNDLE_SHAPE"
    );
    ensure_exact_list_items(bundle, 5, "PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_BUNDLE")?;
    let block_hash: H256 = bundle
        .val_at(0)
        .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_BLOCK_HASH")?;
    let period: u64 = bundle
        .val_at(1)
        .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_PERIOD")?;
    let round: u64 = bundle
        .val_at(2)
        .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_ROUND")?;
    let step: u64 = bundle
        .val_at(3)
        .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_STEP")?;
    let optimized_votes = bundle
        .at(4)
        .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_VOTES")?;
    let vote_count = optimized_votes
        .item_count()
        .context("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_VOTES_SHAPE")?;
    ensure_exact_list_items(
        &optimized_votes,
        vote_count,
        "PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_VOTES",
    )?;
    ensure!(
        vote_count == votes.len(),
        "PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_VOTE_COUNT"
    );

    for (index, (optimized, canonical_bytes)) in
        optimized_votes.iter().zip(votes.iter()).enumerate()
    {
        ensure_exact_list_items(
            &optimized,
            2,
            &format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_OPTIMIZED_{index}"),
        )?;
        let proof = optimized
            .at(0)
            .and_then(|value| value.data())
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_PROOF_{index}"))?;
        let signature = optimized
            .at(1)
            .and_then(|value| value.data())
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_SIGNATURE_{index}"))?;

        let canonical = Rlp::new(canonical_bytes);
        let canonical_count = canonical
            .item_count()
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_VOTE_{index}"))?;
        ensure!(
            matches!(canonical_count, 3 | 4),
            "PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_VOTE_SHAPE_{index}"
        );
        let canonical_block_hash: H256 = canonical
            .val_at(0)
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_BLOCK_{index}"))?;
        let sortition_bytes = canonical
            .at(1)
            .and_then(|value| value.data())
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_SORTITION_{index}"))?;
        let sortition = Rlp::new(sortition_bytes);
        ensure_single_rlp_value(
            &sortition,
            sortition_bytes,
            &format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_SORTITION_{index}"),
        )?;
        ensure_exact_list_items(
            &sortition,
            4,
            &format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_SORTITION_{index}"),
        )?;
        let canonical_signature = canonical
            .at(2)
            .and_then(|value| value.data())
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_SIGNATURE_{index}"))?;
        ensure!(
            canonical_block_hash == block_hash
                && sortition.val_at::<u64>(0)? == period
                && sortition.val_at::<u64>(1)? == round
                && sortition.val_at::<u64>(2)? == step
                && sortition.at(3)?.data()? == proof
                && canonical_signature == signature,
            "PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_MISMATCH_{index}"
        );
    }
    Ok(())
}

fn validate_finalized_dag_bundle_shape(bundle: &Rlp<'_>) -> Result<()> {
    for (field, name) in [
        (0, "ORDERED_TRANSACTIONS"),
        (1, "TRANSACTION_INDEXES"),
        (2, "COMPACT_BLOCKS"),
    ] {
        let list = bundle
            .at(field)
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_DAG_{name}"))?;
        let count = list
            .item_count()
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_DAG_{name}"))?;
        ensure_exact_list_items(&list, count, &format!("PBFT_PERIOD_DATA_QUEUE_DAG_{name}"))?;
    }

    let ordered_transactions = bundle.at(0)?;
    for (position, transaction_hash) in ordered_transactions.iter().enumerate() {
        transaction_hash
            .as_val::<H256>()
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_DAG_TRANSACTION_{position}"))?;
    }

    let index_lists = bundle.at(1)?;
    let compact_blocks = bundle.at(2)?;
    ensure!(
        index_lists.item_count()? == compact_blocks.item_count()?,
        "PBFT_PERIOD_DATA_QUEUE_DAG_INDEX_BLOCK_COUNT"
    );
    for position in 0..compact_blocks.item_count()? {
        let indexes = index_lists.at(position)?;
        let index_count = indexes.item_count()?;
        ensure_exact_list_items(
            &indexes,
            index_count,
            &format!("PBFT_PERIOD_DATA_QUEUE_DAG_INDEXES_{position}"),
        )?;
        let block = compact_blocks.at(position)?;
        ensure_exact_list_items(
            &block,
            7,
            &format!("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_{position}"),
        )?;
        let vdf_bytes = block
            .at(3)
            .and_then(|value| value.data())
            .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_VDF_{position}"))?;
        if !vdf_bytes.is_empty() {
            let vdf = Rlp::new(vdf_bytes);
            ensure_single_rlp_value(
                &vdf,
                vdf_bytes,
                &format!("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_VDF_{position}"),
            )?;
            ensure_exact_list_items(
                &vdf,
                4,
                &format!("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_VDF_{position}"),
            )?;
            decode_vdf_sortition_payload(vdf_bytes)
                .with_context(|| format!("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_VDF_{position}"))?;
        }
        let tips = block.at(4)?;
        let tip_count = tips.item_count()?;
        ensure_exact_list_items(
            &tips,
            tip_count,
            &format!("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_TIPS_{position}"),
        )?;
    }
    Ok(())
}

fn ensure_single_rlp_value(rlp: &Rlp<'_>, encoded: &[u8], field: &str) -> Result<()> {
    let payload = rlp.payload_info()?;
    let encoded_len = payload
        .header_len
        .checked_add(payload.value_len)
        .ok_or_else(|| anyhow!("{field} RLP length overflows"))?;
    ensure!(
        encoded_len == encoded.len(),
        "{field} RLP has trailing bytes or incomplete payload"
    );
    Ok(())
}

fn ensure_exact_list_items(rlp: &Rlp<'_>, expected: usize, field: &str) -> Result<()> {
    ensure!(rlp.is_list(), "{field} must be an RLP list");
    let payload = rlp.payload_info()?;
    let mut decoded_len = 0usize;
    for index in 0..expected {
        let child = rlp.at(index)?;
        decoded_len = decoded_len
            .checked_add(child.as_raw().len())
            .ok_or_else(|| anyhow!("{field} child length overflows"))?;
    }
    ensure!(
        decoded_len == payload.value_len,
        "{field} must contain exactly {expected} complete items"
    );
    Ok(())
}

/// Coherent read-only view of Rust-owned period-data queue state.
///
/// The caller supplies the remaining PBFT-chain compatibility facts. The
/// snapshot derives all queue fields under the manager serialization lock and
/// never exposes the queue itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodDataQueueSnapshot {
    pub period: u64,
    pub syncing_period: u64,
    pub last_block_hash_or_chain: H256,
    pub size: usize,
    pub empty: bool,
}

/// Result of attempting to enqueue one period-data payload.
///
/// Inputs/outputs:
/// - `accepted`: true when the entry was appended to Rust queue metadata.
/// - `clear_existing`: true when Rust dropped old entries before adding the
///   accepted entry because the PBFT chain moved beyond queued state.
/// - period fields expose the legacy admission calculation for diagnostics and
///   bridge tests.
/// - `effective_size`: processable queue size after the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodDataQueuePushOutcome {
    pub accepted: bool,
    pub clear_existing: bool,
    pub expected_next_period: u64,
    pub actual_period: u64,
    pub current_period: u64,
    pub effective_size: usize,
}

/// Plan returned after popping queue metadata.
///
/// Inputs/outputs:
/// - `period_data_rlp`: canonical period-data bytes returned for temporary C++
///   executor materialization after Rust pops the entry.
/// - `source_peer_id`: 64-byte canonical peer id that supplied this payload.
/// - `cert_vote_rlps`: canonical PBFT cert-vote payloads selected by Rust for
///   the popped block. They are either the next queued entry's previous-cert
///   payloads or the current last-block cert-vote payloads.
/// - `use_last_block_cert_votes`: true when Rust selected the cert votes passed
///   with the last queued block; false means cert votes came from the next
///   queued entry.
/// - `current_period` and `effective_size` describe queue state after pop.
/// - `entry_period`, `block_hash`, `prev_block_hash`, `pivot_hash`, and
///   `final_chain_hash` are the compact PBFT block facts for the popped
///   payload.
/// - `reward_vote_hashes` are compact reward-vote references from the popped
///   PBFT block.
/// - `pillar_vote_rlps` are canonical pillar-vote payload bytes from the
///   popped period-data payload.
/// - `transaction_rlps` are canonical transaction payload bytes from the
///   popped period-data payload.
/// - `previous_cert_vote_rlps` are canonical cert-vote payload bytes from the
///   popped period-data payload's previous-cert sidecar.
/// - transaction hash lists are compact sync validation facts for the popped
///   payload.
/// - transaction identities are compact finalized-status facts for the popped
///   payload's transaction list.
/// - previous-cert-vote flags are compact vote sidecar facts for the popped
///   payload.
/// - `pillar_votes_present` is the compact pillar sidecar presence fact for
///   the popped payload.
/// - extra-data flags are compact PBFT block extra-data facts for the popped
///   payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueuePopPlan {
    pub period_data_rlp: Vec<u8>,
    pub source_peer_id: [u8; 64],
    pub entry_period: u64,
    pub block_hash: H256,
    pub prev_block_hash: H256,
    pub pivot_hash: H256,
    pub final_chain_hash: H256,
    pub reward_vote_hashes: Vec<H256>,
    pub pillar_vote_rlps: Vec<Vec<u8>>,
    pub transaction_rlps: Vec<Vec<u8>>,
    pub cert_vote_rlps: Vec<Vec<u8>>,
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
    pub dag_transaction_hashes: Vec<H256>,
    pub period_data_transaction_hashes: Vec<H256>,
    pub period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
    pub previous_cert_votes_present: bool,
    pub previous_cert_first_vote_has_weight: bool,
    pub pillar_votes_present: bool,
    pub extra_data_present: bool,
    pub extra_data_pillar_block_hash_present: bool,
    pub use_last_block_cert_votes: bool,
    pub current_period: u64,
    pub effective_size: usize,
}

/// Rust-owned PBFT period-data queue metadata.
///
/// Behavior preserved from C++:
/// - push accepts `max(period_, max_pbft_size) + 1`
/// - an empty queue also accepts `max_pbft_size + 2`
/// - chain progress past queued state clears old queued entries on accepted push
/// - `size()` reports only entries with available cert votes, not raw length
/// - popping the last entry resets the tracked period to zero
/// - stale cleanup removes front entries but does not otherwise mutate period
///   or last-cert-vote availability
#[derive(Debug, Default, Clone)]
pub struct PeriodDataQueue {
    entries: VecDeque<PeriodDataQueueEntryRef>,
    period: u64,
    last_block_cert_vote_rlps: Vec<Vec<u8>>,
}

impl PeriodDataQueue {
    /// Creates an empty period-data queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the latest accepted queue period, or zero when reset.
    pub fn period(&self) -> u64 {
        self.period
    }

    /// Returns the PBFT syncing period visible to network status.
    ///
    /// Inputs:
    /// - `pbft_chain_size`: local PBFT chain size supplied by the PBFT chain
    ///   compatibility executor.
    ///
    /// Outputs:
    /// - The maximum of the Rust-owned queue period and the supplied PBFT chain
    ///   size, preserving the legacy status-period calculation without making
    ///   the PBFT manager read queue metadata as an authoritative mirror.
    pub fn syncing_period(&self, pbft_chain_size: u64) -> u64 {
        self.period.max(pbft_chain_size)
    }

    /// Returns the PBFT block hash to use as the next chain-link fact.
    ///
    /// Inputs:
    /// - `current_period`: current PBFT period supplied by the PBFT chain
    ///   compatibility executor. PBFT-chain period remains authoritative at
    ///   this boundary.
    /// - `chain_last_hash`: last PBFT-chain block hash supplied by the PBFT
    ///   chain compatibility executor.
    ///
    /// Outputs:
    /// - The last queued PBFT block hash when Rust queue metadata proves the
    ///   queued period is not stale for `current_period`.
    /// - Otherwise `chain_last_hash`.
    pub fn last_block_hash_or_chain(&self, current_period: u64, chain_last_hash: H256) -> H256 {
        self.entries
            .back()
            .filter(|entry| entry.period >= current_period)
            .map(|entry| entry.block_hash)
            .unwrap_or(chain_last_hash)
    }

    /// Returns processable queue size under legacy cert-vote visibility rules.
    ///
    /// The tail entry is hidden when no side-car cert votes are available,
    /// because its cert votes may arrive only in a subsequent queued block.
    pub fn size(&self) -> usize {
        if !self.last_block_cert_vote_rlps.is_empty() || self.entries.is_empty() {
            self.entries.len()
        } else {
            self.entries.len().saturating_sub(1)
        }
    }

    /// Returns true when no period-data entries are queued.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears all queue metadata and resets period state.
    pub fn clear(&mut self) {
        self.period = 0;
        self.entries.clear();
        self.last_block_cert_vote_rlps.clear();
    }

    /// Returns one coherent queue snapshot using supplied PBFT-chain facts.
    pub fn snapshot(
        &self,
        pbft_chain_size: u64,
        current_period: u64,
        chain_last_hash: H256,
    ) -> PeriodDataQueueSnapshot {
        PeriodDataQueueSnapshot {
            period: self.period(),
            syncing_period: self.syncing_period(pbft_chain_size),
            last_block_hash_or_chain: self
                .last_block_hash_or_chain(current_period, chain_last_hash),
            size: self.size(),
            empty: self.is_empty(),
        }
    }

    /// Attempts to admit one complete period-data queue request.
    ///
    /// Rejected period sequencing leaves the queue unchanged. Accepted chain
    /// advancement may clear stale entries before appending the request.
    /// Checked period arithmetic overflow is returned as an error.
    pub fn push(
        &mut self,
        request: PeriodDataQueuePushRequest,
    ) -> Result<PeriodDataQueuePushOutcome> {
        let PeriodDataQueuePushRequest {
            entry,
            max_pbft_size,
            current_block_cert_vote_rlps,
        } = request;
        let entry_period = entry.period;
        let expected_next_period = std::cmp::max(self.period, max_pbft_size)
            .checked_add(1)
            .ok_or_else(|| anyhow!("period data queue next-period calculation overflowed"))?;
        let empty_queue_backfill_period = max_pbft_size.checked_add(2);

        let queue_empty_backfill =
            self.entries.is_empty() && Some(entry_period) == empty_queue_backfill_period;
        if entry_period != expected_next_period && !queue_empty_backfill {
            return Ok(PeriodDataQueuePushOutcome {
                accepted: false,
                clear_existing: false,
                expected_next_period,
                actual_period: entry_period,
                current_period: self.period,
                effective_size: self.size(),
            });
        }

        let clear_existing = max_pbft_size > self.period && !self.entries.is_empty();
        if clear_existing {
            self.entries.clear();
        }

        self.period = entry_period;
        self.entries.push_back(entry);
        self.last_block_cert_vote_rlps = current_block_cert_vote_rlps;

        Ok(PeriodDataQueuePushOutcome {
            accepted: true,
            clear_existing,
            expected_next_period,
            actual_period: entry_period,
            current_period: self.period,
            effective_size: self.size(),
        })
    }

    /// Pops queue metadata and returns the C++ payload/cert-vote handoff plan.
    ///
    /// Error behavior:
    /// - returns an error when the raw queue is empty.
    pub fn pop(&mut self) -> Result<PeriodDataQueuePopPlan> {
        let Some(entry) = self.entries.pop_front() else {
            return Err(anyhow!("cannot pop from empty period data queue"));
        };

        if let Some(next) = self.entries.front() {
            return Ok(PeriodDataQueuePopPlan {
                period_data_rlp: entry.period_data_rlp,
                source_peer_id: entry.source_peer_id,
                entry_period: entry.period,
                block_hash: entry.block_hash,
                prev_block_hash: entry.prev_block_hash,
                pivot_hash: entry.pivot_hash,
                final_chain_hash: entry.final_chain_hash,
                reward_vote_hashes: entry.reward_vote_hashes,
                pillar_vote_rlps: entry.pillar_vote_rlps,
                transaction_rlps: entry.transaction_rlps,
                cert_vote_rlps: next.previous_cert_vote_rlps.clone(),
                previous_cert_vote_rlps: entry.previous_cert_vote_rlps,
                dag_transaction_hashes: entry.dag_transaction_hashes,
                period_data_transaction_hashes: entry.period_data_transaction_hashes,
                period_data_transaction_identities: entry.period_data_transaction_identities,
                previous_cert_votes_present: entry.previous_cert_votes_present,
                previous_cert_first_vote_has_weight: entry.previous_cert_first_vote_has_weight,
                pillar_votes_present: entry.pillar_votes_present,
                extra_data_present: entry.extra_data_present,
                extra_data_pillar_block_hash_present: entry.extra_data_pillar_block_hash_present,
                use_last_block_cert_votes: false,
                current_period: self.period,
                effective_size: self.size(),
            });
        }

        self.period = 0;
        let cert_vote_rlps = std::mem::take(&mut self.last_block_cert_vote_rlps);
        Ok(PeriodDataQueuePopPlan {
            period_data_rlp: entry.period_data_rlp,
            source_peer_id: entry.source_peer_id,
            entry_period: entry.period,
            block_hash: entry.block_hash,
            prev_block_hash: entry.prev_block_hash,
            pivot_hash: entry.pivot_hash,
            final_chain_hash: entry.final_chain_hash,
            reward_vote_hashes: entry.reward_vote_hashes,
            pillar_vote_rlps: entry.pillar_vote_rlps,
            transaction_rlps: entry.transaction_rlps,
            cert_vote_rlps,
            previous_cert_vote_rlps: entry.previous_cert_vote_rlps,
            dag_transaction_hashes: entry.dag_transaction_hashes,
            period_data_transaction_hashes: entry.period_data_transaction_hashes,
            period_data_transaction_identities: entry.period_data_transaction_identities,
            previous_cert_votes_present: entry.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: entry.previous_cert_first_vote_has_weight,
            pillar_votes_present: entry.pillar_votes_present,
            extra_data_present: entry.extra_data_present,
            extra_data_pillar_block_hash_present: entry.extra_data_pillar_block_hash_present,
            use_last_block_cert_votes: true,
            current_period: self.period,
            effective_size: self.size(),
        })
    }

    /// Returns the last queued entry metadata, if any.
    pub fn last_entry(&self) -> Option<PeriodDataQueueEntryRef> {
        self.entries.back().cloned()
    }

    /// Removes queued entries with period lower than `period`.
    ///
    /// This intentionally preserves legacy behavior: only front entries are
    /// removed, while `period` and last-cert-vote payload availability are left intact.
    /// Returns number of queue entries removed from the live metadata deque.
    pub fn clean_old_data(&mut self, period: u64) -> usize {
        let mut removed = 0usize;
        while self
            .entries
            .front()
            .map(|entry| entry.period < period)
            .unwrap_or(false)
        {
            if self.entries.pop_front().is_some() {
                removed += 1;
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_validation::legacy_pbft_vote_signing_hash;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use tiny_keccak::{Hasher, Keccak};

    fn decode_encoded_period_data_queue_push(
        request: EncodedPeriodDataQueuePushRequest,
    ) -> Result<PeriodDataQueuePushRequest> {
        Ok(super::decode_encoded_period_data_queue_push(request)?.with_chain_size(0))
    }

    fn keccak256(bytes: &[u8]) -> H256 {
        let mut hasher = Keccak::v256();
        let mut output = [0u8; 32];
        hasher.update(bytes);
        hasher.finalize(&mut output);
        output.into()
    }

    fn append_unsigned_pbft_fields(stream: &mut RlpStream, period: u64, order_hash: &[u8]) {
        stream.append(&H256::repeat_byte(0x11));
        stream.append(&H256::repeat_byte(0x22));
        stream.append(&order_hash);
        stream.append(&H256::repeat_byte(0x44));
        stream.append(&period);
        stream.append(&7u64);
        stream.begin_list(0);
    }

    fn signed_pbft_block(
        period: u64,
        extra_data: Option<&[u8]>,
        order_hash: &[u8],
        recovery_id_override: Option<u8>,
    ) -> Vec<u8> {
        let unsigned_fields = if extra_data.is_some() { 8 } else { 7 };
        let mut unsigned = RlpStream::new_list(unsigned_fields);
        append_unsigned_pbft_fields(&mut unsigned, period, order_hash);
        if let Some(extra_data) = extra_data {
            unsigned.append(&extra_data);
        }
        let signing_key = SigningKey::from_slice(&[0x42; 32]).unwrap();
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id_override.unwrap_or_else(|| recovery_id.to_byte()));

        let mut block = RlpStream::new_list(unsigned_fields + 1);
        append_unsigned_pbft_fields(&mut block, period, order_hash);
        if let Some(extra_data) = extra_data {
            block.append(&extra_data);
        }
        block.append(&signature_bytes);
        block.out().to_vec()
    }

    fn encoded_period_data_with(
        period: u64,
        previous_cert_bundle: Option<&[u8]>,
        extra_data: Option<&[u8]>,
    ) -> Vec<u8> {
        let block = signed_pbft_block(period, extra_data, &[0x33; 32], None);
        encoded_period_data_from_block(&block, previous_cert_bundle, None)
    }

    fn encoded_period_data_from_block(
        block: &[u8],
        previous_cert_bundle: Option<&[u8]>,
        dag_bundle: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&block, 1);
        if let Some(previous_cert_bundle) = previous_cert_bundle {
            period_data.append_raw(previous_cert_bundle, 1);
        } else {
            period_data.append_empty_data();
        }
        if let Some(dag_bundle) = dag_bundle {
            period_data.append_raw(dag_bundle, 1);
        } else {
            period_data.append_empty_data();
        }
        period_data.begin_list(0);
        period_data.out().to_vec()
    }

    fn encoded_period_data(period: u64) -> Vec<u8> {
        encoded_period_data_with(period, None, None)
    }

    fn pbft_sync_packet(period_data: &[u8], current_cert_bundle: Option<&[u8]>) -> Vec<u8> {
        let mut packet = RlpStream::new_list(3);
        packet.append(&true);
        packet.append_raw(period_data, 1);
        if let Some(bundle) = current_cert_bundle {
            packet.append_raw(bundle, 1);
        } else {
            packet.append(&0u8);
        }
        packet.out().to_vec()
    }

    fn signed_cert_bundle() -> (Vec<u8>, Vec<u8>) {
        let block_hash = H256::repeat_byte(0x77);
        let period = 1u64;
        let round = 0u64;
        let step = 3u64;
        let proof = vec![0x61; 80];
        let mut sortition = RlpStream::new_list(4);
        sortition.append(&period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&proof);
        let sortition_bytes = sortition.out().to_vec();
        let signing_key = SigningKey::from_slice(&[0x24; 32]).unwrap();
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(
                legacy_pbft_vote_signing_hash(block_hash, &sortition_bytes).as_bytes(),
            )
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut canonical = RlpStream::new_list(3);
        canonical.append(&block_hash);
        canonical.append(&sortition_bytes);
        canonical.append(&signature_bytes);
        let canonical = canonical.out().to_vec();

        let mut optimized = RlpStream::new_list(2);
        optimized.append(&proof);
        optimized.append(&signature_bytes);
        let optimized = optimized.out().to_vec();
        let mut bundle = RlpStream::new_list(5);
        bundle.append(&block_hash);
        bundle.append(&period);
        bundle.append(&round);
        bundle.append(&step);
        bundle.begin_list(1);
        bundle.append_raw(&optimized, 1);
        (bundle.out().to_vec(), canonical)
    }

    #[test]
    fn raw_pbft_sync_packet_retains_period_data_and_reconstructs_current_votes() {
        let period_data = encoded_period_data(1);
        let (bundle, canonical_vote) = signed_cert_bundle();
        let decoded =
            super::decode_pbft_sync_packet_precheck(&pbft_sync_packet(&period_data, Some(&bundle)))
                .expect("canonical raw PBFT-sync packet");

        assert_eq!(decoded.period_data.entry.period_data_rlp, period_data);
        assert!(decoded.current_cert_votes_present);
        assert_eq!(
            decoded.period_data.current_block_cert_vote_rlps,
            vec![canonical_vote]
        );
    }

    #[test]
    fn raw_pbft_sync_packet_rejects_non_three_item_and_trailing_outer_rlp() {
        let period_data = encoded_period_data(1);
        let mut short = RlpStream::new_list(2);
        short.append(&true);
        short.append_raw(&period_data, 1);
        assert!(super::decode_pbft_sync_packet_precheck(&short.out()).is_err());

        let mut trailing = pbft_sync_packet(&period_data, None);
        trailing.push(0x80);
        assert!(super::decode_pbft_sync_packet_precheck(&trailing).is_err());
    }

    #[test]
    fn raw_pbft_sync_packet_requires_canonical_period_one_cert_placeholder() {
        let block = signed_pbft_block(1, None, &[0; 32], None);
        let noncanonical_period_data = encoded_period_data_from_block(&block, Some(&[0xc0]), None);
        let error = match super::decode_pbft_sync_packet_precheck(&pbft_sync_packet(
            &noncanonical_period_data,
            None,
        )) {
            Ok(_) => panic!("period one must retain the canonical empty certificate placeholder"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("PBFT_SYNC_PACKET_PREVIOUS_CERT_PLACEHOLDER"));
    }

    #[test]
    fn raw_pbft_sync_packet_exposes_order_hash_mismatch_without_mutation() {
        let period_data = encoded_period_data(1);
        let decoded =
            super::decode_pbft_sync_packet_precheck(&pbft_sync_packet(&period_data, None))
                .expect("strict period data still decodes before order comparison");

        assert_eq!(decoded.declared_order_hash, H256::repeat_byte(0x33));
        assert_eq!(decoded.calculated_order_hash, H256::zero());
    }

    #[test]
    fn encoded_push_derives_block_and_presence_facts_before_admission() {
        let encoded = encoded_period_data(1);
        let request = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded.clone(),
            source_peer_id: [0x55; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect("canonical encoded period data decodes");

        assert_eq!(request.entry.period_data_rlp, encoded);
        assert_eq!(request.entry.source_peer_id, [0x55; 64]);
        assert_eq!(request.entry.period, 1);
        assert_eq!(request.entry.prev_block_hash, H256::repeat_byte(0x11));
        assert_eq!(request.entry.pivot_hash, H256::repeat_byte(0x22));
        assert_eq!(request.entry.final_chain_hash, H256::repeat_byte(0x44));
        assert!(request.entry.reward_vote_hashes.is_empty());
        assert!(request.entry.dag_transaction_hashes.is_empty());
        assert!(request.entry.transaction_rlps.is_empty());
        assert!(!request.entry.previous_cert_votes_present);
        assert!(!request.entry.pillar_votes_present);
        assert!(!request.entry.extra_data_present);
    }

    #[test]
    fn encoded_push_rejects_malformed_outer_shape_without_queue_mutation() {
        let mut malformed = RlpStream::new_list(3);
        malformed.append_empty_data();
        malformed.append_empty_data();
        malformed.append_empty_data();
        let queue = PeriodDataQueue::new();

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: malformed.out().to_vec(),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("invalid outer shape must fail")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_OUTER_SHAPE"));
        assert!(queue.is_empty());
    }

    #[test]
    fn encoded_push_rejects_trailing_outer_bytes() {
        let mut encoded = encoded_period_data(1);
        encoded.push(0x80);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded,
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("trailing bytes must fail")
        .to_string();

        assert!(error.contains("trailing bytes"));
    }

    #[test]
    fn encoded_push_rejects_out_of_range_pbft_recovery_id() {
        let block = signed_pbft_block(1, None, &[0x33; 32], Some(4));
        let encoded = encoded_period_data_from_block(&block, None, None);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded,
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("out-of-range recovery id must fail")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_BLOCK_SIGNATURE"));
    }

    #[test]
    fn encoded_push_rejects_signed_block_with_malformed_order_hash() {
        let block = signed_pbft_block(1, None, &[0x33; 31], None);
        let encoded = encoded_period_data_from_block(&block, None, None);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded,
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("malformed order hash must fail before admission")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_ORDER_HASH"));
    }

    #[test]
    fn encoded_push_rejects_invalid_current_certificate_signature() {
        let (_, valid_vote) = signed_cert_bundle();
        let valid = Rlp::new(&valid_vote);
        let mut invalid = RlpStream::new_list(3);
        invalid.append_raw(valid.at(0).unwrap().as_raw(), 1);
        invalid.append_raw(valid.at(1).unwrap().as_raw(), 1);
        invalid.append(&[0u8; 65].as_slice());

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded_period_data(1),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: vec![invalid.out().to_vec()],
        })
        .expect_err("invalid current certificate signature must fail")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_CERT_VOTE_SIGNATURE"));
    }

    #[test]
    fn encoded_push_accepts_binary_node_implementation_extra_data() {
        let mut extra = RlpStream::new_list(6);
        for version in [1u16, 2, 3, 4] {
            extra.append(&version);
        }
        extra.append(&[0xff, 0xfe].as_slice());
        extra.append_empty_data();
        let encoded = encoded_period_data_with(1, None, Some(&extra.out()));

        let request = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded,
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect("binary legacy implementation bytes are valid");

        assert!(request.entry.extra_data_present);
        assert!(!request.entry.extra_data_pillar_block_hash_present);
    }

    #[test]
    fn encoded_push_rejects_trailing_pbft_extra_data() {
        let mut extra = RlpStream::new_list(6);
        for version in [1u16, 2, 3, 4] {
            extra.append(&version);
        }
        extra.append(&b"node".as_slice());
        extra.append_empty_data();
        let mut extra = extra.out().to_vec();
        extra.push(0x80);
        let block = signed_pbft_block(1, Some(&extra), &[0x33; 32], None);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded_period_data_from_block(&block, None, None),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("trailing extra-data RLP must fail")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_EXTRA_DATA"));
    }

    #[test]
    fn encoded_push_rejects_malformed_nonempty_dag_vdf() {
        let mut compact_block = RlpStream::new_list(7);
        compact_block.append(&H256::zero());
        compact_block.append(&1u64);
        compact_block.append(&1u64);
        compact_block.append(&[0x00].as_slice());
        compact_block.begin_list(0);
        compact_block.append(&[0u8; 65].as_slice());
        compact_block.append(&0u64);
        let compact_block = compact_block.out().to_vec();
        let mut dag_bundle = RlpStream::new_list(3);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(1);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(1);
        dag_bundle.append_raw(&compact_block, 1);
        let block = signed_pbft_block(1, None, &[0x33; 32], None);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded_period_data_from_block(&block, None, Some(&dag_bundle.out())),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("malformed embedded DAG VDF must fail")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_VDF_0"));
    }

    #[test]
    fn encoded_push_rejects_trailing_dag_vdf_bytes() {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&[0x11; 80].as_slice());
        vdf.append_empty_data();
        vdf.append_empty_data();
        vdf.append(&1u16);
        let mut vdf = vdf.out().to_vec();
        vdf.push(0x80);
        let mut compact_block = RlpStream::new_list(7);
        compact_block.append(&H256::zero());
        compact_block.append(&1u64);
        compact_block.append(&1u64);
        compact_block.append(&vdf);
        compact_block.begin_list(0);
        compact_block.append(&[0u8; 65].as_slice());
        compact_block.append(&0u64);
        let compact_block = compact_block.out().to_vec();
        let mut dag_bundle = RlpStream::new_list(3);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(1);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(1);
        dag_bundle.append_raw(&compact_block, 1);
        let block = signed_pbft_block(1, None, &[0x33; 32], None);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded_period_data_from_block(&block, None, Some(&dag_bundle.out())),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("trailing embedded DAG VDF bytes must fail")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_DAG_BLOCK_VDF_0"));
    }

    #[test]
    fn encoded_push_rejects_unreferenced_malformed_dag_transaction_hash() {
        let mut dag_bundle = RlpStream::new_list(3);
        dag_bundle.begin_list(1);
        dag_bundle.append(&[0x55; 31].as_slice());
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(0);
        let block = signed_pbft_block(1, None, &[0x33; 32], None);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded_period_data_from_block(&block, None, Some(&dag_bundle.out())),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: Vec::new(),
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("every ordered DAG transaction hash must be typed")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_DAG_TRANSACTION_0"));
    }

    #[test]
    fn encoded_push_validates_nonempty_previous_cert_bundle_against_full_votes() {
        let (bundle, vote) = signed_cert_bundle();
        let encoded = encoded_period_data_with(2, Some(&bundle), None);

        let request = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded,
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: vec![vote],
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect("matching optimized and full cert votes decode");

        assert!(request.entry.previous_cert_votes_present);
        assert_eq!(request.entry.previous_cert_vote_rlps.len(), 1);
    }

    #[test]
    fn encoded_push_rejects_optimized_and_full_previous_cert_mismatch() {
        let (bundle, valid_vote) = signed_cert_bundle();
        let valid = Rlp::new(&valid_vote);
        let mut mismatched = RlpStream::new_list(3);
        mismatched.append(&H256::repeat_byte(0x78));
        mismatched.append_raw(valid.at(1).unwrap().as_raw(), 1);
        mismatched.append_raw(valid.at(2).unwrap().as_raw(), 1);

        let error = decode_encoded_period_data_queue_push(EncodedPeriodDataQueuePushRequest {
            period_data_rlp: encoded_period_data_with(2, Some(&bundle), None),
            source_peer_id: [0; 64],
            previous_cert_vote_rlps: vec![mismatched.out().to_vec()],
            current_block_cert_vote_rlps: Vec::new(),
        })
        .expect_err("optimized and normalized previous votes must match")
        .to_string();

        assert!(error.contains("PBFT_PERIOD_DATA_QUEUE_PREVIOUS_CERT_MISMATCH"));
    }

    fn push(
        queue: &mut PeriodDataQueue,
        id: u64,
        period: u64,
        max_size: u64,
        cert_votes: usize,
    ) -> bool {
        let previous_cert_vote_rlps = if id % 2 == 0 {
            vec![vec![id as u8, 0xc0]]
        } else {
            Vec::new()
        };
        let current_block_cert_vote_rlps = (0..cert_votes)
            .map(|idx| vec![id as u8, 0xd0 + idx as u8])
            .collect();
        queue
            .push(PeriodDataQueuePushRequest {
                entry: PeriodDataQueueEntryRef {
                    period_data_rlp: vec![id as u8],
                    source_peer_id: [id as u8; 64],
                    period,
                    block_hash: H256::from_low_u64_be(id),
                    prev_block_hash: H256::from_low_u64_be(id + 1000),
                    pivot_hash: H256::from_low_u64_be(id + 2000),
                    final_chain_hash: H256::from_low_u64_be(id + 2500),
                    reward_vote_hashes: vec![H256::from_low_u64_be(id + 2600)],
                    pillar_vote_rlps: vec![vec![id as u8, 0xa0]],
                    transaction_rlps: vec![vec![id as u8, 0xb0]],
                    previous_cert_vote_rlps,
                    dag_transaction_hashes: vec![H256::from_low_u64_be(id + 3000)],
                    period_data_transaction_hashes: vec![H256::from_low_u64_be(id + 4000)],
                    period_data_transaction_identities: vec![PeriodDataQueueTransactionIdentity {
                        input_index: 0,
                        hash: H256::from_low_u64_be(id + 4000),
                        transaction_nonce: [id as u8; 32],
                        sender: [id as u8; 20],
                    }],
                    previous_cert_votes_present: id % 2 == 0,
                    previous_cert_first_vote_has_weight: id % 3 == 0,
                    pillar_votes_present: id % 5 == 0,
                    extra_data_present: id % 7 == 0,
                    extra_data_pillar_block_hash_present: id % 7 == 0 && id % 11 == 0,
                },
                max_pbft_size: max_size,
                current_block_cert_vote_rlps,
            })
            .unwrap()
            .accepted
    }

    #[test]
    fn push_accepts_sequential_periods_and_rejects_gaps() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 1, 1, 0, 1));
        assert_eq!(queue.period(), 1);
        assert!(!push(&mut queue, 2, 3, 0, 1));
        assert_eq!(queue.period(), 1);
        assert!(push(&mut queue, 3, 2, 0, 1));
        assert_eq!(queue.period(), 2);
    }

    #[test]
    fn push_accepts_empty_queue_backfill_period() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 2, 2, 0, 1));
        assert_eq!(queue.period(), 2);
    }

    #[test]
    fn accepted_push_clears_existing_entries_when_chain_advances() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 2, 2, 0, 1));
        let outcome = queue
            .push(PeriodDataQueuePushRequest {
                entry: PeriodDataQueueEntryRef {
                    period_data_rlp: vec![4],
                    source_peer_id: [4; 64],
                    period: 4,
                    block_hash: H256::from_low_u64_be(4),
                    prev_block_hash: H256::from_low_u64_be(1004),
                    pivot_hash: H256::from_low_u64_be(2004),
                    final_chain_hash: H256::from_low_u64_be(2504),
                    reward_vote_hashes: vec![H256::from_low_u64_be(2604)],
                    pillar_vote_rlps: vec![vec![4, 0xa0]],
                    transaction_rlps: vec![vec![4, 0xb0]],
                    previous_cert_vote_rlps: vec![vec![4, 0xc0]],
                    dag_transaction_hashes: vec![H256::from_low_u64_be(3004)],
                    period_data_transaction_hashes: vec![H256::from_low_u64_be(4004)],
                    period_data_transaction_identities: vec![PeriodDataQueueTransactionIdentity {
                        input_index: 0,
                        hash: H256::from_low_u64_be(4004),
                        transaction_nonce: [4; 32],
                        sender: [4; 20],
                    }],
                    previous_cert_votes_present: true,
                    previous_cert_first_vote_has_weight: false,
                    pillar_votes_present: true,
                    extra_data_present: true,
                    extra_data_pillar_block_hash_present: false,
                },
                max_pbft_size: 3,
                current_block_cert_vote_rlps: vec![vec![4, 0xd0]],
            })
            .unwrap();

        assert!(outcome.accepted);
        assert!(outcome.clear_existing);
        assert_eq!(queue.period(), 4);
        assert_eq!(queue.syncing_period(3), 4);
        assert_eq!(
            queue.last_block_hash_or_chain(4, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(4)
        );
        assert_eq!(
            queue.last_entry().unwrap().block_hash,
            H256::from_low_u64_be(4)
        );
        assert_eq!(
            queue.last_entry().unwrap().block_hash,
            H256::from_low_u64_be(4)
        );
        assert_eq!(
            queue.last_entry().unwrap().prev_block_hash,
            H256::from_low_u64_be(1004)
        );
        assert_eq!(
            queue.last_entry().unwrap().pivot_hash,
            H256::from_low_u64_be(2004)
        );
        assert_eq!(
            queue.last_entry().unwrap().final_chain_hash,
            H256::from_low_u64_be(2504)
        );
        assert_eq!(
            queue.last_entry().unwrap().dag_transaction_hashes,
            vec![H256::from_low_u64_be(3004)]
        );
        assert_eq!(
            queue.last_entry().unwrap().pillar_vote_rlps,
            vec![vec![4, 0xa0]]
        );
        assert_eq!(
            queue.last_entry().unwrap().transaction_rlps,
            vec![vec![4, 0xb0]]
        );
        assert_eq!(
            queue.last_entry().unwrap().previous_cert_vote_rlps,
            vec![vec![4, 0xc0]]
        );
        assert_eq!(
            queue.last_entry().unwrap().period_data_transaction_hashes,
            vec![H256::from_low_u64_be(4004)]
        );
        assert_eq!(
            queue
                .last_entry()
                .unwrap()
                .period_data_transaction_identities,
            vec![PeriodDataQueueTransactionIdentity {
                input_index: 0,
                hash: H256::from_low_u64_be(4004),
                transaction_nonce: [4; 32],
                sender: [4; 20]
            }]
        );
        assert!(queue.last_entry().unwrap().previous_cert_votes_present);
        assert!(
            !queue
                .last_entry()
                .unwrap()
                .previous_cert_first_vote_has_weight
        );
        assert!(queue.last_entry().unwrap().pillar_votes_present);
        assert!(queue.last_entry().unwrap().extra_data_present);
        assert!(
            !queue
                .last_entry()
                .unwrap()
                .extra_data_pillar_block_hash_present
        );
        assert_eq!(queue.size(), 1);
    }

    #[test]
    fn size_hides_tail_without_last_block_cert_votes() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 1, 1, 0, 0));
        assert_eq!(queue.size(), 0);
        assert!(!queue.is_empty());

        assert!(push(&mut queue, 2, 2, 0, 0));
        assert_eq!(queue.size(), 1);
    }

    #[test]
    fn pop_selects_next_entry_cert_votes_before_last_cert_votes() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 11, 1, 0, 1));
        assert!(push(&mut queue, 22, 2, 0, 1));

        let first = queue.pop().unwrap();
        assert_eq!(first.source_peer_id, [11; 64]);
        assert_eq!(first.period_data_rlp, vec![11]);
        assert_eq!(first.entry_period, 1);
        assert_eq!(first.block_hash, H256::from_low_u64_be(11));
        assert_eq!(first.prev_block_hash, H256::from_low_u64_be(1011));
        assert_eq!(first.pivot_hash, H256::from_low_u64_be(2011));
        assert_eq!(first.final_chain_hash, H256::from_low_u64_be(2511));
        assert_eq!(first.pillar_vote_rlps, vec![vec![11, 0xa0]]);
        assert_eq!(first.transaction_rlps, vec![vec![11, 0xb0]]);
        assert_eq!(first.cert_vote_rlps, vec![vec![22, 0xc0]]);
        assert!(first.previous_cert_vote_rlps.is_empty());
        assert_eq!(
            first.dag_transaction_hashes,
            vec![H256::from_low_u64_be(3011)]
        );
        assert_eq!(
            first.period_data_transaction_hashes,
            vec![H256::from_low_u64_be(4011)]
        );
        assert_eq!(first.period_data_transaction_identities.len(), 1);
        assert_eq!(
            first.period_data_transaction_identities[0].hash,
            H256::from_low_u64_be(4011)
        );
        assert!(!first.previous_cert_votes_present);
        assert!(!first.previous_cert_first_vote_has_weight);
        assert!(!first.pillar_votes_present);
        assert!(!first.extra_data_present);
        assert!(!first.extra_data_pillar_block_hash_present);
        assert!(!first.use_last_block_cert_votes);
        assert_eq!(queue.period(), 2);

        let second = queue.pop().unwrap();
        assert_eq!(second.period_data_rlp, vec![22]);
        assert_eq!(second.source_peer_id, [22; 64]);
        assert!(second.previous_cert_votes_present);
        assert_eq!(second.cert_vote_rlps, vec![vec![22, 0xd0]]);
        assert_eq!(second.previous_cert_vote_rlps, vec![vec![22, 0xc0]]);
        assert!(!second.previous_cert_first_vote_has_weight);
        assert!(!second.pillar_votes_present);
        assert!(!second.extra_data_present);
        assert!(!second.extra_data_pillar_block_hash_present);
        assert!(second.use_last_block_cert_votes);
        assert_eq!(queue.period(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn pop_preserves_zero_peer_identity_for_database_replay() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 0, 1, 0, 1));

        let popped = queue.pop().expect("queued replay payload pops");

        assert_eq!(popped.source_peer_id, [0; 64]);
    }

    #[test]
    fn clean_old_data_removes_older_entries_and_preserves_queue_state() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 5, 5, 4, 1));
        assert!(push(&mut queue, 6, 6, 4, 1));

        let removed = queue.clean_old_data(6);

        assert_eq!(removed, 1);
        assert_eq!(queue.period(), 6);
        assert_eq!(queue.syncing_period(8), 8);
        assert_eq!(
            queue.last_block_hash_or_chain(6, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(6)
        );
        assert_eq!(
            queue.last_block_hash_or_chain(7, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(99)
        );
        assert_eq!(
            queue.last_entry().unwrap().block_hash,
            H256::from_low_u64_be(6)
        );
    }

    #[test]
    fn clear_resets_all_state_and_pop_empty_errors() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 1, 1, 0, 1));

        queue.clear();

        assert_eq!(queue.period(), 0);
        assert_eq!(queue.syncing_period(7), 7);
        assert_eq!(
            queue.last_block_hash_or_chain(1, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(99)
        );
        assert!(queue.is_empty());
        assert_eq!(queue.size(), 0);
        let err = queue.pop().unwrap_err().to_string();
        assert!(err.contains("empty period data queue"));
    }
}
