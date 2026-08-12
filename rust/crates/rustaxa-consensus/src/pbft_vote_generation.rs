//! Native PBFT vote generation and FinalChain lookup fact types.
//!
//! This module is composition-first by design: FinalChain-facing fact queries are
//! represented as typed request/result values and resolved in `PbftService` at the
//! service boundary. Rust-only generation remains deterministic byte/fact
//! construction with no live vote object, storage write, or replay-cache mutation.

use anyhow::{Context, Result, anyhow};
use ethereum_types::{H160, H256};
use k256::ecdsa::SigningKey;
use rlp::RlpStream;
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_vdf::vrf::{
    self, VRF_OUTPUT_BYTES, VRF_PROOF_BYTES, VRF_PUBLIC_KEY_BYTES, VRF_SECRET_KEY_BYTES,
};

use crate::pbft_vote_validation::{
    calculate_pbft_vote_weight, keccak256, legacy_pbft_vote_signed_hash,
    legacy_pbft_vote_signing_hash, legacy_vrf_message_rlp, pbft_vote_sortition_threshold,
};
use crate::verified_votes::PbftVoteType;

/// Status for a PBFT FinalChain fact query.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PbftFinalChainFact<T> {
    /// Query resolved and returned typed payload data.
    Ready(T),
    /// Query could not be resolved because the period is future or the backing
    /// FinalChain snapshot is unavailable/corrupt.
    Unavailable {
        /// Stable diagnostic code for unavailable data and errors.
        error_code: String,
    },
}

impl<T> PbftFinalChainFact<T> {
    /// Stable bridge-facing status byte code.
    pub const fn as_u8(&self) -> u8 {
        match self {
            Self::Ready(_) => 0,
            Self::Unavailable { .. } => 1,
        }
    }

    /// Reports whether the fact contains ready data.
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// Reports whether the fact carries an unavailable-state diagnostic.
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }

    /// Returns the unavailable diagnostic, or an empty string for ready data.
    pub fn error_code(&self) -> &str {
        match self {
            Self::Ready(_) => "",
            Self::Unavailable { error_code } => error_code,
        }
    }
}

impl PbftFinalChainFact<u64> {
    /// Projects the ready value or the legacy compatibility zero sentinel.
    pub const fn data_or_zero(&self) -> u64 {
        match self {
            Self::Ready(value) => *value,
            Self::Unavailable { .. } => 0,
        }
    }
}

/// Request to collect total PBFT DPoS vote-count facts for one consensus period.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposTotalVoteCountRequest {
    /// PBFT-consensus period to query.
    pub period: u64,
}

/// Response for one-period PBFT DPoS total-vote fact lookup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposTotalVoteCountFacts {
    /// Last finalized block number sampled before processing this query.
    ///
    /// This is a diagnostic snapshot and is not atomically tied to all sub-reads
    /// for the requested period.
    pub last_block_number: FinalChainBlockNumber,
    /// Typed outcome for downstream compatibility encoding.
    pub status: PbftFinalChainFact<u64>,
}

/// One-wallet address fact for batch FinalChain DPoS checks.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposAddressVoteFact {
    /// Wallet address under inspection.
    pub address: H160,
    /// Typed status for per-wallet fact availability.
    pub status: PbftFinalChainFact<u64>,
}

impl PbftFinalChainDposAddressVoteFact {
    /// Derives eligibility from a ready nonzero vote count.
    pub const fn is_eligible(&self) -> bool {
        match &self.status {
            PbftFinalChainFact::Ready(vote_count) => *vote_count > 0,
            PbftFinalChainFact::Unavailable { .. } => false,
        }
    }

    /// Projects the ready vote count or the legacy compatibility zero sentinel.
    pub const fn vote_count(&self) -> u64 {
        self.status.data_or_zero()
    }
}

