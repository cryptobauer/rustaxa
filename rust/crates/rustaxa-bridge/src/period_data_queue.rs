use crate::ffi::rustaxa_ffi::{
    PbftSyncTransactionHash, PeriodDataQueueEntryRef, PeriodDataQueuePbftVotePayload,
    PeriodDataQueuePillarVotePayload, PeriodDataQueuePopPlan, PeriodDataQueuePushOutcome,
    PeriodDataQueueTransactionIdentity, PeriodDataQueueTransactionPayload,
};
use rustaxa_consensus::period_data_queue::PeriodDataQueue;

/// Pushes one CXX-safe period-data queue payload into a Rust-owned queue.
///
/// The PBFT manager runtime owns the queue metadata. C++ temporarily retains
/// live `PeriodData`/vote sidecars and passes compact facts through this helper
/// until those payload model types move to Rust.
pub(crate) fn bridge_period_data_queue_push(
    queue: &mut PeriodDataQueue,
    entry_id: u64,
    period: u64,
    block_hash: [u8; 32],
    prev_block_hash: [u8; 32],
    pivot_hash: [u8; 32],
    final_chain_hash: [u8; 32],
    reward_vote_hashes: Vec<PbftSyncTransactionHash>,
    pillar_vote_rlps: Vec<PeriodDataQueuePillarVotePayload>,
    transaction_rlps: Vec<PeriodDataQueueTransactionPayload>,
    previous_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
    dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
    period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
    period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
    previous_cert_votes_present: bool,
    previous_cert_first_vote_has_weight: bool,
    pillar_votes_present: bool,
    extra_data_present: bool,
    extra_data_pillar_block_hash_present: bool,
    max_pbft_size: u64,
    current_block_cert_vote_rlps: Vec<PeriodDataQueuePbftVotePayload>,
) -> Result<PeriodDataQueuePushOutcome, anyhow::Error> {
    Ok(queue
        .push(
            entry_id,
            period,
            ethereum_types::H256::from(block_hash),
            ethereum_types::H256::from(prev_block_hash),
            ethereum_types::H256::from(pivot_hash),
            ethereum_types::H256::from(final_chain_hash),
            bridge_hashes_to_h256(reward_vote_hashes),
            pillar_vote_rlps
                .into_iter()
                .map(|payload| payload.vote_rlp)
                .collect(),
            transaction_rlps
                .into_iter()
                .map(|payload| payload.transaction_rlp)
                .collect(),
            pbft_vote_rlps_to_vec(previous_cert_vote_rlps),
            dag_transaction_hashes
                .into_iter()
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            period_data_transaction_hashes
                .into_iter()
                .map(|hash| ethereum_types::H256::from(hash.hash))
                .collect(),
            period_data_transaction_identities
                .into_iter()
                .map(|identity| {
                    rustaxa_consensus::period_data_queue::PeriodDataQueueTransactionIdentity {
                        input_index: identity.input_index,
                        hash: ethereum_types::H256::from(identity.hash),
                        transaction_nonce: identity.transaction_nonce,
                        sender: identity.sender,
                    }
                })
                .collect(),
            previous_cert_votes_present,
            previous_cert_first_vote_has_weight,
            pillar_votes_present,
            extra_data_present,
            extra_data_pillar_block_hash_present,
            max_pbft_size,
            pbft_vote_rlps_to_vec(current_block_cert_vote_rlps),
        )?
        .into())
}

/// Pops one CXX-safe handoff plan from a Rust-owned period-data queue.
pub(crate) fn bridge_period_data_queue_pop(
    queue: &mut PeriodDataQueue,
) -> Result<PeriodDataQueuePopPlan, anyhow::Error> {
    Ok(queue.pop()?.into())
}

/// Cleans old entries from a Rust-owned period-data queue.
pub(crate) fn bridge_period_data_queue_clean_old_data(
    queue: &mut PeriodDataQueue,
    period: u64,
) -> Vec<PeriodDataQueueEntryRef> {
    queue
        .clean_old_data(period)
        .into_iter()
        .map(Into::into)
        .collect()
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueueEntryRef>
    for PeriodDataQueueEntryRef
{
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueueEntryRef) -> Self {
        Self {
            entry_id: value.entry_id,
            period: value.period,
            block_hash: value.block_hash.into(),
            prev_block_hash: value.prev_block_hash.into(),
            pivot_hash: value.pivot_hash.into(),
            final_chain_hash: value.final_chain_hash.into(),
            reward_vote_hashes: transaction_hashes_to_bridge(value.reward_vote_hashes),
            pillar_vote_rlps: pillar_vote_rlps_to_bridge(value.pillar_vote_rlps),
            transaction_rlps: transaction_rlps_to_bridge(value.transaction_rlps),
            previous_cert_vote_rlps: pbft_vote_rlps_to_bridge(value.previous_cert_vote_rlps),
            dag_transaction_hashes: transaction_hashes_to_bridge(value.dag_transaction_hashes),
            period_data_transaction_hashes: transaction_hashes_to_bridge(
                value.period_data_transaction_hashes,
            ),
            period_data_transaction_identities: transaction_identities_to_bridge(
                value.period_data_transaction_identities,
            ),
            previous_cert_votes_present: value.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: value.previous_cert_first_vote_has_weight,
            pillar_votes_present: value.pillar_votes_present,
            extra_data_present: value.extra_data_present,
            extra_data_pillar_block_hash_present: value.extra_data_pillar_block_hash_present,
        }
    }
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueuePushOutcome>
    for PeriodDataQueuePushOutcome
{
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueuePushOutcome) -> Self {
        Self {
            accepted: value.accepted,
            clear_existing: value.clear_existing,
            expected_next_period: value.expected_next_period,
            actual_period: value.actual_period,
            current_period: value.current_period,
            effective_size: value.effective_size,
        }
    }
}

