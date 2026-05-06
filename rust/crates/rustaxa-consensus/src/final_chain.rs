use anyhow::Result;
use ethereum_types::{H256, U256};
use keccak_hasher::KeccakHasher;
use rlp::Rlp;
use rustaxa_storage::Storage;
use rustaxa_types::codec::rlp::final_chain::{
    LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
    StoredBlockHeaderRlpOwned,
};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::{
    Account, FinalizationTransaction, GenesisAccount, GenesisValidator, StoredFinalChainBlockHeader,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use triehash::ordered_trie_root;

const EMPTY_TRIE_ROOT: [u8; 32] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];
const VALUE_TRANSFER_GAS: u64 = 21_000;

/// Rust final-chain domain surface used by the C++ shim.
pub struct FinalChain {
    storage: Arc<Storage>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    accounts: Mutex<HashMap<[u8; 20], Account>>,
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

        let final_chain = FinalChain {
            storage,
            block_gas_limit,
            genesis_timestamp,
            accounts: Mutex::new(genesis_accounts),
            genesis_vrf_keys,
        };
        final_chain.ensure_genesis_header()?;
        Ok(final_chain)
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

    /// Returns the latest in-memory account view tracked by Rust finalization.
    pub fn account(&self, address: [u8; 20]) -> Result<Option<Account>, anyhow::Error> {
        Ok(self
            .accounts
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?
            .get(&address)
            .cloned())
    }

    pub fn vrf_key(&self, address: [u8; 20]) -> Result<Option<[u8; 32]>, anyhow::Error> {
        Ok(self.genesis_vrf_keys.get(&address).copied())
    }

    pub fn estimate_call_gas(&self, gas_limit: u64) -> Result<u64, anyhow::Error> {
        Ok(gas_limit)
    }

    /// Returns canonical transaction RLPs for a finalized period.
    pub fn transaction_rlps(&self, period: u64) -> Result<Vec<Vec<u8>>, anyhow::Error> {
        let period_data = self.storage.period().data_raw(period)?;
        if period_data.is_empty() {
            return Ok(vec![]);
        }
        let period_data_rlp = Rlp::new(&period_data);
        let transactions = period_data_rlp.at(3)?;
        let mut transaction_rlps = Vec::with_capacity(transactions.item_count()?);
        for i in 0..transactions.item_count()? {
            transaction_rlps.push(transactions.at(i)?.as_raw().to_vec());
        }
        Ok(transaction_rlps)
    }

    /// Returns one finalized transaction receipt RLP by block period and position.
    pub fn transaction_receipt_rlp(
        &self,
        period: u64,
        position: u64,
    ) -> Result<Option<Vec<u8>>, anyhow::Error> {
        let receipts_rlp = self.storage.period().receipt(period)?;
        if receipts_rlp.is_empty() {
            return Ok(None);
        }
        let receipts = Rlp::new(&receipts_rlp);
        if position as usize >= receipts.item_count()? {
            return Ok(None);
        }
        Ok(Some(receipts.at(position as usize)?.as_raw().to_vec()))
    }

