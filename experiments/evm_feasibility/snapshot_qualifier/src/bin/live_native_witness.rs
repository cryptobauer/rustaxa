//! Fixed N4 capture command; no root/source/tuple override.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "--preflight" {
        return rustaxa_snapshot_qualifier::native_witness::preflight(Path::new(&args[1]));
    }
    ensure!(
        args.len() == 2,
        "usage: live_native_witness BUNDLE_OUTPUT REPORT_OUTPUT | --preflight REPORT_OUTPUT"
    );
    rustaxa_snapshot_qualifier::native_witness::capture(Path::new(&args[0]), Path::new(&args[1]))
}
