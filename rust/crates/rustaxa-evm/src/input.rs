//! Canonical application transaction inputs adapted to the general executor.
//!
//! Reuses the existing Rust legacy wire/system codec and signature recovery.
//! The application retains chain admission, ordering and system authorization;
//! this module neither admits network packets nor selects consensus inputs.
//! Wider direct execution/simulation inputs use the domain contracts separately.

use crate::contracts::{
    ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
};
use anyhow::{Result, ensure};
use num_bigint::BigUint;
use rustaxa_types::{FinalChainNonce, FinalChainTransactionPosition, LegacyTransactionEnvelope};

/// Authority supplied by the application for one canonical input. System mode
/// must come from the application's ordered system plan, never from an unsigned
/// transaction's self-description. Ordinary inputs require a recovered signer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyInputKind {
    Signed,
    System,
}

/// Decodes one application-selected legacy/system transaction into wide values.
///
/// Retains exact RLP/hash, sender, destination, position, gas, nonce and payload.
/// Malformed encodings and unrecoverable ordinary signatures return errors from
/// this boundary. Intrinsic gas, affordability, nonce ordering and fee arithmetic
/// remain in the execution envelope: an admitted transaction can consume fees
/// on these failures, so this adapter must not reject it using `cost()` or
/// `intrinsic_gas_covered`. Existing wire widths remain U256; widening the internal
/// representation does not admit wider wire encodings or typed transactions.
pub fn decode_legacy_input(
    position: FinalChainTransactionPosition,
    bytes: &[u8],
    kind: LegacyInputKind,
) -> Result<ExecutionTransaction> {
    let envelope = match kind {
        LegacyInputKind::Signed => LegacyTransactionEnvelope::decode(bytes)?,
        LegacyInputKind::System => LegacyTransactionEnvelope::decode_system(bytes)?,
    };
    ensure!(envelope.signature_valid, "EVM_INPUT_SIGNATURE_INVALID");
    let sender = envelope
        .sender
        .ok_or_else(|| anyhow::anyhow!("EVM_INPUT_SENDER_UNAVAILABLE"))?;
    let nonce_bytes = envelope.nonce.to_big_endian();
    let first = nonce_bytes.iter().position(|b| *b != 0).unwrap_or(32);
    Ok(ExecutionTransaction {
        position,
        hash: envelope.hash.0,
        sender: sender.0,
        receiver: envelope.receiver.map(|address| address.0),
        nonce: FinalChainNonce::from_bytes(&nonce_bytes[first..])?,
        gas_price: ExecutionGasPrice::new(BigUint::from_bytes_be(
            &envelope.gas_price.to_big_endian(),
        )),
        gas_limit: envelope.gas.into(),
        value: ExecutionValue::new(BigUint::from_bytes_be(&envelope.value.to_big_endian())),
        input: envelope.data,
        canonical_rlp: Some(envelope.rlp),
        kind: match kind {
            LegacyInputKind::System => ExecutionTransactionKind::System,
            LegacyInputKind::Signed if envelope.receiver.is_none() => {
                ExecutionTransactionKind::Create
            }
            LegacyInputKind::Signed => ExecutionTransactionKind::Call,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, U256};
    use k256::ecdsa::SigningKey;
    use revm::primitives::keccak256;
    use rustaxa_types::{
        LegacySystemTransactionInput, TARAXA_SYSTEM_ACCOUNT, encode_legacy_system_transaction,
    };

    #[test]
    fn wire_maxima_widen_without_preempting_charged_envelope_failures() {
        let bytes = encode_legacy_system_transaction(&LegacySystemTransactionInput {
            nonce: U256::MAX,
            value: U256::MAX,
            gas_price: U256::MAX,
            gas: 2,
            data: vec![1],
            receiver: Some(H160([0x44; 20])),
            chain_id: 841,
        });
        let tx = decode_legacy_input(3_u32.into(), &bytes, LegacyInputKind::System).unwrap();
        assert_eq!(tx.nonce.to_bytes(), vec![255; 32]);
        assert_eq!(tx.nonce.next().to_bytes().len(), 33);
        assert_eq!(tx.gas_price.value(), &BigUint::from_bytes_be(&[255; 32]));
        assert_eq!(tx.value.value(), tx.gas_price.value());
        assert_eq!(tx.gas_limit.as_u64(), 2);
        assert_eq!(tx.position, 3_u32.into());
        assert_eq!(tx.sender, TARAXA_SYSTEM_ACCOUNT);
        assert_eq!(tx.receiver, Some([0x44; 20]));
        assert_eq!(tx.input, vec![1]);
        assert_eq!(tx.hash, keccak256(&bytes).0);
        assert_eq!(tx.canonical_rlp.as_deref(), Some(bytes.as_slice()));
        // The same unsigned payload is not an ordinary signed transaction.
        assert!(decode_legacy_input(0_u32.into(), &bytes, LegacyInputKind::Signed).is_err());
    }

    #[test]
    fn signed_creation_retains_identity_and_defers_intrinsic_gas() {
        let key = SigningKey::from_slice(&[0x31; 32]).unwrap();
        let mut payload = rlp::RlpStream::new_list(9);
        payload.append(&7_u64).append(&2_u64).append(&21_000_u64);
        payload.append_empty_data();
        payload
            .append(&3_u64)
            .append(&vec![0_u8, 1])
            .append(&841_u64)
            .append(&0_u8)
            .append(&0_u8);
        let payload = payload.out();
        let (signature, recovery) = key
            .sign_prehash_recoverable(&keccak256(&payload).0)
            .unwrap();
        let signature = signature.to_bytes();
        let mut signed = rlp::RlpStream::new_list(9);
        let raw = rlp::Rlp::new(&payload);
        for i in 0..6 {
            signed.append_raw(raw.at(i).unwrap().as_raw(), 1);
        }
        signed.append(&(841 * 2 + 35 + u64::from(recovery.to_byte())));
        signed.append(&U256::from_big_endian(&signature[..32]));
        signed.append(&U256::from_big_endian(&signature[32..]));
        let bytes = signed.out();
        let tx = decode_legacy_input(0_u32.into(), &bytes, LegacyInputKind::Signed).unwrap();
        let public = key.verifying_key().to_encoded_point(false);
        let signer_hash = keccak256(&public.as_bytes()[1..]);
        assert_eq!(tx.sender.as_slice(), &signer_hash[12..]);
        assert_eq!(tx.hash, keccak256(&bytes).0);
        assert_eq!(tx.kind, ExecutionTransactionKind::Create);
        assert_eq!(tx.receiver, None);
        assert_eq!(tx.nonce, 7_u64);
        assert_eq!(tx.gas_limit.as_u64(), 21_000);
        assert_eq!(tx.input, vec![0, 1]);
        assert_eq!(tx.canonical_rlp.as_deref(), Some(bytes.as_ref()));
        assert!(decode_legacy_input(0_u32.into(), &[2, 0xc0], LegacyInputKind::Signed).is_err());
    }
}
