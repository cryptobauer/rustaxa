//! Durable concrete-state staging and atomic publication primitives.
//!
//! The lifecycle stores the existing StateAPI marker, provenance, catalog, and
//! descriptor keys. It validates storage facts against application-authored
//! canonical bytes; it does not approve an execution projection or publish the
//! corresponding FinalChain application generation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rocksdb::{ColumnFamilyDescriptor, DB, Options, WriteBatch, WriteOptions};
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::codec::rlp::concrete_lifecycle::{
    concrete_storage_slot_catalog_hash, decode_concrete_execution_marker,
    decode_concrete_state_provenance, decode_concrete_storage_catalog,
    encode_concrete_state_provenance, encode_concrete_storage_catalog,
};
use rustaxa_types::concrete_lifecycle::{
    ConcreteStorageSlot, FINAL_CHAIN_CONCRETE_PROJECTION_VERSION,
    FinalChainConcreteExecutionMarker, FinalChainConcreteIdentity, FinalChainConcreteState,
    FinalChainConcreteStateProvenance,
};
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};

use super::codec::{corrupt, decode_descriptor};
use super::reader::ConcreteStoragePath;
use super::writer::{
    ConcreteObserverPhaseDelta, ConcreteStateMutationBatch, ConcreteStateWriter,
    PreparedConcreteState, PreparedConcreteView, REQUIRED_COLUMNS, RowKey, encode_descriptor,
};

const PROVENANCE_KEY: &[u8] = b"rustaxa_concrete_state_provenance_v1";
const PENDING_KEY: &[u8] = b"rustaxa_concrete_execution_pending_v1";
const CATALOG_KEY: &[u8] = b"rustaxa_concrete_storage_catalog_v1";
const DESCRIPTOR_KEY: &[u8] = b"last_committed_descriptor";

/// Exact concrete database facts observed before or after one lifecycle step.
/// Bytes are preserved so FinalChain recovery can compare the StateAPI marker
/// and provenance authored for its pending publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteLifecycleObservation {
    pub identity: FinalChainConcreteIdentity,
    pub committed: ConcreteStateIdentity,
    pub generation: u64,
    pub provenance_rlp: Vec<u8>,
    pub catalog_rlp: Vec<u8>,
    pub pending_marker_rlp: Vec<u8>,
}

/// One physical put emitted by a successful concrete commit.
#[cfg(feature = "synthetic-fixtures")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteCommittedPut {
    column: &'static str,
    key: Vec<u8>,
    value: Vec<u8>,
    intermediate: bool,
}
#[cfg(feature = "synthetic-fixtures")]
impl ConcreteCommittedPut {
    pub fn column(&self) -> &str {
        self.column
    }
    pub fn key(&self) -> &[u8] {
        &self.key
    }
    pub fn value(&self) -> &[u8] {
        &self.value
    }
    pub fn intermediate(&self) -> bool {
        self.intermediate
    }
}
/// Sealed observations of the batch that completed its synchronous write.
#[cfg(feature = "synthetic-fixtures")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteCommittedReceipt {
    identity: FinalChainConcreteIdentity,
    prior: ConcreteStateIdentity,
    next: ConcreteStateIdentity,
    generation: u64,
    puts: Vec<ConcreteCommittedPut>,
}
#[cfg(feature = "synthetic-fixtures")]
impl ConcreteCommittedReceipt {
    pub fn identity(&self) -> FinalChainConcreteIdentity {
        self.identity
    }
    pub fn prior(&self) -> ConcreteStateIdentity {
        self.prior
    }
    pub fn next(&self) -> ConcreteStateIdentity {
        self.next
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn puts(&self) -> &[ConcreteCommittedPut] {
        &self.puts
    }
}

#[cfg(feature = "synthetic-fixtures")]
fn concrete_committed_receipt(
    identity: FinalChainConcreteIdentity,
    prior: ConcreteStateIdentity,
    next: ConcreteStateIdentity,
    generation: u64,
    intermediate: &BTreeMap<RowKey, Vec<u8>>,
    rows: &BTreeMap<RowKey, Vec<u8>>,
) -> ConcreteCommittedReceipt {
    let puts = intermediate
        .iter()
        .map(|(row, value)| (row, value, true))
        .chain(rows.iter().map(|(row, value)| (row, value, false)))
        .map(|(row, value, intermediate)| ConcreteCommittedPut {
            column: row.column,
            key: row.key.clone(),
            value: value.clone(),
            intermediate,
        })
        .collect();
    ConcreteCommittedReceipt {
        identity,
        prior,
        next,
        generation,
        puts,
    }
}

/// Application-approved facts required for one atomic concrete database commit.
/// Storage independently checks these hashes, exact canonical lifecycle bytes,
/// prepared root, generation lineage, and monotonic catalog before writing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteCommitApproval {
    pub marker_rlp: Vec<u8>,
    pub provenance_rlp: Vec<u8>,
    pub catalog_rlp: Vec<u8>,
    pub projection_hash: [u8; 32],
    pub catalog_hash: [u8; 32],
}

/// One exact account row emitted by a successful ordered observer phase.
/// A tombstone records an actual main-trie deletion. `Absent` is never emitted
/// as a change because a missing-path delete is a no-op.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteObserverAccountChange {
    pub address: [u8; 20],
    pub account: ConcreteRead<ConcreteAccountRecord>,
}

/// Sealed result of one successful ordered observer phase.
///
/// The public identity and account rows can feed application projection. Private
/// writer and sequence fields prevent a result from selecting another lifecycle,
/// marker, or phase for a prepared read or final commit.
#[derive(Debug)]
pub struct ConcreteObserverPhaseOutput {
    identity: ConcreteStateIdentity,
    accounts: Vec<ConcreteObserverAccountChange>,
    writer_id: u64,
    sequence: u64,
}

impl ConcreteObserverPhaseOutput {
    /// Returns the root and period derived after this exact observer phase.
    pub fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    /// Borrows exact physical account rows changed by this phase.
    pub fn changed_accounts(&self) -> &[ConcreteObserverAccountChange] {
        &self.accounts
    }
}

/// One opened concrete lifecycle generation. The contained writer and its
/// prepared rows remain private so callers cannot bypass marker/provenance
/// validation on the atomic commit path.
pub struct ConcreteStateLifecycle {
    writer: ConcreteStateWriter,
    provenance: FinalChainConcreteStateProvenance,
    provenance_rlp: Vec<u8>,
    catalog: Vec<ConcreteStorageSlot>,
    catalog_rlp: Vec<u8>,
    pending: Option<FinalChainConcreteExecutionMarker>,
    pending_rlp: Vec<u8>,
    intermediate_content: BTreeMap<RowKey, Vec<u8>>,
    observer_prepared: Option<PreparedConcreteState>,
    legacy_prepared: bool,
    poisoned: bool,
}

struct LoadedLifecycle {
    provenance: FinalChainConcreteStateProvenance,
    provenance_rlp: Vec<u8>,
    catalog: Vec<ConcreteStorageSlot>,
    catalog_rlp: Vec<u8>,
    pending: Option<FinalChainConcreteExecutionMarker>,
    pending_rlp: Vec<u8>,
    committed: ConcreteStateIdentity,
}

impl ConcreteStateLifecycle {
    /// Exclusively creates a new database directory, computes the real period-0
    /// root from `genesis`, generates a nonzero database identity, and installs
    /// rows, descriptor, generation-0 provenance, and catalog in one synchronous
    /// batch. Existing paths are never opened or removed. A failed creation is
    /// retained for inspection.
    pub fn create_fresh_exclusive(
        path: impl AsRef<Path>,
        chain_id: [u8; 32],
        genesis: ConcreteStateMutationBatch,
        genesis_catalog: Vec<ConcreteStorageSlot>,
    ) -> Result<Self, ConcreteReadError> {
        Self::create_initial_exclusive(
            path.as_ref(),
            chain_id,
            genesis,
            genesis_catalog,
            FinalChainBlockNumber::GENESIS,
        )
    }

    /// Exclusively initializes a declared synthetic fixture at its first retained
    /// coordinate. The incremental writer computes the root from complete logical
    /// inputs; this API accepts no expected root, existing database or execution
    /// marker. Generation zero identifies initialization, not executed history.
    ///
    /// This test seam cannot qualify imported or production state. The fixture
    /// owner must authenticate its inventory and paired native/header/scheduler
    /// facts, then execute each subsequent retained period through normal owners.
    #[cfg(feature = "synthetic-fixtures")]
    pub fn create_synthetic_exclusive(
        path: impl AsRef<Path>,
        chain_id: [u8; 32],
        period: FinalChainBlockNumber,
        state: ConcreteStateMutationBatch,
        catalog: Vec<ConcreteStorageSlot>,
    ) -> Result<Self, ConcreteReadError> {
        if period.is_genesis() {
            return Err(corrupt(
                "synthetic nonzero initialization requires a nonzero period",
            ));
        }
        Self::create_initial_exclusive(path.as_ref(), chain_id, state, catalog, period)
    }

    /// Appends one explicitly declared synthetic prehistory state using the real
    /// incremental writer. This fixture-only transition is permitted only while
    /// provenance still identifies initialization and no execution is pending.
    /// It accepts no root or execution marker. Callers pair every returned root
    /// with validated native/account/header seed facts before normal execution.
    #[cfg(feature = "synthetic-fixtures")]
    pub fn initialize_synthetic_successor(
        self,
        state: ConcreteStateMutationBatch,
        mut catalog: Vec<ConcreteStorageSlot>,
    ) -> Result<Self, ConcreteReadError> {
        if self.pending.is_some()
            || self.provenance.plan_hash != [0; 32]
            || self.provenance.projection_hash != [0; 32]
            || self.provenance.transactions_hash != [0; 32]
            || self.provenance.rewards_hash != [0; 32]
            || self.provenance.committed_state.period == 0
        {
            return Err(corrupt(
                "synthetic prehistory requires an unexecuted nonzero initializer",
            ));
        }
        catalog.extend(self.catalog.iter().copied());
        catalog.sort_unstable();
        catalog.dedup();
        if state.storage.iter().any(|row| {
            !catalog.contains(&ConcreteStorageSlot {
                address: row.address,
                key: row.key.0,
            })
        }) {
            return Err(corrupt(
                "synthetic prehistory mutation is absent from its catalog",
            ));
        }
        let period = concrete_identity(self.provenance.committed_state)
            .period
            .checked_next()
            .ok_or_else(|| corrupt("synthetic prehistory period overflow"))?;
        let prepared = self.writer.prepare(period, state)?;
        let committed = prepared.next_identity();
        let mut provenance = self.provenance.clone();
        provenance.generation = provenance
            .generation
            .checked_add(1)
            .ok_or_else(|| corrupt("synthetic initialization generation overflow"))?;
        provenance.committed_state = lifecycle_state(committed);
        provenance.catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        let provenance_rlp = encode_concrete_state_provenance(&provenance);
        decode_concrete_state_provenance(&provenance_rlp).map_err(corrupt)?;
        let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
        let mut batch = WriteBatch::default();
        self.writer
            .append_prepared_contents(&prepared, &mut batch, false)?;
        batch.put(DESCRIPTOR_KEY, encode_descriptor(committed));
        batch.put(PROVENANCE_KEY, provenance_rlp);
        batch.put(CATALOG_KEY, catalog_rlp);
        write_sync(&self.writer.db, batch)?;
        let path = self.writer.path().to_path_buf();
        let chain_id = provenance.identity.chain_id;
        drop(self);
        Self::open(path, chain_id, committed)
    }

