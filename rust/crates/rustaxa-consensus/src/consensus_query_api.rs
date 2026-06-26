//! Public query facade for Rust-owned consensus read models.
//!
//! The facade is the narrow read-only API that RPC, GraphQL, plugins, debug
//! tools, and CLI code should call instead of reaching into consensus managers,
//! mutable sidecars, or generic storage iterators. It owns only a cloned Rust
//! storage handle and mutates no state; callers receive stable DTOs plus
//! canonical bytes when compatibility materializers still need legacy encodings.

use anyhow::{Context, Result};
use ethereum_types::{H160, H256};
use rlp::Rlp;
use rustaxa_storage::Storage;
use rustaxa_types::codec::rlp::dag::DagBlockRlp;
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::dag::DagBlock;
use rustaxa_types::final_chain::StoredFinalChainBlockHeader;
use rustaxa_vdf::vdf_sortition::decode_vdf_sortition_payload;
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

/// Stable public view of one DAG block.
///
/// The view is loaded from Rust DAG storage and contains the base facts public
/// RPC/GraphQL formatters need for DAG block JSON without exposing a live DAG
/// manager or C++ block object. `finalized_period_found` distinguishes
/// non-finalized blocks from finalized blocks whose period/position index has
/// already been written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DagBlockView {
    pub found: bool,
    pub pivot: [u8; 32],
    pub level: u64,
    pub tips: Vec<[u8; 32]>,
    pub transactions: Vec<[u8; 32]>,
    pub trx_estimations: u64,
    pub signature: Vec<u8>,
    pub hash: [u8; 32],
    pub sender: [u8; 20],
    pub timestamp: u64,
    pub finalized_period_found: bool,
    pub finalized_period: u64,
    pub finalized_position: u32,
    pub has_vdf: bool,
    pub vdf_proof: Vec<u8>,
    pub vdf_sol1: Vec<u8>,
    pub vdf_sol2: Vec<u8>,
    pub vdf_difficulty: u16,
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

    /// Returns a public DAG block view by block hash.
    ///
    /// The query reads canonical DAG block bytes and finalized period metadata
    /// from Rust storage. It does not expand transaction hashes into
    /// transaction objects and it does not consult a live `DagManager` or
    /// `PbftManager`. Missing DAG block bytes return `found == false`;
    /// malformed block or VDF bytes are returned as errors for public adapters
    /// to map to their existing invalid-params behavior.
    pub fn dag_block_by_hash(&self, hash: [u8; 32]) -> Result<DagBlockView> {
        let requested_hash = H256::from(hash);
        let Some(block_rlp) = self.storage.dag().by_hash_rlp_optional(requested_hash)? else {
            return Ok(DagBlockView::default());
        };
        let block = DagBlock::try_from(DagBlockRlp::new(&block_rlp))
            .context("CONSENSUS_QUERY_DAG_BLOCK_DECODE")?;
        let vdf = decode_vdf_sortition_payload(&block.vdf)
            .context("CONSENSUS_QUERY_DAG_BLOCK_VDF_DECODE")?;
        let finalized = self.storage.dag().period_optional(requested_hash)?;
        let sender = block.recover_sender().unwrap_or_default();

        Ok(DagBlockView {
            found: true,
            pivot: block.pivot.into(),
            level: block.level,
            tips: block.tips.into_iter().map(Into::into).collect(),
            transactions: block.transactions.into_iter().map(Into::into).collect(),
            trx_estimations: block.gas_estimation,
            signature: block.signature.to_vec(),
            hash: keccak256(&block_rlp).into(),
            sender: sender.into(),
            timestamp: block.timestamp,
            finalized_period_found: finalized.is_some(),
            finalized_period: finalized.map(|(period, _)| period).unwrap_or_default(),
            finalized_position: finalized.map(|(_, position)| position).unwrap_or_default(),
            has_vdf: true,
            vdf_proof: vdf.vrf_proof.to_vec(),
            vdf_sol1: vdf.vdf_solution_proof,
            vdf_sol2: vdf.vdf_solution_output,
            vdf_difficulty: vdf.difficulty,
        })
    }

    /// Returns public DAG block views for a contiguous level window.
    ///
    /// The query reads level indexes and DAG block bytes from Rust storage and
    /// materializes the same stable DTO as [`Self::dag_block_by_hash`]. Missing
    /// block payloads referenced by a level index are skipped, matching the
    /// legacy storage query behavior during transitional repair windows.
    pub fn dag_blocks_by_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<DagBlockView>> {
        let hashes = self
            .storage
            .dag()
            .hashes_at_level_range(level, number_of_levels)?;
        let mut views = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let view = self.dag_block_by_hash(hash.into())?;
            if view.found {
                views.push(view);
            }
        }
        Ok(views)
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

    fn dag_block_rlp() -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11; 80]);
        vdf.append(&vec![0x22, 0x23]);
        vdf.append(&vec![0x33, 0x34]);
        vdf.append(&7u16);

        let mut block = RlpStream::new_list(8);
        block.append(&H256::from_low_u64_be(1));
        block.append(&5u64);
        block.append(&123u64);
        block.append(&vdf.out().to_vec());
        block.append_list(&[H256::from_low_u64_be(2)]);
        block.append_list(&[H256::from_low_u64_be(3), H256::from_low_u64_be(4)]);
        block.append(&vec![0x44; 65]);
        block.append(&987u64);
        block.out().to_vec()
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

    #[test]
    fn query_api_reads_dag_block_view_and_finalized_period_from_storage() {
        let (path, storage) = test_storage("dag_block_view");
        let api = ConsensusQueryApi::new(storage.clone());
        let block_rlp = dag_block_rlp();
        let block_hash = keccak256(&block_rlp);
        storage.dag().write(block_hash, 5, 1, &block_rlp).unwrap();
        storage.dag().write_period(block_hash, 9, 2).unwrap();

        let view = api.dag_block_by_hash(block_hash.0).unwrap();
        assert!(view.found);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.pivot, H256::from_low_u64_be(1).0);
        assert_eq!(view.level, 5);
        assert_eq!(view.timestamp, 123);
        assert_eq!(view.tips, vec![H256::from_low_u64_be(2).0]);
        assert_eq!(
            view.transactions,
            vec![H256::from_low_u64_be(3).0, H256::from_low_u64_be(4).0]
        );
        assert_eq!(view.trx_estimations, 987);
        assert_eq!(view.signature, vec![0x44; 65]);
        assert!(view.finalized_period_found);
        assert_eq!(view.finalized_period, 9);
        assert_eq!(view.vdf_proof, vec![0x11; 80]);
        assert_eq!(view.vdf_sol1, vec![0x22, 0x23]);
        assert_eq!(view.vdf_sol2, vec![0x33, 0x34]);
        assert_eq!(view.vdf_difficulty, 7);

        let level_views = api.dag_blocks_by_level(5, 1).unwrap();
        assert_eq!(level_views.len(), 1);
        assert_eq!(level_views[0].hash, block_hash.0);

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
