//! Legacy-compatible VDF/VRF sortition contract for C++ `VdfSortition`.
//!
//! This module owns the externally visible deterministic behavior: VRF proof
//! verification, vote normalization, VDF difficulty calculation, legacy RLP
//! payload shape, and Wesolowski proof validation with the legacy ASCII-hex
//! modulus. Invalid peer-controlled payloads are returned as status-coded data;
//! malformed caller contracts are returned as `anyhow` errors.

use crate::prover::{CancellationToken, WesolowskiProver};
use crate::vdf::{Solution, WesolowskiVdf};
use crate::verifier::WesolowskiVerifier;
use crate::vrf::{
    VRF_OUTPUT_BYTES, VRF_PROOF_BYTES, VRF_PUBLIC_KEY_BYTES, VRF_SECRET_KEY_BYTES,
    normalized_vote_count, proof_array, prove as prove_vrf, public_key_from_secret,
    threshold_from_output, verify_output,
};
use anyhow::{Result, bail, ensure};
use rlp::{Rlp, RlpStream};

/// Legacy VDF/VRF sortition verification succeeded.
pub const LEGACY_SORTITION_STATUS_VALID: u8 = 0;
/// Caller-provided inputs violate the legacy sortition contract.
pub const LEGACY_SORTITION_STATUS_INVALID_ARGUMENT: u8 = 1;
/// The VRF public key shape or value is invalid.
pub const LEGACY_SORTITION_STATUS_INVALID_VRF_PUBLIC_KEY: u8 = 2;
/// The VRF proof failed strict verification against the public key and input.
pub const LEGACY_SORTITION_STATUS_INVALID_VRF_PROOF: u8 = 3;
/// The encoded VDF difficulty does not match the VRF-derived expected difficulty.
pub const LEGACY_SORTITION_STATUS_DIFFICULTY_MISMATCH: u8 = 4;
/// The VDF proof/output pair failed Wesolowski verification.
pub const LEGACY_SORTITION_STATUS_INVALID_VDF_PROOF: u8 = 5;
/// The sortition RLP payload is malformed.
pub const LEGACY_SORTITION_STATUS_MALFORMED_RLP: u8 = 6;
/// VDF proving was cancelled.
pub const LEGACY_SORTITION_STATUS_PROVER_CANCELLED: u8 = 7;
/// Unexpected internal Rust error while handling the legacy sortition contract.
pub const LEGACY_SORTITION_STATUS_INTERNAL_ERROR: u8 = 255;

/// Legacy VDF modulus bytes used by C++ `VdfSortition`.
///
/// Compatibility note: C++ defines this as `dev::asBytes("<hex text>")`,
/// preserving ASCII hex characters instead of decoding them. This is
/// consensus-visible for existing VDF proofs, so the Rust compatibility layer
/// deliberately uses the exact same 256 bytes.
pub const LEGACY_ASCII_HEX_MODULUS: &[u8] =
    b"3d1055a514e17cce1290ccb5befb256b00b8aac664e39e754466fcd631004c9e23d16f23\
9aee2a207e5173a7ee8f90ee9ab9b6a745d27c6e850e7ca7332388dfef7e5bbe6267d1f7\
9f9330e44715b3f2066f903081836c1c83ca29126f8fdc5f5922bf3f9ddb4540171691ac\
cc1ef6a34b2a804a18159c89c39b16edee2ede35";

/// Runtime sortition parameters used by legacy `VdfSortition`.
///
/// Inputs mirror C++ `SortitionParams`: one VRF threshold and four VDF
/// difficulty/lambda fields. The struct is intentionally flat at the bridge
/// boundary and independent of higher-level consensus configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacySortitionParams {
    /// Upper VRF threshold used for stale difficulty bucketing.
    pub vrf_threshold_upper: u16,
    /// Minimum VDF difficulty for non-stale eligible proposals.
    pub vdf_difficulty_min: u16,
    /// Maximum VDF difficulty for non-stale eligible proposals.
    pub vdf_difficulty_max: u16,
    /// Difficulty used when the VRF threshold falls into the stale range.
    pub vdf_difficulty_stale: u16,
    /// Wesolowski hash-to-prime lambda bound.
    pub vdf_lambda_bound: u16,
}