    fn create_initial_exclusive(
        path: &Path,
        chain_id: [u8; 32],
        genesis: ConcreteStateMutationBatch,
        mut genesis_catalog: Vec<ConcreteStorageSlot>,
        period: FinalChainBlockNumber,
    ) -> Result<Self, ConcreteReadError> {
        if chain_id == [0; 32] {
            return Err(corrupt("fresh concrete chain identity is zero"));
        }
        std::fs::create_dir(path).map_err(io)?;
        #[cfg(test)]
        crate::closeout_test_scratch::created(path);
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let descriptors = REQUIRED_COLUMNS
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()));
        let db = DB::open_cf_descriptors(&options, path, descriptors).map_err(io)?;
        let writer = ConcreteStateWriter::from_fresh_database(db, path.to_path_buf());
        let prepared = if period.is_genesis() {
            writer.prepare_genesis(genesis)?
        } else {
            #[cfg(feature = "synthetic-fixtures")]
            {
                writer.prepare_synthetic_initial_state(period, genesis)?
            }
            #[cfg(not(feature = "synthetic-fixtures"))]
            {
                return Err(corrupt("nonzero initialization requires fixture support"));
            }
        };
        let database_id = random_database_id()?;
        let identity = FinalChainConcreteIdentity {
            policy_version: FINAL_CHAIN_CONCRETE_PROJECTION_VERSION,
            database_id,
            chain_id,
        };
        genesis_catalog.sort_unstable();
        genesis_catalog.dedup();
        let catalog_rlp = encode_concrete_storage_catalog(genesis_catalog.iter().copied());
        let catalog_hash = concrete_storage_slot_catalog_hash(genesis_catalog.iter().copied());
        let committed = prepared.next_identity();
        let provenance = FinalChainConcreteStateProvenance {
            identity,
            generation: 0,
            plan_hash: [0; 32],
            committed_state: lifecycle_state(committed),
            transactions_hash: [0; 32],
            rewards_hash: [0; 32],
            projection_hash: [0; 32],
            catalog_hash,
        };
        let provenance_rlp = encode_concrete_state_provenance(&provenance);
        // Decode the generated bytes through the strict shared codec before any
        // lifecycle metadata becomes durable.
        decode_concrete_state_provenance(&provenance_rlp).map_err(corrupt)?;

        let mut batch = WriteBatch::default();
        writer.append_prepared_contents(&prepared, &mut batch, true)?;
        batch.put(DESCRIPTOR_KEY, encode_descriptor(committed));
        batch.put(PROVENANCE_KEY, &provenance_rlp);
        batch.put(CATALOG_KEY, &catalog_rlp);
        write_sync(&writer.db, batch)?;
        drop(writer);
        Self::open(path, chain_id, committed)
    }

    /// Opens an existing lifecycle database at the exact expected chain and
    /// descriptor. Provenance, catalog, and any pending marker are decoded and
    /// cross-checked; pending recovery facts are returned, never discarded.
    pub fn open(
        path: impl AsRef<Path>,
        expected_chain_id: [u8; 32],
        expected_committed: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        let writer = ConcreteStateWriter::open(path, expected_committed)?;
        let loaded = load_lifecycle(&writer.db, expected_chain_id, Some(expected_committed))?;
        Ok(Self {
            writer,
            provenance: loaded.provenance,
            provenance_rlp: loaded.provenance_rlp,
            catalog: loaded.catalog,
            catalog_rlp: loaded.catalog_rlp,
            pending: loaded.pending,
            pending_rlp: loaded.pending_rlp,
            intermediate_content: BTreeMap::new(),
            observer_prepared: None,
            legacy_prepared: false,
            poisoned: false,
        })
    }

    /// Inspects an existing concrete database through RocksDB's read-only API
    /// without requiring an application-supplied expected root.
    /// No writable owner may be open for this path during inspection. The observed
    /// descriptor must pair exactly with canonical provenance and catalog for
    /// `expected_chain_id`; any pending marker must extend that descriptor.
    /// This reports recovery facts only and never creates, repairs, adopts, or
    /// publishes the database.
    pub fn inspect_existing(
        path: impl AsRef<Path>,
        expected_chain_id: [u8; 32],
    ) -> Result<ConcreteLifecycleObservation, ConcreteReadError> {
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
        let db =
            DB::open_cf_descriptors_read_only(&options, path, descriptors, false).map_err(io)?;
        let loaded = load_lifecycle(&db, expected_chain_id, None)?;
        Ok(ConcreteLifecycleObservation {
            identity: loaded.provenance.identity,
            committed: loaded.committed,
            generation: loaded.provenance.generation,
            provenance_rlp: loaded.provenance_rlp,
            catalog_rlp: loaded.catalog_rlp,
            pending_marker_rlp: loaded.pending_rlp,
        })
    }

    /// Returns exact persisted lifecycle facts without changing marker or head.
    /// A prior marker write error makes durability ambiguous, so a poisoned
    /// handle refuses observations until the caller drops and reopens it.
    pub fn observation(&self) -> Result<ConcreteLifecycleObservation, ConcreteReadError> {
        self.ensure_usable()?;
        Ok(ConcreteLifecycleObservation {
            identity: self.provenance.identity,
            committed: concrete_identity(self.provenance.committed_state),
            generation: self.provenance.generation,
            provenance_rlp: self.provenance_rlp.clone(),
            catalog_rlp: self.catalog_rlp.clone(),
            pending_marker_rlp: self.pending_rlp.clone(),
        })
    }

    /// Borrows the immutable prior-generation read port over this same RocksDB
    /// handle. It never exposes prepared overlay rows.
    pub fn prior_reader(&self) -> &dyn ConcreteStateRead {
        &self.writer
    }

    /// Opens retained logical-proof access while pinning the durable descriptor.
    /// The caller must independently bind the historical identity to a finalized header.
    pub fn open_retained_checkpoint(
        &self,
        historical: ConcreteStateIdentity,
    ) -> Result<super::reader::ConcreteCheckpointReaders, ConcreteReadError> {
        self.ensure_usable()?;
        let owner = super::reader::ReadOwner::snapshot(self.writer.db.clone())?;
        let committed = concrete_identity(self.provenance.committed_state);
        let loaded = load_lifecycle_with(
            |key| owner.get(key),
            self.provenance.identity.chain_id,
            Some(committed),
        )?;
        if loaded.provenance_rlp != self.provenance_rlp
            || loaded.catalog_rlp != self.catalog_rlp
            || loaded.pending_rlp != self.pending_rlp
        {
            return Err(corrupt("concrete lifecycle snapshot metadata mismatch"));
        }
        let retained = if historical == committed {
            vec![committed]
        } else {
            vec![committed, historical]
        };
        super::reader::ConcreteCheckpointReaders::from_owner(owner, committed, retained)
    }

    /// Verifies logical storage membership at the lifecycle's prior identity.
    ///
    /// Prepared overlay rows are excluded. This result is separate from raw
    /// storage history. Missing or invalid proof dependencies return errors.
    pub fn verify_prior_storage_path(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        self.writer.verify_prior_storage_path(address, key)
    }

    /// Applies one settled observer phase to the latest unpublished prepared
    /// root and physical overlay. Storage operations run in supplied order.
    /// On failure the preceding phase remains current and readable.
    ///
    /// A durable execution marker must already be staged, and all phases remain
    /// in its single consecutive period. This method writes no database rows.
    pub fn apply_observer_phase(
        &mut self,
        delta: ConcreteObserverPhaseDelta,
    ) -> Result<ConcreteObserverPhaseOutput, ConcreteReadError> {
        self.ensure_usable()?;
        if self.legacy_prepared {
            return Err(corrupt(
                "ordered observer phases cannot follow cumulative preparation",
            ));
        }
        let marker = self
            .pending
            .as_ref()
            .ok_or_else(|| corrupt("concrete execution marker is not staged"))?;
        let next_period = FinalChainBlockNumber::new(marker.period);
        let prepared = self.writer.prepare_observer_phase(
            next_period,
            self.observer_prepared.as_ref(),
            delta,
        )?;
        let accounts = prepared
            .changed_accounts()
            .iter()
            .map(|(address, account)| ConcreteObserverAccountChange {
                address: *address,
                account: account.clone(),
            })
            .collect();
        let (writer_id, sequence) = prepared.token();
        let output = ConcreteObserverPhaseOutput {
            identity: prepared.next_identity(),
            accounts,
            writer_id,
            sequence,
        };
        self.observer_prepared = Some(prepared);
        Ok(output)
    }

    /// Borrows the fixed execution-only view selected by the latest phase
    /// output. Advancing or discarding the lifecycle requires mutable access,
    /// so the view cannot move while an execution journal borrows it.
    pub fn prepared_view<'a>(
        &'a self,
        output: &ConcreteObserverPhaseOutput,
    ) -> Result<PreparedConcreteView<'a>, ConcreteReadError> {
        self.ensure_usable()?;
        let prepared = self
            .observer_prepared
            .as_ref()
            .ok_or_else(|| corrupt("no ordered observer phase is prepared"))?;
        let (writer_id, sequence) = self.writer.prepared_token(prepared)?;
        if output.writer_id != writer_id
            || output.sequence != sequence
            || output.identity != prepared.next_identity()
        {
            return Err(corrupt("ordered observer phase output is stale or foreign"));
        }
        self.writer.prepared_view(prepared)
    }

    /// Durably stages exact canonical execution-marker bytes before execution.
    /// Repeating the same marker is idempotent; a different pending marker is
    /// rejected as ambiguous. A RocksDB write error poisons this handle because
    /// durability is uncertain; the caller must drop it and reopen for recovery.
    pub fn stage_execution(&mut self, marker_rlp: &[u8]) -> Result<(), ConcreteReadError> {
        self.ensure_usable()?;
        let marker = decode_concrete_execution_marker(marker_rlp).map_err(corrupt)?;
        validate_marker(&self.provenance, &marker)?;
        if let Some(pending) = &self.pending {
            if pending == &marker && self.pending_rlp == marker_rlp {
                return Ok(());
            }
            return Err(corrupt("a different concrete execution is already pending"));
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        if let Err(error) = self
            .writer
            .db
            .put_opt(PENDING_KEY, marker_rlp, &options)
            .map_err(|e| diagnostic_io(e, "marker_write", "default", "pending_marker"))
        {
            self.poisoned = true;
            return Err(error);
        }
        self.pending = Some(marker);
        self.pending_rlp = marker_rlp.to_vec();
        Ok(())
    }

    /// Prepares compatible rows for one consecutive period without persisting
    /// them. Repeated calls may supply cumulative transaction mutations to
    /// obtain intermediate roots; only the latest preparation remains valid.
    pub fn prepare(
        &mut self,
        next_period: FinalChainBlockNumber,
        mutations: ConcreteStateMutationBatch,
    ) -> Result<PreparedConcreteState, ConcreteReadError> {
        self.ensure_usable()?;
        if self.pending.is_none() {
            return Err(corrupt("concrete execution marker is not staged"));
        }
        if self.observer_prepared.is_some() {
            return Err(corrupt(
                "cumulative preparation cannot follow ordered observer phases",
            ));
        }
        let prepared = self.writer.prepare(next_period, mutations)?;
        self.legacy_prepared = true;
        for (row, value) in &prepared.rows {
            if !matches!(row.column, "1" | "2" | "4") {
                continue;
            }
            match self.intermediate_content.insert(row.clone(), value.clone()) {
                Some(previous) if previous != *value => {
                    return Err(corrupt("intermediate content-addressed row conflicts"));
                }
                _ => {}
            }
        }
        Ok(prepared)
    }

    /// Returns an exact account from the latest prepared overlay or its
    /// authenticated prior fallback over the same database handle.
    pub fn prepared_account(
        &self,
        prepared: &PreparedConcreteState,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.ensure_usable()?;
        self.writer.prepared_account(prepared, address)
    }

    /// Atomically commits prepared CF1-CF5 rows with descriptor, exact approved
    /// provenance, monotonic catalog, and pending-marker deletion. This consumes
    /// the handle; callers reopen and let FinalChain publish application state
    /// only after independently validating the returned exact facts.
    pub fn commit_approved(
        self,
        prepared: PreparedConcreteState,
        approval: ConcreteCommitApproval,
    ) -> Result<ConcreteLifecycleObservation, ConcreteReadError> {
        if self.observer_prepared.is_some() {
            return Err(corrupt(
                "ordered observer state requires its sealed phase output",
            ));
        }
        self.commit_prepared(prepared, approval)
    }

    /// Atomically commits the latest ordered observer overlay through the same
    /// marker, provenance, catalog, descriptor and synchronous write batch as
    /// cumulative preparation. The sealed output must identify the latest phase;
    /// an earlier or foreign output is rejected even when its root is equal.
    pub fn commit_observer_approved(
        mut self,
        output: ConcreteObserverPhaseOutput,
        approval: ConcreteCommitApproval,
    ) -> Result<ConcreteLifecycleObservation, ConcreteReadError> {
        self.ensure_usable()?;
        let prepared = self
            .observer_prepared
            .take()
            .ok_or_else(|| corrupt("no ordered observer phase is prepared"))?;
        let (writer_id, sequence) = self.writer.prepared_token(&prepared)?;
        if output.writer_id != writer_id
            || output.sequence != sequence
            || output.identity != prepared.next_identity()
        {
            return Err(corrupt("ordered observer phase output is stale or foreign"));
        }
        self.commit_prepared(prepared, approval)
    }

    /// Publish only authenticated final account and touched-storage trie nodes from the overlay.
    /// The caller supplies finite ceilings; failure occurs before database writes.
    pub fn commit_observer_final_trie_nodes(
        mut self,
        output: ConcreteObserverPhaseOutput,
        approval: ConcreteCommitApproval,
        limits: super::reader::ConcreteStorageInventoryLimits,
    ) -> Result<ConcreteLifecycleObservation, super::reader::ConcreteStorageInventoryError> {
        use super::reader::ConcreteStorageInventoryError as Error;
        self.ensure_usable().map_err(Error::Read)?;
        let mut prepared = self
            .observer_prepared
            .take()
            .ok_or_else(|| Error::Read(corrupt("no ordered observer phase is prepared")))?;
        let (writer_id, sequence) = self.writer.prepared_token(&prepared).map_err(Error::Read)?;
        if output.writer_id != writer_id
            || output.sequence != sequence
            || output.identity != prepared.next_identity()
        {
            return Err(Error::Read(corrupt(
                "ordered observer phase output is stale or foreign",
            )));
        }
        self.writer.retain_final_trie_nodes(&mut prepared, limits)?;
        self.commit_prepared(prepared, approval)
            .map_err(Error::Read)
    }

    /// Commits one ordered observer state and returns its successful batch receipt.
    #[cfg(feature = "synthetic-fixtures")]
    pub fn commit_observer_approved_with_receipt(
        mut self,
        output: ConcreteObserverPhaseOutput,
        approval: ConcreteCommitApproval,
    ) -> Result<(ConcreteLifecycleObservation, ConcreteCommittedReceipt), ConcreteReadError> {
        self.ensure_usable()?;
        let prepared = self
            .observer_prepared
            .take()
            .ok_or_else(|| corrupt("no ordered observer phase is prepared"))?;
        let (writer_id, sequence) = self.writer.prepared_token(&prepared)?;
        if output.writer_id != writer_id
            || output.sequence != sequence
            || output.identity != prepared.next_identity()
        {
            return Err(corrupt("ordered observer phase output is stale or foreign"));
        }
        let mut receipt = None;
        let observation = self.commit_prepared_inner(
            prepared,
            approval,
            |identity, prior, next, generation, intermediate, rows| {
                receipt = Some(concrete_committed_receipt(
                    identity,
                    prior,
                    next,
                    generation,
                    intermediate,
                    rows,
                ));
            },
        )?;
        Ok((
            observation,
            receipt.expect("commit callback follows successful write"),
        ))
    }

    /// Commits final trie nodes and returns the successful physical batch receipt.
    #[cfg(feature = "synthetic-fixtures")]
    pub fn commit_observer_final_trie_nodes_with_receipt(
        mut self,
        output: ConcreteObserverPhaseOutput,
        approval: ConcreteCommitApproval,
        limits: super::reader::ConcreteStorageInventoryLimits,
    ) -> Result<
        (ConcreteLifecycleObservation, ConcreteCommittedReceipt),
        super::reader::ConcreteStorageInventoryError,
    > {
        use super::reader::ConcreteStorageInventoryError as Error;
        self.ensure_usable().map_err(Error::Read)?;
        let mut prepared = self
            .observer_prepared
            .take()
            .ok_or_else(|| Error::Read(corrupt("no ordered observer phase is prepared")))?;
        let (writer_id, sequence) = self.writer.prepared_token(&prepared).map_err(Error::Read)?;
        if output.writer_id != writer_id
            || output.sequence != sequence
            || output.identity != prepared.next_identity()
        {
            return Err(Error::Read(corrupt(
                "ordered observer phase output is stale or foreign",
            )));
        }
        self.writer.retain_final_trie_nodes(&mut prepared, limits)?;
        let mut receipt = None;
        let observation = self
            .commit_prepared_inner(
                prepared,
                approval,
                |identity, prior, next, generation, intermediate, rows| {
                    receipt = Some(concrete_committed_receipt(
                        identity,
                        prior,
                        next,
                        generation,
                        intermediate,
                        rows,
                    ));
                },
            )
            .map_err(Error::Read)?;
        Ok((
            observation,
            receipt.expect("commit callback follows successful write"),
        ))
    }

    fn commit_prepared(
        self,
        prepared: PreparedConcreteState,
        approval: ConcreteCommitApproval,
    ) -> Result<ConcreteLifecycleObservation, ConcreteReadError> {
        self.commit_prepared_inner(prepared, approval, |_, _, _, _, _, _| {})
    }

    fn commit_prepared_inner<F>(
        self,
        prepared: PreparedConcreteState,
        approval: ConcreteCommitApproval,
        on_commit: F,
    ) -> Result<ConcreteLifecycleObservation, ConcreteReadError>
    where
        F: FnOnce(
            FinalChainConcreteIdentity,
            ConcreteStateIdentity,
            ConcreteStateIdentity,
            u64,
            &BTreeMap<RowKey, Vec<u8>>,
            &BTreeMap<RowKey, Vec<u8>>,
        ),
    {
        self.ensure_usable()?;
        let marker = self
            .pending
            .as_ref()
            .ok_or_else(|| corrupt("concrete execution marker is not staged"))?;
        if approval.marker_rlp != self.pending_rlp {
            return Err(corrupt(
                "approved concrete marker differs from staged bytes",
            ));
        }
        let approved_marker =
            decode_concrete_execution_marker(&approval.marker_rlp).map_err(corrupt)?;
        if &approved_marker != marker {
            return Err(corrupt(
                "approved concrete marker fields differ from staged marker",
            ));
        }
        if prepared.next_identity().period.as_u64() != marker.period {
            return Err(corrupt(
                "prepared concrete period differs from staged marker",
            ));
        }
        let provenance =
            decode_concrete_state_provenance(&approval.provenance_rlp).map_err(corrupt)?;
        let catalog = decode_concrete_storage_catalog(&approval.catalog_rlp).map_err(corrupt)?;
        let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        if catalog_hash != approval.catalog_hash
            || provenance.catalog_hash != approval.catalog_hash
            || provenance.projection_hash != approval.projection_hash
        {
            return Err(corrupt("approved projection or catalog hash mismatch"));
        }
        if !self
            .catalog
            .iter()
            .all(|prior| catalog.binary_search(prior).is_ok())
        {
            return Err(corrupt("concrete storage catalog is not monotonic"));
        }
        let expected = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: lifecycle_state(prepared.next_identity()),
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash: approval.projection_hash,
            catalog_hash: approval.catalog_hash,
        };
        if provenance != expected {
            return Err(corrupt(
                "approved provenance does not match prepared concrete state",
            ));
        }

        let mut batch = WriteBatch::default();
        for (row, value) in &self.intermediate_content {
            let handle = self
                .writer
                .db
                .cf_handle(row.column)
                .ok_or_else(|| corrupt("intermediate content column disappeared"))?;
            if let Some(existing) = self.writer.db.get_cf(&handle, &row.key).map_err(|e| {
                diagnostic_io(e, "prior_prepared_get", row.column, "intermediate_content")
            })? && existing != *value
            {
                return Err(corrupt(
                    "intermediate content row conflicts with durable bytes",
                ));
            }
            batch.put_cf(&handle, &row.key, value);
        }
        self.writer
            .append_prepared_contents(&prepared, &mut batch, false)?;
        batch.put(DESCRIPTOR_KEY, encode_descriptor(prepared.next_identity()));
        batch.put(PROVENANCE_KEY, &approval.provenance_rlp);
        batch.put(CATALOG_KEY, &approval.catalog_rlp);
        batch.delete(PENDING_KEY);
        write_sync(&self.writer.db, batch).map_err(|e| {
            #[cfg(feature = "synthetic-fixtures")]
            if std::env::var_os("RUST_LIB_BACKTRACE").as_deref() == Some(std::ffi::OsStr::new("1")) {
                eprintln!("sst_diagnostic operation=commit_write column=multiple key=commit_batch error={e:?}");
            }
            e
        })?;
        on_commit(
            provenance.identity,
            concrete_identity(self.provenance.committed_state),
            prepared.next_identity(),
            provenance.generation,
            &self.intermediate_content,
            &prepared.rows,
        );
        Ok(ConcreteLifecycleObservation {
            identity: provenance.identity,
            committed: prepared.next_identity(),
            generation: provenance.generation,
            provenance_rlp: approval.provenance_rlp,
            catalog_rlp: approval.catalog_rlp,
            pending_marker_rlp: Vec::new(),
        })
    }

    /// Deletes only the exact staged marker in a synchronous write. Prepared
    /// rows were memory-only and are invalidated. A RocksDB write error poisons
    /// this handle; the caller must drop it and reopen before another action.
    pub fn discard_execution(&mut self, marker_rlp: &[u8]) -> Result<(), ConcreteReadError> {
        self.ensure_usable()?;
        if self.pending.is_none() || self.pending_rlp != marker_rlp {
            return Err(corrupt(
                "concrete discard marker differs from pending bytes",
            ));
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        self.writer.invalidate_preparations()?;
        if let Err(error) = self.writer.db.delete_opt(PENDING_KEY, &options).map_err(io) {
            self.poisoned = true;
            return Err(error);
        }
        self.pending = None;
        self.pending_rlp.clear();
        self.intermediate_content.clear();
        self.observer_prepared = None;
        self.legacy_prepared = false;
        Ok(())
    }

    fn ensure_usable(&self) -> Result<(), ConcreteReadError> {
        if self.poisoned {
            return Err(corrupt(
                "concrete lifecycle write outcome is uncertain; drop and reopen",
            ));
        }
        Ok(())
    }
}

