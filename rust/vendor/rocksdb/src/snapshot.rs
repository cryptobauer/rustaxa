// Copyright 2020 Tyler Neely
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use crate::{
    db::DBAccess, ffi, AsColumnFamilyRef, DBIteratorWithThreadMode, DBPinnableSlice,
    DBRawIteratorWithThreadMode, Error, IteratorMode, ReadOptions, DB,
};

/// A type alias to keep compatibility. See [`SnapshotWithThreadMode`] for details
pub type Snapshot<'a> = SnapshotWithThreadMode<'a, DB>;

/// A consistent view of the database at the point of creation.
///
/// # Examples
///
/// ```
/// use rocksdb::{DB, IteratorMode, Options};
///
/// let tempdir = tempfile::Builder::new()
///     .prefix("_path_for_rocksdb_storage3")
///     .tempdir()
///     .expect("Failed to create temporary path for the _path_for_rocksdb_storage3");
/// let path = tempdir.path();
/// {
///     let db = DB::open_default(path).unwrap();
///     let snapshot = db.snapshot(); // Creates a longer-term snapshot of the DB, but closed when goes out of scope
///     let mut iter = snapshot.iterator(IteratorMode::Start); // Make as many iterators as you'd like from one snapshot
/// }
/// let _ = DB::destroy(&Options::default(), path);
/// ```
///
pub struct SnapshotWithThreadMode<'a, D: DBAccess> {
    db: &'a D,
    pub(crate) inner: *const ffi::rocksdb_snapshot_t,
}

impl<'a, D: DBAccess> SnapshotWithThreadMode<'a, D> {
    /// Creates a new `SnapshotWithThreadMode` of the database `db`.
    pub fn new(db: &'a D) -> Self {
        let snapshot = unsafe { db.create_snapshot() };
        Self {
            db,
            inner: snapshot,
        }
    }

