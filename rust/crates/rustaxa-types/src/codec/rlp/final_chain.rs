use crate::codec::rlp::pbft::SignedPbftBlockRlp;
use crate::final_chain::{
    BlockHeaderContext, FinalChainBlockHeader, FinalChainBlockNumber, FinalChainLogBloom,
    StoredFinalChainBlockHeader,
};
use crate::pbft::PbftBlockMetadata;
use anyhow::Result;
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use tiny_keccak::{Hasher, Keccak};

const STORED_HEADER_PARENT_HASH_POS: usize = 0;
const STORED_HEADER_STATE_ROOT_POS: usize = 1;
const STORED_HEADER_TRANSACTIONS_ROOT_POS: usize = 2;
const STORED_HEADER_RECEIPTS_ROOT_POS: usize = 3;
const STORED_HEADER_LOG_BLOOM_POS: usize = 4;
const STORED_HEADER_GAS_USED_POS: usize = 5;
const STORED_HEADER_TOTAL_REWARD_POS: usize = 6;
const EMPTY_UNCLES_HASH: [u8; 32] = [
    0x1d, 0xcc, 0x4d, 0xe8, 0xde, 0xc7, 0x5d, 0x7a, 0xab, 0x85, 0xb5, 0x67, 0xb6, 0xcc, 0xd4, 0x1a,
    0xd3, 0x12, 0x45, 0x1b, 0x94, 0x8a, 0x74, 0x13, 0xf0, 0xa1, 0x42, 0xfd, 0x40, 0xd4, 0x93, 0x47,
];

#[derive(Debug, Clone, Copy)]
pub struct StoredBlockHeaderRlp<'a>(&'a [u8]);

impl<'a> StoredBlockHeaderRlp<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyBlockHeaderRlp(Vec<u8>);

impl LegacyBlockHeaderRlp {
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the final-chain block hash embedded as field zero in legacy header RLP.
    pub fn hash(&self) -> Result<H256> {
        Ok(Rlp::new(&self.0).val_at(0)?)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LegacyBlockHeaderRlpInput<'a> {
    stored_header: StoredBlockHeaderRlp<'a>,
    signed_pbft_block: Option<SignedPbftBlockRlp<'a>>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    block_number: FinalChainBlockNumber,
}

impl<'a> LegacyBlockHeaderRlpInput<'a> {
    pub fn new(
        stored_header: StoredBlockHeaderRlp<'a>,
        block_gas_limit: u64,
        genesis_timestamp: u64,
    ) -> Self {
        Self {
            stored_header,
            signed_pbft_block: None,
            block_gas_limit,
            genesis_timestamp,
            block_number: FinalChainBlockNumber::GENESIS,
        }
    }

    pub fn signed_pbft_block(mut self, signed_pbft_block: SignedPbftBlockRlp<'a>) -> Self {
        self.signed_pbft_block = Some(signed_pbft_block);
        self
    }

    pub fn block_number(mut self, block_number: FinalChainBlockNumber) -> Self {
        self.block_number = block_number;
        self
    }
}

impl TryFrom<StoredBlockHeaderRlp<'_>> for StoredFinalChainBlockHeader {
    type Error = anyhow::Error;

    fn try_from(value: StoredBlockHeaderRlp<'_>) -> Result<Self, Self::Error> {
        decode_stored_block_header_rlp(&Rlp::new(value.0))
    }
}

impl From<&StoredFinalChainBlockHeader> for StoredBlockHeaderRlpOwned {
    fn from(header: &StoredFinalChainBlockHeader) -> Self {
        StoredBlockHeaderRlpOwned(encode_stored_block_header_rlp(header))
    }
}

/// Owned encoded form of the seven-field final-chain header stored in RocksDB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlockHeaderRlpOwned(Vec<u8>);

