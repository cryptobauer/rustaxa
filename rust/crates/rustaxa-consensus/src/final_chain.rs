use crate::dag::{
    DAG_VERIFY_DPOS_STATUS_ELIGIBLE, DAG_VERIFY_DPOS_STATUS_NOT_CHECKED,
    DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE, DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
    DagDposAuthorizationFacts,
};
use crate::rewards_stats::{
    FinalizedRewardsPeriodFact, RewardCertVoteFact, RewardDagBlockFact, RewardTransactionFact,
    RewardsBlockDistribution, RewardsFrequencyRule, RewardsStatsConfig, RewardsStatsPeriodRlp,
    RewardsStatsRuntime, RewardsStatsStatus, decode_rewards_block_distributions,
};
use anyhow::Result;
use ethereum_types::{H256, U256};
use keccak_hasher::KeccakHasher;
use rlp::Rlp;
use rustaxa_storage::{
    FinalChainExecutionStatus, FinalChainRewardsStatsUpdate, StatusField, Storage,
};
use rustaxa_types::codec::rlp::final_chain::{
    LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
    StoredBlockHeaderRlpOwned,
};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::{
    Account, DposValidatorMetadata, DposValidatorStake, DposValidatorVoteCount,
    FinalChainCallOutcome, FinalChainCallRequest, FinalChainRewardsConfig, FinalizationDagBlock,
    FinalizationTransaction, GenesisAccount, GenesisDposConfig, GenesisValidator,
    StoredFinalChainBlockHeader,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::Mutex;
use triehash::ordered_trie_root;

type DposDelegations = BTreeMap<[u8; 20], BTreeMap<[u8; 20], Vec<u8>>>;

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
const DPOS_DELEGATE_SELECTOR: [u8; 4] = [0x5c, 0x19, 0xa9, 0x5c];
const DPOS_UNDELEGATE_SELECTOR: [u8; 4] = [0x4d, 0x99, 0xdd, 0x16];
const DPOS_REDELEGATE_SELECTOR: [u8; 4] = [0x70, 0x38, 0x12, 0xcc];
const DPOS_REGISTER_VALIDATOR_SELECTOR: [u8; 4] = [0xd6, 0xfd, 0xc1, 0x27];
const DPOS_REGISTER_VALIDATOR_GAS: u64 = 80_000;
const DPOS_DELEGATE_GAS: u64 = 40_000;
const DPOS_UNDELEGATE_GAS: u64 = 60_000;
const DPOS_REDELEGATE_GAS: u64 = 80_000;
const ASPEN_YIELD_PRECISION: u64 = 1_000_000;

/// Rust final-chain domain surface used by the C++ shim.
pub struct FinalChain {
    storage: Arc<Storage>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    accounts: Mutex<HashMap<[u8; 20], Account>>,
    genesis_vrf_keys: HashMap<[u8; 20], [u8; 32]>,
    dpos_eligibility_balance_threshold: Vec<u8>,
    dpos_vote_eligibility_balance_step: Vec<u8>,
    dpos_validator_maximum_stake: Vec<u8>,
    dpos_minimum_deposit: Vec<u8>,
    dpos_delegation_delay: u64,
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
    /// Hardfork and interval rules used by Rust rewards-stat planning during
    /// native finalization.
    rewards_config: FinalChainRewardsConfig,
    /// Mutable reward-stat interval cache used by native finalization.
    ///
    /// The runtime is loaded from persisted `BlockRewardsStats` rows on startup
    /// and replaced only after a finalized block and its rewards-cache mutation
    /// have committed successfully.
    rewards_stats_runtime: Mutex<RewardsStatsRuntime>,
    /// Account snapshots keyed by finalized block number for proposal-period
    /// account reads. Missing accounts remain absent from each snapshot.
    account_snapshots: Mutex<HashMap<u64, HashMap<[u8; 20], Account>>>,
    /// Highest finalized block whose account snapshot has been loaded into the
    /// latest account map. Latest account reads fail when this lags
    /// LAST_NUMBER, preventing restart paths from silently using genesis state.
    latest_account_snapshot_block: Mutex<u64>,
    dpos_snapshots: Mutex<HashMap<u64, DposSnapshot>>,
}

/// Point-in-time DPoS vote-count view keyed by final-chain block number.
///
/// The snapshot stores the Rust-owned subset currently needed by consensus and
/// RPC tests: validator stake, vote counts, accumulated validator/delegator
/// rewards, validator metadata, and VRF keys. Finalization appends block-keyed
/// snapshots instead of answering historical queries from stale genesis data.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DposSnapshot {
    /// Total stake by validator address at this block.
    total_stakes: BTreeMap<[u8; 20], Vec<u8>>,
    /// Accumulated commission reward by validator address at this block.
    commission_rewards: BTreeMap<[u8; 20], Vec<u8>>,
    /// Accumulated delegator reward pool by validator address at this block.
    delegator_rewards: BTreeMap<[u8; 20], Vec<u8>>,
    /// Validator metadata by validator address at this block.
    validator_metadata: BTreeMap<[u8; 20], DposValidatorMetadata>,
    /// Validator VRF keys by validator address at this block.
    vrf_keys: BTreeMap<[u8; 20], [u8; 32]>,
    /// Eligible vote count by validator address at this block.
    vote_counts: BTreeMap<[u8; 20], u64>,
    /// Total eligible vote count at this block.
    total_vote_count: u64,
    /// Delegated stake by validator and delegator at this block.
    delegations: DposDelegations,
    /// Aspen part-one minted-token counter at this block.
    minted_tokens: Vec<u8>,
    /// Aspen part-two total supply at this block.
    ///
    /// Empty bytes mean the Go-compatible lazy migration has not happened yet.
    total_supply: Vec<u8>,
    /// Aspen part-two yield fraction scaled by `ASPEN_YIELD_PRECISION`.
    current_yield: u64,
}

