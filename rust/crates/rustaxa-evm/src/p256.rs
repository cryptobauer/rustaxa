//! Taraxa's Cacti P-256 signature-verification precompile.
//!
//! This module owns the exact address-`0x0100` primitive contract evidenced by
//! the pinned Taraxa Go implementation: a fixed 6,900-gas quote, an exact
//! 160-byte input, a 32-byte true word for a valid signature, and empty output
//! for every invalid length, signature, or public key. It does not select a
//! historical registry or enable Cacti; the application profile retains that
//! authority, while frame settlement retains value, gas forwarding and
//! rollback.

use revm::precompile::secp256r1;
use rustaxa_types::FinalChainGas;

use crate::contracts::{
    NativeInvocationResult, NativeOutcome, NativePortError, NativeStatus, StatelessGasQuote,
    StatelessInvocation,
};

/// Exact address of Taraxa's Cacti P-256 verifier.
pub const P256_VERIFY_ADDRESS: [u8; 20] =
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0];

/// Fixed action-gas requirement of the Cacti P-256 verifier.
pub const P256_VERIFY_GAS: FinalChainGas = FinalChainGas::new(6_900);

/// An immutable, single-consumption P-256 invocation and gas quote.
///
/// Construction accepts only the exact `0x0100` code address. It binds every
/// call-context field without validating the signature or reading state.
/// Invocation checks funding before crypto work, then delegates only P-256
/// verification to the pinned REVM primitive using Taraxa's 6,900-gas cost.
#[derive(Debug)]
pub struct PreparedP256Call {
    invocation: StatelessInvocation,
    quote: StatelessGasQuote,
}

impl PreparedP256Call {
    /// Takes ownership of one exact address-`0x0100` invocation.
    ///
    /// An address mismatch is a domain error. Input length and signature/public
    /// key validity intentionally remain normal funded execution outcomes and
    /// therefore do not fail preparation.
    pub fn prepare(invocation: StatelessInvocation) -> Result<Self, NativePortError> {
        if invocation.contract != P256_VERIFY_ADDRESS {
            return Err(NativePortError::Domain(
                "unsupported P-256 precompile address".into(),
            ));
        }
        Ok(Self {
            quote: StatelessGasQuote {
                invocation: invocation.id,
                required_gas: P256_VERIFY_GAS,
            },
            invocation,
        })
    }

    /// Borrows the complete immutable invocation.
    pub fn invocation(&self) -> &StatelessInvocation {
        &self.invocation
    }

    /// Returns the fixed gas quote bound to this invocation identity.
    pub fn quote(&self) -> StatelessGasQuote {
        self.quote
    }

    /// Consumes the preparation and runs the verifier when fully funded.
    ///
    /// Underfunding returns `InsufficientGas` without crypto work. Every funded
    /// invalid input completes successfully with empty output, matching Go;
    /// successful verification returns the canonical 32-byte true word. The
    /// pure primitive produces no account, raw-storage or log effects. Backend
    /// errors or an unexpected gas value abort as infrastructure failures.
    pub fn invoke(self) -> Result<NativeInvocationResult, NativePortError> {
        if self.invocation.supplied_gas < self.quote.required_gas {
            return Ok(NativeInvocationResult::InsufficientGas {
                required_gas: self.quote.required_gas,
            });
        }
        let result =
            secp256r1::p256_verify_osaka(&self.invocation.input, self.quote.required_gas.as_u64())
                .map_err(|error| {
                    NativePortError::Infrastructure(format!("P-256 primitive: {error:?}"))
                })?;
        if result.gas_used != self.quote.required_gas.as_u64() {
            return Err(NativePortError::Infrastructure(
                "P-256 primitive gas mismatch".into(),
            ));
        }
        Ok(NativeInvocationResult::Completed(NativeOutcome {
            status: NativeStatus::Success,
            gas_used: self.quote.required_gas,
            output: result.bytes.to_vec(),
            account_mutations: Vec::new(),
            raw_mutations: Vec::new(),
            logs: Vec::new(),
            diagnostic: None,
        }))
    }
}
