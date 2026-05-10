use crate::ffi::rustaxa_ffi::{
    LegacySortitionParams, VdfSortitionPayload, VdfSortitionPayloadVerifyResult,
    VdfSortitionProofResult, VdfSortitionVerifyConfig,
    VdfSortitionVerifyResult as LegacyVdfSortitionVerifyResult, VrfProofResult, VrfVerifyOutput,
    VrfVerifyResult,
};
use rustaxa_vdf::prover::{CancellationToken as InnerCancellationToken, WesolowskiProver};
use rustaxa_vdf::sortition::{
    LegacySortitionParams as DomainLegacySortitionParams, LEGACY_SORTITION_STATUS_INTERNAL_ERROR,
    LEGACY_SORTITION_STATUS_INVALID_ARGUMENT, LEGACY_SORTITION_STATUS_VALID,
};
use rustaxa_vdf::vdf::{Solution as InnerSolution, WesolowskiVdf as InnerWesolowskiVdf};
use rustaxa_vdf::vdf_sortition as domain_sortition_vdf;
use rustaxa_vdf::verifier::WesolowskiVerifier;
use rustaxa_vdf::{sortition as domain_sortition, vrf as domain_vrf};

// Wrapper types to satisfy Orphan Rule since we are bridging types from another crate
pub struct WesolowskiVdf(InnerWesolowskiVdf);
pub struct CancellationToken(InnerCancellationToken);
pub struct Solution(InnerSolution);

pub fn make_vdf(lambda: u32, time_bits: u32, input: &[u8], modulus: &[u8]) -> Box<WesolowskiVdf> {
    Box::new(WesolowskiVdf(InnerWesolowskiVdf::new(
        lambda,
        time_bits,
        input.to_vec(),
        modulus.to_vec(),
    )))
}

pub fn make_solution(proof: &[u8], output: &[u8]) -> Box<Solution> {
    Box::new(Solution(InnerSolution {
        first: proof.to_vec(),
        second: output.to_vec(),
    }))
}

pub fn make_cancellation_token() -> Box<CancellationToken> {
    Box::new(CancellationToken(InnerCancellationToken::new()))
}

pub fn make_cancellation_token_with_atomic(atomic_ptr: *const bool) -> Box<CancellationToken> {
    Box::new(CancellationToken(InnerCancellationToken::from_atomic_ptr(
        atomic_ptr,
    )))
}

pub fn cancellation_token_cancel(token: &CancellationToken) {
    token.0.cancel();
}

pub fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool {
    let verifier = WesolowskiVerifier::new(&vdf.0);
    verifier.verify(&solution.0)
}

pub fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution> {
    let prover = WesolowskiProver::new(&vdf.0);
    Box::new(Solution(prover.prove(&cancelled.0)))
}

pub fn solution_get_proof(solution: &Solution) -> &[u8] {
    &solution.0.first
}

pub fn solution_get_output(solution: &Solution) -> &[u8] {
    &solution.0.second
}

pub fn verify_legacy_vrf_sortition(
    public_key: &[u8; 32],
    proof: &[u8; 80],
    message: &[u8],
    vote_count: u16,
    strict: bool,
) -> VrfVerifyResult {
    let output = if strict {
        match domain_vrf::verify_output(public_key, proof, message) {
            Ok(Some(output)) => output,
            Ok(None) => {
                return vrf_failure(
                    domain_sortition::LEGACY_SORTITION_STATUS_INVALID_VRF_PROOF,
                    "",
                )
            }
            Err(err) => {
                return vrf_failure(LEGACY_SORTITION_STATUS_INVALID_ARGUMENT, &err.to_string())
            }
        }
    } else {
        match domain_vrf::proof_to_hash(proof) {
            Ok(output) => output,
            Err(err) => {
                return vrf_failure(LEGACY_SORTITION_STATUS_INVALID_ARGUMENT, &err.to_string())
            }
        }
    };
    let threshold = domain_vrf::threshold_from_output(&output, vote_count);
    VrfVerifyResult {
        ok: true,
        status: LEGACY_SORTITION_STATUS_VALID,
        error: String::new(),
        output,
        threshold,
    }
}

