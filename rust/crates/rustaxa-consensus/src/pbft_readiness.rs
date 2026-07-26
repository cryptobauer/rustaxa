//! Native PBFT service readiness ownership.
//!
//! This module owns a tiny readiness gate shared by native PBFT service
//! siblings. The value is intentionally tiny, cloneable, and atomic so Rust
//! call sites can coordinate bootstrap transitions without rebuilding it from
//! bridge-owned mutable state.

use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};

/// Cloneable atomic readiness flag for a native PBFT service owner.
///
/// Inputs:
/// - `initially_ready`: whether the service should start in ready state.
///
/// Outputs:
/// - `is_ready() -> bool`: latest readiness value with acquire ordering.
/// - `mark_ready()`: publishes ready to `true` with release ordering.
///
/// Invariants and edge behavior:
/// - readiness is monotonic: values can move from `pending` to `ready`,
///   but never transition from `ready` back to `pending`.
/// - clones share the same `AtomicBool` via `Arc`, so all handles observe the
///   same transition.
/// - no error path is expected: readiness is a pure control flag.
#[derive(Debug, Clone)]
pub struct PbftServiceReadiness {
    ready: Arc<AtomicBool>,
}

impl PbftServiceReadiness {
    /// Creates a new readiness owner with explicit initial state.
    pub fn new(initially_ready: bool) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(initially_ready)),
        }
    }

    /// Creates a new owner in the pending state (`false`).
    pub fn pending() -> Self {
        Self::new(false)
    }

    /// Creates a new owner in the ready state (`true`).
    pub fn ready() -> Self {
        Self::new(true)
    }

    /// Returns the latest readiness value using acquire ordering.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Publishes ready state with release ordering.
    ///
    /// Repeated calls are idempotent and preserve monotonicity.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_service_is_not_ready() {
        let service_readiness = PbftServiceReadiness::pending();

        assert!(!service_readiness.is_ready());
    }

    #[test]
    fn readiness_is_monotonic() {
        let service_readiness = PbftServiceReadiness::pending();

        assert!(!service_readiness.is_ready());

        service_readiness.mark_ready();
        assert!(service_readiness.is_ready());

        service_readiness.mark_ready();
        assert!(service_readiness.is_ready());
    }

    #[test]
    fn clones_share_state() {
        let service_readiness = PbftServiceReadiness::pending();
        let clone = service_readiness.clone();

        assert!(!service_readiness.is_ready());
        assert!(!clone.is_ready());

        clone.mark_ready();

        assert!(service_readiness.is_ready());
        assert!(clone.is_ready());
    }
}
