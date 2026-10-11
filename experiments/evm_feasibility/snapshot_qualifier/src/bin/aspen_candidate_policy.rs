//! Fixed saved inputs only; derive candidate DAG policy with no database access.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::aspen_policy;
use std::{env, path::PathBuf};
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let output = PathBuf::from(
        args.next()
            .context("usage: aspen_candidate_policy OUTPUT")?,
    );
    ensure!(
        args.next().is_none() && !output.to_string_lossy().starts_with("--"),
        "unexpected mode"
    );
    let output = aspen_policy::output_path(&output)?;
    let policy = aspen_policy::load_fixed()?;
    let report = policy.report(aspen_policy::implementation()?)?;
    aspen_policy::publish(&output, &policy, &report)
}
