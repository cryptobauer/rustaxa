//! Preparation and unpublished persistence of incremental concrete-state rows.
//!
//! This module deliberately stops before durable head publication. It validates
//! the current descriptor, derives trie roots, and can stage compatible rows in
//! the existing columns. FinalChain lifecycle code must later atomically bind
//! those rows to descriptor, provenance, and publication state.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
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
    PathProof, PhysicalTrieStore, SelectedVersion, TrieSchema, verify_path,
};
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
    pub(super) db: DB,
    path: PathBuf,
    prior: ConcreteStateIdentity,
    writer_id: u64,
    sequence: Cell<u64>,
    persisted_sequence: Cell<Option<u64>>,
}

impl ConcreteStateWriter {
    /// Opens all existing reference columns read-write and verifies `prior`
    /// against the persisted descriptor. No missing file or column is created.
    pub fn open(
        path: impl AsRef<Path>,
        prior: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        let path = path.as_ref();
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
        let db = DB::open_cf_descriptors(&options, path, descriptors).map_err(io)?;
        let observed = current_identity(&db)?;
        if observed != prior {
            return Err(ConcreteReadError::IdentityMismatch {
                expected: prior,
                observed,
            });
        }
        Ok(Self {
            db,
            path: path.to_path_buf(),
            prior,
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
            if current_identity(&self.db)? != self.prior {
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

    /// Persists CF1-CF5 content and version rows for `prepared`, without moving
    /// the descriptor or writing provenance/catalog/publication metadata. Old
    /// roots and old version rows remain readable. This is staging only; a
    /// successful return is not a committed or published state generation.
    pub fn persist_contents(
        &self,
        prepared: &PreparedConcreteState,
    ) -> Result<(), ConcreteReadError> {
        if prepared.writer_id != self.writer_id
            || prepared.prior != self.prior
            || prepared.sequence != self.sequence.get()
            || self.persisted_sequence.get().is_some()
        {
            return Err(corrupt(
                "prepared state is stale, foreign, or already persisted",
            ));
        }
        if current_identity(&self.db)? != self.prior {
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
        } else if current_identity(&self.db)? != self.prior {
            return Err(corrupt("cannot use prepared rows after descriptor changed"));
        }
        for (row, value) in &prepared.rows {
            let handle = self
                .db
                .cf_handle(row.column)
                .ok_or_else(|| corrupt("prepared row column disappeared"))?;
            if row.column == "1" || row.column == "2" || row.column == "4" {
                if let Some(existing) = self.db.get_cf(&handle, &row.key).map_err(io)?
                    && existing != *value
                {
                    return Err(corrupt(
                        "content-addressed row conflicts with existing bytes",
                    ));
                }
            } else if self.db.get_cf(&handle, &row.key).map_err(io)?.is_some() {
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
            db,
            path,
            prior: ConcreteStateIdentity {
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
        if current_identity(&self.db)? != self.prior {
            return Err(corrupt("prepared state prior is no longer the descriptor"));
        }
        Ok(())
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
        self.db.get_cf(&handle, key).map_err(io)
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
        self.writer.db.get_cf(&handle, hash).map_err(io)
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
    let (key, value) = entry.map_err(io)?;
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

fn current_identity(db: &DB) -> Result<ConcreteStateIdentity, ConcreteReadError> {
    let bytes = db
        .get(DESCRIPTOR_KEY)
        .map_err(io)?
        .ok_or_else(|| corrupt("concrete descriptor is missing"))?;
    decode_descriptor(&bytes)
}

pub(super) fn encode_descriptor(identity: ConcreteStateIdentity) -> Vec<u8> {
    let mut stream = RlpStream::new_list(2);
    stream.append(&identity.period.as_u64());
    stream.append(&identity.state_root.as_slice());
    stream.out().to_vec()
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

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDb {
        path: PathBuf,
    }

    impl TestDb {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustaxa-concrete-writer-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
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