/// Decoded legacy `VdfSortition::rlp()` payload.
///
/// Wire layout is exactly `[vrf_proof, vdf_proof, vdf_output, difficulty]`.
/// Proof and output byte vectors preserve their canonical RLP bytes so callers
/// do not lose leading-zero or empty-vector behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVdfSortition {
    pub vrf_proof: [u8; VRF_PROOF_BYTES],
    pub vdf_proof: Vec<u8>,
    pub vdf_output: Vec<u8>,
    pub difficulty: u16,
}

/// Verification result for legacy VDF/VRF sortition payloads.
///
/// Invalid peer-controlled proofs and difficulty mismatches are returned as
/// status data. Malformed caller contracts, such as invalid vote denominators,
/// are returned as errors by lower-level helpers and mapped by the bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVdfSortitionVerification {
    pub ok: bool,
    pub status: u8,
    pub vrf_output: [u8; VRF_OUTPUT_BYTES],
    pub vrf_threshold: u16,
    pub expected_difficulty: u16,
    pub actual_difficulty: u16,
}

/// Proving result for legacy VDF/VRF sortition payloads.
///
/// Outputs are byte-for-byte suitable for legacy C++ `VdfSortition::rlp()`
/// serialization and verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVdfSortitionProof {
    pub ok: bool,
    pub status: u8,
    pub vrf_proof: [u8; VRF_PROOF_BYTES],
    pub vrf_output: [u8; VRF_OUTPUT_BYTES],
    pub vrf_threshold: u16,
    pub difficulty: u16,
    pub vdf_proof: Vec<u8>,
    pub vdf_output: Vec<u8>,
}

/// Encodes a legacy sortition payload with the C++ field order.
pub fn encode_legacy_vdf_sortition(sortition: &LegacyVdfSortition) -> Vec<u8> {
    let mut stream = RlpStream::new_list(4);
    stream.append(&&sortition.vrf_proof[..]);
    stream.append(&sortition.vdf_proof);
    stream.append(&sortition.vdf_output);
    stream.append(&sortition.difficulty);
    stream.out().to_vec()
}

/// Decodes legacy `VdfSortition::rlp()` bytes.
///
/// Malformed field count, VRF proof size, or RLP value shape is reported as an
/// error so the bridge can map it to `MALFORMED_RLP`.
pub fn decode_legacy_vdf_sortition(payload: &[u8]) -> Result<LegacyVdfSortition> {
    const FIELD_COUNT: usize = 4;

    let rlp = Rlp::new(payload);
    ensure!(
        rlp.item_count()? == FIELD_COUNT,
        "legacy VDF sortition must have four RLP fields"
    );
    Ok(LegacyVdfSortition {
        vrf_proof: proof_array(rlp.at(0)?.data()?)?,
        vdf_proof: rlp.val_at(1)?,
        vdf_output: rlp.val_at(2)?,
        difficulty: rlp.val_at(3)?,
    })
}

/// Calculates legacy VDF difficulty from sortition parameters and VRF threshold.
///
/// The arithmetic mirrors C++ `VdfSortition::calculateDifficulty`, including
/// integer division for bucket width.
pub fn calculate_legacy_difficulty(params: LegacySortitionParams, threshold: u16) -> Result<u16> {
    const THRESHOLD_CORRECTION: u32 = 10;

    ensure!(
        params.vdf_difficulty_max >= params.vdf_difficulty_min,
        "VDF difficulty max must be greater than or equal to min"
    );
    let number_of_difficulties =
        u32::from(params.vdf_difficulty_max - params.vdf_difficulty_min) + 1;
    let corrected_threshold = u32::from(threshold) * THRESHOLD_CORRECTION;
    if corrected_threshold >= u32::from(params.vrf_threshold_upper) {
        Ok(params.vdf_difficulty_stale)
    } else {
        let bucket_width = u32::from(params.vrf_threshold_upper) / number_of_difficulties;
        ensure!(
            bucket_width != 0,
            "VDF difficulty bucket width cannot be zero"
        );
        Ok(params.vdf_difficulty_min + (corrected_threshold / bucket_width) as u16)
    }
}

