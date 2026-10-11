//! Fixed errors for explicitly bounded diagnostic reads. Ordinary APIs keep
//! their full engine messages. Engine allocation failures remain outside this
//! contract: the C API can lose status if its error allocation fails.
use crate::ffi;
use libc::{c_char, c_void};

/// An allocation-free diagnostic error. Engine text is discarded without
/// scanning or copying it. Open metadata and engine work are separate costs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundedReadError {
    /// Empty, nonUTF8, NUL-containing or over4096-byte path, before FFI open.
    InvalidPath,
    /// Not one through nine unique valid CF names including explicit default.
    InvalidDescriptors,
    /// Read-only open returned an engine error; all returned outputs are freed.
    Open,
    /// Open returned a null DB or CF handle without an engine error.
    NullHandle,
    /// The supplied read options have no live engine object.
    ReadOptions,
    /// Point read returned an engine error; any returned pin is destroyed.
    PinnedRead,
    /// Raw cursor has an engine error; invalidity must not imply exhaustion.
    IteratorStatus,
}
impl std::fmt::Display for BoundedReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidPath => "invalid bounded database path",
            Self::InvalidDescriptors => "invalid bounded column descriptors",
            Self::Open => "bounded read-only open failed",
            Self::NullHandle => "bounded open returned a null handle",
            Self::ReadOptions => "bounded read options unavailable",
            Self::PinnedRead => "bounded pinned read failed",
            Self::IteratorStatus => "bounded iterator failed",
        })
    }
}
impl std::error::Error for BoundedReadError {}

// The callback is shared with injection tests. The pointer is never read.
fn discard_with(
    error: *mut c_char,
    kind: BoundedReadError,
    free: impl FnOnce(*mut c_void),
) -> Result<(), BoundedReadError> {
    if error.is_null() {
        Ok(())
    } else {
        free(error.cast());
        Err(kind)
    }
}

/// Takes ownership of a nullable error allocated by RocksDB. Never scans it.
/// Caller must provide a live allocation, exclusively owned by this function.
pub(crate) unsafe fn discard_error(
    error: *mut c_char,
    kind: BoundedReadError,
) -> Result<(), BoundedReadError> {
    discard_with(error, kind, |p| unsafe { ffi::rocksdb_free(p) })
}

// Shared with tests that inject an invalid cursor error without damaging a DB.
pub(crate) fn iterator_status_with(
    operation: impl FnOnce(&mut *mut c_char),
) -> Result<(), BoundedReadError> {
    let mut error = std::ptr::null_mut();
    operation(&mut error);
    unsafe { discard_error(error, BoundedReadError::IteratorStatus) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn bounded_error_is_opaque_and_freed_once_for_long_nonterminated_inputs() {
        for n in [1, 4096, 1_048_576, 8_388_608] {
            let pointer = unsafe { libc::malloc(n) };
            assert!(!pointer.is_null());
            // No terminator: a CStr/strlen conversion would access outside the
            // allocation. This test uses the same classification as production.
            unsafe { std::ptr::write_bytes(pointer, 0xff, n) };
            let frees = Cell::new(0);
            let result = discard_with(pointer.cast(), BoundedReadError::PinnedRead, |p| {
                assert_eq!(p, pointer);
                frees.set(frees.get() + 1);
                unsafe { ffi::rocksdb_free(p) };
            });
            assert_eq!(result, Err(BoundedReadError::PinnedRead));
            assert_eq!(frees.get(), 1);
        }
        let frees = Cell::new(0);
        assert_eq!(
            discard_with(std::ptr::null_mut(), BoundedReadError::PinnedRead, |_| {
                frees.set(1)
            }),
            Ok(())
        );
        assert_eq!(frees.get(), 0);
        // An inaccessible mock address proves classification never reads it.
        assert_eq!(
            discard_with(
                std::ptr::dangling_mut(),
                BoundedReadError::IteratorStatus,
                |_| ()
            ),
            Err(BoundedReadError::IteratorStatus)
        );
    }
}
