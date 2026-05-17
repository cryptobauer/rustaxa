//! CXX bridge adapters for Rust legacy transaction envelope inspection.
//!
//! The domain semantics live in `rustaxa-types`; this module converts the
//! inspected envelope into plain CXX payloads that C++ shims can use while
//! public APIs still materialize legacy `Transaction` objects.

use crate::ffi::rustaxa_ffi::LegacyTransactionInspection;
use anyhow::Result;
use ethereum_types::H160;
use rustaxa_types::LegacyTransactionEnvelope;

const LEGACY_TRANSACTION_SOURCE_REGULAR: u8 = 0;
const LEGACY_TRANSACTION_SOURCE_SYSTEM: u8 = 1;

/// Inspects one legacy transaction RLP payload through the shared Rust envelope.
///
/// `source = 0` decodes a regular signed transaction and reports invalid
/// signatures as `sender_found = false`. `source = 1` decodes a Taraxa system
/// transaction with the fixed system sender. Malformed RLP and arithmetic
/// overflow are returned as errors with stable bridge context.
pub fn inspect_legacy_transaction_rlp(
    tx_rlp: Vec<u8>,
    source: u8,
) -> Result<LegacyTransactionInspection> {
    let envelope = match source {
        LEGACY_TRANSACTION_SOURCE_REGULAR => LegacyTransactionEnvelope::decode(&tx_rlp),
        LEGACY_TRANSACTION_SOURCE_SYSTEM => LegacyTransactionEnvelope::decode_system(&tx_rlp),
        _ => anyhow::bail!("TX_LEGACY_SOURCE_INVALID"),
    }?;
    legacy_transaction_inspection_from_envelope(envelope)
}

pub(crate) fn legacy_transaction_inspection_from_envelope(
    envelope: LegacyTransactionEnvelope,
) -> Result<LegacyTransactionInspection> {
    let cost = envelope.cost()?;
    let receiver = envelope.receiver.unwrap_or_else(H160::zero);
    let sender = envelope.sender.unwrap_or_else(H160::zero);
    let data_size = envelope.data.len();

    Ok(LegacyTransactionInspection {
        hash: envelope.hash.0,
        sender_found: envelope.sender.is_some(),
        sender: sender.0,
        signature_valid: envelope.signature_valid,
        nonce: envelope.nonce.to_big_endian(),
        gas_price: envelope.gas_price.to_big_endian(),
        gas_limit: envelope.gas,
        receiver_found: envelope.receiver.is_some(),
        receiver: receiver.0,
        value: envelope.value.to_big_endian(),
        data: envelope.data,
        data_size,
        chain_id: envelope.chain_id,
        intrinsic_gas_covered: envelope.intrinsic_gas_covered,
        cost: cost.to_big_endian(),
        tx_rlp: envelope.rlp,
    })
}