    /// Creates an iterator over the data in this snapshot, using the default read options.
    pub fn iterator(&self, mode: IteratorMode) -> DBIteratorWithThreadMode<'a, D> {
        let readopts = ReadOptions::default();
        self.iterator_opt(mode, readopts)
    }

    /// Creates an iterator over the data in this snapshot under the given column family, using
    /// the default read options.
    pub fn iterator_cf(
        &self,
        cf_handle: &impl AsColumnFamilyRef,
        mode: IteratorMode,
    ) -> DBIteratorWithThreadMode<D> {
        let readopts = ReadOptions::default();
        self.iterator_cf_opt(cf_handle, readopts, mode)
    }

    /// Creates an iterator over the data in this snapshot, using the given read options.
    pub fn iterator_opt(
        &self,
        mode: IteratorMode,
        mut readopts: ReadOptions,
    ) -> DBIteratorWithThreadMode<'a, D> {
        readopts.set_snapshot(self);
        DBIteratorWithThreadMode::<D>::new(self.db, readopts, mode)
    }

    /// Creates an iterator over the data in this snapshot under the given column family, using
    /// the given read options.
    pub fn iterator_cf_opt(
        &self,
        cf_handle: &impl AsColumnFamilyRef,
        mut readopts: ReadOptions,
        mode: IteratorMode,
    ) -> DBIteratorWithThreadMode<D> {
        readopts.set_snapshot(self);
        DBIteratorWithThreadMode::new_cf(self.db, cf_handle.inner(), readopts, mode)
    }

    /// Creates a raw iterator over the data in this snapshot, using the default read options.
    pub fn raw_iterator(&self) -> DBRawIteratorWithThreadMode<D> {
        let readopts = ReadOptions::default();
        self.raw_iterator_opt(readopts)
    }

    /// Creates a raw iterator over the data in this snapshot under the given column family, using
    /// the default read options.
    pub fn raw_iterator_cf(
        &self,
        cf_handle: &impl AsColumnFamilyRef,
    ) -> DBRawIteratorWithThreadMode<D> {
        let readopts = ReadOptions::default();
        self.raw_iterator_cf_opt(cf_handle, readopts)
    }

    /// Creates a raw iterator over the data in this snapshot, using the given read options.
    pub fn raw_iterator_opt(&self, mut readopts: ReadOptions) -> DBRawIteratorWithThreadMode<D> {
        readopts.set_snapshot(self);
        DBRawIteratorWithThreadMode::new(self.db, readopts)
    }

    /// Creates a raw iterator over the data in this snapshot under the given column family, using
    /// the given read options.
    pub fn raw_iterator_cf_opt(
        &self,
        cf_handle: &impl AsColumnFamilyRef,
        mut readopts: ReadOptions,
    ) -> DBRawIteratorWithThreadMode<D> {
        readopts.set_snapshot(self);
        DBRawIteratorWithThreadMode::new_cf(self.db, cf_handle.inner(), readopts)
    }

    /// Returns the bytes associated with a key value with default read options.
    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Error> {
        let readopts = ReadOptions::default();
        self.get_opt(key, readopts)
    }

    /// Returns the bytes associated with a key value and given column family with default read
    /// options.
    pub fn get_cf<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
    ) -> Result<Option<Vec<u8>>, Error> {
        let readopts = ReadOptions::default();
        self.get_cf_opt(cf, key.as_ref(), readopts)
    }

    /// Returns the bytes associated with a key value and given read options.
    pub fn get_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<Vec<u8>>, Error> {
        readopts.set_snapshot(self);
        self.db.get_opt(key.as_ref(), &readopts)
    }

    /// Returns the bytes associated with a key value, given column family and read options.
    pub fn get_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<Vec<u8>>, Error> {
        readopts.set_snapshot(self);
        self.db.get_cf_opt(cf, key.as_ref(), &readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_opt but
    /// leverages default options.
    pub fn get_pinned<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<DBPinnableSlice>, Error> {
        let readopts = ReadOptions::default();
        self.get_pinned_opt(key, readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_cf_opt but
    /// leverages default options.
    pub fn get_pinned_cf<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
    ) -> Result<Option<DBPinnableSlice>, Error> {
        let readopts = ReadOptions::default();
        self.get_pinned_cf_opt(cf, key.as_ref(), readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy.
    pub fn get_pinned_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<DBPinnableSlice>, Error> {
        readopts.set_snapshot(self);
        self.db.get_pinned_opt(key.as_ref(), &readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_opt but
    /// allows specifying ColumnFamily.
    pub fn get_pinned_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<DBPinnableSlice>, Error> {
        readopts.set_snapshot(self);
        self.db.get_pinned_cf_opt(cf, key.as_ref(), &readopts)
    }

    /// Returns the bytes associated with the given key values and default read options.
    pub fn multi_get<K: AsRef<[u8]>, I>(&self, keys: I) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        I: IntoIterator<Item = K>,
    {
        let readopts = ReadOptions::default();
        self.multi_get_opt(keys, readopts)
    }

    /// Returns the bytes associated with the given key values and default read options.
    pub fn multi_get_cf<'b, K, I, W>(&self, keys_cf: I) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'b W, K)>,
        W: AsColumnFamilyRef + 'b,
    {
        let readopts = ReadOptions::default();
        self.multi_get_cf_opt(keys_cf, readopts)
    }

    /// Returns the bytes associated with the given key values and given read options.
    pub fn multi_get_opt<K, I>(
        &self,
        keys: I,
        mut readopts: ReadOptions,
    ) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = K>,
    {
        readopts.set_snapshot(self);
        self.db.multi_get_opt(keys, &readopts)
    }

    /// Returns the bytes associated with the given key values, given column family and read options.
    pub fn multi_get_cf_opt<'b, K, I, W>(
        &self,
        keys_cf: I,
        mut readopts: ReadOptions,
    ) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'b W, K)>,
        W: AsColumnFamilyRef + 'b,
    {
        readopts.set_snapshot(self);
        self.db.multi_get_cf_opt(keys_cf, &readopts)
    }
}

impl<D: DBAccess> Drop for SnapshotWithThreadMode<'_, D> {
    fn drop(&mut self) {
        unsafe {
            self.db.release_snapshot(self.inner);
        }
    }
}

/// `Send` and `Sync` implementations for `SnapshotWithThreadMode` are safe, because `SnapshotWithThreadMode` is
/// immutable and can be safely shared between threads.
unsafe impl<D: DBAccess> Send for SnapshotWithThreadMode<'_, D> {}
unsafe impl<D: DBAccess> Sync for SnapshotWithThreadMode<'_, D> {}

/// An immutable snapshot that owns its database.
///
/// Column family handles passed to this type must belong to its database and
/// must remain valid for the read or iterator lifetime.
///
/// An iterator cannot outlive the snapshot:
/// ```compile_fail
/// use std::sync::Arc;
/// use rocksdb::{DB, IteratorMode, OwnedSnapshotWithThreadMode};
/// fn escape(db: Arc<DB>) {
///     let snapshot = OwnedSnapshotWithThreadMode::new(db).unwrap();
///     let cf = snapshot.db().cf_handle("default").unwrap();
///     let iter = snapshot.iterator_cf(&cf, IteratorMode::Start);
///     drop(snapshot);
///     drop(iter);
/// }
/// ```
/// Arbitrary database implementations cannot use the safe constructor:
/// ```compile_fail
/// use std::sync::Arc;
/// use rocksdb::{DBAccess, OwnedSnapshotWithThreadMode};
/// fn unsupported<D: DBAccess>(db: Arc<D>) {
///     let _ = OwnedSnapshotWithThreadMode::new(db);
/// }
/// ```
pub struct OwnedSnapshotWithThreadMode<D: DBAccess> {
    db: Arc<D>,
    inner: *const ffi::rocksdb_snapshot_t,
}

impl OwnedSnapshotWithThreadMode<DB> {
    /// Creates a snapshot of the writable primary database.
    pub fn new(db: Arc<DB>) -> Result<Self, Error> {
        let inner = unsafe { db.create_snapshot() };
        Self::from_pointer(db, inner)
    }

    fn from_pointer(db: Arc<DB>, inner: *const ffi::rocksdb_snapshot_t) -> Result<Self, Error> {
        if inner.is_null() {
            return Err(Error::new("RocksDB could not create a snapshot".into()));
        }
        Ok(Self { db, inner })
    }
}

impl<D: DBAccess> OwnedSnapshotWithThreadMode<D> {
    /// Returns the owner used to obtain column family handles.
    pub fn db(&self) -> &D {
        &self.db
    }

    fn read_options(&self) -> ReadOptions {
        let mut options = ReadOptions::default();
        // The holder keeps the snapshot and its owner alive for each read.
        unsafe { options.set_snapshot_raw(self.inner) };
        options
    }

    /// Reads from the default column family at this snapshot.
    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Error> {
        self.db.get_opt(key, &self.read_options())
    }

    /// Reads from a column family of this snapshot's database.
    pub fn get_cf<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.db.get_cf_opt(cf, key, &self.read_options())
    }

    /// Iterates a column family of this snapshot's database.
    pub fn iterator_cf<'s>(
        &'s self,
        cf: &impl AsColumnFamilyRef,
        mode: IteratorMode,
    ) -> DBIteratorWithThreadMode<'s, D> {
        DBIteratorWithThreadMode::new_cf(&*self.db, cf.inner(), self.read_options(), mode)
    }
}