impl From<rustaxa_consensus::period_data_queue::PeriodDataQueuePopPlan> for PeriodDataQueuePopPlan {
    fn from(value: rustaxa_consensus::period_data_queue::PeriodDataQueuePopPlan) -> Self {
        Self {
            entry_id: value.entry_id,
            entry_period: value.entry_period,
            block_hash: value.block_hash.into(),
            prev_block_hash: value.prev_block_hash.into(),
            pivot_hash: value.pivot_hash.into(),
            final_chain_hash: value.final_chain_hash.into(),
            reward_vote_hashes: transaction_hashes_to_bridge(value.reward_vote_hashes),
            pillar_vote_rlps: pillar_vote_rlps_to_bridge(value.pillar_vote_rlps),
            transaction_rlps: transaction_rlps_to_bridge(value.transaction_rlps),
            cert_vote_rlps: pbft_vote_rlps_to_bridge(value.cert_vote_rlps),
            previous_cert_vote_rlps: pbft_vote_rlps_to_bridge(value.previous_cert_vote_rlps),
            dag_transaction_hashes: transaction_hashes_to_bridge(value.dag_transaction_hashes),
            period_data_transaction_hashes: transaction_hashes_to_bridge(
                value.period_data_transaction_hashes,
            ),
            period_data_transaction_identities: transaction_identities_to_bridge(
                value.period_data_transaction_identities,
            ),
            previous_cert_votes_present: value.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: value.previous_cert_first_vote_has_weight,
            pillar_votes_present: value.pillar_votes_present,
            extra_data_present: value.extra_data_present,
            extra_data_pillar_block_hash_present: value.extra_data_pillar_block_hash_present,
            use_last_block_cert_votes: value.use_last_block_cert_votes,
            next_entry_id: value.next_entry_id,
            current_period: value.current_period,
            effective_size: value.effective_size,
        }
    }
}

fn transaction_hashes_to_bridge(hashes: Vec<ethereum_types::H256>) -> Vec<PbftSyncTransactionHash> {
    hashes
        .into_iter()
        .map(|hash| PbftSyncTransactionHash { hash: hash.into() })
        .collect()
}

fn bridge_hashes_to_h256(hashes: Vec<PbftSyncTransactionHash>) -> Vec<ethereum_types::H256> {
    hashes
        .into_iter()
        .map(|hash| ethereum_types::H256::from(hash.hash))
        .collect()
}

fn pillar_vote_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<PeriodDataQueuePillarVotePayload> {
    rlps.into_iter()
        .map(|vote_rlp| PeriodDataQueuePillarVotePayload { vote_rlp })
        .collect()
}

fn transaction_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<PeriodDataQueueTransactionPayload> {
    rlps.into_iter()
        .map(|transaction_rlp| PeriodDataQueueTransactionPayload { transaction_rlp })
        .collect()
}

fn pbft_vote_rlps_to_vec(payloads: Vec<PeriodDataQueuePbftVotePayload>) -> Vec<Vec<u8>> {
    payloads
        .into_iter()
        .map(|payload| payload.vote_rlp)
        .collect()
}

fn pbft_vote_rlps_to_bridge(rlps: Vec<Vec<u8>>) -> Vec<PeriodDataQueuePbftVotePayload> {
    rlps.into_iter()
        .map(|vote_rlp| PeriodDataQueuePbftVotePayload { vote_rlp })
        .collect()
}

fn transaction_identities_to_bridge(
    identities: Vec<rustaxa_consensus::period_data_queue::PeriodDataQueueTransactionIdentity>,
) -> Vec<PeriodDataQueueTransactionIdentity> {
    identities
        .into_iter()
        .map(|identity| PeriodDataQueueTransactionIdentity {
            input_index: identity.input_index,
            hash: identity.hash.into(),
            transaction_nonce: identity.transaction_nonce,
            sender: identity.sender,
        })
        .collect()
}
