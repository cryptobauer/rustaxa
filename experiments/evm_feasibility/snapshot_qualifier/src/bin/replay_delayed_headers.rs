//! Offline replay of captured guarded reads; never opens a database.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::delayed_headers::{self, Evidence};
use std::{env, fs::File, io::Read, path::PathBuf};
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: replay_delayed_headers CAPTURE OUTPUT")?,
    );
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let mut bytes = Vec::new();
    File::open(input)?
        .take(delayed_headers::OUTPUT_CAP as u64 + 1)
        .read_to_end(&mut bytes)?;
    let report = delayed_headers::replay(&Evidence::load_fixed()?, &bytes)?;
    delayed_headers::publish(&output, &report)
}