fn validate_marker(
    provenance: &FinalChainConcreteStateProvenance,
    marker: &FinalChainConcreteExecutionMarker,
) -> Result<(), ConcreteReadError> {
    if marker.identity != provenance.identity
        || marker.generation
            != provenance
                .generation
                .checked_add(1)
                .ok_or_else(|| corrupt("concrete lifecycle generation overflow"))?
        || marker.prior_state != provenance.committed_state
    {
        return Err(corrupt("concrete execution marker lineage mismatch"));
    }
    Ok(())
}

fn lifecycle_state(identity: ConcreteStateIdentity) -> FinalChainConcreteState {
    FinalChainConcreteState {
        period: identity.period.as_u64(),
        root: identity.state_root,
    }
}

fn concrete_identity(state: FinalChainConcreteState) -> ConcreteStateIdentity {
    ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(state.period),
        state_root: state.root,
    }
}

fn load_lifecycle(
    db: &DB,
    expected_chain_id: [u8; 32],
    expected_committed: Option<ConcreteStateIdentity>,
) -> Result<LoadedLifecycle, ConcreteReadError> {
    load_lifecycle_with(|key| db.get(key), expected_chain_id, expected_committed)
}

fn load_lifecycle_with(
    get: impl Fn(&[u8]) -> Result<Option<Vec<u8>>, rocksdb::Error>,
    expected_chain_id: [u8; 32],
    expected_committed: Option<ConcreteStateIdentity>,
) -> Result<LoadedLifecycle, ConcreteReadError> {
    let descriptor_rlp = required_default(&get, DESCRIPTOR_KEY, "descriptor")?;
    let committed = decode_descriptor(&descriptor_rlp)?;
    if encode_descriptor(committed) != descriptor_rlp {
        return Err(corrupt(
            "concrete lifecycle descriptor is not canonical RLP",
        ));
    }
    if let Some(expected) = expected_committed
        && committed != expected
    {
        return Err(ConcreteReadError::IdentityMismatch {
            expected,
            observed: committed,
        });
    }

    let provenance_rlp = required_default(&get, PROVENANCE_KEY, "provenance")?;
    let provenance = decode_concrete_state_provenance(&provenance_rlp).map_err(corrupt)?;
    if provenance.identity.chain_id != expected_chain_id {
        return Err(corrupt("concrete lifecycle chain identity mismatch"));
    }
    if concrete_identity(provenance.committed_state) != committed {
        return Err(corrupt(
            "concrete lifecycle provenance does not match the committed descriptor",
        ));
    }

    let catalog_rlp = required_default(&get, CATALOG_KEY, "storage catalog")?;
    let catalog = decode_concrete_storage_catalog(&catalog_rlp).map_err(corrupt)?;
    if concrete_storage_slot_catalog_hash(catalog.iter().copied()) != provenance.catalog_hash {
        return Err(corrupt("concrete lifecycle storage catalog hash mismatch"));
    }

    let (pending, pending_rlp) = match get(PENDING_KEY).map_err(io)? {
        None => (None, Vec::new()),
        Some(bytes) => {
            let marker = decode_concrete_execution_marker(&bytes).map_err(corrupt)?;
            validate_marker(&provenance, &marker)?;
            (Some(marker), bytes)
        }
    };

    Ok(LoadedLifecycle {
        provenance,
        provenance_rlp,
        catalog,
        catalog_rlp,
        pending,
        pending_rlp,
        committed,
    })
}

