//! Taraxa's Cacti Falcon-512 signature-verification precompile.
//!
//! This module owns the exact address-`0xfa1c` stateless contract evidenced by
//! the pinned Go implementation. It quotes 1,465 gas plus 6 gas for each
//! 32-byte input word, parses the historical `verify(bytes,bytes,bytes)` ABI,
//! and verifies FN-DSA-512 signatures with the historically compatible
//! `fn-dsa-vrfy` 0.3 implementation. It does not select the Cacti registry or
//! settle CALL-family frames.

use std::borrow::Cow;

use fn_dsa_vrfy::{
    DOMAIN_NONE, FN_DSA_LOGN_512, HASH_ID_RAW, VerifyingKey as _, VerifyingKeyStandard,
    signature_size, vrfy_key_size,
};
use rustaxa_types::FinalChainGas;

use crate::contracts::{
    NativeContractFailure, NativeInvocationResult, NativeOutcome, NativePortError, NativeStatus,
    StatelessGasQuote, StatelessInvocation,
};

/// Exact address of Taraxa's Cacti Falcon-512 verifier.
pub const FALCON_VERIFY_ADDRESS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfa, 0x1c,
];

/// Fixed part of the Falcon-512 action-gas quote.
pub const FALCON_BASE_GAS: u64 = 1_465;

/// Per-32-byte-word part of the Falcon-512 action-gas quote.
pub const FALCON_PER_WORD_GAS: u64 = 6;

const VERIFY_SELECTOR: [u8; 4] = [0xde, 0x8f, 0x50, 0xa1];
const INVALID_INPUT_FORMAT: &str = "invalid input format";
const INVALID_METHOD_SIGNATURE: &str = "invalid method signature";

/// An immutable, single-consumption Falcon-512 invocation and gas quote.
///
/// Preparation accepts only exact address `0xfa1c`, retains every invocation
/// fact, and quotes from the complete ABI input length. It performs no parsing
/// or cryptography. Invocation checks funding before parsing, so underfunded
/// malformed inputs remain ordinary insufficient-gas results.
#[derive(Debug)]
pub struct PreparedFalconCall {
    invocation: StatelessInvocation,
    quote: StatelessGasQuote,
}

impl PreparedFalconCall {
    /// Takes ownership of one exact address-`0xfa1c` invocation and quotes it.
    ///
    /// Unsupported addresses return a domain error. An input-length gas
    /// overflow returns an infrastructure error without parsing the ABI.
    pub fn prepare(invocation: StatelessInvocation) -> Result<Self, NativePortError> {
        if invocation.contract != FALCON_VERIFY_ADDRESS {
            return Err(NativePortError::Domain(
                "unsupported Falcon precompile address".into(),
            ));
        }
        let words = u64::try_from(invocation.input.len().div_ceil(32)).map_err(|_| {
            NativePortError::Infrastructure("Falcon input word count exceeds u64".into())
        })?;
        let required_gas = words
            .checked_mul(FALCON_PER_WORD_GAS)
            .and_then(|gas| gas.checked_add(FALCON_BASE_GAS))
            .ok_or_else(|| NativePortError::Infrastructure("Falcon gas overflow".into()))?;
        Ok(Self {
            quote: StatelessGasQuote {
                invocation: invocation.id,
                required_gas: FinalChainGas::new(required_gas),
            },
            invocation,
        })
    }

    /// Borrows every immutable invocation fact bound to this preparation.
    pub fn invocation(&self) -> &StatelessInvocation {
        &self.invocation
    }

    /// Returns the input-length quote bound to this invocation identity.
    pub fn quote(&self) -> StatelessGasQuote {
        self.quote
    }

    /// Consumes the preparation and runs the historical verifier when funded.
    ///
    /// Inputs shorter than four bytes and wrong selectors are exact Go-shaped
    /// contract failures with empty output. All other malformed ABI, invalid
    /// key/signature, and empty-message cases succeed with `bytes32(1)`.
    /// Historical valid signatures succeed with `bytes32(0)`. ABI offsets and
    /// lengths use their low 64 bits, need not be aligned or ordered, and may
    /// point into any in-bounds part of the post-selector input. Go right-pads
    /// that post-selector view by four zero bytes; trailing input is otherwise
    /// ignored. Every funded completion consumes the complete quote and emits
    /// no account, raw-storage, or log effects.
    pub fn invoke(self) -> Result<NativeInvocationResult, NativePortError> {
        if self.invocation.supplied_gas < self.quote.required_gas {
            return Ok(NativeInvocationResult::InsufficientGas {
                required_gas: self.quote.required_gas,
            });
        }
        let input = &self.invocation.input;
        if input.len() < VERIFY_SELECTOR.len() {
            return Ok(self.failure(INVALID_INPUT_FORMAT));
        }
        if input[..VERIFY_SELECTOR.len()] != VERIFY_SELECTOR {
            return Ok(self.failure(INVALID_METHOD_SIGNATURE));
        }
        let mut body = input[VERIFY_SELECTOR.len()..].to_vec();
        body.resize(input.len(), 0);
        match verify_abi(&body) {
            Ok(output) => Ok(self.success(output)),
            Err(AbiFailure::ReferencePanic) => Err(NativePortError::Infrastructure(
                "Falcon reference ABI would panic".into(),
            )),
        }
    }

