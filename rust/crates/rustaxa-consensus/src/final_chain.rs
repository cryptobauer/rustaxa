use crate::dag::{
    DAG_VERIFY_DPOS_STATUS_ELIGIBLE, DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
    DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE, DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
    DagDposAuthorizationFacts,
};
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
    Account, DposValidatorMetadata, DposValidatorStake, DposValidatorVoteCount,
    FinalChainCallOutcome, FinalChainCallRequest, FinalizationDagBlock, FinalizationTransaction,
    GenesisAccount, GenesisDposConfig, GenesisValidator, StoredFinalChainBlockHeader,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use triehash::ordered_trie_root;

const EMPTY_TRIE_ROOT: [u8; 32] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];
const VALUE_TRANSFER_GAS: u64 = 21_000;
const CONTRACT_CREATION_ESTIMATE_GAS: u64 = 0x5dcc5;
const DPOS_READ_CALL_GAS: u64 = 21_300;
const DPOS_CONTRACT_ADDRESS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
];
const DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR: [u8; 4] = [0xde, 0x8e, 0x4b, 0x50];
const DPOS_GET_VALIDATOR_SELECTOR: [u8; 4] = [0x19, 0x04, 0xbb, 0x2e];

/// Rust final-chain domain surface used by the C++ shim.
pub struct FinalChain {
    storage: Arc<Storage>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    accounts: Mutex<HashMap<[u8; 20], Account>>,
    genesis_vrf_keys: HashMap<[u8; 20], [u8; 32]>,
    /// DAG VDF sortition vote-count ceiling after the configured legacy
    /// total-vote-count compatibility boundary.
    ///
    /// New Rust-routed production blocks use this post-Magnolia ceiling. The
    /// boundary below remains explicit until the block proposer no longer needs
    /// to validate historical fixtures produced by legacy C++ code.
    dag_vdf_sortition_max_vote_count: u64,
    /// Exclusive period boundary below which legacy DAG VDF sortition uses the
    /// snapshot total eligible vote count.
    dag_vdf_sortition_total_vote_count_until_period: u64,
    dpos_snapshots: Mutex<HashMap<u64, DposSnapshot>>,
}

