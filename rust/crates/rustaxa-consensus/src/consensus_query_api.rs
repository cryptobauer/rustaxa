//! Public query facade for Rust-owned consensus read models.
//!
//! The facade is the narrow read-only API that RPC, GraphQL, plugins, debug
//! tools, and CLI code should call instead of reaching into consensus managers,
//! mutable sidecars, or generic storage iterators. It owns no storage handle and
//! mutates no state; callers pass the current Rust storage owner for each query
//! and receive stable DTOs plus canonical bytes when compatibility materializers
//! still need legacy encodings.

use anyhow::{Context, Result};
use ethereum_types::{H160, H256};
use rlp::Rlp;
use rustaxa_storage::Storage;
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::final_chain::StoredFinalChainBlockHeader;
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;

/// Hash lookup result returned by public query facade methods.
///
/// `found` is false when the requested durable row does not exist. When
/// `found` is true, `hash` contains the canonical 32-byte object hash in
/// Taraxa/Ethereum byte order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryHashLookup {
    pub found: bool,
    pub hash: [u8; 32],
}

/// Stable public view of a finalized FinalChain block.
///
/// The view combines Rust FinalChain lookup rows with the PBFT period-data hash
/// index used by public block formatters. It deliberately exposes plain values
/// and canonical stored-header bytes rather than manager pointers, storage
/// iterators, or mutable compatibility sidecars.
///
/// `found` is false when either the block header or block hash row is absent.
/// The remaining fields are defaulted in that case. For genesis, or for
/// temporary compatibility data that lacks period-data rows, `has_pbft_hash` is
/// false and `pbft_block_hash` is zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainBlockView {
    pub found: bool,
    pub number: u64,
    pub hash: [u8; 32],
    pub parent_hash: [u8; 32],
    pub author: [u8; 20],
    pub state_root: [u8; 32],
    pub transactions_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub log_bloom: Vec<u8>,
    pub gas_used: u64,
    pub total_reward: [u8; 32],
    pub stored_header_rlp: Vec<u8>,
    pub has_pbft_hash: bool,
    pub pbft_block_hash: [u8; 32],
}

/// Read-only public query facade over Rust consensus storage.
///
/// The API owns only a cloned Rust storage handle. It does not own RPC/GraphQL
/// formatting, transaction expansion, account/state reads, network sync
/// execution, or plugin lifecycle. Those callers may format returned DTOs into
/// legacy JSON or objects while this facade remains the single consensus read
/// boundary.
pub struct ConsensusQueryApi {
    storage: Arc<Storage>,
}

impl ConsensusQueryApi {
    /// Creates a public query facade over a shared Rust storage owner.
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    /// Returns the canonical PBFT block hash for a finalized period.
    ///
    /// The hash is derived from the signed PBFT block embedded at item 0 of the
    /// stored `PeriodData` RLP, matching the legacy public block hash contract.
    /// Missing period data returns `found == false`; malformed period data is an
    /// error so public formatters can preserve their existing invalid-params
    /// behavior.
    pub fn pbft_block_hash_by_period(&self, period: u64) -> Result<QueryHashLookup> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(QueryHashLookup::default());
        }
        let period_rlp = Rlp::new(&period_data);
        Ok(QueryHashLookup {
            found: true,
            hash: keccak256(
                period_rlp
                    .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)
                    .context("CONSENSUS_QUERY_PERIOD_DATA_PBFT_BLOCK")?
                    .as_raw(),
            )
            .into(),
        })
    }

    /// Returns a finalized FinalChain block view by block number.
    ///
    /// The query reads only Rust-owned FinalChain and period storage rows. It
    /// does not materialize C++ `BlockHeader`/`PbftBlock` objects and it does
    /// not expand transactions or receipts. Callers that need legacy encodings
    /// can use `stored_header_rlp` while migrating to the typed fields.
    pub fn final_chain_block_by_number(&self, number: u64) -> Result<FinalChainBlockView> {
        let Some(stored_header_rlp) = self.storage.final_chain().block_header_raw(number)? else {
            return Ok(FinalChainBlockView::default());
        };
        let Some(hash_bytes) = self.storage.final_chain().block_hash_by_number(number)? else {
            return Ok(FinalChainBlockView::default());
        };
        let block_hash = h256_bytes(&hash_bytes).context("CONSENSUS_QUERY_FINAL_CHAIN_HASH")?;
        let stored_header =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&stored_header_rlp))
                .context("CONSENSUS_QUERY_FINAL_CHAIN_HEADER")?;
        let pbft_hash = self.pbft_block_hash_by_period(number)?;

        Ok(FinalChainBlockView {
            found: true,
            number,
            hash: block_hash.into(),
            parent_hash: stored_header.parent_hash.into(),
            author: H160::zero().into(),
            state_root: stored_header.state_root.into(),
            transactions_root: stored_header.transactions_root.into(),
            receipts_root: stored_header.receipts_root.into(),
            log_bloom: stored_header.log_bloom,
            gas_used: stored_header.gas_used,
            total_reward: stored_header.total_reward.to_big_endian(),
            stored_header_rlp,
            has_pbft_hash: pbft_hash.found,
            pbft_block_hash: pbft_hash.hash,
        })
    }
}