pub fn prove_legacy_vrf_sortition(
    secret_key: &[u8; 64],
    message: &[u8],
    vote_count: u16,
) -> VrfProofResult {
    let public_key = match domain_vrf::public_key_from_secret(secret_key) {
        Ok(public_key) => public_key,
        Err(err) => {
            return vrf_proof_failure(LEGACY_SORTITION_STATUS_INVALID_ARGUMENT, &err.to_string())
        }
    };
    let proof = match domain_vrf::prove(secret_key, message) {
        Ok(proof) => proof,
        Err(err) => {
            return vrf_proof_failure(LEGACY_SORTITION_STATUS_INVALID_ARGUMENT, &err.to_string())
        }
    };
    let output = match domain_vrf::verify_output(&public_key, &proof, message) {
        Ok(Some(output)) => output,
        Ok(None) => {
            return vrf_proof_failure(
                domain_sortition::LEGACY_SORTITION_STATUS_INVALID_VRF_PROOF,
                "",
            )
        }
        Err(err) => {
            return vrf_proof_failure(LEGACY_SORTITION_STATUS_INVALID_ARGUMENT, &err.to_string())
        }
    };
    let threshold = domain_vrf::threshold_from_output(&output, vote_count);

    VrfProofResult {
        ok: true,
        status: LEGACY_SORTITION_STATUS_VALID,
        error: String::new(),
        public_key,
        proof,
        output,
        threshold,
    }
}

pub fn prove_legacy_vdf_sortition(
    params: LegacySortitionParams,
    secret_key: &[u8; 64],
    vrf_input: &[u8],
    vdf_input: &[u8],
    vote_count: u64,
    total_vote_count: u64,
    cancellation_token: &CancellationToken,
) -> VdfSortitionProofResult {
    match domain_sortition::prove_legacy_vdf_sortition(
        to_domain_sortition_params(params),
        secret_key,
        vrf_input,
        vdf_input,
        vote_count,
        total_vote_count,
        &cancellation_token.0,
    ) {
        Ok(proof) => VdfSortitionProofResult {
            ok: proof.ok,
            status: proof.status,
            error: String::new(),
            vrf_proof: proof.vrf_proof,
            vrf_output: proof.vrf_output,
            vrf_threshold: proof.vrf_threshold,
            difficulty: proof.difficulty,
            vdf_proof: proof.vdf_proof,
            vdf_output: proof.vdf_output,
        },
        Err(err) => VdfSortitionProofResult {
            ok: false,
            status: LEGACY_SORTITION_STATUS_INTERNAL_ERROR,
            error: err.to_string(),
            vrf_proof: [0_u8; 80],
            vrf_output: [0_u8; 64],
            vrf_threshold: 0,
            difficulty: 0,
            vdf_proof: Vec::new(),
            vdf_output: Vec::new(),
        },
    }
}

pub fn verify_legacy_vdf_sortition(
    params: LegacySortitionParams,
    public_key: &[u8; 32],
    sortition_rlp: &[u8],
    vrf_input: &[u8],
    vdf_input: &[u8],
    vote_count: u64,
    total_vote_count: u64,
) -> LegacyVdfSortitionVerifyResult {
    match domain_sortition::verify_legacy_vdf_sortition(
        to_domain_sortition_params(params),
        public_key,
        sortition_rlp,
        vrf_input,
        vdf_input,
        vote_count,
        total_vote_count,
    ) {
        Ok(verification) => LegacyVdfSortitionVerifyResult {
            ok: verification.ok,
            status: verification.status,
            error: String::new(),
            vrf_output: verification.vrf_output,
            vrf_threshold: verification.vrf_threshold,
            expected_difficulty: verification.expected_difficulty,
            actual_difficulty: verification.actual_difficulty,
        },
        Err(err) => LegacyVdfSortitionVerifyResult {
            ok: false,
            status: LEGACY_SORTITION_STATUS_INVALID_ARGUMENT,
            error: err.to_string(),
            vrf_output: [0_u8; 64],
            vrf_threshold: 0,
            expected_difficulty: 0,
            actual_difficulty: 0,
        },
    }
}

pub fn vdf_sortition_payload_encode(payload: &VdfSortitionPayload) -> Vec<u8> {
    domain_sortition_vdf::encode_vdf_sortition_payload(&domain_sortition_vdf::VdfSortitionPayload {
        vrf_proof: payload.vrf_proof,
        vdf_solution_proof: payload.vdf_solution_proof.clone(),
        vdf_solution_output: payload.vdf_solution_output.clone(),
        difficulty: payload.difficulty,
    })
}

