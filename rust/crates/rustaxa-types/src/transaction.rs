//! Legacy transaction envelope decoding shared by Rust rewrite shims.
//!
//! The envelope mirrors the currently supported C++ `Transaction` RLP shape:
//! `[nonce, gas_price, gas, receiver, value, data, v, r, s]`. It preserves the
//! canonical bytes for hashing/storage, decodes deterministic transaction facts,
//! and performs the same EIP-155 sender recovery contract used by legacy C++.
//! Typed/EIP-2718 transactions are intentionally unsupported because the C++
//! transaction class also rejects them in this code path.

use anyhow::{Context, Result, ensure};
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use tiny_keccak::{Hasher, Keccak};

const LEGACY_TRANSACTION_FIELDS: usize = 9;
const TX_GAS: u64 = 21_000;
const TX_GAS_CONTRACT_CREATION: u64 = 53_000;
const TX_DATA_ZERO_GAS: u64 = 4;
const TX_DATA_NON_ZERO_GAS: u64 = 68;

/// Taraxa system account used by C++ `SystemTransaction`.
pub const TARAXA_SYSTEM_ACCOUNT: [u8; 20] = *b"\0TaraxaSystemAccount";

/// Fully inspected legacy transaction envelope.
///
/// `sender` is `None` when a regular signed transaction cannot recover a valid
/// signer. System transactions use [`LegacyTransactionEnvelope::decode_system`]
/// to assign the fixed Taraxa system account without ECDSA recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyTransactionEnvelope {
    /// Canonical RLP bytes passed by the caller.
    pub rlp: Vec<u8>,
    /// Keccak256 hash of `rlp`, matching C++ `Transaction::getHash()`.
    pub hash: H256,
    /// Decoded transaction nonce.
    pub nonce: U256,
    /// Decoded gas price.
    pub gas_price: U256,
    /// Decoded gas limit.
    pub gas: u64,
    /// Optional receiver address. `None` means contract creation.
    pub receiver: Option<H160>,
    /// Decoded value.
    pub value: U256,
    /// Calldata/initcode bytes.
    pub data: Vec<u8>,
    /// Legacy chain id. `0` means the transaction is not replay-protected.
    pub chain_id: u64,
    /// Recovered or system sender.
    pub sender: Option<H160>,
    /// True when the regular transaction signature recovered a sender or when
    /// decoding a system transaction with the fixed system account.
    pub signature_valid: bool,
    /// Intrinsic gas result using the legacy C++ constants.
    pub intrinsic_gas_covered: bool,
}

impl LegacyTransactionEnvelope {
    /// Decodes and inspects a regular signed legacy transaction.
    ///
    /// Malformed RLP, invalid chain-id encodings, invalid receiver shape, and
    /// arithmetic overflow return an error. Invalid signatures do not error;
    /// they produce `sender = None` and `signature_valid = false`, matching the
    /// legacy split between transaction construction and lazy sender recovery.
    pub fn decode(rlp_bytes: &[u8]) -> Result<Self> {
        decode_legacy_transaction(rlp_bytes, SenderMode::Recover)
    }

    /// Decodes a legacy system transaction and assigns the Taraxa system sender.
    ///
    /// System transactions share the same RLP field order as regular legacy
    /// transactions but do not carry a recoverable user signature.
    pub fn decode_system(rlp_bytes: &[u8]) -> Result<Self> {
        decode_legacy_transaction(rlp_bytes, SenderMode::System)
    }

