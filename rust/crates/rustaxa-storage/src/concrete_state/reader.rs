//! Descriptor-pinned RocksDB implementation of the concrete-state read port.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rocksdb::{ColumnFamilyDescriptor, DB, Direction, IteratorMode, Options};
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};

use super::codec::{
    account_version_prefix, decode_descriptor, decode_physical_account, keccak256,
    storage_prefix_for_path, storage_trie_path, storage_version_prefix, versioned_key,
};
use super::physical_node::{
    InventoryError, InventoryLimits, InventoryResource, PathProof, PhysicalTrieStore,
    SelectedVersion, TrieSchema, inventory_storage_trie, verify_path,
};

const DESCRIPTOR_KEY: &[u8] = b"last_committed_descriptor";
const REQUIRED_COLUMNS: &[&str] = &["default", "1", "2", "3", "4", "5", "6", "7", "8"];

/// Immutable reader for one verified concrete-state database generation.
///
/// Construction opens an existing database with RocksDB's read-only API and
/// requires its descriptor to equal the committed identity supplied by
/// FinalChain. An older requested identity is accepted only alongside that
/// verified descriptor and remains subject to per-read proofs. The handle never
/// creates a database or column family. Account reads authenticate
/// one trie path against the pinned root. Storage reads expose the selected
/// physical history row, including orphan rows retained outside a current
/// account storage root. Callers may separately authenticate logical slot
/// membership with [`Self::verify_storage_path`]. A missing row or proof
/// dependency is `HistoryUnavailable`; without explicit pruning evidence this
/// reader never reports `Pruned`.
pub struct ConcreteStateReader {
    db: DB,
    identity: ConcreteStateIdentity,
}

/// One read-only concrete database shared by an explicit set of retained identities.
///
/// Construction pins the durable descriptor independently from the caller's retained
/// roots and authenticates every supplied root before returning. Point reads require an
/// exact listed period/root pair. The handle does not discover a retention range, cache
/// account absence across roots, create database state, or expose mutation/publication
/// authority. Root traversal proves that supplied bytes are accessible and internally
/// consistent; the application remains responsible for sourcing each historical
/// period/root pair from its authoritative finalized header.
pub struct ConcreteCheckpointReaders {
    db: DB,
    committed: ConcreteStateIdentity,
    retained: BTreeMap<FinalChainBlockNumber, ConcreteStateIdentity>,
}

/// Authenticated logical result for one storage path. This diagnostic is
/// separate from [`ConcreteStateRead::storage`], which exposes retained physical
/// history even when a row is orphaned from the current account trie.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteStoragePath {
    Member(Vec<u8>),
    NonMember,
}

/// Explicit ceilings for one authenticated storage-trie inventory.
///
/// Nodes include hashed and embedded decoded trie nodes. Leaves count live storage paths,
/// and value bytes count the exact selected physical values retained in the result. Zero is
/// a valid limit. Exceeding any limit fails without returning a partial inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcreteStorageInventoryLimits {
    pub max_nodes: u64,
    pub max_leaves: u64,
    pub max_value_bytes: u64,
}

/// One authenticated live storage leaf keyed by its irreversible trie path hash.
///
/// `hashed_path` is `keccak256(logical_key)`. The inventory cannot invert it or infer the
/// native/EVM meaning of an unknown key. `value` preserves the exact selected physical bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteStorageInventoryEntry {
    pub hashed_path: [u8; 32],
    pub value: Vec<u8>,
}

/// Complete authenticated live storage leaves for one account at one exact identity.
///
/// Success proves coverage of the live trie rooted at `storage_root`. It does not prove
/// all-ever historical keys, deleted slots, semantic catalog completeness, or linkage of a
/// caller-supplied historical identity to a finalized header. Entries are sorted by hashed
/// path. A present account without a storage root has a complete empty inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConcreteStorageInventory {
    pub identity: ConcreteStateIdentity,
    pub address: [u8; 20],
    pub storage_root: Option<[u8; 32]>,
    pub nodes_visited: u64,
    pub value_bytes: u64,
    pub entries: Vec<ConcreteStorageInventoryEntry>,
}

/// Resource whose caller-supplied inventory ceiling was exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcreteStorageInventoryResource {
    Nodes,
    Leaves,
    ValueBytes,
}

/// Fail-closed outcome for an authenticated storage inventory.
///
/// Read errors preserve identity and integrity diagnostics. Limit errors report the first
/// required count that exceeded its ceiling; neither error exposes partial entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteStorageInventoryError {
    Read(ConcreteReadError),
    LimitExceeded {
        resource: ConcreteStorageInventoryResource,
        limit: u64,
        required: u64,
    },
}

impl std::fmt::Display for ConcreteStorageInventoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "concrete storage inventory: {self:?}")
    }
}

impl std::error::Error for ConcreteStorageInventoryError {}

impl ConcreteStateReader {
    /// Opens `state_db` read-only and pins `expected`. Existing column-family
    /// names are preserved; every reference concrete-state column must exist.
    pub fn open_read_only(
        path: impl AsRef<Path>,
        expected: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        Self::open_historical_read_only(path, expected, expected)
    }

    /// Opens a reader pinned to an older FinalChain-supplied identity while
    /// independently requiring the database's current descriptor to equal
    /// `committed`. This does not attest the caller-supplied historical identity
    /// or assert broad retention. Account and explicit logical-path reads still
    /// authenticate the requested root; raw slot selection remains independent,
    /// and missing physical storage history remains unavailable.
    pub fn open_historical_read_only(
        path: impl AsRef<Path>,
        committed: ConcreteStateIdentity,
        requested: ConcreteStateIdentity,
    ) -> Result<Self, ConcreteReadError> {
        let db = open_database_read_only(path.as_ref())?;
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
        if requested.period.as_u64() > committed.period.as_u64() {
            return Err(ConcreteReadError::FuturePeriod {
                requested: requested.period,
                committed: committed.period,
            });
        }
        if requested.period == committed.period && requested.state_root != committed.state_root {
            return Err(ConcreteReadError::IdentityMismatch {
                expected: requested,
                observed: committed,
            });
        }
        Ok(Self {
            db,
            identity: requested,
        })
    }

