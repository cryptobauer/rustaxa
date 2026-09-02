//! Rust implementations of Taraxa VDF, VRF, and VDF/VRF sortition primitives.
//!
//! The crate keeps compatibility-oriented modules separate from low-level
//! puzzle/prover/verifier code so native consensus can own proof execution
//! without depending on storage or unrelated orchestration.

pub mod config;
pub mod hash;
pub mod prover;
pub mod puzzle;
pub mod sortition;
pub mod vdf;
pub mod vdf_sortition;
pub mod verifier;
pub mod vrf;
