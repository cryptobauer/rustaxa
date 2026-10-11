//! Additive diagnostic methods on the existing database owner.
use super::{
    c_char, c_int, c_uchar, ffi, iter, ptr, AsColumnFamilyRef, BTreeMap, CString,
    ColumnFamilyDescriptor, DBCommon, DBInner, DBPinnableSlice, DBWithThreadMode,
    DBWithThreadModeInner, MultiThreaded, Options, Path, ReadOptions, SingleThreaded, ThreadMode,
    DEFAULT_COLUMN_FAMILY_NAME,
};
use crate::bounded_read::{discard_error, BoundedReadError};

struct OpenOutputs {
    db: *mut ffi::rocksdb_t,
    handles: Vec<*mut ffi::rocksdb_column_family_handle_t>,
}

// Shared cleanup order, including null/partial outputs, for production and
// fault injection tests. CF owners must disappear before their database.
fn cleanup_outputs(
    db: *mut ffi::rocksdb_t,
    handles: &[*mut ffi::rocksdb_column_family_handle_t],
    mut destroy_cf: impl FnMut(*mut ffi::rocksdb_column_family_handle_t),
    close_db: impl FnOnce(*mut ffi::rocksdb_t),
) {
    for &handle in handles {
        if !handle.is_null() {
            destroy_cf(handle);
        }
    }
    if !db.is_null() {
        close_db(db);
    }
}
impl Drop for OpenOutputs {
    fn drop(&mut self) {
        cleanup_outputs(
            self.db,
            &self.handles,
            |p| unsafe { ffi::rocksdb_column_family_handle_destroy(p) },
            |p| unsafe { ffi::rocksdb_close(p) },
        );
    }
}

unsafe fn open_status(
    error: *mut c_char,
    db: *mut ffi::rocksdb_t,
    handles: &[*mut ffi::rocksdb_column_family_handle_t],
) -> Result<(), BoundedReadError> {
    unsafe { discard_error(error, BoundedReadError::Open) }?;
    if db.is_null() || handles.iter().any(|h| h.is_null()) {
        Err(BoundedReadError::NullHandle)
    } else {
        Ok(())
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::SingleThreaded {}
    impl Sealed for super::MultiThreaded {}
}

/// Built-in CF ownership modes accepted by bounded read-only construction.
/// Sealing prevents caller-defined, potentially panicking ownership transfer.
/// Ordinary constructors continue to accept the original `ThreadMode` trait.
pub trait BoundedReadThreadMode: ThreadMode + sealed::Sealed {}
impl BoundedReadThreadMode for SingleThreaded {}
impl BoundedReadThreadMode for MultiThreaded {}

impl<T: BoundedReadThreadMode> DBWithThreadMode<T> {
    /// Opens a fixed subset read-only without any directory creation or column
    /// discovery. The subset must contain `default`; no descriptor is inserted.
    /// The nonempty UTF8 path is limited to 4096 bytes and must have no NUL.
    /// One through nine unique, nonempty CF names of at most 64 bytes are allowed;
    /// CF names must have no NUL. `error_if_log_file_exist` rejects an existing WAL.
    /// Options/comparators and owner lifetimes are retained as supplied.
    ///
    /// Caller must precharge finite metadata and use source-fixed descriptors
    /// and options for an application-bound claim. This includes input Vec/String
    /// capacities and resources retained by options, before their construction.
    /// Engine memory/I/O is not
    /// bounded. Errors contain no engine text; ordinary open APIs are unchanged.
    /// InvalidPath/InvalidDescriptors reject before opening; Open/NullHandle
    /// release every non-null CF before closing the DB. Only built-in modes
    /// can construct a bounded owner; caller-defined transfer is rejected.
    ///
    /// ```compile_fail
    /// fn rejected<T: rocksdb::ThreadMode>(
    ///     opts: &rocksdb::Options,
    ///     path: &std::path::Path,
    ///     cfs: Vec<rocksdb::ColumnFamilyDescriptor>,
    /// ) {
    ///     let _ = rocksdb::DBWithThreadMode::<T>::open_cf_descriptors_read_only_bounded(
    ///         opts, path, cfs, false,
    ///     );
    /// }
    /// ```
    pub fn open_cf_descriptors_read_only_bounded<P: AsRef<Path>>(
        opts: &Options,
        path: P,
        cfs: Vec<ColumnFamilyDescriptor>,
        error_if_log_file_exist: bool,
    ) -> Result<Self, BoundedReadError> {
        let path = path.as_ref();
        let text = path.to_str().ok_or(BoundedReadError::InvalidPath)?;
        if text.is_empty() || text.len() > 4096 || text.as_bytes().contains(&0) {
            return Err(BoundedReadError::InvalidPath);
        }
        if cfs.is_empty()
            || cfs.len() > 9
            || !cfs.iter().any(|c| c.name == DEFAULT_COLUMN_FAMILY_NAME)
            || cfs.iter().enumerate().any(|(i, c)| {
                c.name.is_empty()
                    || c.name.len() > 64
                    || c.name.as_bytes().contains(&0)
                    || cfs[..i].iter().any(|p| p.name == c.name)
            })
        {
            return Err(BoundedReadError::InvalidDescriptors);
        }
        let cpath = CString::new(text).map_err(|_| BoundedReadError::InvalidPath)?;
        let outlive = iter::once(opts.outlive.clone())
            .chain(cfs.iter().map(|cf| cf.options.outlive.clone()))
            .collect();
        let names: Vec<CString> = cfs
            .iter()
            .map(|c| CString::new(c.name.as_bytes()).unwrap())
            .collect();
        let name_ptrs: Vec<_> = names.iter().map(|n| n.as_ptr()).collect();
        let cfopts: Vec<_> = cfs.iter().map(|c| c.options.inner.cast_const()).collect();
        let mut outputs = OpenOutputs {
            db: ptr::null_mut(),
            handles: vec![ptr::null_mut(); cfs.len()],
        };
        let mut error = ptr::null_mut();
        outputs.db = unsafe {
            ffi::rocksdb_open_for_read_only_column_families(
                opts.inner,
                cpath.as_ptr(),
                cfs.len() as c_int,
                name_ptrs.as_ptr(),
                cfopts.as_ptr(),
                outputs.handles.as_mut_ptr(),
                c_uchar::from(error_if_log_file_exist),
                &raw mut error,
            )
        };
        unsafe { open_status(error, outputs.db, &outputs.handles) }?;
        let mut cf_map = BTreeMap::new();
        for (descriptor, handle) in cfs.iter().zip(outputs.handles.iter()) {
            cf_map.insert(descriptor.name.clone(), *handle);
        }
        let owner_path = path.to_path_buf();
        let thread_map = T::new_cf_map_internal(cf_map);
        outputs.handles.fill(ptr::null_mut());
        let db = std::mem::replace(&mut outputs.db, ptr::null_mut());
        let result = Self {
            inner: DBWithThreadModeInner { inner: db },
            path: owner_path,
            cfs: thread_map,
            _outlive: outlive,
        };
        Ok(result)
    }
}

// Pins are exclusively owned by this adapter on entry; a returned error wins
// over a non-null value and releases that value before returning.
unsafe fn pinned_status(
    value: *mut ffi::rocksdb_pinnableslice_t,
    error: *mut c_char,
    destroy: impl FnOnce(*mut ffi::rocksdb_pinnableslice_t),
) -> Result<(), BoundedReadError> {
    if let Err(e) = unsafe { discard_error(error, BoundedReadError::PinnedRead) } {
        if !value.is_null() {
            destroy(value);
        }
        return Err(e);
    }
    Ok(())
}

unsafe fn pinned_result<'a>(
    value: *mut ffi::rocksdb_pinnableslice_t,
    error: *mut c_char,
) -> Result<Option<DBPinnableSlice<'a>>, BoundedReadError> {
    unsafe { pinned_status(value, error, |p| ffi::rocksdb_pinnableslice_destroy(p)) }?;
    if value.is_null() {
        Ok(None)
    } else {
        Ok(Some(unsafe { DBPinnableSlice::from_c(value) }))
    }
}

