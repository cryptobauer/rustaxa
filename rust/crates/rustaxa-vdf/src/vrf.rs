//! Taraxa VRF compatibility wrapper over the vendored libsodium VRF ABI.
//!
//! This module centralizes all unsafe calls to the C VRF implementation and
//! exposes shape-checked Rust functions for key derivation, proof generation,
//! proof verification, compatibility proof-to-hash conversion, and legacy
//! threshold calculation.

use anyhow::{Context, Result, bail, ensure};
use std::os::raw::{c_int, c_uchar, c_ulonglong};
use std::sync::Once;

/// Size in bytes of a Taraxa VRF public key.
pub const VRF_PUBLIC_KEY_BYTES: usize = 32;
/// Size in bytes of a Taraxa VRF secret key.
pub const VRF_SECRET_KEY_BYTES: usize = 64;
/// Size in bytes of a Taraxa VRF proof.
pub const VRF_PROOF_BYTES: usize = 80;
/// Size in bytes of a Taraxa VRF output.
pub const VRF_OUTPUT_BYTES: usize = 64;

static INIT: Once = Once::new();

#[link(name = "sodium", kind = "static")]
unsafe extern "C" {
    fn sodium_init() -> c_int;
    fn crypto_vrf_sk_to_pk(pk: *mut c_uchar, sk: *const c_uchar);
    fn crypto_vrf_is_valid_key(pk: *const c_uchar) -> c_int;
    fn crypto_vrf_prove(
        proof: *mut c_uchar,
        sk: *const c_uchar,
        message: *const c_uchar,
        message_len: c_ulonglong,
    ) -> c_int;
    fn crypto_vrf_verify(
        output: *mut c_uchar,
        pk: *const c_uchar,
        proof: *const c_uchar,
        message: *const c_uchar,
        message_len: c_ulonglong,
    ) -> c_int;
    fn crypto_vrf_proof_to_hash(output: *mut c_uchar, proof: *const c_uchar) -> c_int;
}

/// Initializes the underlying Taraxa VRF library once per process.
///
/// C++ callers normally initialize libsodium through `common::init`. Rust unit
/// tests and standalone bridge calls do not have that guarantee, so every
/// public VRF entrypoint calls this helper before invoking the C ABI.
pub fn initialize_vrf() -> Result<()> {
    let mut init_result = 0;
    INIT.call_once(|| {
        // SAFETY: `sodium_init` has no preconditions and is explicitly allowed
        // to be called more than once by libsodium. `Once` keeps the result
        // stable for the process.
        init_result = unsafe { sodium_init() };
    });
    ensure!(
        init_result != -1,
        "Taraxa VRF library initialization failed"
    );
    Ok(())
}

/// Derives a VRF public key from a 64-byte secret key.
///
/// Inputs:
/// - `secret_key`: Taraxa VRF secret key bytes.
///
/// Output:
/// - 32-byte public key bytes matching C++ `getVrfPublicKey`.
pub fn public_key_from_secret(secret_key: &[u8]) -> Result<[u8; VRF_PUBLIC_KEY_BYTES]> {
    initialize_vrf()?;
    let secret_key = secret_key_array(secret_key)?;
    let mut public_key = [0_u8; VRF_PUBLIC_KEY_BYTES];
    // SAFETY: array lengths match the Taraxa VRF ABI constants and pointers are
    // valid for the duration of the call.
    unsafe {
        crypto_vrf_sk_to_pk(public_key.as_mut_ptr(), secret_key.as_ptr());
    }
    Ok(public_key)
}

/// Returns whether a 32-byte VRF public key is accepted by the Taraxa VRF ABI.
pub fn is_valid_public_key(public_key: &[u8]) -> Result<bool> {
    initialize_vrf()?;
    let public_key = public_key_array(public_key)?;
    // SAFETY: array length matches the Taraxa VRF ABI constant and the pointer
    // is valid for the duration of the call.
    Ok(unsafe { crypto_vrf_is_valid_key(public_key.as_ptr()) == 1 })
}

/// Creates a VRF proof for `message` using a 64-byte secret key.
///
/// This mirrors C++ `getVrfProof`: success is reported by the C function
/// returning zero.
pub fn prove(secret_key: &[u8], message: &[u8]) -> Result<[u8; VRF_PROOF_BYTES]> {
    initialize_vrf()?;
    let secret_key = secret_key_array(secret_key)?;
    let mut proof = [0_u8; VRF_PROOF_BYTES];
    // SAFETY: array lengths match ABI constants and message pointer/length are
    // borrowed from an immutable slice for the duration of the call.
    let code = unsafe {
        crypto_vrf_prove(
            proof.as_mut_ptr(),
            secret_key.as_ptr(),
            message.as_ptr(),
            message
                .len()
                .try_into()
                .context("VRF message length does not fit C ABI")?,
        )
    };
    ensure!(code == 0, "VRF proof creation failed");
    Ok(proof)
}