/// Verifies a legacy `VdfSortition::rlp()` payload.
///
/// Inputs:
/// - `params`: legacy sortition parameters for the proposal period.
/// - `public_key`: 32-byte VRF public key.
/// - `sortition_rlp`: legacy `[proof, sol1, sol2, difficulty]` payload.
/// - `vrf_input`: exact bytes originally signed by VRF.
/// - `vdf_input`: exact bytes used for Wesolowski proof generation.
/// - `vote_count` / `total_vote_count`: legacy vote normalization inputs.
///
/// Output:
/// - status-coded verification facts, preserving invalid proof/difficulty as
///   data rather than panicking or falling back to C++.
pub fn verify_legacy_vdf_sortition(
    params: LegacySortitionParams,
    public_key: &[u8; VRF_PUBLIC_KEY_BYTES],
    sortition_rlp: &[u8],
    vrf_input: &[u8],
    vdf_input: &[u8],
    vote_count: u64,
    total_vote_count: u64,
) -> Result<LegacyVdfSortitionVerification> {
    let sortition = match decode_legacy_vdf_sortition(sortition_rlp) {
        Ok(sortition) => sortition,
        Err(_) => {
            return Ok(verification_failure(
                LEGACY_SORTITION_STATUS_MALFORMED_RLP,
                0,
                0,
                0,
                [0_u8; VRF_OUTPUT_BYTES],
            ));
        }
    };
    let normalized_votes = normalized_vote_count(vote_count, total_vote_count)?;
    let Some(vrf_output) = verify_output(public_key, &sortition.vrf_proof, vrf_input)? else {
        return Ok(verification_failure(
            LEGACY_SORTITION_STATUS_INVALID_VRF_PROOF,
            0,
            0,
            sortition.difficulty,
            [0_u8; VRF_OUTPUT_BYTES],
        ));
    };
    let vrf_threshold = threshold_from_output(&vrf_output, normalized_votes);
    let expected_difficulty = calculate_legacy_difficulty(params, vrf_threshold)?;
    if sortition.difficulty != expected_difficulty {
        return Ok(verification_failure(
            LEGACY_SORTITION_STATUS_DIFFICULTY_MISMATCH,
            vrf_threshold,
            expected_difficulty,
            sortition.difficulty,
            vrf_output,
        ));
    }

    let vdf = WesolowskiVdf::new(
        u32::from(params.vdf_lambda_bound),
        u32::from(sortition.difficulty),
        vdf_input.to_vec(),
        LEGACY_ASCII_HEX_MODULUS.to_vec(),
    );
    let solution = Solution {
        first: sortition.vdf_proof,
        second: sortition.vdf_output,
    };
    if !WesolowskiVerifier::new(&vdf).verify(&solution) {
        return Ok(verification_failure(
            LEGACY_SORTITION_STATUS_INVALID_VDF_PROOF,
            vrf_threshold,
            expected_difficulty,
            sortition.difficulty,
            vrf_output,
        ));
    }

    Ok(LegacyVdfSortitionVerification {
        ok: true,
        status: LEGACY_SORTITION_STATUS_VALID,
        vrf_output,
        vrf_threshold,
        expected_difficulty,
        actual_difficulty: sortition.difficulty,
    })
}

/// Proves a legacy VDF/VRF sortition payload using the Rust primitives.
///
/// The returned proof bytes use the exact legacy RLP field values expected by
/// C++ `VdfSortition`, including the legacy ASCII-hex VDF modulus.
pub fn prove_legacy_vdf_sortition(
    params: LegacySortitionParams,
    secret_key: &[u8; VRF_SECRET_KEY_BYTES],
    vrf_input: &[u8],
    vdf_input: &[u8],
    vote_count: u64,
    total_vote_count: u64,
    cancellation_token: &CancellationToken,
) -> Result<LegacyVdfSortitionProof> {
    let normalized_votes = normalized_vote_count(vote_count, total_vote_count)?;
    let vrf_proof = prove_vrf(secret_key, vrf_input)?;
    let public_key = public_key_from_secret(secret_key)?;
    let vrf_output = verify_output(&public_key, &vrf_proof, vrf_input)?
        .ok_or_else(|| anyhow::anyhow!("VRF proof created by Rust did not verify"))?;
    let vrf_threshold = threshold_from_output(&vrf_output, normalized_votes);
    let difficulty = calculate_legacy_difficulty(params, vrf_threshold)?;

    let vdf = WesolowskiVdf::new(
        u32::from(params.vdf_lambda_bound),
        u32::from(difficulty),
        vdf_input.to_vec(),
        LEGACY_ASCII_HEX_MODULUS.to_vec(),
    );
    let solution = WesolowskiProver::new(&vdf).prove(cancellation_token);
    if solution.first.is_empty() && solution.second.is_empty() && cancellation_token.is_cancelled()
    {
        return Ok(LegacyVdfSortitionProof {
            ok: false,
            status: LEGACY_SORTITION_STATUS_PROVER_CANCELLED,
            vrf_proof,
            vrf_output,
            vrf_threshold,
            difficulty,
            vdf_proof: Vec::new(),
            vdf_output: Vec::new(),
        });
    }
    if solution.first.is_empty() || solution.second.is_empty() {
        bail!("VDF prover returned an empty proof or output");
    }

    Ok(LegacyVdfSortitionProof {
        ok: true,
        status: LEGACY_SORTITION_STATUS_VALID,
        vrf_proof,
        vrf_output,
        vrf_threshold,
        difficulty,
        vdf_proof: solution.first,
        vdf_output: solution.second,
    })
}

