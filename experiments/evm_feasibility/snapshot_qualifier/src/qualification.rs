//! Bounded paired identity qualification through existing Rust repository and
//! checkpoint owners. Historical pins are comparison targets, never inferred
//! from path names. Failure prevents dependent probes; no range inventory runs.
use anyhow::{Context, Result, ensure};
use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
use rustaxa_storage::{
    Column, ConcreteCheckpointReaders, FinalChainRepository, MetadataRepository,
};
use rustaxa_types::{
    FinalChainBlockNumber, StoredFinalChainBlockHeader,
    codec::rlp::final_chain::StoredBlockHeaderRlp, concrete_state::ConcreteStateIdentity,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

/// Historical retained-head comparison target, not an assertion about input.
pub const HEAD: u64 = 25_706_949;
const GENESIS: &str = "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377";
const ROOT: &str = "b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227";
const PRIOR_ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const HEADER_SHA: &str = "1bebb6391e97caa4e8d34db4336810ec71eae83ef975d48bfb05e0b56e81363f";

/// Exact point-check evidence. Network is genesis identity only; producer
/// revision, historical completeness and adoption authority remain unqualified.
#[derive(Debug, Serialize)]
pub struct PairEvidence {
    pub genesis_hash_hex: String,
    pub network_identity_basis: &'static str,
    pub head: u64,
    pub header_sha256: String,
    pub state_root_hex: String,
    pub prior_period: u64,
    pub prior_state_root_hex: String,
    pub descriptor_matches_header: bool,
    pub current_and_prior_roots_authenticated: bool,
    pub broad_inventory_performed: bool,
    pub exact_checkpoint_timing_qualified: bool,
    pub producer_revision_qualified: bool,
    pub complete_state_or_adoption_authorized: bool,
}

/// Opens only an existing application DB read-only, preserving Rust-owned
/// comparator descriptors; no create/repair/write capability is enabled.
pub fn open_application(path: &Path) -> Result<Arc<DBWithThreadMode<MultiThreaded>>> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    options.set_max_open_files(128);
    let columns = DBWithThreadMode::<MultiThreaded>::list_cf(&options, path)?;
    let descriptors = columns.iter().map(|name| {
        Column::from_name(name).map_or_else(
            |_| ColumnFamilyDescriptor::new(name, Options::default()),
            |column| column.descriptor(&Options::default()),
        )
    });
    Ok(Arc::new(DBWithThreadMode::open_cf_descriptors_read_only(
        &options,
        path,
        descriptors,
        false,
    )?))
}

/// Performs four application point reads then asks the concrete owner to
/// validate the durable descriptor and authenticate both retained roots.
/// Returns a pinned owner only after every historical identity check succeeds.
pub fn qualify(
    application: &Path,
    state: &Path,
) -> Result<(
    ConcreteCheckpointReaders,
    ConcreteStateIdentity,
    PairEvidence,
)> {
    let db = open_application(application)?;
    let metadata = MetadataRepository::new(db.clone());
    let chain = FinalChainRepository::new(db);
    let genesis = metadata.genesis_hash()?.context("missing genesis")?;
    ensure!(
        hex::encode(&genesis) == GENESIS,
        "restored genesis differs from historical mainnet identity"
    );
    let raw_head = chain.meta_value(1)?.context("missing head")?;
    let head = u64::from_le_bytes(
        raw_head
            .as_slice()
            .try_into()
            .context("head must be eight bytes")?,
    );
    ensure!(head == HEAD, "restored head differs from historical target");
    let header = chain
        .block_header_raw(head)?
        .context("missing head header")?;
    let prior = chain
        .block_header_raw(head - 1)?
        .context("missing prior header")?;
    let identity = validate_headers(head, &header, &prior)?;
    let prior_header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&prior))?;
    let prior_identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(head - 1),
        state_root: prior_header.state_root.into(),
    };
    let readers =
        ConcreteCheckpointReaders::open_read_only(state, identity, [prior_identity, identity])?;
    let evidence = PairEvidence {
        genesis_hash_hex: hex::encode(genesis),
        network_identity_basis: "exact historical mainnet genesis hash; not producer/config certification",
        head,
        header_sha256: hex::encode(Sha256::digest(&header)),
        state_root_hex: hex::encode(identity.state_root),
        prior_period: head - 1,
        prior_state_root_hex: hex::encode(prior_identity.state_root),
        descriptor_matches_header: readers.committed_identity() == identity,
        current_and_prior_roots_authenticated: true,
        broad_inventory_performed: false,
        exact_checkpoint_timing_qualified: false,
        producer_revision_qualified: false,
        complete_state_or_adoption_authorized: false,
    };
    Ok((readers, identity, evidence))
}

fn validate_headers(head: u64, header: &[u8], prior: &[u8]) -> Result<ConcreteStateIdentity> {
    ensure!(head == HEAD, "head differs from target");
    ensure!(
        hex::encode(Sha256::digest(header)) == HEADER_SHA,
        "head header hash differs from historical row"
    );
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(header))?;
    let prior = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(prior))?;
    ensure!(
        hex::encode(header.state_root) == ROOT,
        "head state root differs"
    );
    ensure!(
        hex::encode(prior.state_root) == PRIOR_ROOT,
        "prior state root differs"
    );
    Ok(ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(head),
        state_root: header.state_root.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_drift_fails_before_concrete_open() {
        assert!(validate_headers(HEAD - 1, &[], &[]).is_err());
        assert!(validate_headers(HEAD, b"different header", &[]).is_err());
    }
}
