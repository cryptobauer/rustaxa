use crate::prover::{CancellationToken, WesolowskiProver};
use crate::vdf::{Solution, WesolowskiVdf};
use crate::verifier::WesolowskiVerifier;

#[cxx::bridge]
mod ffi {
    extern "Rust" {
        type WesolowskiVdf;
        type WesolowskiVerifier<'a>;

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

// Boilerplate for C++ to call Rust functions. We are not moving this to the actual
// modules because only C++ needs this.

pub fn make_vdf(lambda: u32, time_bits: u32, input: &[u8], modulus: &[u8]) -> Box<WesolowskiVdf> {
    Box::new(WesolowskiVdf::new(
        lambda,
        time_bits,
        input.to_vec(),
        modulus.to_vec(),
    ))
}

pub fn make_solution(proof: &[u8], output: &[u8]) -> Box<Solution> {
    Box::new(Solution {
        first: proof.to_vec(),
        second: output.to_vec(),
    })
}

pub fn make_cancellation_token() -> Box<CancellationToken> {
    Box::new(CancellationToken::new())
}

pub fn make_cancellation_token_with_atomic(atomic_ptr: *const bool) -> Box<CancellationToken> {
    Box::new(CancellationToken::from_atomic_ptr(atomic_ptr))
}

pub fn cancellation_token_cancel(token: &CancellationToken) {
    token.cancel();
}

pub fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool {
    let verifier = WesolowskiVerifier::new(vdf);
    verifier.verify(solution)
}

pub fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution> {
    let prover = WesolowskiProver::new(vdf);
    Box::new(prover.prove(cancelled))
}

pub fn solution_get_proof(solution: &Solution) -> &[u8] {
    &solution.first
}

pub fn solution_get_output(solution: &Solution) -> &[u8] {
    &solution.second
}
