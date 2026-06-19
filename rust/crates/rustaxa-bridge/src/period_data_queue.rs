use crate::ffi::rustaxa_ffi::{
    PbftSyncTransactionHash, PeriodDataQueueEntryRef, PeriodDataQueueLastEntryLookup,
    PeriodDataQueuePopPlan, PeriodDataQueuePushOutcome, PeriodDataQueueTransactionIdentity,
};
use crate::ffi::BridgePeriodDataQueue;
use rustaxa_consensus::period_data_queue::PeriodDataQueue;

/// Creates an empty Rust period-data queue metadata store for PBFT syncing.
pub fn create_period_data_queue() -> Box<BridgePeriodDataQueue> {
    Box::new(BridgePeriodDataQueue(PeriodDataQueue::new()))
}

impl BridgePeriodDataQueue {
    /// Returns the current queue period marker.
    pub fn period_data_queue_period(&self) -> u64 {
        self.0.period()
    }

    /// Returns the queue-aware PBFT syncing period for network status.
    pub fn period_data_queue_syncing_period(&self, pbft_chain_size: u64) -> u64 {
        self.0.syncing_period(pbft_chain_size)
    }

    /// Returns the Rust-owned queue hash decision or the supplied PBFT-chain hash.
    pub fn period_data_queue_last_block_hash_or_chain(
        &self,
        current_period: u64,
        chain_last_hash: [u8; 32],
    ) -> [u8; 32] {
        self.0
            .last_block_hash_or_chain(current_period, ethereum_types::H256::from(chain_last_hash))
            .into()
    }

    /// Returns processable queue size under legacy cert-vote visibility rules.
    pub fn period_data_queue_size(&self) -> usize {
        self.0.size()
    }

    /// Returns true when queue metadata has no entries.
    pub fn period_data_queue_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Clears all Rust queue metadata.
    pub fn period_data_queue_clear(&mut self) {
        self.0.clear();
    }

    /// Attempts to push one C++ payload reference into Rust queue metadata.
    pub fn period_data_queue_push(
        &mut self,
        entry_id: u64,
        period: u64,
        block_hash: [u8; 32],
        prev_block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        final_chain_hash: [u8; 32],
        reward_vote_hashes: Vec<PbftSyncTransactionHash>,
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
        pillar_votes_present: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
        max_pbft_size: u64,
        current_block_cert_votes_count: usize,
    ) -> Result<PeriodDataQueuePushOutcome, anyhow::Error> {
        Ok(self
            .0
            .push(
                entry_id,
                period,
                ethereum_types::H256::from(block_hash),
                ethereum_types::H256::from(prev_block_hash),
                ethereum_types::H256::from(pivot_hash),
                ethereum_types::H256::from(final_chain_hash),
                bridge_hashes_to_h256(reward_vote_hashes),
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
                current_block_cert_votes_count,
            )?
            .into())
    }

    /// Pops one queue metadata entry and returns the C++ payload handoff plan.
    pub fn period_data_queue_pop(&mut self) -> Result<PeriodDataQueuePopPlan, anyhow::Error> {
        Ok(self.0.pop()?.into())
    }

    /// Returns the last queued entry metadata.
    pub fn period_data_queue_last_entry(&self) -> PeriodDataQueueLastEntryLookup {
        self.0
            .last_entry()
            .map(|entry| PeriodDataQueueLastEntryLookup {
                found: true,
                entry_id: entry.entry_id,
                period: entry.period,
                block_hash: entry.block_hash.into(),
                prev_block_hash: entry.prev_block_hash.into(),
                pivot_hash: entry.pivot_hash.into(),
                final_chain_hash: entry.final_chain_hash.into(),
                reward_vote_hashes: transaction_hashes_to_bridge(entry.reward_vote_hashes),
                dag_transaction_hashes: transaction_hashes_to_bridge(entry.dag_transaction_hashes),
                period_data_transaction_hashes: transaction_hashes_to_bridge(
                    entry.period_data_transaction_hashes,
                ),
                period_data_transaction_identities: transaction_identities_to_bridge(
                    entry.period_data_transaction_identities,
                ),
                previous_cert_votes_present: entry.previous_cert_votes_present,
                previous_cert_first_vote_has_weight: entry.previous_cert_first_vote_has_weight,
                pillar_votes_present: entry.pillar_votes_present,
                extra_data_present: entry.extra_data_present,
                extra_data_pillar_block_hash_present: entry.extra_data_pillar_block_hash_present,
            })
            .unwrap_or(PeriodDataQueueLastEntryLookup {
                found: false,
                entry_id: 0,
                period: 0,
                block_hash: [0; 32],
                prev_block_hash: [0; 32],
                pivot_hash: [0; 32],
                final_chain_hash: [0; 32],
                reward_vote_hashes: Vec::new(),
                dag_transaction_hashes: Vec::new(),
                period_data_transaction_hashes: Vec::new(),
                period_data_transaction_identities: Vec::new(),
                previous_cert_votes_present: false,
                previous_cert_first_vote_has_weight: false,
                pillar_votes_present: false,
                extra_data_present: false,
                extra_data_pillar_block_hash_present: false,
            })
    }

    /// Removes old queue metadata and returns removed C++ payload ids.
    pub fn period_data_queue_clean_old_data(
        &mut self,
        period: u64,
    ) -> Vec<PeriodDataQueueEntryRef> {
        self.0
            .clean_old_data(period)
            .into_iter()
            .map(Into::into)
            .collect()
    }
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
