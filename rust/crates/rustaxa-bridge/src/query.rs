//! CXX bridge for the Rust-owned public consensus query facade.
//!
//! The bridge exposes read-only DTOs for RPC, GraphQL, plugins, debug tools, and
//! CLI callers. It adapts the shared Rust storage owner into
//! `ConsensusQueryApi` calls without handing public layers manager pointers,
//! storage iterators, or mutable compatibility sidecars.

use crate::ffi::{rustaxa_ffi, BridgeConsensusQueryApi, BridgeStorage};

fn query_hash_lookup_to_ffi(lookup: rustaxa_consensus::QueryHashLookup) -> rustaxa_ffi::HashLookup {
    rustaxa_ffi::HashLookup {
        found: lookup.found,
        hash: lookup.hash,
    }
}

fn final_chain_block_view_to_ffi(
    view: rustaxa_consensus::FinalChainBlockView,
) -> rustaxa_ffi::FinalChainBlockView {
    rustaxa_ffi::FinalChainBlockView {
        found: view.found,
        number: view.number,
        hash: view.hash,
        parent_hash: view.parent_hash,
        author: view.author,
        state_root: view.state_root,
        transactions_root: view.transactions_root,
        receipts_root: view.receipts_root,
        log_bloom: view.log_bloom,
        gas_used: view.gas_used,
        total_reward: view.total_reward,
        stored_header_rlp: view.stored_header_rlp,
        has_pbft_hash: view.has_pbft_hash,
        pbft_block_hash: view.pbft_block_hash,
    }
}

fn dag_hashes_to_ffi(hashes: Vec<[u8; 32]>) -> Vec<rustaxa_ffi::DagHash> {
    hashes
        .into_iter()
        .map(|hash| rustaxa_ffi::DagHash { hash })
        .collect()
}

fn dag_block_view_to_ffi(view: rustaxa_consensus::DagBlockView) -> rustaxa_ffi::DagBlockPublicView {
    rustaxa_ffi::DagBlockPublicView {
        found: view.found,
        pivot: view.pivot,
        level: view.level,
        tips: dag_hashes_to_ffi(view.tips),
        transactions: dag_hashes_to_ffi(view.transactions),
        trx_estimations: view.trx_estimations,
        signature: view.signature,
        hash: view.hash,
        sender: view.sender,
        timestamp: view.timestamp,
        finalized_period_found: view.finalized_period_found,
        finalized_period: view.finalized_period,
        finalized_position: view.finalized_position,
        has_vdf: view.has_vdf,
        vdf_proof: view.vdf_proof,
        vdf_sol1: view.vdf_sol1,
        vdf_sol2: view.vdf_sol2,
        vdf_difficulty: view.vdf_difficulty,
    }
}

/// Creates a stateless public consensus query facade.
pub fn create_consensus_query_api(storage: &BridgeStorage) -> Box<BridgeConsensusQueryApi> {
    Box::new(BridgeConsensusQueryApi(
        rustaxa_consensus::ConsensusQueryApi::new(storage.0.clone()),
    ))
}

impl BridgeConsensusQueryApi {
    /// Returns the canonical PBFT block hash for a finalized period.
    pub fn consensus_query_pbft_block_hash_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::HashLookup, anyhow::Error> {
        Ok(query_hash_lookup_to_ffi(
            self.0.pbft_block_hash_by_period(period)?,
        ))
    }

    /// Returns a stable FinalChain public block view by finalized block number.
    pub fn consensus_query_final_chain_block_by_number(
        &self,
        number: u64,
    ) -> Result<rustaxa_ffi::FinalChainBlockView, anyhow::Error> {
        Ok(final_chain_block_view_to_ffi(
            self.0.final_chain_block_by_number(number)?,
        ))
    }

    /// Returns a stable DAG public block view by block hash.
    pub fn consensus_query_dag_block_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::DagBlockPublicView, anyhow::Error> {
        Ok(dag_block_view_to_ffi(self.0.dag_block_by_hash(*hash)?))
    }