fn verification_failure(
    status: u8,
    vrf_threshold: u16,
    expected_difficulty: u16,
    actual_difficulty: u16,
    vrf_output: [u8; VRF_OUTPUT_BYTES],
) -> LegacyVdfSortitionVerification {
    LegacyVdfSortitionVerification {
        ok: false,
        status,
        vrf_output,
        vrf_threshold,
        expected_difficulty,
        actual_difficulty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_KEY: [u8; VRF_SECRET_KEY_BYTES] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn params() -> LegacySortitionParams {
        LegacySortitionParams {
            vrf_threshold_upper: 0x5ff,
            vdf_difficulty_min: 5,
            vdf_difficulty_max: 10,
            vdf_difficulty_stale: 9,
            vdf_lambda_bound: 64,
        }
    }

    #[test]
    fn legacy_sortition_roundtrip_proves_encodes_and_verifies() {
        let vrf_input = [0xA1, 0x02, 0x03];
        let vdf_input = [0xB1, 0x04];
        let public_key = public_key_from_secret(&SECRET_KEY).unwrap();
        let proof = prove_legacy_vdf_sortition(
            params(),
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(proof.ok);

        let encoded = encode_legacy_vdf_sortition(&LegacyVdfSortition {
            vrf_proof: proof.vrf_proof,
            vdf_proof: proof.vdf_proof,
            vdf_output: proof.vdf_output,
            difficulty: proof.difficulty,
        });
        let verification = verify_legacy_vdf_sortition(
            params(),
            &public_key,
            &encoded,
            &vrf_input,
            &vdf_input,
            1,
            1,
        )
        .unwrap();

        assert!(verification.ok);
        assert_eq!(verification.status, LEGACY_SORTITION_STATUS_VALID);
        assert_eq!(verification.expected_difficulty, proof.difficulty);
        assert_eq!(verification.vrf_threshold, proof.vrf_threshold);
        assert_eq!(verification.vrf_output, proof.vrf_output);
    }

    #[test]
    fn legacy_sortition_reports_invalid_vrf_and_vdf_as_status() {
        let vrf_input = [0xA1, 0x02, 0x03];
        let vdf_input = [0xB1, 0x04];
        let public_key = public_key_from_secret(&SECRET_KEY).unwrap();
        let proof = prove_legacy_vdf_sortition(
            params(),
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .unwrap();

        let encoded = encode_legacy_vdf_sortition(&LegacyVdfSortition {
            vrf_proof: proof.vrf_proof,
            vdf_proof: proof.vdf_proof.clone(),
            vdf_output: proof.vdf_output.clone(),
            difficulty: proof.difficulty,
        });
        let invalid_vrf = verify_legacy_vdf_sortition(
            params(),
            &public_key,
            &encoded,
            b"wrong",
            &vdf_input,
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            invalid_vrf.status,
            LEGACY_SORTITION_STATUS_INVALID_VRF_PROOF
        );

        let mut bad_output = proof.vdf_output;
        bad_output[0] = bad_output[0].wrapping_add(1);
        let encoded_bad_vdf = encode_legacy_vdf_sortition(&LegacyVdfSortition {
            vrf_proof: proof.vrf_proof,
            vdf_proof: proof.vdf_proof,
            vdf_output: bad_output,
            difficulty: proof.difficulty,
        });
        let invalid_vdf = verify_legacy_vdf_sortition(
            params(),
            &public_key,
            &encoded_bad_vdf,
            &vrf_input,
            &vdf_input,
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            invalid_vdf.status,
            LEGACY_SORTITION_STATUS_INVALID_VDF_PROOF
        );
    }
}
