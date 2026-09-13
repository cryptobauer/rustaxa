//! Narrow REVM host backed by the authoritative execution journal.
//!
//! The host projects arbitrary-width Taraxa values only where the EVM operand
//! stack requires a 256-bit word. It preserves journal storage, transient,
//! refund and log ownership and fails explicitly when immutable state or code
//! cannot be loaded. This first host slice intentionally has no self-destruct,
//! native-dispatch or nested-frame implementation.

use std::borrow::Cow;

use num_bigint::BigUint;
use revm::{
    bytecode::Bytecode,
    context_interface::{
        Host,
        cfg::GasParams,
        context::{SStoreResult, SelfDestructResult, StateLoad},
        host::LoadError,
        journaled_state::AccountInfoLoad,
    },
    primitives::{Address, B256, KECCAK_EMPTY, Log, StorageKey, StorageValue, U256},
    state::AccountInfo,
};
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteStateRead, ConcreteStorageKey},
};

use crate::{
    contracts::{
        BlockHashRead, BlockHashReadError, ExecutionBlockContext, ExecutionLog,
        ExecutionTransaction,
    },
    journal::{ExecutionJournal, JournalError},
};

/// A host-side failure that must abort the pending transaction/period.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostError {
    /// Concrete state or journal mutation failed.
    Journal(JournalError),
    /// Canonical historical block data was unavailable or invalid.
    BlockHash(BlockHashReadError),
    /// SSTORE gas comparison cannot preserve an ordinary value wider than one EVM word.
    StorageWordWidth {
        /// Account containing the unsupported value.
        address: [u8; 20],
        /// Logical slot containing the unsupported value.
        key: ConcreteStorageKey,
    },
    /// SELFDESTRUCT lifecycle parity is not implemented in this bounded host.
    SelfDestructUnavailable,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "execution host: {self:?}")
    }
}

impl std::error::Error for HostError {}

/// REVM host for one top-level bytecode call.
pub struct JournalHost<'a, R, B> {
    journal: &'a mut ExecutionJournal<R>,
    block_hashes: &'a B,
    block: &'a ExecutionBlockContext,
    transaction: &'a ExecutionTransaction,
    gas_params: GasParams,
    fault: Option<HostError>,
}

impl<'a, R: ConcreteStateRead, B: BlockHashRead> JournalHost<'a, R, B> {
    /// Binds one interpreter to its transaction, block and journal authority.
    pub fn new(
        journal: &'a mut ExecutionJournal<R>,
        block_hashes: &'a B,
        block: &'a ExecutionBlockContext,
        transaction: &'a ExecutionTransaction,
        gas_params: GasParams,
    ) -> Self {
        Self {
            journal,
            block_hashes,
            block,
            transaction,
            gas_params,
            fault: None,
        }
    }

    /// Takes the first host failure recorded by an instruction.
    pub fn take_error(&mut self) -> Option<HostError> {
        self.fault.take()
    }

    fn fail<T>(&mut self, error: HostError) -> Result<T, LoadError> {
        if self.fault.is_none() {
            self.fault = Some(error);
        }
        Err(LoadError::DBError)
    }

    fn account_info(
        &mut self,
        address: [u8; 20],
        load_code: bool,
    ) -> Result<(AccountInfo, bool), LoadError> {
        let metadata = match self.journal.account_metadata(address) {
            Ok(metadata) => metadata,
            Err(error) => return self.fail(HostError::Journal(error)),
        };
        let balance = U256::from_be_bytes(metadata.balance.low_word());
        let semantic_empty = metadata.nonce.is_zero()
            && metadata.balance.value() == &num_bigint::BigInt::default()
            && metadata.code_size == 0;
        let mut nonce = metadata.nonce.as_u64().unwrap_or(1);
        let code_hash = if !metadata.exists {
            B256::ZERO
        } else if metadata.code_size == 0 {
            KECCAK_EMPTY
        } else {
            match metadata.code_hash {
                Some(hash) => B256::from(hash),
                None => {
                    return self.fail(HostError::Journal(JournalError::MissingCodeHash {
                        address,
                    }));
                }
            }
        };
        let code = if load_code {
            let bytes = match self.journal.account_code(address) {
                Ok(bytes) => bytes,
                Err(error) => return self.fail(HostError::Journal(error)),
            };
            Some(Bytecode::new_raw(bytes.into()))
        } else {
            None
        };
        // REVM uses AccountInfo::is_empty for EXTCODEHASH. Preserve the
        // reference's full-width EIP-161 test when a wide nonce/balance projects
        // to zero in the bounded host representation.
        if metadata.exists
            && !semantic_empty
            && nonce == 0
            && balance.is_zero()
            && code_hash == KECCAK_EMPTY
        {
            nonce = 1;
        }
        Ok((
            AccountInfo {
                balance,
                nonce,
                code_hash,
                account_id: None,
                code,
            },
            !metadata.exists,
        ))
    }
}

