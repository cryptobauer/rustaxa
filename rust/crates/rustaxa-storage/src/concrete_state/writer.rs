//! Preparation and unpublished persistence of incremental concrete-state rows.
//!
//! This module deliberately stops before durable head publication. It validates
//! the current descriptor, derives trie roots, and can stage compatible rows in
//! the existing columns. FinalChain lifecycle code must later atomically bind
//! those rows to descriptor, provenance, and publication state.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rlp::{Rlp, RlpStream};
use rocksdb::{ColumnFamilyDescriptor, DB, Direction, IteratorMode, Options, WriteBatch};
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};

use super::codec::{
    account_version_prefix, corrupt, decode_descriptor, decode_physical_account, empty_trie_root,
    keccak256, storage_prefix_for_path, storage_trie_path, storage_version_prefix, versioned_key,
};
use super::physical_node::{
    InventoryLimits, PathProof, PhysicalTrieStore, SelectedVersion, TrieSchema,
    inventory_account_trie, inventory_touched_storage_trie, verify_path,
};
use super::reader::ConcreteStoragePath;
use super::trie_writer::{IncrementalTrie, TrieWriteStore};

const DESCRIPTOR_KEY: &[u8] = b"last_committed_descriptor";
pub(super) const REQUIRED_COLUMNS: &[&str] = &["default", "1", "2", "3", "4", "5", "6", "7", "8"];
static NEXT_WRITER_ID: AtomicU64 = AtomicU64::new(1);
type OrderedStorageChanges = BTreeMap<[u8; 20], Vec<(ConcreteStorageKey, Option<Vec<u8>>)>>;

/// One account-row change. Storage roots in upsert records are never trusted:
/// the writer preserves the authenticated prior root or replaces it with the
/// incrementally derived root when this batch changes slots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteAccountMutation {
    Upsert {
        address: [u8; 20],
        record: ConcreteAccountRecord,
    },
    Delete {
        address: [u8; 20],
    },
}

impl ConcreteStateRead for ConcreteStateWriter {
    fn identity(&self) -> ConcreteStateIdentity {
        self.prior
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.verify_prior_account(address)?;
        match select_version(
            &self.db,
            "3",
            account_version_prefix(address),
            self.prior.period,
        )? {
            Some(value) if value.is_empty() => Ok(ConcreteRead::Tombstone),
            Some(value) => decode_physical_account(&value).map(ConcreteRead::Present),
            None => Ok(ConcreteRead::Absent),
        }
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match select_version(
            &self.db,
            "5",
            storage_version_prefix(address, key),
            self.prior.period,
        )? {
            Some(value) if value.is_empty() => Ok(ConcreteRead::Tombstone),
            Some(value) => Ok(ConcreteRead::Present(value)),
            None => Err(ConcreteReadError::HistoryUnavailable(self.prior)),
        }
    }

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let Some(code) = self.get("1", &code_hash)? else {
            return Err(ConcreteReadError::HistoryUnavailable(self.prior));
        };
        if keccak256(&code) != code_hash {
            return Err(corrupt("code bytes do not match their Keccak-256 key"));
        }
        Ok(ConcreteRead::Present(code))
    }
}

impl ConcreteAccountMutation {
    fn address(&self) -> [u8; 20] {
        match self {
            Self::Upsert { address, .. } | Self::Delete { address } => *address,
        }
    }
}

/// One physical slot-history change at a logical pre-hash key. `None` writes
/// the reference empty version-row tombstone and removes trie membership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteStorageMutation {
    pub address: [u8; 20],
    pub key: ConcreteStorageKey,
    pub value: Option<Vec<u8>>,
}

/// Immutable code installation. The key must be Keccak-256 of `code` and an
/// existing row may only be restaged with identical bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteCodeInsertion {
    pub code_hash: [u8; 32],
    pub code: Vec<u8>,
}

/// Storage-owned mutation input for exactly one consecutive state period.
/// Ordinary EVM slots and native raw slots use the same logical-key boundary;
/// the caller must not pass an already hashed physical key.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConcreteStateMutationBatch {
    pub accounts: Vec<ConcreteAccountMutation>,
    pub storage: Vec<ConcreteStorageMutation>,
    pub code: Vec<ConcreteCodeInsertion>,
}

/// One settled observer phase applied in the exact supplied slot order.
///
/// Account and code keys must be unique. Storage operations may repeat a
/// logical key because ordinary writes precede raw writes and both operations
/// can affect the compatible trie and physical tombstone outcome. Empty live
/// bytes are invalid; use `value: None` for deletion. The caller supplies
/// logical keys, never pre-hashed trie or database keys.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConcreteObserverPhaseDelta {
    pub accounts: Vec<ConcreteAccountMutation>,
    pub storage: Vec<ConcreteStorageMutation>,
    pub code: Vec<ConcreteCodeInsertion>,
}

/// Validated, deterministic compatible rows for one unpublished generation.
/// Fields and construction stay private so callers cannot forge a root/write
/// pairing or retarget rows to another writer/database generation.
#[derive(Debug)]
pub struct PreparedConcreteState {
    prior: ConcreteStateIdentity,
    next: ConcreteStateIdentity,
    writer_id: u64,
    sequence: u64,
    changed_accounts: Vec<([u8; 20], ConcreteRead<ConcreteAccountRecord>)>,
    storage_owners: BTreeSet<[u8; 20]>,
    pub(super) rows: BTreeMap<RowKey, Vec<u8>>,
}

/// Immutable borrowed execution view of one unpublished prepared phase.
///
/// Construction and lifetime remain lifecycle-controlled. Reads use the exact
/// prepared root and in-memory physical rows before the pinned durable prior.
/// Missing durable physical history remains unavailable. This view cannot be
/// used as a committed-state reader and exposes no mutation or publication API.
pub struct PreparedConcreteView<'a> {
    writer: &'a ConcreteStateWriter,
    prepared: &'a PreparedConcreteState,
}

/// One exact account result derived by historical read-only preparation.
///
/// The value is an in-memory summary of a changed account at the derived root.
/// It neither asserts persistence nor carries a writer, row plan, preparation
/// token, lifecycle approval, or publication authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalConcreteAccountChange {
    address: [u8; 20],
    value: ConcreteRead<ConcreteAccountRecord>,
}

impl HistoricalConcreteAccountChange {
    /// Returns the unhashed account address selected by the mutation batch.
    pub fn address(&self) -> [u8; 20] {
        self.address
    }

    /// Borrows the exact derived account result and physical RLP, if present.
    pub fn value(&self) -> &ConcreteRead<ConcreteAccountRecord> {
        &self.value
    }
}

/// Read-only result of deriving one period from a retained historical root.
///
/// The identity and account values describe an in-memory incremental-trie
/// calculation. Row count is diagnostic only. This type contains no rows,
/// writer identity, sequence, persistence method, or commit approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalConcretePreparationSummary {
    next: ConcreteStateIdentity,
    row_count: usize,
    changed_accounts: Vec<HistoricalConcreteAccountChange>,
}

impl HistoricalConcretePreparationSummary {
    /// Returns the identity derived from the historical root and supplied mutations.
    pub fn next_identity(&self) -> ConcreteStateIdentity {
        self.next
    }

    /// Returns the number of compatible rows calculated but never persisted.
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    /// Borrows exact read summaries for accounts changed by the calculation.
    pub fn changed_accounts(&self) -> &[HistoricalConcreteAccountChange] {
        &self.changed_accounts
    }
}

/// Read-only incremental preparation over a retained historical state root.
///
/// Opening independently pins the durable current descriptor and an older
/// base identity. The database uses RocksDB's read-only API. Preparation reuses
/// the compatible incremental writer while keeping its prepared rows and token
/// private; only a root and changed-account summaries escape. Missing or corrupt
/// trie/value/code dependencies fail closed. Construction verifies one
/// deterministic account path against the base root, so even an empty mutation
/// batch cannot bless an arbitrary historical root; touched paths authenticate
/// their complete demanded closure. The caller must establish the historical
/// period/root binding from qualified application headers; this path check does
/// not prove that binding or retention of the entire trie.
///
/// This surface deliberately cannot persist or publish:
///
/// ```compile_fail
/// use rustaxa_storage::{HistoricalConcretePreparation, ConcreteStateMutationBatch};
/// use rustaxa_types::FinalChainBlockNumber;
///
/// fn cannot_publish(handle: &HistoricalConcretePreparation) {
///     let summary = handle
///         .prepare(FinalChainBlockNumber::new(1), ConcreteStateMutationBatch::default())
///         .unwrap();
///     handle.persist_contents(&summary).unwrap();
/// }
/// ```
pub struct HistoricalConcretePreparation {
    writer: ConcreteStateWriter,
}

/// Read-only, ordered execution preparation over one retained prior state.
///
/// No writable owner may be open for this path while this handle exists.
/// The caller must bind the historical identity to qualified application headers
/// and concrete owner provenance. Active candidate execution requires the exact
/// predecessor, with `historical == committed`; opening an older root does not
/// provide that authority. The initial path check does not prove full retention.
/// No prepared rows, writer, lifecycle token or publication capability escapes.
///
/// ```compile_fail
/// use rustaxa_storage::ConcreteExecutionProbe;
/// fn cannot_persist(probe: &ConcreteExecutionProbe) {
///     probe.persist_contents();
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{ConcreteCommitApproval, ConcreteExecutionProbe};
/// fn cannot_approve(probe: &ConcreteExecutionProbe) -> ConcreteCommitApproval {
///     probe.commit_approval()
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{ConcreteExecutionProbe, PreparedConcreteState};
/// fn cannot_extract(probe: ConcreteExecutionProbe) -> PreparedConcreteState {
///     probe.prepared.unwrap()
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::ConcreteExecutionProbe;
/// use rustaxa_types::concrete_state::ConcreteStateRead;
/// fn committed(_: &impl ConcreteStateRead) {}
/// fn cannot_query_committed(probe: &ConcreteExecutionProbe) {
///     committed(probe);
/// }
/// ```
pub struct ConcreteExecutionProbe {
    writer: ConcreteStateWriter,
    prepared: Option<PreparedConcreteState>,
    next_period: FinalChainBlockNumber,
}

/// Memory-only execution over a clean lifecycle's existing database handle.
/// The owner stays borrowed until the probe and its views are no longer used.
/// This capability does not authenticate native state or grant vote authority.
///
/// ```compile_fail
/// use rustaxa_storage::ConcreteStateLifecycle;
/// fn cannot_stage(owner: &mut ConcreteStateLifecycle) {
///     let probe = owner.borrow_execution_probe().unwrap();
///     owner.stage_execution(&[]).unwrap();
///     let _ = probe.current_view();
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::ConcreteStateLifecycle;
/// fn cannot_discard(owner: &mut ConcreteStateLifecycle) {
///     let probe = owner.borrow_execution_probe().unwrap();
///     owner.discard_execution(&[]).unwrap();
///     let _ = probe.current_view();
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{ConcreteStateLifecycle, ConcreteCommitApproval, PreparedConcreteState};
/// fn cannot_commit(owner: ConcreteStateLifecycle, state: PreparedConcreteState, approval: ConcreteCommitApproval) {
///     let probe = owner.borrow_execution_probe().unwrap();
///     owner.commit_approved(state, approval).unwrap();
///     let _ = probe.current_view();
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{ConcreteStateLifecycle, BorrowedConcreteExecutionProbe};
/// fn cannot_escape(owner: ConcreteStateLifecycle) -> BorrowedConcreteExecutionProbe<'static> {
///     owner.borrow_execution_probe().unwrap()
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::BorrowedConcreteExecutionProbe;
/// fn cannot_extract(probe: BorrowedConcreteExecutionProbe<'_>) { let _ = probe.probe; }
/// ```
/// ```compile_fail
/// use rustaxa_storage::BorrowedConcreteExecutionProbe;
/// fn cannot_publish(probe: BorrowedConcreteExecutionProbe<'_>) { probe.persist_contents(); }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{BorrowedConcreteExecutionProbe, ConcreteStateWriter};
/// fn cannot_construct(writer: &ConcreteStateWriter) {
///     let _ = BorrowedConcreteExecutionProbe::from_owner(writer);
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{BorrowedConcreteExecutionProbe, ConcreteObserverPhaseDelta};
/// fn cannot_advance(probe: &mut BorrowedConcreteExecutionProbe<'_>) {
///     let view = probe.current_view().unwrap();
///     probe.prepare_observer_phase(ConcreteObserverPhaseDelta::default()).unwrap();
///     let _ = view.verify_storage_path([0; 20], rustaxa_types::concrete_state::ConcreteStorageKey([0; 32]));
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::{BorrowedConcreteExecutionProbe, ConcreteExecutionProbeView};
/// fn cannot_keep_view(probe: BorrowedConcreteExecutionProbe<'_>) -> ConcreteExecutionProbeView<'static> {
///     probe.current_view().unwrap()
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::BorrowedConcreteExecutionProbe;
/// use rustaxa_types::concrete_state::ConcreteStateRead;
/// fn committed(_: &impl ConcreteStateRead) {}
/// fn cannot_read_committed(probe: &BorrowedConcreteExecutionProbe<'_>) { committed(probe); }
/// ```
/// ```compile_fail
/// use rustaxa_storage::BorrowedConcreteExecutionProbe;
/// use rustaxa_types::concrete_state::ConcreteStateRead;
/// fn committed(_: &impl ConcreteStateRead) {}
/// fn cannot_read_committed(probe: &BorrowedConcreteExecutionProbe<'_>) {
///     committed(&probe.current_view().unwrap());
/// }
/// ```
pub struct BorrowedConcreteExecutionProbe<'owner> {
    probe: ConcreteExecutionProbe,
    _owner: &'owner ConcreteStateWriter,
}

