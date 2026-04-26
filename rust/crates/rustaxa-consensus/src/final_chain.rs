use anyhow::Result;
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use rustaxa_storage::Storage;
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

const DB_META_LAST_NUMBER: u32 = 1;
const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;
const PBFT_PERIOD_POS: usize = 4;
const PBFT_TIMESTAMP_POS: usize = 5;
const PBFT_EXTRA_DATA_POS: usize = 7;
const BLOCK_HEADER_PARENT_HASH_POS: usize = 0;
const BLOCK_HEADER_STATE_ROOT_POS: usize = 1;
const BLOCK_HEADER_TRANSACTIONS_ROOT_POS: usize = 2;
const BLOCK_HEADER_RECEIPTS_ROOT_POS: usize = 3;
const BLOCK_HEADER_LOG_BLOOM_POS: usize = 4;
const BLOCK_HEADER_GAS_USED_POS: usize = 5;
const BLOCK_HEADER_TOTAL_REWARD_POS: usize = 6;
const EMPTY_UNCLES_HASH: [u8; 32] = [
    0x1d, 0xcc, 0x4d, 0xe8, 0xde, 0xc7, 0x5d, 0x7a, 0xab, 0x85, 0xb5, 0x67, 0xb6, 0xcc, 0xd4, 0x1a,
    0xd3, 0x12, 0x45, 0x1b, 0x94, 0x8a, 0x74, 0x13, 0xf0, 0xa1, 0x42, 0xfd, 0x40, 0xd4, 0x93, 0x47,
];

pub struct FinalChain {
    storage: Arc<Storage>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
}

impl FinalChain {
    pub fn new(
        storage: Arc<Storage>,
        block_gas_limit: u64,
        genesis_timestamp: u64,
    ) -> Result<Self> {
        Ok(FinalChain {
            storage,
            block_gas_limit,
            genesis_timestamp,
        })
    }

    pub fn last_block_number(&self) -> Result<u64, anyhow::Error> {
        let Some(raw) = self.storage.final_chain().meta_value(DB_META_LAST_NUMBER)? else {
            return Ok(0);
        };
        decode_u64_le(&raw, "final_chain_meta/LAST_NUMBER")
    }

    pub fn block_number(&self, hash: [u8; 32]) -> Result<Option<u64>, anyhow::Error> {
        let Some(raw) = self
            .storage
            .final_chain()
            .block_number_by_hash(ethereum_types::H256::from(hash))?
        else {
            return Ok(None);
        };
        Ok(Some(decode_u64_le(&raw, "final_chain_blk_number_by_hash")?))
    }

    pub fn block_hash(&self, num: u64) -> Result<Option<Vec<u8>>, anyhow::Error> {
        self.storage.final_chain().block_hash_by_number(num)
    }

    pub fn block_header(&self, num: u64) -> Result<Option<Vec<u8>>, anyhow::Error> {
        let Some(raw_header) = self.storage.final_chain().block_header_raw(num)? else {
            return Ok(None);
        };
        let pbft_block = if num == 0 {
            None
        } else {
            let period_data = self.storage.period().data_raw(num)?;
            if period_data.is_empty() {
                return Ok(None);
            }
            let period_data_rlp = Rlp::new(&period_data);
            Some(
                period_data_rlp
                    .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)?
                    .as_raw()
                    .to_vec(),
            )
        };
        Ok(Some(build_block_header_rlp(
            &raw_header,
            pbft_block.as_deref(),
            self.block_gas_limit,
            self.genesis_timestamp,
        )?))
    }

    pub fn transaction_location(&self, hash: [u8; 32]) -> Result<Option<Vec<u8>>, anyhow::Error> {
        self.storage
            .transaction()
            .location_rlp(ethereum_types::H256::from(hash))
    }

    pub fn transaction_count(&self, period: u64) -> Result<u64, anyhow::Error> {
        self.storage.transaction().count(period)
    }
}

fn decode_u64_le(raw: &[u8], field: &str) -> Result<u64, anyhow::Error> {
    if raw.len() != std::mem::size_of::<u64>() {
        anyhow::bail!(
            "invalid {field} value size: expected {}, got {}",
            std::mem::size_of::<u64>(),
            raw.len()
        );
    }

    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(raw);
    Ok(u64::from_le_bytes(bytes))
}

