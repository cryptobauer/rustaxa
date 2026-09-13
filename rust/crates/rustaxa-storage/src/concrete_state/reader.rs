//! Descriptor-pinned RocksDB implementation of the concrete-state read port.

use std::collections::BTreeSet;
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
    PathProof, PhysicalTrieStore, SelectedVersion, TrieSchema, verify_path,
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

/// Authenticated logical result for one storage path. This diagnostic is
/// separate from [`ConcreteStateRead::storage`], which exposes retained physical
/// history even when a row is orphaned from the current account trie.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteStoragePath {
    Member(Vec<u8>),
    NonMember,
}

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
        let mut options = Options::default();
        options.create_if_missing(false);
        options.create_missing_column_families(false);
        let columns = DB::list_cf(&options, path.as_ref()).map_err(io)?;
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
        let db = DB::open_cf_descriptors_read_only(&options, path.as_ref(), descriptors, false)
            .map_err(io)?;
        let descriptor = db
            .get(DESCRIPTOR_KEY)
            .map_err(io)?
            .ok_or_else(|| ConcreteReadError::Corrupt("concrete descriptor is missing".into()))?;
        let observed = decode_descriptor(&descriptor)?;
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

    fn version(
        &self,
        column: &str,
        prefix: [u8; 32],
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        <Self as PhysicalTrieStore>::value(self, column, prefix, self.identity.period)
    }

    fn reconcile<T>(
        &self,
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

    /// Verifies the logical slot path under the pinned account storage root.
    /// Account or slot non-membership is returned independently of any retained
    /// orphan physical row. Missing nodes or referenced values fail closed.
    pub fn verify_storage_path(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteStoragePath, ConcreteReadError> {
        let root = match self.account(address)? {
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
            self,
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
}

impl ConcreteStateRead for ConcreteStateReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        let prefix = account_version_prefix(address);
        let selected = self.version("3", prefix)?;
        let proof = verify_path(
            self,
            self.identity.state_root,
            prefix,
            "2",
            "3",
            |path| path,
            TrieSchema::Account,
        )?;
        self.reconcile(proof, selected, |bytes| decode_physical_account(&bytes))
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let selected = self.version("5", storage_version_prefix(address, key))?;
        let Some(selected) = selected else {
            return Err(ConcreteReadError::HistoryUnavailable(self.identity));
        };
        if selected.value.is_empty() {
            Ok(ConcreteRead::Tombstone)
        } else {
            Ok(ConcreteRead::Present(selected.value))
        }
    }

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let handle = self
            .db
            .cf_handle("1")
            .ok_or_else(|| ConcreteReadError::Corrupt("code column family is missing".into()))?;
        let Some(code) = self.db.get_cf(&handle, code_hash).map_err(io)? else {
            return Err(ConcreteReadError::HistoryUnavailable(self.identity));
        };
        if keccak256(&code) != code_hash {
            return Err(ConcreteReadError::Corrupt(
                "code bytes do not match their Keccak-256 key".into(),
            ));
        }
        Ok(ConcreteRead::Present(code))
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
        let handle = self.db.cf_handle(column).ok_or_else(|| {
            ConcreteReadError::Corrupt(format!("value column family {column:?} is missing"))
        })?;
        let target = versioned_key(prefix, period);
        let mut iterator = self
            .db
            .iterator_cf(&handle, IteratorMode::From(&target, Direction::Reverse));
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
