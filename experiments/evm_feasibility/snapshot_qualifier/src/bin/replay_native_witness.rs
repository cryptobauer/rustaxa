//! Fixed N4 offline replay command; no root/source/tuple override.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: replay_native_witness BUNDLE_INPUT REPORT_OUTPUT"
    );
    rustaxa_snapshot_qualifier::native_witness::replay(Path::new(&args[0]), Path::new(&args[1]))
}