    /// Verifies the logical slot path under the pinned account storage root.
    /// Account or slot non-membership is returned independently of any retained
    /// orphan physical row. Missing nodes or referenced values fail closed.
    pub fn verify_storage_path(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        verify_storage_path(self, address, key)
    }
}

impl ConcreteCheckpointReaders {
    /// Opens one existing database read-only for exact caller-authorized identities.
    ///
    /// `committed` must equal the durable descriptor. `retained` must contain that exact
    /// identity, may contain older identities, and must not repeat a period. Each root is
    /// authenticated through a deterministic account path during construction, so even an
    /// empty later workload cannot turn an injected root into a retention claim.
    pub fn open_read_only(
        path: impl AsRef<Path>,
        committed: ConcreteStateIdentity,
        retained: impl IntoIterator<Item = ConcreteStateIdentity>,
    ) -> Result<Self, ConcreteReadError> {
        let db = open_database_read_only(path.as_ref())?;
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

        let mut identities = BTreeMap::new();
        for identity in retained {
            if identity.period.as_u64() > committed.period.as_u64() {
                return Err(ConcreteReadError::FuturePeriod {
                    requested: identity.period,
                    committed: committed.period,
                });
            }
            if let Some(previous) = identities.insert(identity.period, identity) {
                return Err(ConcreteReadError::Corrupt(format!(
                    "duplicate retained concrete period {} (roots equal: {})",
                    identity.period.as_u64(),
                    previous.state_root == identity.state_root,
                )));
            }
        }
        if identities.get(&committed.period) != Some(&committed) {
            return Err(ConcreteReadError::Corrupt(
                "retained concrete identities do not include the durable descriptor".into(),
            ));
        }
        for identity in identities.values().copied() {
            let pinned = PinnedDatabase { db: &db, identity };
            read_account(&pinned, [0; 20])?;
        }

        Ok(Self {
            db,
            committed,
            retained: identities,
        })
    }

    /// Returns the exact descriptor independently observed when the handle opened.
    pub fn committed_identity(&self) -> ConcreteStateIdentity {
        self.committed
    }

    /// Returns the exact authenticated identities accepted at construction in period order.
    pub fn retained_identities(&self) -> Vec<ConcreteStateIdentity> {
        self.retained.values().copied().collect()
    }

    /// Reads one account at an exact retained identity.
    pub fn account_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        let pinned = self.pinned(identity)?;
        read_account(&pinned, address)
    }

    /// Reads one selected physical storage value at an exact retained identity.
    ///
    /// As with [`ConcreteStateRead::storage`], callers that need logical membership must
    /// separately call [`Self::verify_storage_path_at`].
    pub fn storage_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let pinned = self.pinned(identity)?;
        read_storage(&pinned, address, key)
    }

    /// Authenticates one logical storage path at an exact retained identity.
    pub fn verify_storage_path_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        let pinned = self.pinned(identity)?;
        verify_storage_path(&pinned, address, key)
    }

    /// Reads immutable code while requiring an exact retained identity selection.
    ///
    /// Code rows are unversioned; the identity requirement prevents a caller from using
    /// this handle as an unbound database reader. Semantic reachability still comes from
    /// an account read at the same identity.
    pub fn code_at(
        &self,
        identity: ConcreteStateIdentity,
        code_hash: [u8; 32],
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.pinned(identity)?;
        read_code(&self.db, identity, code_hash)
    }

    /// Returns every authenticated live storage leaf for one account and exact identity.
    ///
    /// Account absence and tombstones remain distinct in [`ConcreteRead`]. Only complete
    /// inventories are returned; reaching a resource ceiling or missing any referenced node
    /// or selected value is an error.
    pub fn storage_inventory_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
        limits: ConcreteStorageInventoryLimits,
    ) -> Result<ConcreteRead<ConcreteStorageInventory>, ConcreteStorageInventoryError> {
        let pinned = self
            .pinned(identity)
            .map_err(ConcreteStorageInventoryError::Read)?;
        storage_inventory(&pinned, address, limits)
    }

    fn pinned(
        &self,
        identity: ConcreteStateIdentity,
    ) -> Result<PinnedDatabase<'_>, ConcreteReadError> {
        if self.retained.get(&identity.period) != Some(&identity) {
            return Err(ConcreteReadError::HistoryUnavailable(identity));
        }
        Ok(PinnedDatabase {
            db: &self.db,
            identity,
        })
    }
}

fn storage_inventory<S: PhysicalTrieStore>(
    store: &S,
    address: [u8; 20],
    limits: ConcreteStorageInventoryLimits,
) -> Result<ConcreteRead<ConcreteStorageInventory>, ConcreteStorageInventoryError> {
    let record = match read_account(store, address).map_err(ConcreteStorageInventoryError::Read)? {
        ConcreteRead::Present(record) => record,
        ConcreteRead::Absent => return Ok(ConcreteRead::Absent),
        ConcreteRead::Tombstone => return Ok(ConcreteRead::Tombstone),
    };
    let Some(root) = record.account.storage_root else {
        return Ok(ConcreteRead::Present(ConcreteStorageInventory {
            identity: store.identity(),
            address,
            storage_root: None,
            nodes_visited: 0,
            value_bytes: 0,
            entries: Vec::new(),
        }));
    };
    let inventory = inventory_storage_trie(
        store,
        root,
        |leaf_path| storage_prefix_for_path(address, leaf_path),
        InventoryLimits {
            max_nodes: limits.max_nodes,
            max_leaves: limits.max_leaves,
            max_value_bytes: limits.max_value_bytes,
        },
    )
    .map_err(map_inventory_error)?;
    Ok(ConcreteRead::Present(ConcreteStorageInventory {
        identity: store.identity(),
        address,
        storage_root: Some(root),
        nodes_visited: inventory.nodes_visited,
        value_bytes: inventory.value_bytes,
        entries: inventory
            .entries
            .into_iter()
            .map(|(hashed_path, value)| ConcreteStorageInventoryEntry { hashed_path, value })
            .collect(),
    }))
}

