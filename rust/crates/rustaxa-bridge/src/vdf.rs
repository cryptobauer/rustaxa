use crate::ffi::rustaxa_ffi::{
    LegacySortitionParams, VdfSortitionPayload, VdfSortitionProofResult,
    VdfSortitionVerifyResult as LegacyVdfSortitionVerifyResult, VrfProofResult,
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

pub fn make_cancellation_token_with_atomic(atomic_ptr: *const bool) -> Box<CancellationToken> {
    Box::new(CancellationToken(InnerCancellationToken::from_atomic_ptr(
        atomic_ptr,
    )))
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

fn to_domain_sortition_params(params: LegacySortitionParams) -> DomainLegacySortitionParams {
    DomainLegacySortitionParams {
        vrf_threshold_upper: params.vrf_threshold_upper,
        vdf_difficulty_min: params.vdf_difficulty_min,
        vdf_difficulty_max: params.vdf_difficulty_max,
        vdf_difficulty_stale: params.vdf_difficulty_stale,
        vdf_lambda_bound: params.vdf_lambda_bound,
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
