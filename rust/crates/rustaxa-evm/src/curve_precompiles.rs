//! Prepared execution of Taraxa's original BN254 and Ficus BLAKE2F primitives.
//!
//! This module selects exact addresses 6–9 and preserves the pinned Go gas,
//! padding, validation and error surface while reusing REVM's cryptographic
//! primitives. It does not choose a historical registry or activate address 9;
//! the application dispatcher owns those decisions. Calls are pure and return
//! no account, raw-storage or log effects.

use revm::precompile::{PrecompileHalt, blake2, bn254};
use rustaxa_types::FinalChainGas;

use crate::contracts::{
    NativeContractFailure, NativeInvocationResult, NativeOutcome, NativePortError, NativeStatus,
    StatelessGasQuote, StatelessInvocation,
};

const BN254_ADD_GAS: u64 = 500;
const BN254_MUL_GAS: u64 = 40_000;
const BN254_PAIR_BASE_GAS: u64 = 100_000;
const BN254_PAIR_PER_POINT_GAS: u64 = 80_000;
const BN254_PAIR_ELEMENT_LENGTH: usize = 192;
const BLAKE2F_INPUT_LENGTH: usize = 213;
const BN254_FIELD_MODULUS: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
];

/// One of the original curve-related primitives at addresses 6–9.
///
/// The first three variants are present in Californicum. `Blake2F` first
/// appears in Ficus, but selecting it here does not establish that Ficus is
/// active for a caller's period.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginalCurvePrecompile {
    /// Address 6: BN254 G1 point addition.
    Bn254Add,
    /// Address 7: BN254 G1 scalar multiplication.
    Bn254Mul,
    /// Address 8: BN254 pairing product check.
    Bn254Pairing,
    /// Address 9: BLAKE2b compression with an input-selected round count.
    Blake2F,
}

impl OriginalCurvePrecompile {
    /// Selects only exact addresses 6–9.
    ///
    /// Returning `Some(Blake2F)` does not activate Ficus. Returning `None`
    /// makes no statement about later Taraxa precompile registries.
    pub fn at_address(address: [u8; 20]) -> Option<Self> {
        if address[..19] != [0; 19] {
            return None;
        }
        match address[19] {
            6 => Some(Self::Bn254Add),
            7 => Some(Self::Bn254Mul),
            8 => Some(Self::Bn254Pairing),
            9 => Some(Self::Blake2F),
            _ => None,
        }
    }

    fn required_gas(self, input: &[u8]) -> Result<FinalChainGas, NativePortError> {
        let gas = match self {
            Self::Bn254Add => BN254_ADD_GAS,
            Self::Bn254Mul => BN254_MUL_GAS,
            Self::Bn254Pairing => {
                let pairs =
                    u64::try_from(input.len() / BN254_PAIR_ELEMENT_LENGTH).map_err(|_| {
                        NativePortError::Infrastructure(
                            "BN254 pairing input length overflow".into(),
                        )
                    })?;
                pairs
                    .checked_mul(BN254_PAIR_PER_POINT_GAS)
                    .and_then(|value| value.checked_add(BN254_PAIR_BASE_GAS))
                    .ok_or_else(|| {
                        NativePortError::Infrastructure("BN254 pairing gas overflow".into())
                    })?
            }
            Self::Blake2F if input.len() != BLAKE2F_INPUT_LENGTH => 0,
            Self::Blake2F => {
                u32::from_be_bytes(input[..4].try_into().expect("four-byte prefix")) as u64
            }
        };
        Ok(gas.into())
    }
}

/// An immutable, single-consumption quote for an exact address 6–9 call.
///
/// Preparation performs no curve or compression work. Invocation rejects
/// insufficient gas before validating or executing the primitive. Funded
/// malformed input is a reference-shaped contract failure which consumes the
/// quote; unexpected backend errors abort the pending execution.
#[derive(Debug)]
pub struct PreparedCurvePrecompileCall {
    invocation: StatelessInvocation,
    primitive: OriginalCurvePrecompile,
    quote: StatelessGasQuote,
}

impl PreparedCurvePrecompileCall {
    /// Takes ownership of every invocation fact and computes Taraxa's quote.
    ///
    /// Address selection is exact. Pairing quotes use the number of complete
    /// 192-byte elements even when execution will reject a trailing remainder.
    /// BLAKE2F malformed lengths quote zero, matching the pinned Go primitive.
    pub fn prepare(invocation: StatelessInvocation) -> Result<Self, NativePortError> {
        let primitive =
            OriginalCurvePrecompile::at_address(invocation.contract).ok_or_else(|| {
                NativePortError::Domain("unsupported curve precompile address".into())
            })?;
        let quote = StatelessGasQuote {
            invocation: invocation.id,
            required_gas: primitive.required_gas(&invocation.input)?,
        };
        Ok(Self {
            invocation,
            primitive,
            quote,
        })
    }

    /// Borrows the complete invocation bound to this prepared operation.
    pub fn invocation(&self) -> &StatelessInvocation {
        &self.invocation
    }

