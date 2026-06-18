use crate::ffi::rustaxa_ffi::{
    PbftSyncTransactionHash, PeriodDataQueueEntryRef, PeriodDataQueueLastEntryLookup,
    PeriodDataQueuePopPlan, PeriodDataQueuePushOutcome,
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
        dag_transaction_hashes: Vec<PbftSyncTransactionHash>,
        period_data_transaction_hashes: Vec<PbftSyncTransactionHash>,
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
                dag_transaction_hashes
                    .into_iter()
                    .map(|hash| ethereum_types::H256::from(hash.hash))
                    .collect(),
                period_data_transaction_hashes
                    .into_iter()
                    .map(|hash| ethereum_types::H256::from(hash.hash))
                    .collect(),
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
                dag_transaction_hashes: transaction_hashes_to_bridge(entry.dag_transaction_hashes),
                period_data_transaction_hashes: transaction_hashes_to_bridge(
                    entry.period_data_transaction_hashes,
                ),
            })
            .unwrap_or(PeriodDataQueueLastEntryLookup {
                found: false,
                entry_id: 0,
                period: 0,
                block_hash: [0; 32],
                prev_block_hash: [0; 32],
                pivot_hash: [0; 32],
                dag_transaction_hashes: Vec::new(),
                period_data_transaction_hashes: Vec::new(),
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
            dag_transaction_hashes: transaction_hashes_to_bridge(value.dag_transaction_hashes),
            period_data_transaction_hashes: transaction_hashes_to_bridge(
                value.period_data_transaction_hashes,
            ),
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
            dag_transaction_hashes: transaction_hashes_to_bridge(value.dag_transaction_hashes),
            period_data_transaction_hashes: transaction_hashes_to_bridge(
                value.period_data_transaction_hashes,
            ),
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
