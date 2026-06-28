//! CXX bridge wrappers for PBFT vote payload construction.
//!
//! The bridge exposes Rust-owned vote RLP builders to consensus shims while
//! preserving existing C++ public APIs. C++ still supplies live vote metadata,
//! storage handles, and slashing transaction submission; Rust owns the exact
//! weighted storage payloads, storage bundles, and normalized slashing evidence
//! bytes derived from canonical signed PBFT vote RLP.

use crate::ffi::rustaxa_ffi::PbftVoteStorageRecord;
use anyhow::Result;
use rustaxa_consensus::{
    build_weighted_pbft_vote_bundle, build_weighted_pbft_vote_payload, PbftVotePayloadRecord,
};

/// Builds a weighted PBFT vote storage record from canonical signed vote bytes.
///
/// Inputs are legacy `PbftVote::rlp(true, false)` bytes plus the authoritative
/// Rust-computed weight. The returned `vote_rlp` matches legacy
/// `PbftVote::rlp(true, true)` and is ready for Rust storage persistence.
pub fn pbft_vote_weighted_payload_from_canonical_vote(
    canonical_vote_rlp: &[u8],
    weight: u64,
) -> Result<PbftVoteStorageRecord> {
    Ok(build_weighted_pbft_vote_payload(canonical_vote_rlp, weight)?.into())
}

/// Builds a raw RLP list of weighted PBFT vote payload records.
///
/// The records must already be weighted storage records produced by Rust. The
/// returned bytes match the legacy latest-round and reward-vote bundle storage
/// shape: an RLP list whose children are appended as raw vote RLP items.
pub fn pbft_vote_bundle_payload_from_records(
    records: Vec<PbftVoteStorageRecord>,
) -> Result<Vec<u8>> {
    let records: Vec<_> = records.into_iter().map(Into::into).collect();
    build_weighted_pbft_vote_bundle(&records)
}

impl From<PbftVotePayloadRecord> for PbftVoteStorageRecord {
    fn from(record: PbftVotePayloadRecord) -> Self {
        Self {
            hash: record.hash.0,
            vote_rlp: record.vote_rlp,
        }
    }
}

impl From<PbftVoteStorageRecord> for PbftVotePayloadRecord {
    fn from(record: PbftVoteStorageRecord) -> Self {
        Self {
            hash: record.hash.into(),
            vote_rlp: record.vote_rlp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftVoteGenerationInput;
    use crate::pbft_vote_generation::pbft_generate_signed_vote;
    use k256::ecdsa::SigningKey;
    use rlp::Rlp;
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x23; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn canonical_vote(block_hash_byte: u8) -> Vec<u8> {
        pbft_generate_signed_vote(PbftVoteGenerationInput {
            block_hash: [block_hash_byte; 32],
            vote_type: 1,
            period: 17,
            round: 2,
            step: 1,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET),
            expected_vrf_public_key: rustaxa_vdf::vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
        .vote_rlp
    }

    #[test]
    fn bridge_builds_weighted_storage_payload() {
        let canonical = canonical_vote(0x11);
        let record = pbft_vote_weighted_payload_from_canonical_vote(&canonical, 44).unwrap();

        let decoded = Rlp::new(&record.vote_rlp);
        assert_eq!(decoded.item_count().unwrap(), 4);
        assert_eq!(decoded.val_at::<u64>(3).unwrap(), 44);
        assert_eq!(record.hash.len(), 32);
    }

    #[test]
    fn bridge_builds_bundle_from_weighted_records() {
        let first =
            pbft_vote_weighted_payload_from_canonical_vote(&canonical_vote(0x21), 12).unwrap();
        let second =
            pbft_vote_weighted_payload_from_canonical_vote(&canonical_vote(0x22), 13).unwrap();
        let first_rlp = first.vote_rlp.clone();
        let second_rlp = second.vote_rlp.clone();

        let bundle = pbft_vote_bundle_payload_from_records(vec![first, second]).unwrap();
        let decoded = Rlp::new(&bundle);

        assert_eq!(decoded.item_count().unwrap(), 2);
        assert_eq!(decoded.at(0).unwrap().as_raw(), first_rlp.as_slice());
        assert_eq!(decoded.at(1).unwrap().as_raw(), second_rlp.as_slice());
    }

    #[test]
    fn bridge_rejects_zero_weight_storage_payload() {
        let err = match pbft_vote_weighted_payload_from_canonical_vote(&canonical_vote(0x31), 0) {
            Ok(_) => panic!("zero-weight PBFT storage payload should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("non-zero weight"));
    }

    #[test]
    fn bridge_rejects_unweighted_records_in_bundle() {
        let err = pbft_vote_bundle_payload_from_records(vec![PbftVoteStorageRecord {
            hash: [0x44; 32],
            vote_rlp: canonical_vote(0x44),
        }])
        .unwrap_err();
        assert!(err.to_string().contains("include weight"));
    }
}