impl StoredBlockHeaderRlpOwned {
    /// Consumes the wrapper and returns the encoded bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    /// Borrows the encoded bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

fn decode_stored_block_header_rlp(rlp: &Rlp<'_>) -> Result<StoredFinalChainBlockHeader> {
    Ok(StoredFinalChainBlockHeader {
        parent_hash: rlp.val_at(STORED_HEADER_PARENT_HASH_POS)?,
        state_root: rlp.val_at(STORED_HEADER_STATE_ROOT_POS)?,
        transactions_root: rlp.val_at(STORED_HEADER_TRANSACTIONS_ROOT_POS)?,
        receipts_root: rlp.val_at(STORED_HEADER_RECEIPTS_ROOT_POS)?,
        log_bloom: FinalChainLogBloom::try_from(rlp.at(STORED_HEADER_LOG_BLOOM_POS)?.data()?)
            .map_err(|error| {
                anyhow::anyhow!("FINAL_CHAIN_STORED_HEADER_LOG_BLOOM_INVALID_LENGTH: {error}")
            })?,
        gas_used: rlp.val_at::<u64>(STORED_HEADER_GAS_USED_POS)?.into(),
        total_reward: rlp.val_at(STORED_HEADER_TOTAL_REWARD_POS)?,
    })
}

fn encode_stored_block_header_rlp(header: &StoredFinalChainBlockHeader) -> Vec<u8> {
    let mut stream = RlpStream::new_list(7);
    stream.append(&header.parent_hash);
    stream.append(&header.state_root);
    stream.append(&header.transactions_root);
    stream.append(&header.receipts_root);
    stream.append(&header.log_bloom.as_ref());
    stream.append(&header.gas_used.as_u64());
    stream.append(&header.total_reward);
    stream.out().to_vec()
}

impl From<&FinalChainBlockHeader> for LegacyBlockHeaderRlp {
    fn from(header: &FinalChainBlockHeader) -> Self {
        LegacyBlockHeaderRlp(encode_legacy_block_header(header))
    }
}

fn encode_legacy_block_header(header: &FinalChainBlockHeader) -> Vec<u8> {
    let mut stream = RlpStream::new_list(13);
    stream.append(&header.hash);
    stream.append(&header.parent_hash);
    stream.append(&header.author);
    stream.append(&header.state_root);
    stream.append(&header.transactions_root);
    stream.append(&header.receipts_root);
    stream.append(&header.log_bloom.as_ref());
    // RLP is a compatibility/storage boundary; keep the domain number typed internally.
    stream.append(&header.number.as_u64());
    stream.append(&header.gas_limit.as_u64());
    stream.append(&header.gas_used.as_u64());
    stream.append(&header.timestamp);
    stream.append(&header.total_reward);
    stream.append(&header.extra_data.as_slice());
    stream.out().to_vec()
}

impl TryFrom<LegacyBlockHeaderRlpInput<'_>> for LegacyBlockHeaderRlp {
    type Error = anyhow::Error;

    fn try_from(value: LegacyBlockHeaderRlpInput<'_>) -> Result<Self, Self::Error> {
        let stored_header = StoredFinalChainBlockHeader::try_from(value.stored_header)?;
        let pbft = match value.signed_pbft_block {
            Some(block) => Some(PbftBlockMetadata::try_from(block)?),
            None => None,
        };
        if let Some(pbft) = pbft.as_ref()
            && FinalChainBlockNumber::new(pbft.period) != value.block_number
        {
            anyhow::bail!("FINAL_CHAIN_BLOCK_NUMBER_METADATA_MISMATCH");
        }
        let hash = ethereum_block_header_hash(
            &stored_header,
            pbft.as_ref(),
            value.block_number,
            value.block_gas_limit,
            value.genesis_timestamp,
        );

        Ok(LegacyBlockHeaderRlp::from(&stored_header.materialize(
            BlockHeaderContext {
                hash,
                pbft: pbft.as_ref(),
                block_number: value.block_number,
                block_gas_limit: value.block_gas_limit.into(),
                genesis_timestamp: value.genesis_timestamp,
            },
        )))
    }
}

fn ethereum_block_header_hash(
    stored_header: &StoredFinalChainBlockHeader,
    pbft: Option<&PbftBlockMetadata>,
    block_number: FinalChainBlockNumber,
    gas_limit: u64,
    genesis_timestamp: u64,
) -> H256 {
    let author = pbft.map(|pbft| pbft.author).unwrap_or_default();
    let timestamp = pbft.map(|pbft| pbft.timestamp).unwrap_or(genesis_timestamp);
    let extra_data = pbft
        .map(|pbft| pbft.extra_data.as_slice())
        .unwrap_or_default();
    keccak256(&encode_ethereum_header_hash_input(
        stored_header,
        author,
        block_number.as_u64(),
        gas_limit,
        timestamp,
        extra_data,
    ))
}