fn build_block_header_rlp(
    raw_header: &[u8],
    pbft_block: Option<&[u8]>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
) -> Result<Vec<u8>, anyhow::Error> {
    let header_data = Rlp::new(raw_header);
    let data = BlockHeaderData::decode(&header_data)?;
    let pbft = match pbft_block {
        Some(pbft_block) => Some(PbftBlockHeaderData::decode(&Rlp::new(pbft_block))?),
        None => None,
    };

    let author = pbft.as_ref().map(|pbft| pbft.author).unwrap_or_default();
    let number = pbft.as_ref().map(|pbft| pbft.period).unwrap_or_default();
    let timestamp = pbft
        .as_ref()
        .map(|pbft| pbft.timestamp)
        .unwrap_or(genesis_timestamp);
    let extra_data = pbft
        .as_ref()
        .map(|pbft| pbft.extra_data.as_slice())
        .unwrap_or_default();
    let hash = block_header_hash(
        &data,
        author,
        number,
        block_gas_limit,
        timestamp,
        extra_data,
    );

    let mut stream = RlpStream::new_list(13);
    stream.append(&hash);
    stream.append(&data.parent_hash);
    stream.append(&author);
    stream.append(&data.state_root);
    stream.append(&data.transactions_root);
    stream.append(&data.receipts_root);
    stream.append(&data.log_bloom.as_slice());
    stream.append(&number);
    stream.append(&block_gas_limit);
    stream.append(&data.gas_used);
    stream.append(&timestamp);
    stream.append(&data.total_reward);
    stream.append(&extra_data);
    Ok(stream.out().to_vec())
}

fn block_header_hash(
    data: &BlockHeaderData,
    author: H160,
    number: u64,
    gas_limit: u64,
    timestamp: u64,
    extra_data: &[u8],
) -> H256 {
    let empty_uncles_hash = H256::from(EMPTY_UNCLES_HASH);
    let zero_hash = H256::zero();
    let zero_nonce = [0u8; 8];
    let mut stream = RlpStream::new_list(15);
    stream.append(&data.parent_hash);
    stream.append(&empty_uncles_hash);
    stream.append(&author);
    stream.append(&data.state_root);
    stream.append(&data.transactions_root);
    stream.append(&data.receipts_root);
    stream.append(&data.log_bloom.as_slice());
    stream.append(&U256::zero());
    stream.append(&number);
    stream.append(&gas_limit);
    stream.append(&data.gas_used);
    stream.append(&timestamp);
    stream.append(&extra_data);
    stream.append(&zero_hash);
    stream.append(&zero_nonce.as_slice());
    keccak256(&stream.out())
}

struct BlockHeaderData {
    parent_hash: H256,
    state_root: H256,
    transactions_root: H256,
    receipts_root: H256,
    log_bloom: Vec<u8>,
    gas_used: u64,
    total_reward: U256,
}

impl BlockHeaderData {
    fn decode(rlp: &Rlp<'_>) -> Result<Self, anyhow::Error> {
        Ok(BlockHeaderData {
            parent_hash: rlp.val_at(BLOCK_HEADER_PARENT_HASH_POS)?,
            state_root: rlp.val_at(BLOCK_HEADER_STATE_ROOT_POS)?,
            transactions_root: rlp.val_at(BLOCK_HEADER_TRANSACTIONS_ROOT_POS)?,
            receipts_root: rlp.val_at(BLOCK_HEADER_RECEIPTS_ROOT_POS)?,
            log_bloom: rlp.at(BLOCK_HEADER_LOG_BLOOM_POS)?.data()?.to_vec(),
            gas_used: rlp.val_at(BLOCK_HEADER_GAS_USED_POS)?,
            total_reward: rlp.val_at(BLOCK_HEADER_TOTAL_REWARD_POS)?,
        })
    }
}

struct PbftBlockHeaderData {
    author: H160,
    period: u64,
    timestamp: u64,
    extra_data: Vec<u8>,
}

impl PbftBlockHeaderData {
    fn decode(rlp: &Rlp<'_>) -> Result<Self, anyhow::Error> {
        let item_count = rlp.item_count()?;
        let author = recover_pbft_block_proposer(rlp.as_raw())
            .ok_or_else(|| anyhow::anyhow!("could not recover PBFT block proposer"))?;
        let extra_data = if item_count == 9 {
            rlp.at(PBFT_EXTRA_DATA_POS)?.data()?.to_vec()
        } else {
            Vec::new()
        };
        Ok(PbftBlockHeaderData {
            author: H160::from(author),
            period: rlp.val_at(PBFT_PERIOD_POS)?,
            timestamp: rlp.val_at(PBFT_TIMESTAMP_POS)?,
            extra_data,
        })
    }
}