    fn success(&self, output: [u8; 32]) -> NativeInvocationResult {
        NativeInvocationResult::Completed(NativeOutcome {
            status: NativeStatus::Success,
            gas_used: self.quote.required_gas,
            output: output.to_vec(),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AbiFailure {
    ReferencePanic,
}

fn verify_abi(input: &[u8]) -> Result<[u8; 32], AbiFailure> {
    let mut result = [0_u8; 32];
    result[31] = 1;
    let Some(header) = input.get(..96) else {
        return Ok(result);
    };
    let signature_offset = low_u64(&header[..32]);
    let key_offset = low_u64(&header[32..64]);
    let message_offset = low_u64(&header[64..96]);
    // Go rejects any zero offset before attempting even the signature slice.
    if signature_offset == 0 || key_offset == 0 || message_offset == 0 {
        return Ok(result);
    }
    let Some(signature) = dynamic(input, signature_offset)? else {
        return Ok(result);
    };
    if signature.declared_length != signature_size(FN_DSA_LOGN_512) as u64 {
        return Ok(result);
    }
    let Some(key) = dynamic(input, key_offset)? else {
        return Ok(result);
    };
    if key.declared_length != vrfy_key_size(FN_DSA_LOGN_512) as u64 {
        return Ok(result);
    }
    let Some(message) = dynamic(input, message_offset)? else {
        return Ok(result);
    };
    let valid = VerifyingKeyStandard::decode(&key.bytes).is_some_and(|key| {
        key.verify(&signature.bytes, &DOMAIN_NONE, &HASH_ID_RAW, &message.bytes)
    });
    if valid {
        result[31] = 0;
    }
    Ok(result)
}

// Keep the original length separate from getData's clamped/padded byte view.
// Go validates fixed-size fields against the former after constructing the latter.
struct AbiField<'a> {
    bytes: Cow<'a, [u8]>,
    declared_length: u64,
}

fn dynamic(input: &[u8], offset: u64) -> Result<Option<AbiField<'_>>, AbiFailure> {
    if offset == 0 {
        return Ok(None);
    }
    if go_len(input) < go_int(offset).wrapping_add(32) {
        return Ok(None);
    }
    let length = low_u64(&go_data(input, offset, 32)?);
    if length == 0 {
        return Ok(None);
    }
    let declared_end = go_int(offset).wrapping_add(32).wrapping_add(go_int(length));
    if go_len(input) < declared_end {
        return Ok(None);
    }
    go_data(input, offset.wrapping_add(32), length).map(|bytes| {
        Some(AbiField {
            bytes,
            declared_length: length,
        })
    })
}

fn go_data(input: &[u8], start: u64, size: u64) -> Result<Cow<'_, [u8]>, AbiFailure> {
    let input_len = u64::try_from(input.len()).expect("input length fits u64");
    let start = start.min(input_len);
    let mut end = start.wrapping_add(size);
    if end > input_len {
        end = input_len;
    }
    if end < start {
        return Err(AbiFailure::ReferencePanic);
    }
    let bytes = &input[usize::try_from(start).unwrap()..usize::try_from(end).unwrap()];
    let signed_size = go_int(size);
    if signed_size < 0 || bytes.len() >= usize::try_from(signed_size).unwrap() {
        return Ok(Cow::Borrowed(bytes));
    }
    let target_len = usize::try_from(signed_size).unwrap();
    let mut padded = Vec::new();
    // A wrapped ABI bound can request Go's make([]byte, MaxInt). Reserve
    // fallibly before copying or padding so host allocation failure cannot
    // abort the process instead of surfacing through the infrastructure port.
    padded
        .try_reserve_exact(target_len)
        .map_err(|_| AbiFailure::ReferencePanic)?;
    padded.extend_from_slice(bytes);
    padded.resize(target_len, 0);
    Ok(Cow::Owned(padded))
}

fn go_len(input: &[u8]) -> i64 {
    i64::try_from(input.len()).expect("host slice length fits Go int")
}

fn go_int(value: u64) -> i64 {
    value as i64
}

fn low_u64(word: &[u8]) -> u64 {
    u64::from_be_bytes(
        word[word.len() - 8..]
            .try_into()
            .expect("eight-byte suffix"),
    )
}
