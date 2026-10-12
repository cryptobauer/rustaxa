//! Offline VRF replay command; raw vote/root authority remains fixed.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4 && args[0] == "--bundle-input" && args[2] == "--report-output",
        "usage: replay_vrf_witness --bundle-input BUNDLE --report-output REPORT"
    );
    rustaxa_snapshot_qualifier::vrf_witness::replay(Path::new(&args[1]), Path::new(&args[3]))
}