    /// Finalizes a PBFT block using the Rust-owned native transfer executor.
    pub fn finalize_block(
        &self,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<FinalizationTransaction>,
    ) -> Result<(Vec<u8>, Vec<Vec<u8>>), anyhow::Error> {
        let pbft =
            rustaxa_types::PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&pbft_block_rlp))?;
        let transaction_count = self.transaction_count(pbft.period)?;
        if transaction_count != transactions.len() as u64 {
            anyhow::bail!(
                "Rust FinalChain::finalize transaction count mismatch: period data has {transaction_count}, bridge provided {}",
                transactions.len()
            );
        }

        let execution = self.execute_native_transactions(pbft.author, &transactions)?;
        let receipts_rlp = encode_receipts_rlp(&execution.receipts);
        let parent_hash = self
            .block_hash(self.last_block_number()?)?
            .map(|bytes| h256_from_slice(&bytes, "parent final-chain hash"))
            .transpose()?
            .unwrap_or_default();
        let stored_header = StoredFinalChainBlockHeader {
            parent_hash,
            state_root: synthetic_state_root(pbft.period),
            transactions_root: ordered_root(
                transactions
                    .iter()
                    .map(|transaction| transaction.rlp.as_slice()),
            ),
            receipts_root: ordered_root(
                execution.receipts.iter().map(|receipt| receipt.as_slice()),
            ),
            log_bloom: vec![0; 256],
            gas_used: execution.gas_used,
            total_reward: ethereum_types::U256::zero(),
        };
        let stored_header_rlp = StoredBlockHeaderRlpOwned::from(&stored_header);
        let full_header = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(
                StoredBlockHeaderRlp::new(stored_header_rlp.as_bytes()),
                self.block_gas_limit,
                self.genesis_timestamp,
            )
            .signed_pbft_block(SignedPbftBlockRlp::new(&pbft_block_rlp)),
        )?;
        self.storage.final_chain().write_block_header(
            pbft.period,
            full_header.hash()?,
            stored_header_rlp.as_bytes(),
            receipts_rlp.as_slice(),
        )?;
        for (position, transaction) in transactions.iter().enumerate() {
            self.storage.transaction().write_location(
                H256::from(transaction.hash),
                pbft.period,
                position as u32,
                false,
            )?;
            self.storage.final_chain().write_receipt_by_trx_hash(
                H256::from(transaction.hash),
                &execution.receipts[position],
            )?;
        }

        Ok((full_header.into_vec(), execution.receipts))
    }

    fn ensure_genesis_header(&self) -> Result<(), anyhow::Error> {
        if self
            .storage
            .final_chain()
            .meta_value(Self::DB_META_LAST_NUMBER)?
            .is_some()
        {
            return Ok(());
        }
        if self.storage.final_chain().block_header_raw(0)?.is_some() {
            return Ok(());
        }

        let stored_header = StoredFinalChainBlockHeader {
            parent_hash: ethereum_types::H256::zero(),
            state_root: synthetic_state_root(0),
            transactions_root: empty_trie_root(),
            receipts_root: empty_trie_root(),
            log_bloom: vec![0; 256],
            gas_used: 0,
            total_reward: ethereum_types::U256::zero(),
        };
        let stored_header_rlp = StoredBlockHeaderRlpOwned::from(&stored_header);
        let full_header = LegacyBlockHeaderRlp::try_from(LegacyBlockHeaderRlpInput::new(
            StoredBlockHeaderRlp::new(stored_header_rlp.as_bytes()),
            self.block_gas_limit,
            self.genesis_timestamp,
        ))?;
        self.storage.final_chain().write_block_header(
            0,
            full_header.hash()?,
            stored_header_rlp.as_bytes(),
            empty_receipts_rlp().as_slice(),
        )
    }

    fn execute_native_transactions(
        &self,
        beneficiary: ethereum_types::H160,
        transactions: &[FinalizationTransaction],
    ) -> Result<NativeExecution, anyhow::Error> {
        let mut accounts = self
            .accounts
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?;
        let mut receipts = Vec::with_capacity(transactions.len());
        let mut cumulative_gas_used = 0u64;

        for transaction in transactions {
            if !transaction.data.is_empty() || transaction.receiver.is_none() {
                anyhow::bail!(
                    "Rust FinalChain::finalize currently supports only native value transfers"
                );
            }
            let receiver_address = transaction.receiver.ok_or_else(|| {
                anyhow::anyhow!("native value transfer missing receiver after validation")
            })?;
            let gas_price = u256_from_big_endian(&transaction.gas_price);
            let value = u256_from_big_endian(&transaction.value);

            let mut status_code = 1u8;
            let gas_used;
            let gas_cost;
            {
                let sender = accounts
                    .entry(transaction.sender)
                    .or_insert_with(empty_account);
                let sender_balance = u256_from_big_endian(&sender.balance);
                let full_gas_cost = gas_price
                    .checked_mul(U256::from(transaction.gas_limit))
                    .ok_or_else(|| anyhow::anyhow!("transaction gas limit cost overflow"))?;
                if sender.nonce > transaction.nonce || sender_balance < full_gas_cost {
                    status_code = 0;
                    gas_used = affordable_gas(sender, gas_price, transaction.gas_limit);
                } else {
                    gas_used = VALUE_TRANSFER_GAS;
                }

                gas_cost = gas_price
                    .checked_mul(U256::from(gas_used))
                    .ok_or_else(|| anyhow::anyhow!("transaction gas cost overflow"))?;
                if status_code == 1 {
                    let total_cost = gas_cost
                        .checked_add(value)
                        .ok_or_else(|| anyhow::anyhow!("transaction total cost overflow"))?;
                    if sender_balance < total_cost {
                        anyhow::bail!(
                            "Rust FinalChain::finalize cannot apply underfunded native transfer"
                        );
                    }
                    sender.balance = u256_to_big_endian(sender_balance - total_cost);
                    sender.nonce = transaction
                        .nonce
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("transaction nonce overflow"))?;
                } else {
                    sender.balance = u256_to_big_endian(sender_balance.saturating_sub(gas_cost));
                }
            };
            cumulative_gas_used = cumulative_gas_used
                .checked_add(gas_used)
                .ok_or_else(|| anyhow::anyhow!("cumulative gas used overflow"))?;

            if status_code == 1 {
                let receiver = accounts
                    .entry(receiver_address)
                    .or_insert_with(empty_account);
                let receiver_balance = u256_from_big_endian(&receiver.balance);
                receiver.balance = u256_to_big_endian(
                    receiver_balance
                        .checked_add(value)
                        .ok_or_else(|| anyhow::anyhow!("receiver balance overflow"))?,
                );
            }
            if !gas_cost.is_zero() {
                let beneficiary = accounts
                    .entry(h160_to_address(beneficiary))
                    .or_insert_with(empty_account);
                let beneficiary_balance = u256_from_big_endian(&beneficiary.balance);
                beneficiary.balance = u256_to_big_endian(
                    beneficiary_balance
                        .checked_add(gas_cost)
                        .ok_or_else(|| anyhow::anyhow!("beneficiary balance overflow"))?,
                );
            }

            receipts.push(encode_receipt_rlp(
                status_code,
                gas_used,
                cumulative_gas_used,
            ));
        }

        Ok(NativeExecution {
            receipts,
            gas_used: cumulative_gas_used,
        })
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