pub fn vdf_sortition_payload_decode(payload: &[u8]) -> anyhow::Result<VdfSortitionPayload> {
    let decoded = domain_sortition_vdf::decode_vdf_sortition_payload(payload)?;
    Ok(VdfSortitionPayload {
        vrf_proof: decoded.vrf_proof,
        vdf_solution_proof: decoded.vdf_solution_proof,
        vdf_solution_output: decoded.vdf_solution_output,
        difficulty: decoded.difficulty,
    })
}

pub fn vdf_sortition_payload_verify(
    payload: &VdfSortitionPayload,
    vdf_input: &[u8],
    config: VdfSortitionVerifyConfig,
    vrf_output: &[u8],
    sender_eligible_vote_count: u64,
    vdf_sortition_max_vote_count: u64,
) -> anyhow::Result<VdfSortitionPayloadVerifyResult> {
    domain_sortition_vdf::verify_vdf_sortition(
        &to_domain_vdf_sortition_payload(payload),
        vdf_input,
        to_domain_vdf_sortition_config(config),
        vrf_output,
        sender_eligible_vote_count,
        vdf_sortition_max_vote_count,
    )
    .map(|result| VdfSortitionPayloadVerifyResult {
        vdf_status: result.vdf_status,
        difficulty: result.difficulty,
        expected_difficulty: result.expected_difficulty,
    })
}

pub fn vdf_sortition_payload_verify_with_modulus(
    payload: &VdfSortitionPayload,
    vdf_input: &[u8],
    config: VdfSortitionVerifyConfig,
    vrf_output: &[u8],
    sender_eligible_vote_count: u64,
    vdf_sortition_max_vote_count: u64,
    modulus: &[u8],
) -> anyhow::Result<VdfSortitionPayloadVerifyResult> {
    domain_sortition_vdf::verify_vdf_sortition_with_modulus(
        &to_domain_vdf_sortition_payload(payload),
        vdf_input,
        to_domain_vdf_sortition_config(config),
        vrf_output,
        sender_eligible_vote_count,
        vdf_sortition_max_vote_count,
        modulus,
    )
    .map(|result| VdfSortitionPayloadVerifyResult {
        vdf_status: result.vdf_status,
        difficulty: result.difficulty,
        expected_difficulty: result.expected_difficulty,
    })
}

pub fn vdf_sortition_threshold_from_output(
    vrf_output: &[u8],
    vote_count: u16,
) -> anyhow::Result<u16> {
    domain_sortition_vdf::threshold_from_vrf_output(vrf_output, vote_count)
}

pub fn vdf_sortition_normalize_vote_count(
    sender_eligible_vote_count: u64,
    vdf_sortition_max_vote_count: u64,
) -> anyhow::Result<u16> {
    domain_sortition_vdf::normalize_vote_count(
        sender_eligible_vote_count,
        vdf_sortition_max_vote_count,
    )
}

pub fn vdf_sortition_difficulty(
    config: VdfSortitionVerifyConfig,
    threshold: u16,
) -> anyhow::Result<u16> {
    domain_sortition_vdf::calculate_vdf_sortition_difficulty(
        to_domain_vdf_sortition_config(config),
        threshold,
    )
}

pub fn vdf_sortition_legacy_modulus() -> Vec<u8> {
    domain_sortition_vdf::legacy_vdf_modulus_ascii_hex().to_vec()
}

pub fn vrf_verify_output(
    vrf_public_key: &[u8],
    vrf_proof: &[u8],
    message: &[u8],
) -> anyhow::Result<VrfVerifyOutput> {
    let output = domain_vrf::verify_output(vrf_public_key, vrf_proof, message)?;
    Ok(match output {
        Some(output) => VrfVerifyOutput {
            is_valid: true,
            output: output.to_vec(),
        },
        None => VrfVerifyOutput {
            is_valid: false,
            output: Vec::new(),
        },
    })
}

pub fn vrf_proof_to_hash(vrf_proof: &[u8]) -> anyhow::Result<Vec<u8>> {
    domain_vrf::proof_to_hash(vrf_proof).map(|output| output.to_vec())
}

pub fn vrf_prove_output(vrf_secret_key: &[u8], message: &[u8]) -> anyhow::Result<Vec<u8>> {
    domain_vrf::prove(vrf_secret_key, message).map(|output| output.to_vec())
}