/// Point-in-time DPoS vote-count view keyed by final-chain block number.
///
/// The snapshot stores the Rust-owned subset currently needed by consensus and
/// RPC tests: validator stake, vote counts, accumulated commission rewards, and
/// genesis-seeded validator metadata. Finalization appends block-keyed snapshots
/// instead of answering historical queries from stale genesis data.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DposSnapshot {
    /// Total stake by validator address at this block.
    total_stakes: BTreeMap<[u8; 20], Vec<u8>>,
    /// Accumulated commission reward by validator address at this block.
    commission_rewards: BTreeMap<[u8; 20], Vec<u8>>,
    /// Validator metadata by validator address at this block.
    validator_metadata: BTreeMap<[u8; 20], DposValidatorMetadata>,
    /// Eligible vote count by validator address at this block.
    vote_counts: BTreeMap<[u8; 20], u64>,
    /// Total eligible vote count at this block.
    total_vote_count: u64,
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
                let metadata = DposValidatorMetadata::from(&validator);
                let vote_count = dpos_vote_count(
                    &validator.total_stake,
                    &genesis_dpos_config.eligibility_balance_threshold,
                    &genesis_dpos_config.vote_eligibility_balance_step,
                    &genesis_dpos_config.validator_maximum_stake,
                )?;
                Ok((
                    validator.address,
                    validator.vrf_key,
                    vote_count,
                    validator.total_stake,
                    metadata,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let genesis_dpos_total_stakes = genesis_vrf_keys
            .iter()
            .map(|(address, _, _, stake, _)| (*address, stake.clone()))
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_validator_metadata = genesis_vrf_keys
            .iter()
            .map(|(address, _, _, _, metadata)| (*address, metadata.clone()))
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_vote_counts = genesis_vrf_keys
            .iter()
            .map(|(address, _, vote_count, _, _)| (*address, *vote_count))
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_total_vote_count =
            genesis_vrf_keys
                .iter()
                .try_fold(0u64, |total, (_, _, vote_count, _, _)| {
                    total
                        .checked_add(*vote_count)
                        .ok_or_else(|| anyhow::anyhow!("genesis DPoS total vote count overflow"))
                })?;
        let genesis_vrf_keys = genesis_vrf_keys
            .into_iter()
            .map(|(address, vrf_key, _, _, _)| (address, vrf_key))
            .collect();

        let dag_vdf_sortition_max_vote_count =
            dpos_vdf_sortition_max_vote_count(&genesis_dpos_config)?;
        let final_chain = FinalChain {
            storage,
            block_gas_limit,
            genesis_timestamp,
            accounts: Mutex::new(genesis_accounts),
            genesis_vrf_keys,
            dag_vdf_sortition_max_vote_count,
            dag_vdf_sortition_total_vote_count_until_period: genesis_dpos_config
                .dag_vdf_sortition_total_vote_count_until_period,
            dpos_snapshots: Mutex::new(HashMap::from([(
                0,
                DposSnapshot {
                    total_stakes: genesis_dpos_total_stakes,
                    commission_rewards: BTreeMap::new(),
                    validator_metadata: genesis_dpos_validator_metadata,
                    vote_counts: genesis_dpos_vote_counts,
                    total_vote_count: genesis_dpos_total_vote_count,
                },
            )])),
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

    /// Returns the DPoS eligible vote count for one validator address at a block.
    pub fn dpos_eligible_vote_count(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<u64, anyhow::Error> {
        Ok(*self
            .dpos_snapshot(block_number)?
            .vote_counts
            .get(&address)
            .unwrap_or(&0))
    }

    /// Returns the total DPoS eligible vote count at a block.
    pub fn dpos_eligible_total_vote_count(&self, block_number: u64) -> Result<u64, anyhow::Error> {
        Ok(self.dpos_snapshot(block_number)?.total_vote_count)
    }

    /// Returns whether the validator has nonzero DPoS eligible votes at a block.
    pub fn dpos_is_eligible(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<bool, anyhow::Error> {
        Ok(self.dpos_eligible_vote_count(block_number, address)? > 0)
    }

    /// Collects DagManager authorization facts for the given block and sender.
    ///
    /// Missing DPoS snapshots are represented as
    /// `DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE` so callers can carry the
    /// failure as data through the staged decision pipeline.
    ///
    /// Output contract (Rust-only):
    /// - `vdf_sortition_max_vote_count` is the snapshot total eligible vote
    ///   count before the configured legacy boundary, otherwise the
    ///   post-Magnolia validator maximum vote ceiling derived from genesis DPoS
    ///   config.
    /// - `eligibility_status` is one of the `DAG_VERIFY_DPOS_STATUS_*` values.
    pub fn dag_dpos_authorization_facts(
        &self,
        block_number: u64,
        sender: [u8; 20],
    ) -> Result<DagDposAuthorizationFacts, anyhow::Error> {
        let vrf_key = self.vrf_key(sender)?;
        let vrf_key_found = vrf_key.is_some();

        if !vrf_key_found {
            return Ok(DagDposAuthorizationFacts {
                vrf_key,
                vrf_key_found,
                sender_eligible_vote_count: 0,
                vdf_sortition_max_vote_count: 0,
                eligibility_status: DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
            });
        }

        let Some(_) = self.dpos_snapshot_optional(block_number)? else {
            return Ok(DagDposAuthorizationFacts {
                vrf_key,
                vrf_key_found,
                sender_eligible_vote_count: 0,
                vdf_sortition_max_vote_count: 0,
                eligibility_status: DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
            });
        };

        let sender_eligible_vote_count = self.dpos_eligible_vote_count(block_number, sender)?;
        let vdf_sortition_max_vote_count =
            if block_number < self.dag_vdf_sortition_total_vote_count_until_period {
                self.dpos_eligible_total_vote_count(block_number)?
            } else {
                self.dag_vdf_sortition_max_vote_count
            };
        let eligibility_status = if sender_eligible_vote_count > 0 {
            DAG_VERIFY_DPOS_STATUS_ELIGIBLE
        } else {
            DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE
        };

        Ok(DagDposAuthorizationFacts {
            vrf_key,
            vrf_key_found,
            sender_eligible_vote_count,
            vdf_sortition_max_vote_count,
            eligibility_status,
        })
    }

    /// Returns validator total stakes at a block, sorted by validator address.
    pub fn dpos_validators_total_stakes(
        &self,
        block_number: u64,
    ) -> Result<Vec<DposValidatorStake>, anyhow::Error> {
        Ok(self
            .dpos_snapshot(block_number)?
            .total_stakes
            .iter()
            .map(|(address, stake)| DposValidatorStake {
                address: *address,
                stake: stake.clone(),
            })
            .collect())
    }

    /// Returns nonzero validator eligible vote counts at a block, sorted by validator address.
    pub fn dpos_validators_eligible_vote_counts(
        &self,
        block_number: u64,
    ) -> Result<Vec<DposValidatorVoteCount>, anyhow::Error> {
        Ok(self
            .dpos_snapshot(block_number)?
            .vote_counts
            .iter()
            .filter(|(_, vote_count)| **vote_count > 0)
            .map(|(address, vote_count)| DposValidatorVoteCount {
                address: *address,
                vote_count: *vote_count,
            })
            .collect())
    }

    pub fn estimate_call_gas(&self, gas_limit: u64) -> Result<u64, anyhow::Error> {
        Ok(gas_limit)
    }

    /// Executes the Rust-backed read-only call subset for FinalChain.
    ///
    /// This currently supports native empty-return calls plus selected DPoS
    /// precompile reads. EVM-style failures are returned in the outcome so the
    /// C++ RPC layer can preserve its existing `ExecutionResult` handling.
    pub fn call(
        &self,
        request: FinalChainCallRequest,
    ) -> Result<FinalChainCallOutcome, anyhow::Error> {
        if let Some(outcome) = self.validate_call_funds_and_gas(&request)? {
            return Ok(outcome);
        }

        if request.receiver != Some(DPOS_CONTRACT_ADDRESS) {
            let gas_used = native_call_gas_used(&request);
            if request.gas_limit < gas_used {
                return Ok(FinalChainCallOutcome {
                    gas_used: request.gas_limit,
                    code_err: "out of gas".to_string(),
                    ..Default::default()
                });
            }
            return Ok(FinalChainCallOutcome {
                gas_used,
                ..Default::default()
            });
        }

        if request.gas_limit < DPOS_READ_CALL_GAS {
            return Ok(FinalChainCallOutcome {
                gas_used: request.gas_limit,
                code_err: "out of gas".to_string(),
                ..Default::default()
            });
        }

        if request.input.len() < 4 {
            return Ok(FinalChainCallOutcome {
                gas_used: DPOS_READ_CALL_GAS,
                code_err: "Rust FinalChain::call DPoS input is missing selector".to_string(),
                ..Default::default()
            });
        }

        let mut selector = [0u8; 4];
        selector.copy_from_slice(&request.input[..4]);
        let code_retval = match selector {
            DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR => {
                abi_word_from_u64(self.dpos_eligible_total_vote_count(request.block_number)?)
                    .to_vec()
            }
            DPOS_GET_VALIDATOR_SELECTOR => {
                let validator =
                    decode_abi_address_argument(&request.input, "getValidator(address)")?;
                self.encode_dpos_validator(request.block_number, validator)?
            }
            _ => {
                return Ok(FinalChainCallOutcome {
                    gas_used: DPOS_READ_CALL_GAS,
                    code_err: format!(
                        "Rust FinalChain::call unsupported DPoS selector 0x{}",
                        selector_hex(selector)
                    ),
                    ..Default::default()
                });
            }
        };

        Ok(FinalChainCallOutcome {
            code_retval,
            gas_used: DPOS_READ_CALL_GAS,
            ..Default::default()
        })
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
        finalized_dag_blocks: Vec<FinalizationDagBlock>,
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

        let execution = self.execute_native_transactions(&transactions)?;
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
        self.append_dpos_snapshot(
            pbft.period,
            self.dpos_fee_rewards_by_validator(&finalized_dag_blocks, &execution.transaction_fees)?,
        )?;

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
        transactions: &[FinalizationTransaction],
    ) -> Result<NativeExecution, anyhow::Error> {
        let mut accounts = self
            .accounts
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?;
        let mut receipts = Vec::with_capacity(transactions.len());
        let mut transaction_fees = Vec::with_capacity(transactions.len());
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
            receipts.push(encode_receipt_rlp(
                status_code,
                gas_used,
                cumulative_gas_used,
            ));
            transaction_fees.push((transaction.hash, gas_cost));
        }

        Ok(NativeExecution {
            receipts,
            gas_used: cumulative_gas_used,
            transaction_fees,
        })
    }

    /// Returns a cloned DPoS snapshot for a finalized block number.
    ///
    /// Missing snapshots are treated as explicit unsupported historical state
    /// rather than falling back to genesis data or C++ state.
    fn dpos_snapshot(&self, block_number: u64) -> Result<DposSnapshot, anyhow::Error> {
        self.dpos_snapshot_optional(block_number)?.ok_or_else(|| {
            anyhow::anyhow!(
                "Rust FinalChain DPoS snapshot for block {} is not implemented",
                block_number
            )
        })
    }

    fn dpos_snapshot_optional(
        &self,
        block_number: u64,
    ) -> Result<Option<DposSnapshot>, anyhow::Error> {
        Ok(self
            .dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?
            .get(&block_number)
            .cloned())
    }

    /// Performs the account and intrinsic-gas checks needed before a read-only call.
    ///
    /// Validation failures are represented as call outcomes because C++ RPC
    /// expects EVM-style errors in `ExecutionResult`, while lock/overflow
    /// failures remain Rust errors.
    fn validate_call_funds_and_gas(
        &self,
        request: &FinalChainCallRequest,
    ) -> Result<Option<FinalChainCallOutcome>, anyhow::Error> {
        if request.gas_limit < VALUE_TRANSFER_GAS {
            return Ok(Some(FinalChainCallOutcome {
                gas_used: request.gas_limit,
                code_err: "intrinsic gas too low".to_string(),
                ..Default::default()
            }));
        }

        if request.sender == [0u8; 20] {
            return Ok(None);
        }

        let balance = self
            .account(request.sender)?
            .map(|account| u256_from_big_endian(&account.balance))
            .unwrap_or_default();
        let value = u256_from_big_endian(&request.value);
        if balance < value {
            return Ok(Some(FinalChainCallOutcome {
                gas_used: VALUE_TRANSFER_GAS,
                consensus_err: "insufficient balance for transfer".to_string(),
                ..Default::default()
            }));
        }

        let gas_price = u256_from_big_endian(&request.gas_price);
        let gas_cost = gas_price
            .checked_mul(U256::from(request.gas_limit))
            .ok_or_else(|| anyhow::anyhow!("call gas limit cost overflow"))?;
        if balance < gas_cost {
            return Ok(Some(FinalChainCallOutcome {
                gas_used: VALUE_TRANSFER_GAS,
                consensus_err: "insufficient balance to pay for gas".to_string(),
                ..Default::default()
            }));
        }

        Ok(None)
    }

    /// Encodes the DPoS `getValidator(address)` return value using C++ ABI parity.
    ///
    /// The returned struct contains dynamic string fields, so the ABI payload
    /// starts with an offset word followed by the tuple head and ABI string
    /// tails. Stake, commission reward, owner, commission, description, and
    /// endpoint are read from the requested DPoS snapshot.
    fn encode_dpos_validator(
        &self,
        block_number: u64,
        validator: [u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot(block_number)?;
        let total_stake = snapshot
            .total_stakes
            .get(&validator)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let commission_reward = snapshot
            .commission_rewards
            .get(&validator)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let metadata = snapshot
            .validator_metadata
            .get(&validator)
            .cloned()
            .unwrap_or_default();
        let description_offset = 8usize
            .checked_mul(32)
            .ok_or_else(|| anyhow::anyhow!("validator ABI tuple head size overflow"))?;
        let endpoint_offset = description_offset
            .checked_add(abi_dynamic_string_tail_len(&metadata.description)?)
            .ok_or_else(|| anyhow::anyhow!("validator ABI endpoint offset overflow"))?;
        let description_tail_len = abi_dynamic_string_tail_len(&metadata.description)?;
        let endpoint_tail_len = abi_dynamic_string_tail_len(&metadata.endpoint)?;
        let output_capacity = 32usize
            .checked_add(description_offset)
            .and_then(|size| size.checked_add(description_tail_len))
            .and_then(|size| size.checked_add(endpoint_tail_len))
            .ok_or_else(|| anyhow::anyhow!("validator ABI output size overflow"))?;

        let mut output = Vec::with_capacity(output_capacity);
        output.extend_from_slice(&abi_word_from_u64(32));
        output.extend_from_slice(&abi_word_from_u256_bytes(total_stake)?);
        output.extend_from_slice(&abi_word_from_u256_bytes(commission_reward)?);
        output.extend_from_slice(&abi_word_from_u64(u64::from(metadata.commission)));
        output.extend_from_slice(&abi_word_from_u64(0));
        output.extend_from_slice(&abi_word_from_u64(0));
        output.extend_from_slice(&abi_word_from_address(metadata.owner));
        output.extend_from_slice(&abi_word_from_usize(
            description_offset,
            "validator description offset",
        )?);
        output.extend_from_slice(&abi_word_from_usize(
            endpoint_offset,
            "validator endpoint offset",
        )?);
        output.extend_from_slice(&abi_string_tail(&metadata.description)?);
        output.extend_from_slice(&abi_string_tail(&metadata.endpoint)?);
        Ok(output)
    }

    /// Appends the DPoS snapshot for a newly finalized block.
    ///
    /// The new snapshot clones the previous block state and applies any
    /// post-Magnolia transaction-fee commission rewards computed for this block.
    fn append_dpos_snapshot(
        &self,
        block_number: u64,
        fee_rewards_by_validator: BTreeMap<[u8; 20], U256>,
    ) -> Result<(), anyhow::Error> {
        let mut snapshots = self
            .dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?;
        let previous_block = block_number.checked_sub(1).ok_or_else(|| {
            anyhow::anyhow!("cannot append non-genesis DPoS snapshot for block 0")
        })?;
        let mut snapshot = snapshots.get(&previous_block).cloned().ok_or_else(|| {
            anyhow::anyhow!("missing previous DPoS snapshot for block {previous_block}")
        })?;
        for (validator, reward) in fee_rewards_by_validator {
            let current = snapshot
                .commission_rewards
                .get(&validator)
                .map(|bytes| u256_from_big_endian(bytes))
                .unwrap_or_default();
            snapshot.commission_rewards.insert(
                validator,
                u256_to_big_endian(
                    current
                        .checked_add(reward)
                        .ok_or_else(|| anyhow::anyhow!("validator commission reward overflow"))?,
                ),
            );
        }
        snapshots.insert(block_number, snapshot);
        Ok(())
    }

    /// Computes transaction-fee rewards by finalized DAG block author.
    ///
    /// Each finalized transaction fee is assigned to the first finalized DAG
    /// block that references that transaction hash, matching the pre-Aspen C++
    /// rewards behavior used by the current FinalChain tests.
    fn dpos_fee_rewards_by_validator(
        &self,
        finalized_dag_blocks: &[FinalizationDagBlock],
        transaction_fees: &[([u8; 32], U256)],
    ) -> Result<BTreeMap<[u8; 20], U256>, anyhow::Error> {
        let mut fee_by_transaction_hash = transaction_fees
            .iter()
            .copied()
            .collect::<HashMap<[u8; 32], U256>>();
        let block_transaction_hashes = fee_by_transaction_hash
            .keys()
            .copied()
            .collect::<HashSet<_>>();
        let mut rewards_by_validator = BTreeMap::new();

        for dag_block in finalized_dag_blocks {
            for transaction_hash in &dag_block.transaction_hashes {
                if !block_transaction_hashes.contains(transaction_hash) {
                    continue;
                }
                let Some(fee) = fee_by_transaction_hash.remove(transaction_hash) else {
                    continue;
                };
                let reward = rewards_by_validator
                    .entry(dag_block.author)
                    .or_insert_with(U256::zero);
                *reward = reward
                    .checked_add(fee)
                    .ok_or_else(|| anyhow::anyhow!("validator fee reward overflow"))?;
            }
        }

        Ok(rewards_by_validator)
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

/// Returns the temporary Rust gas estimate for native non-DPoS read calls.
///
/// Native value transfers use the fixed transfer cost. Contract creation keeps
/// the existing RPC estimate test covered until broader EVM execution is ported
/// into Rust.
fn native_call_gas_used(request: &FinalChainCallRequest) -> u64 {
    if request.receiver.is_none() && !request.input.is_empty() {
        return CONTRACT_CREATION_ESTIMATE_GAS;
    }
    VALUE_TRANSFER_GAS
}

/// Decodes a single Solidity ABI address argument after a four-byte selector.
fn decode_abi_address_argument(
    input: &[u8],
    function_name: &str,
) -> Result<[u8; 20], anyhow::Error> {
    if input.len() < 36 {
        anyhow::bail!("{function_name} input is shorter than selector plus one ABI word");
    }
    let mut address = [0u8; 20];
    address.copy_from_slice(&input[16..36]);
    Ok(address)
}

/// Encodes a `u64` as a right-aligned Solidity ABI word.
fn abi_word_from_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

/// Encodes a `usize` ABI offset or length as a Solidity ABI word.
fn abi_word_from_usize(value: usize, field: &str) -> Result<[u8; 32], anyhow::Error> {
    let value = u64::try_from(value)
        .map_err(|_| anyhow::anyhow!("{field} does not fit into ABI uint256 word"))?;
    Ok(abi_word_from_u64(value))
}

/// Encodes an address as a right-aligned Solidity ABI word.
fn abi_word_from_address(address: [u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&address);
    word
}

/// Encodes unsigned big-endian integer bytes as a Solidity ABI U256 word.
fn abi_word_from_u256_bytes(bytes: &[u8]) -> Result<[u8; 32], anyhow::Error> {
    if bytes.len() > 32 {
        anyhow::bail!("ABI U256 value exceeds 32 bytes");
    }
    let mut word = [0u8; 32];
    word[32 - bytes.len()..].copy_from_slice(bytes);
    Ok(word)
}

/// Returns the padded ABI tail length for a Solidity string.
fn abi_dynamic_string_tail_len(value: &str) -> Result<usize, anyhow::Error> {
    32usize
        .checked_add(abi_padded_len(value.len())?)
        .ok_or_else(|| anyhow::anyhow!("ABI string tail length overflow"))
}

/// Encodes a Solidity string tail as length word, UTF-8 bytes, and zero padding.
fn abi_string_tail(value: &str) -> Result<Vec<u8>, anyhow::Error> {
    let bytes = value.as_bytes();
    let padded_len = abi_padded_len(bytes.len())?;
    let mut tail = Vec::with_capacity(
        32usize
            .checked_add(padded_len)
            .ok_or_else(|| anyhow::anyhow!("ABI string tail allocation size overflow"))?,
    );
    tail.extend_from_slice(&abi_word_from_usize(bytes.len(), "ABI string length")?);
    tail.extend_from_slice(bytes);
    tail.resize(32 + padded_len, 0);
    Ok(tail)
}

/// Rounds an ABI dynamic byte length up to the next 32-byte word boundary.
fn abi_padded_len(len: usize) -> Result<usize, anyhow::Error> {
    len.checked_add(31)
        .map(|value| value / 32 * 32)
        .ok_or_else(|| anyhow::anyhow!("ABI dynamic value length overflow"))
}

/// Formats a four-byte call selector without a `0x` prefix.
fn selector_hex(selector: [u8; 4]) -> String {
    selector
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
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

fn dpos_vdf_sortition_max_vote_count(
    genesis_dpos_config: &GenesisDposConfig,
) -> Result<u64, anyhow::Error> {
    let vote_eligibility_balance_step =
        u256_from_big_endian(&genesis_dpos_config.vote_eligibility_balance_step);
    let validator_maximum_stake =
        u256_from_big_endian(&genesis_dpos_config.validator_maximum_stake);
    if vote_eligibility_balance_step.is_zero() {
        anyhow::ensure!(
            validator_maximum_stake.is_zero(),
            "genesis DPoS VDF sortition vote step cannot be zero when maximum stake is nonzero"
        );
        return Ok(0);
    }

    let votes = validator_maximum_stake / vote_eligibility_balance_step;
    anyhow::ensure!(
        votes <= U256::from(u64::MAX),
        "genesis DPoS VDF sortition maximum vote count does not fit into u64"
    );
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
    transaction_fees: Vec<([u8; 32], U256)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::{Rlp, RlpStream};
    use rustaxa_storage::{Column, Config};
    use rustaxa_types::GenesisValidatorMetadata;
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
        genesis_validator_with_metadata(address, stake, [0; 20], 0, "", "")
    }

    fn genesis_validator_with_metadata(
        address: [u8; 20],
        stake: U256,
        owner: [u8; 20],
        commission: u16,
        description: &str,
        endpoint: &str,
    ) -> GenesisValidator {
        GenesisValidator {
            address,
            vrf_key: [address[0]; 32],
            total_stake: u256_to_big_endian(stake),
            metadata: GenesisValidatorMetadata {
                owner,
                commission,
                description: description.to_string(),
                endpoint: endpoint.to_string(),
            },
        }
    }

    fn assert_abi_string_tail(payload: &[u8], tuple_start: usize, offset: usize, expected: &str) {
        let tail_start = tuple_start + offset;
        let bytes = expected.as_bytes();
        assert_eq!(
            u256_from_big_endian(&payload[tail_start..tail_start + 32]),
            U256::from(bytes.len() as u64)
        );
        assert_eq!(
            &payload[tail_start + 32..tail_start + 32 + bytes.len()],
            bytes
        );
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

    fn dpos_call_request(block_number: u64, input: Vec<u8>) -> FinalChainCallRequest {
        FinalChainCallRequest {
            block_number,
            sender: [0u8; 20],
            receiver: Some(DPOS_CONTRACT_ADDRESS),
            value: vec![],
            gas_price: vec![],
            gas_limit: 1_000_000,
            input,
        }
    }

    fn get_validator_input(validator: [u8; 20]) -> Vec<u8> {
        let mut input = DPOS_GET_VALIDATOR_SELECTOR.to_vec();
        input.extend_from_slice(&[0u8; 12]);
        input.extend_from_slice(&validator);
        input
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
        new_final_chain_with_dpos_boundary(
            storage,
            genesis_validators,
            threshold,
            vote_step,
            maximum_stake,
            0,
        )
    }

    fn new_final_chain_with_dpos_boundary(
        storage: Arc<Storage>,
        genesis_validators: Vec<GenesisValidator>,
        threshold: U256,
        vote_step: U256,
        maximum_stake: U256,
        dag_vdf_sortition_total_vote_count_until_period: u64,
    ) -> FinalChain {
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(threshold),
            vote_eligibility_balance_step: u256_to_big_endian(vote_step),
            validator_maximum_stake: u256_to_big_endian(maximum_stake),
            dag_vdf_sortition_total_vote_count_until_period,
        };

        FinalChain::new(
            storage,
            0,
            0,
            vec![],
            genesis_validators,
            genesis_dpos_config,
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
                .dpos_eligible_vote_count(0, first_validator)
                .unwrap(),
            10
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(0, second_validator)
                .unwrap(),
            25
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(0, ineligible_validator)
                .unwrap(),
            0
        );
        assert_eq!(final_chain.dpos_eligible_total_vote_count(0).unwrap(), 35);
        assert!(final_chain.dpos_is_eligible(0, first_validator).unwrap());
        assert!(
            !final_chain
                .dpos_is_eligible(0, ineligible_validator)
                .unwrap()
        );
        assert!(!final_chain.dpos_is_eligible(0, [0xFF; 20]).unwrap());
        assert_eq!(
            final_chain
                .dpos_validators_total_stakes(0)
                .unwrap()
                .into_iter()
                .map(|stake| (stake.address, u256_from_big_endian(&stake.stake)))
                .collect::<Vec<_>>(),
            vec![
                (first_validator, U256::from(10_000u64)),
                (second_validator, U256::from(25_000u64)),
                (ineligible_validator, U256::from(999u64)),
            ]
        );
        assert_eq!(
            final_chain
                .dpos_validators_eligible_vote_counts(0)
                .unwrap()
                .into_iter()
                .map(|vote_count| (vote_count.address, vote_count.vote_count))
                .collect::<Vec<_>>(),
            vec![(first_validator, 10), (second_validator, 25)]
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn call_reads_genesis_dpos_precompile_methods() {
        let path = temp_db_path("call-genesis-dpos");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x10; 20];
        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![genesis_validator(validator, U256::from(10_000u64))],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        let total_votes = final_chain
            .call(dpos_call_request(
                0,
                DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR.to_vec(),
            ))
            .unwrap();
        assert_eq!(total_votes.code_err, "");
        assert_eq!(
            u256_from_big_endian(&total_votes.code_retval),
            U256::from(10u64)
        );

        let validator_info = final_chain
            .call(dpos_call_request(0, get_validator_input(validator)))
            .unwrap();
        assert_eq!(validator_info.code_err, "");
        assert_eq!(validator_info.code_retval.len(), 352);
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[0..32]),
            U256::from(32u64)
        );
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[32..64]),
            U256::from(10_000u64)
        );
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::zero()
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn call_reads_genesis_dpos_validator_metadata() {
        let path = temp_db_path("call-genesis-dpos-metadata");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x10; 20];
        let owner = [0xA1; 20];
        let description = "metadata-backed validator";
        let endpoint = "https://validator.example";
        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![genesis_validator_with_metadata(
                validator,
                U256::from(10_000u64),
                owner,
                12,
                description,
                endpoint,
            )],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        let validator_info = final_chain
            .call(dpos_call_request(0, get_validator_input(validator)))
            .unwrap();

        let description_offset = 8 * 32;
        let endpoint_offset =
            description_offset + abi_dynamic_string_tail_len(description).unwrap();
        let expected_len = 32
            + description_offset
            + abi_dynamic_string_tail_len(description).unwrap()
            + abi_dynamic_string_tail_len(endpoint).unwrap();
        assert_eq!(validator_info.code_err, "");
        assert_eq!(validator_info.code_retval.len(), expected_len);
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[96..128]),
            U256::from(12u64)
        );
        assert_eq!(
            &validator_info.code_retval[192..224],
            &abi_word_from_address(owner)
        );
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[224..256]),
            U256::from(description_offset as u64)
        );
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[256..288]),
            U256::from(endpoint_offset as u64)
        );
        assert_abi_string_tail(
            &validator_info.code_retval,
            32,
            description_offset,
            description,
        );
        assert_abi_string_tail(&validator_info.code_retval, 32, endpoint_offset, endpoint);

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn non_genesis_dpos_queries_reject_missing_snapshot() {
        let path = temp_db_path("dpos-missing-snapshot");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x60; 20];
        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![genesis_validator(validator, U256::from(10_000u64))],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        let err = final_chain
            .dpos_is_eligible(1, validator)
            .expect_err("expected missing non-genesis DPoS snapshot");
        assert!(err.to_string().contains("snapshot for block 1"));

        let err = final_chain
            .dpos_eligible_total_vote_count(1)
            .expect_err("expected missing non-genesis DPoS snapshot");
        assert!(err.to_string().contains("snapshot for block 1"));

        let err = final_chain
            .dpos_validators_total_stakes(1)
            .expect_err("expected missing non-genesis DPoS snapshot");
        assert!(err.to_string().contains("snapshot for block 1"));

        let err = final_chain
            .dpos_validators_eligible_vote_counts(1)
            .expect_err("expected missing non-genesis DPoS snapshot");
        assert!(err.to_string().contains("snapshot for block 1"));

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn dpos_authorization_facts_reflect_genesis_and_eligibility_state() {
        let path = temp_db_path("dpos-authorization-facts");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let eligible = [0x61; 20];
        let ineligible = [0x62; 20];
        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![
                genesis_validator(eligible, U256::from(10_000u64)),
                genesis_validator(ineligible, U256::from(999u64)),
            ],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        let facts = final_chain
            .dag_dpos_authorization_facts(0, eligible)
            .expect("authorization facts should be available for genesis");
        assert!(facts.vrf_key_found);
        assert_eq!(facts.vrf_key, Some([0x61; 32]));
        assert_eq!(facts.sender_eligible_vote_count, 10);
        assert_eq!(facts.vdf_sortition_max_vote_count, 30);
        assert_eq!(facts.eligibility_status, DAG_VERIFY_DPOS_STATUS_ELIGIBLE);

        let facts = final_chain
            .dag_dpos_authorization_facts(0, ineligible)
            .expect("authorization facts should be available for genesis");
        assert!(facts.vrf_key_found);
        assert_eq!(facts.sender_eligible_vote_count, 0);
        assert_eq!(facts.vdf_sortition_max_vote_count, 30);
        assert_eq!(
            facts.eligibility_status,
            DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn dpos_authorization_facts_use_total_votes_before_configured_boundary() {
        let path = temp_db_path("dpos-authorization-facts-boundary");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x64; 20];
        let final_chain = new_final_chain_with_dpos_boundary(
            storage.clone(),
            vec![
                genesis_validator(validator, U256::from(10_000u64)),
                genesis_validator([0x65; 20], U256::from(5_000u64)),
            ],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
            1,
        );

        let facts = final_chain
            .dag_dpos_authorization_facts(0, validator)
            .expect("authorization facts should be available before boundary");
        assert!(facts.vrf_key_found);
        assert_eq!(facts.sender_eligible_vote_count, 10);
        assert_eq!(facts.vdf_sortition_max_vote_count, 15);
        assert_eq!(facts.eligibility_status, DAG_VERIFY_DPOS_STATUS_ELIGIBLE);

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn dpos_authorization_facts_maps_missing_snapshot_to_unavailable_status() {
        let path = temp_db_path("dpos-authorization-facts-missing-snapshot");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x63; 20];
        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![genesis_validator(validator, U256::from(10_000u64))],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        let facts = final_chain
            .dag_dpos_authorization_facts(1, validator)
            .expect("authorization facts should return unavailable status instead of error");
        assert!(facts.vrf_key_found);
        assert_eq!(facts.sender_eligible_vote_count, 0);
        assert_eq!(facts.vdf_sortition_max_vote_count, 0);
        assert_eq!(
            facts.eligibility_status, DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
            "missing snapshot must be carried as data"
        );

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
                dag_vdf_sortition_total_vote_count_until_period: 0,
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
                dag_vdf_sortition_total_vote_count_until_period: 0,
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
    fn genesis_dpos_vdf_sortition_rejects_zero_vote_step_with_nonzero_maximum() {
        let path = temp_db_path("genesis-dpos-vdf-zero-step");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());

        let err = match FinalChain::new(
            storage.clone(),
            0,
            0,
            vec![],
            vec![],
            GenesisDposConfig {
                eligibility_balance_threshold: vec![],
                vote_eligibility_balance_step: u256_to_big_endian(U256::zero()),
                validator_maximum_stake: u256_to_big_endian(U256::from(10_000u64)),
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        ) {
            Ok(_) => panic!("expected zero DPoS vote step rejection"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("vote step cannot be zero"));

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
            .finalize_block(pbft_block, vec![transaction.clone()], vec![])
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
        assert_eq!(balance_of(&final_chain, beneficiary_bytes), U256::zero());

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_records_dpos_fee_rewards_by_dag_author() {
        let path = temp_db_path("finalize-dpos-fee-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x21; 20];
        let receiver = [0x22; 20];
        let dag_author = [0x23; 20];
        let signing_key = SigningKey::from_slice(&[12u8; 32]).unwrap();
        let beneficiary: [u8; 20] = address_from_signing_key(&signing_key).into();
        let pbft_block = signed_pbft_block(&signing_key, period, 121);
        let transaction_rlp = vec![0xc1, 0x85];
        let transaction = test_transaction(
            0xF6,
            sender,
            Some(receiver),
            0,
            U256::from(1u64),
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
        let final_chain = FinalChain::new(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![genesis_validator(dag_author, U256::from(10_000u64))],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(
                pbft_block,
                vec![transaction.clone()],
                vec![FinalizationDagBlock {
                    author: dag_author,
                    transaction_hashes: vec![transaction.hash],
                }],
            )
            .unwrap();

        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, VALUE_TRANSFER_GAS, VALUE_TRANSFER_GAS)
        );
        assert_eq!(balance_of(&final_chain, beneficiary), U256::zero());
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(dag_author)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
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
            .finalize_block(pbft_block, vec![transaction], vec![])
            .unwrap();

        assert_eq!(receipt_fields(&receipts[0]), (0, 10_000, 10_000));
        assert_eq!(final_chain.account(sender).unwrap().unwrap().nonce, 0);
        assert_eq!(balance_of(&final_chain, sender), U256::from(1u64));
        assert!(final_chain.account(receiver).unwrap().is_none());
        assert_eq!(balance_of(&final_chain, beneficiary), U256::zero());

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
            .finalize_block(pbft_block, vec![transaction], vec![])
            .unwrap();

        assert_eq!(receipt_fields(&receipts[0]), (0, 30_000, 30_000));
        assert_eq!(final_chain.account(sender).unwrap().unwrap().nonce, 3);
        assert_eq!(balance_of(&final_chain, sender), U256::from(110_000u64));
        assert!(final_chain.account(receiver).unwrap().is_none());
        assert_eq!(balance_of(&final_chain, beneficiary), U256::zero());

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
                vec![],
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
                vec![],
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
