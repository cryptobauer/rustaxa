//! CXX bridge wrappers for Rust PBFT vote generation.
//!
//! The bridge exposes a byte-oriented generation contract to the C++
//! `VoteManager` shim. C++ still owns FinalChain reads, wallet selection, and
//! live `PbftVote` sidecars; Rust returns canonical signed vote bytes and
//! derived facts that can be parity-checked before production routing flips.

use crate::ffi::rustaxa_ffi::{
    PbftGeneratedVote as FfiPbftGeneratedVote,
    PbftVoteGenerationInput as FfiPbftVoteGenerationInput,
    PbftVoteWeightFacts as FfiPbftVoteWeightFacts,
};
use anyhow::Result;
use ethereum_types::{H160, H256};
use rustaxa_consensus::pbft_vote_generation::{
    generate_pbft_vote, generate_pbft_vote_with_weight, PbftGeneratedVote, PbftVoteGenerationInput,
    PbftVoteWeightFacts,
};
use rustaxa_consensus::verified_votes::PbftVoteType;

/// Generates one signed PBFT vote payload in Rust.
pub fn pbft_generate_signed_vote(
    input: FfiPbftVoteGenerationInput,
) -> Result<FfiPbftGeneratedVote> {
    Ok(generate_pbft_vote(input.try_into()?)?.into())
}

/// Generates one signed and weighted PBFT vote payload in Rust.
pub fn pbft_generate_signed_vote_with_weight(
    input: FfiPbftVoteGenerationInput,
    facts: FfiPbftVoteWeightFacts,
) -> Result<FfiPbftGeneratedVote> {
    Ok(generate_pbft_vote_with_weight(input.try_into()?, facts.into())?.into())
}

impl TryFrom<FfiPbftVoteGenerationInput> for PbftVoteGenerationInput {
    type Error = anyhow::Error;

    fn try_from(value: FfiPbftVoteGenerationInput) -> Result<Self> {
        Ok(Self {
            block_hash: H256::from(value.block_hash),
            vote_type: PbftVoteType::try_from(value.vote_type)?,
            period: value.period,
            round: value.round,
            step: value.step,
            node_secret: value.node_secret,
            vrf_secret: value.vrf_secret,
            expected_voter: H160::from(value.expected_voter),
            expected_vrf_public_key: value.expected_vrf_public_key,
        })
    }
}

impl From<FfiPbftVoteWeightFacts> for PbftVoteWeightFacts {
    fn from(value: FfiPbftVoteWeightFacts) -> Self {
        Self {
            voter_dpos_vote_count: value.voter_dpos_vote_count,
            total_dpos_vote_count: value.total_dpos_vote_count,
            committee_size: value.committee_size,
            number_of_proposers: value.number_of_proposers,
        }
    }
}

impl From<PbftGeneratedVote> for FfiPbftGeneratedVote {
    fn from(value: PbftGeneratedVote) -> Self {
        Self {
            status: value.status.as_u8(),
            error_code: value.error_code.to_owned(),
            accepted: value.accepted,
            vote_hash: value.vote_hash.into(),
            signing_hash: value.signing_hash.into(),
            block_hash: value.block_hash.into(),
            voter: value.voter.0,
            voter_public_key: value.voter_public_key,
            vrf_public_key: value.vrf_public_key,
            vrf_proof: value.vrf_proof,
            vrf_output: value.vrf_output,
            period: value.period,
            round: value.round,
            step: value.step,
            vote_type: value.vote_type.into(),
            has_weight: value.has_weight,
            weight: value.weight,
            vote_rlp: value.vote_rlp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    const NODE_SECRET: [u8; 32] = [0x24; 32];
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
        let mut hasher = tiny_keccak::Keccak::v256();
        tiny_keccak::Hasher::update(&mut hasher, &public_key.as_bytes()[1..]);
        tiny_keccak::Hasher::finalize(hasher, &mut output);
        output[12..].try_into().unwrap()
    }

    fn input(vote_type: u8, step: u64) -> FfiPbftVoteGenerationInput {
        FfiPbftVoteGenerationInput {
            block_hash: [0x11; 32],
            vote_type,
            period: 9,
            round: 1,
            step,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET),
            expected_vrf_public_key: rustaxa_vdf::vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        }
    }

    #[test]
    fn bridge_generates_signed_vote_bytes() {
        let vote = pbft_generate_signed_vote(input(3, 3)).unwrap();
        assert!(vote.accepted);
        assert!(!vote.has_weight);
        assert_eq!(vote.vote_type, 3);
        assert!(!vote.vote_rlp.is_empty());
    }

    #[test]
    fn bridge_generates_weighted_vote_bytes() {
        let vote = pbft_generate_signed_vote_with_weight(
            input(1, 1),
            FfiPbftVoteWeightFacts {
                voter_dpos_vote_count: 100,
                total_dpos_vote_count: 100,
                committee_size: 50,
                number_of_proposers: 100,
            },
        )
        .unwrap();
        assert!(vote.accepted);
        assert!(vote.has_weight);
        assert_eq!(vote.weight, 100);
    }
}
