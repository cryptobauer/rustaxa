//! Fixed read-only application-copy probe: three identity guards, seven pairs.
use anyhow::{Context, Result, ensure};
use rocksdb::{DBWithThreadMode, MultiThreaded};
use rustaxa_snapshot_qualifier::{
    delayed_headers::{self, Evidence, PointSource, Read},
    paths, qualification,
};
use rustaxa_storage::{FinalChainRepository, MetadataRepository, PeriodRepository};
use std::{env, path::PathBuf};
struct Application {
    chain: FinalChainRepository<DBWithThreadMode<MultiThreaded>>,
    metadata: MetadataRepository<DBWithThreadMode<MultiThreaded>>,
    periods: PeriodRepository<DBWithThreadMode<MultiThreaded>>,
}
impl PointSource for Application {
    fn read(&mut self, request: &Read) -> Result<Option<Vec<u8>>> {
        match request {
            Read::Head => self.chain.meta_value(1),
            Read::Genesis => self.metadata.genesis_hash(),
            Read::Header(period) => self.chain.block_header_raw(*period),
            Read::PeriodData(period) => Ok(Some(self.periods.data_raw(*period)?)),
        }
    }
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let copy = PathBuf::from(args.next().context("usage: delayed_headers COPY OUTPUT")?);
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let guarded = paths::validate(&copy, &output)?;
    let evidence = Evidence::load_fixed()?;
    let pins = delayed_headers::source_pins(&evidence)?;
    let database = qualification::open_application(&guarded.application)?;
    let mut source = Application {
        chain: FinalChainRepository::new(database.clone()),
        metadata: MetadataRepository::new(database.clone()),
        periods: PeriodRepository::new(database),
    };
    let report = delayed_headers::acquire(&evidence, &mut source, pins)?;
    delayed_headers::publish(&guarded.output, &report)?;
    ensure!(
        report["qualification"]["headers_and_roots_authenticated_to_local_h"] == true,
        "delayed header acquisition failed; bounded diagnostic published"
    );
    Ok(())
}