struct NativeRewardsStatsPlan {
    fee_rewards_by_validator: BTreeMap<[u8; 20], U256>,
    distribution_stats: Vec<RewardsBlockDistribution>,
    storage_update: Option<OwnedRewardsStatsUpdate>,
    runtime_after_commit: RewardsStatsRuntime,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DposRewardDeltas {
    commission_rewards: BTreeMap<[u8; 20], U256>,
    delegator_rewards: BTreeMap<[u8; 20], U256>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MintedRewardPlan {
    dpos_rewards: DposRewardDeltas,
    total_minted_reward: U256,
    total_supply_after: Option<U256>,
    current_yield: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BlockRewardContext {
    block_reward: U256,
    total_supply_before: Option<U256>,
    current_yield: u64,
}

struct OwnedRewardsStatsUpdate {
    current_period: u64,
    cache_current_period: bool,
    clear_cached_stats: bool,
    current_block_stats_rlp: Vec<u8>,
}

impl OwnedRewardsStatsUpdate {
    fn as_storage_update(&self) -> FinalChainRewardsStatsUpdate<'_> {
        FinalChainRewardsStatsUpdate {
            current_period: self.current_period,
            cache_current_period: self.cache_current_period,
            clear_cached_stats: self.clear_cached_stats,
            current_block_stats_rlp: &self.current_block_stats_rlp,
        }
    }
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
        Self::new_with_rewards_config(
            storage,
            block_gas_limit,
            genesis_timestamp,
            genesis_accounts,
            genesis_validators,
            genesis_dpos_config,
            FinalChainRewardsConfig::default(),
        )
    }

    pub fn new_with_rewards_config(
        storage: Arc<Storage>,
        block_gas_limit: u64,
        genesis_timestamp: u64,
        genesis_accounts: Vec<GenesisAccount>,
        genesis_validators: Vec<GenesisValidator>,
        genesis_dpos_config: GenesisDposConfig,
        rewards_config: FinalChainRewardsConfig,
    ) -> Result<Self> {
        let genesis_account_balance_sum = genesis_accounts
            .iter()
            .try_fold(U256::zero(), |total, account| {
                total.checked_add(u256_from_big_endian(&account.balance))
            });
        let mut rewards_config = rewards_config;
        if rewards_config.genesis_balance_sum.is_empty() {
            if let Some(genesis_account_balance_sum) = genesis_account_balance_sum {
                rewards_config.genesis_balance_sum =
                    u256_to_big_endian(genesis_account_balance_sum);
            } else {
                anyhow::ensure!(
                    rewards_config.aspen_part_two_period == 0,
                    "genesis account balance sum overflow"
                );
            }
        }
        let genesis_accounts: HashMap<[u8; 20], Account> = genesis_accounts
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
                    validator.delegations,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let genesis_dpos_total_stakes = genesis_vrf_keys
            .iter()
            .map(|(address, _, _, stake, _, _)| (*address, stake.clone()))
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_validator_metadata = genesis_vrf_keys
            .iter()
            .map(|(address, _, _, _, metadata, _)| (*address, metadata.clone()))
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_vote_counts = genesis_vrf_keys
            .iter()
            .map(|(address, _, vote_count, _, _, _)| (*address, *vote_count))
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_delegations = genesis_vrf_keys
            .iter()
            .map(|(address, _, _, stake, _, delegations)| {
                let mut validator_delegations = BTreeMap::new();
                if delegations.is_empty() {
                    validator_delegations.insert(*address, stake.clone());
                } else {
                    for (delegator, amount) in delegations {
                        validator_delegations.insert(*delegator, amount.clone());
                    }
                }
                (*address, validator_delegations)
            })
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_total_vote_count =
            genesis_vrf_keys
                .iter()
                .try_fold(0u64, |total, (_, _, vote_count, _, _, _)| {
                    total
                        .checked_add(*vote_count)
                        .ok_or_else(|| anyhow::anyhow!("genesis DPoS total vote count overflow"))
                })?;
        let genesis_vrf_keys: HashMap<[u8; 20], [u8; 32]> = genesis_vrf_keys
            .into_iter()
            .map(|(address, vrf_key, _, _, _, _)| (address, vrf_key))
            .collect();

        let dag_vdf_sortition_max_vote_count =
            dpos_vdf_sortition_max_vote_count(&genesis_dpos_config)?;
        let rewards_stats_runtime = rewards_stats_runtime_from_storage(&storage, &rewards_config)?;
        let final_chain = FinalChain {
            storage,
            block_gas_limit,
            genesis_timestamp,
            accounts: Mutex::new(genesis_accounts.clone()),
            genesis_vrf_keys: genesis_vrf_keys.clone(),
            dpos_eligibility_balance_threshold: genesis_dpos_config
                .eligibility_balance_threshold
                .clone(),
            dpos_vote_eligibility_balance_step: genesis_dpos_config
                .vote_eligibility_balance_step
                .clone(),
            dpos_validator_maximum_stake: genesis_dpos_config.validator_maximum_stake.clone(),
            dpos_minimum_deposit: genesis_dpos_config.minimum_deposit.clone(),
            dpos_delegation_delay: genesis_dpos_config.delegation_delay,
            dag_vdf_sortition_max_vote_count,
            dag_vdf_sortition_total_vote_count_until_period: genesis_dpos_config
                .dag_vdf_sortition_total_vote_count_until_period,
            rewards_config,
            rewards_stats_runtime: Mutex::new(rewards_stats_runtime),
            account_snapshots: Mutex::new(HashMap::from([(0, genesis_accounts.clone())])),
            latest_account_snapshot_block: Mutex::new(0),
            dpos_snapshots: Mutex::new(HashMap::from([(
                0,
                DposSnapshot {
                    total_stakes: genesis_dpos_total_stakes,
                    commission_rewards: BTreeMap::new(),
                    delegator_rewards: BTreeMap::new(),
                    validator_metadata: genesis_dpos_validator_metadata,
                    vrf_keys: genesis_vrf_keys
                        .iter()
                        .map(|(address, vrf_key)| (*address, *vrf_key))
                        .collect(),
                    vote_counts: genesis_dpos_vote_counts,
                    total_vote_count: genesis_dpos_total_vote_count,
                    delegations: genesis_dpos_delegations,
                    minted_tokens: Vec::new(),
                    total_supply: Vec::new(),
                    current_yield: 0,
                },
            )])),
        };
        final_chain.ensure_genesis_header()?;
        final_chain.load_persisted_account_snapshots()?;
        final_chain.load_persisted_dpos_snapshots()?;
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
        let last_block = self.last_block_number()?;
        let latest_snapshot_block = *self
            .latest_account_snapshot_block
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain latest account snapshot lock poisoned"))?;
        if latest_snapshot_block != last_block {
            anyhow::bail!("final-chain account snapshot unavailable for latest block {last_block}");
        }
        Ok(self
            .accounts
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?
            .get(&address)
            .cloned())
    }

    /// Returns an account exactly as it existed at a finalized block number.
    ///
    /// `Ok(None)` means the address had no account in that block snapshot.
    /// Missing snapshots are errors so callers never silently substitute latest
    /// state for historical proposal-period decisions.
    pub fn account_at_block(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<Option<Account>, anyhow::Error> {
        Ok(self
            .account_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account snapshot lock poisoned"))?
            .get(&block_number)
            .ok_or_else(|| {
                anyhow::anyhow!("final-chain account snapshot unavailable for block {block_number}")
            })?
            .get(&address)
            .cloned())
    }

    pub fn vrf_key(&self, address: [u8; 20]) -> Result<Option<[u8; 32]>, anyhow::Error> {
        let latest_block = self.last_block_number()?;
        self.vrf_key_at_block(latest_block, address)
    }

    /// Returns the validator VRF key from the DPoS snapshot at a block.
    ///
    /// Inputs are a finalized block number and validator address. The output is
    /// the snapshot VRF key when the validator exists at that block, or `None`
    /// when the snapshot exists without that validator. Missing or corrupt
    /// snapshots remain hard errors so callers do not silently use stale keys.
    pub fn vrf_key_at_block(
        &self,
        block_number: u64,
        address: [u8; 20],
    ) -> Result<Option<[u8; 32]>, anyhow::Error> {
        if let Some(vrf_key) = self.dpos_snapshot(block_number)?.vrf_keys.get(&address) {
            return Ok(Some(*vrf_key));
        }
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
        let Some(snapshot) = self.dpos_snapshot_optional(block_number)? else {
            let vrf_key = self.genesis_vrf_keys.get(&sender).copied();
            let vrf_key_found = vrf_key.is_some();
            return Ok(DagDposAuthorizationFacts {
                vrf_key,
                vrf_key_found,
                sender_eligible_vote_count: 0,
                vdf_sortition_max_vote_count: 0,
                eligibility_status: DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
            });
        };
        let vrf_key = snapshot
            .vrf_keys
            .get(&sender)
            .copied()
            .or_else(|| self.genesis_vrf_keys.get(&sender).copied());
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

        let sender_eligible_vote_count = *snapshot.vote_counts.get(&sender).unwrap_or(&0);
        let vdf_sortition_max_vote_count =
            if block_number < self.dag_vdf_sortition_total_vote_count_until_period {
                snapshot.total_vote_count
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

    /// Returns the block-scoped total amount delegated in DPoS.
    ///
    /// The value is derived from the Rust DPoS snapshot total-stake map and is
    /// encoded as unsigned big-endian bytes for C++ `u256` conversion.
    pub fn dpos_total_amount_delegated(&self, block_number: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(u256_to_big_endian(
            self.dpos_total_amount_delegated_u256(block_number)?,
        ))
    }

    /// Returns the block-scoped Aspen part-two yield value.
    ///
    /// Before Aspen part two this matches the legacy state API and returns
    /// zero. After part two, the value is the persisted yield fraction scaled
    /// by `ASPEN_YIELD_PRECISION`.
    pub fn dpos_yield(&self, block_number: u64) -> Result<u64, anyhow::Error> {
        if !self.aspen_part_two_active(block_number) {
            return Ok(0);
        }
        Ok(self
            .dpos_snapshot_at_finalized_block(block_number)?
            .current_yield)
    }

    /// Returns the block-scoped Aspen part-two total supply.
    ///
    /// Before Aspen part two this matches the legacy state API and returns
    /// zero. After part two, missing supply is surfaced as an explicit Rust
    /// state error instead of silently falling back to legacy C++.
    pub fn dpos_total_supply(&self, block_number: u64) -> Result<Vec<u8>, anyhow::Error> {
        if !self.aspen_part_two_active(block_number) {
            return Ok(Vec::new());
        }
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        anyhow::ensure!(
            !snapshot.total_supply.is_empty(),
            "Rust FinalChain Aspen total supply is missing for block {block_number}"
        );
        Ok(snapshot.total_supply)
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
            DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR => abi_word_from_u64(
                self.dpos_snapshot_at_finalized_block(request.block_number)?
                    .total_vote_count,
            )
            .to_vec(),
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
        self.finalize_block_with_rewards_context(
            pbft_block_rlp,
            transactions,
            finalized_dag_blocks,
            0,
        )
    }

    /// Finalizes a PBFT block with caller-supplied reward-rate context.
    pub fn finalize_block_with_rewards_context(
        &self,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<FinalizationTransaction>,
        finalized_dag_blocks: Vec<FinalizationDagBlock>,
        blocks_per_year: u32,
    ) -> Result<(Vec<u8>, Vec<Vec<u8>>), anyhow::Error> {
        self.finalize_block_with_rewards_facts(
            pbft_block_rlp,
            transactions,
            finalized_dag_blocks,
            blocks_per_year,
            Vec::new(),
        )
    }

    /// Finalizes a PBFT block with caller-supplied reward-rate and cert-vote
    /// facts.
    pub fn finalize_block_with_rewards_facts(
        &self,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<FinalizationTransaction>,
        finalized_dag_blocks: Vec<FinalizationDagBlock>,
        blocks_per_year: u32,
        cert_votes: Vec<RewardCertVoteFact>,
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
        let pre_magnolia_fee_reward_period = self.pre_magnolia_fee_reward_period(pbft.period);
        let rewards_stats_plan = self.native_rewards_stats_plan(
            &pbft,
            &transactions,
            &finalized_dag_blocks,
            &execution.transaction_fees,
            blocks_per_year,
            cert_votes,
        )?;
        let dpos_fee_rewards = if pre_magnolia_fee_reward_period {
            BTreeMap::new()
        } else {
            rewards_stats_plan.fee_rewards_by_validator.clone()
        };
        let mut account_snapshot = execution.accounts;
        let mut dpos_snapshot =
            self.plan_dpos_snapshot(pbft.period, execution.dpos_transactions)?;
        let minted_reward_plan = self.plan_minted_rewards(
            pbft.period,
            &rewards_stats_plan.distribution_stats,
            &dpos_snapshot,
        )?;
        let mut dpos_reward_deltas = minted_reward_plan.dpos_rewards;
        if pre_magnolia_fee_reward_period {
            self.credit_pre_magnolia_pbft_fee_reward(
                &mut account_snapshot,
                pbft.author.into(),
                total_transaction_fees(&execution.transaction_fees)?,
            )?;
        } else {
            merge_reward_map(
                &mut dpos_reward_deltas.commission_rewards,
                &dpos_fee_rewards,
            )?;
            self.credit_post_magnolia_dpos_fee_rewards(&mut account_snapshot, &dpos_fee_rewards)?;
        }
        self.credit_dpos_contract_minted_rewards(
            &mut account_snapshot,
            minted_reward_plan.total_minted_reward,
        )?;
        self.apply_dpos_reward_deltas(
            &mut dpos_snapshot,
            dpos_reward_deltas,
            pbft.period,
            minted_reward_plan.total_minted_reward,
            minted_reward_plan.total_supply_after,
            minted_reward_plan.current_yield,
        )?;
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
            total_reward: minted_reward_plan.total_minted_reward,
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
        let dpos_snapshot_rlp = encode_dpos_snapshot_rlp(&dpos_snapshot);
        let account_snapshot_rlp = encode_account_snapshot_rlp(&account_snapshot);
        let execution_status = self.finalization_execution_status(
            finalized_dag_blocks.len() as u64,
            transactions.len() as u64,
        )?;
        let rewards_stats_storage_update = rewards_stats_plan
            .storage_update
            .as_ref()
            .map(OwnedRewardsStatsUpdate::as_storage_update);
        self.storage
            .final_chain()
            .write_block_header_with_snapshots_execution_status_and_rewards_stats(
                pbft.period,
                full_header.hash()?,
                stored_header_rlp.as_bytes(),
                receipts_rlp.as_slice(),
                Some(&dpos_snapshot_rlp),
                Some(&account_snapshot_rlp),
                Some(execution_status),
                rewards_stats_storage_update,
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
        self.insert_account_snapshot(pbft.period, account_snapshot)?;
        self.insert_dpos_snapshot(pbft.period, dpos_snapshot)?;
        self.commit_rewards_stats_runtime(rewards_stats_plan.runtime_after_commit)?;

        Ok((full_header.into_vec(), execution.receipts))
    }

    /// Reports whether transaction fees still belong to the PBFT beneficiary.
    ///
    /// The configured boundary is the Magnolia block number passed through the
    /// bridge. It is exclusive, matching legacy C++ reward planning:
    /// `period < magnolia_hf.block_num` credits gas fees to the PBFT block
    /// beneficiary, while Magnolia and later periods route fees through DPoS
    /// commission rewards. A zero boundary means Magnolia behavior is active
    /// from genesis.
    fn pre_magnolia_fee_reward_period(&self, block_number: u64) -> bool {
        self.rewards_config.magnolia_period != 0
            && block_number < self.rewards_config.magnolia_period
    }

    /// Credits legacy pre-Magnolia gas fees to the PBFT block beneficiary.
    ///
    /// Inputs are the beneficiary address decoded from the signed PBFT block and
    /// the total gas fee already deducted by transaction execution. The method
    /// creates an empty beneficiary account when needed, preserves nonce/code
    /// fields, and reports overflow without partially mutating account state.
    fn credit_pre_magnolia_pbft_fee_reward(
        &self,
        accounts: &mut HashMap<[u8; 20], Account>,
        beneficiary: [u8; 20],
        reward: U256,
    ) -> Result<(), anyhow::Error> {
        if reward.is_zero() {
            return Ok(());
        }
        let account = accounts.entry(beneficiary).or_insert_with(empty_account);
        let balance = u256_from_big_endian(&account.balance);
        account.balance = u256_to_big_endian(
            balance
                .checked_add(reward)
                .ok_or_else(|| anyhow::anyhow!("pre-Magnolia PBFT beneficiary reward overflow"))?,
        );
        Ok(())
    }

    /// Credits post-Magnolia fee rewards to the Rust DPoS contract account.
    ///
    /// Native rewards-stat planning separately records per-validator commission
    /// ownership in the DPoS snapshot. The executable account mutation mirrors
    /// the legacy DPoS precompile behavior for transaction fees by accumulating
    /// the distributed fee total on the DPoS contract balance before the
    /// account snapshot is persisted with final-chain visibility.
    fn credit_post_magnolia_dpos_fee_rewards(
        &self,
        accounts: &mut HashMap<[u8; 20], Account>,
        fee_rewards_by_validator: &BTreeMap<[u8; 20], U256>,
    ) -> Result<(), anyhow::Error> {
        let reward =
            fee_rewards_by_validator
                .values()
                .try_fold(U256::zero(), |total, reward| {
                    total.checked_add(*reward).ok_or_else(|| {
                        anyhow::anyhow!("post-Magnolia DPoS fee reward total overflow")
                    })
                })?;
        if reward.is_zero() {
            return Ok(());
        }
        let account = accounts
            .entry(DPOS_CONTRACT_ADDRESS)
            .or_insert_with(empty_account);
        let balance = u256_from_big_endian(&account.balance);
        account.balance = u256_to_big_endian(
            balance
                .checked_add(reward)
                .ok_or_else(|| anyhow::anyhow!("DPoS contract fee reward balance overflow"))?,
        );
        Ok(())
    }

    /// Credits fixed-yield minted rewards to the Rust DPoS contract account.
    ///
    /// The DPoS snapshot records ownership of the minted reward through
    /// commission and delegator pools. The executable account mutation mirrors
    /// legacy DPoS precompile accounting by increasing the DPoS contract
    /// balance by the minted total. Transaction fees are credited separately and
    /// are not included in `total_minted_reward`.
    fn credit_dpos_contract_minted_rewards(
        &self,
        accounts: &mut HashMap<[u8; 20], Account>,
        total_minted_reward: U256,
    ) -> Result<(), anyhow::Error> {
        if total_minted_reward.is_zero() {
            return Ok(());
        }
        let account = accounts
            .entry(DPOS_CONTRACT_ADDRESS)
            .or_insert_with(empty_account);
        let balance = u256_from_big_endian(&account.balance);
        account.balance = u256_to_big_endian(
            balance
                .checked_add(total_minted_reward)
                .ok_or_else(|| anyhow::anyhow!("DPoS contract minted reward balance overflow"))?,
        );
        Ok(())
    }

    /// Plans minted DPoS rewards for decoded distribution stats.
    ///
    /// Before Aspen part two this uses the fixed annual yield formula. At and
    /// after Aspen part two, it performs the Go-compatible lazy supply
    /// migration, calculates the dynamic yield curve with integer arithmetic,
    /// and advances the transient supply after each decoded rewards period.
    fn plan_minted_rewards(
        &self,
        current_block_number: u64,
        distribution_stats: &[RewardsBlockDistribution],
        snapshot: &DposSnapshot,
    ) -> Result<MintedRewardPlan, anyhow::Error> {
        let mut plan = MintedRewardPlan::default();
        let mut dynamic_total_supply = if snapshot.total_supply.is_empty() {
            None
        } else {
            Some(u256_from_big_endian(&snapshot.total_supply))
        };

        for stats in distribution_stats {
            let reward_context = self.minted_block_reward(
                current_block_number,
                stats,
                snapshot,
                dynamic_total_supply,
            )?;
            let block_reward = reward_context.block_reward;
            if block_reward.is_zero() {
                if let Some(total_supply) = reward_context.total_supply_before {
                    dynamic_total_supply = Some(total_supply);
                    plan.total_supply_after = Some(total_supply);
                    plan.current_yield = reward_context.current_yield;
                }
                continue;
            }

            let mut dag_proposers_reward = block_reward;
            let mut votes_reward = U256::zero();
            let mut block_author_reward = U256::zero();
            if stats.total_votes_weight > 0 {
                dag_proposers_reward = percent_of(
                    block_reward,
                    self.rewards_config.dag_proposers_reward_percent,
                    "DAG proposer reward",
                )?;
                votes_reward = block_reward
                    .checked_sub(dag_proposers_reward)
                    .ok_or_else(|| anyhow::anyhow!("vote reward subtraction underflow"))?;
                let bonus_reward = percent_of(
                    block_reward,
                    self.rewards_config.max_block_author_reward_percent,
                    "block author reward",
                )?;
                votes_reward = votes_reward
                    .checked_sub(bonus_reward)
                    .ok_or_else(|| anyhow::anyhow!("block author reward exceeds vote reward"))?;
                let max_votes_weight = stats.max_votes_weight.max(stats.total_votes_weight);
                block_author_reward = if max_votes_weight == stats.total_votes_weight {
                    bonus_reward
                } else {
                    let two_t_plus_one = max_votes_weight
                        .checked_mul(2)
                        .and_then(|value| value.checked_div(3))
                        .and_then(|value| value.checked_add(1))
                        .ok_or_else(|| anyhow::anyhow!("reward max vote weight overflow"))?;
                    let denominator =
                        max_votes_weight
                            .checked_sub(two_t_plus_one)
                            .ok_or_else(|| {
                                anyhow::anyhow!("reward max vote weight denominator underflow")
                            })?;
                    if denominator == 0 {
                        U256::zero()
                    } else {
                        let bonus_votes_weight =
                            stats.total_votes_weight.saturating_sub(two_t_plus_one);
                        bonus_reward
                            .checked_mul(U256::from(bonus_votes_weight))
                            .ok_or_else(|| {
                                anyhow::anyhow!("block author reward multiplication overflow")
                            })?
                            / U256::from(denominator)
                    }
                };
            }

            let minted_before_stats = plan.total_minted_reward;
            if !block_author_reward.is_zero() {
                self.add_minted_validator_reward(
                    snapshot,
                    &mut plan,
                    stats.block_author.0,
                    block_author_reward,
                )?;
            }

            for (validator, validator_stats) in &stats.validators_stats {
                let mut validator_reward = U256::zero();
                if validator_stats.dag_blocks_count > 0 {
                    anyhow::ensure!(
                        stats.total_dag_blocks_count > 0,
                        "reward DAG block count is zero while validator DAG count is nonzero"
                    );
                    let dag_reward = dag_proposers_reward
                        .checked_mul(U256::from(validator_stats.dag_blocks_count))
                        .ok_or_else(|| anyhow::anyhow!("DAG reward multiplication overflow"))?
                        / U256::from(stats.total_dag_blocks_count);
                    validator_reward = validator_reward
                        .checked_add(dag_reward)
                        .ok_or_else(|| anyhow::anyhow!("validator DAG reward overflow"))?;
                }
                if validator_stats.vote_weight > 0 {
                    anyhow::ensure!(
                        stats.total_votes_weight > 0,
                        "reward total vote weight is zero while validator vote weight is nonzero"
                    );
                    let vote_reward = votes_reward
                        .checked_mul(U256::from(validator_stats.vote_weight))
                        .ok_or_else(|| anyhow::anyhow!("vote reward multiplication overflow"))?
                        / U256::from(stats.total_votes_weight);
                    validator_reward = validator_reward
                        .checked_add(vote_reward)
                        .ok_or_else(|| anyhow::anyhow!("validator vote reward overflow"))?;
                }
                if !validator_reward.is_zero() {
                    self.add_minted_validator_reward(
                        snapshot,
                        &mut plan,
                        *validator,
                        validator_reward,
                    )?;
                }
            }
            if let Some(total_supply_before) = reward_context.total_supply_before {
                let minted_for_stats = plan
                    .total_minted_reward
                    .checked_sub(minted_before_stats)
                    .ok_or_else(|| anyhow::anyhow!("stats minted reward subtraction underflow"))?;
                let total_supply_after = total_supply_before
                    .checked_add(minted_for_stats)
                    .ok_or_else(|| anyhow::anyhow!("Aspen total supply overflow"))?;
                dynamic_total_supply = Some(total_supply_after);
                plan.total_supply_after = Some(total_supply_after);
                plan.current_yield = reward_context.current_yield;
            }
        }

        Ok(plan)
    }

    fn minted_block_reward(
        &self,
        current_block_number: u64,
        stats: &RewardsBlockDistribution,
        snapshot: &DposSnapshot,
        dynamic_total_supply: Option<U256>,
    ) -> Result<BlockRewardContext, anyhow::Error> {
        if self.aspen_part_two_active(current_block_number) {
            let total_supply_before = match dynamic_total_supply {
                Some(total_supply) => total_supply,
                None => self.initial_aspen_part_two_supply(snapshot)?,
            };
            let current_yield = self.aspen_current_yield(total_supply_before)?;
            let block_reward = self.aspen_dynamic_block_reward(
                current_block_number,
                stats,
                snapshot,
                current_yield,
            )?;
            return Ok(BlockRewardContext {
                block_reward,
                total_supply_before: Some(total_supply_before),
                current_yield,
            });
        }

        Ok(BlockRewardContext {
            block_reward: self.fixed_yield_block_reward(snapshot)?,
            total_supply_before: None,
            current_yield: 0,
        })
    }

    fn fixed_yield_block_reward(&self, snapshot: &DposSnapshot) -> Result<U256, anyhow::Error> {
        let amount_delegated = total_staked_amount(snapshot)?;
        if amount_delegated.is_zero() || self.rewards_config.yield_percentage == 0 {
            return Ok(U256::zero());
        }
        anyhow::ensure!(
            self.rewards_config.dpos_blocks_per_year != 0,
            "fixed-yield reward distribution requires nonzero DPoS blocks per year"
        );
        let denominator = U256::from(100u64)
            .checked_mul(U256::from(self.rewards_config.dpos_blocks_per_year))
            .ok_or_else(|| anyhow::anyhow!("fixed-yield reward denominator overflow"))?;
        Ok(amount_delegated
            .checked_mul(U256::from(self.rewards_config.yield_percentage))
            .ok_or_else(|| anyhow::anyhow!("fixed-yield reward multiplication overflow"))?
            / denominator)
    }

    fn initial_aspen_part_two_supply(
        &self,
        snapshot: &DposSnapshot,
    ) -> Result<U256, anyhow::Error> {
        let genesis_balance_sum = u256_from_big_endian(&self.rewards_config.genesis_balance_sum);
        let generated_rewards = u256_from_big_endian(&self.rewards_config.aspen_generated_rewards);
        let minted_tokens = u256_from_big_endian(&snapshot.minted_tokens);
        genesis_balance_sum
            .checked_add(generated_rewards)
            .and_then(|total| total.checked_add(minted_tokens))
            .ok_or_else(|| anyhow::anyhow!("Aspen initial total supply overflow"))
    }

    fn aspen_current_yield(&self, total_supply: U256) -> Result<u64, anyhow::Error> {
        anyhow::ensure!(
            !total_supply.is_zero(),
            "Aspen dynamic yield requires nonzero total supply"
        );
        let max_supply = u256_from_big_endian(&self.rewards_config.aspen_max_supply);
        anyhow::ensure!(
            max_supply >= total_supply,
            "Aspen maximum supply is below current total supply"
        );
        let yield_value = max_supply
            .checked_sub(total_supply)
            .and_then(|remaining| remaining.checked_mul(U256::from(ASPEN_YIELD_PRECISION)))
            .ok_or_else(|| anyhow::anyhow!("Aspen dynamic yield multiplication overflow"))?
            / total_supply;
        anyhow::ensure!(
            yield_value <= U256::from(u64::MAX),
            "Aspen dynamic yield does not fit into u64"
        );
        Ok(yield_value.as_u64())
    }

    fn aspen_dynamic_block_reward(
        &self,
        current_block_number: u64,
        stats: &RewardsBlockDistribution,
        snapshot: &DposSnapshot,
        current_yield: u64,
    ) -> Result<U256, anyhow::Error> {
        let amount_delegated = total_staked_amount(snapshot)?;
        if amount_delegated.is_zero() || current_yield == 0 {
            return Ok(U256::zero());
        }
        let blocks_per_year = self.reward_blocks_per_year(current_block_number, stats)?;
        let denominator = U256::from(ASPEN_YIELD_PRECISION)
            .checked_mul(U256::from(blocks_per_year))
            .ok_or_else(|| anyhow::anyhow!("Aspen dynamic reward denominator overflow"))?;
        amount_delegated
            .checked_mul(U256::from(current_yield))
            .ok_or_else(|| anyhow::anyhow!("Aspen dynamic reward multiplication overflow"))
            .map(|reward| reward / denominator)
    }

    fn reward_blocks_per_year(
        &self,
        current_block_number: u64,
        stats: &RewardsBlockDistribution,
    ) -> Result<u32, anyhow::Error> {
        let blocks_per_year = if self.rewards_config.cacti_period != 0
            && current_block_number >= self.rewards_config.cacti_period
        {
            anyhow::ensure!(
                stats.blocks_per_year != 0,
                "Cacti reward distribution requires runtime blocks per year"
            );
            stats.blocks_per_year
        } else {
            self.rewards_config.dpos_blocks_per_year
        };
        anyhow::ensure!(
            blocks_per_year != 0,
            "reward distribution requires nonzero DPoS blocks per year"
        );
        Ok(blocks_per_year)
    }

    fn aspen_part_two_active(&self, block_number: u64) -> bool {
        self.rewards_config.aspen_part_two_period != 0
            && block_number >= self.rewards_config.aspen_part_two_period
    }

    fn add_minted_validator_reward(
        &self,
        snapshot: &DposSnapshot,
        plan: &mut MintedRewardPlan,
        validator: [u8; 20],
        reward: U256,
    ) -> Result<(), anyhow::Error> {
        let Some(metadata) = snapshot.validator_metadata.get(&validator) else {
            return Ok(());
        };
        let commission = percent_of_max_commission(reward, metadata.commission)?;
        let delegator_reward = reward
            .checked_sub(commission)
            .ok_or_else(|| anyhow::anyhow!("delegator reward subtraction underflow"))?;
        merge_reward_value(
            &mut plan.dpos_rewards.commission_rewards,
            validator,
            commission,
        )?;
        merge_reward_value(
            &mut plan.dpos_rewards.delegator_rewards,
            validator,
            delegator_reward,
        )?;
        plan.total_minted_reward = plan
            .total_minted_reward
            .checked_add(reward)
            .ok_or_else(|| anyhow::anyhow!("total minted reward overflow"))?;
        Ok(())
    }

    /// Applies validated DPoS reward deltas to a staged snapshot.
    ///
    /// The caller must pass a clone that has not been persisted or published.
    /// The method updates commission rewards, delegator reward pools, and the
    /// Aspen reward-supply counters using checked arithmetic.
    fn apply_dpos_reward_deltas(
        &self,
        snapshot: &mut DposSnapshot,
        rewards: DposRewardDeltas,
        block_number: u64,
        total_minted_reward: U256,
        total_supply_after: Option<U256>,
        current_yield: u64,
    ) -> Result<(), anyhow::Error> {
        apply_reward_map(
            &mut snapshot.commission_rewards,
            rewards.commission_rewards,
            "validator commission reward overflow",
        )?;
        apply_reward_map(
            &mut snapshot.delegator_rewards,
            rewards.delegator_rewards,
            "validator delegator reward overflow",
        )?;
        if self.aspen_part_two_active(block_number) {
            if let Some(total_supply_after) = total_supply_after {
                snapshot.total_supply = u256_to_big_endian(total_supply_after);
                snapshot.current_yield = current_yield;
                snapshot.minted_tokens.clear();
            }
            return Ok(());
        }
        if !total_minted_reward.is_zero()
            && block_number >= self.rewards_config.aspen_part_one_period
        {
            let current = u256_from_big_endian(&snapshot.minted_tokens);
            snapshot.minted_tokens = u256_to_big_endian(
                current
                    .checked_add(total_minted_reward)
                    .ok_or_else(|| anyhow::anyhow!("Aspen minted-token counter overflow"))?,
            );
        }
        snapshot.current_yield = 0;
        Ok(())
    }

    fn finalization_execution_status(
        &self,
        finalized_dag_block_delta: u64,
        executed_transaction_delta: u64,
    ) -> Result<FinalChainExecutionStatus, anyhow::Error> {
        let executed_dag_block_count = self
            .storage
            .metadata()
            .status_field(StatusField::ExecutedBlkCount as u8)?
            .checked_add(finalized_dag_block_delta)
            .ok_or_else(|| anyhow::anyhow!("executed DAG block count overflow"))?;
        let executed_transaction_count = self
            .storage
            .metadata()
            .status_field(StatusField::ExecutedTrxCount as u8)?
            .checked_add(executed_transaction_delta)
            .ok_or_else(|| anyhow::anyhow!("executed transaction count overflow"))?;
        Ok(FinalChainExecutionStatus {
            executed_dag_block_count,
            executed_transaction_count,
        })
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
        let mut accounts = self.current_account_snapshot()?;
        let mut receipts = Vec::with_capacity(transactions.len());
        let mut transaction_fees = Vec::with_capacity(transactions.len());
        let mut dpos_transactions = Vec::new();
        let mut cumulative_gas_used = 0u64;

        for transaction in transactions {
            let mut dpos_transaction = if transaction.receiver == Some(DPOS_CONTRACT_ADDRESS) {
                Some(decode_dpos_transaction(
                    &transaction.data,
                    transaction.sender,
                )?)
            } else if !transaction.data.is_empty() || transaction.receiver.is_none() {
                anyhow::bail!(
                    "Rust FinalChain::finalize currently supports only native value transfers and selected DPoS actions"
                );
            } else {
                None
            };
            let gas_price = u256_from_big_endian(&transaction.gas_price);
            let value = u256_from_big_endian(&transaction.value);
            if let Some(dpos_tx) = dpos_transaction.as_mut() {
                match dpos_tx {
                    DposTransaction::Register(registration) => {
                        registration.stake = u256_to_big_endian(value);
                    }
                    DposTransaction::Delegate { amount, .. } => {
                        *amount = u256_to_big_endian(value);
                    }
                    DposTransaction::Undelegate { .. } | DposTransaction::Redelegate { .. } => {}
                }
            }
            let required_gas = if let Some(dpos_transaction) = dpos_transaction.as_ref() {
                match dpos_transaction {
                    DposTransaction::Register(_) => DPOS_REGISTER_VALIDATOR_GAS,
                    DposTransaction::Delegate { .. } => DPOS_DELEGATE_GAS,
                    DposTransaction::Undelegate { .. } => DPOS_UNDELEGATE_GAS,
                    DposTransaction::Redelegate { .. } => DPOS_REDELEGATE_GAS,
                }
            } else {
                VALUE_TRANSFER_GAS
            };

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
                    gas_used = required_gas.min(transaction.gas_limit);
                    if transaction.gas_limit < required_gas {
                        status_code = 0;
                    }
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
                if let Some(dpos_tx) = dpos_transaction {
                    dpos_transactions.push(dpos_tx);
                } else {
                    let receiver_address = transaction.receiver.ok_or_else(|| {
                        anyhow::anyhow!("native value transfer missing receiver after validation")
                    })?;
                    if receiver_address == DPOS_CONTRACT_ADDRESS {
                        anyhow::bail!(
                            "Rust FinalChain::finalize unsupported DPoS transaction selector"
                        );
                    }
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
            }
            receipts.push(encode_receipt_rlp(
                status_code,
                gas_used,
                cumulative_gas_used,
            ));
            transaction_fees.push((transaction.hash, gas_cost));
        }

        Ok(NativeExecution {
            accounts,
            receipts,
            gas_used: cumulative_gas_used,
            transaction_fees,
            dpos_transactions,
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
        let snapshot_block_number = block_number.saturating_sub(self.dpos_delegation_delay);
        Ok(self
            .dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?
            .get(&snapshot_block_number)
            .cloned())
    }

    /// Returns the DPoS snapshot produced for the requested finalized block.
    ///
    /// Read-only DPoS precompile calls use the current finalized validator
    /// state. Delegation-delay snapshot selection is reserved for DAG
    /// authorization and explicit DPoS eligibility APIs.
    fn dpos_snapshot_at_finalized_block(
        &self,
        block_number: u64,
    ) -> Result<DposSnapshot, anyhow::Error> {
        self.dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?
            .get(&block_number)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Rust FinalChain DPoS snapshot for finalized block {} is not implemented",
                    block_number
                )
            })
    }

    fn dpos_total_amount_delegated_u256(&self, block_number: u64) -> Result<U256, anyhow::Error> {
        total_staked_amount(&self.dpos_snapshot_at_finalized_block(block_number)?)
    }

    /// Loads Rust-persisted historical DPoS snapshots for finalized blocks.
    ///
    /// Missing snapshot payloads are left absent so pre-existing databases do
    /// not silently answer historical DPoS queries from genesis state. Corrupt
    /// persisted snapshot payloads are hard errors because they would make
    /// PBFT/pillar DPoS reads nondeterministic.
    fn load_persisted_dpos_snapshots(&self) -> Result<(), anyhow::Error> {
        let last_block = self.last_block_number()?;
        if last_block == 0 {
            return Ok(());
        }

        let mut snapshots = self
            .dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?;
        for block_number in 1..=last_block {
            let Some(raw_snapshot) = self.storage.final_chain().dpos_snapshot_raw(block_number)?
            else {
                continue;
            };
            snapshots.insert(block_number, decode_dpos_snapshot_rlp(&raw_snapshot)?);
        }
        Ok(())
    }

    /// Loads Rust-persisted historical account snapshots for finalized blocks.
    ///
    /// Missing non-genesis payloads are left absent so older databases do not
    /// fabricate account facts from genesis. Latest account reads then fail
    /// explicitly until the finalized head has a Rust account snapshot. Corrupt
    /// payloads are hard errors because account facts drive transaction purge,
    /// proposal filtering, and read-only call validation.
    fn load_persisted_account_snapshots(&self) -> Result<(), anyhow::Error> {
        let last_block = self.last_block_number()?;
        if last_block == 0 {
            return Ok(());
        }

        let mut loaded_latest = 0u64;
        let mut loaded_accounts = None;
        let mut snapshots = self
            .account_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account snapshot lock poisoned"))?;
        for block_number in 1..=last_block {
            let Some(raw_snapshot) = self
                .storage
                .final_chain()
                .account_snapshot_raw(block_number)?
            else {
                continue;
            };
            let snapshot = decode_account_snapshot_rlp(&raw_snapshot).map_err(|err| {
                anyhow::anyhow!(
                    "failed to decode persisted account snapshot for block {block_number}: {err}"
                )
            })?;
            if block_number > loaded_latest {
                loaded_latest = block_number;
                loaded_accounts = Some(snapshot.clone());
            }
            snapshots.insert(block_number, snapshot);
        }
        drop(snapshots);

        if let Some(accounts) = loaded_accounts {
            *self
                .accounts
                .lock()
                .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))? = accounts;
            *self.latest_account_snapshot_block.lock().map_err(|_| {
                anyhow::anyhow!("final-chain latest account snapshot lock poisoned")
            })? = loaded_latest;
        }
        Ok(())
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
    /// endpoint are read from the exact finalized-block DPoS snapshot. DAG
    /// authorization queries intentionally apply the configured delegation
    /// delay before selecting a snapshot.
    fn encode_dpos_validator(
        &self,
        block_number: u64,
        validator: [u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
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

    /// Plans the DPoS snapshot for a newly finalized block.
    ///
    /// The new snapshot clones the previous block state and applies finalized
    /// DPoS transactions. Reward deltas are applied separately after native
    /// reward planning has completed so overflow can abort before persistence.
    fn plan_dpos_snapshot(
        &self,
        block_number: u64,
        dpos_transactions: Vec<DposTransaction>,
    ) -> Result<DposSnapshot, anyhow::Error> {
        let snapshots = self
            .dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?;
        let previous_block = block_number.checked_sub(1).ok_or_else(|| {
            anyhow::anyhow!("cannot append non-genesis DPoS snapshot for block 0")
        })?;
        let mut snapshot = snapshots.get(&previous_block).cloned().ok_or_else(|| {
            anyhow::anyhow!("missing previous DPoS snapshot for block {previous_block}")
        })?;
        for dpos_tx in dpos_transactions {
            match dpos_tx {
                DposTransaction::Register(registration) => {
                    self.apply_dpos_registration(&mut snapshot, registration)?;
                }
                DposTransaction::Delegate {
                    delegator,
                    validator,
                    amount,
                } => {
                    self.apply_dpos_delegate(&mut snapshot, delegator, validator, amount)?;
                }
                DposTransaction::Undelegate {
                    delegator,
                    validator,
                    amount,
                } => {
                    self.apply_dpos_undelegate(&mut snapshot, delegator, validator, amount)?;
                }
                DposTransaction::Redelegate {
                    delegator,
                    from,
                    to,
                    amount,
                } => {
                    self.apply_dpos_redelegate(&mut snapshot, delegator, from, to, amount)?;
                }
            }
        }
        Ok(snapshot)
    }

    fn apply_dpos_registration(
        &self,
        snapshot: &mut DposSnapshot,
        registration: DposRegistration,
    ) -> Result<(), anyhow::Error> {
        if snapshot.total_stakes.contains_key(&registration.validator) {
            anyhow::bail!("Rust FinalChain::finalize DPoS validator is already registered");
        }
        if u256_from_big_endian(&registration.stake)
            > u256_from_big_endian(&self.dpos_validator_maximum_stake)
        {
            anyhow::bail!("Rust FinalChain::finalize DPoS validator stake exceeds maximum");
        }

        let vote_count = dpos_vote_count(
            &registration.stake,
            &self.dpos_eligibility_balance_threshold,
            &self.dpos_vote_eligibility_balance_step,
            &self.dpos_validator_maximum_stake,
        )?;
        snapshot.total_vote_count = snapshot
            .total_vote_count
            .checked_add(vote_count)
            .ok_or_else(|| anyhow::anyhow!("DPoS total vote count overflow"))?;
        snapshot
            .total_stakes
            .insert(registration.validator, registration.stake.clone());
        snapshot
            .commission_rewards
            .entry(registration.validator)
            .or_default();
        snapshot
            .delegator_rewards
            .entry(registration.validator)
            .or_default();
        snapshot
            .validator_metadata
            .insert(registration.validator, registration.metadata);
        snapshot
            .vrf_keys
            .insert(registration.validator, registration.vrf_key);
        snapshot
            .vote_counts
            .insert(registration.validator, vote_count);
        let mut delegations = BTreeMap::new();
        delegations.insert(registration.validator, registration.stake.clone());
        snapshot
            .delegations
            .insert(registration.validator, delegations);
        Ok(())
    }

    fn apply_dpos_delegate(
        &self,
        snapshot: &mut DposSnapshot,
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        let Some(stake) = snapshot.total_stakes.get(&validator) else {
            anyhow::bail!("Rust FinalChain::finalize DPoS validator does not exist for delegate")
        };
        let current_stake = u256_from_big_endian(stake);
        let add_amount = u256_from_big_endian(&amount);
        if add_amount < u256_from_big_endian(&self.dpos_minimum_deposit) {
            anyhow::bail!("Rust FinalChain::finalize DPoS delegation is below minimum deposit");
        }
        let new_stake = current_stake
            .checked_add(add_amount)
            .ok_or_else(|| anyhow::anyhow!("DPoS delegate stake addition overflow"))?;
        if new_stake > u256_from_big_endian(&self.dpos_validator_maximum_stake) {
            anyhow::bail!("Rust FinalChain::finalize DPoS validator stake exceeds maximum");
        }
        let delegations = snapshot.delegations.entry(validator).or_default();
        let current_delegation = delegations
            .get(&delegator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        delegations.insert(
            delegator,
            u256_to_big_endian(
                current_delegation
                    .checked_add(add_amount)
                    .ok_or_else(|| anyhow::anyhow!("DPoS delegation addition overflow"))?,
            ),
        );
        self.set_validator_stake(snapshot, validator, new_stake)
    }

    fn set_validator_stake(
        &self,
        snapshot: &mut DposSnapshot,
        validator: [u8; 20],
        new_stake: U256,
    ) -> Result<(), anyhow::Error> {
        let new_stake_bytes = u256_to_big_endian(new_stake);
        let previous_vote_count = *snapshot.vote_counts.get(&validator).unwrap_or(&0);
        let new_vote_count = dpos_vote_count(
            &new_stake_bytes,
            &self.dpos_eligibility_balance_threshold,
            &self.dpos_vote_eligibility_balance_step,
            &self.dpos_validator_maximum_stake,
        )?;
        snapshot.total_vote_count = if previous_vote_count > new_vote_count {
            snapshot
                .total_vote_count
                .checked_sub(previous_vote_count - new_vote_count)
                .ok_or_else(|| anyhow::anyhow!("DPoS total vote count underflow"))?
        } else {
            snapshot
                .total_vote_count
                .checked_add(new_vote_count - previous_vote_count)
                .ok_or_else(|| anyhow::anyhow!("DPoS total vote count overflow"))?
        };
        snapshot.total_stakes.insert(validator, new_stake_bytes);
        snapshot.vote_counts.insert(validator, new_vote_count);
        Ok(())
    }

    fn apply_dpos_undelegate(
        &self,
        snapshot: &mut DposSnapshot,
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        let Some(stake) = snapshot.total_stakes.get(&validator) else {
            anyhow::bail!("Rust FinalChain::finalize DPoS validator does not exist for undelegate")
        };
        let current_stake = u256_from_big_endian(stake);
        let remove_amount = u256_from_big_endian(&amount);
        let delegations = snapshot.delegations.get_mut(&validator).ok_or_else(|| {
            anyhow::anyhow!("Rust FinalChain::finalize DPoS delegation does not exist")
        })?;
        let current_delegation = delegations
            .get(&delegator)
            .map(|bytes| u256_from_big_endian(bytes))
            .ok_or_else(|| {
                anyhow::anyhow!("Rust FinalChain::finalize DPoS delegator stake does not exist")
            })?;
        if current_delegation < remove_amount {
            anyhow::bail!(
                "Rust FinalChain::finalize DPoS delegator stake underflows on undelegate"
            );
        }
        if current_stake < remove_amount {
            anyhow::bail!("Rust FinalChain::finalize DPoS stake underflows on undelegate");
        }
        let new_delegation = current_delegation - remove_amount;
        if new_delegation.is_zero() {
            delegations.remove(&delegator);
        } else {
            delegations.insert(delegator, u256_to_big_endian(new_delegation));
        }
        let new_stake = current_stake - remove_amount;
        self.set_validator_stake(snapshot, validator, new_stake)
    }

    fn apply_dpos_redelegate(
        &self,
        snapshot: &mut DposSnapshot,
        delegator: [u8; 20],
        from: [u8; 20],
        to: [u8; 20],
        amount: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        let amount = u256_from_big_endian(&amount);
        let from_stake = snapshot
            .total_stakes
            .get(&from)
            .map(|bytes| u256_from_big_endian(bytes))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Rust FinalChain::finalize DPoS source validator does not exist for redelegate"
                )
            })?;
        if from_stake < amount {
            anyhow::bail!("Rust FinalChain::finalize DPoS stake underflows on redelegate")
        }
        let amount = u256_to_big_endian(amount);
        self.apply_dpos_undelegate(snapshot, delegator, from, amount.clone())?;
        self.apply_dpos_delegate(snapshot, delegator, to, amount)?;
        Ok(())
    }

    fn insert_dpos_snapshot(
        &self,
        block_number: u64,
        snapshot: DposSnapshot,
    ) -> Result<(), anyhow::Error> {
        let mut snapshots = self
            .dpos_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain DPoS snapshot lock poisoned"))?;
        snapshots.insert(block_number, snapshot);
        Ok(())
    }

    fn current_account_snapshot(&self) -> Result<HashMap<[u8; 20], Account>, anyhow::Error> {
        self.accounts
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))
            .map(|accounts| accounts.clone())
    }

    fn insert_account_snapshot(
        &self,
        block_number: u64,
        accounts: HashMap<[u8; 20], Account>,
    ) -> Result<(), anyhow::Error> {
        let mut live_accounts = self
            .accounts
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?;
        let mut snapshots = self
            .account_snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain account snapshot lock poisoned"))?;
        let mut latest_snapshot_block = self
            .latest_account_snapshot_block
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain latest account snapshot lock poisoned"))?;

        *live_accounts = accounts.clone();
        snapshots.insert(block_number, accounts);
        *latest_snapshot_block = block_number;
        Ok(())
    }

    /// Plans rewards-stat lifecycle work for one native finalized period.
    ///
    /// The returned plan contains DPoS commission rewards extracted from the
    /// planner's distribution stats, a storage cache intent for the finalization
    /// batch, and the rewards runtime state that should replace the live runtime
    /// only after the surrounding finalization writes commit.
    fn native_rewards_stats_plan(
        &self,
        pbft: &rustaxa_types::PbftBlockMetadata,
        finalized_transactions: &[FinalizationTransaction],
        finalized_dag_blocks: &[FinalizationDagBlock],
        transaction_fees: &[([u8; 32], U256)],
        blocks_per_year: u32,
        cert_votes: Vec<RewardCertVoteFact>,
    ) -> Result<NativeRewardsStatsPlan, anyhow::Error> {
        let gas_used_by_hash = transaction_fees
            .iter()
            .map(|(hash, fee)| {
                let transaction = finalized_transactions
                    .iter()
                    .find(|transaction| transaction.hash == *hash)
                    .ok_or_else(|| anyhow::anyhow!("missing transaction for executed fee hash"))?;
                let gas_price = u256_from_big_endian(&transaction.gas_price);
                let gas_used = gas_used_from_fee(*fee, gas_price)?;
                Ok((*hash, gas_used))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        let dpos_eligible_total_vote_count = if let Some(vote) = cert_votes.first() {
            self.dpos_eligible_total_vote_count(vote.period.saturating_sub(1))?
        } else {
            u64::from(self.rewards_config.committee_size)
        };
        let fact = FinalizedRewardsPeriodFact {
            period: pbft.period,
            block_author: pbft.author,
            blocks_per_year,
            dpos_eligible_total_vote_count,
            transactions: finalized_transactions
                .iter()
                .map(|transaction| RewardTransactionFact {
                    hash: H256::from(transaction.hash),
                    gas_price: u256_from_big_endian(&transaction.gas_price),
                    gas_used: *gas_used_by_hash.get(&transaction.hash).unwrap_or(&0),
                })
                .collect(),
            dag_blocks: finalized_dag_blocks
                .iter()
                .map(|dag_block| RewardDagBlockFact {
                    author: dag_block.author.into(),
                    difficulty: dag_block.difficulty,
                    transaction_hashes: dag_block
                        .transaction_hashes
                        .iter()
                        .copied()
                        .map(H256::from)
                        .collect(),
                })
                .collect(),
            cert_votes,
        };
        let mut runtime = self
            .rewards_stats_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain rewards stats runtime lock poisoned"))?
            .clone();
        let plan = runtime.process_period(fact);
        if plan.status != RewardsStatsStatus::Applied {
            anyhow::bail!(
                "Rust FinalChain::finalize rewards stats rejected period {}: {}",
                plan.current_period,
                plan.error_code
            );
        }
        let distribution_stats = decode_rewards_block_distributions(&plan.distribution_stats)?;
        let fee_rewards_by_validator = fee_rewards_from_distribution_stats(&distribution_stats)?;
        let storage_update = (plan.cache_current_period || plan.clear_cached_stats).then(|| {
            OwnedRewardsStatsUpdate {
                current_period: plan.current_period,
                cache_current_period: plan.cache_current_period,
                clear_cached_stats: plan.clear_cached_stats,
                current_block_stats_rlp: plan.current_block_stats_rlp.clone(),
            }
        });
        if plan.clear_cached_stats {
            runtime.clear_committed(plan.current_period);
        }
        Ok(NativeRewardsStatsPlan {
            fee_rewards_by_validator,
            distribution_stats,
            storage_update,
            runtime_after_commit: runtime,
        })
    }

    fn commit_rewards_stats_runtime(
        &self,
        runtime_after_commit: RewardsStatsRuntime,
    ) -> Result<(), anyhow::Error> {
        *self
            .rewards_stats_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain rewards stats runtime lock poisoned"))? =
            runtime_after_commit;
        Ok(())
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

fn rewards_stats_runtime_from_storage(
    storage: &Arc<Storage>,
    rewards_config: &FinalChainRewardsConfig,
) -> Result<RewardsStatsRuntime, anyhow::Error> {
    let last_block_number = storage
        .final_chain()
        .meta_value(FinalChain::DB_META_LAST_NUMBER)?
        .map(|raw| decode_u64_le(&raw, "final_chain_meta/LAST_NUMBER"))
        .transpose()?
        .unwrap_or_default();
    let frequency = rewards_distribution_frequency(
        &rewards_config.rewards_distribution_frequency,
        last_block_number,
    );
    let persisted_stats = if last_block_number != 0
        && frequency > 1
        && last_block_number.is_multiple_of(u64::from(frequency))
    {
        storage.metadata().clear_block_rewards_stats()?;
        Vec::new()
    } else {
        storage
            .metadata()
            .block_rewards_stats_rlp()?
            .into_iter()
            .map(|(period, data)| RewardsStatsPeriodRlp { period, data })
            .collect()
    };

    RewardsStatsRuntime::new(
        RewardsStatsConfig {
            committee_size: rewards_config.committee_size,
            magnolia_period: rewards_config.magnolia_period,
            aspen_part_one_period: rewards_config.aspen_part_one_period,
        },
        rewards_frequency_rules(rewards_config),
        persisted_stats,
    )
}

fn rewards_frequency_rules(rewards_config: &FinalChainRewardsConfig) -> Vec<RewardsFrequencyRule> {
    rewards_config
        .rewards_distribution_frequency
        .iter()
        .map(|(from_period, frequency)| RewardsFrequencyRule {
            from_period: *from_period,
            frequency: *frequency,
        })
        .collect()
}

fn rewards_distribution_frequency(rules: &[(u64, u32)], period: u64) -> u32 {
    rules
        .iter()
        .rev()
        .find(|(from_period, _)| *from_period <= period)
        .map(|(_, frequency)| *frequency)
        .unwrap_or(1)
}

fn h256_from_slice(raw: &[u8], field: &str) -> Result<ethereum_types::H256, anyhow::Error> {
    if raw.len() != 32 {
        anyhow::bail!("invalid {field} size: expected 32, got {}", raw.len());
    }
    Ok(ethereum_types::H256::from_slice(raw))
}

fn encode_dpos_snapshot_rlp(snapshot: &DposSnapshot) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(11);
    append_address_bytes_map(&mut stream, &snapshot.total_stakes);
    append_address_bytes_map(&mut stream, &snapshot.commission_rewards);
    append_validator_metadata_map(&mut stream, &snapshot.validator_metadata);
    append_address_fixed_hash_map(&mut stream, &snapshot.vrf_keys);
    append_vote_count_map(&mut stream, &snapshot.vote_counts);
    stream.append(&snapshot.total_vote_count);
    append_delegations_map(&mut stream, &snapshot.delegations);
    append_address_bytes_map(&mut stream, &snapshot.delegator_rewards);
    stream.append(&snapshot.minted_tokens.as_slice());
    stream.append(&snapshot.total_supply.as_slice());
    stream.append(&snapshot.current_yield);
    stream.out().to_vec()
}

fn decode_dpos_snapshot_rlp(raw: &[u8]) -> Result<DposSnapshot, anyhow::Error> {
    let rlp = Rlp::new(raw);
    let item_count = rlp.item_count()?;
    if item_count != 5 && item_count != 6 && item_count != 7 && item_count != 9 && item_count != 11
    {
        anyhow::bail!(
            "DPoS snapshot RLP must contain exactly five, six, seven, nine, or eleven items"
        );
    }
    let total_stakes = decode_address_bytes_map(&rlp.at(0)?, "total stakes")?;
    let commission_rewards = decode_address_bytes_map(&rlp.at(1)?, "commission rewards")?;
    let validator_metadata = decode_validator_metadata_map(&rlp.at(2)?)?;
    let (vrf_keys, vote_counts, total_vote_count, delegations) = if item_count >= 6 {
        (
            decode_address_fixed_hash_map(&rlp.at(3)?, "VRF key")?,
            decode_vote_count_map(&rlp.at(4)?)?,
            rlp.val_at(5)?,
            if item_count >= 7 {
                decode_delegations_map(&rlp.at(6)?)?
            } else {
                synthesize_self_delegations(&total_stakes)
            },
        )
    } else {
        (
            BTreeMap::new(),
            decode_vote_count_map(&rlp.at(3)?)?,
            rlp.val_at(4)?,
            synthesize_self_delegations(&total_stakes),
        )
    };
    let delegator_rewards = if item_count >= 9 {
        decode_address_bytes_map(&rlp.at(7)?, "delegator rewards")?
    } else {
        BTreeMap::new()
    };
    let minted_tokens = if item_count >= 9 {
        rlp.at(8)?.data()?.to_vec()
    } else {
        Vec::new()
    };
    let (total_supply, current_yield) = if item_count == 11 {
        (rlp.at(9)?.data()?.to_vec(), rlp.val_at(10)?)
    } else {
        (Vec::new(), 0)
    };
    Ok(DposSnapshot {
        total_stakes,
        commission_rewards,
        delegator_rewards,
        validator_metadata,
        vrf_keys,
        vote_counts,
        total_vote_count,
        delegations,
        minted_tokens,
        total_supply,
        current_yield,
    })
}

/// Encodes a full FinalChain account snapshot into deterministic RLP.
///
/// The payload is a sorted list of account entries keyed by address. Each entry
/// stores the account fields currently exposed through the Rust/C++ bridge:
/// nonce, balance bytes, storage root, code hash, and code size. Empty accounts
/// are omitted by the caller by absence from the map, matching the in-memory
/// account model used for historical reads.
fn encode_account_snapshot_rlp(accounts: &HashMap<[u8; 20], Account>) -> Vec<u8> {
    let sorted_accounts = accounts.iter().collect::<BTreeMap<_, _>>();
    let mut stream = rlp::RlpStream::new_list(sorted_accounts.len());
    for (address, account) in sorted_accounts {
        stream.begin_list(6);
        stream.append(&address.as_slice());
        stream.append(&account.nonce);
        stream.append(&account.balance.as_slice());
        stream.append(&account.storage_root_hash.as_slice());
        stream.append(&account.code_hash.as_slice());
        stream.append(&account.code_size);
    }
    stream.out().to_vec()
}

/// Decodes a persisted account snapshot payload.
///
/// Malformed field counts, address lengths, root lengths, or code-hash lengths
/// are hard errors because FinalChain account facts feed consensus transaction
/// filtering and read-only execution checks after restart.
fn decode_account_snapshot_rlp(raw: &[u8]) -> Result<HashMap<[u8; 20], Account>, anyhow::Error> {
    let rlp = Rlp::new(raw);
    let mut accounts = HashMap::with_capacity(rlp.item_count()?);
    for item in rlp.iter() {
        if item.item_count()? != 6 {
            anyhow::bail!("account snapshot entry must contain exactly six items");
        }
        let address = decode_address(&item.at(0)?, "account address")?;
        accounts.insert(
            address,
            Account {
                nonce: item.val_at(1)?,
                balance: item.val_at(2)?,
                storage_root_hash: decode_fixed_hash(&item.at(3)?, "account storage root")?,
                code_hash: decode_fixed_hash(&item.at(4)?, "account code hash")?,
                code_size: item.val_at(5)?,
            },
        );
    }
    Ok(accounts)
}

fn append_address_bytes_map(stream: &mut rlp::RlpStream, map: &BTreeMap<[u8; 20], Vec<u8>>) {
    stream.begin_list(map.len());
    for (address, value) in map {
        stream.begin_list(2);
        stream.append(&address.as_slice());
        stream.append(&value.as_slice());
    }
}

fn append_validator_metadata_map(
    stream: &mut rlp::RlpStream,
    map: &BTreeMap<[u8; 20], DposValidatorMetadata>,
) {
    stream.begin_list(map.len());
    for (address, metadata) in map {
        stream.begin_list(5);
        stream.append(&address.as_slice());
        stream.append(&metadata.owner.as_slice());
        stream.append(&metadata.commission);
        stream.append(&metadata.description.as_str());
        stream.append(&metadata.endpoint.as_str());
    }
}

fn append_address_fixed_hash_map(stream: &mut rlp::RlpStream, map: &BTreeMap<[u8; 20], [u8; 32]>) {
    stream.begin_list(map.len());
    for (address, value) in map {
        stream.begin_list(2);
        stream.append(&address.as_slice());
        stream.append(&value.as_slice());
    }
}

fn append_vote_count_map(stream: &mut rlp::RlpStream, map: &BTreeMap<[u8; 20], u64>) {
    stream.begin_list(map.len());
    for (address, vote_count) in map {
        stream.begin_list(2);
        stream.append(&address.as_slice());
        stream.append(vote_count);
    }
}

fn append_delegations_map(stream: &mut rlp::RlpStream, map: &DposDelegations) {
    stream.begin_list(map.len());
    for (validator, delegations) in map {
        stream.begin_list(2);
        stream.append(&validator.as_slice());
        stream.begin_list(delegations.len());
        for (delegator, stake) in delegations {
            stream.begin_list(2);
            stream.append(&delegator.as_slice());
            stream.append(&stake.as_slice());
        }
    }
}

fn decode_delegations_map(rlp: &Rlp<'_>) -> Result<DposDelegations, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!("DPoS snapshot delegations entry must contain exactly two items");
        }
        let validator = decode_address(&item.at(0)?, "delegations validator")?;
        let mut delegations = BTreeMap::new();
        for delegation in item.at(1)?.iter() {
            if delegation.item_count()? != 2 {
                anyhow::bail!("DPoS snapshot delegation item must contain exactly two items");
            }
            delegations.insert(
                decode_address(&delegation.at(0)?, "delegator address")?,
                delegation.val_at(1)?,
            );
        }
        map.insert(validator, delegations);
    }
    Ok(map)
}

fn synthesize_self_delegations(total_stakes: &BTreeMap<[u8; 20], Vec<u8>>) -> DposDelegations {
    total_stakes
        .iter()
        .map(|(validator, stake)| {
            let mut delegations = BTreeMap::new();
            delegations.insert(*validator, stake.clone());
            (*validator, delegations)
        })
        .collect()
}

fn decode_address_fixed_hash_map(
    rlp: &Rlp<'_>,
    field: &str,
) -> Result<BTreeMap<[u8; 20], [u8; 32]>, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!("DPoS snapshot {field} entry must contain exactly two items");
        }
        map.insert(
            decode_address(&item.at(0)?, field)?,
            decode_fixed_hash(&item.at(1)?, field)?,
        );
    }
    Ok(map)
}

