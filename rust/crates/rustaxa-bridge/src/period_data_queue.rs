use crate::ffi::rustaxa_ffi::{
    PeriodDataQueueEntryRef, PeriodDataQueueLastEntryLookup, PeriodDataQueuePopPlan,
    PeriodDataQueuePushOutcome,
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
        max_pbft_size: u64,
        current_block_cert_votes_count: usize,
    ) -> Result<PeriodDataQueuePushOutcome, anyhow::Error> {
        Ok(self
            .0
            .push(
                entry_id,
                period,
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
            })
            .unwrap_or(PeriodDataQueueLastEntryLookup {
                found: false,
                entry_id: 0,
                period: 0,
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
            use_last_block_cert_votes: value.use_last_block_cert_votes,
            next_entry_id: value.next_entry_id,
            current_period: value.current_period,
            effective_size: value.effective_size,
        }
    }
}