impl<'owner> BorrowedConcreteExecutionProbe<'owner> {
    pub(super) fn from_owner(
        owner: &'owner ConcreteStateWriter,
    ) -> Result<Self, ConcreteReadError> {
        if owner.prior != owner.durable_prior
            || current_identity(&owner.db)? != owner.durable_prior
            || owner.persisted_sequence.get().is_some()
        {
            return Err(corrupt("execution probe owner is not clean and committed"));
        }
        let next_period = owner
            .prior
            .period
            .checked_next()
            .ok_or_else(|| corrupt("execution probe prior period overflow"))?;
        Ok(Self {
            probe: ConcreteExecutionProbe {
                writer: ConcreteStateWriter {
                    db: Arc::clone(&owner.db),
                    path: owner.path.clone(),
                    prior: owner.prior,
                    durable_prior: owner.durable_prior,
                    writer_id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
                    sequence: Cell::new(0),
                    persisted_sequence: Cell::new(None),
                },
                prepared: None,
                next_period,
            },
            _owner: owner,
        })
    }

    /// Returns the owner's pinned durable descriptor.
    pub fn committed_identity(&self) -> ConcreteStateIdentity {
        self.probe.committed_identity()
    }

    /// Returns the current committed prior used for every prepared phase.
    pub fn historical_identity(&self) -> ConcreteStateIdentity {
        self.probe.historical_identity()
    }

    /// Applies one ordered phase in private memory; errors retain the prior phase.
    pub fn prepare_observer_phase(
        &mut self,
        delta: ConcreteObserverPhaseDelta,
    ) -> Result<(), ConcreteReadError> {
        self.probe.prepare_observer_phase(delta)
    }

    /// Borrows the current execution view without publication authority.
    pub fn current_view(&self) -> Result<ConcreteExecutionProbeView<'_>, ConcreteReadError> {
        self.probe.current_view()
    }
}

/// Borrowed execution-only view of the exact prior or current prepared phase.
///
/// ```compile_fail
/// use rustaxa_storage::{ConcreteExecutionProbe, ConcreteObserverPhaseDelta};
/// use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
/// fn cannot_advance(probe: &mut ConcreteExecutionProbe) {
///     let view = probe.current_view().unwrap();
///     probe.prepare_observer_phase(ConcreteObserverPhaseDelta::default()).unwrap();
///     let _ = view.identity();
/// }
/// ```
/// ```compile_fail
/// use rustaxa_storage::ConcreteExecutionProbe;
/// use rustaxa_types::concrete_state::ConcreteStateRead;
/// fn committed(_: &impl ConcreteStateRead) {}
/// fn cannot_query_committed(probe: &ConcreteExecutionProbe) {
///     committed(&probe.current_view().unwrap());
/// }
/// ```
pub struct ConcreteExecutionProbeView<'a> {
    writer: &'a ConcreteStateWriter,
    prepared: Option<&'a PreparedConcreteState>,
}

impl ConcreteExecutionProbe {
    /// Opens existing state read-only with exact current and historical checks.
    /// Missing/corrupt root dependencies and a period overflow fail closed.
    pub fn open_read_only(
        path: impl AsRef<Path>,
        committed: ConcreteStateIdentity,
        historical: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        let next_period = historical
            .period
            .checked_next()
            .ok_or_else(|| corrupt("execution probe prior period overflow"))?;
        let historical =
            HistoricalConcretePreparation::open_read_only(path, committed, historical)?;
        Ok(Self {
            writer: historical.writer,
            prepared: None,
            next_period,
        })
    }

    /// Returns the exact durable identity pinned at open.
    pub fn committed_identity(&self) -> ConcreteStateIdentity {
        self.writer.durable_prior
    }

    /// Returns the retained prior identity used by every phase.
    pub fn historical_identity(&self) -> ConcreteStateIdentity {
        self.writer.prior
    }

    /// Applies one settled phase in supplied order, entirely in memory.
    /// On error, the previous phase remains usable. Repeated slot operations
    /// are not reduced to a final-value map before trie preparation.
    pub fn prepare_observer_phase(
        &mut self,
        delta: ConcreteObserverPhaseDelta,
    ) -> Result<(), ConcreteReadError> {
        let next =
            self.writer
                .prepare_observer_phase(self.next_period, self.prepared.as_ref(), delta)?;
        self.prepared = Some(next);
        Ok(())
    }

    /// Borrows the exact current phase without committed-query authority.
    pub fn current_view(&self) -> Result<ConcreteExecutionProbeView<'_>, ConcreteReadError> {
        match self.prepared.as_ref() {
            Some(prepared) => self.writer.validate_prepared(prepared)?,
            None if current_identity(&self.writer.db)? != self.writer.durable_prior => {
                return Err(corrupt("execution probe descriptor changed after open"));
            }
            None => {}
        }
        Ok(ConcreteExecutionProbeView {
            writer: &self.writer,
            prepared: self.prepared.as_ref(),
        })
    }
}

impl ConcreteExecutionProbeView<'_> {
    /// Authenticates logical membership separately from raw physical history.
    /// Nonmembership does not supply a missing raw row or imply a zero value.
    pub fn verify_storage_path(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        match self.prepared {
            Some(prepared) => self
                .writer
                .prepared_view(prepared)?
                .verify_storage_path(address, key),
            None => self.writer.verify_prior_storage_path(address, key),
        }
    }
}

impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead
    for ConcreteExecutionProbeView<'_>
{
    fn identity(&self) -> ConcreteStateIdentity {
        self.prepared
            .map_or(self.writer.prior, |prepared| prepared.next)
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        match self.prepared {
            Some(prepared) => self.writer.prepared_account(prepared, address),
            None => ConcreteStateRead::account(self.writer, address),
        }
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match self.prepared {
            Some(prepared) => self.writer.prepared_storage(prepared, address, key),
            None => ConcreteStateRead::storage(self.writer, address, key),
        }
    }

    fn ordinary_storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match self.storage(address, key) {
            Err(error @ ConcreteReadError::HistoryUnavailable(identity))
                if identity == self.identity() =>
            {
                match self.verify_storage_path(address, key)? {
                    ConcreteStoragePath::NonMember => Ok(ConcreteRead::Absent),
                    ConcreteStoragePath::Member(_) => Err(error),
                }
            }
            result => result,
        }
    }

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match self.prepared {
            Some(prepared) => self.writer.prepared_code(prepared, code_hash),
            None => ConcreteStateRead::code(self.writer, code_hash),
        }
    }
}

impl HistoricalConcretePreparation {
    /// Opens existing columns read-only and pins current and historical identities.
    /// No writable owner may be open for this path while this handle exists.
    ///
    /// `committed` must exactly equal the durable descriptor. `historical` may
    /// be older, but cannot be future; when periods match, roots must match.
    /// A deterministic account proof must authenticate against the historical root.
    pub fn open_read_only(
        path: impl AsRef<Path>,
        committed: ConcreteStateIdentity,
        historical: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        let path = path.as_ref();
        let db = open_existing_database(path, true)?;
        let observed = current_identity(&db)?;
        if committed.period.as_u64() > observed.period.as_u64() {
            return Err(ConcreteReadError::FuturePeriod {
                requested: committed.period,
                committed: observed.period,
            });
        }
        if committed != observed {
            return Err(ConcreteReadError::IdentityMismatch {
                expected: committed,
                observed,
            });
        }
        if historical.period.as_u64() > committed.period.as_u64() {
            return Err(ConcreteReadError::FuturePeriod {
                requested: historical.period,
                committed: committed.period,
            });
        }
        if historical.period == committed.period && historical.state_root != committed.state_root {
            return Err(ConcreteReadError::IdentityMismatch {
                expected: historical,
                observed: committed,
            });
        }
        let writer = ConcreteStateWriter {
            db: Arc::new(db),
            path: path.to_path_buf(),
            prior: historical,
            durable_prior: committed,
            writer_id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
            sequence: Cell::new(0),
            persisted_sequence: Cell::new(None),
        };
        // Authenticate one deterministic path so an empty batch cannot return a
        // caller-injected root without resolving and checking its physical node.
        writer.verify_prior_account([0; 20])?;
        Ok(Self { writer })
    }

    /// Returns the durable descriptor independently pinned when the handle opened.
    pub fn committed_identity(&self) -> ConcreteStateIdentity {
        self.writer.durable_prior
    }

    /// Returns the retained historical identity used as the preparation base.
    pub fn historical_identity(&self) -> ConcreteStateIdentity {
        self.writer.prior
    }

    /// Derives exactly the next historical period without mutating RocksDB.
    ///
    /// The next period must immediately follow the historical base. Account,
    /// storage, and code mutations retain normal writer validation. Returned
    /// account values and root have no durable or publication authority.
    pub fn prepare(
        &self,
        next_period: FinalChainBlockNumber,
        mutations: ConcreteStateMutationBatch,
    ) -> Result<HistoricalConcretePreparationSummary, ConcreteReadError> {
        let prepared = self.writer.prepare(next_period, mutations)?;
        Ok(HistoricalConcretePreparationSummary {
            next: prepared.next_identity(),
            row_count: prepared.row_count(),
            changed_accounts: prepared
                .changed_accounts()
                .iter()
                .map(|(address, value)| HistoricalConcreteAccountChange {
                    address: *address,
                    value: value.clone(),
                })
                .collect(),
        })
    }
}

impl PreparedConcreteView<'_> {
    /// Proves one logical storage path in this exact unpublished preparation.
    /// Missing or invalid proof dependencies fail; raw history stays separate.
    /// This proof does not publish state or provide committed-reader authority.
    pub fn verify_storage_path(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        self.writer
            .verify_prepared_storage_path(self.prepared, address, key)
    }
}

impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead for PreparedConcreteView<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.prepared.next
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.writer.prepared_account(self.prepared, address)
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.writer.prepared_storage(self.prepared, address, key)
    }

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.writer.prepared_code(self.prepared, code_hash)
    }
}

impl PreparedConcreteState {
    /// Identity derived from the prepared account trie. This is not a published
    /// descriptor and does not attest durable lifecycle completion.
    pub fn next_identity(&self) -> ConcreteStateIdentity {
        self.next
    }

    /// Number of compatible column-family rows staged by this preparation.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn changed_accounts(&self) -> &[([u8; 20], ConcreteRead<ConcreteAccountRecord>)] {
        &self.changed_accounts
    }

    pub(super) fn token(&self) -> (u64, u64) {
        (self.writer_id, self.sequence)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RowKey {
    pub(super) column: &'static str,
    pub(super) key: Vec<u8>,
}

/// Mutable handle for compatible concrete-state preparation. Opening requires
/// an exact descriptor match. The handle neither creates a database nor
/// publishes descriptors, provenance, catalogs, or application markers.
pub struct ConcreteStateWriter {
    pub(super) db: Arc<DB>,
    path: PathBuf,
    prior: ConcreteStateIdentity,
    durable_prior: ConcreteStateIdentity,
    writer_id: u64,
    sequence: Cell<u64>,
    persisted_sequence: Cell<Option<u64>>,
}

pub(super) struct ConcreteTransitionWriter {
    writer: ConcreteStateWriter,
    prepared: Option<PreparedConcreteState>,
    source_token: (u64, u64),
    next_period: FinalChainBlockNumber,
}

pub(super) struct SealedConcreteTransition {
    source_token: (u64, u64),
    prior: ConcreteStateIdentity,
    next: ConcreteStateIdentity,
    changed_accounts: Vec<([u8; 20], ConcreteRead<ConcreteAccountRecord>)>,
    storage_owners: BTreeSet<[u8; 20]>,
    rows: BTreeMap<RowKey, Vec<u8>>,
}

impl SealedConcreteTransition {
    pub(super) fn account_update(
        &self,
        address: [u8; 20],
    ) -> Result<Option<ConcreteRead<ConcreteAccountRecord>>, ConcreteReadError> {
        let key = RowKey {
            column: "3",
            key: versioned_key(account_version_prefix(address), self.next.period).to_vec(),
        };
        match self.rows.get(&key) {
            None => Ok(None),
            Some(row) if row.is_empty() => Ok(Some(ConcreteRead::Tombstone)),
            Some(row) => decode_physical_account(row)
                .map(ConcreteRead::Present)
                .map(Some),
        }
    }

    pub(super) fn next_identity(&self) -> ConcreteStateIdentity {
        self.next
    }
}

impl ConcreteTransitionWriter {
    pub(super) fn verify_source(
        &self,
        owner: &ConcreteStateWriter,
    ) -> Result<(), ConcreteReadError> {
        if owner.transition_source_token()? != self.source_token {
            return Err(corrupt("concrete transition source changed"));
        }
        Ok(())
    }

    pub(super) fn current_view(&self) -> Result<ConcreteExecutionProbeView<'_>, ConcreteReadError> {
        match self.prepared.as_ref() {
            Some(prepared) => self.writer.validate_prepared(prepared)?,
            None => {
                self.writer.transition_source_token()?;
            }
        }
        Ok(ConcreteExecutionProbeView {
            writer: &self.writer,
            prepared: self.prepared.as_ref(),
        })
    }

    pub(super) fn apply_phase(
        &mut self,
        delta: ConcreteObserverPhaseDelta,
    ) -> Result<&PreparedConcreteState, ConcreteReadError> {
        let prepared =
            self.writer
                .prepare_observer_phase(self.next_period, self.prepared.as_ref(), delta)?;
        self.prepared = Some(prepared);
        self.latest()
    }

    pub(super) fn latest(&self) -> Result<&PreparedConcreteState, ConcreteReadError> {
        let prepared = self
            .prepared
            .as_ref()
            .ok_or_else(|| corrupt("concrete transition has no successful phase"))?;
        self.writer.validate_prepared(prepared)?;
        Ok(prepared)
    }

    pub(super) fn seal(self) -> Result<SealedConcreteTransition, ConcreteReadError> {
        self.latest()?;
        let prepared = self
            .prepared
            .ok_or_else(|| corrupt("concrete transition has no successful phase"))?;
        Ok(SealedConcreteTransition {
            source_token: self.source_token,
            prior: prepared.prior,
            next: prepared.next,
            changed_accounts: prepared.changed_accounts,
            storage_owners: prepared.storage_owners,
            rows: prepared.rows,
        })
    }
}

