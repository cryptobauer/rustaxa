use rustaxa_vdf::prover::{CancellationToken as InnerCancellationToken, WesolowskiProver};
use rustaxa_vdf::vdf::{Solution as InnerSolution, WesolowskiVdf as InnerWesolowskiVdf};
use rustaxa_vdf::verifier::WesolowskiVerifier;

// Wrapper types to satisfy Orphan Rule since we are bridging types from another crate
pub struct WesolowskiVdf(InnerWesolowskiVdf);
pub struct CancellationToken(InnerCancellationToken);
pub struct Solution(InnerSolution);

#[cxx::bridge(namespace = "rustaxa::vdf")]
mod ffi {
    extern "Rust" {
        type WesolowskiVdf;
        // WesolowskiVerifier is not exposed to C++, so we don't list it here

        fn make_vdf(
            lambda: u32,
            time_bits: u32,
            input: &[u8],
            modulus: &[u8],
        ) -> Box<WesolowskiVdf>;

        type CancellationToken;
        type Solution;

        fn make_solution(proof: &[u8], output: &[u8]) -> Box<Solution>;

        fn make_cancellation_token() -> Box<CancellationToken>;
        unsafe fn make_cancellation_token_with_atomic(
            atomic_ptr: *const bool,
        ) -> Box<CancellationToken>;
        fn cancellation_token_cancel(token: &CancellationToken);

        fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution>;
        fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool;

        fn solution_get_proof(solution: &Solution) -> &[u8];
        fn solution_get_output(solution: &Solution) -> &[u8];
    }
}

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
    Box::new(CancellationToken(InnerCancellationToken::from_atomic_ptr(atomic_ptr)))
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