fn required_default(
    get: &impl Fn(&[u8]) -> Result<Option<Vec<u8>>, rocksdb::Error>,
    key: &[u8],
    label: &str,
) -> Result<Vec<u8>, ConcreteReadError> {
    get(key)
        .map_err(io)?
        .ok_or_else(|| corrupt(format!("concrete lifecycle {label} is missing")))
}

fn random_database_id() -> Result<[u8; 32], ConcreteReadError> {
    let mut database_id = [0_u8; 32];
    getrandom::fill(&mut database_id).map_err(io)?;
    if database_id == [0; 32] {
        return Err(corrupt("generated concrete database identity is zero"));
    }
    Ok(database_id)
}

fn write_sync(db: &DB, batch: WriteBatch) -> Result<(), ConcreteReadError> {
    let mut options = WriteOptions::default();
    options.set_sync(true);
    db.write_opt(batch, &options).map_err(io)
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
    use std::sync::atomic::{AtomicU64, Ordering};

    use num_bigint::BigUint;
    use rlp::RlpStream;
    use rustaxa_types::FinalChainNonce;
    use rustaxa_types::codec::rlp::concrete_lifecycle::{
        concrete_state_bytes_digest, encode_concrete_execution_marker,
    };
    use rustaxa_types::concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteStorageKey,
    };

    use super::*;
    use crate::{ConcreteAccountMutation, ConcreteStorageMutation};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestPath(PathBuf);

    impl TestPath {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustaxa-concrete-lifecycle-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            crate::closeout_test_scratch::register(&path);
            Self(path)
        }
    }

    impl Drop for TestPath {
        fn drop(&mut self) {
            if self.0.exists() && !crate::closeout_test_scratch::preserve(&self.0) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn retained_snapshot_keeps_code_absence_and_owner_lifetime() {
        let path = TestPath::new();
        let chain = [0x36; 32];
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let committed = lifecycle.observation().unwrap().committed;
        let old = lifecycle.open_retained_checkpoint(committed).unwrap();
        let code = b"future snapshot code";
        let hash = super::super::codec::keccak256(code);
        assert!(matches!(
            old.code_at(committed, hash),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        let cf = lifecycle.writer.db.cf_handle("1").unwrap();
        lifecycle.writer.db.put_cf(&cf, hash, code).unwrap();
        lifecycle.writer.db.flush_cf(&cf).unwrap();
        lifecycle
            .writer
            .db
            .compact_range_cf::<&[u8], &[u8]>(&cf, None, None);
        let fresh = lifecycle.open_retained_checkpoint(committed).unwrap();
        assert!(matches!(
            old.code_at(committed, hash),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        assert_eq!(
            fresh.code_at(committed, hash).unwrap(),
            ConcreteRead::Present(code.to_vec())
        );
        let catalog = lifecycle.catalog_rlp.clone();
        lifecycle.writer.db.put(CATALOG_KEY, [0xff]).unwrap();
        assert!(lifecycle.open_retained_checkpoint(committed).is_err());
        assert!(matches!(
            old.code_at(committed, hash),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        assert_eq!(
            fresh.code_at(committed, hash).unwrap(),
            ConcreteRead::Present(code.to_vec())
        );
        lifecycle.writer.db.put(CATALOG_KEY, catalog).unwrap();
        let owner = super::super::reader::ReadOwner::snapshot(lifecycle.writer.db.clone()).unwrap();
        assert!(
            super::super::reader::ConcreteCheckpointReaders::from_owner(
                owner,
                committed,
                [committed, committed]
            )
            .is_err()
        );
        let owner = super::super::reader::ReadOwner::snapshot(lifecycle.writer.db.clone()).unwrap();
        assert!(
            super::super::reader::ConcreteCheckpointReaders::from_owner(owner, committed, [])
                .is_err()
        );
        let future = ConcreteStateIdentity {
            period: 1.into(),
            state_root: committed.state_root,
        };
        assert!(lifecycle.open_retained_checkpoint(future).is_err());
        let wrong = ConcreteStateIdentity {
            period: committed.period,
            state_root: [0x77; 32],
        };
        assert!(lifecycle.open_retained_checkpoint(wrong).is_err());
        lifecycle.poisoned = true;
        assert!(lifecycle.open_retained_checkpoint(committed).is_err());
        drop(lifecycle);
        assert!(matches!(
            old.code_at(committed, hash),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        drop(old);
        drop(fresh);
        assert_eq!(
            ConcreteStateLifecycle::inspect_existing(&path.0, chain)
                .unwrap()
                .committed,
            committed
        );
        drop(ConcreteStateLifecycle::open(&path.0, chain, committed).unwrap());
    }

    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn synthetic_nonzero_initialization_computes_root_and_preserves_strict_history() {
        let path = TestPath::new();
        let address = [0xaa; 20];
        let key = ConcreteStorageKey([1; 32]);
        let initial = ConcreteStateMutationBatch {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address,
                record: account(1, 100),
            }],
            storage: vec![ConcreteStorageMutation {
                address,
                key,
                value: Some(vec![0x11]),
            }],
            code: Vec::new(),
        };
        let catalog = vec![ConcreteStorageSlot {
            address,
            key: key.0,
        }];
        let chain = [0x33; 32];
        assert!(
            ConcreteStateLifecycle::create_synthetic_exclusive(
                &path.0,
                chain,
                0.into(),
                initial.clone(),
                catalog.clone()
            )
            .is_err()
        );
        assert!(!path.0.exists());
        let owner = ConcreteStateLifecycle::create_synthetic_exclusive(
            &path.0,
            chain,
            25_706_889.into(),
            initial.clone(),
            catalog.clone(),
        )
        .unwrap();
        let observed = owner.observation().unwrap();
        assert_eq!(observed.committed.period.as_u64(), 25_706_889);
        assert_eq!(observed.generation, 0);
        assert!(observed.pending_marker_rlp.is_empty());
        assert_eq!(
            owner.prior_reader().storage(address, key).unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        drop(owner);
        assert!(
            ConcreteStateLifecycle::create_synthetic_exclusive(
                &path.0,
                chain,
                25_706_889.into(),
                initial.clone(),
                catalog.clone()
            )
            .is_err()
        );
        let reopened = ConcreteStateLifecycle::open(&path.0, chain, observed.committed).unwrap();
        assert_eq!(reopened.observation().unwrap(), observed);
        drop(reopened);
        let genesis_path = TestPath::new();
        let genesis = ConcreteStateLifecycle::create_fresh_exclusive(
            &genesis_path.0,
            chain,
            initial,
            catalog,
        )
        .unwrap();
        // Trie content is independent of the physical row version coordinate.
        assert_eq!(
            genesis.observation().unwrap().committed.state_root,
            observed.committed.state_root
        );
        let old = super::super::ConcreteStateReader::open_historical_read_only(
            &path.0,
            observed.committed,
            ConcreteStateIdentity {
                period: 25_706_888.into(),
                state_root: observed.committed.state_root,
            },
        )
        .unwrap();
        assert!(matches!(
            old.account(address),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
    }

    #[test]
    fn creates_stages_recovers_and_atomically_commits_exact_lifecycle() {
        let path = TestPath::new();
        let chain_id = [0x33; 32];
        let address = {
            let mut address = [0_u8; 20];
            address[19] = 0xaa;
            address
        };
        let key = ConcreteStorageKey({
            let mut key = [0_u8; 32];
            key[31] = 1;
            key
        });
        let catalog = vec![ConcreteStorageSlot {
            address,
            key: key.0,
        }];
        let genesis = ConcreteStateMutationBatch {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address,
                record: account(1, 100),
            }],
            storage: vec![ConcreteStorageMutation {
                address,
                key,
                value: Some(vec![0x11]),
            }],
            code: Vec::new(),
        };
        let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            genesis,
            catalog.clone(),
        )
        .unwrap();
        let genesis_observed = lifecycle.observation().unwrap();
        assert_eq!(genesis_observed.generation, 0);
        assert_eq!(
            hex::encode(genesis_observed.committed.state_root),
            "e8269fee2b975af999fca4ae0f98390c2b86a727e060065ecaf870aea2cf951b"
        );
        assert_ne!(genesis_observed.identity.database_id, [0; 32]);
        assert!(genesis_observed.pending_marker_rlp.is_empty());
        assert_eq!(
            lifecycle.prior_reader().storage(address, key).unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        drop(lifecycle);
        assert_eq!(
            ConcreteStateLifecycle::inspect_existing(&path.0, chain_id).unwrap(),
            genesis_observed
        );
        let mut lifecycle =
            ConcreteStateLifecycle::open(&path.0, chain_id, genesis_observed.committed).unwrap();

        let marker = FinalChainConcreteExecutionMarker {
            identity: genesis_observed.identity,
            generation: 1,
            plan_hash: [0x44; 32],
            period: 1,
            prior_state: lifecycle_state(genesis_observed.committed),
            transactions_hash: [0x55; 32],
            rewards_hash: [0x66; 32],
        };
        let marker_rlp = encode_concrete_execution_marker(&marker);
        lifecycle.stage_execution(&marker_rlp).unwrap();
        lifecycle.stage_execution(&marker_rlp).unwrap();
        drop(lifecycle);
        let staged = ConcreteStateLifecycle::inspect_existing(&path.0, chain_id).unwrap();
        assert_eq!(staged.committed, genesis_observed.committed);
        assert_eq!(staged.pending_marker_rlp, marker_rlp);

        let mut lifecycle =
            ConcreteStateLifecycle::open(&path.0, chain_id, genesis_observed.committed).unwrap();
        assert_eq!(
            lifecycle.observation().unwrap().pending_marker_rlp,
            marker_rlp
        );
        assert!(lifecycle.discard_execution(&[0xc0]).is_err());
        let retained = lifecycle
            .open_retained_checkpoint(genesis_observed.committed)
            .unwrap();
        let intermediate = lifecycle
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch {
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![0x33]),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        let intermediate_root = intermediate.next_identity().state_root;
        let prepared = lifecycle
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch {
                    storage: vec![ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![0, 0x44]),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            hex::encode(prepared.next_identity().state_root),
            "b9d139c10bb0fe50e36d00dd6235a3b21b9c7ad871cee485b9af2efefffe10c9"
        );
        assert!(matches!(
            lifecycle.prepared_account(&prepared, address).unwrap(),
            ConcreteRead::Present(_)
        ));
        let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
        let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        let projection = b"accepted concrete projection";
        let projection_hash = concrete_state_bytes_digest(projection);
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: lifecycle_state(prepared.next_identity()),
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        };
        let approval = ConcreteCommitApproval {
            marker_rlp: marker_rlp.clone(),
            provenance_rlp: encode_concrete_state_provenance(&provenance),
            catalog_rlp,
            projection_hash,
            catalog_hash,
        };
        assert_eq!(
            retained
                .storage_at(genesis_observed.committed, address, key)
                .unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        let old_account = retained
            .account_at(genesis_observed.committed, address)
            .unwrap();
        assert!(matches!(old_account, ConcreteRead::Present(_)));
        assert_eq!(
            retained
                .account_at(genesis_observed.committed, [0x99; 20])
                .unwrap(),
            ConcreteRead::Absent
        );
        assert_eq!(
            retained
                .verify_storage_path_at(genesis_observed.committed, address, key)
                .unwrap(),
            ConcreteStoragePath::Member(vec![0x11])
        );
        let limits = super::super::reader::ConcreteStorageInventoryLimits {
            max_nodes: 100,
            max_leaves: 100,
            max_value_bytes: 1000,
        };
        let ConcreteRead::Present(old_inventory) = retained
            .storage_inventory_at(genesis_observed.committed, address, limits)
            .unwrap()
        else {
            panic!("old inventory is absent");
        };
        assert_eq!(old_inventory.entries.len(), 1);
        assert_eq!(old_inventory.entries[0].value, vec![0x11]);
        let db = lifecycle.writer.db.clone();
        let committed = lifecycle.commit_approved(prepared, approval).unwrap();
        let cf = db.cf_handle("5").unwrap();
        db.flush_cf(&cf).unwrap();
        db.compact_range_cf::<&[u8], &[u8]>(&cf, None, None);
        let owner = super::super::reader::ReadOwner::snapshot(db.clone()).unwrap();
        let fresh = super::super::reader::ConcreteCheckpointReaders::from_owner(
            owner,
            committed.committed,
            [committed.committed],
        )
        .unwrap();
        assert_eq!(
            retained
                .storage_at(genesis_observed.committed, address, key)
                .unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        assert_eq!(
            fresh.storage_at(committed.committed, address, key).unwrap(),
            ConcreteRead::Present(vec![0, 0x44])
        );
        assert_eq!(
            retained
                .account_at(genesis_observed.committed, address)
                .unwrap(),
            old_account
        );
        let ConcreteRead::Present(fresh_account) =
            fresh.account_at(committed.committed, address).unwrap()
        else {
            panic!("fresh account is absent");
        };
        assert_eq!(
            fresh
                .verify_storage_path_at(committed.committed, address, key)
                .unwrap(),
            ConcreteStoragePath::Member(vec![0, 0x44])
        );
        assert_eq!(
            retained
                .verify_storage_path_at(genesis_observed.committed, address, key)
                .unwrap(),
            ConcreteStoragePath::Member(vec![0x11])
        );
        assert_eq!(
            retained
                .storage_inventory_at(genesis_observed.committed, address, limits)
                .unwrap(),
            ConcreteRead::Present(old_inventory)
        );
        let ConcreteRead::Present(inventory) = fresh
            .storage_inventory_at(committed.committed, address, limits)
            .unwrap()
        else {
            panic!("fresh inventory is absent");
        };
        assert_eq!(inventory.entries.len(), 1);
        assert_eq!(inventory.entries[0].value, vec![0, 0x44]);
        let forged = ConcreteStateIdentity {
            period: genesis_observed.committed.period,
            state_root: [0x77; 32],
        };
        let owner = super::super::reader::ReadOwner::snapshot(db.clone()).unwrap();
        assert!(
            super::super::reader::ConcreteCheckpointReaders::from_owner(
                owner,
                committed.committed,
                [committed.committed, forged]
            )
            .is_err()
        );

        let absent_key = ConcreteStorageKey([0x99; 32]);
        let tombstone_key = super::super::codec::versioned_key(
            super::super::codec::storage_version_prefix(address, absent_key),
            committed.committed.period,
        );
        let values = db.cf_handle("5").unwrap();
        db.put_cf(&values, tombstone_key, []).unwrap();
        let owner = super::super::reader::ReadOwner::snapshot(db.clone()).unwrap();
        let tombstone = super::super::reader::ConcreteCheckpointReaders::from_owner(
            owner,
            committed.committed,
            [committed.committed],
        )
        .unwrap();
        assert_eq!(
            tombstone
                .storage_at(committed.committed, address, absent_key)
                .unwrap(),
            ConcreteRead::Tombstone
        );
        assert_eq!(
            tombstone
                .verify_storage_path_at(committed.committed, address, absent_key)
                .unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert!(matches!(
            fresh.storage_at(committed.committed, address, absent_key),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        drop(tombstone);
        db.delete_cf(&values, tombstone_key).unwrap();

        let storage_root = fresh_account.account.storage_root.unwrap();
        let nodes = db.cf_handle("4").unwrap();
        let node = db.get_cf(&nodes, storage_root).unwrap().unwrap();
        db.delete_cf(&nodes, storage_root).unwrap();
        let owner = super::super::reader::ReadOwner::snapshot(db.clone()).unwrap();
        let missing = super::super::reader::ConcreteCheckpointReaders::from_owner(
            owner,
            committed.committed,
            [committed.committed],
        )
        .unwrap();
        assert!(matches!(
            missing.verify_storage_path_at(committed.committed, address, key),
            Err(ConcreteReadError::HistoryUnavailable(_))
        ));
        drop(missing);
        db.put_cf(&nodes, storage_root, [0xff]).unwrap();
        let owner = super::super::reader::ReadOwner::snapshot(db.clone()).unwrap();
        let corrupt = super::super::reader::ConcreteCheckpointReaders::from_owner(
            owner,
            committed.committed,
            [committed.committed],
        )
        .unwrap();
        assert!(matches!(
            corrupt.verify_storage_path_at(committed.committed, address, key),
            Err(ConcreteReadError::Corrupt(_))
        ));
        drop(corrupt);
        db.put_cf(&nodes, storage_root, node).unwrap();
        drop(retained);
        drop(fresh);
        drop(db);
        assert_eq!(committed.generation, 1);
        assert!(committed.pending_marker_rlp.is_empty());
        assert_eq!(
            ConcreteStateLifecycle::inspect_existing(&path.0, chain_id).unwrap(),
            committed
        );

        let mut lifecycle =
            ConcreteStateLifecycle::open(&path.0, chain_id, committed.committed).unwrap();
        assert_eq!(lifecycle.observation().unwrap(), committed);
        let nodes = lifecycle.writer.db.cf_handle("2").unwrap();
        assert!(
            lifecycle
                .writer
                .db
                .get_cf(&nodes, intermediate_root)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            lifecycle.prior_reader().storage(address, key).unwrap(),
            ConcreteRead::Present(vec![0, 0x44])
        );

        let discard_marker = FinalChainConcreteExecutionMarker {
            identity: committed.identity,
            generation: 2,
            plan_hash: [0x77; 32],
            period: 2,
            prior_state: lifecycle_state(committed.committed),
            transactions_hash: [0x88; 32],
            rewards_hash: [0x99; 32],
        };
        let discard_rlp = encode_concrete_execution_marker(&discard_marker);
        lifecycle.stage_execution(&discard_rlp).unwrap();
        lifecycle.discard_execution(&discard_rlp).unwrap();
        assert!(
            lifecycle
                .observation()
                .unwrap()
                .pending_marker_rlp
                .is_empty()
        );
    }

    #[test]
    fn inspection_rejects_foreign_chain_and_missing_or_corrupt_metadata() {
        let missing = TestPath::new();
        let chain_id = [0x31; 32];
        let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &missing.0,
            chain_id,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        drop(lifecycle);

        assert!(ConcreteStateLifecycle::inspect_existing(&missing.0, [0x32; 32]).is_err());
        mutate_default(&missing.0, PROVENANCE_KEY, None);
        assert!(ConcreteStateLifecycle::inspect_existing(&missing.0, chain_id).is_err());

        let corrupt_path = TestPath::new();
        let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &corrupt_path.0,
            chain_id,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let observed = lifecycle.observation().unwrap();
        drop(lifecycle);
        mutate_default(&corrupt_path.0, CATALOG_KEY, Some(&[0xff]));
        assert!(ConcreteStateLifecycle::inspect_existing(&corrupt_path.0, chain_id).is_err());
        mutate_default(&corrupt_path.0, CATALOG_KEY, Some(&observed.catalog_rlp));
        let mismatched = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(1),
            state_root: observed.committed.state_root,
        };
        mutate_default(
            &corrupt_path.0,
            DESCRIPTOR_KEY,
            Some(&encode_descriptor(mismatched)),
        );
        assert!(ConcreteStateLifecycle::inspect_existing(&corrupt_path.0, chain_id).is_err());
        mutate_default(
            &corrupt_path.0,
            DESCRIPTOR_KEY,
            Some(&encode_descriptor(observed.committed)),
        );
        mutate_default(&corrupt_path.0, PENDING_KEY, Some(&[]));
        assert!(ConcreteStateLifecycle::inspect_existing(&corrupt_path.0, chain_id).is_err());
    }

    #[test]
    fn exclusive_creation_and_monotonic_catalog_fail_closed() {
        let path = TestPath::new();
        let chain_id = [1; 32];
        let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            ConcreteStateMutationBatch::default(),
            vec![ConcreteStorageSlot {
                address: [2; 20],
                key: [3; 32],
            }],
        )
        .unwrap();
        assert!(matches!(
            ConcreteStateLifecycle::create_fresh_exclusive(
                &path.0,
                chain_id,
                ConcreteStateMutationBatch::default(),
                Vec::new(),
            ),
            Err(ConcreteReadError::Io(_))
        ));

        let observation = lifecycle.observation().unwrap();
        let marker = FinalChainConcreteExecutionMarker {
            identity: observation.identity,
            generation: 1,
            plan_hash: [4; 32],
            period: 1,
            prior_state: lifecycle_state(observation.committed),
            transactions_hash: [5; 32],
            rewards_hash: [6; 32],
        };
        let marker_rlp = encode_concrete_execution_marker(&marker);
        let mut lifecycle = lifecycle;
        lifecycle.stage_execution(&marker_rlp).unwrap();
        let prepared = lifecycle
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch::default(),
            )
            .unwrap();
        let empty_catalog = Vec::new();
        let catalog_rlp = encode_concrete_storage_catalog(empty_catalog.iter().copied());
        let catalog_hash = concrete_storage_slot_catalog_hash(empty_catalog.iter().copied());
        let projection_hash = [7; 32];
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: 1,
            plan_hash: marker.plan_hash,
            committed_state: lifecycle_state(prepared.next_identity()),
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        };
        let approval = ConcreteCommitApproval {
            marker_rlp,
            provenance_rlp: encode_concrete_state_provenance(&provenance),
            catalog_rlp,
            projection_hash,
            catalog_hash,
        };
        assert!(lifecycle.commit_approved(prepared, approval).is_err());
    }

    #[test]
    fn discarded_preparation_cannot_be_rebound_to_another_marker() {
        let path = TestPath::new();
        let chain_id = [0x10; 32];
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let observation = lifecycle.observation().unwrap();
        let marker_a = FinalChainConcreteExecutionMarker {
            identity: observation.identity,
            generation: 1,
            plan_hash: [0x11; 32],
            period: 1,
            prior_state: lifecycle_state(observation.committed),
            transactions_hash: [0x12; 32],
            rewards_hash: [0x13; 32],
        };
        let marker_a_rlp = encode_concrete_execution_marker(&marker_a);
        lifecycle.stage_execution(&marker_a_rlp).unwrap();
        let stale = lifecycle
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch::default(),
            )
            .unwrap();
        lifecycle.discard_execution(&marker_a_rlp).unwrap();

        let marker_b = FinalChainConcreteExecutionMarker {
            plan_hash: [0x21; 32],
            transactions_hash: [0x22; 32],
            rewards_hash: [0x23; 32],
            ..marker_a
        };
        let marker_b_rlp = encode_concrete_execution_marker(&marker_b);
        lifecycle.stage_execution(&marker_b_rlp).unwrap();
        let catalog = Vec::new();
        let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
        let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        let projection_hash = [0x24; 32];
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker_b.identity,
            generation: marker_b.generation,
            plan_hash: marker_b.plan_hash,
            committed_state: lifecycle_state(stale.next_identity()),
            transactions_hash: marker_b.transactions_hash,
            rewards_hash: marker_b.rewards_hash,
            projection_hash,
            catalog_hash,
        };
        let approval = ConcreteCommitApproval {
            marker_rlp: marker_b_rlp,
            provenance_rlp: encode_concrete_state_provenance(&provenance),
            catalog_rlp,
            projection_hash,
            catalog_hash,
        };
        assert!(lifecycle.commit_approved(stale, approval).is_err());
    }

    #[test]
    fn ordered_put_then_raw_delete_matches_pinned_tombstone() {
        let path = TestPath::new();
        let chain_id = [0x41; 32];
        let address = address(0xaa);
        let key = storage_key(1);
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            ConcreteStateMutationBatch {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address,
                    record: account(1, 100),
                }],
                ..Default::default()
            },
            Vec::new(),
        )
        .unwrap();
        let observed = lifecycle.observation().unwrap();
        let (_marker, marker_rlp) = marker(&observed, 0x42);
        lifecycle.stage_execution(&marker_rlp).unwrap();

        let output = lifecycle
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                storage: vec![
                    ConcreteStorageMutation {
                        address,
                        key,
                        value: Some(vec![1]),
                    },
                    ConcreteStorageMutation {
                        address,
                        key,
                        value: None,
                    },
                ],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            hex::encode(output.identity().state_root),
            "237820c1eb1d743f94d99c8faf766e168377d9c2224b7a08ba3ce62b5ec6f97a"
        );
        assert_eq!(output.changed_accounts().len(), 1);
        {
            let view = lifecycle.prepared_view(&output).unwrap();
            assert_eq!(
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    &view, address, key,
                )
                .unwrap(),
                ConcreteRead::Tombstone
            );
            assert!(matches!(
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    &view,
                    address,
                    storage_key(9),
                ),
                Err(ConcreteReadError::HistoryUnavailable(identity)) if identity == output.identity()
            ));
        }

        let fixture = ordered_fixture();
        assert_eq!(
            prepared_columns(lifecycle.observer_prepared.as_ref().unwrap()),
            fixture["ordered_put_raw_delete"]["columns"]
        );
        lifecycle.discard_execution(&marker_rlp).unwrap();
        assert!(lifecycle.prepared_view(&output).is_err());
    }

    #[test]
    fn ordered_delete_recreate_preserves_orphans_and_commits_atomically() {
        let path = TestPath::new();
        let chain_id = [0x51; 32];
        let address = address(0xaa);
        let key_one = storage_key(1);
        let key_two = storage_key(2);
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let observed = lifecycle.observation().unwrap();
        let (marker, marker_rlp) = marker(&observed, 0x52);
        lifecycle.stage_execution(&marker_rlp).unwrap();

        let seed = lifecycle
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address,
                    record: account(1, 100),
                }],
                storage: vec![ConcreteStorageMutation {
                    address,
                    key: key_one,
                    value: Some(vec![0x11]),
                }],
                code: Vec::new(),
            })
            .unwrap();
        assert_eq!(
            hex::encode(seed.identity().state_root),
            "e8269fee2b975af999fca4ae0f98390c2b86a727e060065ecaf870aea2cf951b"
        );

        let deleted = lifecycle
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Delete { address }],
                ..Default::default()
            })
            .unwrap();
        assert!(lifecycle.prepared_view(&seed).is_err());
        assert_eq!(
            hex::encode(deleted.identity().state_root),
            "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
        );
        {
            let deleted_view = lifecycle.prepared_view(&deleted).unwrap();
            assert_eq!(
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                    &deleted_view,
                    address,
                )
                .unwrap(),
                ConcreteRead::Tombstone
            );
            assert_eq!(
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    &deleted_view,
                    address,
                    key_one,
                )
                .unwrap(),
                ConcreteRead::Present(vec![0x11])
            );
        }

        let recreated = lifecycle
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address,
                    record: account(2, 90),
                }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            hex::encode(recreated.identity().state_root),
            "d20632b3bf474f10287ff869a0b80dd489127e5410ad726052766f2dd4460a8f"
        );
        {
            let recreated_view = lifecycle.prepared_view(&recreated).unwrap();
            assert_eq!(
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    &recreated_view,
                    address,
                    key_one,
                )
                .unwrap(),
                ConcreteRead::Present(vec![0x11])
            );
        }

        let contradictory = lifecycle.apply_observer_phase(ConcreteObserverPhaseDelta {
            accounts: vec![ConcreteAccountMutation::Delete { address }],
            storage: vec![ConcreteStorageMutation {
                address,
                key: key_two,
                value: Some(vec![0x22]),
            }],
            ..Default::default()
        });
        assert!(contradictory.is_err());
        assert!(lifecycle.prepared_view(&recreated).is_ok());

        let final_phase = lifecycle
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                storage: vec![ConcreteStorageMutation {
                    address,
                    key: key_two,
                    value: Some(vec![0x22]),
                }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            hex::encode(final_phase.identity().state_root),
            "e3e9fcb0c82680f256eba6c039c343bbe2966558caf353a276ef5e3abde87840"
        );
        {
            let final_view = lifecycle.prepared_view(&final_phase).unwrap();
            for (key, value) in [(key_one, 0x11), (key_two, 0x22)] {
                assert_eq!(
                    rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                        &final_view,
                        address,
                        key,
                    )
                    .unwrap(),
                    ConcreteRead::Present(vec![value])
                );
            }
        }
        let fixture = ordered_fixture();
        assert_eq!(
            prepared_columns(lifecycle.observer_prepared.as_ref().unwrap()),
            fixture["delete_recreate"][3]["columns"]
        );

        let orphan_delete = lifecycle
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                storage: vec![ConcreteStorageMutation {
                    address,
                    key: key_one,
                    value: None,
                }],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(orphan_delete.identity(), final_phase.identity());
        assert!(orphan_delete.changed_accounts().is_empty());
        assert!(lifecycle.prepared_view(&final_phase).is_err());
        {
            let orphan_view = lifecycle.prepared_view(&orphan_delete).unwrap();
            assert_eq!(
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    &orphan_view,
                    address,
                    key_one,
                )
                .unwrap(),
                ConcreteRead::Present(vec![0x11])
            );
        }
        assert_eq!(
            prepared_columns(lifecycle.observer_prepared.as_ref().unwrap()),
            fixture["delete_recreate"][3]["columns"]
        );

        let catalog = vec![
            ConcreteStorageSlot {
                address,
                key: key_one.0,
            },
            ConcreteStorageSlot {
                address,
                key: key_two.0,
            },
        ];
        let approval = approval(&marker, &marker_rlp, orphan_delete.identity(), &catalog);
        let committed = lifecycle
            .commit_observer_approved(orphan_delete, approval)
            .unwrap();
        assert_eq!(
            committed.committed.state_root,
            hex_hash("e3e9fcb0c82680f256eba6c039c343bbe2966558caf353a276ef5e3abde87840")
        );

        let reopened =
            ConcreteStateLifecycle::open(&path.0, chain_id, committed.committed).unwrap();
        assert_eq!(
            reopened.prior_reader().storage(address, key_one).unwrap(),
            ConcreteRead::Present(vec![0x11])
        );
        assert_eq!(
            reopened.prior_reader().storage(address, key_two).unwrap(),
            ConcreteRead::Present(vec![0x22])
        );
    }

    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn committed_receipt_is_returned_only_after_exact_success() {
        let path = TestPath::new();
        let chain = [0x51; 32];
        let mut owner = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let prior = owner.observation().unwrap();
        let (marker, marker_rlp) = marker(&prior, 0x52);
        owner.stage_execution(&marker_rlp).unwrap();
        let output = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let next = output.identity();
        let (committed, receipt) = owner
            .commit_observer_approved_with_receipt(
                output,
                approval(&marker, &marker_rlp, next, &[]),
            )
            .unwrap();
        assert_eq!(receipt.identity(), committed.identity);
        assert_eq!(receipt.prior(), prior.committed);
        assert_eq!(receipt.next(), committed.committed);
        assert_eq!(receipt.generation(), committed.generation);
        let reopened = ConcreteStateLifecycle::open(&path.0, chain, committed.committed).unwrap();
        assert_eq!(reopened.observation().unwrap(), committed);

        let bad_path = TestPath::new();
        let mut bad = staged_empty_lifecycle(&bad_path, [0x53; 32], 0x54);
        let pending = bad.pending.as_ref().unwrap().clone();
        let marker_rlp = bad.pending_rlp.clone();
        let stale = bad
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let latest = bad
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        assert!(
            bad.commit_observer_approved_with_receipt(
                stale,
                approval(&pending, &marker_rlp, latest.identity(), &[])
            )
            .is_err()
        );
    }

    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn committed_receipt_is_not_released_on_actual_write_error() {
        let path = TestPath::new();
        let spare = TestPath::new();
        let chain = [0x57; 32];
        let mut owner = staged_empty_lifecycle(&path, chain, 0x58);
        let prior = owner.observation().unwrap();
        let pending = owner.pending.as_ref().unwrap().clone();
        let marker_rlp = owner.pending_rlp.clone();
        let output = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let approval = approval(&pending, &marker_rlp, output.identity(), &[]);
        let prepared = owner.observer_prepared.take().unwrap();
        assert_eq!(std::sync::Arc::strong_count(&owner.writer.db), 1);
        let spare_db = DB::open_default(&spare.0).unwrap();
        let old = std::mem::replace(&mut owner.writer.db, std::sync::Arc::new(spare_db));
        drop(old);
        let options = Options::default();
        let columns = DB::list_cf(&options, &path.0).unwrap();
        let descriptors = columns
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
        let read_only =
            DB::open_cf_descriptors_read_only(&options, &path.0, descriptors, false).unwrap();
        let spare_db = std::mem::replace(&mut owner.writer.db, std::sync::Arc::new(read_only));
        drop(spare_db);
        let called = std::cell::Cell::new(false);
        let error = owner
            .commit_prepared_inner(prepared, approval, |_, _, _, _, _, _| called.set(true))
            .unwrap_err();
        assert!(
            matches!(error, ConcreteReadError::Io(_)),
            "expected actual RocksDB write error: {error:?}"
        );
        assert!(!called.get());
        let after = ConcreteStateLifecycle::inspect_existing(&path.0, chain).unwrap();
        assert_eq!(after, prior);
    }

    #[test]
    fn observer_outputs_are_lifecycle_bound_and_preparation_modes_do_not_mix() {
        let path_a = TestPath::new();
        let path_b = TestPath::new();
        let path_c = TestPath::new();
        let mut observer_a = staged_empty_lifecycle(&path_a, [0x61; 32], 0x62);
        let mut observer_b = staged_empty_lifecycle(&path_b, [0x63; 32], 0x64);
        let output_a = observer_a
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let output_b = observer_b
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        assert!(observer_a.prepared_view(&output_b).is_err());
        assert!(
            observer_a
                .prepare(
                    FinalChainBlockNumber::new(1),
                    ConcreteStateMutationBatch::default(),
                )
                .is_err()
        );
        assert!(observer_a.prepared_view(&output_a).is_ok());

        let mut cumulative = staged_empty_lifecycle(&path_c, [0x65; 32], 0x66);
        cumulative
            .prepare(
                FinalChainBlockNumber::new(1),
                ConcreteStateMutationBatch::default(),
            )
            .unwrap();
        assert!(
            cumulative
                .apply_observer_phase(ConcreteObserverPhaseDelta::default())
                .is_err()
        );
    }

    fn ordered_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../../experiments/evm_feasibility/fixtures/ordered_overlay.json"
        ))
        .unwrap()
    }

    fn prepared_columns(prepared: &PreparedConcreteState) -> serde_json::Value {
        let mut columns = serde_json::Map::new();
        for name in ["code", "main_nodes", "accounts", "storage_nodes", "slots"] {
            columns.insert(
                name.to_owned(),
                serde_json::Value::Object(Default::default()),
            );
        }
        for (row, value) in &prepared.rows {
            let (name, key) = match row.column {
                "1" => ("code", row.key.as_slice()),
                "2" => ("main_nodes", row.key.as_slice()),
                "3" => {
                    assert_eq!(row.key.len(), 40);
                    assert_eq!(
                        row.key[32..],
                        prepared.next_identity().period.as_u64().to_be_bytes()
                    );
                    ("accounts", &row.key[..32])
                }
                "4" => ("storage_nodes", row.key.as_slice()),
                "5" => {
                    assert_eq!(row.key.len(), 40);
                    assert_eq!(
                        row.key[32..],
                        prepared.next_identity().period.as_u64().to_be_bytes()
                    );
                    ("slots", &row.key[..32])
                }
                column => panic!("unexpected prepared column {column}"),
            };
            columns[name].as_object_mut().unwrap().insert(
                hex::encode(key),
                serde_json::Value::String(hex::encode(value)),
            );
        }
        serde_json::Value::Object(columns)
    }

    fn marker(
        observed: &ConcreteLifecycleObservation,
        seed: u8,
    ) -> (FinalChainConcreteExecutionMarker, Vec<u8>) {
        let marker = FinalChainConcreteExecutionMarker {
            identity: observed.identity,
            generation: observed.generation + 1,
            plan_hash: [seed; 32],
            period: observed.committed.period.as_u64() + 1,
            prior_state: lifecycle_state(observed.committed),
            transactions_hash: [seed.wrapping_add(1); 32],
            rewards_hash: [seed.wrapping_add(2); 32],
        };
        let rlp = encode_concrete_execution_marker(&marker);
        (marker, rlp)
    }

    fn staged_empty_lifecycle(
        path: &TestPath,
        chain_id: [u8; 32],
        marker_seed: u8,
    ) -> ConcreteStateLifecycle {
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let observed = lifecycle.observation().unwrap();
        let (_, marker_rlp) = marker(&observed, marker_seed);
        lifecycle.stage_execution(&marker_rlp).unwrap();
        lifecycle
    }

    fn approval(
        marker: &FinalChainConcreteExecutionMarker,
        marker_rlp: &[u8],
        committed: ConcreteStateIdentity,
        catalog: &[ConcreteStorageSlot],
    ) -> ConcreteCommitApproval {
        let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
        let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        let projection_hash = concrete_state_bytes_digest(b"ordered observer projection");
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: lifecycle_state(committed),
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        };
        ConcreteCommitApproval {
            marker_rlp: marker_rlp.to_vec(),
            provenance_rlp: encode_concrete_state_provenance(&provenance),
            catalog_rlp,
            projection_hash,
            catalog_hash,
        }
    }

    #[test]
    fn public_final_account_nodes_keep_reachable_overlay_and_fail_before_write() {
        use super::super::reader::{ConcreteStorageInventoryError, ConcreteStorageInventoryLimits};
        let path = TestPath::new();
        let chain_id = [0x72; 32];
        let mut owner = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain_id,
            ConcreteStateMutationBatch {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address: address(1),
                    record: account(0, 10),
                }],
                ..Default::default()
            },
            Vec::new(),
        )
        .unwrap();
        let prior = owner.observation().unwrap();
        let (marker, marker_rlp) = marker(&prior, 0x73);
        owner.stage_execution(&marker_rlp).unwrap();
        let first = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: (2..32)
                    .map(|i| ConcreteAccountMutation::Upsert {
                        address: address(i),
                        record: account(0, u64::from(i)),
                    })
                    .collect(),
                ..Default::default()
            })
            .unwrap();
        let first_rows = owner.observer_prepared.as_ref().unwrap().rows.clone();
        assert!(matches!(
            rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                &owner.prepared_view(&first).unwrap(),
                address(2)
            )
            .unwrap(),
            ConcreteRead::Present(_)
        ));
        let final_output = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address: address(1),
                    record: account(1, 11),
                }],
                ..Default::default()
            })
            .unwrap();
        let before = owner.observer_prepared.as_ref().unwrap().rows.clone();
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 10_000,
            max_leaves: 1_000,
            max_value_bytes: 1_000_000,
        };
        let mut prepared = owner.observer_prepared.take().unwrap();
        let too_small = ConcreteStorageInventoryLimits {
            max_nodes: 0,
            ..limits
        };
        assert!(matches!(
            owner
                .writer
                .retain_final_trie_nodes(&mut prepared, too_small),
            Err(ConcreteStorageInventoryError::LimitExceeded { .. })
        ));
        assert_eq!(prepared.rows, before);
        let root_key = RowKey {
            column: "2",
            key: final_output.identity().state_root.to_vec(),
        };
        let original_root = prepared.rows.remove(&root_key).unwrap();
        assert!(
            owner
                .writer
                .retain_final_trie_nodes(&mut prepared, limits)
                .is_err()
        );
        prepared.rows.insert(root_key.clone(), vec![0xff]);
        assert!(
            owner
                .writer
                .retain_final_trie_nodes(&mut prepared, limits)
                .is_err()
        );
        prepared.rows.insert(root_key, original_root);
        owner
            .writer
            .retain_final_trie_nodes(&mut prepared, limits)
            .unwrap();
        assert!(first_rows.iter().any(|(key, value)| key.column == "2"
            && key.key != first.identity().state_root
            && prepared.rows.get(key) == Some(value)));
        assert!(
            before
                .iter()
                .any(|(key, _)| key.column == "2" && !prepared.rows.contains_key(key))
        );
        assert!(
            before
                .iter()
                .filter(|(key, _)| key.column != "2")
                .all(|(key, value)| prepared.rows.get(key) == Some(value))
        );
        owner.observer_prepared = Some(prepared);
        let final_identity = final_output.identity();
        let committed = owner
            .commit_observer_final_trie_nodes(
                final_output,
                approval(&marker, &marker_rlp, final_identity, &[]),
                limits,
            )
            .unwrap();
        assert_eq!(committed.pending_marker_rlp, Vec::<u8>::new());
        let reopened =
            ConcreteStateLifecycle::open(&path.0, chain_id, committed.committed).unwrap();
        assert!(matches!(
            reopened.prior_reader().account(address(2)).unwrap(),
            ConcreteRead::Present(_)
        ));
        drop(reopened);
        let prior_reader = super::super::reader::ConcreteStateReader::open_historical_read_only(
            &path.0,
            committed.committed,
            prior.committed,
        )
        .unwrap();
        assert!(matches!(
            prior_reader.account(address(1)).unwrap(),
            ConcreteRead::Present(_)
        ));
    }

    #[test]
    fn public_final_account_limit_failure_retains_marker_and_allows_retry() {
        use super::super::reader::{ConcreteStorageInventoryError, ConcreteStorageInventoryLimits};
        let path = TestPath::new();
        let chain = [0x81; 32];
        let mut owner = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let prior = owner.observation().unwrap();
        let (marker, marker_rlp) = marker(&prior, 0x82);
        owner.stage_execution(&marker_rlp).unwrap();
        let delta = ConcreteObserverPhaseDelta {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address: address(9),
                record: account(0, 7),
            }],
            ..Default::default()
        };
        let output = owner.apply_observer_phase(delta.clone()).unwrap();
        let final_identity = output.identity();
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 0,
            max_leaves: 100,
            max_value_bytes: 10_000,
        };
        assert!(matches!(
            owner.commit_observer_final_trie_nodes(
                output,
                approval(&marker, &marker_rlp, final_identity, &[]),
                limits
            ),
            Err(ConcreteStorageInventoryError::LimitExceeded { .. })
        ));
        let after = ConcreteStateLifecycle::inspect_existing(&path.0, chain).unwrap();
        assert_eq!(after.committed, prior.committed);
        assert_eq!(after.pending_marker_rlp, marker_rlp);
        let mut retry = ConcreteStateLifecycle::open(&path.0, chain, prior.committed).unwrap();
        let output = retry.apply_observer_phase(delta).unwrap();
        assert_eq!(output.identity(), final_identity);
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 100,
            ..limits
        };
        let committed = retry
            .commit_observer_final_trie_nodes(
                output,
                approval(&marker, &marker_rlp, final_identity, &[]),
                limits,
            )
            .unwrap();
        assert!(committed.pending_marker_rlp.is_empty());
    }

    #[test]
    fn public_final_account_empty_root_and_stale_output() {
        use super::super::reader::ConcreteStorageInventoryLimits;
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 0,
            max_leaves: 0,
            max_value_bytes: 0,
        };
        let path = TestPath::new();
        let chain = [0x83; 32];
        let mut owner = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain,
            ConcreteStateMutationBatch::default(),
            Vec::new(),
        )
        .unwrap();
        let prior = owner.observation().unwrap();
        let (marker, marker_rlp) = marker(&prior, 0x84);
        owner.stage_execution(&marker_rlp).unwrap();
        let stale = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let latest = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let final_identity = latest.identity();
        assert!(
            owner
                .commit_observer_final_trie_nodes(
                    stale,
                    approval(&marker, &marker_rlp, final_identity, &[]),
                    limits
                )
                .is_err()
        );
        let mut retry = ConcreteStateLifecycle::open(&path.0, chain, prior.committed).unwrap();
        let output = retry
            .apply_observer_phase(ConcreteObserverPhaseDelta::default())
            .unwrap();
        let committed = retry
            .commit_observer_final_trie_nodes(
                output,
                approval(&marker, &marker_rlp, final_identity, &[]),
                limits,
            )
            .unwrap();
        assert!(committed.pending_marker_rlp.is_empty());
    }

    #[test]
    fn public_final_storage_children_and_code_survive_reopen() {
        use super::super::reader::ConcreteStorageInventoryLimits;
        let path = TestPath::new();
        let chain = [0x90; 32];
        let owner_address = address(3);
        let code = vec![0x60, 0x01, 0x00];
        let code_hash = super::super::codec::keccak256(&code);
        let mut record = account(0, 9);
        record.account.code_hash = Some(code_hash);
        record.account.code_size = code.len() as u64;
        let mut rlp = RlpStream::new_list(5);
        rlp.append(&0_u64);
        rlp.append(&9_u64);
        rlp.append_empty_data();
        rlp.append(&code_hash.as_slice());
        rlp.append(&(code.len() as u64));
        record.physical_rlp = rlp.out().to_vec();
        let mut owner = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain,
            ConcreteStateMutationBatch {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address: owner_address,
                    record,
                }],
                code: vec![super::super::writer::ConcreteCodeInsertion {
                    code_hash,
                    code: code.clone(),
                }],
                ..Default::default()
            },
            Vec::new(),
        )
        .unwrap();
        let prior = owner.observation().unwrap();
        let (marker, marker_rlp) = marker(&prior, 0x91);
        owner.stage_execution(&marker_rlp).unwrap();
        let first = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                storage: (1..32)
                    .map(|i| ConcreteStorageMutation {
                        address: owner_address,
                        key: storage_key(i),
                        value: Some(vec![i]),
                    })
                    .collect(),
                ..Default::default()
            })
            .unwrap();
        let first_rows = owner.observer_prepared.as_ref().unwrap().rows.clone();
        let output = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                storage: vec![ConcreteStorageMutation {
                    address: owner_address,
                    key: storage_key(1),
                    value: Some(vec![77]),
                }],
                ..Default::default()
            })
            .unwrap();
        let identity = output.identity();
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 10_000,
            max_leaves: 1_000,
            max_value_bytes: 1_000_000,
        };
        let mut prepared = owner.observer_prepared.take().unwrap();
        let original = prepared.rows.clone();
        let one_node = ConcreteStorageInventoryLimits {
            max_nodes: 1,
            ..limits
        };
        assert!(matches!(
            owner
                .writer
                .retain_final_trie_nodes(&mut prepared, one_node),
            Err(super::super::reader::ConcreteStorageInventoryError::LimitExceeded { .. })
        ));
        assert_eq!(prepared.rows, original);
        owner
            .writer
            .retain_final_trie_nodes(&mut prepared, limits)
            .unwrap();
        let root = match owner
            .writer
            .prepared_account(&prepared, owner_address)
            .unwrap()
        {
            ConcreteRead::Present(record) => record.account.storage_root.unwrap(),
            _ => panic!("missing account"),
        };
        let child = prepared
            .rows
            .keys()
            .find(|key| key.column == "4" && key.key != root)
            .unwrap()
            .clone();
        assert!(first_rows.get(&child) == prepared.rows.get(&child));
        assert!(
            original
                .iter()
                .any(|(key, _)| key.column == "4" && !prepared.rows.contains_key(key))
        );
        prepared.rows = original.clone();
        let child_value = prepared.rows.remove(&child).unwrap();
        assert!(
            owner
                .writer
                .retain_final_trie_nodes(&mut prepared, limits)
                .is_err()
        );
        prepared.rows.insert(child.clone(), vec![0xff]);
        assert!(
            owner
                .writer
                .retain_final_trie_nodes(&mut prepared, limits)
                .is_err()
        );
        let mut wrong_branch = RlpStream::new_list(16);
        for _ in 0..16 {
            wrong_branch.append_empty_data();
        }
        prepared
            .rows
            .insert(child.clone(), wrong_branch.out().to_vec());
        let wrong_rows = prepared.rows.clone();
        assert!(
            matches!(owner.writer.retain_final_trie_nodes(&mut prepared, limits),
            Err(super::super::reader::ConcreteStorageInventoryError::Read(ConcreteReadError::Corrupt(message)))
                if message.contains("physical trie child hash mismatch"))
        );
        assert_eq!(prepared.rows, wrong_rows);
        assert_eq!(owner.observation().unwrap().pending_marker_rlp, marker_rlp);
        prepared.rows.insert(child, child_value);
        owner.observer_prepared = Some(prepared);
        let catalog = (1..32)
            .map(|i| ConcreteStorageSlot {
                address: owner_address,
                key: storage_key(i).0,
            })
            .collect::<Vec<_>>();
        let committed = owner
            .commit_observer_final_trie_nodes(
                output,
                approval(&marker, &marker_rlp, identity, &catalog),
                limits,
            )
            .unwrap();
        let reopened = ConcreteStateLifecycle::open(&path.0, chain, committed.committed).unwrap();
        assert_eq!(
            reopened
                .prior_reader()
                .storage(owner_address, storage_key(1))
                .unwrap(),
            ConcreteRead::Present(vec![77])
        );
        assert_eq!(
            reopened
                .prior_reader()
                .storage(owner_address, storage_key(2))
                .unwrap(),
            ConcreteRead::Present(vec![2])
        );
        assert_eq!(
            reopened.prior_reader().code(code_hash).unwrap(),
            ConcreteRead::Present(code)
        );
        assert_ne!(first.identity(), identity);
    }

    #[test]
    fn public_final_storage_tracks_earlier_deleted_and_recreated_owners_with_shared_budget() {
        use super::super::reader::{
            ConcreteStorageInventoryError, ConcreteStorageInventoryLimits,
            ConcreteStorageInventoryResource,
        };
        let path = TestPath::new();
        let chain = [0xa1; 32];
        let owners = [address(1), address(2), address(3), address(4)];
        let mut owner = ConcreteStateLifecycle::create_fresh_exclusive(
            &path.0,
            chain,
            ConcreteStateMutationBatch {
                accounts: owners
                    .iter()
                    .map(|address| ConcreteAccountMutation::Upsert {
                        address: *address,
                        record: account(0, 10),
                    })
                    .collect(),
                ..Default::default()
            },
            Vec::new(),
        )
        .unwrap();
        let prior = owner.observation().unwrap();
        let (marker, marker_rlp) = marker(&prior, 0xa2);
        owner.stage_execution(&marker_rlp).unwrap();
        let first = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                storage: owners
                    .iter()
                    .enumerate()
                    .map(|(i, address)| ConcreteStorageMutation {
                        address: *address,
                        key: storage_key(1),
                        value: Some(vec![(i + 1) as u8]),
                    })
                    .collect(),
                ..Default::default()
            })
            .unwrap();
        assert!(
            matches!(rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            &owner.prepared_view(&first).unwrap(), owners[0], storage_key(1)).unwrap(), ConcreteRead::Present(value) if value == vec![1])
        );
        owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![
                    ConcreteAccountMutation::Delete { address: owners[2] },
                    ConcreteAccountMutation::Delete { address: owners[3] },
                ],
                ..Default::default()
            })
            .unwrap();
        let output = owner
            .apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address: owners[3],
                    record: account(1, 20),
                }],
                storage: vec![ConcreteStorageMutation {
                    address: owners[3],
                    key: storage_key(2),
                    value: Some(vec![44]),
                }],
                ..Default::default()
            })
            .unwrap();
        let identity = output.identity();
        let mut prepared = owner.observer_prepared.take().unwrap();
        let original = prepared.rows.clone();
        let (account_nodes, first_storage_nodes) = owner
            .writer
            .test_account_and_storage_node_counts(&prepared, owners[0]);
        let (same_account_nodes, second_storage_nodes) = owner
            .writer
            .test_account_and_storage_node_counts(&prepared, owners[1]);
        assert_eq!(account_nodes, same_account_nodes);
        assert!(first_storage_nodes > 0 && second_storage_nodes > 0);
        let cap = account_nodes.checked_add(first_storage_nodes).unwrap();
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: cap,
            max_leaves: 1_000,
            max_value_bytes: 1_000_000,
        };
        assert_eq!(
            owner.writer.retain_final_trie_nodes(&mut prepared, limits),
            Err(ConcreteStorageInventoryError::LimitExceeded {
                resource: ConcreteStorageInventoryResource::Nodes,
                limit: cap,
                required: cap + 1
            })
        );
        assert_eq!(prepared.rows, original);
        owner.observer_prepared = Some(prepared);
        let catalog = owners
            .iter()
            .map(|address| ConcreteStorageSlot {
                address: *address,
                key: storage_key(1).0,
            })
            .chain(std::iter::once(ConcreteStorageSlot {
                address: owners[3],
                key: storage_key(2).0,
            }))
            .collect::<Vec<_>>();
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 10_000,
            ..limits
        };
        let committed = owner
            .commit_observer_final_trie_nodes(
                output,
                approval(&marker, &marker_rlp, identity, &catalog),
                limits,
            )
            .unwrap();
        let reopened = ConcreteStateLifecycle::open(&path.0, chain, committed.committed).unwrap();
        for (index, address) in owners[..2].iter().enumerate() {
            assert_eq!(
                reopened
                    .prior_reader()
                    .storage(*address, storage_key(1))
                    .unwrap(),
                ConcreteRead::Present(vec![(index + 1) as u8])
            );
        }
        assert!(matches!(
            reopened.prior_reader().account(owners[2]).unwrap(),
            ConcreteRead::Tombstone
        ));
        assert!(matches!(
            reopened.prior_reader().account(owners[3]).unwrap(),
            ConcreteRead::Present(_)
        ));
        assert_eq!(
            reopened
                .prior_reader()
                .storage(owners[3], storage_key(2))
                .unwrap(),
            ConcreteRead::Present(vec![44])
        );
        drop(reopened);
        let historical = super::super::reader::ConcreteStateReader::open_historical_read_only(
            &path.0,
            committed.committed,
            prior.committed,
        )
        .unwrap();
        for address in owners {
            assert!(matches!(
                historical.account(address).unwrap(),
                ConcreteRead::Present(_)
            ));
        }
    }

    fn address(last: u8) -> [u8; 20] {
        let mut address = [0; 20];
        address[19] = last;
        address
    }

    fn storage_key(last: u8) -> rustaxa_types::concrete_state::ConcreteStorageKey {
        let mut key = [0; 32];
        key[31] = last;
        rustaxa_types::concrete_state::ConcreteStorageKey(key)
    }

    fn hex_hash(value: &str) -> [u8; 32] {
        hex::decode(value).unwrap().try_into().unwrap()
    }

    fn account(nonce: u64, balance: u64) -> ConcreteAccountRecord {
        let mut stream = RlpStream::new_list(5);
        stream.append(&nonce);
        stream.append(&balance);
        stream.append_empty_data();
        stream.append_empty_data();
        stream.append(&0_u64);
        ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: FinalChainNonce::from_u64(nonce),
                balance: ConcreteAccountBalance::new(BigUint::from(balance)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
            physical_rlp: stream.out().to_vec(),
        }
    }

    fn mutate_default(path: &Path, key: &[u8], value: Option<&[u8]>) {
        let mut options = Options::default();
        options.create_if_missing(false);
        options.create_missing_column_families(false);
        let columns = DB::list_cf(&options, path).unwrap();
        let descriptors = columns
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
        let db = DB::open_cf_descriptors(&options, path, descriptors).unwrap();
        match value {
            Some(value) => db.put(key, value).unwrap(),
            None => db.delete(key).unwrap(),
        }
    }
}
