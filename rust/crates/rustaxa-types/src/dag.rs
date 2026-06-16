use ethereum_types::{H160, H256};
use rlp::RlpStream;
use tiny_keccak::{Hasher, Keccak};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagBlock {
    pub pivot: H256,
    pub level: u64,
    pub timestamp: u64,
    pub vdf: Vec<u8>,
    pub tips: Vec<H256>,
    pub transactions: Vec<H256>,
    pub signature: [u8; 65],
    pub gas_estimation: u64,
}

impl DagBlock {
    /// Returns the legacy signing hash for this DAG block.
    ///
    /// The hash is Keccak256 over the canonical DAG block RLP without the signature field and with transaction hashes
    /// included, matching C++ `DagBlock::sha3(false)`. This is the message signed by DAG block proposers and the message
    /// used to recover the proposer address from `signature`.
    pub fn signing_hash(&self) -> H256 {
        keccak256(&self.rlp_without_signature())
    }

    /// Recovers the DAG block proposer address from the stored recoverable ECDSA signature.
    ///
    /// Returns `None` when the signature is not a valid 65-byte recoverable signature for the block signing hash. The
    /// address derivation matches legacy C++ `DagBlock::getSender`: recover the uncompressed public key, hash the
    /// 64-byte X/Y payload with Keccak256, and use the rightmost 20 bytes.
    pub fn recover_sender(&self) -> Option<H160> {
        recover_address(&self.signature, &self.signing_hash())
    }

    fn rlp_without_signature(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(7);
        stream.append(&self.pivot);
        stream.append(&self.level);
        stream.append(&self.timestamp);
        stream.append(&self.vdf);
        stream.append_list(&self.tips);
        stream.append_list(&self.transactions);
        stream.append(&self.gas_estimation);
        stream.out().to_vec()
    }
}

fn keccak256(data: &[u8]) -> H256 {
    let mut out = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut out);
    H256(out)
}

fn recover_address(signature: &[u8; 65], message_hash: &H256) -> Option<H160> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let recovery_id = RecoveryId::try_from(signature[64]).ok()?;
    let signature = Signature::try_from(&signature[..64]).ok()?;
    let recovered_key =
        VerifyingKey::recover_from_prehash(message_hash.as_bytes(), &signature, recovery_id)
            .ok()?;
    let uncompressed = recovered_key.to_encoded_point(false);
    let public_key_hash = keccak256(&uncompressed.as_bytes()[1..]);
    Some(H160::from_slice(&public_key_hash.as_bytes()[12..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public = signing_key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&public.as_bytes()[1..]);
        H160::from_slice(&hash.as_bytes()[12..])
    }

    fn signed_block(seed: u8) -> DagBlock {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let mut block = DagBlock {
            pivot: H256::from_low_u64_be(1),
            level: 7,
            timestamp: 99,
            vdf: vec![1, 2, 3],
            tips: vec![H256::from_low_u64_be(2)],
            transactions: vec![H256::from_low_u64_be(3)],
            signature: [0; 65],
            gas_estimation: 123,
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .expect("sign block");
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();
        block
    }

    #[test]
    fn dag_block_recovers_sender_from_legacy_signing_hash() {
        let signing_key = SigningKey::from_slice(&[0x44; 32]).expect("signing key");
        let block = signed_block(0x44);

        assert_eq!(
            block.recover_sender(),
            Some(address_from_signing_key(&signing_key))
        );
    }

    #[test]
    fn dag_block_sender_recovery_rejects_invalid_recovery_id() {
        let mut block = signed_block(0x45);
        block.signature[64] = 5;

        assert_eq!(block.recover_sender(), None);
    }
}