/// Request to collect an ordered subset aggregate DPoS vote-count fact.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposWalletAggregateVoteCountRequest {
    /// PBFT-consensus period to query.
    pub period: u64,
    /// Current eligible-wallet period observed by PBFT manager runtime for this request.
    ///
    /// The aggregate is only read when this value equals `period`, ensuring PBFT
    /// can short-circuit on a deterministic not-ready boundary before any FinalChain
    /// address lookup.
    pub eligible_wallet_period: u64,
    /// Ordered wallet subset for aggregate sum.
    pub addresses: Vec<H160>,
}

/// Response for one-period PBFT DPoS wallet aggregate fact lookup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposWalletAggregateVoteCountFacts {
    /// Last finalized block number sampled before processing this query.
    ///
    /// This is a diagnostic snapshot and is not atomically tied to all sub-reads
    /// for the requested wallet subset.
    pub last_block_number: FinalChainBlockNumber,
    /// Typed outcome for downstream compatibility encoding.
    pub status: PbftFinalChainFact<u64>,
    /// Whether `eligible_wallet_period` in the request matched `period`.
    ///
    /// The aggregate call is only valid when this is true. A false value is a
    /// stable boundary outcome and still returns `status = Unavailable`.
    pub eligible_wallet_period_ready: bool,
}

/// Request for a single-wallet DPoS eligibility query.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposWalletEligibilityRequest {
    /// PBFT-consensus period to query.
    pub period: u64,
    /// Wallet address to test for eligibility.
    pub address: H160,
}

/// Response for one-wallet DPoS eligibility query.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposWalletEligibilityFacts {
    /// Last finalized block number sampled before processing this query.
    ///
    /// This is a diagnostic snapshot and is not atomically tied to all sub-reads
    /// for the requested address.
    pub last_block_number: FinalChainBlockNumber,
    /// Echoed wallet address.
    pub address: H160,
    /// Typed outcome for downstream compatibility encoding.
    pub status: PbftFinalChainFact<u64>,
}

impl PbftFinalChainDposWalletEligibilityFacts {
    /// Derives eligibility from a ready nonzero vote count.
    pub const fn is_eligible(&self) -> bool {
        match &self.status {
            PbftFinalChainFact::Ready(vote_count) => *vote_count > 0,
            PbftFinalChainFact::Unavailable { .. } => false,
        }
    }

    /// Projects the ready vote count or the legacy compatibility zero sentinel.
    pub const fn vote_count(&self) -> u64 {
        self.status.data_or_zero()
    }
}

/// Request for ordered multi-wallet DPoS eligibility lookup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposWalletEligibilityBatchRequest {
    /// PBFT-consensus period to query.
    pub period: u64,
    /// Ordered list of wallet addresses.
    pub addresses: Vec<H160>,
}

/// Response for ordered multi-wallet DPoS eligibility lookup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalChainDposWalletEligibilityBatchFacts {
    /// Last finalized block number sampled before processing this query.
    ///
    /// This is a diagnostic snapshot and is not atomically tied to all sub-reads
    /// for the requested addresses.
    pub last_block_number: FinalChainBlockNumber,
    /// Typed top-level batch outcome.
    pub status: PbftFinalChainFact<()>,
    /// Per-address facts preserving request order.
    pub address_facts: Vec<PbftFinalChainDposAddressVoteFact>,
}

/// Status for a local PBFT vote generation attempt.
///
/// Statuses are returned in the generation result instead of bridge errors for
/// expected consensus outcomes such as zero local stake or zero sortition
/// weight. Bridge/internal errors remain reserved for malformed call shapes.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteGenerationStatus {
    /// The vote payload was generated and accepted.
    Generated,
    /// The supplied vote-type value or type/step pairing is invalid.
    InvalidVoteType,
    /// The node secret does not derive the expected voter address.
    NodeSecretMismatch,
    /// The VRF secret does not derive the expected VRF public key.
    VrfSecretMismatch,
    /// Weighted generation was requested with zero voter stake.
    ZeroStake,
    /// Weighted generation was requested with zero total DPoS votes.
    ZeroTotalDpos,
    /// Weighted generation completed, but the generated sortition weight is zero.
    ZeroWeight,
}

