//! Fixed execution-read views and one-way adaptation of committed readers.

use super::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};

/// Immutable execution input from a committed reader or a prepared state phase.
///
/// Every account, slot and code read is pinned to the same fixed view for this
/// reader's lifetime. Prepared views must borrow their storage owner so mutation
/// cannot advance the phase while a journal uses it. Their identity describes
/// that phase, not a published generation or a historical query capability;
/// private preparation tokens and FinalChain approval remain owner-controlled.
/// Account storage roots are derived by storage, while slot reads select raw
/// physical history independently of account/trie reachability. Missing physical
/// history still returns `HistoryUnavailable`; neither a prepared identity nor
/// logical nonmembership establishes retention or permits inferred absence.
/// No read creates, repairs, adopts, commits or publishes a database.
///
/// Existing committed readers adapt one-way through the blanket implementation.
/// Implementing this trait alone does not implement [`ConcreteStateRead`] and
/// cannot expose an execution overlay through the committed-query boundary.
pub trait ConcreteExecutionRead {
    /// Returns the fixed committed/prepared phase identity, never a moving head.
    fn identity(&self) -> ConcreteStateIdentity;

    /// Reads exact physical account RLP and matching decoded fields at this view.
    /// Account absence requires authenticated nonmembership or proved coverage.
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError>;

    /// Selects exact raw slot bytes/tombstones from this phase's produced writes
    /// and its pinned committed history. Values are not narrowed or filtered by
    /// account existence; the execution semantic layer owns those decisions.
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>;

    /// Reads immutable staged or committed code by hash. Bytes must match the
    /// hash; consumers also validate reachable account code size and must reject
    /// unavailable referenced code rather than substituting an empty program.
    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>;
}

impl<T: ConcreteStateRead + ?Sized> ConcreteExecutionRead for T {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateRead::identity(self)
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        ConcreteStateRead::account(self, address)
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteStateRead::storage(self, address, key)
    }

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteStateRead::code(self, code_hash)
    }
}
