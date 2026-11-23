//! Prepared execution of the four original stateless precompiles.
//!
//! Taraxa's pinned registries all contain ECRECOVER, SHA-256, RIPEMD-160 and
//! identity at addresses 1–4. This module reuses individual pinned REVM
//! primitives without selecting an Ethereum registry or fork. The application
//! dispatcher owns invocation ordering and native registration; the frame owns
//! value transfer, gas forwarding and settlement. Other precompiles remain an
//! explicit unsupported boundary here.

use revm::precompile::{hash, identity, secp256k1};
use rustaxa_types::FinalChainGas;

use crate::contracts::{
    NativeInvocationResult, NativeOutcome, NativePortError, NativeStatus, StatelessGasQuote,
    StatelessInvocation,
};

/// One of the four original primitives shared by every pinned Taraxa registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginalStatelessPrecompile {
    /// Address 1: recover a secp256k1 address, accepting high-S signatures.
    EcRecover,
    /// Address 2: SHA-256 digest.
    Sha256,
    /// Address 3: RIPEMD-160 digest, left padded to 32 bytes.
    Ripemd160,
    /// Address 4: unchanged input bytes.
    Identity,
}

impl OriginalStatelessPrecompile {
    /// Selects only exact addresses 1–4. Returning `None` says nothing about
    /// whether the wider historical native registry contains that address.
    pub fn at_address(address: [u8; 20]) -> Option<Self> {
        if address[..19] != [0; 19] {
            return None;
        }
        match address[19] {
            1 => Some(Self::EcRecover),
            2 => Some(Self::Sha256),
            3 => Some(Self::Ripemd160),
            4 => Some(Self::Identity),
            _ => None,
        }
    }

    fn required_gas(self, input_length: usize) -> Result<FinalChainGas, NativePortError> {
        let (base, per_word) = match self {
            Self::EcRecover => return Ok(3_000_u64.into()),
            Self::Sha256 => (60_u64, 12_u64),
            Self::Ripemd160 => (600, 120),
            Self::Identity => (15, 3),
        };
        let words = u64::try_from(input_length.div_ceil(32)).map_err(|_| {
            NativePortError::Infrastructure("stateless input length overflow".into())
        })?;
        words
            .checked_mul(per_word)
            .and_then(|gas| gas.checked_add(base))
            .map(FinalChainGas::from)
            .ok_or_else(|| NativePortError::Infrastructure("stateless gas overflow".into()))
    }
}

/// A gas quote bound by ownership to the complete immutable invocation.
///
/// Preparation does no cryptographic work and does not read or mutate journal
/// state. Consumption runs at most once and rejects insufficient gas before
/// executing the primitive. The surrounding dispatcher still owns period and
/// stateless-ordinal validation; this helper cannot publish or advance them.
#[derive(Debug)]
pub struct PreparedStatelessCall {
    invocation: StatelessInvocation,
    primitive: OriginalStatelessPrecompile,
    quote: StatelessGasQuote,
}

impl PreparedStatelessCall {
    /// Takes ownership of an exact invocation and quotes its original primitive.
    /// Unsupported code addresses and unrepresentable host lengths/gas return
    /// an infrastructure/domain error without effects. Caller/static/value facts
    /// stay bound even though these four pure primitives do not inspect them.
    pub fn prepare(invocation: StatelessInvocation) -> Result<Self, NativePortError> {
        let primitive =
            OriginalStatelessPrecompile::at_address(invocation.contract).ok_or_else(|| {
                NativePortError::Domain("unsupported original stateless address".into())
            })?;
        let quote = StatelessGasQuote {
            invocation: invocation.id,
            required_gas: primitive.required_gas(invocation.input.len())?,
        };
        Ok(Self {
            invocation,
            primitive,
            quote,
        })
    }

    /// Borrows the exact invocation owned by this prepared operation.
    pub fn invocation(&self) -> &StatelessInvocation {
        &self.invocation
    }

    /// Returns the immutable quote for frame admission/result validation.
    pub fn quote(&self) -> StatelessGasQuote {
        self.quote
    }

    /// Consumes this preparation and executes its primitive if gas is sufficient.
    ///
    /// Insufficient gas performs no primitive work and charges no native gas;
    /// the frame retains the reference's unused child gas. Successful primitive
    /// evaluation consumes exactly the quote and emits no account/raw/log effects.
    /// Invalid ECRECOVER inputs complete successfully with empty output, including
    /// invalid scalar/recovery values; transaction low-S policy is not applied.
    /// Unexpected errors or a changed primitive gas schedule abort the pending
    /// execution instead of becoming a fabricated contract failure.
    pub fn invoke(self) -> Result<NativeInvocationResult, NativePortError> {
        if self.invocation.supplied_gas < self.quote.required_gas {
            return Ok(NativeInvocationResult::InsufficientGas {
                required_gas: self.quote.required_gas,
            });
        }
        let run = match self.primitive {
            OriginalStatelessPrecompile::EcRecover => secp256k1::ec_recover_run,
            OriginalStatelessPrecompile::Sha256 => hash::sha256_run,
            OriginalStatelessPrecompile::Ripemd160 => hash::ripemd160_run,
            OriginalStatelessPrecompile::Identity => identity::identity_run,
        };
        let result =
            run(&self.invocation.input, self.quote.required_gas.as_u64()).map_err(|error| {
                NativePortError::Infrastructure(format!("stateless primitive: {error:?}"))
            })?;
        if result.gas_used != self.quote.required_gas.as_u64() {
            return Err(NativePortError::Infrastructure(
                "stateless primitive gas mismatch".into(),
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
