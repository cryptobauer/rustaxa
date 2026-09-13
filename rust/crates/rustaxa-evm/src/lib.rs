//! Taraxa concrete execution backend under implementation.
//!
//! FinalChain owns ordered inputs, native business kernels and publication.
//! This crate will own envelope, interpreter, frame and journal mechanics;
//! storage implements the shared concrete read ports. No production crate
//! depends on this crate. Integration is selectable only by isolated tests
//! until a separately authorized and validated routing change.

pub mod contracts;
pub mod input;
