//! PBFT vote payload construction for storage and slashing effects.
//!
//! This module owns legacy-compatible PBFT vote RLP construction for effects
//! that still execute outside Rust. Callers provide canonical signed vote bytes
//! plus explicit weights when persistence requires `PbftVote::rlp(true, true)`.
//! Rust validates and re-encodes the storage/slashing payloads without owning
//! live C++ `PbftVote` objects, storage batches, or slashing transaction
//! submission.

use anyhow::{Result, anyhow, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};

use crate::pbft_vote_validation::inspect_canonical_pbft_vote;

const SIGNATURE_BYTES: usize = 65;

/// Rust-built PBFT vote payload for a downstream executor.
///
/// `hash` is the canonical signed vote hash used as the legacy storage key and
/// verified-vote identity. `vote_rlp` is either the unweighted canonical signed
/// vote payload for slashing or the weighted storage payload when constructed
/// by [`build_weighted_pbft_vote_payload`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVotePayloadRecord {
    /// Canonical signed PBFT vote hash.
    pub hash: H256,
    /// Legacy PBFT vote RLP bytes for the requested executor.
    pub vote_rlp: Vec<u8>,
}

/// Builds the weighted PBFT vote storage payload from canonical signed bytes.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes. Inputs
///   that already contain a weight are accepted but re-encoded with `weight`.
/// - `weight`: authoritative Rust-calculated non-zero vote weight.
///
/// Outputs:
/// - `PbftVotePayloadRecord` whose `vote_rlp` matches
///   `PbftVote::rlp(true, true)` for persistence.
///
/// Invariants and edge behavior:
/// - The vote hash is derived from the canonical signed vote and never includes
///   the embedded storage weight.
/// - Malformed RLP, invalid signatures, or zero weight are rejected as errors
///   because this function is used after admission validation has succeeded.
pub fn build_weighted_pbft_vote_payload(
    canonical_vote_rlp: &[u8],
    weight: u64,
) -> Result<PbftVotePayloadRecord> {
    ensure!(
        weight > 0,
        "PBFT weighted vote payload requires non-zero weight"
    );
    let fields = decode_signed_vote_fields(canonical_vote_rlp)?;
    let inspection = inspect_canonical_pbft_vote(canonical_vote_rlp)?;
    ensure!(
        inspection.signature_valid,
        "PBFT weighted vote payload requires a valid signature"
    );

    let mut stream = RlpStream::new_list(4);
    stream.append(&fields.block_hash);
    stream.append(&fields.vrf_sortition_rlp);
    stream.append(&fields.signature.as_slice());
    stream.append(&weight);

    Ok(PbftVotePayloadRecord {
        hash: inspection.vote_hash,
        vote_rlp: stream.out().to_vec(),
    })
}

/// Builds the unweighted PBFT vote payload used by slashing calldata.
///
/// Inputs:
/// - `canonical_vote_rlp`: legacy signed PBFT vote bytes. Inputs with embedded
///   weight are accepted and normalized back to the unweighted signed payload.
///
/// Outputs:
/// - `PbftVotePayloadRecord` whose `vote_rlp` matches
///   `PbftVote::rlp(true, false)` and whose hash is the signed vote hash.
///
/// Edge behavior:
/// - Malformed RLP or invalid signatures are rejected because slashing proof
///   planning requires canonical, signed votes.
pub fn build_slashing_pbft_vote_payload(
    canonical_vote_rlp: &[u8],
) -> Result<PbftVotePayloadRecord> {
    let fields = decode_signed_vote_fields(canonical_vote_rlp)?;
    let inspection = inspect_canonical_pbft_vote(canonical_vote_rlp)?;
    ensure!(
        inspection.signature_valid,
        "PBFT slashing payload requires a valid signature"
    );

    let mut stream = RlpStream::new_list(3);
    stream.append(&fields.block_hash);
    stream.append(&fields.vrf_sortition_rlp);
    stream.append(&fields.signature.as_slice());

    Ok(PbftVotePayloadRecord {
        hash: inspection.vote_hash,
        vote_rlp: stream.out().to_vec(),
    })
}

