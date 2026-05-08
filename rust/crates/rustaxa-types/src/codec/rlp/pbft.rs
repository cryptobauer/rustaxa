use crate::pbft::{PbftBlockLink, PbftBlockMetadata};
use anyhow::{Result, anyhow};
use ethereum_types::{H160, H256};
use rlp::{Rlp, RlpStream};
use tiny_keccak::{Hasher, Keccak};

const PBFT_PERIOD_POS: usize = 4;
const PBFT_TIMESTAMP_POS: usize = 5;
const PBFT_EXTRA_DATA_POS: usize = 7;
const PBFT_PREV_HASH_POS: usize = 0;
const PBFT_PIVOT_DAG_HASH_POS: usize = 1;

#[derive(Debug, Clone, Copy)]
pub struct SignedPbftBlockRlp<'a>(&'a [u8]);

impl<'a> SignedPbftBlockRlp<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
}

impl TryFrom<SignedPbftBlockRlp<'_>> for PbftBlockLink {
    type Error = anyhow::Error;

    fn try_from(value: SignedPbftBlockRlp<'_>) -> Result<Self, Self::Error> {
        decode_signed_block_link_rlp(&Rlp::new(value.0))
    }
}

impl TryFrom<SignedPbftBlockRlp<'_>> for PbftBlockMetadata {
    type Error = anyhow::Error;

    fn try_from(value: SignedPbftBlockRlp<'_>) -> Result<Self, Self::Error> {
        decode_signed_block_metadata_rlp(&Rlp::new(value.0))
    }
}

fn decode_signed_block_link_rlp(rlp: &Rlp<'_>) -> Result<PbftBlockLink> {
    let item_count = rlp.item_count()?;
    if item_count < 8 {
        return Err(anyhow!(
            "invalid signed PBFT block RLP: expected at least 8 fields, got {item_count}"
        ));
    }

    Ok(PbftBlockLink {
        block_hash: keccak256(rlp.as_raw()),
        prev_block_hash: rlp.val_at(PBFT_PREV_HASH_POS)?,
        pivot_dag_block_hash: rlp.val_at(PBFT_PIVOT_DAG_HASH_POS)?,
        period: rlp.val_at(PBFT_PERIOD_POS)?,
    })
}

fn decode_signed_block_metadata_rlp(rlp: &Rlp<'_>) -> Result<PbftBlockMetadata> {
    let item_count = rlp.item_count()?;
    let author = recover_signed_block_author(rlp.as_raw())
        .ok_or_else(|| anyhow!("could not recover PBFT block proposer"))?;
    let extra_data = if item_count == 9 {
        rlp.at(PBFT_EXTRA_DATA_POS)?.data()?.to_vec()
    } else {
        Vec::new()
    };

    Ok(PbftBlockMetadata {
        author,
        period: rlp.val_at(PBFT_PERIOD_POS)?,
        timestamp: rlp.val_at(PBFT_TIMESTAMP_POS)?,
        extra_data,
    })
}

fn recover_signed_block_author(block_rlp: &[u8]) -> Option<H160> {
    let rlp = Rlp::new(block_rlp);
    let item_count = rlp.item_count().ok()?;
    if item_count < 8 {
        return None;
    }

    let signature: Vec<u8> = rlp.val_at(item_count - 1).ok()?;
    let mut unsigned_stream = RlpStream::new_list(item_count - 1);
    for i in 0..item_count - 1 {
        unsigned_stream.append_raw(rlp.at(i).ok()?.as_raw(), 1);
    }
    let message_hash = keccak256(&unsigned_stream.out());

    recover_address(&signature, &message_hash)
}

fn recover_address(signature: &[u8], message_hash: &H256) -> Option<H160> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    if signature.len() != 65 {
        return None;
    }

    let recovery_id = RecoveryId::try_from(signature[64] % 4).ok()?;
    let signature = Signature::try_from(&signature[..64]).ok()?;
    let recovered_key =
        VerifyingKey::recover_from_prehash(message_hash.as_bytes(), &signature, recovery_id)
            .ok()?;
    let uncompressed = recovered_key.to_encoded_point(false);
    let pubkey_hash = keccak256(&uncompressed.as_bytes()[1..]);

    Some(H160::from_slice(&pubkey_hash.as_bytes()[12..]))
}

fn keccak256(data: &[u8]) -> H256 {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    H256::from(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    fn append_pbft_fields_without_extra(stream: &mut RlpStream, period: u64, timestamp: u64) {
        stream.append(&ethereum_types::H256::from_low_u64_be(10));
        stream.append(&ethereum_types::H256::from_low_u64_be(11));
        stream.append(&ethereum_types::H256::from_low_u64_be(12));
        stream.append(&ethereum_types::H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&timestamp);
        stream.begin_list(0);
    }

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    fn signed_pbft_block_without_extra(
        signing_key: &SigningKey,
        period: u64,
        timestamp: u64,
    ) -> Vec<u8> {
        let mut unsigned_stream = RlpStream::new_list(7);
        append_pbft_fields_without_extra(&mut unsigned_stream, period, timestamp);
        let message_hash = keccak256(&unsigned_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed_stream = RlpStream::new_list(8);
        append_pbft_fields_without_extra(&mut signed_stream, period, timestamp);
        signed_stream.append(&signature_bytes);
        signed_stream.out().to_vec()
    }

    fn invalid_signed_pbft_block() -> Vec<u8> {
        let mut stream = RlpStream::new_list(8);
        append_pbft_fields_without_extra(&mut stream, 7, 11);
        stream.append(&vec![0u8; 64]);
        stream.out().to_vec()
    }

    #[test]
    fn decodes_signed_pbft_block_without_extra_data() {
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let block = signed_pbft_block_without_extra(&signing_key, 7, 11);

        let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&block)).unwrap();

        assert_eq!(metadata.author, address_from_signing_key(&signing_key));
        assert_eq!(metadata.period, 7);
        assert_eq!(metadata.timestamp, 11);
        assert!(metadata.extra_data.is_empty());
    }

    #[test]
    fn decodes_signed_pbft_block_link_fields() {
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let block = signed_pbft_block_without_extra(&signing_key, 7, 11);

        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block)).unwrap();

        assert_eq!(link.block_hash, keccak256(&block));
        assert_eq!(
            link.prev_block_hash,
            ethereum_types::H256::from_low_u64_be(10)
        );
        assert_eq!(
            link.pivot_dag_block_hash,
            ethereum_types::H256::from_low_u64_be(11)
        );
        assert_eq!(link.period, 7);
    }

    #[test]
    fn rejects_signed_pbft_block_when_proposer_cannot_be_recovered() {
        let err =
            PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&invalid_signed_pbft_block()))
                .unwrap_err();

        assert!(
            err.to_string()
                .contains("could not recover PBFT block proposer")
        );
    }
}