fn decode_address_bytes_map(
    rlp: &Rlp<'_>,
    field: &str,
) -> Result<BTreeMap<[u8; 20], Vec<u8>>, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!("DPoS snapshot {field} entry must contain exactly two items");
        }
        map.insert(decode_address(&item.at(0)?, field)?, item.val_at(1)?);
    }
    Ok(map)
}

fn decode_validator_metadata_map(
    rlp: &Rlp<'_>,
) -> Result<BTreeMap<[u8; 20], DposValidatorMetadata>, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 5 {
            anyhow::bail!("DPoS snapshot metadata entry must contain exactly five items");
        }
        map.insert(
            decode_address(&item.at(0)?, "validator metadata address")?,
            DposValidatorMetadata {
                owner: decode_address(&item.at(1)?, "validator metadata owner")?,
                commission: item.val_at(2)?,
                description: item.val_at(3)?,
                endpoint: item.val_at(4)?,
            },
        );
    }
    Ok(map)
}

fn decode_vote_count_map(rlp: &Rlp<'_>) -> Result<BTreeMap<[u8; 20], u64>, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!("DPoS snapshot vote-count entry must contain exactly two items");
        }
        map.insert(
            decode_address(&item.at(0)?, "vote-count address")?,
            item.val_at(1)?,
        );
    }
    Ok(map)
}

