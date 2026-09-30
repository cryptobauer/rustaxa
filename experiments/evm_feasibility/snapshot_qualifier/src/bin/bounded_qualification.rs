//! Four-key application qualification plus concrete descriptor/root verification.
//! Uses no inventory/replay and creates a new evidence file exclusively.
use anyhow::{Context, Result, ensure};
use rustaxa_snapshot_qualifier::{paths, qualification};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: bounded_qualification COPY OUTPUT")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: bounded_qualification COPY OUTPUT")?,
    );
    ensure!(args.next().is_none());
    let paths = paths::validate(&input, &output)?;
    let (_, _, evidence) = qualification::qualify(&paths.application, &paths.state)?;
    let source_hash = hex::encode(Sha256::digest(
        [
            include_bytes!("bounded_qualification.rs").as_slice(),
            include_bytes!("../qualification.rs"),
            include_bytes!("../paths.rs"),
        ]
        .concat(),
    ));
    let report = json!({"schema": 1, "input_copy": paths.input, "tool_source_sha256": source_hash, "open_mode": "read_only", "pair": evidence});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