fn h256_bytes(bytes: &[u8]) -> Result<H256> {
    let array: [u8; 32] = bytes
        .try_into()
        .with_context(|| format!("expected 32-byte hash, got {}", bytes.len()))?;
    Ok(H256::from(array))
}

fn keccak256(data: &[u8]) -> H256 {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    H256::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::U256;
    use rlp::RlpStream;
    use rustaxa_storage::Config;
    use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlpOwned;
    use std::sync::Arc;

    fn test_storage(name: &str) -> (std::path::PathBuf, Arc<Storage>) {
        let path = std::env::temp_dir().join(format!(
            "rustaxa_consensus_query_api_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        (path, storage)
    }

    fn period_data_rlp(pbft_block_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(pbft_block_rlp, 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn stored_header_rlp() -> Vec<u8> {
        StoredBlockHeaderRlpOwned::from(&StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(1),
            state_root: H256::from_low_u64_be(2),
            transactions_root: H256::from_low_u64_be(3),
            receipts_root: H256::from_low_u64_be(4),
            log_bloom: vec![0xAA; 256],
            gas_used: 55,
            total_reward: U256::from(66u64),
        })
        .into_vec()
    }

    #[test]
    fn query_api_reads_final_chain_block_view_and_pbft_hash_from_storage() {
        let (path, storage) = test_storage("block_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_hash = H256::from_low_u64_be(77);
        let pbft_block_rlp = vec![0xC2, 0x01, 0x02];
        storage
            .period()
            .write(9, &period_data_rlp(&pbft_block_rlp))
            .unwrap();
        storage
            .final_chain()
            .write_block_header(9, block_hash, &stored_header_rlp(), &[])
            .unwrap();

        let view = api.final_chain_block_by_number(9).unwrap();
        assert!(view.found);
        assert_eq!(view.number, 9);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.parent_hash, H256::from_low_u64_be(1).0);
        assert_eq!(view.state_root, H256::from_low_u64_be(2).0);
        assert_eq!(view.transactions_root, H256::from_low_u64_be(3).0);
        assert_eq!(view.receipts_root, H256::from_low_u64_be(4).0);
        assert_eq!(view.log_bloom, vec![0xAA; 256]);
        assert_eq!(view.gas_used, 55);
        assert_eq!(view.total_reward, U256::from(66u64).to_big_endian());
        assert!(!view.stored_header_rlp.is_empty());
        assert!(view.has_pbft_hash);
        assert_eq!(view.pbft_block_hash, keccak256(&pbft_block_rlp).0);

        let lookup = api.pbft_block_hash_by_period(9).unwrap();
        assert!(lookup.found);
        assert_eq!(lookup.hash, view.pbft_block_hash);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn query_api_returns_not_found_for_missing_public_rows() {
        let (path, storage) = test_storage("missing");
        let api = ConsensusQueryApi::new(storage.clone());

        assert!(!api.final_chain_block_by_number(44).unwrap().found);
        assert!(!api.pbft_block_hash_by_period(44).unwrap().found);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