impl PbftVoteGenerationStatus {
    /// Stable numeric status used by the CXX bridge.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Generated => 0,
            Self::InvalidVoteType => 1,
            Self::NodeSecretMismatch => 2,
            Self::VrfSecretMismatch => 3,
            Self::ZeroStake => 4,
            Self::ZeroTotalDpos => 5,
            Self::ZeroWeight => 6,
        }
    }
}

/// Caller-supplied facts for local PBFT vote generation.
///
/// Inputs:
/// - `block_hash`, `vote_type`, `period`, `round`, and `step` define the vote.
/// - `node_secret` signs `PbftVote::sha3(false)`.
/// - `vrf_secret` creates the PBFT sortition proof for `[period, round, step]`.
/// - Expected voter/VRF public facts let Rust catch wallet mismatch before
///   returning a payload to the C++ sidecar layer.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteGenerationInput {
    pub block_hash: H256,
    pub vote_type: PbftVoteType,
    pub period: u64,
    pub round: u64,
    pub step: u64,
    pub node_secret: [u8; 32],
    pub vrf_secret: [u8; VRF_SECRET_KEY_BYTES],
    pub expected_voter: H160,
    pub expected_vrf_public_key: [u8; VRF_PUBLIC_KEY_BYTES],
}

/// Optional DPoS facts required when generating a weighted storage payload.
///
/// The facts are read by the caller from FinalChain. Rust uses them only to
/// choose the legacy sortition threshold and calculate the embedded vote
/// weight.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteWeightFacts {
    pub voter_dpos_vote_count: u64,
    pub total_dpos_vote_count: u64,
    pub committee_size: u64,
    pub number_of_proposers: u64,
}

/// Canonical Rust-generated PBFT vote payload and derived facts.
///
/// `vote_rlp` is `PbftVote::rlp(true, false)` for unweighted generation and
/// `PbftVote::rlp(true, true)` for weighted generation. `vote_hash` always
/// identifies the unweighted signed vote, matching legacy C++ hash semantics.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftGeneratedVote {
    pub status: PbftVoteGenerationStatus,
    pub error_code: &'static str,
    pub accepted: bool,
    pub vote_hash: H256,
    pub signing_hash: H256,
    pub block_hash: H256,
    pub voter: H160,
    pub voter_public_key: [u8; 64],
    pub vrf_public_key: [u8; VRF_PUBLIC_KEY_BYTES],
    pub vrf_proof: [u8; VRF_PROOF_BYTES],
    pub vrf_output: [u8; VRF_OUTPUT_BYTES],
    pub period: u64,
    pub round: u64,
    pub step: u64,
    pub vote_type: PbftVoteType,
    pub has_weight: bool,
    pub weight: u64,
    pub vote_rlp: Vec<u8>,
}

/// Generates a signed canonical PBFT vote without embedded weight.
///
/// This is side-effect free. Invalid consensus inputs are returned as
/// non-accepted statuses; malformed cryptographic secret shapes are bridge
/// errors because they indicate caller misuse of the API.
pub fn generate_pbft_vote(input: PbftVoteGenerationInput) -> Result<PbftGeneratedVote> {
    generate_pbft_vote_inner(input, None)
}

/// Generates a signed PBFT vote and embeds the legacy calculated weight.
///
/// The returned `vote_rlp` contains the fourth weight field only when the
/// generation status is `Generated`. Zero stake, zero total DPoS votes, and
/// zero sortition weight return rejected statuses with empty payload bytes.
pub fn generate_pbft_vote_with_weight(
    input: PbftVoteGenerationInput,
    facts: PbftVoteWeightFacts,
) -> Result<PbftGeneratedVote> {
    generate_pbft_vote_inner(input, Some(facts))
}