fn h256_from_slice(raw: &[u8], field: &str) -> Result<ethereum_types::H256, anyhow::Error> {
    if raw.len() != 32 {
        anyhow::bail!("invalid {field} size: expected 32, got {}", raw.len());
    }
    Ok(ethereum_types::H256::from_slice(raw))
}

fn empty_trie_root() -> ethereum_types::H256 {
    ethereum_types::H256::from(EMPTY_TRIE_ROOT)
}

fn empty_receipts_rlp() -> Vec<u8> {
    rlp::RlpStream::new_list(0).out().to_vec()
}

fn encode_receipts_rlp(receipts: &[Vec<u8>]) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(receipts.len());
    for receipt in receipts {
        stream.append_raw(receipt, 1);
    }
    stream.out().to_vec()
}

fn encode_receipt_rlp(status_code: u8, gas_used: u64, cumulative_gas_used: u64) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(5);
    stream.append(&status_code);
    stream.append(&gas_used);
    stream.append(&cumulative_gas_used);
    stream.begin_list(0);
    stream.append(&0u8);
    stream.out().to_vec()
}

fn ordered_root<'a>(values: impl Iterator<Item = &'a [u8]>) -> H256 {
    H256::from_slice(ordered_trie_root::<KeccakHasher, _>(values).as_ref())
}

fn u256_from_big_endian(bytes: &[u8]) -> U256 {
    U256::from_big_endian(bytes)
}

fn u256_to_big_endian(value: U256) -> Vec<u8> {
    let bytes = value.to_big_endian();
    let first_nonzero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    bytes[first_nonzero..].to_vec()
}

fn empty_account() -> Account {
    Account {
        nonce: 0,
        balance: vec![],
        storage_root_hash: [0; 32],
        code_hash: [0; 32],
        code_size: 0,
    }
}

fn h160_to_address(value: ethereum_types::H160) -> [u8; 20] {
    let mut address = [0u8; 20];
    address.copy_from_slice(value.as_bytes());
    address
}

fn affordable_gas(account: &Account, gas_price: U256, gas_limit: u64) -> u64 {
    if gas_price.is_zero() {
        return gas_limit;
    }
    let affordable = u256_from_big_endian(&account.balance) / gas_price;
    affordable.min(U256::from(gas_limit)).as_u64()
}

fn synthetic_state_root(period: u64) -> ethereum_types::H256 {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-state-root");
    hasher.update(&period.to_le_bytes());
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    ethereum_types::H256::from(output)
}

/// Result of applying the Rust native-transfer subset for one final-chain block.
struct NativeExecution {
    receipts: Vec<Vec<u8>>,
    gas_used: u64,
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