    /// Returns the total transaction cost `gas_price * gas + value`.
    pub fn cost(&self) -> Result<U256> {
        self.gas_price
            .checked_mul(U256::from(self.gas))
            .and_then(|gas_cost| gas_cost.checked_add(self.value))
            .ok_or_else(|| anyhow::anyhow!("legacy transaction cost overflow"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SenderMode {
    Recover,
    System,
}

fn decode_legacy_transaction(
    rlp_bytes: &[u8],
    sender_mode: SenderMode,
) -> Result<LegacyTransactionEnvelope> {
    let rlp = Rlp::new(rlp_bytes);
    ensure!(
        rlp.item_count().context("legacy transaction RLP shape")? == LEGACY_TRANSACTION_FIELDS,
        "legacy transaction RLP must contain 9 fields"
    );

    let nonce = rlp.val_at::<U256>(0).context("legacy transaction nonce")?;
    let gas_price = rlp
        .val_at::<U256>(1)
        .context("legacy transaction gas price")?;
    let gas = rlp.val_at::<u64>(2).context("legacy transaction gas")?;
    let receiver = decode_receiver(&rlp)?;
    let value = rlp.val_at::<U256>(4).context("legacy transaction value")?;
    let data = rlp
        .at(5)
        .context("legacy transaction data")?
        .data()
        .context("legacy transaction data bytes")?
        .to_vec();
    let v = rlp.val_at::<U256>(6).context("legacy transaction v")?;
    let r = decode_signature_scalar(&rlp, 7).context("legacy transaction r")?;
    let s = decode_signature_scalar(&rlp, 8).context("legacy transaction s")?;

    let (chain_id, sender, signature_valid) = match sender_mode {
        SenderMode::System => (
            chain_id_for_system(v, r, s)?,
            Some(H160::from(TARAXA_SYSTEM_ACCOUNT)),
            true,
        ),
        SenderMode::Recover => recover_sender(&rlp, v, r, s)?,
    };

    Ok(LegacyTransactionEnvelope {
        rlp: rlp_bytes.to_vec(),
        hash: keccak256(rlp_bytes),
        nonce,
        gas_price,
        gas,
        receiver,
        value,
        data: data.clone(),
        chain_id,
        sender,
        signature_valid,
        intrinsic_gas_covered: intrinsic_gas_covered(&data, receiver.is_none(), gas),
    })
}

fn decode_receiver(rlp: &Rlp<'_>) -> Result<Option<H160>> {
    let receiver = rlp.at(3).context("legacy transaction receiver")?;
    let bytes = receiver
        .data()
        .context("legacy transaction receiver bytes")?;
    match bytes.len() {
        0 => Ok(None),
        20 => Ok(Some(H160::from_slice(bytes))),
        _ => anyhow::bail!("legacy transaction receiver must be empty or 20 bytes"),
    }
}

fn decode_signature_scalar(rlp: &Rlp<'_>, field: usize) -> Result<U256> {
    let bytes = rlp
        .at(field)
        .context("legacy transaction signature scalar")?
        .data()
        .context("legacy transaction signature scalar bytes")?;
    ensure!(
        bytes.len() <= 32,
        "legacy transaction signature scalar exceeds 32 bytes"
    );
    Ok(U256::from_big_endian(bytes))
}

fn chain_id_for_system(v: U256, r: U256, s: U256) -> Result<u64> {
    if r.is_zero() && s.is_zero() {
        return to_chain_id(v);
    }
    let (chain_id, _) = chain_and_recovery_id(v)?;
    optional_chain_id_to_u64(chain_id)
}

fn recover_sender(rlp: &Rlp<'_>, v: U256, r: U256, s: U256) -> Result<(u64, Option<H160>, bool)> {
    if r.is_zero() && s.is_zero() {
        return Ok((to_chain_id(v)?, None, false));
    }

    let (chain_id, recovery_id) = chain_and_recovery_id(v)?;
    let mut signature = [0u8; 65];
    signature[..32].copy_from_slice(&r.to_big_endian());
    signature[32..64].copy_from_slice(&s.to_big_endian());
    signature[64] = recovery_id;
    let hash = signature_hash(rlp, chain_id)?;
    let sender = recover_address(&signature, &hash);
    Ok((
        optional_chain_id_to_u64(chain_id)?,
        sender,
        sender.is_some(),
    ))
}

fn chain_and_recovery_id(v: U256) -> Result<(Option<U256>, u8)> {
    if v > U256::from(36u64) {
        let chain_id = (v - U256::from(35u64)) / U256::from(2u64);
        let recovery_id = v
            .checked_sub(chain_id * U256::from(2u64) + U256::from(35u64))
            .ok_or_else(|| anyhow::anyhow!("legacy transaction recovery id underflow"))?;
        return Ok((Some(chain_id), u256_to_recovery_id(recovery_id)?));
    }

    ensure!(
        v == U256::from(27u64) || v == U256::from(28u64),
        "legacy transaction recovery id is invalid"
    );
    Ok((None, u256_to_recovery_id(v - U256::from(27u64))?))
}

fn to_chain_id(value: U256) -> Result<u64> {
    ensure!(
        !value.is_zero() && value <= U256::from(u64::MAX),
        "legacy transaction chain id must be in (0, 2^64]"
    );
    Ok(value.low_u64())
}

fn optional_chain_id_to_u64(value: Option<U256>) -> Result<u64> {
    match value {
        Some(value) => to_chain_id(value),
        None => Ok(0),
    }
}

fn u256_to_recovery_id(value: U256) -> Result<u8> {
    ensure!(
        value <= U256::from(3u64),
        "legacy transaction recovery id out of range"
    );
    Ok(value.low_u32() as u8)
}

fn signature_hash(rlp: &Rlp<'_>, chain_id: Option<U256>) -> Result<H256> {
    let mut stream = RlpStream::new_list(if chain_id.is_some() { 9 } else { 6 });
    for field in 0..6 {
        stream.append_raw(
            rlp.at(field)
                .with_context(|| format!("legacy transaction field {field}"))?
                .as_raw(),
            1,
        );
    }
    if let Some(chain_id) = chain_id {
        stream.append(&chain_id);
        stream.append(&U256::zero());
        stream.append(&U256::zero());
    }
    Ok(keccak256(&stream.out()))
}

fn recover_address(signature: &[u8; 65], message_hash: &H256) -> Option<H160> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let recovery_id = RecoveryId::try_from(signature[64]).ok()?;
    let signature = Signature::try_from(&signature[..64]).ok()?;
    let recovered_key =
        VerifyingKey::recover_from_prehash(message_hash.as_bytes(), &signature, recovery_id)
            .ok()?;
    let uncompressed = recovered_key.to_encoded_point(false);
    let pubkey_hash = keccak256(&uncompressed.as_bytes()[1..]);
    Some(H160::from_slice(&pubkey_hash.as_bytes()[12..]))
}

fn intrinsic_gas_covered(data: &[u8], is_contract_creation: bool, gas_limit: u64) -> bool {
    intrinsic_gas(data, is_contract_creation)
        .map(|required| required <= gas_limit)
        .unwrap_or(false)
}

/// Computes the legacy intrinsic gas requirement for transaction data.
///
/// The constants and overflow behavior mirror C++ `IntrinsicGas`: overflow is
/// an error for callers that need the exact value, while
/// `intrinsic_gas_covered` maps overflow to `false`.
pub fn intrinsic_gas(data: &[u8], is_contract_creation: bool) -> Result<u64> {
    let mut gas = if is_contract_creation {
        TX_GAS_CONTRACT_CREATION
    } else {
        TX_GAS
    };
    let non_zero = data.iter().filter(|byte| **byte != 0).count() as u64;
    gas = gas
        .checked_add(
            non_zero
                .checked_mul(TX_DATA_NON_ZERO_GAS)
                .ok_or_else(|| anyhow::anyhow!("legacy transaction non-zero data gas overflow"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("legacy transaction non-zero intrinsic gas overflow"))?;
    let zero = data.len() as u64 - non_zero;
    gas = gas
        .checked_add(
            zero.checked_mul(TX_DATA_ZERO_GAS)
                .ok_or_else(|| anyhow::anyhow!("legacy transaction zero data gas overflow"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("legacy transaction zero intrinsic gas overflow"))?;
    Ok(gas)
}

fn keccak256(data: &[u8]) -> H256 {
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    H256::from(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    fn signed_legacy_transaction_rlp(
        signing_key: &SigningKey,
        nonce: u64,
        gas: u64,
        data: Vec<u8>,
        chain_id: u64,
    ) -> Vec<u8> {
        let mut signature_stream = RlpStream::new_list(9);
        signature_stream.append(&U256::from(nonce));
        signature_stream.append(&U256::from(2u64));
        signature_stream.append(&gas);
        signature_stream.append(&H160::from([0x44u8; 20]));
        signature_stream.append(&U256::from(3u64));
        signature_stream.append(&data);
        signature_stream.append(&U256::from(chain_id));
        signature_stream.append(&U256::zero());
        signature_stream.append(&U256::zero());
        let message_hash = keccak256(&signature_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .expect("test transaction should sign");
        let signature = signature.to_bytes();
        let r = U256::from_big_endian(&signature[..32]);
        let s = U256::from_big_endian(&signature[32..]);
        let v = U256::from(chain_id * 2 + 35 + u64::from(recovery_id.to_byte()));

        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(nonce));
        stream.append(&U256::from(2u64));
        stream.append(&gas);
        stream.append(&H160::from([0x44u8; 20]));
        stream.append(&U256::from(3u64));
        stream.append(&data);
        stream.append(&v);
        stream.append(&r);
        stream.append(&s);
        stream.out().to_vec()
    }

    fn system_transaction_rlp(nonce: u64) -> Vec<u8> {
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(nonce));
        stream.append(&U256::zero());
        stream.append(&0u64);
        stream.append(&H160::zero());
        stream.append(&U256::zero());
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(1u64));
        stream.append(&U256::zero());
        stream.append(&U256::zero());
        stream.out().to_vec()
    }

    #[test]
    fn legacy_transaction_envelope_recovers_sender_and_fields() {
        let signing_key = SigningKey::from_slice(&[0x31u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let rlp = signed_legacy_transaction_rlp(&signing_key, 7, 21_100, vec![0, 1], 2999);

        let envelope = LegacyTransactionEnvelope::decode(&rlp).unwrap();

        assert_eq!(envelope.hash, keccak256(&rlp));
        assert_eq!(envelope.sender, Some(sender));
        assert!(envelope.signature_valid);
        assert_eq!(envelope.nonce, U256::from(7u64));
        assert_eq!(envelope.gas_price, U256::from(2u64));
        assert_eq!(envelope.gas, 21_100);
        assert_eq!(envelope.receiver, Some(H160::from([0x44u8; 20])));
        assert_eq!(envelope.value, U256::from(3u64));
        assert_eq!(envelope.data, vec![0, 1]);
        assert_eq!(envelope.chain_id, 2999);
        assert_eq!(envelope.cost().unwrap(), U256::from(42_203u64));
        assert!(envelope.intrinsic_gas_covered);
    }

    #[test]
    fn legacy_transaction_envelope_reports_intrinsic_gas_failure() {
        let signing_key = SigningKey::from_slice(&[0x32u8; 32]).unwrap();
        let rlp = signed_legacy_transaction_rlp(&signing_key, 1, 21_000, vec![1], 2999);
        let envelope = LegacyTransactionEnvelope::decode(&rlp).unwrap();

        assert!(!envelope.intrinsic_gas_covered);
        assert_eq!(intrinsic_gas(&[1], false).unwrap(), 21_068);
    }

    #[test]
    fn legacy_transaction_envelope_handles_unsigned_chain_id_shape() {
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(1u64));
        stream.append(&U256::zero());
        stream.append(&21_000u64);
        stream.append(&H160::from([0x44u8; 20]));
        stream.append(&U256::zero());
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(2999u64));
        stream.append(&U256::zero());
        stream.append(&U256::zero());
        let envelope = LegacyTransactionEnvelope::decode(&stream.out()).unwrap();

        assert_eq!(envelope.chain_id, 2999);
        assert_eq!(envelope.sender, None);
        assert!(!envelope.signature_valid);
    }

    #[test]
    fn legacy_transaction_envelope_accepts_zero_padded_signature_scalars() {
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(1u64));
        stream.append(&U256::zero());
        stream.append(&21_000u64);
        stream.append(&H160::from([0x44u8; 20]));
        stream.append(&U256::zero());
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(2999u64));
        stream.append(&vec![0u8; 32]);
        stream.append(&vec![0u8; 32]);
        let envelope = LegacyTransactionEnvelope::decode(&stream.out()).unwrap();

        assert_eq!(envelope.chain_id, 2999);
        assert_eq!(envelope.sender, None);
        assert!(!envelope.signature_valid);
    }

    #[test]
    fn legacy_transaction_envelope_decodes_system_sender() {
        let envelope =
            LegacyTransactionEnvelope::decode_system(&system_transaction_rlp(3)).unwrap();

        assert_eq!(envelope.sender, Some(H160::from(TARAXA_SYSTEM_ACCOUNT)));
        assert_eq!(envelope.nonce, U256::from(3u64));
        assert!(envelope.signature_valid);
    }

    #[test]
    fn legacy_transaction_envelope_rejects_bad_receiver_shape() {
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::zero());
        stream.append(&U256::zero());
        stream.append(&21_000u64);
        stream.append(&vec![1u8, 2u8]);
        stream.append(&U256::zero());
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(27u64));
        stream.append(&U256::one());
        stream.append(&U256::one());

        assert!(
            LegacyTransactionEnvelope::decode(&stream.out())
                .unwrap_err()
                .to_string()
                .contains("receiver")
        );
    }
}
