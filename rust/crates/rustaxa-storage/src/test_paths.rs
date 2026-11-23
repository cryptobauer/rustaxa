//! Isolated filesystem paths for tests.

use std::cell::Cell;
use std::path::PathBuf;
use tempfile::TempDir;

thread_local! {
    static ROOT: TempDir = tempfile::Builder::new()
        .prefix("rustaxa-test-")
        .tempdir()
        .expect("create test directory");
    static NEXT_PATH: Cell<u64> = const { Cell::new(0) };
}

/// Return an unused path inside the current test thread's private directory.
/// The directory and any remaining files are removed when the thread exits.
pub(crate) fn new_path(name: &str) -> PathBuf {
    let index = NEXT_PATH.get();
    NEXT_PATH.set(index + 1);
    ROOT.with(|root| root.path().join(format!("{index}-{name}")))
}