fn map_inventory_error(error: InventoryError) -> ConcreteStorageInventoryError {
    match error {
        InventoryError::Read(error) => ConcreteStorageInventoryError::Read(error),
        InventoryError::LimitExceeded {
            resource,
            limit,
            required,
        } => ConcreteStorageInventoryError::LimitExceeded {
            resource: match resource {
                InventoryResource::Nodes => ConcreteStorageInventoryResource::Nodes,
                InventoryResource::Leaves => ConcreteStorageInventoryResource::Leaves,
                InventoryResource::ValueBytes => ConcreteStorageInventoryResource::ValueBytes,
            },
            limit,
            required,
        },
    }
}

struct PinnedDatabase<'a> {
    db: &'a DB,
    identity: ConcreteStateIdentity,
}

impl PhysicalTrieStore for PinnedDatabase<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        let handle = self.db.cf_handle(column).ok_or_else(|| {
            ConcreteReadError::Corrupt(format!("node column family {column:?} is missing"))
        })?;
        self.db.get_cf(&handle, hash).map_err(io)
    }

    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        selected_version(self.db, column, prefix, period)
    }
}

fn read_account<S: PhysicalTrieStore>(
    store: &S,
    address: [u8; 20],
) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
    let prefix = account_version_prefix(address);
    let selected = store.value("3", prefix, store.identity().period)?;
    let proof = verify_path(
        store,
        store.identity().state_root,
        prefix,
        "2",
        "3",
        |path| path,
        TrieSchema::Account,
    )?;
    reconcile(proof, selected, |bytes| decode_physical_account(&bytes))
}

fn read_storage<S: PhysicalTrieStore>(
    store: &S,
    address: [u8; 20],
    key: ConcreteStorageKey,
) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
    let selected = store.value(
        "5",
        storage_version_prefix(address, key),
        store.identity().period,
    )?;
    let Some(selected) = selected else {
        return Err(ConcreteReadError::HistoryUnavailable(store.identity()));
    };
    if selected.value.is_empty() {
        Ok(ConcreteRead::Tombstone)
    } else {
        Ok(ConcreteRead::Present(selected.value))
    }
}

fn verify_storage_path<S: PhysicalTrieStore>(
    store: &S,
    address: [u8; 20],
    key: ConcreteStorageKey,
) -> Result<ConcreteStoragePath, ConcreteReadError> {
    let root = match read_account(store, address)? {
        ConcreteRead::Present(record) => record.account.storage_root,
        ConcreteRead::Absent | ConcreteRead::Tombstone => {
            return Ok(ConcreteStoragePath::NonMember);
        }
    };
    let Some(root) = root else {
        return Ok(ConcreteStoragePath::NonMember);
    };
    let path = storage_trie_path(key);
    verify_path(
        store,
        root,
        path,
        "4",
        "5",
        |leaf_path| storage_prefix_for_path(address, leaf_path),
        TrieSchema::Storage,
    )
    .map(|proof| match proof {
        PathProof::Member(value) => ConcreteStoragePath::Member(value),
        PathProof::NonMember => ConcreteStoragePath::NonMember,
    })
}

fn reconcile<T>(
    proof: PathProof,
    selected: Option<SelectedVersion>,
    decode: impl FnOnce(Vec<u8>) -> Result<T, ConcreteReadError>,
) -> Result<ConcreteRead<T>, ConcreteReadError> {
    match (proof, selected) {
        (PathProof::Member(proved), Some(selected)) if selected.value == proved => {
            if proved.is_empty() {
                Err(ConcreteReadError::Corrupt(
                    "trie membership selected an empty version".into(),
                ))
            } else {
                decode(proved).map(ConcreteRead::Present)
            }
        }
        (PathProof::Member(_), _) => Err(ConcreteReadError::Corrupt(
            "trie member and selected physical version differ".into(),
        )),
        (PathProof::NonMember, Some(selected)) if selected.value.is_empty() => {
            Ok(ConcreteRead::Tombstone)
        }
        (PathProof::NonMember, Some(_)) => Err(ConcreteReadError::Corrupt(
            "trie non-membership conflicts with a live physical version".into(),
        )),
        (PathProof::NonMember, None) => Ok(ConcreteRead::Absent),
    }
}

fn read_code(
    db: &DB,
    identity: ConcreteStateIdentity,
    code_hash: [u8; 32],
) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
    let handle = db
        .cf_handle("1")
        .ok_or_else(|| ConcreteReadError::Corrupt("code column family is missing".into()))?;
    let Some(code) = db.get_cf(&handle, code_hash).map_err(io)? else {
        return Err(ConcreteReadError::HistoryUnavailable(identity));
    };
    if keccak256(&code) != code_hash {
        return Err(ConcreteReadError::Corrupt(
            "code bytes do not match their Keccak-256 key".into(),
        ));
    }
    Ok(ConcreteRead::Present(code))
}