fn encode_ethereum_header_hash_input(
    stored_header: &StoredFinalChainBlockHeader,
    author: H160,
    number: u64,
    gas_limit: u64,
    timestamp: u64,
    extra_data: &[u8],
) -> Vec<u8> {
    let empty_uncles_hash = H256::from(EMPTY_UNCLES_HASH);
    let zero_hash = H256::zero();
    let zero_nonce = [0u8; 8];
    let mut stream = RlpStream::new_list(15);
    stream.append(&stored_header.parent_hash);
    stream.append(&empty_uncles_hash);
    stream.append(&author);
    stream.append(&stored_header.state_root);
    stream.append(&stored_header.transactions_root);
    stream.append(&stored_header.receipts_root);
    stream.append(&stored_header.log_bloom.as_ref());
    stream.append(&U256::zero());
    stream.append(&number);
    stream.append(&gas_limit);
    stream.append(&stored_header.gas_used.as_u64());
    stream.append(&timestamp);
    stream.append(&extra_data);
    stream.append(&zero_hash);
    stream.append(&zero_nonce.as_slice());
    stream.out().to_vec()
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
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;

    fn header_data_rlp(gas_used: u64, total_reward: U256) -> Vec<u8> {
        let mut header_stream = RlpStream::new_list(7);
        header_stream.append(&H256::from_low_u64_be(1));
        header_stream.append(&H256::from_low_u64_be(2));
        header_stream.append(&H256::from_low_u64_be(3));
        header_stream.append(&H256::from_low_u64_be(4));
        header_stream.append(&[0u8; 256].as_slice());
        header_stream.append(&gas_used);
        header_stream.append(&total_reward);
        header_stream.out().to_vec()
    }

    fn append_pbft_block_fields(
        stream: &mut RlpStream,
        period: u64,
        timestamp: u64,
        extra_data: &[u8],
    ) {
        stream.append(&H256::from_low_u64_be(10));
        stream.append(&H256::from_low_u64_be(11));
        stream.append(&H256::from_low_u64_be(12));
        stream.append(&H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&timestamp);
        stream.begin_list(0);
        stream.append(&extra_data);
    }

    fn signed_pbft_block_rlp(
        signing_key: &SigningKey,
        period: u64,
        timestamp: u64,
        extra_data: &[u8],
    ) -> Vec<u8> {
        let mut unsigned_stream = RlpStream::new_list(8);
        append_pbft_block_fields(&mut unsigned_stream, period, timestamp, extra_data);
        let msg = keccak256(&unsigned_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(msg.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed_stream = RlpStream::new_list(9);
        append_pbft_block_fields(&mut signed_stream, period, timestamp, extra_data);
        signed_stream.append(&signature_bytes);
        signed_stream.out().to_vec()
    }

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    #[test]
    fn reconstructs_genesis_block_header_rlp() {
        let block_gas_limit = 1000u64;
        let genesis_timestamp = 1234u64;
        let header = header_data_rlp(5, U256::from(6u64));

        let full_header = LegacyBlockHeaderRlp::try_from(LegacyBlockHeaderRlpInput::new(
            StoredBlockHeaderRlp::new(&header),
            block_gas_limit,
            genesis_timestamp,
        ))
        .unwrap();
        let full_header_rlp = Rlp::new(full_header.as_bytes());

        assert_eq!(full_header_rlp.item_count().unwrap(), 13);
        assert_eq!(
            full_header_rlp.val_at::<H256>(1).unwrap(),
            H256::from_low_u64_be(1)
        );
        assert_eq!(full_header_rlp.val_at::<u64>(7).unwrap(), 0);
        assert_eq!(full_header_rlp.val_at::<u64>(8).unwrap(), block_gas_limit);
        assert_eq!(
            full_header_rlp.val_at::<u64>(10).unwrap(),
            genesis_timestamp
        );
        assert_eq!(full_header.clone().into_vec(), full_header.as_bytes());
    }

    #[test]
    fn typed_bloom_preserves_stored_header_rlp_and_legacy_hash() {
        let raw = header_data_rlp(5, U256::from(6u64));
        let stored =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw)).unwrap();
        assert_eq!(StoredBlockHeaderRlpOwned::from(&stored).into_vec(), raw);
        let full = LegacyBlockHeaderRlp::try_from(LegacyBlockHeaderRlpInput::new(
            StoredBlockHeaderRlp::new(&raw),
            1000,
            1234,
        ))
        .unwrap();
        assert_eq!(
            full.hash().unwrap(),
            H256::from([
                0xd4, 0xac, 0x1e, 0x1f, 0xcb, 0x08, 0xb1, 0x23, 0x77, 0xb7, 0xd5, 0x64, 0x4b, 0xc8,
                0xc6, 0x73, 0x35, 0x10, 0x08, 0xe6, 0x32, 0xb7, 0xd7, 0x1b, 0x06, 0x0c, 0x6e, 0x2c,
                0x6e, 0xc4, 0x2d, 0xfd,
            ])
        );
    }

    #[test]
    fn reconstructs_non_genesis_pbft_fields() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let expected_author = address_from_signing_key(&signing_key);
        let period = 42u64;
        let timestamp = 12345u64;
        let block_gas_limit = 99_000_000u64;
        let gas_used = 777u64;
        let total_reward = U256::from(888u64);
        let extra_data = vec![0xC2, 0x01, 0x02];
        let raw_header = header_data_rlp(gas_used, total_reward);
        let pbft_block = signed_pbft_block_rlp(&signing_key, period, timestamp, &extra_data);

        let full_header = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(
                StoredBlockHeaderRlp::new(&raw_header),
                block_gas_limit,
                0,
            )
            .block_number(period.into())
            .signed_pbft_block(SignedPbftBlockRlp::new(&pbft_block)),
        )
        .unwrap();
        let full_header_rlp = Rlp::new(full_header.as_bytes());

        assert_eq!(full_header_rlp.item_count().unwrap(), 13);
        assert_eq!(
            full_header_rlp.val_at::<H256>(1).unwrap(),
            H256::from_low_u64_be(1)
        );
        assert_eq!(full_header_rlp.val_at::<H160>(2).unwrap(), expected_author);
        assert_eq!(full_header_rlp.val_at::<u64>(7).unwrap(), period);
        assert_eq!(full_header_rlp.val_at::<u64>(8).unwrap(), block_gas_limit);
        assert_eq!(full_header_rlp.val_at::<u64>(9).unwrap(), gas_used);
        assert_eq!(full_header_rlp.val_at::<u64>(10).unwrap(), timestamp);
        assert_eq!(full_header_rlp.val_at::<U256>(11).unwrap(), total_reward);
        assert_eq!(
            full_header_rlp.at(12).unwrap().data().unwrap(),
            extra_data.as_slice()
        );
    }

    #[test]
    fn rejects_header_number_when_admitted_identity_mismatches_pbft_metadata() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let pbft_block = signed_pbft_block_rlp(&signing_key, 42, 12345, &[]);
        let raw_header = header_data_rlp(0, U256::zero());
        let error = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(StoredBlockHeaderRlp::new(&raw_header), 1_000, 0)
                .block_number(43u64.into())
                .signed_pbft_block(SignedPbftBlockRlp::new(&pbft_block)),
        )
        .expect_err("mismatched admitted identity must be rejected");
        assert!(
            error
                .to_string()
                .contains("FINAL_CHAIN_BLOCK_NUMBER_METADATA_MISMATCH")
        );
    }

    #[test]
    fn rejects_malformed_stored_block_header_rlp() {
        let mut malformed = RlpStream::new_list(1);
        malformed.append(&H256::from_low_u64_be(1));

        assert!(
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&malformed.out()))
                .is_err()
        );
    }

    #[test]
    fn rejects_stored_header_with_wrong_log_bloom_width() {
        let mut malformed = RlpStream::new_list(7);
        malformed.append(&H256::from_low_u64_be(1));
        malformed.append(&H256::from_low_u64_be(2));
        malformed.append(&H256::from_low_u64_be(3));
        malformed.append(&H256::from_low_u64_be(4));
        malformed.append(&[0u8; 255].as_slice());
        malformed.append(&5u64);
        malformed.append(&U256::from(6u64));

        let error =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&malformed.out()))
                .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("FINAL_CHAIN_STORED_HEADER_LOG_BLOOM_INVALID_LENGTH:")
        );
    }

    #[test]
    fn legacy_header_input_propagates_invalid_pbft_signature() {
        let raw_header = header_data_rlp(5, U256::from(6u64));
        let mut invalid_pbft = RlpStream::new_list(9);
        append_pbft_block_fields(&mut invalid_pbft, 1, 2, &[]);
        invalid_pbft.append(&vec![0u8; 64]);

        let err = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(StoredBlockHeaderRlp::new(&raw_header), 1000, 1234)
                .signed_pbft_block(SignedPbftBlockRlp::new(&invalid_pbft.out())),
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("could not recover PBFT block proposer")
        );
    }
}