/// Builds a legacy latest-round `2t+1` vote bundle from weighted vote records.
///
/// Inputs:
/// - `records`: weighted PBFT vote payload records in the exact order selected
///   by the verified-vote executor.
///
/// Outputs:
/// - RLP list where each item is appended as a raw weighted PBFT vote payload,
///   matching the C++ `RLPStream(votes.size()).appendRaw(vote->rlp(true,true))`
///   storage shape.
///
/// Edge behavior:
/// - Empty bundles are rejected because legacy persistence expects at least one
///   vote and derives metadata from the first record.
/// - Each record must decode as a four-field weighted PBFT vote payload.
pub fn build_weighted_pbft_vote_bundle(records: &[PbftVotePayloadRecord]) -> Result<Vec<u8>> {
    ensure!(
        !records.is_empty(),
        "PBFT 2t+1 vote bundle requires at least one vote"
    );

    let mut stream = RlpStream::new_list(records.len());
    for record in records {
        ensure_weighted_vote_payload(record)?;
        stream.append_raw(&record.vote_rlp, 1);
    }
    Ok(stream.out().to_vec())
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SignedVoteFields {
    block_hash: H256,
    vrf_sortition_rlp: Vec<u8>,
    signature: [u8; SIGNATURE_BYTES],
}

fn decode_signed_vote_fields(vote_rlp: &[u8]) -> Result<SignedVoteFields> {
    let vote = Rlp::new(vote_rlp);
    let item_count = vote.item_count()?;
    ensure!(
        item_count == 3 || item_count == 4,
        "PBFT vote payload must contain block_hash, vrf_sortition, signature and optional weight"
    );

    let block_hash: H256 = vote.val_at(0)?;
    let vrf_sortition_rlp = vote.val_at::<Vec<u8>>(1)?;
    let signature = vote.val_at::<Vec<u8>>(2)?;
    ensure!(
        signature.len() == SIGNATURE_BYTES,
        "PBFT vote payload signature must be exactly {SIGNATURE_BYTES} bytes"
    );
    let signature = signature
        .try_into()
        .map_err(|_| anyhow!("PBFT vote signature length checked above"))?;

    Ok(SignedVoteFields {
        block_hash,
        vrf_sortition_rlp,
        signature,
    })
}

fn ensure_weighted_vote_payload(record: &PbftVotePayloadRecord) -> Result<()> {
    let vote = Rlp::new(&record.vote_rlp);
    ensure!(
        vote.item_count()? == 4,
        "PBFT storage vote payload must include weight"
    );
    let inspection = inspect_canonical_pbft_vote(&record.vote_rlp)?;
    ensure!(
        inspection.vote_hash == record.hash,
        "PBFT storage vote payload hash mismatches record key"
    );
    ensure!(
        inspection.has_embedded_weight && inspection.embedded_weight > 0,
        "PBFT storage vote payload requires non-zero embedded weight"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_generation::{
        PbftVoteGenerationInput, PbftVoteGenerationStatus, generate_pbft_vote,
    };
    use crate::verified_votes::PbftVoteType;
    use k256::ecdsa::SigningKey;
    use rustaxa_vdf::vrf;
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x42; 32];
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

    fn vote_rlp(block_hash: [u8; 32]) -> Vec<u8> {
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type: PbftVoteType::Cert,
            period: 12,
            round: 2,
            step: 3,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        assert_eq!(generated.status, PbftVoteGenerationStatus::Generated);
        generated.vote_rlp
    }

    #[test]
    fn weighted_payload_preserves_vote_hash_and_embeds_weight() {
        let canonical = vote_rlp([7; 32]);
        let canonical_hash = inspect_canonical_pbft_vote(&canonical).unwrap().vote_hash;

        let record = build_weighted_pbft_vote_payload(&canonical, 42).unwrap();
        let weighted = inspect_canonical_pbft_vote(&record.vote_rlp).unwrap();

        assert_eq!(record.hash, canonical_hash);
        assert_eq!(weighted.vote_hash, canonical_hash);
        assert!(weighted.has_embedded_weight);
        assert_eq!(weighted.embedded_weight, 42);
    }

    #[test]
    fn slashing_payload_normalizes_away_embedded_weight() {
        let canonical = vote_rlp([8; 32]);
        let weighted = build_weighted_pbft_vote_payload(&canonical, 11).unwrap();

        let slashing = build_slashing_pbft_vote_payload(&weighted.vote_rlp).unwrap();
        let inspected = inspect_canonical_pbft_vote(&slashing.vote_rlp).unwrap();

        assert_eq!(slashing.hash, weighted.hash);
        assert!(!inspected.has_embedded_weight);
    }

    #[test]
    fn weighted_bundle_uses_raw_vote_items_in_order() {
        let first = build_weighted_pbft_vote_payload(&vote_rlp([1; 32]), 10).unwrap();
        let second = build_weighted_pbft_vote_payload(&vote_rlp([2; 32]), 20).unwrap();

        let bundle = build_weighted_pbft_vote_bundle(&[first.clone(), second.clone()]).unwrap();
        let decoded = Rlp::new(&bundle);

        assert_eq!(decoded.item_count().unwrap(), 2);
        assert_eq!(decoded.at(0).unwrap().as_raw(), first.vote_rlp.as_slice());
        assert_eq!(decoded.at(1).unwrap().as_raw(), second.vote_rlp.as_slice());
    }
}
