//! Owns one checked RocksDB snapshot for concurrent concrete-state reads.

use std::ptr::NonNull;
use std::sync::Arc;

use librocksdb_sys::{rocksdb_column_family_handle_t, rocksdb_iterator_t, rocksdb_snapshot_t};
use rocksdb::{
    AsColumnFamilyRef, DB, DBAccess, DBIteratorWithThreadMode, DBPinnableSlice, Error,
    IteratorMode, ReadOptions, SnapshotWithThreadMode,
};
use rustaxa_types::concrete_state::ConcreteReadError;

/// Pins one database sequence and keeps the database open until release.
pub(super) struct OwnedSnapshot {
    db: Arc<DB>,
    snapshot: NonNull<rocksdb_snapshot_t>,
}

impl OwnedSnapshot {
    /// Captures the current sequence of a writable database. Returns an I/O error
    /// if RocksDB cannot create a snapshot.
    pub(super) fn new(db: Arc<DB>) -> Result<Self, ConcreteReadError> {
        // SAFETY: The Arc keeps the database open until the snapshot is released.
        let snapshot = unsafe { db.create_snapshot() };
        let snapshot = check_snapshot(snapshot)?;
        Ok(Self { db, snapshot })
    }

    /// Returns the database that owns the snapshot and its column families.
    pub(super) fn db(&self) -> &DB {
        &self.db
    }

    /// Reads a default column family key at the pinned sequence.
    pub(super) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        self.db.get_opt(key, &self.read_options())
    }

    /// Reads a key in a column family of this database at the pinned sequence.
    pub(super) fn get_cf(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: impl AsRef<[u8]>,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.db.get_cf_opt(cf, key, &self.read_options())
    }

    /// Iterates a column family of this database at the pinned sequence.
    /// The iterator borrows the snapshot until the iterator is dropped.
    pub(super) fn iterator_cf(
        &self,
        cf: &impl AsColumnFamilyRef,
        mode: IteratorMode<'_>,
    ) -> DBIteratorWithThreadMode<'_, DB> {
        self.db.iterator_cf_opt(cf, self.read_options(), mode)
    }

    fn read_options(&self) -> ReadOptions {
        let mut options = ReadOptions::default();
        let access = SnapshotAccess(self);
        options.set_snapshot(&SnapshotWithThreadMode::new(&access));
        options
    }
}

fn check_snapshot(
    snapshot: *const rocksdb_snapshot_t,
) -> Result<NonNull<rocksdb_snapshot_t>, ConcreteReadError> {
    NonNull::new(snapshot.cast_mut())
        .ok_or_else(|| ConcreteReadError::Io("RocksDB could not create a snapshot".into()))
}

impl Drop for OwnedSnapshot {
    fn drop(&mut self) {
        // SAFETY: This holder owns the snapshot. The Arc still holds its database.
        unsafe { self.db.release_snapshot(self.snapshot.as_ptr()) };
    }
}

// SAFETY: RocksDB permits concurrent reads of an immutable snapshot. Rust
// keeps the database and snapshot alive for each read and iterator.
unsafe impl Send for OwnedSnapshot {}
unsafe impl Sync for OwnedSnapshot {}

/// Lends the checked pointer to RocksDB's public read options API.
/// The temporary snapshot borrows the pointer. The owned snapshot releases it.
struct SnapshotAccess<'a>(&'a OwnedSnapshot);

impl DBAccess for SnapshotAccess<'_> {
    unsafe fn create_snapshot(&self) -> *const rocksdb_snapshot_t {
        self.0.snapshot.as_ptr()
    }

    unsafe fn release_snapshot(&self, _snapshot: *const rocksdb_snapshot_t) {}

    unsafe fn create_iterator(&self, options: &ReadOptions) -> *mut rocksdb_iterator_t {
        // SAFETY: The caller supplies valid read options for this database.
        unsafe { self.0.db.create_iterator(options) }
    }

    unsafe fn create_iterator_cf(
        &self,
        cf: *mut rocksdb_column_family_handle_t,
        options: &ReadOptions,
    ) -> *mut rocksdb_iterator_t {
        // SAFETY: The caller supplies a valid column family and options for this database.
        unsafe { self.0.db.create_iterator_cf(cf, options) }
    }

    fn get_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        options: &ReadOptions,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.0.db.get_opt(key, options)
    }

    fn get_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        options: &ReadOptions,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.0.db.get_cf_opt(cf, key, options)
    }

    fn get_pinned_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        options: &ReadOptions,
    ) -> Result<Option<DBPinnableSlice<'_>>, Error> {
        self.0.db.get_pinned_opt(key, options)
    }

    fn get_pinned_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        options: &ReadOptions,
    ) -> Result<Option<DBPinnableSlice<'_>>, Error> {
        self.0.db.get_pinned_cf_opt(cf, key, options)
    }

    fn multi_get_opt<K, I>(
        &self,
        keys: I,
        options: &ReadOptions,
    ) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = K>,
    {
        self.0.db.multi_get_opt(keys, options)
    }

    fn multi_get_cf_opt<'a, K, I, W>(
        &self,
        keys: I,
        options: &ReadOptions,
    ) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'a W, K)>,
        W: AsColumnFamilyRef + 'a,
    {
        self.0.db.multi_get_cf_opt(keys, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_null_snapshot() {
        assert!(matches!(
            check_snapshot(std::ptr::null()),
            Err(ConcreteReadError::Io(message)) if message == "RocksDB could not create a snapshot"
        ));
    }
}
