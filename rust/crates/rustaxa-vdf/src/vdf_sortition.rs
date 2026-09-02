//! DAG-facing VDF sortition payload helpers.
//!
//! The types in this module expose a smaller standalone API for Rust callers
//! and bridge adapters that already have a VRF output and need to encode,
//! decode, or verify only the VDF portion of a legacy sortition payload. The
//! module deliberately keeps owned byte vectors at the boundary so adapters do
//! not borrow Rust internals across calls.

use crate::prover::{CancellationToken, WesolowskiProver};
use crate::sortition::{self, LEGACY_ASCII_HEX_MODULUS, LegacySortitionParams, LegacyVdfSortition};
use crate::vdf::WesolowskiVdf;
use crate::verifier::WesolowskiVerifier;
use crate::vrf::{
    VRF_OUTPUT_BYTES, VRF_PROOF_BYTES, normalized_vote_count,
    threshold_from_output as legacy_threshold_from_output,
};
use anyhow::{Result, ensure};

/// Result for VDF/VRF sortition verification in DAG-facing flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VdfSortitionVerifyResult {
    /// Verification status encoded as valid/invalid constants.
    pub vdf_status: u8,
    /// Difficulty stored in the submitted payload.
    pub difficulty: u16,
    /// Difficulty derived from vote output and parameters.
    pub expected_difficulty: u16,
}

/// Canonical payload used by VdfSortition verify/prove boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdfSortitionPayload {
    /// 80-byte VRF proof from the legacy payload.
    pub vrf_proof: [u8; VRF_PROOF_BYTES],
    /// Wesolowski proof bytes (`sol1` in legacy JSON/RLP naming).
    pub vdf_solution_proof: Vec<u8>,
    /// Wesolowski output bytes (`sol2` in legacy JSON/RLP naming).
    pub vdf_solution_output: Vec<u8>,
    /// VDF difficulty encoded in the payload.
    pub difficulty: u16,
}

/// Terminal result of one VDF-sortition proof attempt.
///
/// Cancellation is represented separately from operational failure so an
/// asynchronous owner can distinguish an expected superseded proposal from a
/// broken prover. Completed payloads preserve the legacy four-field wire
/// representation when passed to [`encode_vdf_sortition_payload`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdfSortitionProofOutcome {
    /// Proof and output were generated successfully.
    Completed(VdfSortitionPayload),
    /// The owned cancellation token was signalled during proof generation.
    Cancelled,
}

/// Runtime parameters for standalone VDF difficulty and modulus checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VdfSortitionVerifyConfig {
    /// Upper VRF threshold used for stale difficulty bucketing.
    pub threshold_upper: u16,
    /// Minimum VDF difficulty for non-stale eligible proposals.
    pub difficulty_min: u16,
    /// Maximum VDF difficulty for non-stale eligible proposals.
    pub difficulty_max: u16,
    /// Difficulty used when the VRF threshold falls into the stale range.
    pub difficulty_stale: u16,
    /// Wesolowski hash-to-prime lambda bound.
    pub lambda_bound: u16,
}

/// Result code when the VDF proof/output passes all checks.
pub const DAG_VERIFY_VDF_STATUS_VALID: u8 = 0;
/// Result code when the VDF proof/output fails a threshold or proof check.
pub const DAG_VERIFY_VDF_STATUS_INVALID: u8 = 1;

/// Legacy Taraxa wire encoding keeps the ASCII hex modulus bytes as bytes,
/// not decoded RSA modulus bytes.
pub fn legacy_vdf_modulus_ascii_hex() -> &'static [u8] {
    LEGACY_ASCII_HEX_MODULUS
}

/// Encodes a VdfSortition payload with legacy field order.
pub fn encode_vdf_sortition_payload(payload: &VdfSortitionPayload) -> Vec<u8> {
    sortition::encode_legacy_vdf_sortition(&LegacyVdfSortition {
        vrf_proof: payload.vrf_proof,
        vdf_proof: payload.vdf_solution_proof.clone(),
        vdf_output: payload.vdf_solution_output.clone(),
        difficulty: payload.difficulty,
    })
}