fn decode_address(rlp: &Rlp<'_>, field: &str) -> Result<[u8; 20], anyhow::Error> {
    let bytes: Vec<u8> = rlp.as_val()?;
    if bytes.len() != 20 {
        anyhow::bail!(
            "invalid snapshot {field} size: expected 20, got {}",
            bytes.len()
        );
    }
    let mut address = [0u8; 20];
    address.copy_from_slice(&bytes);
    Ok(address)
}

fn decode_fixed_hash(rlp: &Rlp<'_>, field: &str) -> Result<[u8; 32], anyhow::Error> {
    let bytes: Vec<u8> = rlp.as_val()?;
    if bytes.len() != 32 {
        anyhow::bail!(
            "invalid snapshot {field} size: expected 32, got {}",
            bytes.len()
        );
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
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

fn gas_used_from_fee(fee: U256, gas_price: U256) -> Result<u64, anyhow::Error> {
    if gas_price.is_zero() {
        anyhow::ensure!(
            fee.is_zero(),
            "transaction fee is nonzero while gas price is zero"
        );
        return Ok(0);
    }
    let gas_used = fee / gas_price;
    anyhow::ensure!(
        fee % gas_price == U256::zero(),
        "transaction fee is not divisible by gas price"
    );
    anyhow::ensure!(
        gas_used <= U256::from(u64::MAX),
        "transaction gas used does not fit into u64"
    );
    Ok(gas_used.as_u64())
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

fn fee_rewards_from_distribution_stats(
    distribution_stats: &[RewardsBlockDistribution],
) -> Result<BTreeMap<[u8; 20], U256>, anyhow::Error> {
    let mut rewards = BTreeMap::new();
    for stats in distribution_stats {
        for (validator, validator_stats) in &stats.validators_stats {
            if !validator_stats.fees_rewards.is_zero() {
                merge_reward_value(&mut rewards, *validator, validator_stats.fees_rewards)?;
            }
        }
    }
    Ok(rewards)
}

fn merge_reward_map(
    target: &mut BTreeMap<[u8; 20], U256>,
    source: &BTreeMap<[u8; 20], U256>,
) -> Result<(), anyhow::Error> {
    for (validator, reward) in source {
        merge_reward_value(target, *validator, *reward)?;
    }
    Ok(())
}

fn merge_reward_value(
    target: &mut BTreeMap<[u8; 20], U256>,
    validator: [u8; 20],
    reward: U256,
) -> Result<(), anyhow::Error> {
    if reward.is_zero() {
        return Ok(());
    }
    let current = target.entry(validator).or_insert_with(U256::zero);
    *current = current
        .checked_add(reward)
        .ok_or_else(|| anyhow::anyhow!("validator reward delta overflow"))?;
    Ok(())
}

fn apply_reward_map(
    target: &mut BTreeMap<[u8; 20], Vec<u8>>,
    rewards: BTreeMap<[u8; 20], U256>,
    overflow_message: &'static str,
) -> Result<(), anyhow::Error> {
    for (validator, reward) in rewards {
        if reward.is_zero() {
            continue;
        }
        let current = target
            .get(&validator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        target.insert(
            validator,
            u256_to_big_endian(
                current
                    .checked_add(reward)
                    .ok_or_else(|| anyhow::anyhow!(overflow_message))?,
            ),
        );
    }
    Ok(())
}

fn total_staked_amount(snapshot: &DposSnapshot) -> Result<U256, anyhow::Error> {
    snapshot
        .total_stakes
        .values()
        .try_fold(U256::zero(), |total, stake| {
            total
                .checked_add(u256_from_big_endian(stake))
                .ok_or_else(|| anyhow::anyhow!("DPoS total delegated stake overflow"))
        })
}

fn percent_of(value: U256, percent: u16, label: &'static str) -> Result<U256, anyhow::Error> {
    Ok(value
        .checked_mul(U256::from(percent))
        .ok_or_else(|| anyhow::anyhow!("{label} percentage multiplication overflow"))?
        / U256::from(100u64))
}

fn percent_of_max_commission(value: U256, commission: u16) -> Result<U256, anyhow::Error> {
    Ok(value
        .checked_mul(U256::from(commission))
        .ok_or_else(|| anyhow::anyhow!("validator commission multiplication overflow"))?
        / U256::from(10_000u64))
}

/// Sums executed transaction fees for legacy pre-Magnolia PBFT rewards.
///
/// The input preserves each transaction hash alongside its gas fee so the same
/// execution facts can also feed post-Magnolia DAG-author commission planning.
/// This helper ignores the hashes, returns zero for empty blocks, and reports
/// overflow as a consensus execution error.
fn total_transaction_fees(transaction_fees: &[([u8; 32], U256)]) -> Result<U256, anyhow::Error> {
    transaction_fees
        .iter()
        .try_fold(U256::zero(), |total, (_, fee)| {
            total
                .checked_add(*fee)
                .ok_or_else(|| anyhow::anyhow!("transaction fee total overflow"))
        })
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

fn decode_abi_address_argument_with_offset(
    input: &[u8],
    start: usize,
    function_name: &str,
) -> Result<[u8; 20], anyhow::Error> {
    if input.len() < start + 32 {
        anyhow::bail!("{function_name} input is shorter than selector plus ABI argument");
    }
    let mut address = [0u8; 20];
    address.copy_from_slice(&input[start + 12..start + 32]);
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

/// Decodes Rust-supported DPoS contract method payloads.
///
/// The function validates Solidity ABI shape and extracts the fields that affect
/// consensus-visible DPoS snapshots. Signature proof validation remains outside
/// this slice, so malformed proofs are rejected only by ABI shape while the
/// registered validator address and caller-owned stake are kept explicit.
fn decode_dpos_transaction(
    input: &[u8],
    owner: [u8; 20],
) -> Result<DposTransaction, anyhow::Error> {
    if input.len() < 4 {
        anyhow::bail!("Rust FinalChain::finalize DPoS transaction input is missing selector");
    }
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&input[..4]);
    match selector {
        DPOS_REGISTER_VALIDATOR_SELECTOR => {
            let registration = decode_dpos_register_validator(input, owner)?;
            Ok(DposTransaction::Register(registration))
        }
        DPOS_DELEGATE_SELECTOR => {
            let validator = decode_abi_address_argument(input, "delegate(address)")?;
            Ok(DposTransaction::Delegate {
                delegator: owner,
                validator,
                amount: vec![],
            })
        }
        DPOS_UNDELEGATE_SELECTOR => {
            if input.len() < 4 + 2 * 32 {
                anyhow::bail!("undelegate input is shorter than selector plus ABI head");
            }
            let validator = decode_abi_address_argument(input, "undelegate(address,...)")?;
            let amount = decode_abi_word_as_vec(input, 4 + 32, "undelegate amount")?;
            Ok(DposTransaction::Undelegate {
                delegator: owner,
                validator,
                amount,
            })
        }
        DPOS_REDELEGATE_SELECTOR => {
            if input.len() < 4 + 3 * 32 {
                anyhow::bail!("reDelegate input is shorter than selector plus ABI head");
            }
            let from = decode_abi_address_argument_with_offset(input, 4, "reDelegate from")?;
            let to = decode_abi_address_argument_with_offset(input, 4 + 32, "reDelegate to")?;
            let amount = decode_abi_word_as_vec(input, 4 + 2 * 32, "reDelegate amount")?;
            Ok(DposTransaction::Redelegate {
                delegator: owner,
                from,
                to,
                amount,
            })
        }
        _ => {
            anyhow::bail!(
                "Rust FinalChain::finalize unsupported DPoS selector 0x{}",
                selector_hex(selector)
            )
        }
    }
}

fn decode_dpos_register_validator(
    input: &[u8],
    owner: [u8; 20],
) -> Result<DposRegistration, anyhow::Error> {
    if input.len() < 4 {
        anyhow::bail!("Rust FinalChain::finalize DPoS transaction input is missing selector");
    }
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&input[..4]);
    if selector != DPOS_REGISTER_VALIDATOR_SELECTOR {
        anyhow::bail!(
            "Rust FinalChain::finalize unsupported DPoS selector 0x{}",
            selector_hex(selector)
        );
    }
    let head_len = 4 + 6 * 32;
    if input.len() < head_len {
        anyhow::bail!("registerValidator input is shorter than selector plus ABI head");
    }

    let validator = decode_abi_address_argument(input, "registerValidator(address,...)")?;
    let proof_offset = decode_abi_word_as_usize(input, 4 + 32, "registerValidator proof offset")?;
    let vrf_key_offset =
        decode_abi_word_as_usize(input, 4 + 2 * 32, "registerValidator VRF key offset")?;
    let commission = decode_abi_word_as_u16(input, 4 + 3 * 32, "registerValidator commission")?;
    let description_offset =
        decode_abi_word_as_usize(input, 4 + 4 * 32, "registerValidator description offset")?;
    let endpoint_offset =
        decode_abi_word_as_usize(input, 4 + 5 * 32, "registerValidator endpoint offset")?;
    let proof = decode_abi_dynamic_bytes(input, proof_offset, "registerValidator proof")?;
    if proof.is_empty() {
        anyhow::bail!("registerValidator proof cannot be empty");
    }
    let vrf_key = decode_abi_dynamic_bytes(input, vrf_key_offset, "registerValidator VRF key")?;
    if vrf_key.len() != 32 {
        anyhow::bail!(
            "registerValidator VRF key must be 32 bytes, got {}",
            vrf_key.len()
        );
    }
    let mut vrf_key_bytes = [0u8; 32];
    vrf_key_bytes.copy_from_slice(&vrf_key);

    Ok(DposRegistration {
        validator,
        stake: vec![],
        vrf_key: vrf_key_bytes,
        metadata: DposValidatorMetadata {
            owner,
            commission,
            description: decode_abi_dynamic_string(
                input,
                description_offset,
                "registerValidator description",
            )?,
            endpoint: decode_abi_dynamic_string(
                input,
                endpoint_offset,
                "registerValidator endpoint",
            )?,
        },
    })
}

fn decode_abi_word_as_usize(
    input: &[u8],
    offset: usize,
    field: &str,
) -> Result<usize, anyhow::Error> {
    let word = abi_word(input, offset, field)?;
    let value = u256_from_big_endian(word);
    if value > U256::from(usize::MAX) {
        anyhow::bail!("{field} does not fit into usize");
    }
    Ok(value.as_usize())
}

fn decode_abi_word_as_u16(input: &[u8], offset: usize, field: &str) -> Result<u16, anyhow::Error> {
    let word = abi_word(input, offset, field)?;
    let value = u256_from_big_endian(word);
    if value > U256::from(u16::MAX) {
        anyhow::bail!("{field} does not fit into uint16");
    }
    Ok(value.as_u32() as u16)
}

fn decode_abi_word_as_vec(
    input: &[u8],
    offset: usize,
    field: &str,
) -> Result<Vec<u8>, anyhow::Error> {
    let word = abi_word(input, offset, field)?;
    Ok(word.to_vec())
}

fn abi_word<'a>(input: &'a [u8], offset: usize, field: &str) -> Result<&'a [u8], anyhow::Error> {
    input
        .get(offset..offset + 32)
        .ok_or_else(|| anyhow::anyhow!("{field} ABI word is out of bounds"))
}

fn decode_abi_dynamic_bytes(
    input: &[u8],
    relative_offset: usize,
    field: &str,
) -> Result<Vec<u8>, anyhow::Error> {
    let offset = 4usize
        .checked_add(relative_offset)
        .ok_or_else(|| anyhow::anyhow!("{field} offset overflow"))?;
    let len_word = abi_word(input, offset, field)?;
    let len_value = u256_from_big_endian(len_word);
    if len_value > U256::from(usize::MAX) {
        anyhow::bail!("{field} length does not fit into usize");
    }
    let len = len_value.as_usize();
    let start = offset
        .checked_add(32)
        .ok_or_else(|| anyhow::anyhow!("{field} start offset overflow"))?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| anyhow::anyhow!("{field} end offset overflow"))?;
    Ok(input
        .get(start..end)
        .ok_or_else(|| anyhow::anyhow!("{field} bytes are out of bounds"))?
        .to_vec())
}

fn decode_abi_dynamic_string(
    input: &[u8],
    relative_offset: usize,
    field: &str,
) -> Result<String, anyhow::Error> {
    let bytes = decode_abi_dynamic_bytes(input, relative_offset, field)?;
    String::from_utf8(bytes).map_err(|err| anyhow::anyhow!("{field} is not valid UTF-8: {err}"))
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
    accounts: HashMap<[u8; 20], Account>,
    receipts: Vec<Vec<u8>>,
    gas_used: u64,
    transaction_fees: Vec<([u8; 32], U256)>,
    dpos_transactions: Vec<DposTransaction>,
}

enum DposTransaction {
    Register(DposRegistration),
    Delegate {
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    },
    Undelegate {
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    },
    Redelegate {
        delegator: [u8; 20],
        from: [u8; 20],
        to: [u8; 20],
        amount: Vec<u8>,
    },
}

/// DPoS validator registration decoded from the Rust-supported contract subset.
///
/// This carries only the deterministic state mutation needed by FinalChain:
/// validator identity, owner metadata, VRF key, and initial self-delegated stake.
struct DposRegistration {
    validator: [u8; 20],
    stake: Vec<u8>,
    vrf_key: [u8; 32],
    metadata: DposValidatorMetadata,
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
            delegations: vec![(address, u256_to_big_endian(stake))],
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

    fn abi_word_from_bytes_offset(offset: usize) -> [u8; 32] {
        abi_word_from_usize(offset, "test ABI offset").unwrap()
    }

    fn register_validator_input(
        validator: [u8; 20],
        proof: &[u8],
        vrf_key: [u8; 32],
        commission: u16,
        description: &str,
        endpoint: &str,
    ) -> Vec<u8> {
        let proof_offset = 6 * 32;
        let vrf_offset = proof_offset + 32 + abi_padded_len(proof.len()).unwrap();
        let description_offset = vrf_offset + 32 + abi_padded_len(vrf_key.len()).unwrap();
        let endpoint_offset =
            description_offset + abi_dynamic_string_tail_len(description).unwrap();
        let mut input = DPOS_REGISTER_VALIDATOR_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_bytes_offset(proof_offset));
        input.extend_from_slice(&abi_word_from_bytes_offset(vrf_offset));
        input.extend_from_slice(&abi_word_from_u64(u64::from(commission)));
        input.extend_from_slice(&abi_word_from_bytes_offset(description_offset));
        input.extend_from_slice(&abi_word_from_bytes_offset(endpoint_offset));
        input.extend_from_slice(&abi_word_from_usize(proof.len(), "test proof length").unwrap());
        input.extend_from_slice(proof);
        input.resize(4 + vrf_offset, 0);
        input
            .extend_from_slice(&abi_word_from_usize(vrf_key.len(), "test VRF key length").unwrap());
        input.extend_from_slice(&vrf_key);
        input.resize(4 + description_offset, 0);
        input.extend_from_slice(&abi_string_tail(description).unwrap());
        input.extend_from_slice(&abi_string_tail(endpoint).unwrap());
        input
    }

    fn delegate_input(validator: [u8; 20]) -> Vec<u8> {
        let mut input = DPOS_DELEGATE_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input
    }

    fn undelegate_input(validator: [u8; 20], amount: U256) -> Vec<u8> {
        let mut input = DPOS_UNDELEGATE_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u256_bytes(&u256_to_big_endian(amount)).unwrap());
        input
    }

    fn redelegate_input(from: [u8; 20], to: [u8; 20], amount: U256) -> Vec<u8> {
        let mut input = DPOS_REDELEGATE_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(from));
        input.extend_from_slice(&abi_word_from_address(to));
        input.extend_from_slice(&abi_word_from_u256_bytes(&u256_to_big_endian(amount)).unwrap());
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
            minimum_deposit: vec![],
            delegation_delay: 0,
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
                minimum_deposit: vec![],
                delegation_delay: 0,
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
                minimum_deposit: vec![],
                delegation_delay: 0,
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
                minimum_deposit: vec![],
                delegation_delay: 0,
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
        assert_eq!(
            final_chain
                .account_at_block(0, sender)
                .unwrap()
                .unwrap()
                .nonce,
            0
        );
        assert!(final_chain.account_at_block(0, receiver).unwrap().is_none());
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
        assert_eq!(
            final_chain
                .account_at_block(0, sender)
                .unwrap()
                .unwrap()
                .nonce,
            0
        );
        assert_eq!(
            final_chain
                .account_at_block(period, sender)
                .unwrap()
                .unwrap()
                .nonce,
            1
        );
        assert!(final_chain.account_at_block(0, receiver).unwrap().is_none());
        assert_eq!(
            u256_from_big_endian(
                &final_chain
                    .account_at_block(period, receiver)
                    .unwrap()
                    .unwrap()
                    .balance
            ),
            U256::from(13u64)
        );
        assert!(
            final_chain
                .account_at_block(period + 1, sender)
                .unwrap_err()
                .to_string()
                .contains("account snapshot unavailable")
        );
        assert_eq!(balance_of(&final_chain, receiver), U256::from(13u64));
        assert_eq!(balance_of(&final_chain, beneficiary_bytes), U256::zero());

        drop(final_chain);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = new_final_chain(
            storage.clone(),
            block_gas_limit,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![],
        );
        assert_eq!(final_chain.last_block_number().unwrap(), period);
        assert_eq!(final_chain.account(sender).unwrap().unwrap().nonce, 1);
        assert_eq!(
            balance_of(&final_chain, sender),
            U256::from(1_000_000u64) - U256::from(13u64) - U256::from(VALUE_TRANSFER_GAS * 2)
        );
        assert_eq!(balance_of(&final_chain, receiver), U256::from(13u64));
        assert_eq!(
            final_chain
                .account_at_block(0, sender)
                .unwrap()
                .unwrap()
                .nonce,
            0
        );
        assert_eq!(
            final_chain
                .account_at_block(period, sender)
                .unwrap()
                .unwrap()
                .nonce,
            1
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_credits_pre_magnolia_fee_to_pbft_beneficiary() {
        let path = temp_db_path("finalize-pre-magnolia-fee");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let receiver = [0x32; 20];
        let signing_key = SigningKey::from_slice(&[8u8; 32]).unwrap();
        let sender: [u8; 20] = address_from_signing_key(&signing_key).into();
        let pbft_block = signed_pbft_block(&signing_key, period, 88);
        let transaction_rlp = vec![0xc1, 0x81];
        let transaction = test_transaction(
            0xB1,
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
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![],
            GenesisDposConfig {
                eligibility_balance_threshold: vec![],
                vote_eligibility_balance_step: vec![],
                validator_maximum_stake: vec![],
                minimum_deposit: vec![],
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 2,
            },
            FinalChainRewardsConfig {
                magnolia_period: 2,
                ..Default::default()
            },
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![transaction], vec![])
            .unwrap();

        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, VALUE_TRANSFER_GAS, VALUE_TRANSFER_GAS)
        );
        assert_eq!(
            balance_of(&final_chain, sender),
            U256::from(1_000_000u64) - U256::from(13u64)
        );
        assert_eq!(balance_of(&final_chain, receiver), U256::from(13u64));
        assert_eq!(
            u256_from_big_endian(
                &final_chain
                    .account_at_block(period, sender)
                    .unwrap()
                    .unwrap()
                    .balance
            ),
            U256::from(1_000_000u64) - U256::from(13u64)
        );

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
        let genesis_validator = genesis_validator(dag_author, U256::from(10_000u64));
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
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
            vec![genesis_validator.clone()],
            genesis_dpos_config.clone(),
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(
                pbft_block,
                vec![transaction.clone()],
                vec![FinalizationDagBlock {
                    author: dag_author,
                    difficulty: 0,
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
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::from(VALUE_TRANSFER_GAS * 2)
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, dag_author)
                .unwrap(),
            10
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            10
        );
        assert!(final_chain.dpos_is_eligible(period, dag_author).unwrap());
        assert_eq!(
            final_chain
                .dpos_validators_total_stakes(period)
                .unwrap()
                .iter()
                .map(|stake| (stake.address, u256_from_big_endian(&stake.stake)))
                .collect::<Vec<_>>(),
            vec![(dag_author, U256::from(10_000u64))]
        );
        assert_eq!(
            final_chain
                .dpos_validators_eligible_vote_counts(period)
                .unwrap()
                .iter()
                .map(|vote_count| (vote_count.address, vote_count.vote_count))
                .collect::<Vec<_>>(),
            vec![(dag_author, 10)]
        );
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::ExecutedBlkCount as u8)
                .unwrap(),
            1
        );
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::ExecutedTrxCount as u8)
                .unwrap(),
            1
        );

        drop(final_chain);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = FinalChain::new(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![genesis_validator],
            genesis_dpos_config,
        )
        .unwrap();
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, dag_author)
                .unwrap(),
            10
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            10
        );
        assert!(final_chain.dpos_is_eligible(period, dag_author).unwrap());
        assert_eq!(
            final_chain
                .dpos_validators_total_stakes(period)
                .unwrap()
                .iter()
                .map(|stake| (stake.address, u256_from_big_endian(&stake.stake)))
                .collect::<Vec<_>>(),
            vec![(dag_author, U256::from(10_000u64))]
        );
        assert_eq!(
            final_chain
                .dpos_validators_eligible_vote_counts(period)
                .unwrap()
                .iter()
                .map(|vote_count| (vote_count.address, vote_count.vote_count))
                .collect::<Vec<_>>(),
            vec![(dag_author, 10)]
        );
        let facts = final_chain
            .dag_dpos_authorization_facts(period, dag_author)
            .unwrap();
        assert_eq!(facts.sender_eligible_vote_count, 10);
        assert_eq!(facts.eligibility_status, DAG_VERIFY_DPOS_STATUS_ELIGIBLE);
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(dag_author)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::from(VALUE_TRANSFER_GAS * 2)
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::from(VALUE_TRANSFER_GAS * 2)
        );
        assert!(
            final_chain
                .dpos_eligible_total_vote_count(period + 1)
                .unwrap_err()
                .to_string()
                .contains("snapshot for block 2")
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_distributes_fixed_yield_minted_dag_rewards() {
        let path = temp_db_path("finalize-fixed-yield-minted-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x61; 20];
        let receiver = [0x62; 20];
        let dag_author = [0x63; 20];
        let genesis_validator = genesis_validator_with_metadata(
            dag_author,
            U256::from(10_000u64),
            [0x64; 20],
            2_500,
            "validator",
            "endpoint",
        );
        let signing_key = SigningKey::from_slice(&[18u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 181);
        let transaction_rlp = vec![0xc1, 0xA1];
        let transaction = test_transaction(
            0xA1,
            sender,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::zero(),
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
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![genesis_validator],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
            FinalChainRewardsConfig {
                committee_size: 100,
                magnolia_period: 0,
                aspen_part_one_period: u64::MAX,
                aspen_part_two_period: 0,
                max_block_author_reward_percent: 0,
                dag_proposers_reward_percent: 100,
                yield_percentage: 20,
                dpos_blocks_per_year: 10,
                rewards_distribution_frequency: vec![(0, 1)],
                ..Default::default()
            },
        )
        .unwrap();

        let (header_rlp, _) = final_chain
            .finalize_block(
                pbft_block,
                vec![transaction.clone()],
                vec![FinalizationDagBlock {
                    author: dag_author,
                    difficulty: 0,
                    transaction_hashes: vec![transaction.hash],
                }],
            )
            .unwrap();

        assert_eq!(
            Rlp::new(&header_rlp).val_at::<U256>(11).unwrap(),
            U256::from(200u64)
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::from(200u64)
        );
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(dag_author)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::from(50u64)
        );
        let snapshot = final_chain
            .dpos_snapshot_at_finalized_block(period)
            .unwrap();
        assert_eq!(
            snapshot
                .delegator_rewards
                .get(&dag_author)
                .map(|reward| u256_from_big_endian(reward))
                .unwrap(),
            U256::from(150u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_distributes_aspen_part_two_dynamic_rewards_and_supply() {
        let path = temp_db_path("finalize-aspen-part-two-dynamic-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x65; 20];
        let receiver = [0x66; 20];
        let signing_key = SigningKey::from_slice(&[19u8; 32]).unwrap();
        let period = 1u64;
        let pbft_block = signed_pbft_block(&signing_key, period, 191);
        let transaction_rlp = vec![0xc1, 0xB1];
        let transaction = test_transaction(
            0xB1,
            validator,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::zero(),
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
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(validator, U256::from(1_000_000u64))],
            vec![genesis_validator_with_metadata(
                validator,
                U256::from(10_000u64),
                [0x67; 20],
                2_500,
                "validator",
                "endpoint",
            )],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
            FinalChainRewardsConfig {
                committee_size: 100,
                magnolia_period: 0,
                aspen_part_one_period: 0,
                aspen_part_two_period: 1,
                max_block_author_reward_percent: 0,
                dag_proposers_reward_percent: 100,
                dpos_blocks_per_year: 10,
                genesis_balance_sum: u256_to_big_endian(U256::from(1_000_000u64)),
                aspen_max_supply: u256_to_big_endian(U256::from(2_000_000u64)),
                rewards_distribution_frequency: vec![(0, 1)],
                ..Default::default()
            },
        )
        .unwrap();

        let (header_rlp, _) = final_chain
            .finalize_block(
                pbft_block,
                vec![transaction.clone()],
                vec![FinalizationDagBlock {
                    author: validator,
                    difficulty: 0,
                    transaction_hashes: vec![transaction.hash],
                }],
            )
            .unwrap();

        assert_eq!(
            Rlp::new(&header_rlp).val_at::<U256>(11).unwrap(),
            U256::from(1_000u64)
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::from(1_000u64)
        );
        assert_eq!(
            final_chain.dpos_total_amount_delegated(period).unwrap(),
            u256_to_big_endian(U256::from(10_000u64))
        );
        assert_eq!(final_chain.dpos_yield(period).unwrap(), 1_000_000);
        assert_eq!(
            final_chain.dpos_total_supply(period).unwrap(),
            u256_to_big_endian(U256::from(1_001_000u64))
        );
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(validator)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::from(250u64)
        );
        let snapshot = final_chain
            .dpos_snapshot_at_finalized_block(period)
            .unwrap();
        assert!(snapshot.minted_tokens.is_empty());
        assert_eq!(
            u256_from_big_endian(&snapshot.total_supply),
            U256::from(1_001_000u64)
        );
        assert_eq!(snapshot.current_yield, 1_000_000);
        assert_eq!(
            snapshot
                .delegator_rewards
                .get(&validator)
                .map(|reward| u256_from_big_endian(reward))
                .unwrap(),
            U256::from(750u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_persists_and_reloads_rewards_stats_interval_cache() {
        let path = temp_db_path("finalize-rewards-stats-cache");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let sender = [0x24; 20];
        let receiver = [0x25; 20];
        let dag_author = [0x26; 20];
        let genesis_validator = genesis_validator(dag_author, U256::from(10_000u64));
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let rewards_config = FinalChainRewardsConfig {
            committee_size: 100,
            magnolia_period: 0,
            aspen_part_one_period: u64::MAX,
            rewards_distribution_frequency: vec![(0, 2)],
            ..Default::default()
        };
        let signing_key = SigningKey::from_slice(&[14u8; 32]).unwrap();
        let period_one = 1u64;
        let period_two = 2u64;
        let gas_price = U256::from(2u64);
        let period_one_pbft = signed_pbft_block(&signing_key, period_one, 131);
        let period_one_rlp = vec![0xc1, 0x91];
        let period_one_transaction = test_transaction(
            0x91,
            sender,
            Some(receiver),
            0,
            U256::from(1u64),
            gas_price,
            50_000,
            vec![],
            period_one_rlp.clone(),
        );
        write_period_data(
            &storage,
            period_one,
            &period_one_pbft,
            std::slice::from_ref(&period_one_rlp),
        );
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![genesis_validator.clone()],
            genesis_dpos_config.clone(),
            rewards_config.clone(),
        )
        .unwrap();

        final_chain
            .finalize_block(
                period_one_pbft,
                vec![period_one_transaction.clone()],
                vec![FinalizationDagBlock {
                    author: dag_author,
                    difficulty: 0,
                    transaction_hashes: vec![period_one_transaction.hash],
                }],
            )
            .unwrap();
        assert_eq!(
            storage.metadata().block_rewards_stats_rlp().unwrap().len(),
            1
        );
        let validator_info = final_chain
            .call(dpos_call_request(
                period_one,
                get_validator_input(dag_author),
            ))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::zero()
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::zero()
        );
        drop(final_chain);

        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![genesis_validator],
            genesis_dpos_config,
            rewards_config,
        )
        .unwrap();
        let period_two_pbft = signed_pbft_block(&signing_key, period_two, 132);
        let period_two_rlp = vec![0xc1, 0x92];
        let period_two_transaction = test_transaction(
            0x92,
            sender,
            Some(receiver),
            1,
            U256::from(1u64),
            gas_price,
            50_000,
            vec![],
            period_two_rlp.clone(),
        );
        write_period_data(
            &storage,
            period_two,
            &period_two_pbft,
            std::slice::from_ref(&period_two_rlp),
        );

        final_chain
            .finalize_block(
                period_two_pbft,
                vec![period_two_transaction.clone()],
                vec![FinalizationDagBlock {
                    author: dag_author,
                    difficulty: 0,
                    transaction_hashes: vec![period_two_transaction.hash],
                }],
            )
            .unwrap();

        assert!(
            storage
                .metadata()
                .block_rewards_stats_rlp()
                .unwrap()
                .is_empty()
        );
        let validator_info = final_chain
            .call(dpos_call_request(
                period_two,
                get_validator_input(dag_author),
            ))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::from(VALUE_TRANSFER_GAS * 4)
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::from(VALUE_TRANSFER_GAS * 4)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_rejects_dpos_fee_reward_account_overflow_without_publishing_block() {
        let path = temp_db_path("finalize-dpos-fee-reward-overflow");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x28; 20];
        let receiver = [0x29; 20];
        let dag_author = [0x2A; 20];
        let genesis_validator = genesis_validator(dag_author, U256::from(10_000u64));
        let signing_key = SigningKey::from_slice(&[16u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 151);
        let transaction_rlp = vec![0xc1, 0x93];
        let transaction = test_transaction(
            0x93,
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
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![
                genesis_account(sender, U256::from(1_000_000u64)),
                genesis_account(DPOS_CONTRACT_ADDRESS, U256::MAX),
            ],
            vec![genesis_validator],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
            FinalChainRewardsConfig {
                committee_size: 100,
                magnolia_period: 0,
                aspen_part_one_period: u64::MAX,
                rewards_distribution_frequency: vec![(0, 1)],
                ..Default::default()
            },
        )
        .unwrap();

        let error = final_chain
            .finalize_block(
                pbft_block,
                vec![transaction.clone()],
                vec![FinalizationDagBlock {
                    author: dag_author,
                    difficulty: 0,
                    transaction_hashes: vec![transaction.hash],
                }],
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains("DPoS contract fee reward balance overflow"));
        assert_eq!(final_chain.last_block_number().unwrap(), 0);
        assert_eq!(balance_of(&final_chain, sender), U256::from(1_000_000u64));
        assert_eq!(balance_of(&final_chain, receiver), U256::zero());
        assert_eq!(balance_of(&final_chain, DPOS_CONTRACT_ADDRESS), U256::MAX);
        assert!(
            storage
                .metadata()
                .block_rewards_stats_rlp()
                .unwrap()
                .is_empty()
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_rejects_duplicate_rewards_cert_voters_without_publishing_block() {
        let path = temp_db_path("finalize-duplicate-cert-voters");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x27; 20];
        let genesis_validator = genesis_validator(validator, U256::from(10_000u64));
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[15u8; 32]).unwrap();
        let period = 1u64;
        let pbft_block = signed_pbft_block(&signing_key, period, 141);
        write_period_data(&storage, period, &pbft_block, &[]);
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            100_000,
            0,
            vec![],
            vec![genesis_validator],
            genesis_dpos_config,
            FinalChainRewardsConfig {
                committee_size: 100,
                magnolia_period: 0,
                aspen_part_one_period: u64::MAX,
                rewards_distribution_frequency: vec![(0, 2)],
                ..Default::default()
            },
        )
        .unwrap();

        let error = final_chain
            .finalize_block_with_rewards_facts(
                pbft_block,
                vec![],
                vec![],
                0,
                vec![
                    RewardCertVoteFact {
                        voter: validator.into(),
                        weight: 10,
                        period,
                    },
                    RewardCertVoteFact {
                        voter: validator.into(),
                        weight: 11,
                        period,
                    },
                ],
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains("REWARDS_STATS_DUPLICATE_VOTER"));
        assert_eq!(final_chain.last_block_number().unwrap(), 0);
        assert!(
            storage
                .metadata()
                .block_rewards_stats_rlp()
                .unwrap()
                .is_empty()
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn dpos_call_reads_current_snapshot_with_delegation_delay() {
        let path = temp_db_path("dpos-call-current-snapshot");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let sender = [0x31; 20];
        let receiver = [0x32; 20];
        let dag_author = [0x33; 20];
        let genesis_validator = genesis_validator(dag_author, U256::from(10_000u64));
        let signing_key = SigningKey::from_slice(&[13u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 122);
        let transaction_rlp = vec![0xc1, 0x86];
        let transaction = test_transaction(
            0xF7,
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
            vec![genesis_validator],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                delegation_delay: 5,
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
                    difficulty: 0,
                    transaction_hashes: vec![transaction.hash],
                }],
            )
            .unwrap();
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(dag_author)))
            .unwrap();

        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::from(receipt_fields(&receipts[0]).1 * 2)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_registers_dpos_validator_snapshot() {
        let path = temp_db_path("finalize-dpos-register-validator");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let owner = [0x31; 20];
        let validator = [0x32; 20];
        let vrf_key = [0xA5; 32];
        let stake = U256::from(5_000u64);
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[15u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 140);
        let transaction_rlp = vec![0xc1, 0x91];
        let input = register_validator_input(
            validator,
            &[0xCC; 65],
            vrf_key,
            10,
            "test validator",
            "test endpoint",
        );
        let transaction = test_transaction(
            0xA7,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            stake,
            U256::from(2u64),
            100_000,
            input,
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
            200_000,
            0,
            vec![genesis_account(owner, U256::from(1_000_000u64))],
            vec![],
            genesis_dpos_config.clone(),
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![transaction], vec![])
            .unwrap();

        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, DPOS_REGISTER_VALIDATOR_GAS, DPOS_REGISTER_VALIDATOR_GAS)
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, validator)
                .unwrap(),
            5
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            5
        );
        assert!(final_chain.dpos_is_eligible(period, validator).unwrap());
        assert_eq!(final_chain.vrf_key(validator).unwrap(), Some(vrf_key));
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(validator)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[32..64]),
            stake
        );
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[96..128]),
            U256::from(10u64)
        );
        assert_eq!(
            &validator_info.code_retval[192..224],
            &abi_word_from_address(owner)
        );

        drop(final_chain);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = FinalChain::new(
            storage.clone(),
            200_000,
            0,
            vec![genesis_account(owner, U256::from(1_000_000u64))],
            vec![],
            genesis_dpos_config,
        )
        .unwrap();
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, validator)
                .unwrap(),
            5
        );
        assert_eq!(final_chain.vrf_key(validator).unwrap(), Some(vrf_key));
        let facts = final_chain
            .dag_dpos_authorization_facts(period, validator)
            .unwrap();
        assert_eq!(facts.vrf_key, Some(vrf_key));
        assert_eq!(facts.sender_eligible_vote_count, 5);
        assert_eq!(facts.eligibility_status, DAG_VERIFY_DPOS_STATUS_ELIGIBLE);
        let genesis_facts = final_chain
            .dag_dpos_authorization_facts(0, validator)
            .unwrap();
        assert_eq!(genesis_facts.vrf_key, None);
        assert!(!genesis_facts.vrf_key_found);
        assert_eq!(
            genesis_facts.eligibility_status,
            DAG_VERIFY_DPOS_STATUS_NOT_CHECKED
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_delegate_action() {
        let path = temp_db_path("finalize-dpos-delegate");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let owner = [0x41; 20];
        let validator = [0x42; 20];
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[18u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 151);

        let register_input = register_validator_input(
            validator,
            &[0xCC; 65],
            [0xA5; 32],
            10,
            "delegate vote",
            "delegate endpoint",
        );
        let register_tx = test_transaction(
            0xA8,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::from(5_000u64),
            U256::from(2u64),
            100_000,
            register_input,
            vec![0xc1, 0x90],
        );

        let delegate_tx = test_transaction(
            0xA9,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::from(4_000u64),
            U256::from(2u64),
            100_000,
            delegate_input(validator),
            vec![0xc1, 0x91],
        );

        write_period_data(
            &storage,
            period,
            &pbft_block,
            &[register_tx.rlp.clone(), delegate_tx.rlp.clone()],
        );
        let final_chain = FinalChain::new(
            storage.clone(),
            200_000,
            0,
            vec![genesis_account(owner, U256::from(1_000_000u64))],
            vec![],
            genesis_dpos_config,
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![register_tx, delegate_tx], vec![])
            .unwrap();

        assert_eq!(receipts.len(), 2);
        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, DPOS_REGISTER_VALIDATOR_GAS, DPOS_REGISTER_VALIDATOR_GAS)
        );
        assert_eq!(
            receipt_fields(&receipts[1]),
            (
                1,
                DPOS_DELEGATE_GAS,
                DPOS_REGISTER_VALIDATOR_GAS + DPOS_DELEGATE_GAS
            )
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, validator)
                .unwrap(),
            9
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            9
        );
        assert_eq!(
            u256_from_big_endian(
                &final_chain
                    .call(dpos_call_request(period, get_validator_input(validator)))
                    .unwrap()
                    .code_retval[32..64],
            ),
            U256::from(9_000u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_undelegate_action() {
        let path = temp_db_path("finalize-dpos-undelegate");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let validator = [0x46; 20];
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[20u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 153);
        let undelegate_tx = test_transaction(
            0xBA,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::from(2u64),
            100_000,
            undelegate_input(validator, U256::from(3_000u64)),
            vec![0xc1, 0xba],
        );

        write_period_data(&storage, period, &pbft_block, &[undelegate_tx.rlp.clone()]);
        let final_chain = FinalChain::new(
            storage.clone(),
            200_000,
            0,
            vec![genesis_account(validator, U256::from(1_000_000u64))],
            vec![genesis_validator(validator, U256::from(10_000u64))],
            genesis_dpos_config,
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft_block, vec![undelegate_tx], vec![])
            .unwrap();

        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, DPOS_UNDELEGATE_GAS, DPOS_UNDELEGATE_GAS)
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, validator)
                .unwrap(),
            7
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            7
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_redelegate_action() {
        let path = temp_db_path("finalize-dpos-redelegate");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let first_validator = [0x44; 20];
        let second_validator = [0x45; 20];
        let owner = first_validator;
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[19u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&signing_key, period, 152);

        let mut txs = Vec::new();
        let tx_rlps = vec![vec![0xc1, 0xa1], vec![0xc1, 0xa2], vec![0xc1, 0xa3]];
        let register_input = register_validator_input(
            first_validator,
            &[0xCC; 65],
            [0xA5; 32],
            10,
            "first",
            "endpoint",
        );
        txs.push(test_transaction(
            0xB0,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::from(10_000u64),
            U256::from(2u64),
            100_000,
            register_input,
            tx_rlps[0].clone(),
        ));
        txs.push(test_transaction(
            0xB1,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::from(2_000u64),
            U256::from(2u64),
            100_000,
            register_validator_input(
                second_validator,
                &[0xDD; 65],
                [0xA6; 32],
                10,
                "second",
                "endpoint",
            ),
            tx_rlps[1].clone(),
        ));
        txs.push(test_transaction(
            0xB2,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            2,
            U256::zero(),
            U256::from(2u64),
            100_000,
            redelegate_input(first_validator, second_validator, U256::from(3_000u64)),
            tx_rlps[2].clone(),
        ));

        write_period_data(
            &storage,
            period,
            &pbft_block,
            &[txs[0].rlp.clone(), txs[1].rlp.clone(), txs[2].rlp.clone()],
        );
        let final_chain = FinalChain::new(
            storage.clone(),
            300_000,
            0,
            vec![genesis_account(owner, U256::from(2_000_000u64))],
            vec![],
            genesis_dpos_config,
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain.finalize_block(pbft_block, txs, vec![]).unwrap();

        assert_eq!(receipts.len(), 3);
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, first_validator)
                .unwrap(),
            7
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, second_validator)
                .unwrap(),
            5
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            12
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn persisted_corrupt_account_snapshot_rejects_startup() {
        let path = temp_db_path("corrupt-account-snapshot");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        storage
            .final_chain()
            .write_block_header_with_snapshots(1, H256::zero(), &[], &[], None, Some(&[0x01]))
            .unwrap();

        let err = match FinalChain::new(
            storage.clone(),
            0,
            0,
            vec![],
            vec![],
            GenesisDposConfig::default(),
        ) {
            Ok(_) => panic!("corrupt account snapshot should reject startup"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("account snapshot"));

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
