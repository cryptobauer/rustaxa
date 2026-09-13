//! Prepared execution of Taraxa's Ficus and Cacti BLS12-381 precompiles.
//!
//! This module owns the historical address remap, native gas schedule, exact
//! input validation and Go-shaped error surface. Curve arithmetic is composed
//! from the repository-pinned REVM BLS primitives rather than an Ethereum
//! precompile registry. Registry activation remains the caller's decision.
//! Calls are pure and produce no account, raw-storage or log effects.

use num_bigint::{BigInt, BigUint, Sign};
use revm::precompile::{PrecompileHalt, bls12_381};
use rustaxa_types::FinalChainGas;

use crate::contracts::{
    NativeContractFailure, NativeInvocationResult, NativeOutcome, NativePortError, NativeStatus,
    StatelessGasQuote, StatelessInvocation,
};

const G1_ADD_GAS: u64 = 600;
const G1_MUL_GAS: u64 = 12_000;
const G2_ADD_GAS: u64 = 4_500;
const G2_MUL_GAS: u64 = 55_000;
const PAIRING_BASE_GAS: u64 = 115_000;
const PAIRING_PER_PAIR_GAS: u64 = 23_000;
const MAP_G1_GAS: u64 = 5_500;
const MAP_G2_GAS: u64 = 110_000;

const G1_LENGTH: usize = 128;
const G2_LENGTH: usize = 256;
const SCALAR_LENGTH: usize = 32;
const G1_ADD_INPUT_LENGTH: usize = G1_LENGTH * 2;
const G1_MUL_INPUT_LENGTH: usize = G1_LENGTH + SCALAR_LENGTH;
const G2_ADD_INPUT_LENGTH: usize = G2_LENGTH * 2;
const G2_MUL_INPUT_LENGTH: usize = G2_LENGTH + SCALAR_LENGTH;
const PAIRING_INPUT_LENGTH: usize = G1_LENGTH + G2_LENGTH;
const MAP_G1_INPUT_LENGTH: usize = 64;
const MAP_G2_INPUT_LENGTH: usize = 128;

const MULTIEXP_DISCOUNT: [u64; 128] = [
    1200, 888, 764, 641, 594, 547, 500, 453, 438, 423, 408, 394, 379, 364, 349, 334, 330, 326, 322,
    318, 314, 310, 306, 302, 298, 294, 289, 285, 281, 277, 273, 269, 268, 266, 265, 263, 262, 260,
    259, 257, 256, 254, 253, 251, 250, 248, 247, 245, 244, 242, 241, 239, 238, 236, 235, 233, 232,
    231, 229, 228, 226, 225, 223, 222, 221, 220, 219, 219, 218, 217, 216, 216, 215, 214, 213, 213,
    212, 211, 211, 210, 209, 208, 208, 207, 206, 205, 205, 204, 203, 202, 202, 201, 200, 199, 199,
    198, 197, 196, 196, 195, 194, 193, 193, 192, 191, 191, 190, 189, 188, 188, 187, 186, 185, 185,
    184, 183, 182, 182, 181, 180, 179, 179, 178, 177, 176, 176, 175, 174,
];

/// Taraxa registry epochs which expose BLS12-381 precompiles.
///
/// The type identifies a table shape only. It does not decide which epoch is
/// active at a period; the historical profile classifier supplies that fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlsRegistry {
    /// Nine Ficus operations at addresses 11 through 19.
    Ficus,
    /// Seven Cacti operations remapped to addresses 11 through 17.
    Cacti,
}

/// One semantic operation in Taraxa's two historical BLS12-381 registries.
///
/// Addition accepts on-curve points without subgroup checks. Multiplication
/// and multi-exponentiation preserve the same pinned Go behavior. Pairing is
/// the operation which enforces G1 and G2 subgroup membership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlsPrecompile {
    /// Add two padded G1 points.
    G1Add,
    /// Multiply one padded G1 point by a 256-bit scalar; Ficus only.
    G1Mul,
    /// Sum one or more padded G1 point/scalar products.
    G1MultiExp,
    /// Add two padded G2 points.
    G2Add,
    /// Multiply one padded G2 point by a 256-bit scalar; Ficus only.
    G2Mul,
    /// Sum one or more padded G2 point/scalar products.
    G2MultiExp,
    /// Check a non-empty sequence of G1/G2 pairing pairs.
    Pairing,
    /// Map one padded base-field element to G1.
    MapG1,
    /// Map one padded extension-field element to G2.
    MapG2,
}