/// Verifies a VRF proof and returns the 64-byte VRF output on success.
///
/// Inputs:
/// - `public_key`: 32-byte Taraxa VRF public key.
/// - `proof`: 80-byte Taraxa VRF proof.
/// - `message`: message bytes used by the prover.
///
/// Output:
/// - `Ok(Some(output))` when strict proof verification succeeds.
/// - `Ok(None)` when peer-controlled proof data is invalid.
/// - `Err(_)` when caller-provided shapes or runtime invariants are invalid.
pub fn verify_output(
    public_key: &[u8],
    proof: &[u8],
    message: &[u8],
) -> Result<Option<[u8; VRF_OUTPUT_BYTES]>> {
    initialize_vrf()?;
    let public_key = public_key_array(public_key)?;
    let proof = proof_array(proof)?;
    let mut output = [0_u8; VRF_OUTPUT_BYTES];
    // SAFETY: array lengths match ABI constants and message pointer/length are
    // borrowed from an immutable slice for the duration of the call.
    let code = unsafe {
        crypto_vrf_verify(
            output.as_mut_ptr(),
            public_key.as_ptr(),
            proof.as_ptr(),
            message.as_ptr(),
            message
                .len()
                .try_into()
                .context("VRF message length does not fit C ABI")?,
        )
    };
    Ok((code == 0).then_some(output))
}

/// Converts a VRF proof to its output without strict public-key verification.
///
/// This preserves C++ `getVrfOutput(..., strict = false)` behavior and should
/// only be used by compatibility callers that deliberately want proof hashing.
pub fn proof_to_hash(proof: &[u8]) -> Result<[u8; VRF_OUTPUT_BYTES]> {
    initialize_vrf()?;
    let proof = proof_array(proof)?;
    let mut output = [0_u8; VRF_OUTPUT_BYTES];
    // SAFETY: array lengths match ABI constants and pointers are valid for the
    // duration of the call.
    let code = unsafe { crypto_vrf_proof_to_hash(output.as_mut_ptr(), proof.as_ptr()) };
    ensure!(code == 0, "VRF proof-to-hash conversion failed");
    Ok(output)
}

/// Computes the legacy little-endian VRF threshold for one or more votes.
///
/// Inputs:
/// - `output`: verified or compatibility-derived VRF output bytes.
/// - `vote_count`: normalized vote count as accepted by C++
///   `VrfSortitionBase::thresholdFromOutput`.
///
/// Output:
/// - minimum 16-bit threshold over the minstd-derived sequence.
pub fn threshold_from_output(output: &[u8; VRF_OUTPUT_BYTES], vote_count: u16) -> u16 {
    const MINSTD_RAND_MULTIPLIER: u16 = 48271;

    let mut threshold = (u16::from(output[1]) << 8) | u16::from(output[0]);
    if vote_count > 1 {
        let mut min_threshold = threshold;
        let mut threshold_candidate = threshold;
        for _ in 1..vote_count {
            threshold_candidate = threshold_candidate.wrapping_mul(MINSTD_RAND_MULTIPLIER);
            if threshold_candidate < min_threshold {
                min_threshold = threshold_candidate;
            }
        }
        threshold = min_threshold;
    }
    threshold
}

pub(crate) fn public_key_array(public_key: &[u8]) -> Result<[u8; VRF_PUBLIC_KEY_BYTES]> {
    public_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("VRF public key must be {VRF_PUBLIC_KEY_BYTES} bytes"))
}

pub(crate) fn secret_key_array(secret_key: &[u8]) -> Result<[u8; VRF_SECRET_KEY_BYTES]> {
    secret_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("VRF secret key must be {VRF_SECRET_KEY_BYTES} bytes"))
}

pub(crate) fn proof_array(proof: &[u8]) -> Result<[u8; VRF_PROOF_BYTES]> {
    proof
        .try_into()
        .map_err(|_| anyhow::anyhow!("VRF proof must be {VRF_PROOF_BYTES} bytes"))
}

pub(crate) fn normalized_vote_count(vote_count: u64, total_vote_count: u64) -> Result<u16> {
    const VOTES_PROPORTION: u64 = 1000;

    if total_vote_count == 0 {
        bail!("VRF total vote count cannot be zero");
    }
    Ok((vote_count.wrapping_mul(VOTES_PROPORTION) / total_vote_count) as u16)
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

    #[test]
    fn vrf_roundtrip_verifies_and_derives_output() {
        let message = b"rustaxa-vrf";
        let public_key = public_key_from_secret(&SECRET_KEY).unwrap();
        assert!(is_valid_public_key(&public_key).unwrap());

        let proof = prove(&SECRET_KEY, message).unwrap();
        let strict_output = verify_output(&public_key, &proof, message)
            .unwrap()
            .unwrap();
        let compatibility_output = proof_to_hash(&proof).unwrap();

        assert_eq!(strict_output, compatibility_output);
        assert!(
            verify_output(&public_key, &proof, b"other")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn threshold_uses_legacy_little_endian_minstd_sequence() {
        let mut output = [0_u8; VRF_OUTPUT_BYTES];
        output[0] = 0x34;
        output[1] = 0x12;

        assert_eq!(threshold_from_output(&output, 1), 0x1234);
        assert_eq!(
            threshold_from_output(&output, 3),
            [
                0x1234_u16,
                0x1234_u16.wrapping_mul(48271),
                0x1234_u16.wrapping_mul(48271).wrapping_mul(48271)
            ]
            .into_iter()
            .min()
            .unwrap()
        );
    }
}