impl ConcreteStateWriter {
    pub(super) fn transition_source_token(&self) -> Result<(u64, u64), ConcreteReadError> {
        self.ensure_durable_prior()?;
        if current_identity(&self.db)? != self.durable_prior
            || self.persisted_sequence.get().is_some()
        {
            return Err(corrupt(
                "concrete transition source is not clean and committed",
            ));
        }
        Ok((self.writer_id, self.sequence.get()))
    }

    pub(super) fn begin_isolated_transition(
        &self,
    ) -> Result<ConcreteTransitionWriter, ConcreteReadError> {
        let source_token = self.transition_source_token()?;
        let next_period = self
            .prior
            .period
            .checked_next()
            .ok_or_else(|| corrupt("concrete transition prior period overflow"))?;
        Ok(ConcreteTransitionWriter {
            writer: Self {
                db: Arc::clone(&self.db),
                path: self.path.clone(),
                prior: self.prior,
                durable_prior: self.durable_prior,
                writer_id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
                sequence: Cell::new(0),
                persisted_sequence: Cell::new(None),
            },
            prepared: None,
            source_token,
            next_period,
        })
    }

    pub(super) fn prepare_transition_import(
        &self,
        sealed: SealedConcreteTransition,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        if self.transition_source_token()? != sealed.source_token
            || sealed.prior != self.prior
            || self.prior.period.checked_next() != Some(sealed.next.period)
        {
            return Err(corrupt("concrete transition source is stale or foreign"));
        }
        let sequence = self
            .sequence
            .get()
            .checked_add(1)
            .ok_or_else(|| corrupt("concrete writer preparation sequence overflow"))?;
        Ok(PreparedConcreteState {
            prior: sealed.prior,
            next: sealed.next,
            writer_id: self.writer_id,
            sequence,
            changed_accounts: sealed.changed_accounts,
            storage_owners: sealed.storage_owners,
            rows: sealed.rows,
        })
    }

    pub(super) fn verify_transition_import(
        &self,
        prepared: &PreparedConcreteState,
    ) -> Result<(), ConcreteReadError> {
        let (writer_id, sequence) = self.transition_source_token()?;
        if prepared.writer_id != writer_id
            || sequence.checked_add(1) != Some(prepared.sequence)
            || prepared.prior != self.prior
            || self.prior.period.checked_next() != Some(prepared.next.period)
        {
            return Err(corrupt("certified concrete import is stale or foreign"));
        }
        Ok(())
    }

    pub(super) fn install_transition_sequence(&self, prepared: &PreparedConcreteState) {
        self.sequence.set(prepared.sequence);
    }

    pub(super) fn install_committed_identity(&mut self, committed: ConcreteStateIdentity) {
        self.prior = committed;
        self.durable_prior = committed;
    }

    /// Opens all existing reference columns read-write and verifies `prior`
    /// against the persisted descriptor. No missing file or column is created.
    pub fn open(
        path: impl AsRef<Path>,
        prior: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        let path = path.as_ref();
        let db = open_existing_database(path, false)?;
        let observed = current_identity(&db)?;
        if observed != prior {
            return Err(ConcreteReadError::IdentityMismatch {
                expected: prior,
                observed,
            });
        }
        Ok(Self {
            db: Arc::new(db),
            path: path.to_path_buf(),
            prior,
            durable_prior: prior,
            writer_id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
            sequence: Cell::new(0),
            persisted_sequence: Cell::new(None),
        })
    }

    /// Derives incremental roots and compatible rows without mutating RocksDB.
    /// The requested period must immediately follow the pinned descriptor.
    pub fn prepare(
        &self,
        next_period: FinalChainBlockNumber,
        mutations: ConcreteStateMutationBatch,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        self.prepare_inner(next_period, mutations, false)
    }

    pub(super) fn prepare_genesis(
        &self,
        mutations: ConcreteStateMutationBatch,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        self.prepare_inner(FinalChainBlockNumber::GENESIS, mutations, true)
    }

    #[cfg(feature = "synthetic-fixtures")]
    pub(super) fn prepare_synthetic_initial_state(
        &self,
        period: FinalChainBlockNumber,
        mutations: ConcreteStateMutationBatch,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        self.prepare_inner(period, mutations, true)
    }

    fn prepare_inner(
        &self,
        next_period: FinalChainBlockNumber,
        mutations: ConcreteStateMutationBatch,
        genesis: bool,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        self.validate_preparation_base(next_period, None, genesis)?;
        let mut account_changes = BTreeMap::new();
        for mutation in mutations.accounts {
            if account_changes
                .insert(mutation.address(), mutation)
                .is_some()
            {
                return Err(corrupt("duplicate account mutation"));
            }
        }
        let mut storage_changes = OrderedStorageChanges::new();
        for mutation in mutations.storage {
            validate_storage_value(&mutation.value)?;
            let slots = storage_changes.entry(mutation.address).or_default();
            if slots.iter().any(|(key, _)| *key == mutation.key) {
                return Err(corrupt("duplicate storage mutation"));
            }
            slots.push((mutation.key, mutation.value));
        }
        let code_changes = normalize_code_insertions(mutations.code)?;
        self.prepare_from_base(
            next_period,
            None,
            account_changes,
            storage_changes,
            code_changes,
        )
    }

    /// Applies one ordered observer phase to the supplied base, or to the
    /// durable prior for the first phase. Rows remain unpublished and in memory.
    /// Repeated storage keys are processed in supplied order.
    pub(super) fn prepare_observer_phase(
        &self,
        next_period: FinalChainBlockNumber,
        base: Option<&PreparedConcreteState>,
        delta: ConcreteObserverPhaseDelta,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        self.validate_preparation_base(next_period, base, false)?;
        let mut account_changes = BTreeMap::new();
        for mutation in delta.accounts {
            if account_changes
                .insert(mutation.address(), mutation)
                .is_some()
            {
                return Err(corrupt("duplicate account mutation"));
            }
        }
        let mut storage_changes = OrderedStorageChanges::new();
        for mutation in delta.storage {
            validate_storage_value(&mutation.value)?;
            storage_changes
                .entry(mutation.address)
                .or_default()
                .push((mutation.key, mutation.value));
        }
        let code_changes = normalize_code_insertions(delta.code)?;
        self.prepare_from_base(
            next_period,
            base,
            account_changes,
            storage_changes,
            code_changes,
        )
    }

    fn validate_preparation_base(
        &self,
        next_period: FinalChainBlockNumber,
        base: Option<&PreparedConcreteState>,
        genesis: bool,
    ) -> Result<(), ConcreteReadError> {
        if genesis {
            if base.is_some()
                || self.prior.period != FinalChainBlockNumber::GENESIS
                || self.prior.state_root != empty_trie_root()
                || self.db.get(DESCRIPTOR_KEY).map_err(io)?.is_some()
            {
                return Err(corrupt(
                    "genesis preparation requires a fresh descriptorless database",
                ));
            }
        } else {
            if current_identity(&self.db)? != self.durable_prior {
                return Err(corrupt("concrete descriptor changed after writer open"));
            }
            match base {
                Some(prepared) => {
                    self.validate_prepared(prepared)?;
                    if prepared.next.period != next_period {
                        return Err(corrupt(
                            "observer phases must remain in one prepared period",
                        ));
                    }
                }
                None if self.prior.period.checked_next() != Some(next_period) => {
                    return Err(corrupt(
                        "prepared state period must immediately follow its prior",
                    ));
                }
                None => {}
            }
        }
        if self.persisted_sequence.get().is_some() {
            return Err(corrupt(
                "this writer already staged unpublished rows for its prior",
            ));
        }
        Ok(())
    }

    fn prepare_from_base(
        &self,
        next_period: FinalChainBlockNumber,
        base: Option<&PreparedConcreteState>,
        mut account_changes: BTreeMap<[u8; 20], ConcreteAccountMutation>,
        storage_changes: OrderedStorageChanges,
        code_changes: BTreeMap<[u8; 32], Vec<u8>>,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        let sequence = self
            .sequence
            .get()
            .checked_add(1)
            .ok_or_else(|| corrupt("concrete writer preparation sequence overflow"))?;
        for address in storage_changes.keys() {
            if matches!(
                account_changes.get(address),
                Some(ConcreteAccountMutation::Delete { .. })
            ) {
                return Err(corrupt(
                    "account deletion cannot be combined with storage mutations",
                ));
            }
        }

        let storage_touched = storage_changes.keys().copied().collect::<BTreeSet<_>>();
        let mut storage_owners =
            base.map_or_else(BTreeSet::new, |prepared| prepared.storage_owners.clone());
        storage_owners.extend(storage_touched.iter().copied());
        let touched_accounts = account_changes
            .keys()
            .copied()
            .chain(storage_changes.keys().copied())
            .collect::<BTreeSet<_>>();
        for address in touched_accounts {
            self.verify_account_at(base, address)?;
        }

        let base_identity = base.map_or(self.prior, |prepared| prepared.next);
        let mut rows = base.map_or_else(BTreeMap::new, |prepared| prepared.rows.clone());
        for (address, slots) in storage_changes {
            let prior_record = self.select_account_at(base, address)?;
            let explicit = account_changes.get(&address);
            let base_record = match explicit {
                Some(ConcreteAccountMutation::Upsert { record, .. }) => {
                    validate_record(record)?;
                    Some(record.clone())
                }
                Some(ConcreteAccountMutation::Delete { .. }) => unreachable!("rejected above"),
                None => prior_record.clone(),
            };
            let prior_storage_root = prior_record
                .as_ref()
                .and_then(|record| record.account.storage_root)
                .unwrap_or_else(empty_trie_root);
            let store = PreparedTrieStore {
                writer: self,
                prepared: base,
                identity: base_identity,
                node_column: "4",
                value_column: "5",
                address: Some(address),
            };
            let unique_keys = slots.iter().map(|(key, _)| *key).collect::<BTreeSet<_>>();
            for key in unique_keys {
                let path = storage_trie_path(key);
                let proof = verify_path(
                    &store,
                    prior_storage_root,
                    path,
                    "4",
                    "5",
                    |path| storage_prefix_for_path(address, path),
                    TrieSchema::Storage,
                )?;
                if let PathProof::Member(proved) = proof {
                    let selected =
                        store.value_for_prefix(storage_prefix_for_path(address, path))?;
                    if selected.as_deref() != Some(proved.as_slice()) {
                        return Err(corrupt(
                            "prepared storage trie member differs from its physical version",
                        ));
                    }
                }
            }

            let mut any_changed = false;
            let mut trie =
                IncrementalTrie::new(&store, "4", TrieSchema::Storage, prior_storage_root);
            for (key, value) in slots {
                let path = storage_trie_path(key);
                let changed = match &value {
                    Some(value) => {
                        trie.put(path, value.clone())?;
                        true
                    }
                    None => trie.delete(path)?,
                };
                if changed {
                    any_changed = true;
                    merge_prepared_row(
                        &mut rows,
                        "5",
                        versioned_key(storage_version_prefix(address, key), next_period).to_vec(),
                        value.unwrap_or_default(),
                    )?;
                }
            }
            let commit = trie.commit()?;
            for (hash, node) in commit.nodes {
                merge_prepared_row(&mut rows, "4", hash.to_vec(), node)?;
            }
            if explicit.is_some() || any_changed {
                let root = (commit.root != empty_trie_root()).then_some(commit.root);
                let base_record = base_record.ok_or_else(|| {
                    corrupt("live storage insertion has no prior or upserted account")
                })?;
                let record = replace_storage_root(&base_record, root)?;
                account_changes
                    .insert(address, ConcreteAccountMutation::Upsert { address, record });
            }
        }

        for (address, mutation) in &mut account_changes {
            if storage_touched.contains(address) {
                continue;
            }
            if let ConcreteAccountMutation::Upsert { record, .. } = mutation {
                let prior_root = self
                    .select_account_at(base, *address)?
                    .and_then(|prior| prior.account.storage_root);
                *record = replace_storage_root(record, prior_root)?;
            }
        }

        let main_store = PreparedTrieStore {
            writer: self,
            prepared: base,
            identity: base_identity,
            node_column: "2",
            value_column: "3",
            address: None,
        };
        let mut main = IncrementalTrie::new(
            &main_store,
            "2",
            TrieSchema::Account,
            base_identity.state_root,
        );
        let mut changed_account_addresses = Vec::new();
        for (address, mutation) in account_changes {
            let path = account_version_prefix(address);
            let value = match mutation {
                ConcreteAccountMutation::Upsert { record, .. } => {
                    validate_record(&record)?;
                    self.validate_code_reference_at(&record, &code_changes, base)?;
                    main.put(path, record.physical_rlp.clone())?;
                    Some(record.physical_rlp)
                }
                ConcreteAccountMutation::Delete { .. } => main.delete(path)?.then(Vec::new),
            };
            if let Some(value) = value {
                changed_account_addresses.push(address);
                merge_prepared_row(
                    &mut rows,
                    "3",
                    versioned_key(path, next_period).to_vec(),
                    value,
                )?;
            }
        }
        let main_commit = main.commit()?;
        for (hash, node) in main_commit.nodes {
            merge_prepared_row(&mut rows, "2", hash.to_vec(), node)?;
        }
        for (code_hash, code) in code_changes {
            if let Some(existing) = self.code_at(base, code_hash)?
                && existing != code
            {
                return Err(corrupt("immutable code key already has different bytes"));
            }
            merge_prepared_row(&mut rows, "1", code_hash.to_vec(), code)?;
        }

        let mut prepared = PreparedConcreteState {
            prior: self.prior,
            next: ConcreteStateIdentity {
                period: next_period,
                state_root: main_commit.root,
            },
            writer_id: self.writer_id,
            sequence,
            changed_accounts: Vec::new(),
            storage_owners,
            rows,
        };
        for address in changed_account_addresses {
            self.verify_account_at(Some(&prepared), address)?;
            let row = prepared
                .rows
                .get(&RowKey {
                    column: "3",
                    key: versioned_key(account_version_prefix(address), next_period).to_vec(),
                })
                .ok_or_else(|| corrupt("prepared account change has no physical row"))?;
            let account = if row.is_empty() {
                ConcreteRead::Tombstone
            } else {
                ConcreteRead::Present(decode_physical_account(row)?)
            };
            prepared.changed_accounts.push((address, account));
        }
        self.sequence.set(sequence);
        Ok(prepared)
    }