impl<T: ThreadMode, I: DBInner> DBCommon<T, I> {
    /// Pinned point read with fixed errors and a DB-tied borrow. No payload is
    /// copied. Null-without-error is missing; a reported engine error is Err.
    /// Engine failure to allocate its C error buffer is outside this guarantee.
    /// Caller charges the operation first and checks the pinned length before
    /// copying. Does not bound engine allocation or I/O.
    ///
    /// ```compile_fail
    /// use rocksdb::{DB, ReadOptions};
    /// let pin;
    /// {
    ///     let db = DB::open_default("unused-borrow-example").unwrap();
    ///     pin = db.get_pinned_bounded_opt(b"key", &ReadOptions::default()).unwrap();
    /// }
    /// drop(pin); // A pinned value cannot outlive its DB owner.
    /// ```
    pub fn get_pinned_bounded_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        readopts: &ReadOptions,
    ) -> Result<Option<DBPinnableSlice<'_>>, BoundedReadError> {
        if readopts.inner.is_null() {
            return Err(BoundedReadError::ReadOptions);
        }
        let key = key.as_ref();
        let mut error = ptr::null_mut();
        let value = unsafe {
            ffi::rocksdb_get_pinned(
                self.inner.inner(),
                readopts.inner,
                key.as_ptr().cast(),
                key.len(),
                &raw mut error,
            )
        };
        unsafe { pinned_result(value, error) }
    }

    /// CF variant of [`Self::get_pinned_bounded_opt`], with the same charging
    /// contract and engine error-buffer limitation, preserving the existing CF
    /// descriptor/comparator and DB pin lifetime. Errors are distinct from
    /// missing data; no owned payload or engine error text is allocated.
    pub fn get_pinned_cf_bounded_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        readopts: &ReadOptions,
    ) -> Result<Option<DBPinnableSlice<'_>>, BoundedReadError> {
        if readopts.inner.is_null() {
            return Err(BoundedReadError::ReadOptions);
        }
        let key = key.as_ref();
        let mut error = ptr::null_mut();
        let value = unsafe {
            ffi::rocksdb_get_pinned_cf(
                self.inner.inner(),
                readopts.inner,
                cf.inner(),
                key.as_ptr().cast(),
                key.len(),
                &raw mut error,
            )
        };
        unsafe { pinned_result(value, error) }
    }
}

#[cfg(test)]
#[path = "db_bounded_tests.rs"]
mod tests;
