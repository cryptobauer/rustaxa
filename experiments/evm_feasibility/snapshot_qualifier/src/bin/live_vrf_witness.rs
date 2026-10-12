//! Fixed VRF capture command. No input/root/family/limit override exists.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "--preflight" {
        return rustaxa_snapshot_qualifier::vrf_witness::preflight(Path::new(&args[1]));
    }
    ensure!(
        args.len() == 4 && args[0] == "--bundle-output" && args[2] == "--report-output",
        "usage: live_vrf_witness --bundle-output BUNDLE --report-output REPORT | --preflight REPORT"
    );
    rustaxa_snapshot_qualifier::vrf_witness::capture(Path::new(&args[1]), Path::new(&args[3]))
}