fn recover_pbft_block_proposer(block_rlp: &[u8]) -> Option<[u8; 20]> {
    let rlp = Rlp::new(block_rlp);
    let item_count = rlp.item_count().ok()?;
    if item_count < 8 {
        return None;
    }

    let sig: Vec<u8> = rlp.val_at(item_count - 1).ok()?;
    let mut stream = RlpStream::new_list(item_count - 1);
    for i in 0..item_count - 1 {
        stream.append_raw(rlp.at(i).ok()?.as_raw(), 1);
    }
    let msg = keccak256(&stream.out());

    ecrecover_address(&sig, &msg)
}

fn ecrecover_address(sig: &[u8], msg: &H256) -> Option<[u8; 20]> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    if sig.len() != 65 {
        return None;
    }
    let recovery_id = RecoveryId::try_from(sig[64] % 4).ok()?;
    let signature = Signature::try_from(&sig[..64]).ok()?;
    let recovered_key =
        VerifyingKey::recover_from_prehash(msg.as_bytes(), &signature, recovery_id).ok()?;
    let uncompressed = recovered_key.to_encoded_point(false);
    let pubkey_hash = keccak256(&uncompressed.as_bytes()[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&pubkey_hash.as_bytes()[12..]);
    Some(addr)
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
    use rustaxa_storage::{Column, Config};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(test_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustaxa-consensus-final-chain-{test_name}-{}-{nanos}",
            std::process::id()
        ))
    }

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
    fn last_block_number_returns_zero_when_missing() {
        let path = temp_db_path("last-missing");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = FinalChain::new(storage.clone(), 0, 0).unwrap();

        assert_eq!(final_chain.last_block_number().unwrap(), 0);

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn reads_batch_one_indexes() {
        let path = temp_db_path("batch-one-indexes");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut batch = storage.create_write_batch();
        let block_number = 42u64;
        let block_hash = [0xAB; 32];

        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainMeta,
                &DB_META_LAST_NUMBER.to_le_bytes(),
                &block_number.to_le_bytes(),
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainBlkHashByNumber,
                &block_number.to_le_bytes(),
                &block_hash,
            )
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainBlkNumberByHash,
                &block_hash,
                &block_number.to_le_bytes(),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let final_chain = FinalChain::new(storage.clone(), 0, 0).unwrap();

        assert_eq!(final_chain.last_block_number().unwrap(), block_number);
        assert_eq!(
            final_chain.block_hash(block_number).unwrap(),
            Some(block_hash.to_vec())
        );
        assert_eq!(
            final_chain.block_number(block_hash).unwrap(),
            Some(block_number)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn reads_batch_two_indexes() {
        let path = temp_db_path("batch-two-indexes");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut batch = storage.create_write_batch();
        let block_number = 0u64;
        let block_gas_limit = 1000u64;
        let genesis_timestamp = 1234u64;
        let header = header_data_rlp(5, U256::from(6u64));
        let tx_period = 7u64;
        let tx_hash = [0xCD; 32];
        let tx_location = vec![0xC2, 0x07, 0x03];
        let period_data = vec![0xC8, 0xC0, 0xC0, 0xC0, 0xC4, 0x81, 0xAA, 0x81, 0xBB];

        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainBlkByNumber,
                &block_number.to_le_bytes(),
                &header,
            )
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::TrxPeriod, &tx_hash, &tx_location)
            .unwrap();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PeriodData,
                &tx_period.to_le_bytes(),
                &period_data,
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let final_chain =
            FinalChain::new(storage.clone(), block_gas_limit, genesis_timestamp).unwrap();

        let full_header = final_chain.block_header(block_number).unwrap().unwrap();
        let full_header_rlp = Rlp::new(&full_header);
        assert_eq!(full_header_rlp.item_count().unwrap(), 13);
        assert_eq!(
            full_header_rlp.val_at::<H256>(1).unwrap(),
            H256::from_low_u64_be(1)
        );
        assert_eq!(full_header_rlp.val_at::<u64>(7).unwrap(), block_number);
        assert_eq!(full_header_rlp.val_at::<u64>(8).unwrap(), block_gas_limit);
        assert_eq!(
            full_header_rlp.val_at::<u64>(10).unwrap(),
            genesis_timestamp
        );
        assert_eq!(
            final_chain.transaction_location(tx_hash).unwrap(),
            Some(tx_location)
        );
        assert_eq!(final_chain.transaction_count(tx_period).unwrap(), 2);

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn block_header_reconstructs_non_genesis_pbft_fields() {
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

        let full_header =
            build_block_header_rlp(&raw_header, Some(&pbft_block), block_gas_limit, 0).unwrap();
        let full_header_rlp = Rlp::new(&full_header);

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
}