fn generate_pbft_vote_inner(
    input: PbftVoteGenerationInput,
    weight_facts: Option<PbftVoteWeightFacts>,
) -> Result<PbftGeneratedVote> {
    if input.vote_type == PbftVoteType::Invalid
        || input.vote_type != pbft_vote_type_from_step(input.step)
    {
        return Ok(rejected(
            PbftVoteGenerationStatus::InvalidVoteType,
            "PBFT_VOTE_GENERATION_INVALID_VOTE_TYPE",
            input,
        ));
    }

    let signing_key = SigningKey::from_slice(&input.node_secret)
        .context("PBFT vote generation node secret is not a valid secp256k1 secret")?;
    let voter_public_key = public_key_from_signing_key(&signing_key);
    let voter = address_from_public_key(&voter_public_key);
    if voter != input.expected_voter {
        return Ok(rejected(
            PbftVoteGenerationStatus::NodeSecretMismatch,
            "PBFT_VOTE_GENERATION_NODE_SECRET_MISMATCH",
            input,
        ));
    }

    let vrf_public_key = vrf::public_key_from_secret(&input.vrf_secret)?;
    if vrf_public_key != input.expected_vrf_public_key {
        return Ok(rejected(
            PbftVoteGenerationStatus::VrfSecretMismatch,
            "PBFT_VOTE_GENERATION_VRF_SECRET_MISMATCH",
            input,
        ));
    }

    let vrf_message = legacy_vrf_message_rlp(input.period, input.round, input.step);
    let vrf_proof = vrf::prove(&input.vrf_secret, &vrf_message)?;
    let vrf_output = vrf::verify_output(&vrf_public_key, &vrf_proof, &vrf_message)?
        .ok_or_else(|| anyhow!("PBFT vote generation created an unverifiable VRF proof"))?;
    let vrf_sortition_rlp =
        legacy_vrf_sortition_rlp(input.period, input.round, input.step, &vrf_proof);
    let signing_hash = legacy_pbft_vote_signing_hash(input.block_hash, &vrf_sortition_rlp);
    let signature = sign_hash(&signing_key, signing_hash)?;
    let vote_hash = legacy_pbft_vote_signed_hash(input.block_hash, &vrf_sortition_rlp, &signature);

    let (has_weight, weight) = if let Some(facts) = weight_facts {
        if facts.voter_dpos_vote_count == 0 {
            return Ok(rejected_with_facts(
                PbftVoteGenerationStatus::ZeroStake,
                "PBFT_VOTE_GENERATION_ZERO_STAKE",
                input,
                voter,
                voter_public_key,
                vrf_public_key,
                vrf_proof,
                vrf_output,
                signing_hash,
                vote_hash,
            ));
        }
        if facts.total_dpos_vote_count == 0 {
            return Ok(rejected_with_facts(
                PbftVoteGenerationStatus::ZeroTotalDpos,
                "PBFT_VOTE_GENERATION_ZERO_TOTAL_DPOS",
                input,
                voter,
                voter_public_key,
                vrf_public_key,
                vrf_proof,
                vrf_output,
                signing_hash,
                vote_hash,
            ));
        }

        let threshold = pbft_vote_sortition_threshold(
            facts.total_dpos_vote_count,
            input.vote_type,
            facts.committee_size,
            facts.number_of_proposers,
        )?;
        let weight = calculate_pbft_vote_weight(
            facts.voter_dpos_vote_count,
            facts.total_dpos_vote_count,
            threshold,
            &vrf_output,
            &voter_public_key,
        )?;
        if weight == 0 {
            return Ok(rejected_with_facts(
                PbftVoteGenerationStatus::ZeroWeight,
                "PBFT_VOTE_GENERATION_ZERO_WEIGHT",
                input,
                voter,
                voter_public_key,
                vrf_public_key,
                vrf_proof,
                vrf_output,
                signing_hash,
                vote_hash,
            ));
        }
        (true, weight)
    } else {
        (false, 0)
    };

    Ok(PbftGeneratedVote {
        status: PbftVoteGenerationStatus::Generated,
        error_code: "",
        accepted: true,
        vote_hash,
        signing_hash,
        block_hash: input.block_hash,
        voter,
        voter_public_key,
        vrf_public_key,
        vrf_proof,
        vrf_output,
        period: input.period,
        round: input.round,
        step: input.step,
        vote_type: input.vote_type,
        has_weight,
        weight,
        vote_rlp: legacy_pbft_vote_rlp(
            input.block_hash,
            &vrf_sortition_rlp,
            &signature,
            has_weight,
            weight,
        ),
    })
}