fn to_domain_sortition_params(params: LegacySortitionParams) -> DomainLegacySortitionParams {
    DomainLegacySortitionParams {
        vrf_threshold_upper: params.vrf_threshold_upper,
        vdf_difficulty_min: params.vdf_difficulty_min,
        vdf_difficulty_max: params.vdf_difficulty_max,
        vdf_difficulty_stale: params.vdf_difficulty_stale,
        vdf_lambda_bound: params.vdf_lambda_bound,
    }
}

fn to_domain_vdf_sortition_payload(
    payload: &VdfSortitionPayload,
) -> domain_sortition_vdf::VdfSortitionPayload {
    domain_sortition_vdf::VdfSortitionPayload {
        vrf_proof: payload.vrf_proof,
        vdf_solution_proof: payload.vdf_solution_proof.clone(),
        vdf_solution_output: payload.vdf_solution_output.clone(),
        difficulty: payload.difficulty,
    }
}

fn to_domain_vdf_sortition_config(
    config: VdfSortitionVerifyConfig,
) -> domain_sortition_vdf::VdfSortitionVerifyConfig {
    domain_sortition_vdf::VdfSortitionVerifyConfig {
        threshold_upper: config.threshold_upper,
        difficulty_min: config.difficulty_min,
        difficulty_max: config.difficulty_max,
        difficulty_stale: config.difficulty_stale,
        lambda_bound: config.lambda_bound,
    }
}

fn vrf_failure(status: u8, error: &str) -> VrfVerifyResult {
    VrfVerifyResult {
        ok: false,
        status,
        error: error.to_string(),
        output: [0_u8; 64],
        threshold: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn fixture_payload() -> (
        VdfSortitionPayload,
        [u8; 64],
        Vec<u8>,
        Vec<u8>,
        VdfSortitionVerifyConfig,
    ) {
        let token = make_cancellation_token();
        let vrf_input = [0xA1, 0x02, 0x03];
        let vdf_input = [0xB1, 0x04];
        let proof = domain_sortition::prove_legacy_vdf_sortition(
            domain_sortition::LegacySortitionParams {
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
            &token.0,
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
            verify_config(),
        )
    }

    #[test]
    fn vdf_sortition_payload_round_trip_and_verify() {
        let (payload, vrf_output, _vrf_input, vdf_input, config) = fixture_payload();
        let encoded = vdf_sortition_payload_encode(&payload);
        let decoded = vdf_sortition_payload_decode(&encoded).unwrap();

        assert_eq!(decoded.vrf_proof, payload.vrf_proof);
        assert_eq!(decoded.vdf_solution_proof, payload.vdf_solution_proof);
        assert_eq!(decoded.vdf_solution_output, payload.vdf_solution_output);
        assert_eq!(decoded.difficulty, payload.difficulty);

        let result =
            vdf_sortition_payload_verify(&payload, &vdf_input, config, &vrf_output, 1, 1).unwrap();

        assert_eq!(result.difficulty, result.expected_difficulty);
    }

    #[test]
    fn vdf_sortition_payload_verify_reports_status_without_panicking() {
        let (payload, _, _, vdf_input, config) = fixture_payload();
        let bad_payload = VdfSortitionPayload {
            difficulty: payload.difficulty.saturating_add(1),
            ..payload
        };
        let result =
            vdf_sortition_payload_verify(&bad_payload, &vdf_input, config, &[0_u8; 64], 1, 1)
                .unwrap();

        assert_eq!(
            result.vdf_status,
            rustaxa_vdf::vdf_sortition::DAG_VERIFY_VDF_STATUS_INVALID
        );
    }

    #[test]
    fn vrf_output_verification_round_trips_with_bridge_entrypoint() {
        let public_key = domain_vrf::public_key_from_secret(&SECRET_KEY).unwrap();
        let proof = domain_vrf::prove(&SECRET_KEY, b"bridge-vrf").unwrap();
        let result = vrf_verify_output(&public_key, &proof, b"bridge-vrf").unwrap();

        assert!(result.is_valid);
        assert!(!result.output.is_empty());

        let compatibility = domain_vrf::proof_to_hash(&proof).unwrap();
        assert_eq!(result.output, compatibility.to_vec());
    }
}

fn vrf_proof_failure(status: u8, error: &str) -> VrfProofResult {
    VrfProofResult {
        ok: false,
        status,
        error: error.to_string(),
        public_key: [0_u8; 32],
        proof: [0_u8; 80],
        output: [0_u8; 64],
        threshold: 0,
    }
}
