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

fn pbft_hashes_to_ffi(hashes: Vec<[u8; 32]>) -> Vec<rustaxa_ffi::PbftFinalizationHash> {
    hashes
        .into_iter()
        .map(|hash| rustaxa_ffi::PbftFinalizationHash { hash })
        .collect()
}

fn pbft_extra_data_view_to_ffi(
    view: rustaxa_consensus::PbftBlockExtraDataView,
) -> rustaxa_ffi::PbftBlockExtraDataView {
    rustaxa_ffi::PbftBlockExtraDataView {
        found: view.found,
        major_version: view.major_version,
        minor_version: view.minor_version,
        patch_version: view.patch_version,
        net_version: view.net_version,
        node_implementation: view.node_implementation,
        has_pillar_block_hash: view.has_pillar_block_hash,
        pillar_block_hash: view.pillar_block_hash,
    }
}

fn pbft_schedule_block_view_to_ffi(
    view: rustaxa_consensus::PbftScheduleBlockView,
) -> rustaxa_ffi::PbftScheduleBlockView {
    rustaxa_ffi::PbftScheduleBlockView {
        found: view.found,
        prev_block_hash: view.prev_block_hash,
        dag_block_hash_as_pivot: view.dag_block_hash_as_pivot,
        order_hash: view.order_hash,
        final_chain_hash: view.final_chain_hash,
        period: view.period,
        timestamp: view.timestamp,
        block_hash: view.block_hash,
        signature: view.signature,
        beneficiary: view.beneficiary,
        reward_votes: pbft_hashes_to_ffi(view.reward_votes),
        has_extra_data: view.has_extra_data,
        extra_data: pbft_extra_data_view_to_ffi(view.extra_data),
        dag_blocks_order: pbft_hashes_to_ffi(view.dag_blocks_order),
    }
}

fn pbft_node_version_view_to_ffi(
    view: rustaxa_consensus::PbftNodeVersionView,
) -> rustaxa_ffi::PbftNodeVersionView {
    rustaxa_ffi::PbftNodeVersionView {
        found: view.found,
        beneficiary: view.beneficiary,
        major_version: view.major_version,
        minor_version: view.minor_version,
        patch_version: view.patch_version,
    }
}

fn pillar_vote_count_change_to_ffi(
    change: rustaxa_consensus::PillarBlockViewVoteCountChange,
) -> rustaxa_ffi::PillarBlockViewVoteCountChange {
    rustaxa_ffi::PillarBlockViewVoteCountChange {
        address: change.address,
        vote_count_change: change.vote_count_change,
    }
}

fn pillar_signature_to_ffi(
    signature: rustaxa_consensus::PillarBlockViewSignature,
) -> rustaxa_ffi::PillarBlockViewSignature {
    rustaxa_ffi::PillarBlockViewSignature {
        r: signature.r,
        vs: signature.vs,
    }
}