    /// Returns stable DAG public block views for a contiguous level window.
    pub fn consensus_query_dag_blocks_by_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<rustaxa_ffi::DagBlockPublicView>, anyhow::Error> {
        Ok(self
            .0
            .dag_blocks_by_level(level, number_of_levels)?
            .into_iter()
            .map(dag_block_view_to_ffi)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H256, U256};
    use rlp::RlpStream;
    use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlpOwned;
    use rustaxa_types::final_chain::StoredFinalChainBlockHeader;

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

    fn keccak256(data: &[u8]) -> H256 {
        let mut hasher = tiny_keccak::Keccak::v256();
        tiny_keccak::Hasher::update(&mut hasher, data);
        let mut out = [0u8; 32];
        tiny_keccak::Hasher::finalize(hasher, &mut out);
        H256::from(out)
    }

    fn stored_header_rlp() -> Vec<u8> {
        StoredBlockHeaderRlpOwned::from(&StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(10),
            state_root: H256::from_low_u64_be(11),
            transactions_root: H256::from_low_u64_be(12),
            receipts_root: H256::from_low_u64_be(13),
            log_bloom: vec![0xBB; 256],
            gas_used: 99,
            total_reward: U256::from(100u64),
        })
        .into_vec()
    }

    #[test]
    fn bridge_consensus_query_api_reads_public_block_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block_hash = H256::from_low_u64_be(88);
        let pbft_block_rlp = vec![0xC2, 0x03, 0x04];

        storage
            .save_period_data(15, period_data_rlp(&pbft_block_rlp))
            .unwrap();
        storage
            .seed_final_chain_conformance_lookup_rows(
                1,
                vec![0x01],
                15,
                &block_hash.0,
                stored_header_rlp(),
                &H256::from_low_u64_be(99).0,
                vec![],
                &H256::from_low_u64_be(100).0,
                vec![],
                15,
                vec![],
            )
            .unwrap();

        let view = api.consensus_query_final_chain_block_by_number(15).unwrap();
        assert!(view.found);
        assert_eq!(view.number, 15);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.parent_hash, H256::from_low_u64_be(10).0);
        assert_eq!(view.state_root, H256::from_low_u64_be(11).0);
        assert_eq!(view.gas_used, 99);
        assert_eq!(view.total_reward, U256::from(100u64).to_big_endian());
        assert!(view.has_pbft_hash);

        let lookup = api.consensus_query_pbft_block_hash_by_period(15).unwrap();
        assert!(lookup.found);
        assert_eq!(lookup.hash, view.pbft_block_hash);

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_dag_block_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_dag_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let block_rlp = dag_block_rlp();
        let block_hash = keccak256(&block_rlp);

        storage
            .save_dag_block(&block_hash.0, 5, 1, block_rlp)
            .unwrap();
        storage.save_dag_block_period(&block_hash.0, 9, 2).unwrap();

        let view = api
            .consensus_query_dag_block_by_hash(&block_hash.0)
            .unwrap();
        assert!(view.found);
        assert_eq!(view.hash, block_hash.0);
        assert_eq!(view.pivot, H256::from_low_u64_be(1).0);
        assert_eq!(view.level, 5);
        assert_eq!(view.transactions.len(), 2);
        assert!(view.finalized_period_found);
        assert_eq!(view.finalized_period, 9);
        assert_eq!(view.vdf_proof, vec![0x11; 80]);
        assert_eq!(view.vdf_sol1, vec![0x22, 0x23]);
        assert_eq!(view.vdf_sol2, vec![0x33, 0x34]);
        assert_eq!(view.vdf_difficulty, 7);

        let level_views = api.consensus_query_dag_blocks_by_level(5, 1).unwrap();
        assert_eq!(level_views.len(), 1);
        assert_eq!(level_views[0].hash, block_hash.0);

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
