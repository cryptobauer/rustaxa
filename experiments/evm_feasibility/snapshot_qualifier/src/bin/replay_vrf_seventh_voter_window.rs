//! Offline fixed remaining-voter replay; no DB input or authority override.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        a.len() == 4 && a[0] == "--bundle-input" && a[2] == "--report-output",
        "usage: replay_vrf_seventh_voter_window --bundle-input BUNDLE --report-output REPORT"
    );
    rustaxa_snapshot_qualifier::vrf_shared_window::seventh_replay(
        Path::new(&a[1]),
        Path::new(&a[3]),
    )
}