impl BlsPrecompile {
    /// Resolves an exact 20-byte address in the selected Taraxa registry.
    ///
    /// Ficus maps all nine variants in enum order to `0x0b..=0x13`. Cacti
    /// omits the two single-multiply variants and maps addition, multi-exp,
    /// pairing and maps to `0x0b..=0x11`. Lookalike addresses with a nonzero
    /// high byte and absent Cacti addresses return `None`.
    pub fn at_address(registry: BlsRegistry, address: [u8; 20]) -> Option<Self> {
        if address[..19] != [0; 19] {
            return None;
        }
        match (registry, address[19]) {
            (BlsRegistry::Ficus, 0x0b) | (BlsRegistry::Cacti, 0x0b) => Some(Self::G1Add),
            (BlsRegistry::Ficus, 0x0c) => Some(Self::G1Mul),
            (BlsRegistry::Ficus, 0x0d) | (BlsRegistry::Cacti, 0x0c) => Some(Self::G1MultiExp),
            (BlsRegistry::Ficus, 0x0e) | (BlsRegistry::Cacti, 0x0d) => Some(Self::G2Add),
            (BlsRegistry::Ficus, 0x0f) => Some(Self::G2Mul),
            (BlsRegistry::Ficus, 0x10) | (BlsRegistry::Cacti, 0x0e) => Some(Self::G2MultiExp),
            (BlsRegistry::Ficus, 0x11) | (BlsRegistry::Cacti, 0x0f) => Some(Self::Pairing),
            (BlsRegistry::Ficus, 0x12) | (BlsRegistry::Cacti, 0x10) => Some(Self::MapG1),
            (BlsRegistry::Ficus, 0x13) | (BlsRegistry::Cacti, 0x11) => Some(Self::MapG2),
            _ => None,
        }
    }

    fn required_gas(self, input: &[u8]) -> Result<FinalChainGas, NativePortError> {
        let gas = match self {
            Self::G1Add => G1_ADD_GAS,
            Self::G1Mul => G1_MUL_GAS,
            Self::G1MultiExp => multiexp_gas(input.len(), G1_MUL_INPUT_LENGTH, G1_MUL_GAS),
            Self::G2Add => G2_ADD_GAS,
            Self::G2Mul => G2_MUL_GAS,
            Self::G2MultiExp => multiexp_gas(input.len(), G2_MUL_INPUT_LENGTH, G2_MUL_GAS),
            Self::Pairing => {
                let pairs = u64::try_from(input.len() / PAIRING_INPUT_LENGTH).map_err(|_| {
                    NativePortError::Infrastructure("BLS pairing input length overflow".into())
                })?;
                pairs
                    .checked_mul(PAIRING_PER_PAIR_GAS)
                    .and_then(|value| value.checked_add(PAIRING_BASE_GAS))
                    .ok_or_else(|| {
                        NativePortError::Infrastructure("BLS pairing gas overflow".into())
                    })?
            }
            Self::MapG1 => MAP_G1_GAS,
            Self::MapG2 => MAP_G2_GAS,
        };
        Ok(gas.into())
    }
}

/// An immutable, single-consumption quote for one exact Taraxa BLS call.
///
/// Preparation binds the registry, address and complete invocation context
/// without curve work. Invocation checks funding before input validation and
/// returns Go-shaped contract failures for funded malformed calls. Backend
/// failures outside the reviewed compatibility set are infrastructure errors.
#[derive(Debug)]
pub struct PreparedBlsCall {
    registry: BlsRegistry,
    invocation: StatelessInvocation,
    primitive: BlsPrecompile,
    quote: StatelessGasQuote,
}

impl PreparedBlsCall {
    /// Selects the exact registry entry and computes Taraxa's native gas quote.
    ///
    /// Multi-exp uses complete element count, returns zero for inputs shorter
    /// than one element, and caps both the count and discount lookup at entry
    /// 128 as the pinned Go code does. Pairing uses complete pair count even
    /// when a trailing remainder will fail during invocation.
    pub fn prepare(
        registry: BlsRegistry,
        invocation: StatelessInvocation,
    ) -> Result<Self, NativePortError> {
        let primitive = BlsPrecompile::at_address(registry, invocation.contract)
            .ok_or_else(|| NativePortError::Domain("unsupported BLS precompile address".into()))?;
        let quote = StatelessGasQuote {
            invocation: invocation.id,
            required_gas: primitive.required_gas(&invocation.input)?,
        };
        Ok(Self {
            registry,
            invocation,
            primitive,
            quote,
        })
    }

