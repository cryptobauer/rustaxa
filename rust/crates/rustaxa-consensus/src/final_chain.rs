use anyhow::Result;
use rlp::Rlp;
use rustaxa_storage::Storage;
use rustaxa_types::codec::rlp::final_chain::{
    LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use std::collections::HashMap;
use std::sync::Arc;

/// Genesis account data needed by the Rust final-chain shim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenesisAccount {
    pub address: [u8; 20],
    pub balance: Vec<u8>,
}

/// Genesis validator key data needed by the Rust final-chain shim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenesisValidator {
    pub address: [u8; 20],
    pub vrf_key: [u8; 32],
}

/// Account view returned through the C++ bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Account {
    pub nonce: u64,
    pub balance: Vec<u8>,
    pub storage_root_hash: [u8; 32],
    pub code_hash: [u8; 32],
    pub code_size: u64,
}

/// Rust final-chain domain surface used by the C++ shim.
pub struct FinalChain {
    storage: Arc<Storage>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    genesis_accounts: HashMap<[u8; 20], Account>,
    genesis_vrf_keys: HashMap<[u8; 20], [u8; 32]>,
}

impl FinalChain {
    const DB_META_LAST_NUMBER: u32 = 1;
    const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;

    pub fn new(
        storage: Arc<Storage>,
        block_gas_limit: u64,
        genesis_timestamp: u64,
        genesis_accounts: Vec<GenesisAccount>,
        genesis_validators: Vec<GenesisValidator>,
    ) -> Result<Self> {
        let genesis_accounts = genesis_accounts
            .into_iter()
            .map(|account| {
                (
                    account.address,
                    Account {
                        nonce: 0,
                        balance: account.balance,
                        storage_root_hash: [0; 32],
                        code_hash: [0; 32],
                        code_size: 0,
                    },
                )
            })
            .collect();
        let genesis_vrf_keys = genesis_validators
            .into_iter()
            .map(|validator| (validator.address, validator.vrf_key))
            .collect();

        Ok(FinalChain {
            storage,
            block_gas_limit,
            genesis_timestamp,
            genesis_accounts,
            genesis_vrf_keys,
        })
    }

    pub fn last_block_number(&self) -> Result<u64, anyhow::Error> {
        let Some(raw) = self
            .storage
            .final_chain()
            .meta_value(Self::DB_META_LAST_NUMBER)?
        else {
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
                    .at(Self::PBFT_BLOCK_POS_IN_PERIOD_DATA)?
                    .as_raw()
                    .to_vec(),
            )
        };
        let mut header_input = LegacyBlockHeaderRlpInput::new(
            StoredBlockHeaderRlp::new(&raw_header),
            self.block_gas_limit,
            self.genesis_timestamp,
        );
        if let Some(pbft_block) = pbft_block.as_deref() {
            header_input = header_input.signed_pbft_block(SignedPbftBlockRlp::new(pbft_block));
        }

        Ok(Some(
            LegacyBlockHeaderRlp::try_from(header_input)?.into_vec(),
        ))
    }

    pub fn transaction_location(&self, hash: [u8; 32]) -> Result<Option<Vec<u8>>, anyhow::Error> {
        self.storage
            .transaction()
            .location_rlp(ethereum_types::H256::from(hash))
    }

    pub fn transaction_count(&self, period: u64) -> Result<u64, anyhow::Error> {
        self.storage.transaction().count(period)
    }

    pub fn account(&self, address: [u8; 20]) -> Result<Option<Account>, anyhow::Error> {
        Ok(self.genesis_accounts.get(&address).cloned())
    }

    pub fn vrf_key(&self, address: [u8; 20]) -> Result<Option<[u8; 32]>, anyhow::Error> {
        Ok(self.genesis_vrf_keys.get(&address).copied())
    }

    pub fn estimate_call_gas(&self, gas_limit: u64) -> Result<u64, anyhow::Error> {
        Ok(gas_limit)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H256, U256};
    use rlp::RlpStream;
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

    #[test]
    fn last_block_number_returns_zero_when_missing() {
        let path = temp_db_path("last-missing");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = FinalChain::new(storage.clone(), 0, 0, vec![], vec![]).unwrap();

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
                &FinalChain::DB_META_LAST_NUMBER.to_le_bytes(),
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

        let final_chain = FinalChain::new(storage.clone(), 0, 0, vec![], vec![]).unwrap();

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

        let final_chain = FinalChain::new(
            storage.clone(),
            block_gas_limit,
            genesis_timestamp,
            vec![],
            vec![],
        )
        .unwrap();

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
}