    pub(super) fn retain_final_trie_nodes(
        &self,
        prepared: &mut PreparedConcreteState,
        limits: super::reader::ConcreteStorageInventoryLimits,
    ) -> Result<(), super::reader::ConcreteStorageInventoryError> {
        use super::reader::ConcreteStorageInventoryError as Error;
        self.validate_prepared(prepared).map_err(Error::Read)?;
        let account_store = PreparedTrieStore {
            writer: self,
            prepared: Some(prepared),
            identity: prepared.next,
            node_column: "2",
            value_column: "3",
            address: None,
        };
        let accounts = inventory_account_trie(
            &account_store,
            prepared.next.state_root,
            |path| path,
            InventoryLimits {
                max_nodes: limits.max_nodes,
                max_leaves: limits.max_leaves,
                max_value_bytes: limits.max_value_bytes,
            },
        )
        .map_err(super::reader::map_inventory_error)?;
        let mut nodes = accounts.nodes_visited;
        let mut leaves = accounts.entries.len() as u64;
        let mut bytes = accounts.value_bytes;
        let mut storage_nodes = BTreeSet::new();
        for address in &prepared.storage_owners {
            let account_path = account_version_prefix(*address);
            let root = match accounts
                .entries
                .binary_search_by_key(&account_path, |(path, _)| *path)
            {
                Ok(index) => {
                    decode_physical_account(&accounts.entries[index].1)
                        .map_err(Error::Read)?
                        .account
                        .storage_root
                }
                Err(_) => None,
            };
            let Some(root) = root else {
                continue;
            };
            let store = PreparedTrieStore {
                writer: self,
                prepared: Some(prepared),
                identity: prepared.next,
                node_column: "4",
                value_column: "5",
                address: Some(*address),
            };
            let remaining = InventoryLimits {
                max_nodes: limits.max_nodes - nodes,
                max_leaves: limits.max_leaves - leaves,
                max_value_bytes: limits.max_value_bytes - bytes,
            };
            let inventory = inventory_touched_storage_trie(
                &store,
                root,
                |path| storage_prefix_for_path(*address, path),
                remaining,
            )
            .map_err(|error| match super::reader::map_inventory_error(error) {
                Error::LimitExceeded {
                    resource, required, ..
                } => {
                    use super::reader::ConcreteStorageInventoryResource as Resource;
                    let (limit, used) = match resource {
                        Resource::Nodes => (limits.max_nodes, nodes),
                        Resource::Leaves => (limits.max_leaves, leaves),
                        Resource::ValueBytes => (limits.max_value_bytes, bytes),
                    };
                    Error::LimitExceeded {
                        resource,
                        limit,
                        required: used.saturating_add(required),
                    }
                }
                other => other,
            })?;
            nodes += inventory.nodes_visited;
            leaves += inventory.entries.len() as u64;
            bytes += inventory.value_bytes;
            storage_nodes.extend(inventory.hashed_nodes);
        }
        prepared.rows.retain(|row, _| {
            let nodes = match row.column {
                "2" => &accounts.hashed_nodes,
                "4" => &storage_nodes,
                _ => return true,
            };
            <[u8; 32]>::try_from(row.key.as_slice()).is_ok_and(|key| nodes.contains(&key))
        });
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn test_account_and_storage_node_counts(
        &self,
        prepared: &PreparedConcreteState,
        address: [u8; 20],
    ) -> (u64, u64) {
        let limits = InventoryLimits {
            max_nodes: 100_000,
            max_leaves: 100_000,
            max_value_bytes: 10_000_000,
        };
        let accounts = inventory_account_trie(
            &PreparedTrieStore {
                writer: self,
                prepared: Some(prepared),
                identity: prepared.next,
                node_column: "2",
                value_column: "3",
                address: None,
            },
            prepared.next.state_root,
            |path| path,
            limits,
        )
        .unwrap();
        let path = account_version_prefix(address);
        let index = accounts
            .entries
            .binary_search_by_key(&path, |(path, _)| *path)
            .unwrap();
        let root = decode_physical_account(&accounts.entries[index].1)
            .unwrap()
            .account
            .storage_root
            .unwrap();
        let storage = inventory_touched_storage_trie(
            &PreparedTrieStore {
                writer: self,
                prepared: Some(prepared),
                identity: prepared.next,
                node_column: "4",
                value_column: "5",
                address: Some(address),
            },
            root,
            |path| storage_prefix_for_path(address, path),
            limits,
        )
        .unwrap();
        (accounts.nodes_visited, storage.nodes_visited)
    }

    /// Persists CF1-CF5 content and version rows for `prepared`, without moving
    /// the descriptor or writing provenance/catalog/publication metadata. Old
    /// roots and old version rows remain readable. This is staging only; a
    /// successful return is not a committed or published state generation.
    pub fn persist_contents(
        &self,
        prepared: &PreparedConcreteState,
    ) -> Result<(), ConcreteReadError> {
        self.ensure_durable_prior()?;
        if prepared.writer_id != self.writer_id
            || prepared.prior != self.prior
            || prepared.sequence != self.sequence.get()
            || self.persisted_sequence.get().is_some()
        {
            return Err(corrupt(
                "prepared state is stale, foreign, or already persisted",
            ));
        }
        if current_identity(&self.db)? != self.durable_prior {
            return Err(corrupt("cannot stage rows after descriptor changed"));
        }
        let mut batch = WriteBatch::default();
        self.append_prepared_contents(prepared, &mut batch, false)?;
        self.db.write(batch).map_err(io)?;
        self.persisted_sequence.set(Some(prepared.sequence));
        Ok(())
    }

    pub(super) fn append_prepared_contents(
        &self,
        prepared: &PreparedConcreteState,
        batch: &mut WriteBatch,
        genesis: bool,
    ) -> Result<(), ConcreteReadError> {
        self.ensure_durable_prior()?;
        if prepared.writer_id != self.writer_id
            || prepared.prior != self.prior
            || prepared.sequence != self.sequence.get()
        {
            return Err(corrupt(
                "prepared state is stale or belongs to another writer",
            ));
        }
        if genesis {
            if self.db.get(DESCRIPTOR_KEY).map_err(io)?.is_some() {
                return Err(corrupt(
                    "fresh database acquired a descriptor before genesis commit",
                ));
            }
        } else if current_identity(&self.db)? != self.durable_prior {
            return Err(corrupt("cannot use prepared rows after descriptor changed"));
        }
        for (row, value) in &prepared.rows {
            let handle = self
                .db
                .cf_handle(row.column)
                .ok_or_else(|| corrupt("prepared row column disappeared"))?;
            if row.column == "1" || row.column == "2" || row.column == "4" {
                if let Some(existing) = self
                    .db
                    .get_cf(&handle, &row.key)
                    .map_err(|e| diagnostic_io(e, "prepared_row_get", row.column, "prepared_row"))?
                    && existing != *value
                {
                    return Err(corrupt(
                        "content-addressed row conflicts with existing bytes",
                    ));
                }
            } else if self
                .db
                .get_cf(&handle, &row.key)
                .map_err(|e| diagnostic_io(e, "prepared_row_get", row.column, "prepared_row"))?
                .is_some()
            {
                return Err(corrupt("prepared version row already exists"));
            }
            batch.put_cf(&handle, &row.key, value);
        }
        Ok(())
    }

    /// Reads one exact account row from `prepared`'s next-generation overlay,
    /// falling back to the authenticated prior generation when this preparation
    /// did not touch the address. The result is an in-memory execution view; it
    /// does not assert that the next identity was persisted or published.
    pub fn prepared_account(
        &self,
        prepared: &PreparedConcreteState,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.validate_prepared(prepared)?;
        self.verify_account_at(Some(prepared), address)?;
        match self.select_value_at(Some(prepared), "3", account_version_prefix(address))? {
            Some(value) if value.is_empty() => Ok(ConcreteRead::Tombstone),
            Some(value) => decode_physical_account(&value).map(ConcreteRead::Present),
            None => Ok(ConcreteRead::Absent),
        }
    }

    /// Borrows an execution-only reader for an exact validated preparation.
    pub(super) fn prepared_view<'a>(
        &'a self,
        prepared: &'a PreparedConcreteState,
    ) -> Result<PreparedConcreteView<'a>, ConcreteReadError> {
        self.validate_prepared(prepared)?;
        Ok(PreparedConcreteView {
            writer: self,
            prepared,
        })
    }

    pub(super) fn prepared_token(
        &self,
        prepared: &PreparedConcreteState,
    ) -> Result<(u64, u64), ConcreteReadError> {
        self.validate_prepared(prepared)?;
        Ok((prepared.writer_id, prepared.sequence))
    }

    fn prepared_storage(
        &self,
        prepared: &PreparedConcreteState,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.validate_prepared(prepared)?;
        match self.select_value_at(Some(prepared), "5", storage_version_prefix(address, key))? {
            Some(value) if value.is_empty() => Ok(ConcreteRead::Tombstone),
            Some(value) => Ok(ConcreteRead::Present(value)),
            None => Err(ConcreteReadError::HistoryUnavailable(prepared.next)),
        }
    }

    fn prepared_code(
        &self,
        prepared: &PreparedConcreteState,
        code_hash: [u8; 32],
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.validate_prepared(prepared)?;
        let Some(code) = self.code_at(Some(prepared), code_hash)? else {
            return Err(ConcreteReadError::HistoryUnavailable(prepared.next));
        };
        if keccak256(&code) != code_hash {
            return Err(corrupt("code bytes do not match their Keccak-256 key"));
        }
        Ok(ConcreteRead::Present(code))
    }

    /// Returns the database path this handle is pinned to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn from_fresh_database(db: DB, path: PathBuf) -> Self {
        Self {
            db: Arc::new(db),
            path,
            prior: ConcreteStateIdentity {
                period: FinalChainBlockNumber::GENESIS,
                state_root: empty_trie_root(),
            },
            durable_prior: ConcreteStateIdentity {
                period: FinalChainBlockNumber::GENESIS,
                state_root: empty_trie_root(),
            },
            writer_id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
            sequence: Cell::new(0),
            persisted_sequence: Cell::new(None),
        }
    }

    pub(super) fn invalidate_preparations(&self) -> Result<(), ConcreteReadError> {
        let sequence = self
            .sequence
            .get()
            .checked_add(1)
            .ok_or_else(|| corrupt("concrete writer preparation sequence overflow"))?;
        self.sequence.set(sequence);
        Ok(())
    }

    fn select_account_at(
        &self,
        prepared: Option<&PreparedConcreteState>,
        address: [u8; 20],
    ) -> Result<Option<ConcreteAccountRecord>, ConcreteReadError> {
        match self.select_value_at(prepared, "3", account_version_prefix(address))? {
            Some(value) if !value.is_empty() => decode_physical_account(&value).map(Some),
            _ => Ok(None),
        }
    }

    fn validate_prepared(&self, prepared: &PreparedConcreteState) -> Result<(), ConcreteReadError> {
        if prepared.writer_id != self.writer_id
            || prepared.prior != self.prior
            || prepared.sequence != self.sequence.get()
        {
            return Err(corrupt(
                "prepared state is stale or belongs to another writer",
            ));
        }
        if current_identity(&self.db)? != self.durable_prior {
            return Err(corrupt("prepared state prior is no longer the descriptor"));
        }
        Ok(())
    }

    fn ensure_durable_prior(&self) -> Result<(), ConcreteReadError> {
        if self.prior != self.durable_prior {
            return Err(corrupt(
                "historical read-only preparation cannot persist compatible rows",
            ));
        }
        Ok(())
    }

    fn verify_prepared_storage_path(
        &self,
        prepared: &PreparedConcreteState,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        let root = match self.prepared_account(prepared, address)? {
            ConcreteRead::Present(record) => record.account.storage_root,
            ConcreteRead::Absent | ConcreteRead::Tombstone => {
                return Ok(ConcreteStoragePath::NonMember);
            }
        };
        let Some(root) = root else {
            return Ok(ConcreteStoragePath::NonMember);
        };
        let store = PreparedTrieStore {
            writer: self,
            prepared: Some(prepared),
            identity: prepared.next,
            node_column: "4",
            value_column: "5",
            address: Some(address),
        };
        let path = storage_trie_path(key);
        match verify_path(
            &store,
            root,
            path,
            "4",
            "5",
            |path| storage_prefix_for_path(address, path),
            TrieSchema::Storage,
        )? {
            PathProof::NonMember => Ok(ConcreteStoragePath::NonMember),
            PathProof::Member(proved) => {
                let selected = store.value_for_prefix(storage_prefix_for_path(address, path))?;
                if selected.as_deref() != Some(proved.as_slice()) {
                    return Err(corrupt(
                        "prepared storage trie member differs from its physical version",
                    ));
                }
                Ok(ConcreteStoragePath::Member(proved))
            }
        }
    }

    pub(super) fn verify_prior_storage_path(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        let root = match ConcreteStateRead::account(self, address)? {
            ConcreteRead::Present(record) => record.account.storage_root,
            ConcreteRead::Absent | ConcreteRead::Tombstone => {
                return Ok(ConcreteStoragePath::NonMember);
            }
        };
        let Some(root) = root else {
            return Ok(ConcreteStoragePath::NonMember);
        };
        let store = PreparedTrieStore {
            writer: self,
            prepared: None,
            identity: self.prior,
            node_column: "4",
            value_column: "5",
            address: Some(address),
        };
        let path = storage_trie_path(key);
        match verify_path(
            &store,
            root,
            path,
            "4",
            "5",
            |path| storage_prefix_for_path(address, path),
            TrieSchema::Storage,
        )? {
            PathProof::NonMember => Ok(ConcreteStoragePath::NonMember),
            PathProof::Member(proved) => {
                let selected = store.value_for_prefix(storage_prefix_for_path(address, path))?;
                if selected.as_deref() != Some(proved.as_slice()) {
                    return Err(corrupt(
                        "prior storage trie member differs from its physical version",
                    ));
                }
                Ok(ConcreteStoragePath::Member(proved))
            }
        }
    }

    fn verify_prior_account(&self, address: [u8; 20]) -> Result<(), ConcreteReadError> {
        self.verify_account_at(None, address)
    }

    fn verify_account_at(
        &self,
        prepared: Option<&PreparedConcreteState>,
        address: [u8; 20],
    ) -> Result<(), ConcreteReadError> {
        let path = account_version_prefix(address);
        let identity = prepared.map_or(self.prior, |prepared| prepared.next);
        let store = PreparedTrieStore {
            writer: self,
            prepared,
            identity,
            node_column: "2",
            value_column: "3",
            address: None,
        };
        let proof = verify_path(
            &store,
            identity.state_root,
            path,
            "2",
            "3",
            |path| path,
            TrieSchema::Account,
        )?;
        let selected = store.value_for_prefix(path)?;
        match (proof, selected) {
            (PathProof::Member(proved), Some(selected)) if proved == selected => Ok(()),
            (PathProof::Member(_), _) => Err(corrupt(
                "prior account trie member differs from its physical version",
            )),
            (PathProof::NonMember, Some(selected)) if !selected.is_empty() => Err(corrupt(
                "prior account non-membership conflicts with a live physical version",
            )),
            (PathProof::NonMember, _) => Ok(()),
        }
    }

    fn get(&self, column: &str, key: &[u8]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        let handle = self
            .db
            .cf_handle(column)
            .ok_or_else(|| corrupt(format!("column family {column:?} is missing")))?;
        self.db
            .get_cf(&handle, key)
            .map_err(|e| diagnostic_io(e, "prior_prepared_get", column, "content"))
    }

    fn select_value_at(
        &self,
        prepared: Option<&PreparedConcreteState>,
        column: &'static str,
        prefix: [u8; 32],
    ) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        PreparedTrieStore {
            writer: self,
            prepared,
            identity: prepared.map_or(self.prior, |prepared| prepared.next),
            node_column: if column == "3" { "2" } else { "4" },
            value_column: column,
            address: None,
        }
        .value_for_prefix(prefix)
    }

    fn code_at(
        &self,
        prepared: Option<&PreparedConcreteState>,
        code_hash: [u8; 32],
    ) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        if let Some(prepared) = prepared {
            let row = RowKey {
                column: "1",
                key: code_hash.to_vec(),
            };
            if let Some(code) = prepared.rows.get(&row) {
                return Ok(Some(code.clone()));
            }
        }
        self.get("1", &code_hash)
    }

    fn validate_code_reference_at(
        &self,
        record: &ConcreteAccountRecord,
        staged: &BTreeMap<[u8; 32], Vec<u8>>,
        prepared: Option<&PreparedConcreteState>,
    ) -> Result<(), ConcreteReadError> {
        if record.account.code_size == 0 {
            return Ok(());
        }
        let hash = record
            .account
            .code_hash
            .ok_or_else(|| corrupt("nonzero account code size has no code hash"))?;
        let code = match staged.get(&hash) {
            Some(code) => code.clone(),
            None => self
                .code_at(prepared, hash)?
                .ok_or_else(|| corrupt("account references unavailable code bytes"))?,
        };
        if code.len() as u64 != record.account.code_size || keccak256(&code) != hash {
            return Err(corrupt("account code hash or size differs from code bytes"));
        }
        Ok(())
    }

    #[cfg(test)]
    fn publish_descriptor_for_reopen_test(
        &self,
        prepared: &PreparedConcreteState,
    ) -> Result<(), ConcreteReadError> {
        if self.persisted_sequence.get() != Some(prepared.sequence) {
            return Err(corrupt("test descriptor advance requires staged contents"));
        }
        self.db
            .put(DESCRIPTOR_KEY, encode_descriptor(prepared.next))
            .map_err(io)
    }
}

