//! Test-only model of one process-local StateAPI owner epoch.
//!
//! Each fixture owner receives a nonzero epoch when it opens. Requests must
//! name that exact epoch, and a successful discard replaces it with a newly
//! issued epoch without coupling the value to durable concrete generations.

use anyhow::{Result, ensure};
use std::{
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STATE_API_EPOCH: AtomicU64 = AtomicU64::new(1);

fn issue_state_api_epoch() -> u64 {
    let mut epoch = NEXT_STATE_API_EPOCH.load(Ordering::Relaxed);
    while epoch != 0 {
        let successor = epoch.checked_add(1).unwrap_or(0);
        match NEXT_STATE_API_EPOCH.compare_exchange_weak(
            epoch,
            successor,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return epoch,
            Err(observed) => epoch = observed,
        }
    }
    panic!("StateAPI fixture epoch exhausted");
}

/// Current process-local epoch for one fixture-owned StateAPI lifecycle.
pub(crate) struct StateApiEpoch(Cell<u64>);

impl StateApiEpoch {
    /// Issues the nonzero epoch for a newly opened fixture owner.
    pub(crate) fn new() -> Self {
        Self(Cell::new(issue_state_api_epoch()))
    }

    /// Returns the current nonzero owner epoch for a preflight observation.
    pub(crate) fn current(&self) -> u64 {
        let epoch = self.0.get();
        assert_ne!(epoch, 0, "fixture owner epoch must remain valid");
        epoch
    }

    /// Rejects a request prepared by zero or by another owner lifecycle.
    pub(crate) fn validate(&self, expected: u64) -> Result<()> {
        ensure!(
            expected != 0 && expected == self.current(),
            "FINAL_CHAIN_STATE_API_EPOCH_MISMATCH"
        );
        Ok(())
    }

    /// Replaces the validated discarded epoch after the owner reopens state.
    pub(crate) fn replace_after_discard(&self, expected: u64) -> Result<(u64, u64)> {
        self.validate(expected)?;
        let replacement = issue_state_api_epoch();
        ensure!(
            replacement != expected,
            "fixture discard reused its StateAPI epoch"
        );
        self.0.set(replacement);
        Ok((expected, replacement))
    }
}