impl<D: DBAccess> Drop for OwnedSnapshotWithThreadMode<D> {
    fn drop(&mut self) {
        // Release while the Arc still owns the same database allocation.
        unsafe { self.db.release_snapshot(self.inner) };
    }
}

// RocksDB permits concurrent snapshot reads and release. Arc ownership also
// requires the database to be safe to share and drop on another thread.
unsafe impl<D: DBAccess + Send + Sync> Send for OwnedSnapshotWithThreadMode<D> {}
unsafe impl<D: DBAccess + Send + Sync> Sync for OwnedSnapshotWithThreadMode<D> {}

#[cfg(test)]
mod owned_tests {
    use super::*;
    use crate::Options;

    #[test]
    fn owned_snapshot_holds_sequence_and_owner() {
        let path =
            std::env::temp_dir().join(format!("rocksdb-owned-snapshot-{}", std::process::id()));
        let mut options = Options::default();
        options.create_if_missing(true);
        let db = Arc::new(DB::open_cf(&options, &path, ["default"]).unwrap());
        assert!(OwnedSnapshotWithThreadMode::from_pointer(db.clone(), std::ptr::null()).is_err());
        db.put(b"key", b"old").unwrap();
        let old = OwnedSnapshotWithThreadMode::new(db.clone()).unwrap();
        db.put(b"key", b"new").unwrap();
        db.put(b"future", b"value").unwrap();
        db.flush().unwrap();
        db.compact_range::<&[u8], &[u8]>(None, None);
        let fresh = OwnedSnapshotWithThreadMode::new(db.clone()).unwrap();
        assert_eq!(old.get(b"key").unwrap().unwrap(), b"old");
        assert!(old.get(b"future").unwrap().is_none());
        assert_eq!(fresh.get(b"future").unwrap().unwrap(), b"value");
        let cf = fresh.db().cf_handle("default").unwrap();
        let fresh_rows = fresh
            .iterator_cf(&cf, IteratorMode::Start)
            .map(|row| row.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(fresh_rows.len(), 2);
        assert!(fresh_rows
            .iter()
            .any(|(key, value)| &**key == b"future" && &**value == b"value"));
        assert!(fresh_rows
            .iter()
            .any(|(key, value)| &**key == b"key" && &**value == b"new"));
        drop(fresh);
        drop(db);
        let cf = old.db().cf_handle("default").unwrap();
        let mut iterator = old.iterator_cf(&cf, IteratorMode::Start);
        let (key, value) = iterator.next().unwrap().unwrap();
        assert_eq!(&*key, b"key");
        assert_eq!(&*value, b"old");
        assert!(iterator.next().is_none());
        drop(iterator);
        assert_eq!(old.get(b"key").unwrap().unwrap(), b"old");
        drop(old);
        let reopened = DB::open(&options, &path).unwrap();
        assert_eq!(reopened.get(b"key").unwrap().unwrap(), b"new");
    }
}
