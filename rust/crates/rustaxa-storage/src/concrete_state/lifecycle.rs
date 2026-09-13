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
    ConcreteStateRead,
};

use super::codec::{corrupt, decode_descriptor};
use super::writer::{
    ConcreteStateMutationBatch, ConcreteStateWriter, PreparedConcreteState, REQUIRED_COLUMNS,
    RowKey, encode_descriptor,
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
        mut genesis_catalog: Vec<ConcreteStorageSlot>,
    ) -> Result<Self, ConcreteReadError> {
        if chain_id == [0; 32] {
            return Err(corrupt("fresh concrete chain identity is zero"));
        }
        let path = path.as_ref();
        std::fs::create_dir(path).map_err(io)?;
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let descriptors = REQUIRED_COLUMNS
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()));
        let db = DB::open_cf_descriptors(&options, path, descriptors).map_err(io)?;
        let writer = ConcreteStateWriter::from_fresh_database(db, path.to_path_buf());
        let prepared = writer.prepare_genesis(genesis)?;
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
            poisoned: false,
        })
    }

    /// Inspects an existing concrete database through RocksDB's read-only API
    /// without requiring an application-supplied expected root. The observed
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
            .map_err(io)
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
        let prepared = self.writer.prepare(next_period, mutations)?;
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
            if let Some(existing) = self.writer.db.get_cf(&handle, &row.key).map_err(io)?
                && existing != *value
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
        write_sync(&self.writer.db, batch)?;
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
    let descriptor_rlp = required_default(db, DESCRIPTOR_KEY, "descriptor")?;
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

    let provenance_rlp = required_default(db, PROVENANCE_KEY, "provenance")?;
    let provenance = decode_concrete_state_provenance(&provenance_rlp).map_err(corrupt)?;
    if provenance.identity.chain_id != expected_chain_id {
        return Err(corrupt("concrete lifecycle chain identity mismatch"));
    }
    if concrete_identity(provenance.committed_state) != committed {
        return Err(corrupt(
            "concrete lifecycle provenance does not match the committed descriptor",
        ));
    }

    let catalog_rlp = required_default(db, CATALOG_KEY, "storage catalog")?;
    let catalog = decode_concrete_storage_catalog(&catalog_rlp).map_err(corrupt)?;
    if concrete_storage_slot_catalog_hash(catalog.iter().copied()) != provenance.catalog_hash {
        return Err(corrupt("concrete lifecycle storage catalog hash mismatch"));
    }

    let (pending, pending_rlp) = match db.get(PENDING_KEY).map_err(io)? {
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

fn required_default(db: &DB, key: &[u8], label: &str) -> Result<Vec<u8>, ConcreteReadError> {
    db.get(key)
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
            Self(std::env::temp_dir().join(format!(
                "rustaxa-concrete-lifecycle-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for TestPath {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
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
        let committed = lifecycle.commit_approved(prepared, approval).unwrap();
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