impl<R: ConcreteStateRead, B: BlockHashRead> Host for JournalHost<'_, R, B> {
    fn basefee(&self) -> U256 {
        U256::ZERO
    }
    fn blob_gasprice(&self) -> U256 {
        U256::ZERO
    }
    fn gas_limit(&self) -> U256 {
        U256::from(self.block.gas_limit.as_u64())
    }
    fn gas_params(&self) -> &GasParams {
        &self.gas_params
    }
    fn is_amsterdam_eip8037_enabled(&self) -> bool {
        false
    }
    fn difficulty(&self) -> U256 {
        low_word(self.block.difficulty.clone())
    }
    fn prevrandao(&self) -> Option<U256> {
        None
    }
    fn block_number(&self) -> U256 {
        U256::from(self.block.period.as_u64())
    }
    fn timestamp(&self) -> U256 {
        U256::from(self.block.timestamp)
    }
    fn beneficiary(&self) -> Address {
        Address::from(self.block.author)
    }
    fn slot_num(&self) -> U256 {
        U256::ZERO
    }
    fn chain_id(&self) -> U256 {
        U256::from(self.block.chain_id)
    }
    fn effective_gas_price(&self) -> U256 {
        U256::from_be_bytes(self.transaction.gas_price.low_word())
    }
    fn caller(&self) -> Address {
        Address::from(self.transaction.sender)
    }
    fn blob_hash(&self, _number: usize) -> Option<U256> {
        None
    }
    fn max_initcode_size(&self) -> usize {
        49_152
    }

    fn block_hash(&mut self, number: u64) -> Option<B256> {
        let current = self.block.period.as_u64();
        if number >= current || current - number > 256 {
            return Some(B256::ZERO);
        }
        match self
            .block_hashes
            .block_hash(FinalChainBlockNumber::new(number))
        {
            Ok(hash) => Some(B256::from(hash)),
            Err(error) => {
                if self.fault.is_none() {
                    self.fault = Some(HostError::BlockHash(error));
                }
                None
            }
        }
    }

    fn selfdestruct(
        &mut self,
        _address: Address,
        _target: Address,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SelfDestructResult>, LoadError> {
        self.fail(HostError::SelfDestructUnavailable)
    }

    fn log(&mut self, log: Log) {
        self.journal.push_log(ExecutionLog {
            address: log.address.into_array(),
            topics: log.data.topics().iter().map(|topic| topic.0).collect(),
            data: log.data.data.to_vec(),
        });
    }

    fn sstore_skip_cold_load(
        &mut self,
        address: Address,
        key: StorageKey,
        value: StorageValue,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        let address = address.into_array();
        let key = ConcreteStorageKey(key.to_be_bytes());
        let (original, present) = match self.journal.ordinary_storage(address, key) {
            Ok(values) => values,
            Err(error) => return self.fail(HostError::Journal(error)),
        };
        if original.bits() > 256 || present.bits() > 256 {
            return self.fail(HostError::StorageWordWidth { address, key });
        }
        if let Err(error) = self
            .journal
            .set_ordinary_storage(address, key, big_uint(value))
        {
            return self.fail(HostError::Journal(error));
        }
        Ok(StateLoad::new(
            SStoreResult {
                original_value: low_word(original),
                present_value: low_word(present),
                new_value: value,
            },
            false,
        ))
    }

    fn sload_skip_cold_load(
        &mut self,
        address: Address,
        key: StorageKey,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<StorageValue>, LoadError> {
        let values = self
            .journal
            .ordinary_storage(address.into_array(), ConcreteStorageKey(key.to_be_bytes()));
        match values {
            Ok((_, current)) => Ok(StateLoad::new(low_word(current), false)),
            Err(error) => self.fail(HostError::Journal(error)),
        }
    }

    fn tstore(&mut self, address: Address, key: StorageKey, value: StorageValue) {
        self.journal.set_transient_storage(
            address.into_array(),
            ConcreteStorageKey(key.to_be_bytes()),
            value.to_be_bytes(),
        );
    }

    fn tload(&mut self, address: Address, key: StorageKey) -> StorageValue {
        U256::from_be_bytes(
            self.journal
                .transient_storage(address.into_array(), ConcreteStorageKey(key.to_be_bytes())),
        )
    }

    fn load_account_info_skip_cold_load(
        &mut self,
        address: Address,
        load_code: bool,
        _skip_cold_load: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        let (account, absent) = self.account_info(address.into_array(), load_code)?;
        Ok(AccountInfoLoad {
            account: Cow::Owned(account),
            is_cold: false,
            is_empty: absent,
        })
    }

    fn load_account_code_hash(&mut self, address: Address) -> Option<StateLoad<B256>> {
        let (account, absent) = self.account_info(address.into_array(), false).ok()?;
        Some(StateLoad::new(
            if absent {
                B256::ZERO
            } else {
                account.code_hash
            },
            false,
        ))
    }
}

fn big_uint(value: U256) -> BigUint {
    BigUint::from_bytes_be(&value.to_be_bytes::<32>())
}

fn low_word(value: BigUint) -> U256 {
    let bytes = value.to_bytes_be();
    let tail = bytes.len().saturating_sub(32);
    let mut word = [0_u8; 32];
    word[32 - (bytes.len() - tail)..].copy_from_slice(&bytes[tail..]);
    U256::from_be_bytes(word)
}
