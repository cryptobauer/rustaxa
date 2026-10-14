//! Fixed partial-period VRF capture; no voter/root/count/limit overrides.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    if a.len() == 2 && a[0] == "--preflight" {
        return rustaxa_snapshot_qualifier::vrf_shared_window::residual_first_preflight(Path::new(
            &a[1],
        ));
    }
    ensure!(
        a.len() == 4 && a[0] == "--bundle-output" && a[2] == "--report-output",
        "usage: live_vrf_residual_first --bundle-output BUNDLE --report-output REPORT | --preflight REPORT"
    );
    rustaxa_snapshot_qualifier::vrf_shared_window::residual_first_capture(
        Path::new(&a[1]),
        Path::new(&a[3]),
    )
}