fn rejected(
    status: PbftVoteGenerationStatus,
    error_code: &'static str,
    input: PbftVoteGenerationInput,
) -> PbftGeneratedVote {
    rejected_with_facts(
        status,
        error_code,
        input,
        H160::zero(),
        [0; 64],
        [0; VRF_PUBLIC_KEY_BYTES],
        [0; VRF_PROOF_BYTES],
        [0; VRF_OUTPUT_BYTES],
        H256::zero(),
        H256::zero(),
    )
}

#[allow(clippy::too_many_arguments)]
fn rejected_with_facts(
    status: PbftVoteGenerationStatus,
    error_code: &'static str,
    input: PbftVoteGenerationInput,
    voter: H160,
    voter_public_key: [u8; 64],
    vrf_public_key: [u8; VRF_PUBLIC_KEY_BYTES],
    vrf_proof: [u8; VRF_PROOF_BYTES],
    vrf_output: [u8; VRF_OUTPUT_BYTES],
    signing_hash: H256,
    vote_hash: H256,
) -> PbftGeneratedVote {
    PbftGeneratedVote {
        status,
        error_code,
        accepted: false,
        vote_hash,
        signing_hash,
        block_hash: input.block_hash,
        voter,
        voter_public_key,
        vrf_public_key,
        vrf_proof,
        vrf_output,
        period: input.period,
        round: input.round,
        step: input.step,
        vote_type: input.vote_type,
        has_weight: false,
        weight: 0,
        vote_rlp: Vec::new(),
    }
}

fn legacy_vrf_sortition_rlp(
    period: u64,
    round: u64,
    step: u64,
    proof: &[u8; VRF_PROOF_BYTES],
) -> Vec<u8> {
    let mut stream = RlpStream::new_list(4);
    stream.append(&period);
    stream.append(&round);
    stream.append(&step);
    stream.append(&proof.as_slice());
    stream.out().to_vec()
}

fn legacy_pbft_vote_rlp(
    block_hash: H256,
    vrf_sortition_rlp: &[u8],
    signature: &[u8; 65],
    has_weight: bool,
    weight: u64,
) -> Vec<u8> {
    let mut stream = RlpStream::new_list(if has_weight { 4 } else { 3 });
    stream.append(&block_hash);
    stream.append(&vrf_sortition_rlp);
    stream.append(&signature.as_slice());
    if has_weight {
        stream.append(&weight);
    }
    stream.out().to_vec()
}

fn sign_hash(signing_key: &SigningKey, signing_hash: H256) -> Result<[u8; 65]> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(signing_hash.as_bytes())
        .context("PBFT vote generation failed to sign vote hash")?;
    let signature_bytes = signature.to_bytes();
    let mut out = [0_u8; 65];
    out[..64].copy_from_slice(&signature_bytes);
    out[64] = recovery_id.to_byte();
    Ok(out)
}

fn public_key_from_signing_key(signing_key: &SigningKey) -> [u8; 64] {
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let mut out = [0_u8; 64];
    out.copy_from_slice(&encoded.as_bytes()[1..]);
    out
}

fn address_from_public_key(public_key: &[u8; 64]) -> H160 {
    let public_key_hash = keccak256(public_key);
    H160::from_slice(&public_key_hash.as_bytes()[12..])
}

