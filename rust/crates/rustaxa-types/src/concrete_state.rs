//! Concrete execution state contracts shared by the executor and storage.
//!
//! These are domain/read boundaries, not a database schema or publication API.
//! Readers pin one verified period/root and preserve physical bytes separately
//! from decoded values. Infrastructure must establish retained coverage before
//! treating a missing version as absence. FinalChain retains commit authority.

use crate::{FinalChainBlockNumber, FinalChainNonce};
use num_bigint::BigUint;

/// Exact concrete period/root observed by a reader; construction alone does not
/// prove that a database contains this root or authorize adopting that database.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcreteStateIdentity {
    pub period: FinalChainBlockNumber,
    pub state_root: [u8; 32],
}

/// Unsigned, arbitrary-width persisted balance. Unlike the existing bounded
/// native-kernel balance, this value can represent every positive Go account
/// integer. Signed envelope intermediates belong to the executor journal.
/// Physical encoding and preservation of original bytes belong to the codec.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConcreteAccountBalance(BigUint);

impl ConcreteAccountBalance {
    /// Wraps an unsigned domain value without truncation or a width limit.
    pub fn new(value: BigUint) -> Self {
        Self(value)
    }

    /// Borrows the authoritative value; callers choose checked conversion at a
    /// bounded native-kernel or interpreter boundary explicitly.
    pub fn value(&self) -> &BigUint {
        &self.0
    }
}

/// Decoded physical account fields. Optional hashes retain the distinction
/// between an absent hash and an explicitly stored empty-trie/code hash.
/// Code size is the reference's u64; nonce and balance have no u256 cap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteAccount {
    pub nonce: FinalChainNonce,
    pub balance: ConcreteAccountBalance,
    pub storage_root: Option<[u8; 32]>,
    pub code_hash: Option<[u8; 32]>,
    pub code_size: u64,
}

/// Decoded account plus its exact five-field physical RLP, retained for
/// untouched-row preservation. A reader must validate that both describe the
/// same row. The four-field commitment codec is a separate operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteAccountRecord {
    pub account: ConcreteAccount,
    pub physical_rlp: Vec<u8>,
}

/// Logical 32-byte storage key before trie hashing. Both EVM words and native
/// raw keys use this boundary; storage applies the reference's two hashes.
/// This is never a physical RocksDB key or an already hashed trie path.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConcreteStorageKey(pub [u8; 32]);

/// Fixed-view row lookup. Absence is only valid within proved retained coverage;
/// a tombstone is an actual selected empty version and must not resurrect an
/// earlier value. Only the EVM semantic layer maps either to a zero value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteRead<T> {
    Present(T),
    Absent,
    Tombstone,
}

/// Read failures that must never become empty accounts, slots, code or roots.
/// Diagnostics are observability only, not protocol inputs. Unknown historical
/// coverage differs from confirmed pruning; a missing referenced node without
/// pruning evidence is corruption or unavailable coverage, not proved pruning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteReadError {
    Pruned(ConcreteStateIdentity),
    HistoryUnavailable(ConcreteStateIdentity),
    FuturePeriod {
        requested: FinalChainBlockNumber,
        committed: FinalChainBlockNumber,
    },
    IdentityMismatch {
        expected: ConcreteStateIdentity,
        observed: ConcreteStateIdentity,
    },
    Corrupt(String),
    Io(String),
}

impl std::fmt::Display for ConcreteReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "concrete state read: {self:?}")
    }
}

impl std::error::Error for ConcreteReadError {}

/// Immutable concrete-state access owned by one verified generation.
/// Implementations pin database visibility and retained roots for the reader's
/// lifetime. Public queries never read an execution overlay or a mutable latest
/// view. Trie integrity, version selection and I/O checks belong to storage.
/// No method creates a database, repairs it, publishes a head or adopts markers.
pub trait ConcreteStateRead {
    /// Returns the period/root fixed at construction, never a moving head.
    fn identity(&self) -> ConcreteStateIdentity;

    /// Reads the account at an unhashed address, preserving exact physical RLP.
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError>;

    /// Reads committed bytes at a logical slot. Values may exceed 32 bytes and
    /// must not be word-decoded here. Missing accounts are semantically handled
    /// by the executor; this method exposes the physical slot history.
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>;

    /// Reads immutable code whose bytes must hash to the requested hash. Code
    /// rows are unversioned and may have been installed after this generation;
    /// semantic reachability comes from the account at the pinned period.
    /// The consumer must reject missing code for
    /// a live nonzero-size account and verify its declared size; it must not
    /// turn an unavailable referenced code row into an empty program.
    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>;
}

/// Fixed prepared/committed execution views, separate from public committed queries.
pub mod execution;