/// Decodes a legacy VdfSortition payload while preserving wire shape checks.
pub fn decode_vdf_sortition_payload(payload: &[u8]) -> Result<VdfSortitionPayload> {
    sortition::decode_legacy_vdf_sortition(payload).map(|payload| VdfSortitionPayload {
        vrf_proof: payload.vrf_proof,
        vdf_solution_proof: payload.vdf_proof,
        vdf_solution_output: payload.vdf_output,
        difficulty: payload.difficulty,
    })
}

/// Converts a VRF output into legacy threshold using the same vote normalization.
pub fn threshold_from_vrf_output(vrf_output: &[u8], vote_count: u16) -> Result<u16> {
    let output = vrf_output
        .try_into()
        .map_err(|_| anyhow::anyhow!("VRF output must be {VRF_OUTPUT_BYTES} bytes"))?;
    Ok(legacy_threshold_from_output(&output, vote_count))
}

/// Normalizes vote counts before threshold conversion.
pub fn normalize_vote_count(sender_eligible_vote_count: u64, max_vote_count: u64) -> Result<u16> {
    normalized_vote_count(sender_eligible_vote_count, max_vote_count)
}

/// Recomputes expected VDF difficulty from runtime parameters and VRF threshold.
pub fn calculate_vdf_sortition_difficulty(
    config: VdfSortitionVerifyConfig,
    threshold: u16,
) -> Result<u16> {
    let legacy_config = LegacySortitionParams {
        vrf_threshold_upper: config.threshold_upper,
        vdf_difficulty_min: config.difficulty_min,
        vdf_difficulty_max: config.difficulty_max,
        vdf_difficulty_stale: config.difficulty_stale,
        vdf_lambda_bound: config.lambda_bound,
    };

    sortition::calculate_legacy_difficulty(legacy_config, threshold)
}

/// Generates the VDF portion of a legacy DAG sortition payload from a
/// previously verified VRF proof.
///
/// Inputs:
/// - `vrf_proof`: exact 80-byte proof produced by the key-custody signer.
/// - `vdf_input`: canonical DAG VDF message bytes.
/// - `difficulty`: Rust-selected difficulty retained by the proposer cursor.
/// - `lambda_bound`: nonzero Wesolowski hash-to-prime bound.
/// - `cancellation_token`: owned token shared with the asynchronous job owner.
///
/// The function does not require or reconstruct private key material. Empty
/// prover output is accepted only when cancellation was observed; otherwise it
/// is returned as an operational error.
pub fn prove_vdf_sortition(
    vrf_proof: &[u8],
    vdf_input: &[u8],
    difficulty: u16,
    lambda_bound: u16,
    cancellation_token: &CancellationToken,
) -> Result<VdfSortitionProofOutcome> {
    ensure!(
        lambda_bound != 0,
        "VDF lambda bound must be greater than zero"
    );
    let vrf_proof: [u8; VRF_PROOF_BYTES] = vrf_proof
        .try_into()
        .map_err(|_| anyhow::anyhow!("VRF proof must be {VRF_PROOF_BYTES} bytes"))?;
    let vdf = WesolowskiVdf::new(
        u32::from(lambda_bound),
        u32::from(difficulty),
        vdf_input.to_vec(),
        legacy_vdf_modulus_ascii_hex().to_vec(),
    );
    let solution = WesolowskiProver::new(&vdf).prove(cancellation_token);
    if solution.first.is_empty() && solution.second.is_empty() && cancellation_token.is_cancelled()
    {
        return Ok(VdfSortitionProofOutcome::Cancelled);
    }
    ensure!(
        !solution.first.is_empty() && !solution.second.is_empty(),
        "VDF prover returned an empty proof or output"
    );
    Ok(VdfSortitionProofOutcome::Completed(VdfSortitionPayload {
        vrf_proof,
        vdf_solution_proof: solution.first,
        vdf_solution_output: solution.second,
        difficulty,
    }))
}