    /// Returns the selected table shape without inferring historical activation.
    pub fn registry(&self) -> BlsRegistry {
        self.registry
    }

    /// Borrows the complete invocation bound to this prepared operation.
    pub fn invocation(&self) -> &StatelessInvocation {
        &self.invocation
    }

    /// Returns the semantic operation selected after applying the epoch remap.
    pub fn primitive(&self) -> BlsPrecompile {
        self.primitive
    }

    /// Returns the immutable quote used for admission and result validation.
    pub fn quote(&self) -> StatelessGasQuote {
        self.quote
    }

    /// Executes the prepared BLS primitive once when sufficiently funded.
    ///
    /// Exact lengths are required. Padded field encodings require sixteen zero
    /// top bytes and canonical field values. Infinity is all-zero coordinates.
    /// Addition and multiplication accept on-curve non-subgroup points, while
    /// pairing rejects non-subgroup G1 or G2 points. Successful and failed
    /// calls consume the quote and contain no state effects.
    pub fn invoke(self) -> Result<NativeInvocationResult, NativePortError> {
        if self.invocation.supplied_gas < self.quote.required_gas {
            return Ok(NativeInvocationResult::InsufficientGas {
                required_gas: self.quote.required_gas,
            });
        }

        let result = run_primitive(self.primitive, &self.invocation.input);
        match result {
            Ok(output) => Ok(self.success(output)),
            Err(error) => match compatibility_error(&error) {
                Some(error) => Ok(self.failure(error)),
                None => Err(NativePortError::Infrastructure(format!(
                    "BLS precompile primitive: {error:?}"
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
}

fn multiexp_gas(input_len: usize, element_len: usize, multiplication_gas: u64) -> u64 {
    let count = input_len / element_len;
    if count == 0 {
        return 0;
    }
    let capped = count.min(MULTIEXP_DISCOUNT.len());
    (capped as u64 * multiplication_gas * MULTIEXP_DISCOUNT[capped - 1]) / 1000
}

fn run_primitive(primitive: BlsPrecompile, input: &[u8]) -> Result<Vec<u8>, PrecompileHalt> {
    match primitive {
        BlsPrecompile::G1Add => exact_length(input, G1_ADD_INPUT_LENGTH)
            .and_then(|_| {
                g1_add(
                    input[..G1_LENGTH].try_into().unwrap(),
                    input[G1_LENGTH..].try_into().unwrap(),
                )
            })
            .map(Vec::from),
        BlsPrecompile::G1Mul => {
            exact_length(input, G1_MUL_INPUT_LENGTH)?;
            g1_validate(input[..G1_LENGTH].try_into().unwrap())?;
            match bls12_381::g1_msm::g1_msm(input, u64::MAX) {
                Ok(output) => Ok(output.bytes.to_vec()),
                Err(PrecompileHalt::Bls12381G1NotInSubgroup) => g1_scalar_mul(
                    input[..G1_LENGTH].try_into().unwrap(),
                    input[G1_LENGTH..].try_into().unwrap(),
                )
                .map(Vec::from),
                Err(error) => Err(error),
            }
        }
        BlsPrecompile::G1MultiExp => {
            validate_multiexp_length(input, G1_MUL_INPUT_LENGTH, true)?;
            for pair in input.as_chunks::<G1_MUL_INPUT_LENGTH>().0 {
                g1_validate(pair[..G1_LENGTH].try_into().unwrap())?;
            }
            match bls12_381::g1_msm::g1_msm(input, u64::MAX) {
                Ok(output) => Ok(output.bytes.to_vec()),
                Err(PrecompileHalt::Bls12381G1NotInSubgroup) => g1_multiexp(input).map(Vec::from),
                Err(error) => Err(error),
            }
        }
        BlsPrecompile::G2Add => exact_length(input, G2_ADD_INPUT_LENGTH)
            .and_then(|_| {
                g2_add(
                    input[..G2_LENGTH].try_into().unwrap(),
                    input[G2_LENGTH..].try_into().unwrap(),
                )
            })
            .map(Vec::from),
        BlsPrecompile::G2Mul => {
            exact_length(input, G2_MUL_INPUT_LENGTH)?;
            g2_validate(input[..G2_LENGTH].try_into().unwrap())?;
            match bls12_381::g2_msm::g2_msm(input, u64::MAX) {
                Ok(output) => Ok(output.bytes.to_vec()),
                Err(PrecompileHalt::Bls12381G2NotInSubgroup) => g2_scalar_mul(
                    input[..G2_LENGTH].try_into().unwrap(),
                    input[G2_LENGTH..].try_into().unwrap(),
                )
                .map(Vec::from),
                Err(error) => Err(error),
            }
        }
        BlsPrecompile::G2MultiExp => {
            validate_multiexp_length(input, G2_MUL_INPUT_LENGTH, false)?;
            for pair in input.as_chunks::<G2_MUL_INPUT_LENGTH>().0 {
                g2_validate(pair[..G2_LENGTH].try_into().unwrap())?;
            }
            match bls12_381::g2_msm::g2_msm(input, u64::MAX) {
                Ok(output) => Ok(output.bytes.to_vec()),
                Err(PrecompileHalt::Bls12381G2NotInSubgroup) => g2_multiexp(input).map(Vec::from),
                Err(error) => Err(error),
            }
        }
        BlsPrecompile::Pairing => {
            if input.is_empty() || !input.len().is_multiple_of(PAIRING_INPUT_LENGTH) {
                return Err(PrecompileHalt::Bls12381PairingInputLength);
            }
            for pair in input.as_chunks::<PAIRING_INPUT_LENGTH>().0 {
                let g1: &[u8; G1_LENGTH] = pair[..G1_LENGTH].try_into().unwrap();
                let g2: &[u8; G2_LENGTH] = pair[G1_LENGTH..].try_into().unwrap();
                g1_validate(g1)?;
                g2_validate(g2)?;
                // A one-pair invocation preserves Go's per-pair subgroup
                // validation order before the next pair is decoded.
                bls12_381::pairing::pairing(pair, u64::MAX)?;
            }
            bls12_381::pairing::pairing(input, u64::MAX).map(|output| output.bytes.to_vec())
        }
        BlsPrecompile::MapG1 => {
            exact_length(input, MAP_G1_INPUT_LENGTH)?;
            validate_padded_fp(input)?;
            bls12_381::map_fp_to_g1::map_fp_to_g1(input, u64::MAX)
                .map(|output| output.bytes.to_vec())
        }
        BlsPrecompile::MapG2 => {
            exact_length(input, MAP_G2_INPUT_LENGTH)?;
            validate_padded_fp(&input[..64])?;
            validate_padded_fp(&input[64..])?;
            bls12_381::map_fp2_to_g2::map_fp2_to_g2(input, u64::MAX)
                .map(|output| output.bytes.to_vec())
        }
    }
}

fn exact_length(input: &[u8], expected: usize) -> Result<(), PrecompileHalt> {
    if input.len() == expected {
        Ok(())
    } else {
        Err(PrecompileHalt::Bls12381ScalarInputLength)
    }
}

fn validate_multiexp_length(
    input: &[u8],
    element_length: usize,
    is_g1: bool,
) -> Result<(), PrecompileHalt> {
    if !input.is_empty() && input.len().is_multiple_of(element_length) {
        Ok(())
    } else if is_g1 {
        Err(PrecompileHalt::Bls12381G1MsmInputLength)
    } else {
        Err(PrecompileHalt::Bls12381G2MsmInputLength)
    }
}

fn g1_add(a: &[u8; G1_LENGTH], b: &[u8; G1_LENGTH]) -> Result<[u8; G1_LENGTH], PrecompileHalt> {
    g1_validate(a)?;
    g1_validate(b)?;
    g1_add_raw(a, b)
}

fn g1_validate(point: &[u8; G1_LENGTH]) -> Result<(), PrecompileHalt> {
    validate_padded_fp(&point[..64])?;
    validate_padded_fp(&point[64..])?;
    g1_add_raw(point, &[0; G1_LENGTH]).map(|_| ())
}

fn g1_add_raw(a: &[u8; G1_LENGTH], b: &[u8; G1_LENGTH]) -> Result<[u8; G1_LENGTH], PrecompileHalt> {
    let mut input = [0_u8; G1_ADD_INPUT_LENGTH];
    input[..G1_LENGTH].copy_from_slice(a);
    input[G1_LENGTH..].copy_from_slice(b);
    let output = bls12_381::g1_add::g1_add(&input, u64::MAX)?;
    Ok(output.bytes.as_ref().try_into().unwrap())
}

fn g2_add(a: &[u8; G2_LENGTH], b: &[u8; G2_LENGTH]) -> Result<[u8; G2_LENGTH], PrecompileHalt> {
    g2_validate(a)?;
    g2_validate(b)?;
    g2_add_raw(a, b)
}

fn g2_validate(point: &[u8; G2_LENGTH]) -> Result<(), PrecompileHalt> {
    for coordinate in point.as_chunks::<64>().0 {
        validate_padded_fp(coordinate)?;
    }
    g2_add_raw(point, &[0; G2_LENGTH]).map(|_| ())
}

fn validate_padded_fp(input: &[u8]) -> Result<(), PrecompileHalt> {
    if input[..16].iter().any(|byte| *byte != 0) {
        return Err(PrecompileHalt::Bls12381FpPaddingInvalid);
    }
    if BigUint::from_bytes_be(&input[16..]) >= field_modulus() {
        return Err(PrecompileHalt::NonCanonicalFp);
    }
    Ok(())
}

fn g2_add_raw(a: &[u8; G2_LENGTH], b: &[u8; G2_LENGTH]) -> Result<[u8; G2_LENGTH], PrecompileHalt> {
    let mut input = [0_u8; G2_ADD_INPUT_LENGTH];
    input[..G2_LENGTH].copy_from_slice(a);
    input[G2_LENGTH..].copy_from_slice(b);
    let output = bls12_381::g2_add::g2_add(&input, u64::MAX)?;
    Ok(output.bytes.as_ref().try_into().unwrap())
}

fn g1_scalar_mul(
    point: &[u8; G1_LENGTH],
    scalar: &[u8; SCALAR_LENGTH],
) -> Result<[u8; G1_LENGTH], PrecompileHalt> {
    let validated = g1_add_raw(point, &[0; G1_LENGTH])?;
    let (p_scalar, phi_scalar) = split_go_scalar(scalar);
    let p_point = if p_scalar.sign() == Sign::Minus {
        g1_negate(validated)
    } else {
        validated
    };
    let p_product = g1_unsigned_mul(&p_point, p_scalar.magnitude())?;
    let phi_point = g1_phi(validated);
    let phi_point = if phi_scalar.sign() == Sign::Minus {
        g1_negate(phi_point)
    } else {
        phi_point
    };
    let phi_product = g1_unsigned_mul(&phi_point, phi_scalar.magnitude())?;
    g1_add(&p_product, &phi_product)
}

fn g1_unsigned_mul(
    point: &[u8; G1_LENGTH],
    scalar: &BigUint,
) -> Result<[u8; G1_LENGTH], PrecompileHalt> {
    let mut result = [0_u8; G1_LENGTH];
    let scalar = scalar.to_bytes_be();
    let Some(first_byte) = scalar.iter().position(|byte| *byte != 0) else {
        return Ok(result);
    };
    for (byte_index, byte) in scalar.iter().enumerate().skip(first_byte) {
        let first_bit = if byte_index == first_byte {
            7 - byte.leading_zeros() as usize
        } else {
            7
        };
        for bit in (0..=first_bit).rev() {
            result = g1_add(&result, &result)?;
            if byte & (1 << bit) != 0 {
                result = g1_add(&result, point)?;
            }
        }
    }
    Ok(result)
}

fn g2_scalar_mul(
    point: &[u8; G2_LENGTH],
    scalar: &[u8; SCALAR_LENGTH],
) -> Result<[u8; G2_LENGTH], PrecompileHalt> {
    let validated = g2_add_raw(point, &[0; G2_LENGTH])?;
    let (p_scalar, phi_scalar) = split_go_scalar(scalar);
    let p_point = if p_scalar.sign() == Sign::Minus {
        g2_negate(validated)
    } else {
        validated
    };
    let p_product = g2_unsigned_mul(&p_point, p_scalar.magnitude())?;
    let phi_point = g2_phi(validated);
    let phi_point = if phi_scalar.sign() == Sign::Minus {
        g2_negate(phi_point)
    } else {
        phi_point
    };
    let phi_product = g2_unsigned_mul(&phi_point, phi_scalar.magnitude())?;
    g2_add(&p_product, &phi_product)
}

fn g2_unsigned_mul(
    point: &[u8; G2_LENGTH],
    scalar: &BigUint,
) -> Result<[u8; G2_LENGTH], PrecompileHalt> {
    let mut result = [0_u8; G2_LENGTH];
    let scalar = scalar.to_bytes_be();
    let Some(first_byte) = scalar.iter().position(|byte| *byte != 0) else {
        return Ok(result);
    };
    for (byte_index, byte) in scalar.iter().enumerate().skip(first_byte) {
        let first_bit = if byte_index == first_byte {
            7 - byte.leading_zeros() as usize
        } else {
            7
        };
        for bit in (0..=first_bit).rev() {
            result = g2_add(&result, &result)?;
            if byte & (1 << bit) != 0 {
                result = g2_add(&result, point)?;
            }
        }
    }
    Ok(result)
}

fn split_go_scalar(input: &[u8; SCALAR_LENGTH]) -> (BigInt, BigInt) {
    // These are gnark-crypto v0.12.1's BLS12-381 lattice values. Reproducing
    // its GLV split matters for the historical Go behavior on on-curve points
    // outside the prime subgroup, which single-multiply does not reject.
    let s = BigInt::from_biguint(Sign::Plus, BigUint::from_bytes_be(input));
    let v1 = [
        BigInt::from(1_u8),
        decimal_bigint("228988810152649578064853576960394133504"),
    ];
    let v2 = [
        decimal_bigint("228988810152649578064853576960394133503"),
        BigInt::from(-1_i8),
    ];
    let b1 = decimal_bigint(
        "255699135089535202043525422716183576215815630510683217819334674386498370757524",
    );
    let b2 = decimal_bigint(
        "-58552240701214274452021999096850125581542494224669441691648594964201968916268199467788850628613995194398422433283173",
    );
    let k1 = (&s * b1) >> 512;
    let k2 = (-(&s * b2)) >> 512;
    let closest_0 = &k1 * &v1[0] + &k2 * &v2[0];
    let closest_1: BigInt = k1 * &v1[1] + k2 * &v2[1];
    (s - closest_0, -closest_1)
}

fn decimal_bigint(value: &str) -> BigInt {
    BigInt::parse_bytes(value.as_bytes(), 10).expect("pinned BLS GLV integer")
}

fn field_modulus() -> BigUint {
    BigUint::parse_bytes(
        b"4002409555221667393417789825735904156556882819939007885332058136124031650490837864442687629129015664037894272559787",
        10,
    )
    .expect("BLS base field modulus")
}

fn transform_fp(point: &mut [u8], offset: usize, multiplier: &BigUint) {
    let modulus = field_modulus();
    let value = BigUint::from_bytes_be(&point[offset..offset + 48]);
    let encoded = (value * multiplier % &modulus).to_bytes_be();
    point[offset..offset + 48].fill(0);
    point[offset + 48 - encoded.len()..offset + 48].copy_from_slice(&encoded);
}

fn negate_fp(point: &mut [u8], offset: usize) {
    let modulus = field_modulus();
    let value = BigUint::from_bytes_be(&point[offset..offset + 48]);
    if value == BigUint::from(0_u8) {
        return;
    }
    let encoded = (modulus - value).to_bytes_be();
    point[offset..offset + 48].fill(0);
    point[offset + 48 - encoded.len()..offset + 48].copy_from_slice(&encoded);
}

fn g1_phi(mut point: [u8; G1_LENGTH]) -> [u8; G1_LENGTH] {
    let root = BigUint::parse_bytes(
        b"4002409555221667392624310435006688643935503118305586438271171395842971157480381377015405980053539358417135540939436",
        10,
    )
    .expect("BLS G1 third root");
    transform_fp(&mut point, 16, &root);
    point
}

fn g2_phi(mut point: [u8; G2_LENGTH]) -> [u8; G2_LENGTH] {
    let modulus = field_modulus();
    let g1_root = BigUint::parse_bytes(
        b"4002409555221667392624310435006688643935503118305586438271171395842971157480381377015405980053539358417135540939436",
        10,
    )
    .expect("BLS G1 third root");
    let root = &g1_root * &g1_root % modulus;
    transform_fp(&mut point, 16, &root);
    transform_fp(&mut point, 80, &root);
    point
}

fn g1_negate(mut point: [u8; G1_LENGTH]) -> [u8; G1_LENGTH] {
    if point.iter().any(|byte| *byte != 0) {
        negate_fp(&mut point, 80);
    }
    point
}

fn g2_negate(mut point: [u8; G2_LENGTH]) -> [u8; G2_LENGTH] {
    if point.iter().any(|byte| *byte != 0) {
        negate_fp(&mut point, 144);
        negate_fp(&mut point, 208);
    }
    point
}

fn reduced_scalar(input: &[u8; SCALAR_LENGTH]) -> [u8; SCALAR_LENGTH] {
    let modulus = BigUint::parse_bytes(
        b"52435875175126190479447740508185965837690552500527637822603658699938581184513",
        10,
    )
    .expect("BLS subgroup order");
    let bytes = (BigUint::from_bytes_be(input) % modulus).to_bytes_be();
    let mut result = [0_u8; SCALAR_LENGTH];
    result[SCALAR_LENGTH - bytes.len()..].copy_from_slice(&bytes);
    result
}

fn g1_multiexp(input: &[u8]) -> Result<[u8; G1_LENGTH], PrecompileHalt> {
    let mut result = [0_u8; G1_LENGTH];
    for pair in input.as_chunks::<G1_MUL_INPUT_LENGTH>().0 {
        let point = pair[..G1_LENGTH].try_into().unwrap();
        let scalar = reduced_scalar(pair[G1_LENGTH..].try_into().unwrap());
        let validated = g1_add_raw(point, &[0; G1_LENGTH])?;
        let product = g1_unsigned_mul(&validated, &BigUint::from_bytes_be(&scalar))?;
        result = g1_add(&result, &product)?;
    }
    Ok(result)
}

fn g2_multiexp(input: &[u8]) -> Result<[u8; G2_LENGTH], PrecompileHalt> {
    let mut result = [0_u8; G2_LENGTH];
    for pair in input.as_chunks::<G2_MUL_INPUT_LENGTH>().0 {
        let point = pair[..G2_LENGTH].try_into().unwrap();
        let scalar = reduced_scalar(pair[G2_LENGTH..].try_into().unwrap());
        let validated = g2_add_raw(point, &[0; G2_LENGTH])?;
        let product = g2_unsigned_mul(&validated, &BigUint::from_bytes_be(&scalar))?;
        result = g2_add(&result, &product)?;
    }
    Ok(result)
}

fn compatibility_error(error: &PrecompileHalt) -> Option<&'static str> {
    match error {
        PrecompileHalt::Bls12381ScalarInputLength
        | PrecompileHalt::Bls12381G1AddInputLength
        | PrecompileHalt::Bls12381G1MsmInputLength
        | PrecompileHalt::Bls12381G2AddInputLength
        | PrecompileHalt::Bls12381G2MsmInputLength
        | PrecompileHalt::Bls12381PairingInputLength
        | PrecompileHalt::Bls12381MapFpToG1InputLength
        | PrecompileHalt::Bls12381MapFp2ToG2InputLength
        | PrecompileHalt::Bls12381FpPaddingLength
        | PrecompileHalt::Bls12381G1PaddingLength
        | PrecompileHalt::Bls12381G2PaddingLength => Some("invalid input length"),
        PrecompileHalt::Bls12381FpPaddingInvalid => Some("invalid field element top bytes"),
        PrecompileHalt::NonCanonicalFp => Some("invalid fp.Element encoding"),
        PrecompileHalt::Bls12381G1NotOnCurve | PrecompileHalt::Bls12381G2NotOnCurve => {
            Some("invalid point: not on curve")
        }
        PrecompileHalt::Bls12381G1NotInSubgroup => Some("g1 point is not on correct subgroup"),
        PrecompileHalt::Bls12381G2NotInSubgroup => Some("g2 point is not on correct subgroup"),
        _ => None,
    }
}
