//! Physical account, descriptor, and key codecs used by concrete-state reads.

use num_bigint::BigUint;
use rlp::{Rlp, RlpStream};
use rustaxa_types::concrete_state::{
    ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteReadError,
    ConcreteStateIdentity, ConcreteStorageKey,
};
use rustaxa_types::{FinalChainBlockNumber, FinalChainNonce};
use tiny_keccak::{Hasher, Keccak};

/// Decodes the reference's two-field committed state descriptor.
pub(crate) fn decode_descriptor(bytes: &[u8]) -> Result<ConcreteStateIdentity, ConcreteReadError> {
    let rlp = exact_rlp(bytes, "descriptor")?;
    if !rlp.is_list() || rlp.item_count().map_err(corrupt)? != 2 {
        return Err(corrupt("descriptor must be a two-field RLP list"));
    }
    let period = rlp.val_at::<u64>(0).map_err(corrupt)?;
    let root = exact_hash(
        rlp.at(1).map_err(corrupt)?.data().map_err(corrupt)?,
        "descriptor root",
    )?;
    Ok(ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(period),
        state_root: root,
    })
}

/// Decodes an exact five-field physical account RLP without narrowing nonce or
/// balance. The reference writer emits canonical nonce bytes, which the shared
/// nonce domain requires. Balance bytes and short legacy optional hashes are
/// accepted as the Go decoder accepts them; their physical bytes remain intact.
pub fn decode_physical_account(bytes: &[u8]) -> Result<ConcreteAccountRecord, ConcreteReadError> {
    let rlp = exact_rlp(bytes, "account")?;
    if !rlp.is_list() || rlp.item_count().map_err(corrupt)? != 5 {
        return Err(corrupt("account must be a five-field RLP list"));
    }
    let nonce_bytes = field_data(&rlp, 0, "nonce")?;
    let balance_bytes = field_data(&rlp, 1, "balance")?;
    let storage_root = optional_hash(field_data(&rlp, 2, "storage root")?, "storage root")?;
    let code_hash = optional_hash(field_data(&rlp, 3, "code hash")?, "code hash")?;
    let code_size = rlp.val_at::<u64>(4).map_err(corrupt)?;
    let nonce = FinalChainNonce::from_bytes(nonce_bytes).map_err(corrupt)?;
    Ok(ConcreteAccountRecord {
        account: ConcreteAccount {
            nonce,
            balance: ConcreteAccountBalance::new(BigUint::from_bytes_be(balance_bytes)),
            storage_root,
            code_hash,
            code_size,
        },
        physical_rlp: bytes.to_vec(),
    })
}

/// Reconstructs the four-field account bytes committed by the reference trie.
/// The record's decoded fields must still match its preserved physical RLP.
pub fn account_commitment_rlp(
    record: &ConcreteAccountRecord,
) -> Result<Vec<u8>, ConcreteReadError> {
    if decode_physical_account(&record.physical_rlp)? != *record {
        return Err(corrupt(
            "decoded account fields do not match preserved physical RLP",
        ));
    }
    let physical = Rlp::new(&record.physical_rlp);
    let nonce = field_data(&physical, 0, "nonce")?;
    let balance = field_data(&physical, 1, "balance")?;
    let storage_root = field_data(&physical, 2, "storage root")?;
    let code_hash = field_data(&physical, 3, "code hash")?;
    let mut stream = RlpStream::new_list(4);
    stream.append(&nonce);
    stream.append(&balance);
    if storage_root.is_empty() {
        stream.append(&empty_trie_root().as_slice());
    } else {
        stream.append(&storage_root);
    }
    if code_hash.is_empty() {
        stream.append(&empty_code_hash().as_slice());
    } else {
        stream.append(&code_hash);
    }
    Ok(stream.out().to_vec())
}

/// Returns the 32-byte version-column prefix for an unhashed account address.
pub fn account_version_prefix(address: [u8; 20]) -> [u8; 32] {
    keccak256(&address)
}

/// Returns the 32-byte version-column prefix for a logical slot. The reference
/// hashes the logical key for the account trie path, then hashes address+path.
pub fn storage_version_prefix(address: [u8; 20], key: ConcreteStorageKey) -> [u8; 32] {
    let path = storage_trie_path(key);
    let mut input = [0_u8; 52];
    input[..20].copy_from_slice(&address);
    input[20..].copy_from_slice(&path);
    keccak256(&input)
}

/// Forms the persisted version key as prefix plus big-endian period.
pub fn versioned_key(prefix: [u8; 32], period: FinalChainBlockNumber) -> [u8; 40] {
    let mut key = [0_u8; 40];
    key[..32].copy_from_slice(&prefix);
    key[32..].copy_from_slice(&period.as_u64().to_be_bytes());
    key
}

pub(crate) fn storage_trie_path(key: ConcreteStorageKey) -> [u8; 32] {
    keccak256(&key.0)
}

pub(crate) fn storage_prefix_for_path(address: [u8; 20], path: [u8; 32]) -> [u8; 32] {
    let mut input = [0_u8; 52];
    input[..20].copy_from_slice(&address);
    input[20..].copy_from_slice(&path);
    keccak256(&input)
}

pub(crate) fn empty_trie_root() -> [u8; 32] {
    // Keccak-256(RLP empty string) is the empty Merkle-Patricia trie root.
    keccak256(&[0x80])
}

pub(crate) fn empty_code_hash() -> [u8; 32] {
    keccak256(&[])
}

pub(crate) fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut output = [0_u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut output);
    output
}

fn exact_rlp<'a>(bytes: &'a [u8], label: &str) -> Result<Rlp<'a>, ConcreteReadError> {
    let rlp = Rlp::new(bytes);
    let total = rlp.payload_info().map_err(corrupt)?.total();
    if total != bytes.len() {
        return Err(corrupt(format!("{label} has trailing bytes")));
    }
    Ok(rlp)
}

fn field_data<'a>(rlp: &Rlp<'a>, index: usize, label: &str) -> Result<&'a [u8], ConcreteReadError> {
    rlp.at(index)
        .map_err(corrupt)?
        .data()
        .map_err(|error| corrupt(format!("invalid account {label}: {error}")))
}

fn optional_hash(bytes: &[u8], label: &str) -> Result<Option<[u8; 32]>, ConcreteReadError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() > 32 {
        return Err(corrupt(format!("{label} exceeds 32 bytes")));
    }
    let mut hash = [0_u8; 32];
    hash[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(Some(hash))
}

fn exact_hash(bytes: &[u8], label: &str) -> Result<[u8; 32], ConcreteReadError> {
    bytes
        .try_into()
        .map_err(|_| corrupt(format!("{label} must be 32 bytes")))
}

pub(crate) fn corrupt(error: impl std::fmt::Display) -> ConcreteReadError {
    ConcreteReadError::Corrupt(error.to_string())
}