fn open_database_read_only(path: &Path) -> Result<DB, ConcreteReadError> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    let columns = DB::list_cf(&options, path).map_err(io)?;
    let present = columns.iter().map(String::as_str).collect::<BTreeSet<_>>();
    for required in REQUIRED_COLUMNS {
        if !present.contains(required) {
            return Err(ConcreteReadError::Corrupt(format!(
                "concrete state column family {required:?} is missing"
            )));
        }
    }
    let descriptors = columns
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
    DB::open_cf_descriptors_read_only(&options, path, descriptors, false).map_err(io)
}

fn current_identity(db: &DB) -> Result<ConcreteStateIdentity, ConcreteReadError> {
    let descriptor = db
        .get(DESCRIPTOR_KEY)
        .map_err(io)?
        .ok_or_else(|| ConcreteReadError::Corrupt("concrete descriptor is missing".into()))?;
    decode_descriptor(&descriptor)
}

impl ConcreteStateRead for ConcreteStateReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        read_account(self, address)
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        read_storage(self, address, key)
    }

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        read_code(&self.db, self.identity, code_hash)
    }
}

impl PhysicalTrieStore for ConcreteStateReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        let handle = self.db.cf_handle(column).ok_or_else(|| {
            ConcreteReadError::Corrupt(format!("node column family {column:?} is missing"))
        })?;
        self.db.get_cf(&handle, hash).map_err(io)
    }

    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        selected_version(&self.db, column, prefix, period)
    }
}

fn selected_version(
    db: &DB,
    column: &str,
    prefix: [u8; 32],
    period: FinalChainBlockNumber,
) -> Result<Option<SelectedVersion>, ConcreteReadError> {
    let handle = db.cf_handle(column).ok_or_else(|| {
        ConcreteReadError::Corrupt(format!("value column family {column:?} is missing"))
    })?;
    let target = versioned_key(prefix, period);
    let mut iterator = db.iterator_cf(&handle, IteratorMode::From(&target, Direction::Reverse));
    let Some(entry) = iterator.next() else {
        return Ok(None);
    };
    let (key, value) = entry.map_err(io)?;
    if key.len() != 40 {
        if key.starts_with(&prefix) {
            return Err(ConcreteReadError::Corrupt(format!(
                "versioned value key in column {column:?} is not 40 bytes"
            )));
        }
        return Ok(None);
    }
    if key[..32] != prefix {
        return Ok(None);
    }
    Ok(Some(SelectedVersion {
        value: value.to_vec(),
    }))
}

