//! Fixed remaining-voter capture. No voter/root/input/limit override exists.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    if a.len() == 2 && a[0] == "--preflight" {
        return rustaxa_snapshot_qualifier::vrf_sixth_voter_window::preflight(Path::new(&a[1]));
    }
    ensure!(
        a.len() == 4 && a[0] == "--bundle-output" && a[2] == "--report-output",
        "usage: live_vrf_sixth_voter_window --bundle-output BUNDLE --report-output REPORT | --preflight REPORT"
    );
    rustaxa_snapshot_qualifier::vrf_sixth_voter_window::capture(Path::new(&a[1]), Path::new(&a[3]))
}
