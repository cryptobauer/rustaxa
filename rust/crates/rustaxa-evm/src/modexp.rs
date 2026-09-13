//! Taraxa's address-5 modular exponentiation with the original gas schedule.
//!
//! Gas uses all 256 bits of each declared length, while the pinned Go execution
//! path uses their low 64 bits. Ethereum registry wrappers do not preserve that
//! distinction. This adapter owns length handling and quoting and reuses only
//! REVM's existing modular arithmetic primitive. It performs no registration,
//! journal mutation, frame settlement or invocation-sequence allocation.

use num_bigint::BigUint;
use rustaxa_types::FinalChainGas;

use crate::contracts::{
    NativeInvocationResult, NativeOutcome, NativePortError, NativeStatus, StatelessGasQuote,
    StatelessInvocation,
};

fn header(input: &[u8], offset: usize) -> BigUint {
    let mut word = [0_u8; 32];
    let available = input.len().saturating_sub(offset).min(32);
    if available != 0 {
        word[..available].copy_from_slice(&input[offset..offset + available]);
    }
    BigUint::from_bytes_be(&word)
}

fn low_u64(value: &BigUint) -> u64 {
    value.iter_u64_digits().next().unwrap_or(0)
}

fn quote(input: &[u8]) -> FinalChainGas {
    let base = header(input, 0);
    let exponent = header(input, 32);
    let modulus = header(input, 64);
    let data = input.get(96..).unwrap_or_default();
    let mut head = [0_u8; 32];
    let head_length = if exponent > BigUint::from(32_u8) {
        32
    } else {
        low_u64(&exponent) as usize
    };
    // Compare the full declared base length before narrowing its offset.
    if base < BigUint::from(data.len()) {
        let offset = low_u64(&base) as usize;
        let available = (data.len() - offset).min(head_length);
        head[..available].copy_from_slice(&data[offset..offset + available]);
    }
    let msb = BigUint::from_bytes_be(&head[..head_length])
        .bits()
        .saturating_sub(1);
    let mut adjusted = if exponent > BigUint::from(32_u8) {
        (exponent - 32_u8) * 8_u8
    } else {
        BigUint::default()
    };
    adjusted += msb;
    let length = base.max(modulus);
    let square = &length * &length;
    let complexity = if length <= BigUint::from(64_u8) {
        square
    } else if length <= BigUint::from(1024_u16) {
        square / 4_u8 + &length * 96_u8 - 3072_u16
    } else {
        square / 16_u8 + &length * 480_u16 - 199680_u32
    };
    let gas = complexity * adjusted.max(BigUint::from(1_u8)) / 20_u8;
    if gas.bits() > 64 {
        u64::MAX.into()
    } else {
        low_u64(&gas).into()
    }
}

fn zeros(length: u64) -> Result<Vec<u8>, NativePortError> {
    let length = usize::try_from(length)
        .map_err(|_| NativePortError::Infrastructure("modexp host length overflow".into()))?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|error| NativePortError::Infrastructure(format!("modexp allocation: {error}")))?;
    result.resize(length, 0);
    Ok(result)
}

fn operand(input: &[u8], offset: u64, length: u64) -> Result<Vec<u8>, NativePortError> {
    let mut result = zeros(length)?;
    let offset = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(input.len());
    let available = (input.len() - offset).min(result.len());
    result[..available].copy_from_slice(&input[offset..offset + available]);
    Ok(result)
}

/// An immutable, single-consumption quote for exact address 5.
///
/// Preparation reads at most the header and exponent head and allocates no
/// declared operand buffers. All invocation facts remain owned and bound even
/// though the pure arithmetic ignores caller, value and static mode. The caller
/// still validates period/sequence and owns child-gas and frame settlement.
#[derive(Debug)]
pub struct PreparedModexpCall {
    invocation: StatelessInvocation,
    quote: StatelessGasQuote,
}

impl PreparedModexpCall {
    /// Owns and quotes an address-5 invocation. Other code addresses return a
    /// domain error without effects. Missing input bytes are right-padded with
    /// zero; full-width length arithmetic saturates only the final gas to u64.
    pub fn prepare(invocation: StatelessInvocation) -> Result<Self, NativePortError> {
        if invocation.contract[..19] != [0; 19] || invocation.contract[19] != 5 {
            return Err(NativePortError::Domain("unsupported modexp address".into()));
        }
        let quote = StatelessGasQuote {
            invocation: invocation.id,
            required_gas: quote(&invocation.input),
        };
        Ok(Self { invocation, quote })
    }

    /// Borrows every exact input fact retained by this preparation.
    pub fn invocation(&self) -> &StatelessInvocation {
        &self.invocation
    }

    /// Returns the bound quote for gas admission and outcome validation.
    pub fn quote(&self) -> StatelessGasQuote {
        self.quote
    }

    /// Executes once if funded, returning exactly modulus-length output and no
    /// state or log effects. Insufficient gas performs no operand allocation or
    /// arithmetic. Declared execution lengths use their low 64 bits, as in Go;
    /// a zero base and modulus length returns empty before reading the exponent.
    /// Zero modulus returns zero bytes. Host allocation/primitive failures abort
    /// pending execution as infrastructure errors, never fabricated EVM errors.
    pub fn invoke(self) -> Result<NativeInvocationResult, NativePortError> {
        if self.invocation.supplied_gas < self.quote.required_gas {
            return Ok(NativeInvocationResult::InsufficientGas {
                required_gas: self.quote.required_gas,
            });
        }
        let input = &self.invocation.input;
        let base_length = low_u64(&header(input, 0));
        let exponent_length = low_u64(&header(input, 32));
        let modulus_length = low_u64(&header(input, 64));
        let output = if base_length == 0 && modulus_length == 0 {
            Vec::new()
        } else {
            let input = input.get(96..).unwrap_or_default();
            let base = operand(input, 0, base_length)?;
            let exponent = operand(input, base_length, exponent_length)?;
            let modulus = operand(
                input,
                base_length.wrapping_add(exponent_length),
                modulus_length,
            )?;
            let mut output = zeros(modulus_length)?;
            if modulus.iter().any(|byte| *byte != 0) {
                let result = revm::precompile::crypto()
                    .modexp(&base, &exponent, &modulus)
                    .map_err(|error| {
                        NativePortError::Infrastructure(format!("modexp primitive: {error:?}"))
                    })?;
                let start = output.len().checked_sub(result.len()).ok_or_else(|| {
                    NativePortError::Infrastructure("modexp primitive output width".into())
                })?;
                output[start..].copy_from_slice(&result);
            }
            output
        };
        Ok(NativeInvocationResult::Completed(NativeOutcome {
            status: NativeStatus::Success,
            gas_used: self.quote.required_gas,
            output,
            account_mutations: Vec::new(),
            raw_mutations: Vec::new(),
            logs: Vec::new(),
            diagnostic: None,
        }))
    }
}
