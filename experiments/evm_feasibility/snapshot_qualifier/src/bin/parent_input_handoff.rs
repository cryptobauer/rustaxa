//! Fixed offline N4 parent-input handoff; no input/source/tuple overrides.
use anyhow::{Result, ensure};
use std::path::Path;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(args.len() == 1, "usage: parent_input_handoff REPORT_OUTPUT");
    rustaxa_snapshot_qualifier::parent_input_handoff::run(Path::new(&args[0]))
}