struct PreparedTrieStore<'a> {
    writer: &'a ConcreteStateWriter,
    prepared: Option<&'a PreparedConcreteState>,
    identity: ConcreteStateIdentity,
    node_column: &'static str,
    value_column: &'static str,
    address: Option<[u8; 20]>,
}

impl PreparedTrieStore<'_> {
    fn value_for_prefix(&self, prefix: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        if let Some(prepared) = self.prepared {
            let row = RowKey {
                column: self.value_column,
                key: versioned_key(prefix, prepared.next.period).to_vec(),
            };
            if let Some(value) = prepared.rows.get(&row) {
                return Ok(Some(value.clone()));
            }
        }
        select_version(
            &self.writer.db,
            self.value_column,
            prefix,
            self.writer.prior.period,
        )
    }
}

impl TrieWriteStore for PreparedTrieStore<'_> {
    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        if column != self.node_column {
            return Err(corrupt("trie requested an unexpected node column"));
        }
        if let Some(prepared) = self.prepared {
            let row = RowKey {
                column: self.node_column,
                key: hash.to_vec(),
            };
            if let Some(node) = prepared.rows.get(&row) {
                return Ok(Some(node.clone()));
            }
        }
        let handle = self
            .writer
            .db
            .cf_handle(column)
            .ok_or_else(|| corrupt("trie node column is missing"))?;
        self.writer
            .db
            .get_cf(&handle, hash)
            .map_err(|e| diagnostic_io(e, "node_get_prior_prepared", column, "trie_node"))
    }

    fn value(&self, key: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        let prefix = match self.address {
            Some(address) => storage_prefix_for_path(address, key),
            None => key,
        };
        self.value_for_prefix(prefix)
    }
}

impl PhysicalTrieStore for PreparedTrieStore<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        <Self as TrieWriteStore>::node(self, column, hash)
    }

    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        if column != self.value_column || period != self.identity.period {
            return Err(corrupt("trie proof requested an unexpected value view"));
        }
        self.value_for_prefix(prefix)
            .map(|selected| selected.map(|value| SelectedVersion { value }))
    }
}

fn select_version(
    db: &DB,
    column: &str,
    prefix: [u8; 32],
    period: FinalChainBlockNumber,
) -> Result<Option<Vec<u8>>, ConcreteReadError> {
    let handle = db
        .cf_handle(column)
        .ok_or_else(|| corrupt(format!("value column family {column:?} is missing")))?;
    let target = versioned_key(prefix, period);
    let mut iterator = db.iterator_cf(&handle, IteratorMode::From(&target, Direction::Reverse));
    let Some(entry) = iterator.next() else {
        return Ok(None);
    };
    let (key, value) = entry.map_err(|e| {
        diagnostic_io(
            e,
            "history_seek_next_prior_prepared",
            column,
            "versioned_value",
        )
    })?;
    if key.len() != 40 || key[..32] != prefix {
        return Ok(None);
    }
    Ok(Some(value.to_vec()))
}

fn replace_storage_root(
    record: &ConcreteAccountRecord,
    storage_root: Option<[u8; 32]>,
) -> Result<ConcreteAccountRecord, ConcreteReadError> {
    validate_record(record)?;
    let rlp = Rlp::new(&record.physical_rlp);
    let mut stream = RlpStream::new_list(5);
    stream.append_raw(rlp.at(0).map_err(corrupt)?.as_raw(), 1);
    stream.append_raw(rlp.at(1).map_err(corrupt)?.as_raw(), 1);
    match storage_root {
        Some(root) => stream.append(&root.as_slice()),
        None => stream.append_empty_data(),
    };
    stream.append_raw(rlp.at(3).map_err(corrupt)?.as_raw(), 1);
    stream.append_raw(rlp.at(4).map_err(corrupt)?.as_raw(), 1);
    decode_physical_account(&stream.out())
}

fn validate_record(record: &ConcreteAccountRecord) -> Result<(), ConcreteReadError> {
    if decode_physical_account(&record.physical_rlp)? != *record {
        return Err(corrupt(
            "account fields do not match preserved physical RLP",
        ));
    }
    Ok(())
}

fn validate_storage_value(value: &Option<Vec<u8>>) -> Result<(), ConcreteReadError> {
    if matches!(value, Some(value) if value.is_empty()) {
        return Err(corrupt(
            "empty live storage bytes must use a delete mutation",
        ));
    }
    Ok(())
}

fn normalize_code_insertions(
    insertions: Vec<ConcreteCodeInsertion>,
) -> Result<BTreeMap<[u8; 32], Vec<u8>>, ConcreteReadError> {
    let mut code_changes = BTreeMap::new();
    for insertion in insertions {
        if keccak256(&insertion.code) != insertion.code_hash {
            return Err(corrupt("code bytes do not match their Keccak-256 key"));
        }
        if code_changes
            .insert(insertion.code_hash, insertion.code)
            .is_some()
        {
            return Err(corrupt("duplicate code insertion"));
        }
    }
    Ok(code_changes)
}

fn merge_prepared_row(
    rows: &mut BTreeMap<RowKey, Vec<u8>>,
    column: &'static str,
    key: Vec<u8>,
    value: Vec<u8>,
) -> Result<(), ConcreteReadError> {
    let row = RowKey { column, key };
    match rows.insert(row, value.clone()) {
        Some(previous) if matches!(column, "1" | "2" | "4") && previous != value => Err(corrupt(
            "prepared content-addressed rows contain a key conflict",
        )),
        _ => Ok(()),
    }
}

fn open_existing_database(path: &Path, read_only: bool) -> Result<DB, ConcreteReadError> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    let columns = DB::list_cf(&options, path).map_err(io)?;
    let present = columns.iter().map(String::as_str).collect::<BTreeSet<_>>();
    for required in REQUIRED_COLUMNS {
        if !present.contains(required) {
            return Err(corrupt(format!(
                "concrete state column family {required:?} is missing"
            )));
        }
    }
    let descriptors = columns
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
    if read_only {
        DB::open_cf_descriptors_read_only(&options, path, descriptors, false).map_err(io)
    } else {
        DB::open_cf_descriptors(&options, path, descriptors).map_err(io)
    }
}

fn current_identity(db: &DB) -> Result<ConcreteStateIdentity, ConcreteReadError> {
    let bytes = db
        .get(DESCRIPTOR_KEY)
        .map_err(|e| diagnostic_io(e, "descriptor_get_prior_prepared", "default", "descriptor"))?
        .ok_or_else(|| corrupt("concrete descriptor is missing"))?;
    decode_descriptor(&bytes)
}

pub(super) fn encode_descriptor(identity: ConcreteStateIdentity) -> Vec<u8> {
    let mut stream = RlpStream::new_list(2);
    stream.append(&identity.period.as_u64());
    stream.append(&identity.state_root.as_slice());
    stream.out().to_vec()
}

fn diagnostic_io(
    error: impl std::fmt::Display,
    operation: &str,
    column: &str,
    key: &str,
) -> ConcreteReadError {
    #[cfg(feature = "synthetic-fixtures")]
    if std::env::var_os("RUST_LIB_BACKTRACE").as_deref() == Some(std::ffi::OsStr::new("1")) {
        eprintln!("sst_diagnostic operation={operation} column={column} key={key} error={error}");
    }
    #[cfg(not(feature = "synthetic-fixtures"))]
    let _ = (operation, column, key);
    io(error)
}