fn pbft_vote_type_from_step(step: u64) -> PbftVoteType {
    match step {
        0 => PbftVoteType::Invalid,
        1 => PbftVoteType::Propose,
        2 => PbftVoteType::Soft,
        3 => PbftVoteType::Cert,
        _ => PbftVoteType::Next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_validation::{
        PbftCanonicalVoteInspectionStatus, PbftVoteValidationExternalFacts,
        inspect_canonical_pbft_vote, validate_canonical_pbft_vote,
    };

    const NODE_SECRET: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn input(vote_type: PbftVoteType, step: u64) -> PbftVoteGenerationInput {
        let signing_key = SigningKey::from_slice(&NODE_SECRET).unwrap();
        let public_key = public_key_from_signing_key(&signing_key);
        PbftVoteGenerationInput {
            block_hash: H256::from_low_u64_be(0xfeed),
            vote_type,
            period: 11,
            round: 2,
            step,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: address_from_public_key(&public_key),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        }
    }

    #[test]
    fn generates_canonical_signed_pbft_vote_bytes() {
        let vote = generate_pbft_vote(input(PbftVoteType::Cert, 3)).unwrap();
        assert!(vote.accepted);
        assert!(!vote.has_weight);
        assert_eq!(vote.weight, 0);
        assert_eq!(vote.vote_hash, keccak256(&vote.vote_rlp));

        let inspection = inspect_canonical_pbft_vote(&vote.vote_rlp).unwrap();
        assert_eq!(inspection.status, PbftCanonicalVoteInspectionStatus::Valid);
        assert_eq!(inspection.vote_hash, vote.vote_hash);
        assert_eq!(inspection.signing_hash, vote.signing_hash);
        assert_eq!(inspection.recovered_voter, vote.voter);
        assert_eq!(inspection.vote_type, PbftVoteType::Cert);
    }

    #[test]
    fn generates_weighted_vote_bytes_that_validate() {
        let input = input(PbftVoteType::Propose, 1);
        let vote = generate_pbft_vote_with_weight(
            input.clone(),
            PbftVoteWeightFacts {
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

        let validation = validate_canonical_pbft_vote(
            &vote.vote_rlp,
            PbftVoteValidationExternalFacts {
                voter_dpos_ready: true,
                voter_dpos_vote_count: 100,
                total_dpos_ready: true,
                total_dpos_vote_count: 100,
                future_dpos_state: false,
                unknown_error: false,
                vrf_key_ready: true,
                has_vrf_key: true,
                vrf_public_key: input.expected_vrf_public_key,
                strict_vrf: true,
                committee_size: 50,
                number_of_proposers: 100,
                has_preverified_weight: false,
                preverified_weight: 0,
            },
        )
        .unwrap();
        assert!(validation.accepted);
        assert_eq!(validation.calculated_weight, vote.weight);
    }

    #[test]
    fn rejects_vote_type_step_mismatch() {
        let vote = generate_pbft_vote(input(PbftVoteType::Soft, 3)).unwrap();
        assert_eq!(vote.status, PbftVoteGenerationStatus::InvalidVoteType);
        assert!(!vote.accepted);
        assert!(vote.vote_rlp.is_empty());
    }

    #[test]
    fn rejects_zero_stake_weighted_generation() {
        let vote = generate_pbft_vote_with_weight(
            input(PbftVoteType::Cert, 3),
            PbftVoteWeightFacts {
                voter_dpos_vote_count: 0,
                total_dpos_vote_count: 100,
                committee_size: 50,
                number_of_proposers: 20,
            },
        )
        .unwrap();
        assert_eq!(vote.status, PbftVoteGenerationStatus::ZeroStake);
        assert!(!vote.accepted);
    }

    #[test]
    fn accepts_next_vote_for_steps_greater_than_three() {
        let vote = generate_pbft_vote(input(PbftVoteType::Next, 7)).unwrap();
        assert!(vote.accepted);
        assert_eq!(vote.vote_type, PbftVoteType::Next);
    }
}