/// Verifies a VDF sortition payload using the legacy ASCII-hex modulus.
pub fn verify_vdf_sortition(
    payload: &VdfSortitionPayload,
    vdf_input: &[u8],
    config: VdfSortitionVerifyConfig,
    vrf_output: &[u8],
    sender_eligible_vote_count: u64,
    vdf_sortition_max_vote_count: u64,
) -> Result<VdfSortitionVerifyResult> {
    verify_vdf_sortition_with_modulus(
        payload,
        vdf_input,
        config,
        vrf_output,
        sender_eligible_vote_count,
        vdf_sortition_max_vote_count,
        legacy_vdf_modulus_ascii_hex(),
    )
}

/// Verifies a VDF sortition payload using an explicit modulus byte array.
pub fn verify_vdf_sortition_with_modulus(
    payload: &VdfSortitionPayload,
    vdf_input: &[u8],
    config: VdfSortitionVerifyConfig,
    vrf_output: &[u8],
    sender_eligible_vote_count: u64,
    vdf_sortition_max_vote_count: u64,
    modulus: &[u8],
) -> Result<VdfSortitionVerifyResult> {
    ensure!(!modulus.is_empty(), "VDF modulus cannot be empty");

    let vote_count =
        normalize_vote_count(sender_eligible_vote_count, vdf_sortition_max_vote_count)?;
    let vrf_threshold = threshold_from_vrf_output(vrf_output, vote_count)?;
    let expected_difficulty = calculate_vdf_sortition_difficulty(config, vrf_threshold)?;
    let mut status = DAG_VERIFY_VDF_STATUS_VALID;
    if payload.difficulty != expected_difficulty {
        status = DAG_VERIFY_VDF_STATUS_INVALID;
    } else {
        let vdf = WesolowskiVdf::new(
            u32::from(config.lambda_bound),
            u32::from(payload.difficulty),
            vdf_input.to_vec(),
            modulus.to_vec(),
        );
        let solution = crate::vdf::Solution {
            first: payload.vdf_solution_proof.clone(),
            second: payload.vdf_solution_output.clone(),
        };
        if !WesolowskiVerifier::new(&vdf).verify(&solution) {
            status = DAG_VERIFY_VDF_STATUS_INVALID;
        }
    }

    Ok(VdfSortitionVerifyResult {
        vdf_status: status,
        difficulty: payload.difficulty,
        expected_difficulty,
    })
}

#[cfg(test)]
mod tests {
    use crate::sortition::{self, LegacySortitionParams};
    use crate::vrf::VRF_OUTPUT_BYTES;

    use super::{
        DAG_VERIFY_VDF_STATUS_INVALID, DAG_VERIFY_VDF_STATUS_VALID, VdfSortitionPayload,
        VdfSortitionVerifyConfig, calculate_vdf_sortition_difficulty, decode_vdf_sortition_payload,
        encode_vdf_sortition_payload, normalize_vote_count, threshold_from_vrf_output,
        verify_vdf_sortition, verify_vdf_sortition_with_modulus,
    };

    const SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn verify_config() -> VdfSortitionVerifyConfig {
        VdfSortitionVerifyConfig {
            threshold_upper: 0x5ff,
            difficulty_min: 5,
            difficulty_max: 10,
            difficulty_stale: 9,
            lambda_bound: 64,
        }
    }

    fn build_fixture_payload() -> (
        VdfSortitionPayload,
        [u8; VRF_OUTPUT_BYTES],
        Vec<u8>,
        Vec<u8>,
    ) {
        let vrf_input = [0xA1, 0x02, 0x03];
        let vdf_input = [0xB1, 0x04];
        let proof = sortition::prove_legacy_vdf_sortition(
            LegacySortitionParams {
                vrf_threshold_upper: 0x5ff,
                vdf_difficulty_min: 5,
                vdf_difficulty_max: 10,
                vdf_difficulty_stale: 9,
                vdf_lambda_bound: 64,
            },
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &crate::prover::CancellationToken::new(),
        )
        .unwrap();

        let payload = VdfSortitionPayload {
            vrf_proof: proof.vrf_proof,
            vdf_solution_proof: proof.vdf_proof,
            vdf_solution_output: proof.vdf_output,
            difficulty: proof.difficulty,
        };

        (
            payload,
            proof.vrf_output,
            vrf_input.to_vec(),
            vdf_input.to_vec(),
        )
    }