fn pillar_block_data_view_to_ffi(
    view: rustaxa_consensus::PillarBlockDataView,
) -> rustaxa_ffi::PillarBlockDataView {
    rustaxa_ffi::PillarBlockDataView {
        found: view.found,
        pbft_period: view.pbft_period,
        state_root: view.state_root,
        previous_pillar_block_hash: view.previous_pillar_block_hash,
        bridge_root: view.bridge_root,
        epoch: view.epoch,
        validator_vote_count_changes: view
            .validator_vote_count_changes
            .into_iter()
            .map(pillar_vote_count_change_to_ffi)
            .collect(),
        block_hash: view.block_hash,
        signatures: view
            .signatures
            .into_iter()
            .map(pillar_signature_to_ffi)
            .collect(),
    }
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

    /// Returns a stable PBFT schedule-block public view by finalized period.
    pub fn consensus_query_pbft_schedule_block_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PbftScheduleBlockView, anyhow::Error> {
        Ok(pbft_schedule_block_view_to_ffi(
            self.0.pbft_schedule_block_by_period(period)?,
        ))
    }

    /// Returns PBFT author and semantic-version facts by finalized period.
    pub fn consensus_query_pbft_node_version_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PbftNodeVersionView, anyhow::Error> {
        Ok(pbft_node_version_view_to_ffi(
            self.0.pbft_node_version_by_period(period)?,
        ))
    }

    /// Returns a stable pillar block-data public view by finalized pillar period.
    pub fn consensus_query_pillar_block_data_by_period(
        &self,
        period: u64,
    ) -> Result<rustaxa_ffi::PillarBlockDataView, anyhow::Error> {
        Ok(pillar_block_data_view_to_ffi(
            self.0.pillar_block_data_by_period(period)?,
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
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlpOwned;
    use rustaxa_types::final_chain::StoredFinalChainBlockHeader;
    use rustaxa_types::pillar::{
        encode_optimized_pillar_votes_bundle_rlp, PillarBlock, PillarVote, ValidatorVoteCountChange,
    };

    fn period_data_rlp(pbft_block_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(pbft_block_rlp, 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn period_data_rlp_with_dag_bundle(pbft_block_rlp: &[u8]) -> Vec<u8> {
        let mut dag_bundle = RlpStream::new_list(3);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(0);
        dag_bundle.begin_list(0);

        let mut stream = RlpStream::new_list(5);
        stream.append_raw(pbft_block_rlp, 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&dag_bundle.out(), 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn period_data_with_pillar_votes_rlp(votes_bundle_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(votes_bundle_rlp, 1);
        stream.out().to_vec()
    }

    fn signature(value: u8) -> [u8; 65] {
        let mut signature = [value; 65];
        signature[64] = value & 1;
        signature
    }

    fn signed_pbft_block_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let mut extra_data = RlpStream::new_list(6);
        extra_data.append(&1u16);
        extra_data.append(&2u16);
        extra_data.append(&3u16);
        extra_data.append(&4u16);
        extra_data.append(&"rustaxa-test".to_string());
        extra_data.append(&H256::from_low_u64_be(55).as_bytes());
        let extra_data_bytes = extra_data.out().to_vec();

        let mut unsigned = RlpStream::new_list(8);
        unsigned.append(&H256::from_low_u64_be(10));
        unsigned.append(&H256::from_low_u64_be(11));
        unsigned.append(&H256::from_low_u64_be(12));
        unsigned.append(&H256::from_low_u64_be(13));
        unsigned.append(&7u64);
        unsigned.append(&99u64);
        unsigned.append_list(&[H256::from_low_u64_be(20), H256::from_low_u64_be(21)]);
        unsigned.append(&extra_data_bytes);
        let message_hash = keccak256(&unsigned.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed = RlpStream::new_list(9);
        signed.append(&H256::from_low_u64_be(10));
        signed.append(&H256::from_low_u64_be(11));
        signed.append(&H256::from_low_u64_be(12));
        signed.append(&H256::from_low_u64_be(13));
        signed.append(&7u64);
        signed.append(&99u64);
        signed.append_list(&[H256::from_low_u64_be(20), H256::from_low_u64_be(21)]);
        signed.append(&extra_data_bytes);
        signed.append(&signature_bytes);
        signed.out().to_vec()
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
    fn bridge_consensus_query_api_reads_pbft_schedule_block_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_schedule_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key);

        storage
            .save_period_data(7, period_data_rlp_with_dag_bundle(&pbft_block))
            .unwrap();

        let view = api
            .consensus_query_pbft_schedule_block_by_period(7)
            .unwrap();
        assert!(view.found);
        assert_eq!(view.prev_block_hash, H256::from_low_u64_be(10).0);
        assert_eq!(view.dag_block_hash_as_pivot, H256::from_low_u64_be(11).0);
        assert_eq!(view.period, 7);
        assert_eq!(view.timestamp, 99);
        assert_eq!(view.block_hash, keccak256(&pbft_block).0);
        assert_eq!(view.signature.len(), 65);
        assert_eq!(view.reward_votes.len(), 2);
        assert_eq!(view.reward_votes[0].hash, H256::from_low_u64_be(20).0);
        assert!(view.has_extra_data);
        assert_eq!(view.extra_data.major_version, 1);
        assert_eq!(view.extra_data.node_implementation, "rustaxa-test");
        assert!(view.dag_blocks_order.is_empty());

        assert!(
            !api.consensus_query_pbft_schedule_block_by_period(8)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_pbft_node_version_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_node_version_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key);

        storage
            .save_period_data(7, period_data_rlp(&pbft_block))
            .unwrap();

        let view = api.consensus_query_pbft_node_version_by_period(7).unwrap();
        assert!(view.found);
        assert_eq!(
            view.beneficiary,
            rustaxa_types::PbftBlockMetadata::try_from(
                rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(&pbft_block)
            )
            .unwrap()
            .author
            .0
        );
        assert_eq!(view.major_version, 1);
        assert_eq!(view.minor_version, 2);
        assert_eq!(view.patch_version, 3);

        assert!(
            !api.consensus_query_pbft_node_version_by_period(8)
                .unwrap()
                .found
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_consensus_query_api_reads_pillar_block_data_view() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rustaxa_bridge_consensus_query_api_pillar_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let storage =
            crate::storage::create_storage(temp_dir.to_str().expect("utf8 temp path")).unwrap();
        let api = create_consensus_query_api(&storage);
        let pillar_storage = crate::pillar_chain::create_pillar_chain_storage(&storage);
        let block = PillarBlock {
            period: 10,
            state_root: H256::from_low_u64_be(0x10),
            previous_pillar_block_hash: H256::from_low_u64_be(0x11),
            bridge_root: H256::from_low_u64_be(0x12),
            epoch: 13,
            validator_vote_count_changes: vec![ValidatorVoteCountChange {
                address: H160::from([0x14; 20]),
                vote_count_change: -7,
            }],
        };
        let vote = PillarVote {
            period: 11,
            block_hash: block.hash(),
            signature: signature(0x21),
        };
        let votes_bundle_rlp =
            encode_optimized_pillar_votes_bundle_rlp(std::slice::from_ref(&vote)).unwrap();

        pillar_storage
            .pillar_chain_storage_apply_finalized_block(10, block.encode_rlp())
            .unwrap();
        storage
            .save_period_data(11, period_data_with_pillar_votes_rlp(&votes_bundle_rlp))
            .unwrap();

        let view = api.consensus_query_pillar_block_data_by_period(10).unwrap();
        assert!(view.found);
        assert_eq!(view.pbft_period, 10);
        assert_eq!(view.state_root, H256::from_low_u64_be(0x10).0);
        assert_eq!(
            view.previous_pillar_block_hash,
            H256::from_low_u64_be(0x11).0
        );
        assert_eq!(view.bridge_root, H256::from_low_u64_be(0x12).0);
        assert_eq!(view.epoch, 13);
        assert_eq!(view.block_hash, <[u8; 32]>::from(block.hash()));
        assert_eq!(view.validator_vote_count_changes.len(), 1);
        assert_eq!(
            view.validator_vote_count_changes[0].address,
            H160::from([0x14; 20]).0
        );
        assert_eq!(view.validator_vote_count_changes[0].vote_count_change, -7);
        assert_eq!(view.signatures.len(), 1);
        let (r, vs) = vote.compact_signature_words();
        assert_eq!(view.signatures[0].r, r);
        assert_eq!(view.signatures[0].vs, vs);

        assert!(
            !api.consensus_query_pillar_block_data_by_period(12)
                .unwrap()
                .found
        );

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