    /// Returns the immutable quote used for admission and result validation.
    pub fn quote(&self) -> StatelessGasQuote {
        self.quote
    }

    /// Executes the prepared primitive once when sufficiently funded.
    ///
    /// Add and multiply right-pad short inputs and ignore trailing bytes.
    /// Pairing requires a multiple of 192 bytes; empty input returns true.
    /// BLAKE2F requires exactly 213 bytes and a final flag of zero or one.
    /// All successful and business-failure outcomes consume exactly the quote
    /// and contain no state effects. Infrastructure errors expose no outcome.
    pub fn invoke(self) -> Result<NativeInvocationResult, NativePortError> {
        if self.invocation.supplied_gas < self.quote.required_gas {
            return Ok(NativeInvocationResult::InsufficientGas {
                required_gas: self.quote.required_gas,
            });
        }

        let result = match self.primitive {
            OriginalCurvePrecompile::Bn254Add => bn254::run_add(
                &self.invocation.input,
                BN254_ADD_GAS,
                self.quote.required_gas.as_u64(),
            ),
            OriginalCurvePrecompile::Bn254Mul => bn254::run_mul(
                &self.invocation.input,
                BN254_MUL_GAS,
                self.quote.required_gas.as_u64(),
            ),
            OriginalCurvePrecompile::Bn254Pairing => {
                if !self
                    .invocation
                    .input
                    .len()
                    .is_multiple_of(BN254_PAIR_ELEMENT_LENGTH)
                {
                    return Ok(self.failure("bad elliptic curve pairing size"));
                }
                bn254::run_pair(
                    &self.invocation.input,
                    BN254_PAIR_PER_POINT_GAS,
                    BN254_PAIR_BASE_GAS,
                    self.quote.required_gas.as_u64(),
                )
            }
            OriginalCurvePrecompile::Blake2F => {
                if self.invocation.input.len() != BLAKE2F_INPUT_LENGTH {
                    return Ok(self.failure("invalid input length"));
                }
                if !matches!(self.invocation.input[212], 0 | 1) {
                    return Ok(self.failure("invalid final flag"));
                }
                blake2::run(&self.invocation.input, self.quote.required_gas.as_u64())
            }
        };

        match result {
            Ok(output) => {
                if output.gas_used != self.quote.required_gas.as_u64() {
                    return Err(NativePortError::Infrastructure(
                        "curve precompile primitive gas mismatch".into(),
                    ));
                }
                Ok(self.success(output.bytes.to_vec()))
            }
            Err(error) => match self.compatibility_error(&error) {
                Some(error) => Ok(self.failure(error)),
                None => Err(NativePortError::Infrastructure(format!(
                    "curve precompile primitive: {error:?}"
                ))),
            },
        }
    }

    fn success(&self, output: Vec<u8>) -> NativeInvocationResult {
        NativeInvocationResult::Completed(NativeOutcome {
            status: NativeStatus::Success,
            gas_used: self.quote.required_gas,
            output,
            account_mutations: Vec::new(),
            raw_mutations: Vec::new(),
            logs: Vec::new(),
            diagnostic: None,
        })
    }

    fn failure(&self, error: &str) -> NativeInvocationResult {
        NativeInvocationResult::Completed(NativeOutcome {
            status: NativeStatus::ContractFailure(NativeContractFailure {
                error: error.into(),
            }),
            gas_used: self.quote.required_gas,
            output: Vec::new(),
            account_mutations: Vec::new(),
            raw_mutations: Vec::new(),
            logs: Vec::new(),
            diagnostic: None,
        })
    }

    fn compatibility_error(&self, error: &PrecompileHalt) -> Option<&'static str> {
        match error {
            PrecompileHalt::Bn254AffineGFailedToCreate => Some("bn256: malformed point"),
            PrecompileHalt::Bn254FieldPointNotAMember => {
                first_noncanonical_coordinate(self.primitive, &self.invocation.input)
            }
            // Length and BLAKE2F validation are handled explicitly above so
            // their Go text cannot silently drift with REVM's display strings.
            _ => None,
        }
    }
}

fn first_noncanonical_coordinate(
    primitive: OriginalCurvePrecompile,
    input: &[u8],
) -> Option<&'static str> {
    let coordinate_count = match primitive {
        OriginalCurvePrecompile::Bn254Add => 4,
        OriginalCurvePrecompile::Bn254Mul => 2,
        OriginalCurvePrecompile::Bn254Pairing => input.len() / 32,
        OriginalCurvePrecompile::Blake2F => return None,
    };
    for coordinate_index in 0..coordinate_count {
        let offset = coordinate_index * 32;
        let mut coordinate = [0_u8; 32];
        let available = input.len().saturating_sub(offset).min(32);
        if available != 0 {
            coordinate[..available].copy_from_slice(&input[offset..offset + available]);
        }
        match coordinate.cmp(&BN254_FIELD_MODULUS) {
            std::cmp::Ordering::Equal => return Some("bn256: coordinate equals modulus"),
            std::cmp::Ordering::Greater => return Some("bn256: coordinate exceeds modulus"),
            std::cmp::Ordering::Less => {}
        }
    }
    None
}