fn io(error: impl std::fmt::Display) -> ConcreteReadError {
    ConcreteReadError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rocksdb::{ColumnFamilyDescriptor, Options};
    use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStateRead};

    use super::super::codec::empty_trie_root;
    use super::*;
    use crate::{account_commitment_rlp, decode_physical_account};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDb {
        path: PathBuf,
        db: Option<DB>,
    }

    impl TestDb {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustaxa-concrete-reader-{}-{}",
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
            Self { path, db: Some(db) }
        }

        fn put(&self, column: &str, key: &[u8], value: &[u8]) {
            let db = self.db.as_ref().unwrap();
            let handle = db.cf_handle(column).unwrap();
            db.put_cf(&handle, key, value).unwrap();
        }

        fn put_descriptor(&self, identity: ConcreteStateIdentity) {
            let mut stream = rlp::RlpStream::new_list(2);
            stream.append(&identity.period.as_u64());
            stream.append(&identity.state_root.as_slice());
            self.db
                .as_ref()
                .unwrap()
                .put(DESCRIPTOR_KEY, stream.out())
                .unwrap();
        }

        fn close(&mut self) {
            self.db.take();
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            self.db.take();
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }

    #[test]
    fn reads_verified_account_storage_and_code_without_narrowing() {
        let period = FinalChainBlockNumber::new(17);
        let address = [0x42; 20];
        let slot = ConcreteStorageKey([0x24; 32]);
        let storage_value = vec![0, 4, 9, 16, 25, 36, 49, 64, 81];
        let storage_path = storage_trie_path(slot);
        let (storage_root, storage_node) = physical_leaf(storage_path, &storage_value, false);
        let code = vec![0x60, 0, 0x60, 1, 1];
        let code_hash = keccak256(&code);
        let physical_account_bytes = physical_account(
            &[1; 40],
            &[2; 48],
            Some(storage_root),
            Some(code_hash),
            code.len() as u64,
        );
        let account = decode_physical_account(&physical_account_bytes).unwrap();
        let account_path = account_version_prefix(address);
        let (state_root, account_node) = physical_leaf(
            account_path,
            &account_commitment_rlp(&account).unwrap(),
            true,
        );
        let identity = ConcreteStateIdentity { period, state_root };
        let prior_period = FinalChainBlockNumber::new(16);
        let prior_physical_account = physical_account(
            &[1; 40],
            &[3; 48],
            Some(storage_root),
            Some(code_hash),
            code.len() as u64,
        );
        let prior_account = decode_physical_account(&prior_physical_account).unwrap();
        let (prior_root, prior_account_node) = physical_leaf(
            account_path,
            &account_commitment_rlp(&prior_account).unwrap(),
            true,
        );
        let prior_identity = ConcreteStateIdentity {
            period: prior_period,
            state_root: prior_root,
        };

        let mut database = TestDb::new();
        database.put_descriptor(identity);
        database.put("2", &state_root, &account_node);
        database.put("2", &prior_root, &prior_account_node);
        database.put(
            "3",
            &versioned_key(account_path, prior_period),
            &prior_physical_account,
        );
        database.put(
            "3",
            &versioned_key(account_path, period),
            &physical_account_bytes,
        );
        database.put("4", &storage_root, &storage_node);
        database.put(
            "5",
            &versioned_key(storage_version_prefix(address, slot), prior_period),
            &storage_value,
        );
        database.put(
            "5",
            &versioned_key(storage_version_prefix(address, slot), period),
            &storage_value,
        );
        let orphan_slot = ConcreteStorageKey([0x99; 32]);
        database.put(
            "5",
            &versioned_key(storage_version_prefix(address, orphan_slot), period),
            &[0xde, 0xad],
        );
        database.put("1", &code_hash, &code);
        database.close();

        let reader = ConcreteStateReader::open_read_only(&database.path, identity).unwrap();
        let ConcreteRead::Present(read_account) = reader.account(address).unwrap() else {
            panic!("account was not present")
        };
        assert_eq!(read_account.physical_rlp, physical_account_bytes);
        assert_eq!(read_account.account.nonce.to_bytes(), vec![1; 40]);
        assert_eq!(
            read_account.account.balance.value().to_bytes_be(),
            vec![2; 48]
        );
        assert_eq!(
            reader.storage(address, slot).unwrap(),
            ConcreteRead::Present(storage_value.clone())
        );
        assert_eq!(
            reader.verify_storage_path(address, slot).unwrap(),
            ConcreteStoragePath::Member(vec![0, 4, 9, 16, 25, 36, 49, 64, 81])
        );
        assert_eq!(
            reader.storage(address, orphan_slot).unwrap(),
            ConcreteRead::Present(vec![0xde, 0xad])
        );
        assert_eq!(
            reader.verify_storage_path(address, orphan_slot).unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert_eq!(reader.code(code_hash).unwrap(), ConcreteRead::Present(code));

        let historical = ConcreteStateReader::open_historical_read_only(
            &database.path,
            identity,
            prior_identity,
        )
        .unwrap();
        let ConcreteRead::Present(prior) = historical.account(address).unwrap() else {
            panic!("prior account was not present")
        };
        assert_eq!(prior.physical_rlp, prior_physical_account);
        assert_eq!(prior.account.balance.value().to_bytes_be(), vec![3; 48]);
        assert_eq!(
            historical.verify_storage_path(address, slot).unwrap(),
            ConcreteStoragePath::Member(storage_value)
        );
    }

    #[test]
    fn checkpoint_readers_pin_exact_current_and_historical_identities() {
        let address = [0x42; 20];
        let account_path = account_version_prefix(address);
        let prior_period = FinalChainBlockNumber::new(16);
        let current_period = FinalChainBlockNumber::new(17);
        let code = vec![0x60, 0, 0x60, 1, 1];
        let code_hash = keccak256(&code);
        let prior_bytes = physical_account(&[1], &[2], None, Some(code_hash), code.len() as u64);
        let current_bytes = physical_account(&[2], &[3], None, Some(code_hash), code.len() as u64);
        let prior_account = decode_physical_account(&prior_bytes).unwrap();
        let current_account = decode_physical_account(&current_bytes).unwrap();
        let (prior_root, prior_node) = physical_leaf(
            account_path,
            &account_commitment_rlp(&prior_account).unwrap(),
            true,
        );
        let (current_root, current_node) = physical_leaf(
            account_path,
            &account_commitment_rlp(&current_account).unwrap(),
            true,
        );
        let prior = ConcreteStateIdentity {
            period: prior_period,
            state_root: prior_root,
        };
        let current = ConcreteStateIdentity {
            period: current_period,
            state_root: current_root,
        };
        let mut database = TestDb::new();
        database.put_descriptor(current);
        database.put("2", &prior_root, &prior_node);
        database.put("2", &current_root, &current_node);
        database.put(
            "3",
            &versioned_key(account_path, prior_period),
            &prior_bytes,
        );
        database.put(
            "3",
            &versioned_key(account_path, current_period),
            &current_bytes,
        );
        database.put("1", &code_hash, &code);
        database.close();

        let readers =
            ConcreteCheckpointReaders::open_read_only(&database.path, current, [current, prior])
                .unwrap();
        assert_eq!(readers.committed_identity(), current);
        assert_eq!(readers.retained_identities(), vec![prior, current]);
        let ConcreteRead::Present(prior_read) = readers.account_at(prior, address).unwrap() else {
            panic!("prior account was not present")
        };
        let ConcreteRead::Present(current_read) = readers.account_at(current, address).unwrap()
        else {
            panic!("current account was not present")
        };
        assert_eq!(prior_read.physical_rlp, prior_bytes);
        assert_eq!(current_read.physical_rlp, current_bytes);
        assert_eq!(
            readers.code_at(prior, code_hash).unwrap(),
            ConcreteRead::Present(code)
        );

        let unlisted = ConcreteStateIdentity {
            period: prior_period,
            state_root: [0x77; 32],
        };
        let slot = ConcreteStorageKey([9; 32]);
        assert_eq!(
            readers.account_at(unlisted, address),
            Err(ConcreteReadError::HistoryUnavailable(unlisted))
        );
        assert_eq!(
            readers.storage_at(unlisted, address, slot),
            Err(ConcreteReadError::HistoryUnavailable(unlisted))
        );
        assert_eq!(
            readers.verify_storage_path_at(unlisted, address, slot),
            Err(ConcreteReadError::HistoryUnavailable(unlisted))
        );
        assert_eq!(
            readers.code_at(unlisted, code_hash),
            Err(ConcreteReadError::HistoryUnavailable(unlisted))
        );
        assert_eq!(
            readers.code_at(prior, [0x55; 32]),
            Err(ConcreteReadError::HistoryUnavailable(prior))
        );
    }

    #[test]
    fn checkpoint_readers_reject_ambiguous_or_unverified_identity_sets() {
        let prior = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(8),
            state_root: empty_trie_root(),
        };
        let current = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(9),
            state_root: empty_trie_root(),
        };
        let future = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(10),
            state_root: empty_trie_root(),
        };
        let inaccessible = ConcreteStateIdentity {
            period: prior.period,
            state_root: [0x99; 32],
        };
        let wrong_current = ConcreteStateIdentity {
            period: current.period,
            state_root: [0x88; 32],
        };
        let mut database = TestDb::new();
        database.put_descriptor(current);
        database.close();

        assert!(matches!(
            ConcreteCheckpointReaders::open_read_only(&database.path, current, [prior]),
            Err(ConcreteReadError::Corrupt(_))
        ));
        assert!(matches!(
            ConcreteCheckpointReaders::open_read_only(
                &database.path,
                current,
                [current, prior, prior]
            ),
            Err(ConcreteReadError::Corrupt(_))
        ));
        assert!(matches!(
            ConcreteCheckpointReaders::open_read_only(
                &database.path,
                current,
                [current, prior, inaccessible]
            ),
            Err(ConcreteReadError::Corrupt(_))
        ));
        assert!(matches!(
            ConcreteCheckpointReaders::open_read_only(&database.path, current, [current, future]),
            Err(ConcreteReadError::FuturePeriod { .. })
        ));
        assert!(matches!(
            ConcreteCheckpointReaders::open_read_only(
                &database.path,
                wrong_current,
                [wrong_current]
            ),
            Err(ConcreteReadError::IdentityMismatch { .. })
        ));
        assert_eq!(
            ConcreteCheckpointReaders::open_read_only(
                &database.path,
                current,
                [current, inaccessible]
            )
            .err(),
            Some(ConcreteReadError::HistoryUnavailable(inaccessible))
        );
    }

    #[test]
    fn checkpoint_inventory_returns_only_complete_authenticated_live_leaves() {
        let period = FinalChainBlockNumber::new(17);
        let address = [0x42; 20];
        let slot = ConcreteStorageKey([0x24; 32]);
        let storage_value = vec![1, 3, 3, 7];
        let hashed_path = storage_trie_path(slot);
        let (storage_root, storage_node) = physical_leaf(hashed_path, &storage_value, false);
        let account_bytes = physical_account(&[1], &[2], Some(storage_root), None, 0);
        let account = decode_physical_account(&account_bytes).unwrap();
        let account_path = account_version_prefix(address);
        let (state_root, account_node) = physical_leaf(
            account_path,
            &account_commitment_rlp(&account).unwrap(),
            true,
        );
        let identity = ConcreteStateIdentity { period, state_root };
        let mut database = TestDb::new();
        database.put_descriptor(identity);
        database.put("2", &state_root, &account_node);
        database.put("3", &versioned_key(account_path, period), &account_bytes);
        database.put("4", &storage_root, &storage_node);
        database.put(
            "5",
            &versioned_key(storage_version_prefix(address, slot), period),
            &storage_value,
        );
        database.close();

        let readers =
            ConcreteCheckpointReaders::open_read_only(&database.path, identity, [identity])
                .unwrap();
        let limits = ConcreteStorageInventoryLimits {
            max_nodes: 1,
            max_leaves: 1,
            max_value_bytes: storage_value.len() as u64,
        };
        assert_eq!(
            readers.storage_inventory_at(identity, address, limits),
            Ok(ConcreteRead::Present(ConcreteStorageInventory {
                identity,
                address,
                storage_root: Some(storage_root),
                nodes_visited: 1,
                value_bytes: storage_value.len() as u64,
                entries: vec![ConcreteStorageInventoryEntry {
                    hashed_path,
                    value: storage_value,
                }],
            }))
        );
        assert_eq!(
            readers.storage_inventory_at(
                identity,
                address,
                ConcreteStorageInventoryLimits {
                    max_nodes: 0,
                    ..limits
                },
            ),
            Err(ConcreteStorageInventoryError::LimitExceeded {
                resource: ConcreteStorageInventoryResource::Nodes,
                limit: 0,
                required: 1,
            })
        );
        let unlisted = ConcreteStateIdentity {
            period,
            state_root: [0x55; 32],
        };
        assert_eq!(
            readers.storage_inventory_at(unlisted, address, limits),
            Err(ConcreteStorageInventoryError::Read(
                ConcreteReadError::HistoryUnavailable(unlisted)
            ))
        );
        assert_eq!(
            readers.storage_inventory_at(identity, [0x99; 20], limits),
            Ok(ConcreteRead::Absent)
        );
    }

    #[test]
    fn proves_absence_preserves_tombstones_and_rejects_unavailable_code() {
        let identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(9),
            state_root: empty_trie_root(),
        };
        let tombstoned = [0x33; 20];
        let missing = [0x55; 20];
        let mut database = TestDb::new();
        database.put_descriptor(identity);
        database.put(
            "3",
            &versioned_key(account_version_prefix(tombstoned), identity.period),
            &[],
        );
        let orphan_slot = ConcreteStorageKey([9; 32]);
        database.put(
            "5",
            &versioned_key(
                storage_version_prefix(tombstoned, orphan_slot),
                identity.period,
            ),
            &[1, 3, 3, 7],
        );
        database.close();

        let reader = ConcreteStateReader::open_read_only(&database.path, identity).unwrap();
        assert_eq!(reader.account(tombstoned).unwrap(), ConcreteRead::Tombstone);
        assert_eq!(reader.account(missing).unwrap(), ConcreteRead::Absent);
        assert_eq!(
            reader.storage(tombstoned, orphan_slot).unwrap(),
            ConcreteRead::Present(vec![1, 3, 3, 7])
        );
        assert_eq!(
            reader.storage(missing, orphan_slot),
            Err(ConcreteReadError::HistoryUnavailable(identity))
        );
        assert_eq!(
            reader.code([7; 32]),
            Err(ConcreteReadError::HistoryUnavailable(identity))
        );
    }

    #[test]
    fn rejects_identity_node_and_code_corruption() {
        let period = FinalChainBlockNumber::new(4);
        let address = [0x77; 20];
        let physical_account = physical_account(&[1], &[2], None, None, 0);
        let account = decode_physical_account(&physical_account).unwrap();
        let path = account_version_prefix(address);
        let (root, mut node) =
            physical_leaf(path, &account_commitment_rlp(&account).unwrap(), true);
        *node.last_mut().unwrap() ^= 1;
        let identity = ConcreteStateIdentity {
            period,
            state_root: root,
        };
        let bad_code_hash = [0x88; 32];
        let mut database = TestDb::new();
        database.put_descriptor(identity);
        database.put("2", &root, &node);
        database.put("3", &versioned_key(path, period), &physical_account);
        database.put("1", &bad_code_hash, &[1, 2, 3]);
        let available_orphan = ConcreteStorageKey([0xaa; 32]);
        database.put(
            "5",
            &versioned_key(storage_version_prefix(address, available_orphan), period),
            &[4, 2],
        );
        database.close();

        let reader = ConcreteStateReader::open_read_only(&database.path, identity).unwrap();
        assert!(matches!(
            reader.account(address),
            Err(ConcreteReadError::Corrupt(_))
        ));
        assert_eq!(
            reader.storage(address, available_orphan).unwrap(),
            ConcreteRead::Present(vec![4, 2])
        );
        assert!(matches!(
            reader.code(bad_code_hash),
            Err(ConcreteReadError::Corrupt(_))
        ));
        let expected_future = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(5),
            state_root: root,
        };
        assert!(matches!(
            ConcreteStateReader::open_read_only(&database.path, expected_future),
            Err(ConcreteReadError::FuturePeriod { .. })
        ));
    }

    #[test]
    fn rejects_malformed_version_keys_for_the_requested_prefix() {
        let identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(9),
            state_root: empty_trie_root(),
        };
        let address = [0x12; 20];
        let mut malformed = account_version_prefix(address).to_vec();
        malformed.extend_from_slice(&[0; 7]);
        let mut database = TestDb::new();
        database.put_descriptor(identity);
        database.put("3", &malformed, &[1]);
        database.close();
        let reader = ConcreteStateReader::open_read_only(&database.path, identity).unwrap();
        assert!(matches!(
            reader.account(address),
            Err(ConcreteReadError::Corrupt(_))
        ));
    }

    #[test]
    fn key_codecs_match_qualified_mainnet_fixture() {
        let mut address = [0_u8; 20];
        address[19] = 0xfe;
        assert_eq!(
            format!(
                "{:x}",
                ethereum_types::H256(account_version_prefix(address))
            ),
            "d24e8fff20c5317074c54fd54ca9f1fc8fef36bb70c44b55f90a70c621b91f9a"
        );
        let logical_key = ConcreteStorageKey(keccak256(&[7]));
        assert_eq!(
            format!(
                "{:x}",
                ethereum_types::H256(storage_version_prefix(address, logical_key))
            ),
            "eb3d31d3bc4e19e9f34879af5649be903e5b2b1136e2df2dd349c560a7704b1b"
        );
    }

    #[test]
    fn account_commitment_preserves_physical_integers_and_normalizes_only_empty_hashes() {
        let physical = physical_account(&[], &[], None, None, 9);
        let record = decode_physical_account(&physical).unwrap();
        assert_eq!(
            account_commitment_rlp(&record).unwrap(),
            decode_hex(
                "f8448080a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
            )
        );

        let legacy = physical_account(&[1], &[0, 2], Some([3; 32]), Some([4; 32]), 0);
        let record = decode_physical_account(&legacy).unwrap();
        let commitment = account_commitment_rlp(&record).unwrap();
        assert_eq!(
            rlp::Rlp::new(&commitment).at(1).unwrap().data().unwrap(),
            [0, 2]
        );
    }

    #[test]
    #[ignore = "requires the independently copied qualified mainnet snapshot"]
    fn qualified_snapshot_fixture_has_verified_paths_and_bytes() {
        let path = std::env::var_os("RUSTAXA_QUALIFIED_STATE_DB")
            .expect("RUSTAXA_QUALIFIED_STATE_DB must name the copied state_db");
        let identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(25_706_949),
            state_root: decode_hex_32(
                "b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227",
            ),
        };
        let mut address = [0_u8; 20];
        address[19] = 0xfe;
        let reader = ConcreteStateReader::open_read_only(Path::new(&path), identity).unwrap();
        let ConcreteRead::Present(account) = reader.account(address).unwrap() else {
            panic!("qualified DPoS account was not present")
        };
        assert_eq!(
            account.physical_rlp,
            decode_hex(
                "f853018c096636510acf94f4f01e9072a0c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504a07c17ca5f75fbd5264551595c4d8c98ad09464ce93fc772cb521b9dbb05e3e447820bb8"
            )
        );
        let code_hash = account.account.code_hash.unwrap();
        let ConcreteRead::Present(code) = reader.code(code_hash).unwrap() else {
            panic!("qualified DPoS code was not present")
        };
        assert_eq!(code.len(), 3_000);
        assert_eq!(keccak256(&code), code_hash);
        let total_supply = ConcreteStorageKey(keccak256(&[7]));
        assert_eq!(
            reader.storage(address, total_supply).unwrap(),
            ConcreteRead::Present(decode_hex("237465dd4fbad4693966174c"))
        );
        assert_eq!(
            reader.verify_storage_path(address, total_supply).unwrap(),
            ConcreteStoragePath::Member(decode_hex("237465dd4fbad4693966174c"))
        );

        let prior_identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(25_706_948),
            state_root: decode_hex_32(
                "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2",
            ),
        };
        let prior = ConcreteStateReader::open_historical_read_only(
            Path::new(&path),
            identity,
            prior_identity,
        )
        .unwrap();
        assert_eq!(ConcreteStateRead::identity(&prior), prior_identity);
        let ConcreteRead::Present(prior_account) = prior.account(address).unwrap() else {
            panic!("qualified prior DPoS account was not present")
        };
        assert_eq!(prior_account.physical_rlp, account.physical_rlp);
        assert_eq!(
            prior.storage(address, total_supply).unwrap(),
            ConcreteRead::Present(decode_hex("237465dd4fbad4693966174c"))
        );
        assert_eq!(
            prior.verify_storage_path(address, total_supply).unwrap(),
            ConcreteStoragePath::Member(decode_hex("237465dd4fbad4693966174c"))
        );

        let checkpoints = ConcreteCheckpointReaders::open_read_only(
            Path::new(&path),
            identity,
            [prior_identity, identity],
        )
        .unwrap();
        let ConcreteRead::Present(inventory) = checkpoints
            .storage_inventory_at(
                identity,
                address,
                ConcreteStorageInventoryLimits {
                    max_nodes: 50_000,
                    max_leaves: 50_000,
                    max_value_bytes: 32 * 1024 * 1024,
                },
            )
            .unwrap()
        else {
            panic!("qualified DPoS account was not present for inventory")
        };
        assert_eq!(inventory.identity, identity);
        assert_eq!(inventory.address, address);
        assert_eq!(inventory.storage_root, account.account.storage_root);
        assert_eq!(inventory.nodes_visited, 31_164);
        assert_eq!(inventory.entries.len(), 23_278);
        assert_eq!(inventory.value_bytes, 259_077);
        let mut inventory_encoding = Vec::new();
        for entry in &inventory.entries {
            inventory_encoding.extend_from_slice(&entry.hashed_path);
            inventory_encoding.extend_from_slice(&(entry.value.len() as u64).to_be_bytes());
            inventory_encoding.extend_from_slice(&entry.value);
        }
        let inventory_digest = keccak256(&inventory_encoding);
        assert_eq!(
            inventory_digest,
            decode_hex_32("8183fbc4a15113b0b6b85ce5603e2ea449fc43b12bb531bdb48a9596776bd93f")
        );
        assert!(inventory.entries.iter().any(|entry| {
            entry.hashed_path == storage_trie_path(total_supply)
                && entry.value == decode_hex("237465dd4fbad4693966174c")
        }));
        let minted_tokens = ConcreteStorageKey(keccak256(&[6]));
        let yield_key = ConcreteStorageKey(keccak256(&[8]));
        assert_eq!(
            checkpoints
                .storage_at(identity, address, minted_tokens)
                .unwrap(),
            ConcreteRead::Tombstone
        );
        assert_eq!(
            checkpoints
                .verify_storage_path_at(identity, address, minted_tokens)
                .unwrap(),
            ConcreteStoragePath::NonMember
        );
        assert!(
            !inventory
                .entries
                .iter()
                .any(|entry| entry.hashed_path == storage_trie_path(minted_tokens))
        );
        assert_eq!(
            checkpoints
                .storage_at(identity, address, yield_key)
                .unwrap(),
            ConcreteRead::Present(decode_hex("83016db8"))
        );
        assert!(inventory.entries.iter().any(|entry| {
            entry.hashed_path == storage_trie_path(yield_key)
                && entry.value == decode_hex("83016db8")
        }));
        eprintln!(
            "qualified DPoS live inventory: nodes={}, leaves={}, value_bytes={}, digest={:x}, minted_path={:x}, total_supply_path={:x}, yield_path={:x}",
            inventory.nodes_visited,
            inventory.entries.len(),
            inventory.value_bytes,
            ethereum_types::H256(inventory_digest),
            ethereum_types::H256(storage_trie_path(minted_tokens)),
            ethereum_types::H256(storage_trie_path(total_supply)),
            ethereum_types::H256(storage_trie_path(yield_key)),
        );
    }

    fn physical_account(
        nonce: &[u8],
        balance: &[u8],
        storage_root: Option<[u8; 32]>,
        code_hash: Option<[u8; 32]>,
        code_size: u64,
    ) -> Vec<u8> {
        let mut stream = rlp::RlpStream::new_list(5);
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
        stream.out().to_vec()
    }

    fn physical_leaf(
        path: [u8; 32],
        value_hash_encoding: &[u8],
        account: bool,
    ) -> ([u8; 32], Vec<u8>) {
        let mut compact = Vec::with_capacity(33);
        compact.push(0x20);
        compact.extend_from_slice(&path);
        let commitment = if account {
            value_hash_encoding.to_vec()
        } else {
            rlp::encode(&value_hash_encoding).to_vec()
        };
        let mut canonical = rlp::RlpStream::new_list(2);
        canonical.append(&compact);
        canonical.append(&commitment);
        let root = keccak256(&canonical.out());
        let mut physical = rlp::RlpStream::new_list(2);
        physical.append(&compact);
        physical.append(&root.as_slice());
        (root, physical.out().to_vec())
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        assert_eq!(input.len() % 2, 0);
        (0..input.len())
            .step_by(2)
            .map(|index| {
                let digit = |value: u8| match value {
                    b'0'..=b'9' => value - b'0',
                    b'a'..=b'f' => value - b'a' + 10,
                    _ => panic!("invalid fixture hex"),
                };
                digit(input.as_bytes()[index]) << 4 | digit(input.as_bytes()[index + 1])
            })
            .collect()
    }

    fn decode_hex_32(input: &str) -> [u8; 32] {
        decode_hex(input).try_into().unwrap()
    }
}
