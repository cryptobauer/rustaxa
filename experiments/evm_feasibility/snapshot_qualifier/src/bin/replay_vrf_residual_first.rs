//! Offline fixed RCV3 replay; no DB input or authority overrides.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        a.len() == 4 && a[0] == "--bundle-input" && a[2] == "--report-output",
        "usage: replay_vrf_residual_first --bundle-input BUNDLE --report-output REPORT"
    );
    rustaxa_snapshot_qualifier::vrf_shared_window::residual_first_replay(
        Path::new(&a[1]),
        Path::new(&a[3]),
    )
}