fn io(error: impl std::fmt::Display) -> ConcreteReadError {
    ConcreteReadError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use num_bigint::BigUint;
    use rustaxa_types::FinalChainNonce;
    use rustaxa_types::concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteRead, ConcreteStateRead,
    };

    use super::*;
    use crate::ConcreteStateReader;

    struct TestDb {
        path: PathBuf,
    }

    impl TestDb {
        fn new() -> Self {
            let path = crate::test_paths::new_path("concrete-writer");
            let mut options = Options::default();
            options.create_if_missing(true);
            options.create_missing_column_families(true);
            let descriptors = REQUIRED_COLUMNS
                .iter()
                .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()));
            let db = DB::open_cf_descriptors(&options, &path, descriptors).unwrap();
            let genesis = ConcreteStateIdentity {
                period: FinalChainBlockNumber::GENESIS,
                state_root: empty_trie_root(),
            };
            db.put(DESCRIPTOR_KEY, encode_descriptor(genesis)).unwrap();
            drop(db);
            Self { path }
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }

    #[test]
    fn stages_fresh_state_reopens_and_continues_with_go_roots() {
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let address = {
            let mut address = [0_u8; 20];
            address[19] = 0xaa;
            address
        };
        let slot = ConcreteStorageKey({
            let mut slot = [0_u8; 32];
            slot[31] = 1;
            slot
        });
        let code = vec![0x60, 0, 0x60, 1, 1];
        let code_hash = keccak256(&code);
        // The pinned Go journal seed has no code reference. The independent
        // insertion still exercises immutable CF1 staging in the same batch.
        let account = account(1, 100, None, None, 0);
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: account,
                    }],
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: slot,
                        value: Some(vec![0x11]),
                    }],
                    code: vec![ConcreteCodeInsertion { code_hash, code }],
                },
            )
            .unwrap();
        assert_eq!(
            hex::encode(prepared.next_identity().state_root),
            "e8269fee2b975af999fca4ae0f98390c2b86a727e060065ecaf870aea2cf951b"
        );
        let prepared_account = match writer.prepared_account(&prepared, address).unwrap() {
            ConcreteRead::Present(record) => record,
            other => panic!("expected prepared account, got {other:?}"),
        };
        assert_eq!(
            prepared_account.account.storage_root,
            Some(decode_hash(
                "6dcc37243a77dfcb10bff800d0ae99a7f0717898504f684e0bedb8a5228c041c"
            ))
        );
        assert_eq!(
            writer.prepared_account(&prepared, [0x99; 20]).unwrap(),
            ConcreteRead::Absent
        );
        assert!(prepared.row_count() >= 5);
        writer.persist_contents(&prepared).unwrap();
        assert_hex_row(
            &writer,
            "2",
            "e8269fee2b975af999fca4ae0f98390c2b86a727e060065ecaf870aea2cf951b",
            "f843a120528b55564e8518548e42b534da3a526179b820f264ee7c6929d00b0b6a31cfc2a0e8269fee2b975af999fca4ae0f98390c2b86a727e060065ecaf870aea2cf951b",
        );
        assert_version_value(
            &writer,
            "3",
            account_version_prefix(address),
            FinalChainBlockNumber::new(1),
            "e50164a06dcc37243a77dfcb10bff800d0ae99a7f0717898504f684e0bedb8a5228c041c8080",
        );
        assert_hex_row(
            &writer,
            "4",
            "6dcc37243a77dfcb10bff800d0ae99a7f0717898504f684e0bedb8a5228c041c",
            "f843a120b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6a06dcc37243a77dfcb10bff800d0ae99a7f0717898504f684e0bedb8a5228c041c",
        );
        assert_version_value(
            &writer,
            "5",
            storage_version_prefix(address, slot),
            FinalChainBlockNumber::new(1),
            "11",
        );
        assert_eq!(
            writer.get("1", &code_hash).unwrap(),
            Some(vec![0x60, 0, 0x60, 1, 1])
        );
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let first = prepared.next_identity();
        drop(writer);

        let reader = ConcreteStateReader::open_read_only(&database.path, first).unwrap();
        assert_eq!(
            reader.storage(address, slot).unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        let first_account = match reader.account(address).unwrap() {
            ConcreteRead::Present(record) => record,
            other => panic!("expected live account, got {other:?}"),
        };
        drop(reader);

        let writer = ConcreteStateWriter::open(&database.path, first).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(2),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: first_account,
                    }],
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: slot,
                        value: Some(vec![0, 0x44]),
                    }],
                    code: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(
            hex::encode(prepared.next_identity().state_root),
            "b9d139c10bb0fe50e36d00dd6235a3b21b9c7ad871cee485b9af2efefffe10c9"
        );
        writer.persist_contents(&prepared).unwrap();
        assert_hex_row(
            &writer,
            "2",
            "b9d139c10bb0fe50e36d00dd6235a3b21b9c7ad871cee485b9af2efefffe10c9",
            "f843a120528b55564e8518548e42b534da3a526179b820f264ee7c6929d00b0b6a31cfc2a0b9d139c10bb0fe50e36d00dd6235a3b21b9c7ad871cee485b9af2efefffe10c9",
        );
        assert_version_value(
            &writer,
            "3",
            account_version_prefix(address),
            FinalChainBlockNumber::new(2),
            "e50164a0f13607ab7039ea1720d1f953244ff09f8f93d403c1711b54f7c20cd84501e0538080",
        );
        assert_hex_row(
            &writer,
            "4",
            "f13607ab7039ea1720d1f953244ff09f8f93d403c1711b54f7c20cd84501e053",
            "f843a120b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6a0f13607ab7039ea1720d1f953244ff09f8f93d403c1711b54f7c20cd84501e053",
        );
        assert_version_value(
            &writer,
            "5",
            storage_version_prefix(address, slot),
            FinalChainBlockNumber::new(2),
            "0044",
        );
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let second = prepared.next_identity();
        drop(writer);

        let reader = ConcreteStateReader::open_read_only(&database.path, second).unwrap();
        assert_eq!(
            reader.storage(address, slot).unwrap(),
            ConcreteRead::Present(vec![0, 0x44])
        );
        assert_eq!(
            reader.code(code_hash).unwrap(),
            ConcreteRead::Present(vec![0x60, 0, 0x60, 1, 1])
        );
        assert_eq!(ConcreteStateRead::identity(&reader), second);
        drop(reader);

        let historical =
            ConcreteStateReader::open_historical_read_only(&database.path, second, first).unwrap();
        assert_eq!(
            historical.storage(address, slot).unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        assert!(matches!(
            historical.account(address).unwrap(),
            ConcreteRead::Present(_)
        ));
        drop(historical);

        let writer = ConcreteStateWriter::open(&database.path, second).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(3),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Delete { address }],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(prepared.next_identity().state_root, empty_trie_root());
        assert_eq!(
            writer.prepared_account(&prepared, address).unwrap(),
            ConcreteRead::Tombstone
        );
        writer.persist_contents(&prepared).unwrap();
        assert_version_value(
            &writer,
            "3",
            account_version_prefix(address),
            FinalChainBlockNumber::new(3),
            "",
        );
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let third = prepared.next_identity();
        drop(writer);

        let reader = ConcreteStateReader::open_read_only(&database.path, third).unwrap();
        assert_eq!(reader.account(address).unwrap(), ConcreteRead::Tombstone);
        assert_eq!(
            reader.storage(address, slot).unwrap(),
            ConcreteRead::Present(vec![0, 0x44])
        );
    }

    #[test]
    fn keeps_distinct_ordinary_and_native_raw_logical_keys() {
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let address = [0x42; 20];
        let ordinary = ConcreteStorageKey([0; 32]);
        let native_raw = ConcreteStorageKey([0x77; 32]);
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: account(0, 0, None, None, 0),
                    }],
                    storage: vec![
                        ConcreteStorageMutation {
                            address,
                            key: ordinary,
                            value: Some(vec![0x01]),
                        },
                        ConcreteStorageMutation {
                            address,
                            key: native_raw,
                            value: Some(vec![0, 0x02]),
                        },
                    ],
                    code: Vec::new(),
                },
            )
            .unwrap();
        writer.persist_contents(&prepared).unwrap();
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let identity = prepared.next_identity();
        drop(writer);

        let reader = ConcreteStateReader::open_read_only(&database.path, identity).unwrap();
        assert_eq!(
            reader.storage(address, ordinary).unwrap(),
            ConcreteRead::Present(vec![0x01])
        );
        assert_eq!(
            reader.storage(address, native_raw).unwrap(),
            ConcreteRead::Present(vec![0, 0x02])
        );
    }

    #[test]
    fn rejects_duplicate_and_stale_preparations_without_publishing() {
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let duplicate = ConcreteStateMutationBatch {
            accounts: vec![
                ConcreteAccountMutation::Delete { address: [1; 20] },
                ConcreteAccountMutation::Delete { address: [1; 20] },
            ],
            ..Default::default()
        };
        assert!(
            writer
                .prepare(FinalChainBlockNumber::new(1), duplicate)
                .is_err()
        );

        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch::default(),
            )
            .unwrap();
        let newer = writer
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch::default(),
            )
            .unwrap();
        assert!(writer.prepared_account(&prepared, [0; 20]).is_err());
        assert!(writer.persist_contents(&prepared).is_err());
        writer.persist_contents(&newer).unwrap();
        assert!(writer.persist_contents(&newer).is_err());
        assert!(
            writer
                .prepare(
                    FinalChainBlockNumber::new(1),
                    ConcreteStateMutationBatch::default(),
                )
                .is_err()
        );
        assert_eq!(current_identity(&writer.db).unwrap(), genesis);
    }

    #[test]
    fn missing_delete_does_not_invent_a_version_tombstone() {
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Delete { address: [9; 20] }],
                    storage: vec![ConcreteStorageMutation {
                        address: [8; 20],
                        key: ConcreteStorageKey([7; 32]),
                        value: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(prepared.next_identity(), genesis_with_period(1));
        assert_eq!(prepared.row_count(), 0);
        writer.persist_contents(&prepared).unwrap();
        assert_eq!(current_identity(&writer.db).unwrap(), genesis);
    }

    #[test]
    fn prior_storage_proof_keeps_prepared_state_and_failures_separate() {
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let address = [0x41; 20];
        let member = ConcreteStorageKey([1; 32]);
        let missing = ConcreteStorageKey([2; 32]);
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let first = writer
            .prepare(
                1.into(),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: account(0, 100, None, None, 0),
                    }],
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: member,
                        value: Some(vec![1]),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        writer.persist_contents(&first).unwrap();
        writer.publish_descriptor_for_reopen_test(&first).unwrap();
        let prior = first.next_identity();
        drop(writer);
        let writer = ConcreteStateWriter::open(&database.path, prior).unwrap();
        let pending = writer
            .prepare(
                2.into(),
                ConcreteStateMutationBatch {
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: missing,
                        value: Some(vec![2]),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        let sequence = writer.sequence.get();
        let persisted_sequence = writer.persisted_sequence.get();
        let snapshot = |writer: &ConcreteStateWriter| {
            REQUIRED_COLUMNS
                .iter()
                .map(|column| {
                    let cf = writer.db.cf_handle(column).unwrap();
                    writer
                        .db
                        .iterator_cf(&cf, IteratorMode::Start)
                        .map(|row| {
                            let (key, value) = row.unwrap();
                            (key.to_vec(), value.to_vec())
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let before = snapshot(&writer);
        assert_eq!(
            writer.storage(address, member).unwrap(),
            ConcreteRead::Present(vec![1])
        );
        assert_eq!(
            writer.verify_prior_storage_path(address, member).unwrap(),
            ConcreteStoragePath::Member(vec![1])
        );
        assert_eq!(
            writer.storage(address, missing),
            Err(ConcreteReadError::HistoryUnavailable(prior))
        );
        assert_eq!(
            writer.verify_prior_storage_path(address, missing).unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert_eq!(
            writer
                .verify_prior_storage_path([0x42; 20], missing)
                .unwrap(),
            ConcreteStoragePath::NonMember
        );
        let prepared_view = writer.prepared_view(&pending).unwrap();
        assert_eq!(
            rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                &prepared_view,
                address,
                missing
            )
            .unwrap(),
            ConcreteRead::Present(vec![2])
        );
        assert_eq!(writer.sequence.get(), sequence);
        assert_eq!(writer.persisted_sequence.get(), persisted_sequence);
        assert_eq!(snapshot(&writer), before);
        assert_eq!(current_identity(&writer.db).unwrap(), prior);
        writer.persist_contents(&pending).unwrap();
        writer.publish_descriptor_for_reopen_test(&pending).unwrap();
        let next = pending.next_identity();
        drop(writer);
        let writer = ConcreteStateWriter::open(&database.path, next).unwrap();
        let deleted = writer
            .prepare(
                3.into(),
                ConcreteStateMutationBatch {
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: member,
                        value: None,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        writer.persist_contents(&deleted).unwrap();
        writer.publish_descriptor_for_reopen_test(&deleted).unwrap();
        let last = deleted.next_identity();
        drop(writer);
        let writer = ConcreteStateWriter::open(&database.path, last).unwrap();
        assert_eq!(
            writer.storage(address, member).unwrap(),
            ConcreteRead::Tombstone
        );
        assert_eq!(
            writer.verify_prior_storage_path(address, member).unwrap(),
            ConcreteStoragePath::NonMember
        );
        let ConcreteRead::Present(account) = writer.account(address).unwrap() else {
            panic!("account missing")
        };
        let storage_root = account.account.storage_root.unwrap();
        let cf4 = writer.db.cf_handle("4").unwrap();
        assert!(writer.db.get_cf(&cf4, storage_root).unwrap().is_some());
        writer.db.delete_cf(&cf4, storage_root).unwrap();
        assert_eq!(
            writer.verify_prior_storage_path(address, missing),
            Err(ConcreteReadError::HistoryUnavailable(last))
        );
        let cf2 = writer.db.cf_handle("2").unwrap();
        assert!(writer.db.get_cf(&cf2, last.state_root).unwrap().is_some());
        writer.db.delete_cf(&cf2, last.state_root).unwrap();
        assert_eq!(
            writer.verify_prior_storage_path(address, missing),
            Err(ConcreteReadError::HistoryUnavailable(last))
        );
    }

    #[test]
    fn prepared_storage_proof_binds_overlay_and_rejects_stale_or_missing_nodes() {
        use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let address = [0x41; 20];
        let member = ConcreteStorageKey([1; 32]);
        let added = ConcreteStorageKey([2; 32]);
        let unknown = ConcreteStorageKey([3; 32]);
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let first = writer
            .prepare(
                1.into(),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: account(0, 100, None, None, 0),
                    }],
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: member,
                        value: Some(vec![1]),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        writer.persist_contents(&first).unwrap();
        writer.publish_descriptor_for_reopen_test(&first).unwrap();
        let prior = first.next_identity();
        drop(writer);
        let writer = ConcreteStateWriter::open(&database.path, prior).unwrap();
        let mut pending = writer
            .prepare(
                2.into(),
                ConcreteStateMutationBatch {
                    storage: vec![
                        ConcreteStorageMutation {
                            address,
                            key: member,
                            value: None,
                        },
                        ConcreteStorageMutation {
                            address,
                            key: added,
                            value: Some(vec![1]),
                        },
                    ],
                    ..Default::default()
                },
            )
            .unwrap();
        let snapshot = |writer: &ConcreteStateWriter| {
            REQUIRED_COLUMNS
                .iter()
                .map(|column| {
                    let cf = writer.db.cf_handle(column).unwrap();
                    writer
                        .db
                        .iterator_cf(&cf, IteratorMode::Start)
                        .map(|row| {
                            let (key, value) = row.unwrap();
                            (key.to_vec(), value.to_vec())
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let before = snapshot(&writer);
        let rows_before = pending.rows.clone();
        let sequence = writer.sequence.get();
        let persisted = writer.persisted_sequence.get();
        let view = writer.prepared_view(&pending).unwrap();
        assert_eq!(
            view.verify_storage_path(address, unknown).unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert_eq!(
            view.storage(address, unknown),
            Err(ConcreteReadError::HistoryUnavailable(pending.next))
        );
        assert_eq!(
            view.verify_storage_path(address, added).unwrap(),
            ConcreteStoragePath::Member(vec![1])
        );
        assert_eq!(
            view.storage(address, added).unwrap(),
            ConcreteRead::Present(vec![1])
        );
        assert_eq!(
            view.verify_storage_path(address, member).unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert_eq!(
            view.storage(address, member).unwrap(),
            ConcreteRead::Tombstone
        );
        assert_eq!(
            writer.verify_prior_storage_path(address, added).unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert_eq!(
            writer.verify_prior_storage_path(address, member).unwrap(),
            ConcreteStoragePath::Member(vec![1])
        );
        assert_eq!(snapshot(&writer), before);
        assert_eq!(pending.rows, rows_before);
        assert_eq!(writer.sequence.get(), sequence);
        assert_eq!(writer.persisted_sequence.get(), persisted);
        assert_eq!(current_identity(&writer.db).unwrap(), prior);
        let root = match writer.prepared_account(&pending, address).unwrap() {
            ConcreteRead::Present(record) => record.account.storage_root.unwrap(),
            _ => panic!("prepared account"),
        };
        let overlay = RowKey {
            column: "4",
            key: root.to_vec(),
        };
        assert!(
            writer
                .db
                .get_cf(&writer.db.cf_handle("4").unwrap(), root)
                .unwrap()
                .is_none()
        );
        let removed = pending
            .rows
            .remove(&overlay)
            .expect("prepared root dependency");
        assert!(
            writer
                .prepared_view(&pending)
                .unwrap()
                .verify_storage_path(address, unknown)
                .is_err()
        );
        pending.rows.insert(overlay, removed);
        assert_eq!(pending.rows, rows_before);
        let other_database = TestDb::new();
        let other = ConcreteStateWriter::open(&other_database.path, genesis).unwrap();
        assert!(other.prepared_view(&pending).is_err());
        let newer = writer
            .prepare(2.into(), ConcreteStateMutationBatch::default())
            .unwrap();
        assert!(writer.prepared_view(&pending).is_err());
        assert!(
            writer
                .verify_prepared_storage_path(&pending, [0x99; 20], unknown)
                .is_err()
        );
        let prior_root = match ConcreteStateRead::account(&writer, address).unwrap() {
            ConcreteRead::Present(record) => record.account.storage_root.unwrap(),
            _ => panic!("prior account"),
        };
        let cf4 = writer.db.cf_handle("4").unwrap();
        let saved = writer
            .db
            .get_cf(&cf4, prior_root)
            .unwrap()
            .expect("prior root dependency");
        writer.db.delete_cf(&cf4, prior_root).unwrap();
        assert!(
            writer
                .prepared_view(&newer)
                .unwrap()
                .verify_storage_path(address, unknown)
                .is_err()
        );
        writer.db.put_cf(&cf4, prior_root, saved).unwrap();
        assert_eq!(snapshot(&writer), before);
        assert_eq!(current_identity(&writer.db).unwrap(), prior);
    }

    #[test]
    fn historical_read_only_preparation_reproduces_retained_next_root() {
        let database = TestDb::new();
        let address = [0x41; 20];
        let (first, second, second_account) = seed_two_periods(&database, address);

        let historical =
            HistoricalConcretePreparation::open_read_only(&database.path, second, first).unwrap();
        assert_eq!(historical.committed_identity(), second);
        assert_eq!(historical.historical_identity(), first);
        let summary = historical
            .prepare(
                second.period,
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: second_account.clone(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(summary.next_identity(), second);
        assert!(summary.row_count() > 0);
        assert_eq!(summary.changed_accounts().len(), 1);
        assert_eq!(summary.changed_accounts()[0].address(), address);
        assert_eq!(
            summary.changed_accounts()[0].value(),
            &ConcreteRead::Present(second_account)
        );
        let inaccessible_prepared = historical
            .writer
            .prepare(second.period, ConcreteStateMutationBatch::default())
            .unwrap();
        assert!(
            historical
                .writer
                .persist_contents(&inaccessible_prepared)
                .is_err()
        );
        drop(historical);

        let reader = ConcreteStateReader::open_read_only(&database.path, second).unwrap();
        assert_eq!(ConcreteStateRead::identity(&reader), second);
    }

    #[test]
    fn historical_open_rejects_identity_mismatch_and_future_base() {
        let database = TestDb::new();
        let address = [0x42; 20];
        let (first, second, _) = seed_two_periods(&database, address);
        let wrong_current = ConcreteStateIdentity {
            period: second.period,
            state_root: [0x77; 32],
        };
        assert!(matches!(
            HistoricalConcretePreparation::open_read_only(&database.path, wrong_current, first),
            Err(ConcreteReadError::IdentityMismatch { .. })
        ));
        let wrong_same_period_base = ConcreteStateIdentity {
            period: second.period,
            state_root: [0x88; 32],
        };
        assert!(matches!(
            HistoricalConcretePreparation::open_read_only(
                &database.path,
                second,
                wrong_same_period_base
            ),
            Err(ConcreteReadError::IdentityMismatch { .. })
        ));
        let future = ConcreteStateIdentity {
            period: second.period.checked_next().unwrap(),
            state_root: [0x99; 32],
        };
        assert!(matches!(
            HistoricalConcretePreparation::open_read_only(&database.path, second, future),
            Err(ConcreteReadError::FuturePeriod { .. })
        ));
    }

    #[test]
    fn historical_open_rejects_missing_and_corrupt_root_even_for_noop() {
        let database = TestDb::new();
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let missing = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: [0x55; 32],
        };
        assert!(matches!(
            HistoricalConcretePreparation::open_read_only(&database.path, genesis, missing),
            Err(ConcreteReadError::IdentityMismatch { .. })
        ));

        let address = [0x43; 20];
        let (first, second, _) = seed_two_periods(&database, address);
        let missing = ConcreteStateIdentity {
            period: first.period,
            state_root: [0x55; 32],
        };
        assert!(matches!(
            HistoricalConcretePreparation::open_read_only(&database.path, second, missing),
            Err(ConcreteReadError::HistoryUnavailable(identity)) if identity == missing
        ));

        let corrupt = ConcreteStateIdentity {
            period: first.period,
            state_root: [0x66; 32],
        };
        let mut options = Options::default();
        options.create_if_missing(false);
        options.create_missing_column_families(false);
        let columns = DB::list_cf(&options, &database.path).unwrap();
        let descriptors = columns
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
        let db = DB::open_cf_descriptors(&options, &database.path, descriptors).unwrap();
        let nodes = db.cf_handle("2").unwrap();
        db.put_cf(&nodes, corrupt.state_root, [1, 2, 3]).unwrap();
        drop(db);
        assert!(matches!(
            HistoricalConcretePreparation::open_read_only(&database.path, second, corrupt),
            Err(ConcreteReadError::Corrupt(_))
        ));
    }

    #[test]
    fn probe_ordered_phases_read_prefix_account_code_and_raw_storage() {
        use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
        let database = TestDb::new();
        let address = [0x61; 20];
        let (first, committed, _) = seed_two_periods(&database, address);
        let mut probe =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, first).unwrap();
        assert_eq!(probe.committed_identity(), committed);
        assert_eq!(probe.historical_identity(), first);
        let prior = probe.current_view().unwrap();
        assert_eq!(prior.identity(), first);
        assert_eq!(
            prior.account(address).unwrap(),
            ConcreteRead::Present(account(0, 100, None, None, 0))
        );
        assert_eq!(prior.account([0; 20]).unwrap(), ConcreteRead::Absent);
        assert!(
            matches!(prior.code([0x55; 32]), Err(ConcreteReadError::HistoryUnavailable(id)) if id == first)
        );
        let key = ConcreteStorageKey([1; 32]);
        let code = vec![0x60, 1, 0];
        let code_hash = keccak256(&code);
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address,
                    record: account(2, 70, None, Some(code_hash), code.len() as u64),
                }],
                storage: vec![
                    ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![1]),
                    },
                    ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![2]),
                    },
                ],
                code: vec![ConcreteCodeInsertion {
                    code_hash,
                    code: code.clone(),
                }],
            })
            .unwrap();
        let prefix = probe.current_view().unwrap();
        let prefix_identity = prefix.identity();
        assert_eq!(prefix_identity.period, committed.period);
        assert_eq!(
            prefix.storage(address, key).unwrap(),
            ConcreteRead::Present(vec![2])
        );
        assert_eq!(
            prefix.verify_storage_path(address, key).unwrap(),
            ConcreteStoragePath::Member(vec![2])
        );
        assert_eq!(
            prefix.code(code_hash).unwrap(),
            ConcreteRead::Present(code.clone())
        );
        let ConcreteRead::Present(prefix_account) = prefix.account(address).unwrap() else {
            panic!("missing prefix account")
        };
        assert_eq!(prefix_account.account.nonce, FinalChainNonce::from_u64(2));
        assert_eq!(prefix_account.account.code_hash, Some(code_hash));
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address,
                    record: account(3, 60, None, Some(code_hash), code.len() as u64),
                }],
                storage: vec![
                    ConcreteStorageMutation {
                        address,
                        key,
                        value: None,
                    },
                    ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![3]),
                    },
                ],
                ..Default::default()
            })
            .unwrap();
        let view = probe.current_view().unwrap();
        assert_ne!(view.identity().state_root, prefix_identity.state_root);
        assert_eq!(view.code(code_hash).unwrap(), ConcreteRead::Present(code));
        assert_eq!(
            view.storage(address, key).unwrap(),
            ConcreteRead::Present(vec![3])
        );
        assert_eq!(
            view.verify_storage_path(address, key).unwrap(),
            ConcreteStoragePath::Member(vec![3])
        );
    }

    #[test]
    fn probe_deleted_account_keeps_raw_history_without_inferred_absence() {
        use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
        let database = TestDb::new();
        let address = [0x62; 20];
        let (_, committed, _) = seed_two_periods(&database, address);
        let mut probe =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, committed).unwrap();
        let key = ConcreteStorageKey([1; 32]);
        let missing = ConcreteStorageKey([2; 32]);
        assert_eq!(
            probe
                .current_view()
                .unwrap()
                .verify_storage_path(address, missing)
                .unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert!(
            matches!(probe.current_view().unwrap().storage(address, missing), Err(ConcreteReadError::HistoryUnavailable(id)) if id == committed)
        );
        assert_eq!(
            probe
                .current_view()
                .unwrap()
                .ordinary_storage(address, missing)
                .unwrap(),
            ConcreteRead::Absent
        );
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta {
                storage: vec![ConcreteStorageMutation {
                    address,
                    key,
                    value: Some(vec![9]),
                }],
                ..Default::default()
            })
            .unwrap();
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Delete { address }],
                ..Default::default()
            })
            .unwrap();
        let view = probe.current_view().unwrap();
        assert_eq!(view.account(address).unwrap(), ConcreteRead::Tombstone);
        assert_eq!(
            view.storage(address, key).unwrap(),
            ConcreteRead::Present(vec![9])
        );
        assert_eq!(
            view.verify_storage_path(address, key).unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert!(
            matches!(view.storage(address, missing), Err(ConcreteReadError::HistoryUnavailable(id)) if id == view.identity())
        );
        assert_eq!(
            view.ordinary_storage(address, missing).unwrap(),
            ConcreteRead::Absent
        );
    }

    #[test]
    fn probe_failed_phase_preserves_prior_view() {
        use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
        let database = TestDb::new();
        let address = [0x63; 20];
        let (_, committed, _) = seed_two_periods(&database, address);
        let mut probe =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, committed).unwrap();
        let key = ConcreteStorageKey([1; 32]);
        let code = vec![0x60, 2, 0];
        let code_hash = keccak256(&code);
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta {
                storage: vec![ConcreteStorageMutation {
                    address,
                    key,
                    value: Some(vec![7]),
                }],
                code: vec![ConcreteCodeInsertion {
                    code_hash,
                    code: code.clone(),
                }],
                ..Default::default()
            })
            .unwrap();
        let identity = probe.current_view().unwrap().identity();
        let record = probe.current_view().unwrap().account(address).unwrap();
        assert!(
            probe
                .prepare_observer_phase(ConcreteObserverPhaseDelta {
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![])
                    }],
                    ..Default::default()
                })
                .is_err()
        );
        assert!(
            probe
                .prepare_observer_phase(ConcreteObserverPhaseDelta {
                    accounts: vec![
                        ConcreteAccountMutation::Delete { address },
                        ConcreteAccountMutation::Delete { address }
                    ],
                    ..Default::default()
                })
                .is_err()
        );
        let view = probe.current_view().unwrap();
        assert_eq!(view.identity(), identity);
        assert_eq!(view.account(address).unwrap(), record);
        assert_eq!(
            view.storage(address, key).unwrap(),
            ConcreteRead::Present(vec![7])
        );
        assert_eq!(view.code(code_hash).unwrap(), ConcreteRead::Present(code));
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        assert_eq!(probe.current_view().unwrap().identity(), identity);
    }

    #[test]
    fn probe_open_exact_and_historical_errors() {
        let database = TestDb::new();
        let (first, committed, _) = seed_two_periods(&database, [0x64; 20]);
        let wrong = ConcreteStateIdentity {
            state_root: [0x77; 32],
            ..committed
        };
        assert!(matches!(
            ConcreteExecutionProbe::open_read_only(&database.path, wrong, first),
            Err(ConcreteReadError::IdentityMismatch { .. })
        ));
        assert!(matches!(
            ConcreteExecutionProbe::open_read_only(&database.path, committed, wrong),
            Err(ConcreteReadError::IdentityMismatch { .. })
        ));
        let future = ConcreteStateIdentity {
            period: committed.period.checked_next().unwrap(),
            ..committed
        };
        assert!(matches!(
            ConcreteExecutionProbe::open_read_only(&database.path, committed, future),
            Err(ConcreteReadError::FuturePeriod { .. })
        ));
        let missing = ConcreteStateIdentity {
            state_root: [0x55; 32],
            ..first
        };
        assert!(
            matches!(ConcreteExecutionProbe::open_read_only(&database.path, committed, missing), Err(ConcreteReadError::HistoryUnavailable(id)) if id == missing)
        );
        let corrupt_root = ConcreteStateIdentity {
            state_root: [0x66; 32],
            ..first
        };
        let db = open_existing_database(&database.path, false).unwrap();
        db.put_cf(
            &db.cf_handle("2").unwrap(),
            corrupt_root.state_root,
            [1, 2, 3],
        )
        .unwrap();
        drop(db);
        assert!(matches!(
            ConcreteExecutionProbe::open_read_only(&database.path, committed, corrupt_root),
            Err(ConcreteReadError::Corrupt(_))
        ));
        let overflow = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(u64::MAX),
            ..committed
        };
        assert!(matches!(
            ConcreteExecutionProbe::open_read_only(&database.path, overflow, overflow),
            Err(ConcreteReadError::Corrupt(_))
        ));
    }

    #[test]
    fn probe_prior_code_checks_hash_and_missing_history() {
        use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
        let database = TestDb::new();
        let (_, committed, _) = seed_two_periods(&database, [0x66; 20]);
        let code = vec![0x60, 3, 0];
        let code_hash = keccak256(&code);
        let writer = ConcreteStateWriter::open(&database.path, committed).unwrap();
        let prepared = writer
            .prepare(
                committed.period.checked_next().unwrap(),
                ConcreteStateMutationBatch {
                    code: vec![ConcreteCodeInsertion {
                        code_hash,
                        code: code.clone(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        writer.persist_contents(&prepared).unwrap();
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let committed = prepared.next_identity();
        drop(writer);
        let mut probe =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, committed).unwrap();
        assert_eq!(
            probe.current_view().unwrap().code(code_hash).unwrap(),
            ConcreteRead::Present(code.clone())
        );
        assert!(
            matches!(probe.current_view().unwrap().code([0x88; 32]), Err(ConcreteReadError::HistoryUnavailable(id)) if id == committed)
        );
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        assert_eq!(
            probe.current_view().unwrap().code(code_hash).unwrap(),
            ConcreteRead::Present(code)
        );
        drop(probe);
        let db = open_existing_database(&database.path, false).unwrap();
        db.put_cf(&db.cf_handle("1").unwrap(), code_hash, [0xff])
            .unwrap();
        drop(db);
        let mut probe =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, committed).unwrap();
        assert!(matches!(
            probe.current_view().unwrap().code(code_hash),
            Err(ConcreteReadError::Corrupt(_))
        ));
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        assert!(matches!(
            probe.current_view().unwrap().code(code_hash),
            Err(ConcreteReadError::Corrupt(_))
        ));
    }

    #[test]
    fn probe_read_only_preserves_all_columns_wal_and_reopen() {
        use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
        let database = TestDb::new();
        let address = [0x65; 20];
        let (_, committed, committed_account) = seed_two_periods(&database, address);
        let options = Options::default();
        let columns = DB::list_cf(&options, &database.path).unwrap();
        let descriptors = columns
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
        let mut db = DB::open_cf_descriptors(&options, &database.path, descriptors).unwrap();
        db.create_cf("probe_extra", &Options::default()).unwrap();
        db.create_cf("probe_empty", &Options::default()).unwrap();
        let extra = db.cf_handle("probe_extra").unwrap();
        db.put_cf(&extra, b"wal-only", b"preserve").unwrap();
        db.flush_wal(true).unwrap();
        drop(db);
        let wal_before = probe_wal_files(&database.path);
        assert!(!wal_before.is_empty());
        let db = open_existing_database(&database.path, true).unwrap();
        assert!(
            db.live_files()
                .unwrap()
                .iter()
                .all(|file| file.column_family_name != "probe_extra")
        );
        drop(db);
        let before = probe_all_columns(&database.path);
        assert!(
            before
                .iter()
                .any(|(name, rows)| name == "probe_empty" && rows.is_empty())
        );
        assert!(before.iter().any(|(name, rows)| name == "probe_extra"
            && rows == &vec![(b"wal-only".to_vec(), b"preserve".to_vec())]));
        let mut probe =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, committed).unwrap();
        probe
            .prepare_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address,
                    record: account(8, 20, None, None, 0),
                }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(probe_all_columns(&database.path), before);
        assert_eq!(probe_wal_files(&database.path), wal_before);
        assert!(
            probe
                .prepare_observer_phase(ConcreteObserverPhaseDelta {
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key: ConcreteStorageKey([1; 32]),
                        value: Some(vec![])
                    }],
                    ..Default::default()
                })
                .is_err()
        );
        assert!(matches!(
            probe
                .current_view()
                .unwrap()
                .storage(address, ConcreteStorageKey([2; 32])),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        assert_eq!(probe_all_columns(&database.path), before);
        assert_eq!(probe_wal_files(&database.path), wal_before);
        drop(probe);
        let reader = ConcreteStateReader::open_read_only(&database.path, committed).unwrap();
        assert_eq!(
            ConcreteStateRead::account(&reader, address).unwrap(),
            ConcreteRead::Present(committed_account)
        );
        drop(reader);
        let mut fresh =
            ConcreteExecutionProbe::open_read_only(&database.path, committed, committed).unwrap();
        fresh
            .prepare_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        assert_eq!(
            fresh.current_view().unwrap().identity().state_root,
            committed.state_root
        );
        drop(fresh);
        assert_eq!(probe_all_columns(&database.path), before);
        assert_eq!(probe_wal_files(&database.path), wal_before);
    }

    type ProbeColumnRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;

    fn probe_all_columns(path: &Path) -> ProbeColumnRows {
        let mut columns = DB::list_cf(&Options::default(), path).unwrap();
        columns.sort();
        let db = open_existing_database(path, true).unwrap();
        columns
            .into_iter()
            .map(|name| {
                let cf = db.cf_handle(&name).unwrap();
                let rows = db
                    .iterator_cf(&cf, IteratorMode::Start)
                    .map(|row| {
                        let (key, value) = row.unwrap();
                        (key.to_vec(), value.to_vec())
                    })
                    .collect();
                (name, rows)
            })
            .collect()
    }

    fn probe_wal_files(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
            .map(|path| {
                (
                    path.file_name().unwrap().into(),
                    std::fs::read(path).unwrap(),
                )
            })
            .collect()
    }

    fn account(
        nonce: u64,
        balance: u64,
        storage_root: Option<[u8; 32]>,
        code_hash: Option<[u8; 32]>,
        code_size: u64,
    ) -> ConcreteAccountRecord {
        let mut stream = RlpStream::new_list(5);
        stream.append(&nonce);
        stream.append(&balance);
        match storage_root {
            Some(root) => stream.append(&root.as_slice()),
            None => stream.append_empty_data(),
        };
        match code_hash {
            Some(hash) => stream.append(&hash.as_slice()),
            None => stream.append_empty_data(),
        };
        stream.append(&code_size);
        ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: FinalChainNonce::from_u64(nonce),
                balance: ConcreteAccountBalance::new(BigUint::from(balance)),
                storage_root,
                code_hash,
                code_size,
            },
            physical_rlp: stream.out().to_vec(),
        }
    }

    #[test]
    fn borrowed_probe_private_constructor_checks_owner_identity_and_sequence() {
        let database = TestDb::new();
        let (_, committed, _) = seed_two_periods(&database, [0x98; 20]);
        let mut writer = ConcreteStateWriter::open(&database.path, committed).unwrap();
        assert!(BorrowedConcreteExecutionProbe::from_owner(&writer).is_ok());
        writer.persisted_sequence.set(Some(1));
        assert!(BorrowedConcreteExecutionProbe::from_owner(&writer).is_err());
        writer.persisted_sequence.set(None);
        writer.prior.period = 0.into();
        assert!(BorrowedConcreteExecutionProbe::from_owner(&writer).is_err());
        writer.prior = committed;
        writer.durable_prior.state_root = [0; 32];
        writer.prior = writer.durable_prior;
        assert!(BorrowedConcreteExecutionProbe::from_owner(&writer).is_err());
        writer.prior = ConcreteStateIdentity {
            period: u64::MAX.into(),
            state_root: committed.state_root,
        };
        writer.durable_prior = writer.prior;
        writer
            .db
            .put(DESCRIPTOR_KEY, encode_descriptor(writer.prior))
            .unwrap();
        assert!(BorrowedConcreteExecutionProbe::from_owner(&writer).is_err());
    }

    fn seed_two_periods(
        database: &TestDb,
        address: [u8; 20],
    ) -> (
        ConcreteStateIdentity,
        ConcreteStateIdentity,
        ConcreteAccountRecord,
    ) {
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: empty_trie_root(),
        };
        let writer = ConcreteStateWriter::open(&database.path, genesis).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: account(0, 100, None, None, 0),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        writer.persist_contents(&prepared).unwrap();
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let first = prepared.next_identity();
        drop(writer);

        let second_account = account(1, 90, None, None, 0);
        let writer = ConcreteStateWriter::open(&database.path, first).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(2),
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address,
                        record: second_account.clone(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        writer.persist_contents(&prepared).unwrap();
        writer
            .publish_descriptor_for_reopen_test(&prepared)
            .unwrap();
        let second = prepared.next_identity();
        drop(writer);
        (first, second, second_account)
    }

    fn assert_hex_row(writer: &ConcreteStateWriter, column: &str, key: &str, value: &str) {
        assert_eq!(
            writer.get(column, &hex::decode(key).unwrap()).unwrap(),
            Some(hex::decode(value).unwrap())
        );
    }

    fn assert_version_value(
        writer: &ConcreteStateWriter,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
        value: &str,
    ) {
        assert_eq!(
            writer.get(column, &versioned_key(prefix, period)).unwrap(),
            Some(hex::decode(value).unwrap())
        );
    }

    fn genesis_with_period(period: u64) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(period),
            state_root: empty_trie_root(),
        }
    }

    fn decode_hash(hexadecimal: &str) -> [u8; 32] {
        hex::decode(hexadecimal).unwrap().try_into().unwrap()
    }
}