    #[test]
    fn vdf_sortition_payload_codec_round_trip() {
        let (payload, _, _, _) = build_fixture_payload();
        let encoded = encode_vdf_sortition_payload(&payload);
        let decoded = decode_vdf_sortition_payload(&encoded).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn vdf_sortition_payload_decode_errors_on_bad_shape() {
        assert!(decode_vdf_sortition_payload(&[0x00]).is_err());
        let malformed = encode_vdf_sortition_payload(&VdfSortitionPayload {
            vrf_proof: [0x01; 80],
            vdf_solution_proof: vec![0x02],
            vdf_solution_output: vec![0x03],
            difficulty: 5,
        })
        .as_slice()[..5]
            .to_vec();
        assert!(decode_vdf_sortition_payload(&malformed).is_err());
    }

    #[test]
    fn vdf_sortition_difficulty_and_threshold_match_legacy_formulas() {
        let config = verify_config();

        let normalized = normalize_vote_count(1, 1).unwrap();
        assert_eq!(normalized, 1000);

        let threshold = threshold_from_vrf_output(&[0x34, 0x12].repeat(32), normalized).unwrap();
        let expected_vdf_difficulty =
            calculate_vdf_sortition_difficulty(config, threshold).unwrap();

        let legacy = sortition::calculate_legacy_difficulty(
            LegacySortitionParams {
                vrf_threshold_upper: config.threshold_upper,
                vdf_difficulty_min: config.difficulty_min,
                vdf_difficulty_max: config.difficulty_max,
                vdf_difficulty_stale: config.difficulty_stale,
                vdf_lambda_bound: config.lambda_bound,
            },
            threshold,
        )
        .unwrap();

        assert_eq!(expected_vdf_difficulty, legacy);
    }

    #[test]
    fn vdf_sortition_verify_reports_status_without_raising_for_normalized_inputs() {
        let (payload, vrf_output, _vrf_input, vdf_input) = build_fixture_payload();
        let config = verify_config();
        let verified =
            verify_vdf_sortition(&payload, &vdf_input, config, &vrf_output, 1, 1).unwrap();

        assert_eq!(verified.vdf_status, DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(verified.difficulty, verified.expected_difficulty);

        let mut bad_payload = payload.clone();
        bad_payload.difficulty = bad_payload.difficulty.saturating_add(1);
        let mismatch =
            verify_vdf_sortition(&bad_payload, &vdf_input, config, &vrf_output, 1, 1).unwrap();
        assert_eq!(mismatch.vdf_status, DAG_VERIFY_VDF_STATUS_INVALID);
        assert_eq!(mismatch.expected_difficulty, verified.expected_difficulty);

        let custom = verify_vdf_sortition_with_modulus(
            &payload,
            &vdf_input,
            config,
            &vrf_output,
            1,
            1,
            crate::vdf_sortition::legacy_vdf_modulus_ascii_hex(),
        )
        .unwrap();
        assert_eq!(custom.vdf_status, DAG_VERIFY_VDF_STATUS_VALID);
    }

    #[test]
    fn vdf_sortition_rejects_invalid_output_shape_or_zero_votes() {
        let (payload, _, _, vdf_input) = build_fixture_payload();
        let config = verify_config();

        assert!(threshold_from_vrf_output(&[1_u8, 2, 3], 1).is_err());
        assert!(verify_vdf_sortition(&payload, &vdf_input, config, &[1_u8, 2, 3], 1, 1,).is_err());

        assert_eq!(
            normalize_vote_count(1, 0).unwrap_err().to_string(),
            "VRF total vote count cannot be zero"
        );
    }

    #[test]
    fn vdf_sortition_verify_rejects_empty_modulus_for_explicit_modulus_path() {
        let (payload, vrf_output, _, vdf_input) = build_fixture_payload();
        let config = verify_config();

        let err =
            verify_vdf_sortition_with_modulus(&payload, &vdf_input, config, &vrf_output, 1, 1, &[])
                .unwrap_err()
                .to_string();

        assert!(err.contains("VDF modulus cannot be empty"));
    }
}
