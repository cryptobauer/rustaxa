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
    Account, FinalizationTransaction, GenesisAccount, GenesisDposConfig, GenesisValidator,
    StoredFinalChainBlockHeader,
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
    genesis_dpos_vote_counts: HashMap<[u8; 20], u64>,
    genesis_dpos_total_vote_count: u64,
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
        genesis_dpos_config: GenesisDposConfig,
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
            .map(|validator| {
                let vote_count = dpos_vote_count(
                    &validator.total_stake,
                    &genesis_dpos_config.eligibility_balance_threshold,
                    &genesis_dpos_config.vote_eligibility_balance_step,
                    &genesis_dpos_config.validator_maximum_stake,
                )?;
                Ok((validator.address, validator.vrf_key, vote_count))
            })
            .collect::<Result<Vec<_>>>()?;
        let genesis_dpos_vote_counts = genesis_vrf_keys
            .iter()
            .map(|(address, _, vote_count)| (*address, *vote_count))
            .collect::<HashMap<_, _>>();
        let genesis_dpos_total_vote_count =
            genesis_vrf_keys
                .iter()
                .try_fold(0u64, |total, (_, _, vote_count)| {
                    total
                        .checked_add(*vote_count)
                        .ok_or_else(|| anyhow::anyhow!("genesis DPoS total vote count overflow"))
                })?;
        let genesis_vrf_keys = genesis_vrf_keys
            .into_iter()
            .map(|(address, vrf_key, _)| (address, vrf_key))
            .collect();

        let final_chain = FinalChain {
            storage,
            block_gas_limit,
            genesis_timestamp,
            accounts: Mutex::new(genesis_accounts),
            genesis_vrf_keys,
            genesis_dpos_vote_counts,
            genesis_dpos_total_vote_count,
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

    /// Returns the genesis DPoS eligible vote count for one validator address.
    pub fn dpos_eligible_vote_count(&self, address: [u8; 20]) -> Result<u64, anyhow::Error> {
        Ok(*self.genesis_dpos_vote_counts.get(&address).unwrap_or(&0))
    }

    /// Returns the total genesis DPoS eligible vote count.
    pub fn dpos_eligible_total_vote_count(&self) -> Result<u64, anyhow::Error> {
        Ok(self.genesis_dpos_total_vote_count)
    }

    /// Returns whether the validator has nonzero genesis DPoS eligible votes.
    pub fn dpos_is_eligible(&self, address: [u8; 20]) -> Result<bool, anyhow::Error> {
        Ok(self.dpos_eligible_vote_count(address)? > 0)
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

fn dpos_vote_count(
    stake: &[u8],
    eligibility_balance_threshold: &[u8],
    vote_eligibility_balance_step: &[u8],
    validator_maximum_stake: &[u8],
) -> Result<u64, anyhow::Error> {
    let stake = u256_from_big_endian(stake);
    let eligibility_balance_threshold = u256_from_big_endian(eligibility_balance_threshold);
    let vote_eligibility_balance_step = u256_from_big_endian(vote_eligibility_balance_step);
    let validator_maximum_stake = u256_from_big_endian(validator_maximum_stake);
    if stake > validator_maximum_stake {
        anyhow::bail!("genesis DPoS validator stake exceeds maximum stake");
    }
    if vote_eligibility_balance_step.is_zero() || stake < eligibility_balance_threshold {
        return Ok(0);
    }

    let votes = stake / vote_eligibility_balance_step;
    if votes > U256::from(u64::MAX) {
        anyhow::bail!("genesis DPoS vote count does not fit into u64");
    }
    Ok(votes.as_u64())
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
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::{Rlp, RlpStream};
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

    fn keccak256(data: &[u8]) -> H256 {
        use tiny_keccak::{Hasher, Keccak};

        let mut hasher = Keccak::v256();
        hasher.update(data);
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        H256::from(output)
    }

    fn append_pbft_block_fields(stream: &mut RlpStream, period: u64, timestamp: u64) {
        stream.append(&H256::from_low_u64_be(10));
        stream.append(&H256::from_low_u64_be(11));
        stream.append(&H256::from_low_u64_be(12));
        stream.append(&H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&timestamp);
        stream.begin_list(0);
    }

    fn signed_pbft_block(signing_key: &SigningKey, period: u64, timestamp: u64) -> Vec<u8> {
        let mut unsigned_stream = RlpStream::new_list(7);
        append_pbft_block_fields(&mut unsigned_stream, period, timestamp);
        let message_hash = keccak256(&unsigned_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed_stream = RlpStream::new_list(8);
        append_pbft_block_fields(&mut signed_stream, period, timestamp);
        signed_stream.append(&signature_bytes);
        signed_stream.out().to_vec()
    }

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    fn period_data_rlp(pbft_block_rlp: &[u8], transaction_rlps: &[Vec<u8>]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(4);
        stream.append_raw(pbft_block_rlp, 1);
        stream.begin_list(0);
        stream.begin_list(0);
        stream.begin_list(transaction_rlps.len());
        for transaction_rlp in transaction_rlps {
            stream.append_raw(transaction_rlp, 1);
        }
        stream.out().to_vec()
    }

    fn write_period_data(
        storage: &Storage,
        period: u64,
        pbft_block_rlp: &[u8],
        transaction_rlps: &[Vec<u8>],
    ) {
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PeriodData,
                &period.to_le_bytes(),
                &period_data_rlp(pbft_block_rlp, transaction_rlps),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
    }

    fn test_transaction(
        hash_byte: u8,
        sender: [u8; 20],
        receiver: Option<[u8; 20]>,
        nonce: u64,
        value: U256,
        gas_price: U256,
        gas_limit: u64,
        data: Vec<u8>,
        rlp: Vec<u8>,
    ) -> FinalizationTransaction {
        FinalizationTransaction {
            hash: [hash_byte; 32],
            sender,
            receiver,
            nonce,
            value: u256_to_big_endian(value),
            gas_price: u256_to_big_endian(gas_price),
            gas_limit,
            data,
            rlp,
        }
    }

    fn genesis_account(address: [u8; 20], balance: U256) -> GenesisAccount {
        GenesisAccount {
            address,
            balance: u256_to_big_endian(balance),
        }
    }

    fn genesis_validator(address: [u8; 20], stake: U256) -> GenesisValidator {
        GenesisValidator {
            address,
            vrf_key: [address[0]; 32],
            total_stake: u256_to_big_endian(stake),
        }
    }

    fn receipt_fields(receipt_rlp: &[u8]) -> (u8, u64, u64) {
        let receipt = Rlp::new(receipt_rlp);
        (
            receipt.val_at(0).unwrap(),
            receipt.val_at(1).unwrap(),
            receipt.val_at(2).unwrap(),
        )
    }

    fn balance_of(final_chain: &FinalChain, address: [u8; 20]) -> U256 {
        final_chain
            .account(address)
            .unwrap()
            .map(|account| u256_from_big_endian(&account.balance))
            .unwrap_or_default()
    }

    fn new_final_chain(
        storage: Arc<Storage>,
        block_gas_limit: u64,
        genesis_timestamp: u64,
        genesis_accounts: Vec<GenesisAccount>,
        genesis_validators: Vec<GenesisValidator>,
    ) -> FinalChain {
        FinalChain::new(
            storage,
            block_gas_limit,
            genesis_timestamp,
            genesis_accounts,
            genesis_validators,
            GenesisDposConfig::default(),
        )
        .unwrap()
    }

    fn new_final_chain_with_dpos(
        storage: Arc<Storage>,
        genesis_validators: Vec<GenesisValidator>,
        threshold: U256,
        vote_step: U256,
        maximum_stake: U256,
    ) -> FinalChain {
        FinalChain::new(
            storage,
            0,
            0,
            vec![],
            genesis_validators,
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(threshold),
                vote_eligibility_balance_step: u256_to_big_endian(vote_step),
                validator_maximum_stake: u256_to_big_endian(maximum_stake),
            },
        )
        .unwrap()
    }

    #[test]
    fn last_block_number_returns_zero_when_missing() {
        let path = temp_db_path("last-missing");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = new_final_chain(storage.clone(), 0, 0, vec![], vec![]);

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

        let final_chain = new_final_chain(storage.clone(), 0, 0, vec![], vec![]);

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

        let final_chain = new_final_chain(
            storage.clone(),
            block_gas_limit,
            genesis_timestamp,
            vec![],
            vec![],
        );

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
    fn genesis_dpos_vote_counts_are_derived_from_validator_stake() {
        let path = temp_db_path("genesis-dpos-votes");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let first_validator = [0x10; 20];
        let second_validator = [0x20; 20];
        let ineligible_validator = [0x30; 20];

        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![
                genesis_validator(first_validator, U256::from(10_000u64)),
                genesis_validator(second_validator, U256::from(25_000u64)),
                genesis_validator(ineligible_validator, U256::from(999u64)),
            ],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(first_validator)
                .unwrap(),
            10
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(second_validator)
                .unwrap(),
            25
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(ineligible_validator)
                .unwrap(),
            0
        );
        assert_eq!(final_chain.dpos_eligible_total_vote_count().unwrap(), 35);
        assert!(final_chain.dpos_is_eligible(first_validator).unwrap());
        assert!(!final_chain.dpos_is_eligible(ineligible_validator).unwrap());
        assert!(!final_chain.dpos_is_eligible([0xFF; 20]).unwrap());

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn genesis_dpos_vote_count_rejects_u64_overflow() {
        let path = temp_db_path("genesis-dpos-overflow");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());

        let err = match FinalChain::new(
            storage.clone(),
            0,
            0,
            vec![],
            vec![genesis_validator(
                [0x40; 20],
                U256::from(u64::MAX) + U256::one(),
            )],
            GenesisDposConfig {
                eligibility_balance_threshold: vec![],
                vote_eligibility_balance_step: u256_to_big_endian(U256::one()),
                validator_maximum_stake: u256_to_big_endian(U256::MAX),
            },
        ) {
            Ok(_) => panic!("expected genesis DPoS vote count overflow"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("does not fit into u64"));

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn genesis_dpos_vote_count_rejects_stake_above_validator_maximum() {
        let path = temp_db_path("genesis-dpos-max-stake");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());

        let err = match FinalChain::new(
            storage.clone(),
            0,
            0,
            vec![],
            vec![genesis_validator([0x50; 20], U256::from(10_001u64))],
            GenesisDposConfig {
                eligibility_balance_threshold: vec![],
                vote_eligibility_balance_step: u256_to_big_endian(U256::one()),
                validator_maximum_stake: u256_to_big_endian(U256::from(10_000u64)),
            },
        ) {
            Ok(_) => panic!("expected genesis DPoS maximum stake rejection"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("exceeds maximum stake"));

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_applies_native_transfer_and_persists_indexes() {
        let path = temp_db_path("finalize-native-transfer");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let timestamp = 77u64;
        let block_gas_limit = 100_000u64;
        let sender = [0x11; 20];
        let receiver = [0x22; 20];
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let beneficiary = address_from_signing_key(&signing_key);
        let beneficiary_bytes: [u8; 20] = beneficiary.into();
        let pbft_block = signed_pbft_block(&signing_key, period, timestamp);
        let transaction_rlp = vec![0xc1, 0x80];
        let transaction = test_transaction(
            0xA1,
            sender,
            Some(receiver),
            0,
            U256::from(13u64),
            U256::from(2u64),
            50_000,
            vec![],
            transaction_rlp.clone(),
        );
        write_period_data(
            &storage,
            period,
            &pbft_block,
            std::slice::from_ref(&transaction_rlp),
        );
        let final_chain = new_final_chain(
            storage.clone(),
            block_gas_limit,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![],
        );
        let genesis_hash = H256::from_slice(&final_chain.block_hash(0).unwrap().unwrap());

        let (header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![transaction.clone()])
            .unwrap();

        assert_eq!(receipts.len(), 1);
        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, VALUE_TRANSFER_GAS, VALUE_TRANSFER_GAS)
        );
        assert_eq!(
            final_chain.transaction_receipt_rlp(period, 0).unwrap(),
            Some(receipts[0].clone())
        );
        assert_eq!(
            final_chain.transaction_receipt_rlp(period, 1).unwrap(),
            None
        );
        assert_eq!(
            final_chain.transaction_rlps(period).unwrap(),
            vec![transaction_rlp.clone()]
        );
        let header = Rlp::new(&header_rlp);
        assert_eq!(header.val_at::<H256>(1).unwrap(), genesis_hash);
        assert_eq!(header.val_at::<H160>(2).unwrap(), beneficiary);
        assert_eq!(
            header.val_at::<H256>(4).unwrap(),
            ordered_root(std::iter::once(transaction_rlp.as_slice()))
        );
        assert_eq!(
            header.val_at::<H256>(5).unwrap(),
            ordered_root(std::iter::once(receipts[0].as_slice()))
        );
        assert_eq!(header.val_at::<u64>(7).unwrap(), period);
        assert_eq!(header.val_at::<u64>(8).unwrap(), block_gas_limit);
        assert_eq!(header.val_at::<u64>(9).unwrap(), VALUE_TRANSFER_GAS);
        assert_eq!(header.val_at::<u64>(10).unwrap(), timestamp);
        assert_eq!(final_chain.last_block_number().unwrap(), period);
        assert_eq!(
            final_chain.block_number(transaction.hash).unwrap(),
            None,
            "transaction hash must not be indexed as a block hash"
        );
        let block_hash = header.val_at::<H256>(0).unwrap();
        assert_eq!(
            final_chain.block_number(block_hash.into()).unwrap(),
            Some(period)
        );
        let location = final_chain
            .transaction_location(transaction.hash)
            .unwrap()
            .unwrap();
        let location = Rlp::new(&location);
        assert_eq!(location.val_at::<u64>(0).unwrap(), period);
        assert_eq!(location.val_at::<u32>(1).unwrap(), 0);
        assert_eq!(
            balance_of(&final_chain, sender),
            U256::from(1_000_000u64) - U256::from(13u64) - U256::from(VALUE_TRANSFER_GAS * 2)
        );
        assert_eq!(final_chain.account(sender).unwrap().unwrap().nonce, 1);
        assert_eq!(balance_of(&final_chain, receiver), U256::from(13u64));
        assert_eq!(
            balance_of(&final_chain, beneficiary_bytes),
            U256::from(VALUE_TRANSFER_GAS * 2)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_failed_transfer_charges_affordable_gas_without_nonce_or_receiver_change() {
        let path = temp_db_path("finalize-failed-transfer");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x33; 20];
        let receiver = [0x44; 20];
        let signing_key = SigningKey::from_slice(&[8u8; 32]).unwrap();
        let beneficiary: [u8; 20] = address_from_signing_key(&signing_key).into();
        let pbft_block = signed_pbft_block(&signing_key, period, 88);
        let transaction_rlp = vec![0xc1, 0x81];
        let transaction = test_transaction(
            0xB2,
            sender,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::from(10u64),
            30_000,
            vec![],
            transaction_rlp.clone(),
        );
        write_period_data(
            &storage,
            period,
            &pbft_block,
            std::slice::from_ref(&transaction_rlp),
        );
        let final_chain = new_final_chain(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(100_001u64))],
            vec![],
        );

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![transaction])
            .unwrap();

        assert_eq!(receipt_fields(&receipts[0]), (0, 10_000, 10_000));
        assert_eq!(final_chain.account(sender).unwrap().unwrap().nonce, 0);
        assert_eq!(balance_of(&final_chain, sender), U256::from(1u64));
        assert!(final_chain.account(receiver).unwrap().is_none());
        assert_eq!(
            balance_of(&final_chain, beneficiary),
            U256::from(100_000u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_low_nonce_consumes_full_gas_limit() {
        let path = temp_db_path("finalize-low-nonce");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x55; 20];
        let receiver = [0x66; 20];
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let beneficiary: [u8; 20] = address_from_signing_key(&signing_key).into();
        let pbft_block = signed_pbft_block(&signing_key, period, 99);
        let transaction_rlp = vec![0xc1, 0x82];
        let transaction = test_transaction(
            0xC3,
            sender,
            Some(receiver),
            2,
            U256::from(1u64),
            U256::from(3u64),
            30_000,
            vec![],
            transaction_rlp.clone(),
        );
        write_period_data(
            &storage,
            period,
            &pbft_block,
            std::slice::from_ref(&transaction_rlp),
        );
        let final_chain = new_final_chain(
            storage.clone(),
            100_000,
            0,
            vec![GenesisAccount {
                address: sender,
                balance: u256_to_big_endian(U256::from(200_000u64)),
            }],
            vec![],
        );
        final_chain
            .accounts
            .lock()
            .unwrap()
            .get_mut(&sender)
            .unwrap()
            .nonce = 3;

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![transaction])
            .unwrap();

        assert_eq!(receipt_fields(&receipts[0]), (0, 30_000, 30_000));
        assert_eq!(final_chain.account(sender).unwrap().unwrap().nonce, 3);
        assert_eq!(balance_of(&final_chain, sender), U256::from(110_000u64));
        assert!(final_chain.account(receiver).unwrap().is_none());
        assert_eq!(balance_of(&final_chain, beneficiary), U256::from(90_000u64));

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_rejects_transaction_count_mismatch_without_execution() {
        let path = temp_db_path("finalize-count-mismatch");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x77; 20];
        let signing_key = SigningKey::from_slice(&[10u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 101);
        write_period_data(&storage, period, &pbft_block, &[]);
        let final_chain = new_final_chain(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(100_000u64))],
            vec![],
        );

        let err = final_chain
            .finalize_block(
                pbft_block,
                vec![test_transaction(
                    0xD4,
                    sender,
                    Some([0x88; 20]),
                    0,
                    U256::from(1u64),
                    U256::from(1u64),
                    30_000,
                    vec![],
                    vec![0xc1, 0x83],
                )],
            )
            .unwrap_err();

        assert!(err.to_string().contains("transaction count mismatch"));
        assert_eq!(final_chain.last_block_number().unwrap(), 0);
        assert_eq!(balance_of(&final_chain, sender), U256::from(100_000u64));

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_rejects_non_native_transfer_without_persisting_block() {
        let path = temp_db_path("finalize-non-native");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x99; 20];
        let signing_key = SigningKey::from_slice(&[11u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 111);
        let transaction_rlp = vec![0xc1, 0x84];
        write_period_data(
            &storage,
            period,
            &pbft_block,
            std::slice::from_ref(&transaction_rlp),
        );
        let final_chain = new_final_chain(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(100_000u64))],
            vec![],
        );

        let err = final_chain
            .finalize_block(
                pbft_block,
                vec![test_transaction(
                    0xE5,
                    sender,
                    None,
                    0,
                    U256::zero(),
                    U256::from(1u64),
                    30_000,
                    vec![0x01],
                    transaction_rlp,
                )],
            )
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("currently supports only native value transfers")
        );
        assert_eq!(final_chain.last_block_number().unwrap(), 0);
        assert_eq!(final_chain.transaction_location([0xE5; 32]).unwrap(), None);

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
