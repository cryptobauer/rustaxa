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
use crate::slashing::{
    VerifiedLegacyDoubleVotingProof, verify_legacy_double_voting_proof_call_data,
};
use anyhow::Result;
use ethereum_types::{H256, U256};
use keccak_hasher::KeccakHasher;
use rlp::Rlp;
use rustaxa_storage::{
    FINAL_CHAIN_BLOOM_INDEX_LEVELS, FINAL_CHAIN_BLOOM_INDEX_SIZE, FinalChainExecutionStatus,
    FinalChainLogBloom, FinalChainLogBloomIndexUpdate, FinalChainRewardsStatsUpdate,
    FinalChainTransactionIndexUpdate, StatusField, Storage, decode_final_chain_log_bloom_chunk,
    final_chain_log_bloom_chunk_id,
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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::Mutex;
use triehash::ordered_trie_root;

type DposDelegations = BTreeMap<[u8; 20], BTreeMap<[u8; 20], Vec<u8>>>;
type DposDelegationRewardCursors = BTreeMap<[u8; 20], BTreeMap<[u8; 20], Vec<u8>>>;
type DposDelegatorValidators = BTreeMap<[u8; 20], Vec<[u8; 20]>>;
type DposUndelegationsV2 = BTreeMap<[u8; 20], Vec<DposValidatorUndelegationsV2>>;

/// Pending V2 undelegations for one validator in legacy iterable-map order.
///
/// The Go precompile keeps a per-delegator validator iterable map and each
/// validator has its own iterable ID map. Removals use swap-remove ordering, so
/// Rust stores explicit vectors instead of sorted maps to preserve paged read
/// parity.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DposValidatorUndelegationsV2 {
    validator: [u8; 20],
    entries: Vec<DposUndelegationV2Entry>,
}

/// One pending V2 undelegation request.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DposUndelegationV2Entry {
    id: u64,
    amount: Vec<u8>,
    block: u64,
}

const EMPTY_TRIE_ROOT: [u8; 32] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];
const VALUE_TRANSFER_GAS: u64 = 21_000;
const CONTRACT_CREATION_ESTIMATE_GAS: u64 = 0x5dcc5;
const DPOS_DEFAULT_METHOD_GAS: u64 = 20_000;
const DPOS_GET_METHOD_GAS: u64 = 5_000;
pub(crate) const DPOS_CONTRACT_ADDRESS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
];
pub(crate) const SLASHING_CONTRACT_ADDRESS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xee,
];
const DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR: [u8; 4] = [0xde, 0x8e, 0x4b, 0x50];
const DPOS_GET_VALIDATOR_SELECTOR: [u8; 4] = [0x19, 0x04, 0xbb, 0x2e];
const DPOS_GET_DELEGATIONS_SELECTOR: [u8; 4] = [0x8b, 0x49, 0xd3, 0x94];
const DPOS_GET_UNDELEGATIONS_V2_SELECTOR: [u8; 4] = [0x78, 0xdf, 0x66, 0xe3];
const DPOS_GET_UNDELEGATION_V2_SELECTOR: [u8; 4] = [0xc1, 0x10, 0x7e, 0x27];
const DPOS_GET_TOTAL_DELEGATION_SELECTOR: [u8; 4] = [0xfc, 0x5e, 0x7e, 0x09];
const DPOS_DELEGATE_SELECTOR: [u8; 4] = [0x5c, 0x19, 0xa9, 0x5c];
const DPOS_UNDELEGATE_SELECTOR: [u8; 4] = [0x4d, 0x99, 0xdd, 0x16];
const DPOS_UNDELEGATE_V2_SELECTOR: [u8; 4] = [0xbd, 0x0e, 0x7f, 0xcc];
const DPOS_CONFIRM_UNDELEGATE_V2_SELECTOR: [u8; 4] = [0x78, 0x8d, 0x09, 0x74];
const DPOS_CANCEL_UNDELEGATE_V2_SELECTOR: [u8; 4] = [0xb6, 0xe1, 0xe3, 0x29];
const DPOS_REDELEGATE_SELECTOR: [u8; 4] = [0x70, 0x38, 0x12, 0xcc];
const DPOS_REGISTER_VALIDATOR_SELECTOR: [u8; 4] = [0xd6, 0xfd, 0xc1, 0x27];
const DPOS_CLAIM_REWARDS_SELECTOR: [u8; 4] = [0xef, 0x5c, 0xfb, 0x8c];
const DPOS_CLAIM_ALL_REWARDS_SELECTOR: [u8; 4] = [0x0b, 0x83, 0xa7, 0x27];
const DPOS_CLAIM_ALL_REWARDS_BATCH_SELECTOR: [u8; 4] = [0x09, 0xb7, 0x2e, 0x00];
const DPOS_CLAIM_COMMISSION_REWARDS_SELECTOR: [u8; 4] = [0xd0, 0xee, 0xbf, 0xe2];
const DPOS_SET_COMMISSION_SELECTOR: [u8; 4] = [0xf0, 0x00, 0x32, 0x2c];
const DPOS_SET_VALIDATOR_INFO_SELECTOR: [u8; 4] = [0x0b, 0xab, 0xea, 0x4c];
const DPOS_GET_VALIDATORS_SELECTOR: [u8; 4] = [0x19, 0xd8, 0x02, 0x4f];
const DPOS_GET_VALIDATORS_FOR_SELECTOR: [u8; 4] = [0x72, 0x4a, 0xc6, 0xb0];
const DPOS_DELEGATED_TOPIC: [u8; 32] = [
    0xe5, 0x54, 0x1a, 0x6b, 0x61, 0x03, 0xd4, 0xfa, 0x7e, 0x02, 0x1e, 0xd5, 0x4f, 0xad, 0x39, 0xc6,
    0x6f, 0x27, 0xa7, 0x6b, 0xd1, 0x3d, 0x37, 0x4c, 0xf6, 0x24, 0x0a, 0xe6, 0xbd, 0x0b, 0xb7, 0x2b,
];
const DPOS_UNDELEGATED_TOPIC: [u8; 32] = [
    0x4d, 0x10, 0xbd, 0x04, 0x97, 0x75, 0xc7, 0x7b, 0xd7, 0xf2, 0x55, 0x19, 0x5a, 0xfb, 0xa5, 0x08,
    0x80, 0x28, 0xec, 0xb3, 0xc7, 0xc2, 0x77, 0xd3, 0x93, 0xcc, 0xff, 0x79, 0x34, 0xf2, 0xf9, 0x2c,
];
const DPOS_UNDELEGATED_V2_TOPIC: [u8; 32] = [
    0xcf, 0xe7, 0xd7, 0x12, 0xcc, 0x67, 0xda, 0xf9, 0xa8, 0xd0, 0x0e, 0x8c, 0xca, 0x58, 0x81, 0x94,
    0x8b, 0xc5, 0x28, 0x98, 0x8f, 0xc3, 0x1a, 0x07, 0x1e, 0xff, 0xa1, 0xdb, 0xe6, 0xdc, 0x91, 0xef,
];
const DPOS_UNDELEGATE_CONFIRMED_V2_TOPIC: [u8; 32] = [
    0xa6, 0x37, 0xe5, 0x66, 0xd8, 0x25, 0x68, 0xef, 0xa4, 0xbd, 0x8c, 0x58, 0x8e, 0x17, 0x23, 0x2a,
    0xee, 0x48, 0x38, 0x73, 0xfa, 0x17, 0xfb, 0x87, 0x3f, 0x6d, 0x39, 0x8b, 0xa8, 0x5e, 0xd5, 0x7c,
];
const DPOS_UNDELEGATE_CANCELED_V2_TOPIC: [u8; 32] = [
    0xe0, 0x47, 0x45, 0x58, 0xd9, 0xb6, 0xee, 0x7a, 0x45, 0xf2, 0xd6, 0xd1, 0x2e, 0xff, 0xd2, 0x19,
    0x09, 0xb5, 0x33, 0x60, 0xeb, 0x73, 0xed, 0xa6, 0xcf, 0x0f, 0x19, 0x70, 0x31, 0x73, 0x8f, 0xee,
];
const DPOS_REDELEGATED_TOPIC: [u8; 32] = [
    0x12, 0xe1, 0x44, 0xc2, 0x7d, 0x0b, 0xad, 0x08, 0xab, 0xc7, 0x7c, 0x66, 0xa6, 0x40, 0xb5, 0xcf,
    0x15, 0xa0, 0x3a, 0x93, 0xf6, 0x58, 0x2f, 0x40, 0xde, 0x69, 0x32, 0xb0, 0x33, 0xa5, 0xfa, 0x5e,
];
const DPOS_REWARDS_CLAIMED_TOPIC: [u8; 32] = [
    0x93, 0x10, 0xcc, 0xfc, 0xb8, 0xde, 0x72, 0x3f, 0x57, 0x8a, 0x9e, 0x42, 0x82, 0xea, 0x9f, 0x52,
    0x1f, 0x05, 0xae, 0x40, 0xdc, 0x08, 0xf3, 0x06, 0x8d, 0xfa, 0xd5, 0x28, 0xa6, 0x5e, 0xe3, 0xc7,
];
const DPOS_COMMISSION_REWARDS_CLAIMED_TOPIC: [u8; 32] = [
    0xf0, 0xec, 0x9e, 0x0f, 0x6a, 0xdd, 0x85, 0x0a, 0x17, 0x38, 0xc5, 0x82, 0x22, 0x44, 0xe2, 0x6f,
    0xfc, 0x3d, 0x1f, 0x14, 0xda, 0x75, 0x37, 0xaa, 0x24, 0x05, 0x82, 0xb2, 0x5a, 0xf1, 0x2a, 0xd0,
];
const DPOS_COMMISSION_SET_TOPIC: [u8; 32] = [
    0xc9, 0x09, 0xda, 0xf7, 0x78, 0xd1, 0x80, 0xf4, 0x3d, 0xac, 0x53, 0xb5, 0x5d, 0x0d, 0xe9, 0x34,
    0xd2, 0xf1, 0xe0, 0xb7, 0x04, 0x12, 0xca, 0x27, 0x49, 0x82, 0xe4, 0xe6, 0xe8, 0x94, 0xeb, 0x1a,
];
const DPOS_VALIDATOR_INFO_SET_TOPIC: [u8; 32] = [
    0x7a, 0xa2, 0x0e, 0x1f, 0x59, 0x76, 0x4c, 0x90, 0x66, 0x57, 0x8f, 0xeb, 0xd6, 0x88, 0xa5, 0x13,
    0x75, 0xad, 0xbd, 0x65, 0x4a, 0xff, 0x86, 0xce, 0xf5, 0x65, 0x93, 0xa1, 0x7a, 0x99, 0x07, 0x1d,
];
const DPOS_VALIDATOR_REGISTERED_TOPIC: [u8; 32] = [
    0xd0, 0x95, 0x01, 0x34, 0x84, 0x73, 0x47, 0x4a, 0x20, 0xc7, 0x72, 0xc7, 0x9c, 0x65, 0x3e, 0x1f,
    0xd7, 0xe8, 0xb4, 0x37, 0xe4, 0x18, 0xfe, 0x23, 0x5d, 0x27, 0x7d, 0x2c, 0x88, 0x85, 0x32, 0x51,
];
const SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR: [u8; 4] = [0xfa, 0xc7, 0xc9, 0x4a];
const SLASHING_GET_JAIL_BLOCK_SELECTOR: [u8; 4] = [0x30, 0x1f, 0xd3, 0x8c];
const SLASHING_GET_JAILED_VALIDATORS_SELECTOR: [u8; 4] = [0x73, 0x9f, 0x30, 0xe2];
const SLASHING_JAILED_TOPIC: [u8; 32] = [
    0x91, 0x46, 0xfd, 0xb6, 0xf5, 0x6d, 0x90, 0x9a, 0xb9, 0x59, 0x90, 0x1f, 0xa5, 0x29, 0xb5, 0xb0,
    0x86, 0x15, 0xac, 0x9e, 0x70, 0x17, 0xc3, 0xa5, 0x5a, 0xe8, 0x60, 0xa9, 0xe9, 0x85, 0x7e, 0x6c,
];
const SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS: u64 = 20_000;
const SLASHING_GET_METHOD_GAS: u64 = 5_000;
const SLASHING_DOUBLE_VOTING_BEHAVIOUR: u8 = 1;
const DPOS_REGISTER_VALIDATOR_GAS: u64 = 80_000;
const DPOS_DELEGATE_GAS: u64 = 40_000;
const DPOS_UNDELEGATE_GAS: u64 = 60_000;
const DPOS_REDELEGATE_GAS: u64 = 80_000;
const DPOS_CLAIM_REWARDS_GAS: u64 = 40_000;
const DPOS_CLAIM_COMMISSION_REWARDS_GAS: u64 = 20_000;
const DPOS_SET_COMMISSION_GAS: u64 = 20_000;
const DPOS_SET_VALIDATOR_INFO_GAS: u64 = 20_000;
const DPOS_BATCH_GET_REWARDS_GAS: u64 = 5_000;
const DPOS_GET_DELEGATIONS_MAX_COUNT: usize = 20;
const DPOS_GET_UNDELEGATIONS_MAX_COUNT: usize = 20;
const DPOS_GET_VALIDATORS_MAX_COUNT: usize = 20;
const DPOS_CLAIM_ALL_REWARDS_MAX_COUNT: usize = 10;
const DPOS_MAX_COMMISSION: u16 = 10_000;
const DPOS_MAX_DESCRIPTION_LENGTH: usize = 100;
const DPOS_MAX_ENDPOINT_LENGTH: usize = 50;
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
    dpos_commission_change_delta: u16,
    dpos_commission_change_frequency: u32,
    dpos_delegation_delay: u64,
    dpos_delegation_locking_period: u64,
    dpos_cornus_period: u64,
    dpos_cornus_delegation_locking_period: u64,
    dpos_cacti_period: u64,
    dpos_cacti_delegation_locking_period: u64,
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
    /// Validator insertion order used by legacy-compatible paged DPoS reads.
    validator_order: Vec<[u8; 20]>,
    /// Delegator-to-validator iteration order used by DPoS paged reads.
    delegator_validators: DposDelegatorValidators,
    /// F1 reward cursor by validator and delegator at this block.
    ///
    /// Each cursor is the validator's cumulative rewards-per-one-stake value
    /// at the delegator's last stake mutation. Read-only reward queries
    /// subtract it from the validator's current cumulative value without
    /// claiming or mutating reward pools.
    delegation_reward_cursors: DposDelegationRewardCursors,
    /// Current F1 cumulative rewards-per-one-stake by validator.
    validator_reward_per_stake: BTreeMap<[u8; 20], Vec<u8>>,
    /// Aspen part-one minted-token counter at this block.
    minted_tokens: Vec<u8>,
    /// Aspen part-two total supply at this block.
    ///
    /// Empty bytes mean the Go-compatible lazy migration has not happened yet.
    total_supply: Vec<u8>,
    /// Aspen part-two yield fraction scaled by `ASPEN_YIELD_PRECISION`.
    current_yield: u64,
    /// Pending V2 undelegations by delegator, preserving legacy iterable-map order.
    undelegations_v2: DposUndelegationsV2,
    /// Last assigned V2 undelegation ID by delegator.
    undelegation_v2_last_ids: BTreeMap<[u8; 20], u64>,
    /// Jail end block by validator address for Rust-executed slashing proofs.
    ///
    /// Entries remain after list cleanup so `getJailBlock(address)` can return
    /// the last persisted jail block just like the legacy storage field.
    slashing_jail_blocks: BTreeMap<[u8; 20], u64>,
    /// Legacy-order jailed validator list after end-block cleanup.
    slashing_jailed_validators: Vec<[u8; 20]>,
    /// Canonical double-voting proof keys already committed by Rust execution.
    slashing_double_voting_proofs: BTreeSet<[u8; 32]>,
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

    /// Returns the configured block gas limit used when materializing legacy
    /// FinalChain block headers.
    ///
    /// The value is immutable for the runtime. External-EVM publication
    /// planning uses it to derive header RLP and hash bytes without taking
    /// ownership of EVM execution or storage commit.
    pub(crate) fn block_gas_limit(&self) -> u64 {
        self.block_gas_limit
    }

    /// Returns the genesis timestamp used by genesis header materialization.
    ///
    /// Non-genesis PBFT metadata supplies its own timestamp; this accessor keeps
    /// the external-EVM publication planner on the same codec path as native
    /// Rust finalization.
    pub(crate) fn genesis_timestamp(&self) -> u64 {
        self.genesis_timestamp
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
        let genesis_validator_order = genesis_validators
            .iter()
            .map(|validator| validator.address)
            .collect::<Vec<_>>();
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
        let genesis_dpos_delegation_reward_cursors = genesis_dpos_delegations
            .iter()
            .map(|(validator, delegations)| {
                (
                    *validator,
                    delegations
                        .keys()
                        .map(|delegator| (*delegator, Vec::new()))
                        .collect::<BTreeMap<_, _>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let genesis_dpos_delegator_validators =
            delegator_validators_from_delegations(&genesis_dpos_delegations);
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
        let dpos_delegation_locking_period = rewards_config.dpos_delegation_locking_period;
        let dpos_cornus_period = rewards_config.cornus_period;
        let dpos_cornus_delegation_locking_period = rewards_config.cornus_delegation_locking_period;
        let dpos_cacti_period = rewards_config.cacti_period;
        let dpos_cacti_delegation_locking_period = rewards_config.cacti_delegation_locking_period;
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
            dpos_commission_change_delta: genesis_dpos_config.commission_change_delta,
            dpos_commission_change_frequency: genesis_dpos_config.commission_change_frequency,
            dpos_delegation_delay: genesis_dpos_config.delegation_delay,
            dpos_delegation_locking_period,
            dpos_cornus_period,
            dpos_cornus_delegation_locking_period,
            dpos_cacti_period,
            dpos_cacti_delegation_locking_period,
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
                    validator_order: genesis_validator_order,
                    delegator_validators: genesis_dpos_delegator_validators,
                    delegation_reward_cursors: genesis_dpos_delegation_reward_cursors,
                    validator_reward_per_stake: BTreeMap::new(),
                    minted_tokens: Vec::new(),
                    total_supply: Vec::new(),
                    current_yield: 0,
                    undelegations_v2: BTreeMap::new(),
                    undelegation_v2_last_ids: BTreeMap::new(),
                    slashing_jail_blocks: BTreeMap::new(),
                    slashing_jailed_validators: Vec::new(),
                    slashing_double_voting_proofs: BTreeSet::new(),
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

    /// Returns the FinalChain hash that a PBFT block for `period` must carry.
    ///
    /// This preserves the legacy PBFT/FinalChain delay contract used by the C++
    /// manager: periods less than or equal to the configured delegation delay
    /// use the zero hash, while later periods use the finalized header hash at
    /// `period - delegation_delay`. A missing delayed header is returned as
    /// `None` so PBFT callers can treat it as a typed "not finalized yet"
    /// condition instead of an infrastructure error.
    pub fn pbft_final_chain_hash(&self, period: u64) -> Result<Option<[u8; 32]>, anyhow::Error> {
        if period <= self.dpos_delegation_delay {
            return Ok(Some([0; 32]));
        }

        let lookup_block = period - self.dpos_delegation_delay;
        let Some(hash) = self.block_hash(lookup_block)? else {
            return Ok(None);
        };
        anyhow::ensure!(
            hash.len() == 32,
            "final_chain_blk_hash_by_number/{lookup_block} has invalid hash length: expected 32, got {}",
            hash.len()
        );
        let mut out = [0u8; 32];
        out.copy_from_slice(&hash);
        Ok(Some(out))
    }

    /// Returns finalized block numbers whose indexed bloom contains `bloom`.
    ///
    /// Inputs are the query bloom and inclusive block-number range. The lookup
    /// follows the legacy two-level FinalChain bloom index exactly: missing
    /// chunks decode as zero chunks, malformed persisted chunks are errors, and
    /// a stored bloom matches when every bit in the query bloom is present.
    pub fn with_block_bloom(
        &self,
        bloom: &FinalChainLogBloom,
        from: u64,
        to: u64,
    ) -> Result<Vec<u64>, anyhow::Error> {
        if from > to {
            return Ok(Vec::new());
        }

        let root_level = FINAL_CHAIN_BLOOM_INDEX_LEVELS - 1;
        let root_units = final_chain_bloom_index_units(FINAL_CHAIN_BLOOM_INDEX_LEVELS)?;
        let first_index = from / root_units;
        let last_index = to / root_units + u64::from(!to.is_multiple_of(root_units));
        let mut result = Vec::new();
        for index in first_index..=last_index {
            self.with_block_bloom_at(bloom, from, to, root_level, index, &mut result)?;
        }
        Ok(result)
    }

    fn with_block_bloom_at(
        &self,
        bloom: &FinalChainLogBloom,
        from: u64,
        to: u64,
        level: u64,
        index: u64,
        result: &mut Vec<u64>,
    ) -> Result<(), anyhow::Error> {
        let course_units = final_chain_bloom_index_units(level + 1)?;
        let fine_units = final_chain_bloom_index_units(level)?;
        let range_start = index
            .checked_mul(course_units)
            .ok_or_else(|| anyhow::anyhow!("final-chain bloom query range overflow"))?;
        let range_end = index
            .checked_add(1)
            .and_then(|value| value.checked_mul(course_units))
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| anyhow::anyhow!("final-chain bloom query range overflow"))?;

        if range_start > to || range_end < from {
            return Ok(());
        }

        let offset_begin = if from > range_start {
            (from - range_start) / fine_units
        } else {
            0
        };
        let offset_end = if to < range_end {
            (to - range_start) / fine_units
        } else {
            FINAL_CHAIN_BLOOM_INDEX_SIZE as u64 - 1
        };
        let chunk_id = final_chain_log_bloom_chunk_id(level, index)?;
        let raw = self.storage.final_chain().log_blooms_chunk_raw(chunk_id)?;
        let chunk = decode_final_chain_log_bloom_chunk(raw.as_deref())?;

        for offset in offset_begin..=offset_end {
            let slot = offset as usize;
            if !log_bloom_contains(&chunk[slot], bloom) {
                continue;
            }
            let child_index = index
                .checked_mul(FINAL_CHAIN_BLOOM_INDEX_SIZE as u64)
                .and_then(|value| value.checked_add(offset))
                .ok_or_else(|| anyhow::anyhow!("final-chain bloom child index overflow"))?;
            if level == 0 {
                result.push(child_index);
            } else {
                self.with_block_bloom_at(bloom, from, to, level - 1, child_index, result)?;
            }
        }
        Ok(())
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
        let snapshot = self.dpos_snapshot(block_number)?;
        Ok(self.dpos_effective_vote_count(&snapshot, block_number, address))
    }

    /// Returns the total DPoS eligible vote count at a block.
    pub fn dpos_eligible_total_vote_count(&self, block_number: u64) -> Result<u64, anyhow::Error> {
        let snapshot = self.dpos_snapshot(block_number)?;
        self.dpos_effective_total_vote_count(&snapshot, block_number)
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

        let sender_eligible_vote_count =
            self.dpos_effective_vote_count(&snapshot, block_number, sender);
        let vdf_sortition_max_vote_count =
            if block_number < self.dag_vdf_sortition_total_vote_count_until_period {
                self.dpos_effective_total_vote_count(&snapshot, block_number)?
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
        let snapshot = self.dpos_snapshot(block_number)?;
        Ok(snapshot
            .vote_counts
            .keys()
            .map(|address| {
                let vote_count = self.dpos_effective_vote_count(&snapshot, block_number, *address);
                (*address, vote_count)
            })
            .filter(|(_, vote_count)| *vote_count > 0)
            .map(|(address, vote_count)| DposValidatorVoteCount {
                address,
                vote_count,
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

        if request.receiver == Some(SLASHING_CONTRACT_ADDRESS) {
            return self.slashing_call(request);
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

        if request.input.len() < 4 {
            return Ok(FinalChainCallOutcome {
                gas_used: 0,
                code_err: "Rust FinalChain::call DPoS input is missing selector".to_string(),
                ..Default::default()
            });
        }

        let mut selector = [0u8; 4];
        selector.copy_from_slice(&request.input[..4]);
        let gas_used = self.dpos_call_required_gas(&request, selector)?;
        if request.gas_limit < gas_used {
            return Ok(FinalChainCallOutcome {
                gas_used: request.gas_limit,
                code_err: "out of gas".to_string(),
                ..Default::default()
            });
        }
        let code_retval = match selector {
            DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR => {
                let snapshot = self.dpos_snapshot_at_finalized_block(request.block_number)?;
                abi_word_from_u64(
                    self.dpos_effective_total_vote_count(&snapshot, request.block_number)?,
                )
                .to_vec()
            }
            DPOS_GET_VALIDATOR_SELECTOR => {
                let validator =
                    decode_abi_address_argument(&request.input, "getValidator(address)")?;
                self.encode_dpos_validator(request.block_number, validator)?
            }
            DPOS_GET_VALIDATORS_SELECTOR => {
                let batch = decode_abi_u32_argument(&request.input, 4, "getValidators(uint32)")?;
                self.encode_dpos_validators(request.block_number, batch)?
            }
            DPOS_GET_VALIDATORS_FOR_SELECTOR => {
                let owner = decode_abi_address_argument_with_offset(
                    &request.input,
                    4,
                    "getValidatorsFor owner",
                )?;
                let batch = decode_abi_u32_argument(&request.input, 36, "getValidatorsFor batch")?;
                self.encode_dpos_validators_for(request.block_number, owner, batch)?
            }
            DPOS_GET_TOTAL_DELEGATION_SELECTOR => {
                let delegator =
                    decode_abi_address_argument(&request.input, "getTotalDelegation(address)")?;
                self.encode_dpos_total_delegation(request.block_number, delegator)?
            }
            DPOS_GET_DELEGATIONS_SELECTOR => {
                let delegator =
                    decode_abi_address_argument(&request.input, "getDelegations(address,uint32)")?;
                let batch = decode_abi_u32_argument(
                    &request.input,
                    36,
                    "getDelegations(address,uint32) batch",
                )?;
                self.encode_dpos_delegations(request.block_number, delegator, batch)?
            }
            DPOS_GET_UNDELEGATIONS_V2_SELECTOR => {
                if !self.is_on_cornus(request.block_number) {
                    return Ok(FinalChainCallOutcome {
                        gas_used,
                        code_err: "Method not supported".to_string(),
                        ..Default::default()
                    });
                }
                let delegator = decode_abi_address_argument(
                    &request.input,
                    "getUndelegationsV2(address,uint32)",
                )?;
                let batch = decode_abi_u32_argument(
                    &request.input,
                    36,
                    "getUndelegationsV2(address,uint32) batch",
                )?;
                self.encode_dpos_undelegations_v2(request.block_number, delegator, batch)?
            }
            DPOS_GET_UNDELEGATION_V2_SELECTOR => {
                if !self.is_on_cornus(request.block_number) {
                    return Ok(FinalChainCallOutcome {
                        gas_used,
                        code_err: "Method not supported".to_string(),
                        ..Default::default()
                    });
                }
                let delegator = decode_abi_address_argument_with_offset(
                    &request.input,
                    4,
                    "getUndelegationV2 delegator",
                )?;
                let validator = decode_abi_address_argument_with_offset(
                    &request.input,
                    36,
                    "getUndelegationV2 validator",
                )?;
                let id = decode_abi_word_as_u64(&request.input, 68, "getUndelegationV2 id")?;
                match self.encode_dpos_undelegation_v2(
                    request.block_number,
                    delegator,
                    validator,
                    id,
                )? {
                    Some(output) => output,
                    None => {
                        return Ok(FinalChainCallOutcome {
                            gas_used,
                            code_err: "Undelegation does not exist".to_string(),
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {
                return Ok(FinalChainCallOutcome {
                    gas_used,
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
            gas_used,
            ..Default::default()
        })
    }

    fn slashing_call(
        &self,
        request: FinalChainCallRequest,
    ) -> Result<FinalChainCallOutcome, anyhow::Error> {
        if request.input.len() < 4 {
            return Ok(FinalChainCallOutcome {
                gas_used: 0,
                code_err: "Rust FinalChain::call slashing input is missing selector".to_string(),
                ..Default::default()
            });
        }
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&request.input[..4]);
        let gas_used = match selector {
            SLASHING_GET_JAIL_BLOCK_SELECTOR | SLASHING_GET_JAILED_VALIDATORS_SELECTOR => {
                SLASHING_GET_METHOD_GAS
            }
            SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR => SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS,
            _ => 0,
        };
        if request.gas_limit < gas_used {
            return Ok(FinalChainCallOutcome {
                gas_used: request.gas_limit,
                code_err: "out of gas".to_string(),
                ..Default::default()
            });
        }
        let snapshot = self.dpos_snapshot(request.block_number)?;
        let code_retval = match selector {
            SLASHING_GET_JAIL_BLOCK_SELECTOR => {
                let validator =
                    decode_abi_address_argument(&request.input, "getJailBlock(address)")?;
                abi_word_from_u64(
                    snapshot
                        .slashing_jail_blocks
                        .get(&validator)
                        .copied()
                        .unwrap_or_default(),
                )
                .to_vec()
            }
            SLASHING_GET_JAILED_VALIDATORS_SELECTOR => {
                encode_abi_address_array(&snapshot.slashing_jailed_validators)
            }
            _ => {
                return Ok(FinalChainCallOutcome {
                    gas_used,
                    code_err: format!(
                        "Rust FinalChain::call unsupported slashing selector 0x{}",
                        selector_hex(selector)
                    ),
                    ..Default::default()
                });
            }
        };
        Ok(FinalChainCallOutcome {
            code_retval,
            gas_used,
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

        let NativeExecution {
            accounts: mut account_snapshot,
            mut receipts,
            gas_used,
            transaction_fees,
            contract_transactions,
        } = self.execute_native_transactions(pbft.period, &transactions)?;
        let pre_magnolia_fee_reward_period = self.pre_magnolia_fee_reward_period(pbft.period);
        let rewards_stats_plan = self.native_rewards_stats_plan(
            &pbft,
            &transactions,
            &finalized_dag_blocks,
            &transaction_fees,
            blocks_per_year,
            cert_votes,
        )?;
        let dpos_fee_rewards = if pre_magnolia_fee_reward_period {
            BTreeMap::new()
        } else {
            rewards_stats_plan.fee_rewards_by_validator.clone()
        };
        let mut dpos_snapshot = self.plan_dpos_snapshot(
            pbft.period,
            contract_transactions,
            &mut account_snapshot,
            &mut receipts,
        )?;
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
                total_transaction_fees(&transaction_fees)?,
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
        let encoded_receipts = encode_native_receipts(&receipts);
        let receipts_rlp = encode_receipts_rlp(&encoded_receipts);
        let parent_hash = self
            .block_hash(self.last_block_number()?)?
            .map(|bytes| h256_from_slice(&bytes, "parent final-chain hash"))
            .transpose()?
            .unwrap_or_default();
        let header_log_bloom = block_log_bloom(&receipts);
        let mut indexed_log_bloom = [0u8; 256];
        indexed_log_bloom.copy_from_slice(&header_log_bloom);
        add_bloom_value(&mut indexed_log_bloom, pbft.author.as_bytes());
        let stored_header = StoredFinalChainBlockHeader {
            parent_hash,
            state_root: synthetic_state_root(pbft.period),
            transactions_root: ordered_root(
                transactions
                    .iter()
                    .map(|transaction| transaction.rlp.as_slice()),
            ),
            receipts_root: ordered_root(encoded_receipts.iter().map(|receipt| receipt.as_slice())),
            log_bloom: header_log_bloom,
            gas_used,
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
        let transaction_index_updates = transactions
            .iter()
            .enumerate()
            .map(|(position, transaction)| FinalChainTransactionIndexUpdate {
                transaction_hash: H256::from(transaction.hash),
                position: position as u32,
                is_system: false,
                receipt_rlp: encoded_receipts[position].as_slice(),
            })
            .collect::<Vec<_>>();
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
                Some(FinalChainLogBloomIndexUpdate {
                    block_number: pbft.period,
                    bloom: &indexed_log_bloom,
                }),
                &transaction_index_updates,
            )?;
        self.insert_account_snapshot(pbft.period, account_snapshot)?;
        self.insert_dpos_snapshot(pbft.period, dpos_snapshot)?;
        self.commit_rewards_stats_runtime(rewards_stats_plan.runtime_after_commit)?;

        Ok((full_header.into_vec(), encoded_receipts))
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

    fn current_validator_reward_per_stake(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
    ) -> Result<U256, anyhow::Error> {
        let base = snapshot
            .validator_reward_per_stake
            .get(&validator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        let rewards_pool = snapshot
            .delegator_rewards
            .get(&validator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        let stake = snapshot
            .total_stakes
            .get(&validator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        if rewards_pool.is_zero() || stake.is_zero() {
            return Ok(base);
        }
        base.checked_add(self.reward_per_stake(rewards_pool, stake)?)
            .ok_or_else(|| anyhow::anyhow!("validator reward-per-stake overflow"))
    }

    fn reward_per_stake(&self, rewards_pool: U256, stake: U256) -> Result<U256, anyhow::Error> {
        anyhow::ensure!(
            !stake.is_zero(),
            "DPoS reward-per-stake calculation requires nonzero stake"
        );
        rewards_pool
            .checked_mul(u256_from_big_endian(&self.dpos_validator_maximum_stake))
            .ok_or_else(|| anyhow::anyhow!("DPoS reward-per-stake multiplication overflow"))
            .map(|value| value / stake)
    }

    fn delegator_reward_from_per_stake(
        &self,
        reward_per_stake: U256,
        stake: U256,
    ) -> Result<U256, anyhow::Error> {
        if reward_per_stake.is_zero() || stake.is_zero() {
            return Ok(U256::zero());
        }
        reward_per_stake
            .checked_mul(stake)
            .ok_or_else(|| anyhow::anyhow!("DPoS delegator reward multiplication overflow"))
            .map(|value| value / u256_from_big_endian(&self.dpos_validator_maximum_stake))
    }

    fn checkpoint_validator_reward_per_stake(
        &self,
        snapshot: &mut DposSnapshot,
        validator: [u8; 20],
    ) -> Result<U256, anyhow::Error> {
        let reward_per_stake = self.current_validator_reward_per_stake(snapshot, validator)?;
        snapshot
            .validator_reward_per_stake
            .insert(validator, u256_to_big_endian(reward_per_stake));
        snapshot.delegator_rewards.insert(validator, Vec::new());
        Ok(reward_per_stake)
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
        block_number: u64,
        transactions: &[FinalizationTransaction],
    ) -> Result<NativeExecution, anyhow::Error> {
        let mut accounts = self.current_account_snapshot()?;
        let mut receipts = Vec::with_capacity(transactions.len());
        let mut transaction_fees = Vec::with_capacity(transactions.len());
        let mut contract_transactions = Vec::new();
        let mut cumulative_gas_used = 0u64;
        let mut dpos_gas_snapshot = self.dpos_snapshot(self.last_block_number()?)?;

        for (position, transaction) in transactions.iter().enumerate() {
            let mut contract_transaction = if transaction.receiver == Some(DPOS_CONTRACT_ADDRESS) {
                Some(NativeContractTransaction::Dpos(decode_dpos_transaction(
                    &transaction.data,
                    transaction.sender,
                    block_number,
                    self.rewards_config.fix_claim_all_block_num,
                    self.dpos_cornus_period,
                )?))
            } else if transaction.receiver == Some(SLASHING_CONTRACT_ADDRESS) {
                Some(NativeContractTransaction::Slashing(
                    decode_slashing_transaction(&transaction.data)?,
                ))
            } else if !transaction.data.is_empty() || transaction.receiver.is_none() {
                anyhow::bail!(
                    "Rust FinalChain::finalize currently supports only native value transfers and selected DPoS/slashing actions"
                );
            } else {
                None
            };
            let gas_price = u256_from_big_endian(&transaction.gas_price);
            let value = u256_from_big_endian(&transaction.value);
            if let Some(NativeContractTransaction::Dpos(dpos_tx)) = contract_transaction.as_mut() {
                match dpos_tx {
                    DposTransaction::Register(registration) => {
                        registration.stake = u256_to_big_endian(value);
                    }
                    DposTransaction::Delegate { amount, .. } => {
                        *amount = u256_to_big_endian(value);
                    }
                    DposTransaction::Undelegate { .. }
                    | DposTransaction::UndelegateV2 { .. }
                    | DposTransaction::ConfirmUndelegateV2 { .. }
                    | DposTransaction::CancelUndelegateV2 { .. }
                    | DposTransaction::Redelegate { .. }
                    | DposTransaction::ClaimRewards { .. }
                    | DposTransaction::ClaimCommissionRewards { .. }
                    | DposTransaction::SetValidatorInfo { .. }
                    | DposTransaction::SetCommission { .. }
                    | DposTransaction::ClaimAllRewards { .. }
                    | DposTransaction::MethodNotSupported => {}
                }
            }
            let contract_nonpayable_value_failure =
                contract_transaction.as_ref().is_some_and(|contract_tx| {
                    !value.is_zero()
                        && !matches!(
                            contract_tx,
                            NativeContractTransaction::Dpos(
                                DposTransaction::Register(_) | DposTransaction::Delegate { .. }
                            )
                        )
                });
            let required_gas = if let Some(contract_transaction) = contract_transaction.as_ref() {
                match contract_transaction {
                    NativeContractTransaction::Dpos(dpos_transaction) => match dpos_transaction {
                        DposTransaction::Register(_) => DPOS_REGISTER_VALIDATOR_GAS,
                        DposTransaction::Delegate { .. } => DPOS_DELEGATE_GAS,
                        DposTransaction::Undelegate { .. } => DPOS_UNDELEGATE_GAS,
                        DposTransaction::UndelegateV2 { .. } => DPOS_UNDELEGATE_GAS,
                        DposTransaction::ConfirmUndelegateV2 { .. } => DPOS_DEFAULT_METHOD_GAS,
                        DposTransaction::CancelUndelegateV2 { .. } => DPOS_UNDELEGATE_GAS,
                        DposTransaction::Redelegate { .. } => DPOS_REDELEGATE_GAS,
                        DposTransaction::ClaimRewards { .. } => DPOS_CLAIM_REWARDS_GAS,
                        DposTransaction::ClaimCommissionRewards { .. } => {
                            DPOS_CLAIM_COMMISSION_REWARDS_GAS
                        }
                        DposTransaction::SetValidatorInfo { .. } => DPOS_SET_VALIDATOR_INFO_GAS,
                        DposTransaction::SetCommission { .. } => DPOS_SET_COMMISSION_GAS,
                        DposTransaction::ClaimAllRewards { delegator, batch } => {
                            let claim_items = dpos_claim_all_rewards_item_count(
                                &dpos_gas_snapshot,
                                *delegator,
                                *batch,
                            )?;
                            let per_item_gas = DPOS_CLAIM_REWARDS_GAS
                                .checked_add(DPOS_BATCH_GET_REWARDS_GAS)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("claimAllRewards per-item gas overflow")
                                })?;
                            claim_items.checked_mul(per_item_gas).ok_or_else(|| {
                                anyhow::anyhow!("claimAllRewards gas multiplication overflow")
                            })?
                        }
                        DposTransaction::MethodNotSupported => 0,
                    },
                    NativeContractTransaction::Slashing(slashing_transaction) => {
                        match slashing_transaction {
                            SlashingTransaction::CommitDoubleVotingProof(_) => {
                                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS
                            }
                            SlashingTransaction::MethodNotSupported => 0,
                        }
                    }
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
                    let charged_value = if contract_nonpayable_value_failure {
                        U256::zero()
                    } else {
                        value
                    };
                    let total_cost = gas_cost
                        .checked_add(charged_value)
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
                if contract_nonpayable_value_failure {
                    status_code = 0;
                } else if let Some(contract_tx) = contract_transaction {
                    if let NativeContractTransaction::Dpos(dpos_tx) = &contract_tx {
                        if matches!(
                            dpos_tx,
                            DposTransaction::Register(_) | DposTransaction::Delegate { .. }
                        ) && !value.is_zero()
                        {
                            let dpos_account = accounts
                                .entry(DPOS_CONTRACT_ADDRESS)
                                .or_insert_with(empty_account);
                            let current_contract_balance =
                                u256_from_big_endian(&dpos_account.balance);
                            dpos_account.balance = u256_to_big_endian(
                                current_contract_balance.checked_add(value).ok_or_else(|| {
                                    anyhow::anyhow!("DPoS contract balance overflow")
                                })?,
                            );
                        }
                        update_dpos_claim_gas_snapshot(&mut dpos_gas_snapshot, dpos_tx)?;
                    }
                    contract_transactions.push((position, contract_tx));
                } else {
                    let receiver_address = transaction.receiver.ok_or_else(|| {
                        anyhow::anyhow!("native value transfer missing receiver after validation")
                    })?;
                    if receiver_address == DPOS_CONTRACT_ADDRESS
                        || receiver_address == SLASHING_CONTRACT_ADDRESS
                    {
                        anyhow::bail!(
                            "Rust FinalChain::finalize unsupported native precompile transaction selector"
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
            receipts.push(NativeReceipt {
                status_code,
                gas_used,
                cumulative_gas_used,
                logs: Vec::new(),
                new_contract_address: None,
            });
            transaction_fees.push((transaction.hash, gas_cost));
        }

        Ok(NativeExecution {
            accounts,
            receipts,
            gas_used: cumulative_gas_used,
            transaction_fees,
            contract_transactions,
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

    fn dpos_call_required_gas(
        &self,
        request: &FinalChainCallRequest,
        selector: [u8; 4],
    ) -> Result<u64, anyhow::Error> {
        match selector {
            DPOS_GET_TOTAL_ELIGIBLE_VOTES_SELECTOR => Ok(DPOS_DEFAULT_METHOD_GAS),
            DPOS_GET_VALIDATOR_SELECTOR => Ok(DPOS_GET_METHOD_GAS),
            DPOS_GET_VALIDATORS_SELECTOR => {
                let snapshot = self.dpos_snapshot_at_finalized_block(request.block_number)?;
                let batch = decode_abi_u32_argument(&request.input, 4, "getValidators(uint32)")?;
                let items = dpos_batch_items_count(
                    snapshot.validator_order.len() as u64,
                    batch,
                    DPOS_GET_VALIDATORS_MAX_COUNT as u64,
                )?;
                items
                    .checked_mul(DPOS_BATCH_GET_REWARDS_GAS)
                    .ok_or_else(|| anyhow::anyhow!("getValidators gas multiplication overflow"))
            }
            DPOS_GET_VALIDATORS_FOR_SELECTOR => {
                self.dpos_snapshot_at_finalized_block(request.block_number)?;
                decode_abi_address_argument_with_offset(
                    &request.input,
                    4,
                    "getValidatorsFor owner",
                )?;
                decode_abi_u32_argument(&request.input, 36, "getValidatorsFor batch")?;
                (DPOS_GET_VALIDATORS_MAX_COUNT as u64)
                    .checked_mul(DPOS_BATCH_GET_REWARDS_GAS)
                    .ok_or_else(|| anyhow::anyhow!("getValidatorsFor gas multiplication overflow"))
            }
            DPOS_GET_TOTAL_DELEGATION_SELECTOR => {
                let snapshot = self.dpos_snapshot_at_finalized_block(request.block_number)?;
                let delegator =
                    decode_abi_address_argument(&request.input, "getTotalDelegation(address)")?;
                let count = snapshot
                    .delegator_validators
                    .get(&delegator)
                    .map(Vec::len)
                    .unwrap_or_default() as u64;
                count
                    .checked_mul(DPOS_BATCH_GET_REWARDS_GAS)
                    .ok_or_else(|| {
                        anyhow::anyhow!("getTotalDelegation gas multiplication overflow")
                    })
            }
            DPOS_GET_DELEGATIONS_SELECTOR => {
                let snapshot = self.dpos_snapshot_at_finalized_block(request.block_number)?;
                let delegator =
                    decode_abi_address_argument(&request.input, "getDelegations(address,uint32)")?;
                let batch = decode_abi_u32_argument(
                    &request.input,
                    36,
                    "getDelegations(address,uint32) batch",
                )?;
                let count = snapshot
                    .delegator_validators
                    .get(&delegator)
                    .map(Vec::len)
                    .unwrap_or_default() as u64;
                let items =
                    dpos_batch_items_count(count, batch, DPOS_GET_DELEGATIONS_MAX_COUNT as u64)?;
                items
                    .checked_mul(DPOS_BATCH_GET_REWARDS_GAS)
                    .ok_or_else(|| anyhow::anyhow!("getDelegations gas multiplication overflow"))
            }
            DPOS_GET_UNDELEGATIONS_V2_SELECTOR => {
                if !self.is_on_cornus(request.block_number) {
                    return Ok(0);
                }
                let snapshot = self.dpos_snapshot_at_finalized_block(request.block_number)?;
                let delegator = decode_abi_address_argument(
                    &request.input,
                    "getUndelegationsV2(address,uint32)",
                )?;
                let batch = decode_abi_u32_argument(
                    &request.input,
                    36,
                    "getUndelegationsV2(address,uint32) batch",
                )?;
                let storage_reads =
                    dpos_undelegations_v2_storage_read_count(&snapshot, delegator, batch)?;
                storage_reads
                    .checked_mul(DPOS_BATCH_GET_REWARDS_GAS)
                    .ok_or_else(|| {
                        anyhow::anyhow!("getUndelegationsV2 gas multiplication overflow")
                    })
            }
            DPOS_GET_UNDELEGATION_V2_SELECTOR => {
                decode_abi_address_argument_with_offset(
                    &request.input,
                    4,
                    "getUndelegationV2 delegator",
                )?;
                decode_abi_address_argument_with_offset(
                    &request.input,
                    36,
                    "getUndelegationV2 validator",
                )?;
                decode_abi_word_as_u64(&request.input, 68, "getUndelegationV2 id")?;
                if self.is_on_cornus(request.block_number) {
                    Ok(DPOS_GET_METHOD_GAS)
                } else {
                    Ok(0)
                }
            }
            _ => Ok(0),
        }
    }

    fn is_on_cornus(&self, block_number: u64) -> bool {
        block_number >= self.dpos_cornus_period
    }

    fn magnolia_active(&self, block_number: u64) -> bool {
        self.rewards_config.magnolia_period != 0
            && block_number >= self.rewards_config.magnolia_period
    }

    fn cacti_active(&self, block_number: u64) -> bool {
        self.rewards_config.cacti_period != 0 && block_number >= self.rewards_config.cacti_period
    }

    fn slashing_is_jailed(
        &self,
        snapshot: &DposSnapshot,
        block_number: u64,
        validator: [u8; 20],
    ) -> bool {
        self.magnolia_active(block_number)
            && snapshot
                .slashing_jail_blocks
                .get(&validator)
                .is_some_and(|jail_block| *jail_block >= block_number)
    }

    fn dpos_effective_vote_count(
        &self,
        snapshot: &DposSnapshot,
        block_number: u64,
        validator: [u8; 20],
    ) -> u64 {
        if self.cacti_active(block_number)
            && self.slashing_is_jailed(snapshot, block_number, validator)
        {
            return 0;
        }
        *snapshot.vote_counts.get(&validator).unwrap_or(&0)
    }

    fn dpos_effective_total_vote_count(
        &self,
        snapshot: &DposSnapshot,
        block_number: u64,
    ) -> Result<u64, anyhow::Error> {
        let mut total = snapshot.total_vote_count;
        for validator in &snapshot.slashing_jailed_validators {
            if self.slashing_is_jailed(snapshot, block_number, *validator) {
                total = total
                    .checked_sub(*snapshot.vote_counts.get(validator).unwrap_or(&0))
                    .ok_or_else(|| anyhow::anyhow!("DPoS jailed vote subtraction underflow"))?;
            }
        }
        Ok(total)
    }

    fn dpos_delegation_locking_period(&self, block_number: u64) -> u64 {
        if self.dpos_cacti_period != 0 && block_number >= self.dpos_cacti_period {
            self.dpos_cacti_delegation_locking_period
        } else if self.is_on_cornus(block_number) {
            self.dpos_cornus_delegation_locking_period
        } else {
            self.dpos_delegation_locking_period
        }
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
        let mut output = Vec::new();
        output.extend_from_slice(&abi_word_from_u64(32));
        output.extend_from_slice(
            &self.encode_dpos_validator_basic_info_payload(&snapshot, validator)?,
        );
        Ok(output)
    }

    fn encode_dpos_validator_basic_info_payload(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        let total_stake = snapshot
            .total_stakes
            .get(&validator)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                anyhow::anyhow!("Rust FinalChain::call DPoS validator does not exist")
            })?;
        let commission_reward = snapshot
            .commission_rewards
            .get(&validator)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let metadata = snapshot
            .validator_metadata
            .get(&validator)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("Rust FinalChain::call DPoS validator metadata is missing")
            })?;
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
        output.extend_from_slice(&abi_word_from_u256_bytes(total_stake)?);
        output.extend_from_slice(&abi_word_from_u256_bytes(commission_reward)?);
        output.extend_from_slice(&abi_word_from_u64(u64::from(metadata.commission)));
        output.extend_from_slice(&abi_word_from_u64(metadata.last_commission_change));
        output.extend_from_slice(&abi_word_from_u64(u64::from(
            dpos_undelegations_v2_count_for_validator(snapshot, validator),
        )));
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

    fn encode_dpos_validators(
        &self,
        block_number: u64,
        batch: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        let start = usize::try_from(batch)
            .map_err(|_| anyhow::anyhow!("getValidators batch does not fit into usize"))?
            .checked_mul(DPOS_GET_VALIDATORS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getValidators batch offset overflow"))?;
        let end_index = start
            .checked_add(DPOS_GET_VALIDATORS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getValidators batch end overflow"))?;
        let page = snapshot
            .validator_order
            .iter()
            .filter(|validator| snapshot.total_stakes.contains_key(*validator))
            .skip(start)
            .take(DPOS_GET_VALIDATORS_MAX_COUNT)
            .copied()
            .collect::<Vec<_>>();
        self.encode_dpos_validator_page(
            &snapshot,
            &page,
            end_index >= snapshot.validator_order.len(),
        )
    }

    fn encode_dpos_validators_for(
        &self,
        block_number: u64,
        owner: [u8; 20],
        batch: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        let to_skip = usize::try_from(batch)
            .map_err(|_| anyhow::anyhow!("getValidatorsFor batch does not fit into usize"))?
            .checked_mul(DPOS_GET_VALIDATORS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getValidatorsFor batch offset overflow"))?;
        let mut skipped = 0usize;
        let mut page = Vec::new();
        let mut is_end = true;
        for validator in &snapshot.validator_order {
            let Some(metadata) = snapshot.validator_metadata.get(validator) else {
                continue;
            };
            if metadata.owner != owner {
                continue;
            }
            if skipped < to_skip {
                skipped += 1;
                continue;
            }
            if page.len() == DPOS_GET_VALIDATORS_MAX_COUNT {
                is_end = false;
                break;
            }
            page.push(*validator);
        }
        self.encode_dpos_validator_page(&snapshot, &page, is_end)
    }

    fn encode_dpos_validator_page(
        &self,
        snapshot: &DposSnapshot,
        validators: &[[u8; 20]],
        is_end: bool,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let payloads = validators
            .iter()
            .map(|validator| self.encode_dpos_validator_data_payload(snapshot, *validator))
            .collect::<Result<Vec<_>>>()?;
        let offsets_len = validators
            .len()
            .checked_mul(32)
            .ok_or_else(|| anyhow::anyhow!("validator page offsets size overflow"))?;
        let mut array_tail = Vec::new();
        array_tail.extend_from_slice(&abi_word_from_usize(
            validators.len(),
            "validator page length",
        )?);
        let mut next_offset = 32usize
            .checked_add(offsets_len)
            .ok_or_else(|| anyhow::anyhow!("validator page first offset overflow"))?;
        for payload in &payloads {
            array_tail.extend_from_slice(&abi_word_from_usize(
                next_offset,
                "validator page element offset",
            )?);
            next_offset = next_offset
                .checked_add(payload.len())
                .ok_or_else(|| anyhow::anyhow!("validator page element offset overflow"))?;
        }
        for payload in payloads {
            array_tail.extend_from_slice(&payload);
        }

        let mut output = Vec::new();
        output.extend_from_slice(&abi_word_from_u64(64));
        output.extend_from_slice(&abi_word_from_bool(is_end));
        output.extend_from_slice(&array_tail);
        Ok(output)
    }

    fn encode_dpos_validator_data_payload(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        let info_payload = self.encode_dpos_validator_basic_info_payload(snapshot, validator)?;
        let mut output = Vec::new();
        output.extend_from_slice(&abi_word_from_address(validator));
        output.extend_from_slice(&abi_word_from_u64(64));
        output.extend_from_slice(&info_payload);
        Ok(output)
    }

    fn encode_dpos_total_delegation(
        &self,
        block_number: u64,
        delegator: [u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        let total = snapshot
            .delegations
            .values()
            .filter_map(|delegations| delegations.get(&delegator))
            .try_fold(U256::zero(), |total, stake| {
                total
                    .checked_add(u256_from_big_endian(stake))
                    .ok_or_else(|| anyhow::anyhow!("DPoS total delegation overflow"))
            })?;
        Ok(abi_word_from_u256_bytes(&u256_to_big_endian(total))?.to_vec())
    }

    fn encode_dpos_delegations(
        &self,
        block_number: u64,
        delegator: [u8; 20],
        batch: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        let validator_order = snapshot
            .delegator_validators
            .get(&delegator)
            .cloned()
            .unwrap_or_default();
        let delegations = validator_order
            .iter()
            .filter_map(|validator| {
                snapshot
                    .delegations
                    .get(validator)
                    .and_then(|validator_delegations| validator_delegations.get(&delegator))
                    .map(|stake| (*validator, stake.as_slice()))
            })
            .collect::<Vec<_>>();

        let start = usize::try_from(batch)
            .map_err(|_| anyhow::anyhow!("getDelegations batch does not fit into usize"))?
            .checked_mul(DPOS_GET_DELEGATIONS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getDelegations batch offset overflow"))?;
        let end_index = start
            .checked_add(DPOS_GET_DELEGATIONS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getDelegations batch end overflow"))?;
        let page = delegations
            .iter()
            .skip(start)
            .take(DPOS_GET_DELEGATIONS_MAX_COUNT)
            .collect::<Vec<_>>();
        let is_end = end_index >= delegations.len();

        let mut output = Vec::new();
        output.extend_from_slice(&abi_word_from_u64(64));
        output.extend_from_slice(&abi_word_from_bool(is_end));
        output.extend_from_slice(&abi_word_from_usize(page.len(), "getDelegations length")?);
        for (validator, stake) in page {
            let reward = self.pending_delegator_reward(&snapshot, *validator, delegator, stake)?;
            output.extend_from_slice(&abi_word_from_address(*validator));
            output.extend_from_slice(&abi_word_from_u256_bytes(stake)?);
            output.extend_from_slice(&abi_word_from_u256_bytes(&u256_to_big_endian(reward))?);
        }
        Ok(output)
    }

    fn encode_dpos_undelegations_v2(
        &self,
        block_number: u64,
        delegator: [u8; 20],
        batch: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        let start = usize::try_from(batch)
            .map_err(|_| anyhow::anyhow!("getUndelegationsV2 batch does not fit into usize"))?
            .checked_mul(DPOS_GET_UNDELEGATIONS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 batch offset overflow"))?;
        let flattened = dpos_undelegations_v2_for_delegator(&snapshot, delegator);
        let end_index = start
            .checked_add(DPOS_GET_UNDELEGATIONS_MAX_COUNT)
            .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 batch end overflow"))?;
        let page = flattened
            .iter()
            .skip(start)
            .take(DPOS_GET_UNDELEGATIONS_MAX_COUNT)
            .copied()
            .collect::<Vec<_>>();
        let is_end = end_index >= flattened.len();

        let mut output = Vec::new();
        output.extend_from_slice(&abi_word_from_u64(64));
        output.extend_from_slice(&abi_word_from_bool(is_end));
        output.extend_from_slice(&abi_word_from_usize(
            page.len(),
            "getUndelegationsV2 length",
        )?);
        for (validator, entry) in page {
            encode_dpos_undelegation_v2_payload(&mut output, &snapshot, validator, entry)?;
        }
        Ok(output)
    }

    fn encode_dpos_undelegation_v2(
        &self,
        block_number: u64,
        delegator: [u8; 20],
        validator: [u8; 20],
        id: u64,
    ) -> Result<Option<Vec<u8>>, anyhow::Error> {
        let snapshot = self.dpos_snapshot_at_finalized_block(block_number)?;
        let Some(entry) = find_undelegation_v2(&snapshot, delegator, validator, id) else {
            return Ok(None);
        };
        let mut output = Vec::new();
        encode_dpos_undelegation_v2_payload(&mut output, &snapshot, validator, entry)?;
        Ok(Some(output))
    }

    fn pending_delegator_reward(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
        delegator: [u8; 20],
        stake: &[u8],
    ) -> Result<U256, anyhow::Error> {
        let current_reward_per_stake =
            self.current_validator_reward_per_stake(snapshot, validator)?;
        let cursor = snapshot
            .delegation_reward_cursors
            .get(&validator)
            .and_then(|cursors| cursors.get(&delegator))
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        anyhow::ensure!(
            current_reward_per_stake >= cursor,
            "DPoS delegation reward cursor exceeds validator reward state"
        );
        self.delegator_reward_from_per_stake(
            current_reward_per_stake - cursor,
            u256_from_big_endian(stake),
        )
    }

    fn apply_dpos_delegator_reward_claim(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        validator: [u8; 20],
        delegator: [u8; 20],
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
        let delegator_stake = snapshot
            .delegations
            .get(&validator)
            .ok_or_else(|| {
                anyhow::anyhow!("Rust FinalChain::finalize DPoS delegation does not exist")
            })?
            .get(&delegator)
            .map(|bytes| u256_from_big_endian(bytes))
            .ok_or_else(|| {
                anyhow::anyhow!("Rust FinalChain::finalize DPoS delegator stake does not exist")
            })?;
        let previous_cursor = snapshot
            .delegation_reward_cursors
            .get(&validator)
            .and_then(|cursors| cursors.get(&delegator))
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        let reward_cursor = self.checkpoint_validator_reward_per_stake(snapshot, validator)?;
        anyhow::ensure!(
            reward_cursor >= previous_cursor,
            "DPoS delegation reward cursor exceeds validator reward state"
        );
        let reward =
            self.delegator_reward_from_per_stake(reward_cursor - previous_cursor, delegator_stake)?;
        let dpos_contract_balance = u256_from_big_endian(
            accounts
                .entry(DPOS_CONTRACT_ADDRESS)
                .or_insert_with(empty_account)
                .balance
                .as_slice(),
        );
        if reward > dpos_contract_balance {
            anyhow::bail!(
                "Rust FinalChain::finalize DPoS contract balance insufficient for reward claim"
            );
        }

        snapshot
            .delegation_reward_cursors
            .entry(validator)
            .or_default()
            .insert(delegator, u256_to_big_endian(reward_cursor));

        if reward.is_zero() {
            return Ok(Vec::new());
        }

        let dpos_account = accounts
            .entry(DPOS_CONTRACT_ADDRESS)
            .or_insert_with(empty_account);
        dpos_account.balance = u256_to_big_endian(
            dpos_contract_balance
                .checked_sub(reward)
                .ok_or_else(|| anyhow::anyhow!("DPoS contract reward underflow"))?,
        );
        let delegator_account = accounts.entry(delegator).or_insert_with(empty_account);
        let current_delegator_balance = u256_from_big_endian(&delegator_account.balance);
        delegator_account.balance = u256_to_big_endian(
            current_delegator_balance
                .checked_add(reward)
                .ok_or_else(|| anyhow::anyhow!("DPoS delegator reward overflow"))?,
        );
        Ok(vec![dpos_rewards_claimed_log(
            delegator, validator, reward,
        )?])
    }

    fn apply_dpos_claim_all_rewards(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        batch: Option<u32>,
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
        let validators = snapshot
            .delegator_validators
            .get(&delegator)
            .cloned()
            .unwrap_or_default();
        let start = if let Some(batch) = batch {
            usize::try_from(batch)
                .map_err(|_| {
                    anyhow::anyhow!("claimAllRewards batch index does not fit into usize")
                })?
                .checked_mul(DPOS_CLAIM_ALL_REWARDS_MAX_COUNT)
                .ok_or_else(|| anyhow::anyhow!("claimAllRewards batch start offset overflow"))?
        } else {
            0usize
        };
        let claim_count = match batch {
            None => validators.len() as u64,
            Some(batch) => dpos_batch_items_count(
                validators.len() as u64,
                batch,
                u64::try_from(DPOS_CLAIM_ALL_REWARDS_MAX_COUNT).map_err(|_| {
                    anyhow::anyhow!("claimAllRewards batch max count does not fit into u64")
                })?,
            )?,
        };
        if start > 0 && start >= validators.len() {
            return Ok(Vec::new());
        }
        let claim_count = usize::try_from(claim_count).map_err(|_| {
            anyhow::anyhow!("claimAllRewards batch item count does not fit into usize")
        })?;
        let mut logs = Vec::new();
        for validator in validators.iter().skip(start).take(claim_count) {
            logs.extend(
                self.apply_dpos_delegator_reward_claim(snapshot, accounts, *validator, delegator)?,
            );
        }
        Ok(logs)
    }

    fn apply_dpos_commission_reward_claim(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        owner: [u8; 20],
        validator: [u8; 20],
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        let Some(metadata) = snapshot.validator_metadata.get(&validator) else {
            return Ok(DposApplyOutcome::contract_failure());
        };
        if metadata.owner != owner {
            return Ok(DposApplyOutcome::contract_failure());
        }

        let reward = snapshot
            .commission_rewards
            .get(&validator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        let dpos_contract_balance = u256_from_big_endian(
            accounts
                .entry(DPOS_CONTRACT_ADDRESS)
                .or_insert_with(empty_account)
                .balance
                .as_slice(),
        );
        if reward > dpos_contract_balance {
            anyhow::bail!(
                "Rust FinalChain::finalize DPoS contract balance insufficient for commission reward claim"
            );
        }

        snapshot
            .commission_rewards
            .insert(validator, u256_to_big_endian(U256::zero()));
        if !reward.is_zero() {
            let dpos_account = accounts
                .entry(DPOS_CONTRACT_ADDRESS)
                .or_insert_with(empty_account);
            dpos_account.balance = u256_to_big_endian(
                dpos_contract_balance
                    .checked_sub(reward)
                    .ok_or_else(|| anyhow::anyhow!("DPoS contract commission reward underflow"))?,
            );
            let owner_account = accounts.entry(owner).or_insert_with(empty_account);
            let current_owner_balance = u256_from_big_endian(&owner_account.balance);
            owner_account.balance = u256_to_big_endian(
                current_owner_balance
                    .checked_add(reward)
                    .ok_or_else(|| anyhow::anyhow!("DPoS commission reward overflow"))?,
            );
        }

        Ok(DposApplyOutcome::success(vec![
            dpos_commission_rewards_claimed_log(owner, validator, reward)?,
        ]))
    }

    fn apply_dpos_validator_info_update(
        &self,
        snapshot: &mut DposSnapshot,
        owner: [u8; 20],
        validator: [u8; 20],
        description: String,
        endpoint: String,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        if endpoint.len() > DPOS_MAX_ENDPOINT_LENGTH
            || description.len() > DPOS_MAX_DESCRIPTION_LENGTH
        {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let Some(metadata) = snapshot.validator_metadata.get_mut(&validator) else {
            return Ok(DposApplyOutcome::contract_failure());
        };
        if metadata.owner != owner {
            return Ok(DposApplyOutcome::contract_failure());
        }
        metadata.description = description;
        metadata.endpoint = endpoint;
        Ok(DposApplyOutcome::success(vec![
            dpos_validator_info_set_log(validator),
        ]))
    }

    fn apply_dpos_commission_update(
        &self,
        snapshot: &mut DposSnapshot,
        owner: [u8; 20],
        validator: [u8; 20],
        commission: u16,
        block_number: u64,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        if commission > DPOS_MAX_COMMISSION {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let Some(metadata) = snapshot.validator_metadata.get_mut(&validator) else {
            return Ok(DposApplyOutcome::contract_failure());
        };
        if metadata.owner != owner {
            return Ok(DposApplyOutcome::contract_failure());
        }
        if self.dpos_commission_change_frequency != 0
            && block_number
                < metadata
                    .last_commission_change
                    .saturating_add(u64::from(self.dpos_commission_change_frequency))
        {
            return Ok(DposApplyOutcome::contract_failure());
        }
        if self.dpos_commission_change_delta != 0 {
            let delta = commission.abs_diff(metadata.commission);
            if delta > self.dpos_commission_change_delta {
                return Ok(DposApplyOutcome::contract_failure());
            }
        }
        metadata.commission = commission;
        metadata.last_commission_change = block_number;
        Ok(DposApplyOutcome::success(vec![dpos_commission_set_log(
            validator, commission,
        )?]))
    }

    /// Plans the DPoS snapshot for a newly finalized block.
    ///
    /// The new snapshot clones the previous block state and applies finalized
    /// DPoS transactions. Reward deltas are applied separately after native
    /// reward planning has completed so overflow can abort before persistence.
    fn plan_dpos_snapshot(
        &self,
        block_number: u64,
        contract_transactions: Vec<(usize, NativeContractTransaction)>,
        accounts: &mut HashMap<[u8; 20], Account>,
        receipts: &mut [NativeReceipt],
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
        let delayed_snapshot_block = block_number.saturating_sub(self.dpos_delegation_delay);
        let slashing_validator_snapshot = if delayed_snapshot_block == block_number {
            None
        } else {
            snapshots.get(&delayed_snapshot_block).cloned()
        };
        for (position, contract_tx) in contract_transactions {
            let outcome = match contract_tx {
                NativeContractTransaction::Dpos(dpos_tx) => match dpos_tx {
                    DposTransaction::Register(registration) => DposApplyOutcome::success(
                        self.apply_dpos_registration(&mut snapshot, registration, block_number)?,
                    ),
                    DposTransaction::Delegate {
                        delegator,
                        validator,
                        amount,
                    } => DposApplyOutcome::success(self.apply_dpos_delegate(
                        &mut snapshot,
                        accounts,
                        delegator,
                        validator,
                        amount,
                    )?),
                    DposTransaction::Undelegate {
                        delegator,
                        validator,
                        amount,
                    } => DposApplyOutcome::success(self.apply_dpos_undelegate(
                        &mut snapshot,
                        accounts,
                        delegator,
                        validator,
                        amount,
                    )?),
                    DposTransaction::UndelegateV2 {
                        delegator,
                        validator,
                        amount,
                    } => self.apply_dpos_undelegate_v2(
                        &mut snapshot,
                        accounts,
                        delegator,
                        validator,
                        amount,
                        block_number,
                    )?,
                    DposTransaction::ConfirmUndelegateV2 {
                        delegator,
                        validator,
                        id,
                    } => self.apply_dpos_confirm_undelegate_v2(
                        &mut snapshot,
                        accounts,
                        delegator,
                        validator,
                        id,
                        block_number,
                    )?,
                    DposTransaction::CancelUndelegateV2 {
                        delegator,
                        validator,
                        id,
                    } => self.apply_dpos_cancel_undelegate_v2(
                        &mut snapshot,
                        accounts,
                        delegator,
                        validator,
                        id,
                    )?,
                    DposTransaction::Redelegate {
                        delegator,
                        from,
                        to,
                        amount,
                    } => DposApplyOutcome::success(self.apply_dpos_redelegate(
                        &mut snapshot,
                        accounts,
                        delegator,
                        from,
                        to,
                        amount,
                    )?),
                    DposTransaction::ClaimRewards {
                        delegator,
                        validator,
                    } => DposApplyOutcome::success(self.apply_dpos_delegator_reward_claim(
                        &mut snapshot,
                        accounts,
                        validator,
                        delegator,
                    )?),
                    DposTransaction::ClaimCommissionRewards { owner, validator } => self
                        .apply_dpos_commission_reward_claim(
                            &mut snapshot,
                            accounts,
                            owner,
                            validator,
                        )?,
                    DposTransaction::SetValidatorInfo {
                        owner,
                        validator,
                        description,
                        endpoint,
                    } => self.apply_dpos_validator_info_update(
                        &mut snapshot,
                        owner,
                        validator,
                        description,
                        endpoint,
                    )?,
                    DposTransaction::SetCommission {
                        owner,
                        validator,
                        commission,
                    } => self.apply_dpos_commission_update(
                        &mut snapshot,
                        owner,
                        validator,
                        commission,
                        block_number,
                    )?,
                    DposTransaction::ClaimAllRewards { delegator, batch } => {
                        DposApplyOutcome::success(self.apply_dpos_claim_all_rewards(
                            &mut snapshot,
                            accounts,
                            delegator,
                            batch,
                        )?)
                    }
                    DposTransaction::MethodNotSupported => DposApplyOutcome::contract_failure(),
                },
                NativeContractTransaction::Slashing(slashing_tx) => self
                    .apply_slashing_transaction(
                        &mut snapshot,
                        slashing_validator_snapshot.as_ref(),
                        block_number,
                        slashing_tx,
                    )?,
            };
            let receipt = receipts.get_mut(position).ok_or_else(|| {
                anyhow::anyhow!(
                    "native contract receipt position {position} is outside finalized transaction list"
                )
            })?;
            receipt.status_code = outcome.status_code;
            receipt.logs = outcome.logs;
        }
        cleanup_slashing_jailed_validators(&mut snapshot, block_number);
        Ok(snapshot)
    }

    fn apply_slashing_transaction(
        &self,
        snapshot: &mut DposSnapshot,
        validator_snapshot: Option<&DposSnapshot>,
        block_number: u64,
        slashing_tx: SlashingTransaction,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        match slashing_tx {
            SlashingTransaction::CommitDoubleVotingProof(proof) => match *proof {
                Ok(proof) => self.apply_slashing_double_voting_proof(
                    snapshot,
                    validator_snapshot,
                    block_number,
                    proof,
                ),
                Err(_) => Ok(DposApplyOutcome::contract_failure()),
            },
            SlashingTransaction::MethodNotSupported => Ok(DposApplyOutcome::contract_failure()),
        }
    }

    fn apply_slashing_double_voting_proof(
        &self,
        snapshot: &mut DposSnapshot,
        validator_snapshot: Option<&DposSnapshot>,
        block_number: u64,
        proof: VerifiedLegacyDoubleVotingProof,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        if !self.magnolia_active(block_number) {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let mut offender = [0u8; 20];
        offender.copy_from_slice(proof.offender.as_bytes());
        let validator_exists = validator_snapshot
            .map(|snapshot| snapshot.total_stakes.contains_key(&offender))
            .unwrap_or_else(|| snapshot.total_stakes.contains_key(&offender));
        if !validator_exists {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let mut proof_key = [0u8; 32];
        proof_key.copy_from_slice(proof.proof_key.as_bytes());
        if !snapshot.slashing_double_voting_proofs.insert(proof_key) {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let jail_time = if self.cacti_active(block_number) {
            self.rewards_config.cacti_jail_time
        } else {
            self.rewards_config.magnolia_jail_time
        };
        let jail_block = block_number
            .checked_add(jail_time)
            .ok_or_else(|| anyhow::anyhow!("slashing jail block overflow"))?;
        snapshot.slashing_jail_blocks.insert(offender, jail_block);
        if !snapshot.slashing_jailed_validators.contains(&offender) {
            snapshot.slashing_jailed_validators.push(offender);
        }
        Ok(DposApplyOutcome::success(vec![slashing_jailed_log(
            offender,
            block_number,
            jail_block,
        )?]))
    }

    fn apply_dpos_registration(
        &self,
        snapshot: &mut DposSnapshot,
        registration: DposRegistration,
        block_number: u64,
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
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
        let owner = registration.metadata.owner;
        let mut metadata = registration.metadata;
        metadata.last_commission_change = block_number;
        snapshot
            .validator_metadata
            .insert(registration.validator, metadata);
        snapshot.validator_order.push(registration.validator);
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
        add_delegator_validator(
            &mut snapshot.delegator_validators,
            registration.validator,
            registration.validator,
        );
        let mut reward_cursors = BTreeMap::new();
        reward_cursors.insert(registration.validator, Vec::new());
        snapshot
            .delegation_reward_cursors
            .insert(registration.validator, reward_cursors);
        snapshot
            .validator_reward_per_stake
            .entry(registration.validator)
            .or_default();
        let mut logs = vec![dpos_validator_registered_log(registration.validator)];
        if !u256_from_big_endian(&registration.stake).is_zero() {
            logs.push(dpos_delegated_log(
                owner,
                registration.validator,
                u256_from_big_endian(&registration.stake),
            )?);
        }
        Ok(logs)
    }

    fn apply_dpos_delegate(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
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
        let mut logs = if current_delegation.is_zero() {
            Vec::new()
        } else {
            self.apply_dpos_delegator_reward_claim(snapshot, accounts, validator, delegator)?
        };
        let reward_cursor = if current_delegation.is_zero() {
            self.checkpoint_validator_reward_per_stake(snapshot, validator)?
        } else {
            U256::zero()
        };
        let delegations = snapshot.delegations.entry(validator).or_default();
        if current_delegation.is_zero() {
            add_delegator_validator(&mut snapshot.delegator_validators, delegator, validator);
            snapshot
                .delegation_reward_cursors
                .entry(validator)
                .or_default()
                .insert(delegator, u256_to_big_endian(reward_cursor));
        }
        delegations.insert(
            delegator,
            u256_to_big_endian(
                current_delegation
                    .checked_add(add_amount)
                    .ok_or_else(|| anyhow::anyhow!("DPoS delegation addition overflow"))?,
            ),
        );
        self.set_validator_stake(snapshot, validator, new_stake)?;
        logs.push(dpos_delegated_log(delegator, validator, add_amount)?);
        Ok(logs)
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
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
        let remove_amount = u256_from_big_endian(&amount);
        let mut logs = self.remove_dpos_delegation_stake(
            snapshot,
            accounts,
            delegator,
            validator,
            remove_amount,
        )?;
        logs.push(dpos_undelegated_log(delegator, validator, remove_amount)?);
        Ok(logs)
    }

    fn apply_dpos_undelegate_v2(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
        block_number: u64,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        let remove_amount = u256_from_big_endian(&amount);
        if self.dpos_delegation_removal_contract_failure(
            snapshot,
            delegator,
            validator,
            remove_amount,
        ) {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let mut logs = self.remove_dpos_delegation_stake(
            snapshot,
            accounts,
            delegator,
            validator,
            remove_amount,
        )?;
        let id = create_undelegation_v2(
            snapshot,
            delegator,
            validator,
            u256_to_big_endian(remove_amount),
            block_number
                .checked_add(self.dpos_delegation_locking_period(block_number))
                .ok_or_else(|| anyhow::anyhow!("DPoS undelegation unlock block overflow"))?,
        )?;
        logs.push(dpos_undelegated_v2_log(
            delegator,
            validator,
            id,
            remove_amount,
        )?);
        Ok(DposApplyOutcome::success(logs))
    }

    fn apply_dpos_confirm_undelegate_v2(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        validator: [u8; 20],
        id: u64,
        block_number: u64,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        let Some(entry) = find_undelegation_v2(snapshot, delegator, validator, id).cloned() else {
            return Ok(DposApplyOutcome::contract_failure());
        };
        if entry.block > block_number {
            return Ok(DposApplyOutcome::contract_failure());
        }
        remove_undelegation_v2(snapshot, delegator, validator, id);
        let amount = u256_from_big_endian(&entry.amount);
        let dpos_contract_balance = u256_from_big_endian(
            accounts
                .entry(DPOS_CONTRACT_ADDRESS)
                .or_insert_with(empty_account)
                .balance
                .as_slice(),
        );
        if dpos_contract_balance < amount {
            anyhow::bail!("DPoS contract balance insufficient for undelegation V2 confirmation");
        }
        let dpos_account = accounts
            .entry(DPOS_CONTRACT_ADDRESS)
            .or_insert_with(empty_account);
        dpos_account.balance = u256_to_big_endian(dpos_contract_balance - amount);
        let delegator_account = accounts.entry(delegator).or_insert_with(empty_account);
        let delegator_balance = u256_from_big_endian(&delegator_account.balance);
        delegator_account.balance = u256_to_big_endian(
            delegator_balance
                .checked_add(amount)
                .ok_or_else(|| anyhow::anyhow!("DPoS undelegation V2 confirmation overflow"))?,
        );
        Ok(DposApplyOutcome::success(vec![
            dpos_undelegate_confirmed_v2_log(delegator, validator, id, amount)?,
        ]))
    }

    fn apply_dpos_cancel_undelegate_v2(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        validator: [u8; 20],
        id: u64,
    ) -> Result<DposApplyOutcome, anyhow::Error> {
        let Some(entry) = find_undelegation_v2(snapshot, delegator, validator, id).cloned() else {
            return Ok(DposApplyOutcome::contract_failure());
        };
        if !snapshot.total_stakes.contains_key(&validator) {
            return Ok(DposApplyOutcome::contract_failure());
        }
        let amount = u256_from_big_endian(&entry.amount);
        let mut logs = if snapshot
            .delegations
            .get(&validator)
            .and_then(|delegations| delegations.get(&delegator))
            .is_some()
        {
            self.apply_dpos_delegator_reward_claim(snapshot, accounts, validator, delegator)?
        } else {
            let reward_cursor = self.checkpoint_validator_reward_per_stake(snapshot, validator)?;
            add_delegator_validator(&mut snapshot.delegator_validators, delegator, validator);
            snapshot
                .delegation_reward_cursors
                .entry(validator)
                .or_default()
                .insert(delegator, u256_to_big_endian(reward_cursor));
            Vec::new()
        };
        let current_delegation = snapshot
            .delegations
            .entry(validator)
            .or_default()
            .get(&delegator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        snapshot.delegations.entry(validator).or_default().insert(
            delegator,
            u256_to_big_endian(
                current_delegation
                    .checked_add(amount)
                    .ok_or_else(|| anyhow::anyhow!("DPoS undelegation V2 cancel overflow"))?,
            ),
        );
        let current_stake = snapshot
            .total_stakes
            .get(&validator)
            .map(|bytes| u256_from_big_endian(bytes))
            .unwrap_or_default();
        self.set_validator_stake(
            snapshot,
            validator,
            current_stake
                .checked_add(amount)
                .ok_or_else(|| anyhow::anyhow!("DPoS undelegation V2 cancel stake overflow"))?,
        )?;
        remove_undelegation_v2(snapshot, delegator, validator, id);
        logs.push(dpos_undelegate_canceled_v2_log(
            delegator, validator, id, amount,
        )?);
        Ok(DposApplyOutcome::success(logs))
    }

    fn dpos_delegation_removal_contract_failure(
        &self,
        snapshot: &DposSnapshot,
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: U256,
    ) -> bool {
        let Some(stake) = snapshot.total_stakes.get(&validator) else {
            return true;
        };
        let current_stake = u256_from_big_endian(stake);
        let Some(current_delegation) = snapshot
            .delegations
            .get(&validator)
            .and_then(|delegations| delegations.get(&delegator))
            .map(|bytes| u256_from_big_endian(bytes))
        else {
            return true;
        };
        if current_delegation < amount || current_stake < amount {
            return true;
        }
        let remaining = current_delegation - amount;
        !remaining.is_zero() && remaining < u256_from_big_endian(&self.dpos_minimum_deposit)
    }

    fn remove_dpos_delegation_stake(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        validator: [u8; 20],
        remove_amount: U256,
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
        let Some(stake) = snapshot.total_stakes.get(&validator) else {
            anyhow::bail!("Rust FinalChain::finalize DPoS validator does not exist for undelegate")
        };
        let current_stake = u256_from_big_endian(stake);
        let current_delegation = snapshot
            .delegations
            .get(&validator)
            .ok_or_else(|| {
                anyhow::anyhow!("Rust FinalChain::finalize DPoS delegation does not exist")
            })?
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
        if !new_delegation.is_zero()
            && new_delegation < u256_from_big_endian(&self.dpos_minimum_deposit)
        {
            anyhow::bail!("Rust FinalChain::finalize DPoS remaining delegation is below minimum");
        }
        let logs =
            self.apply_dpos_delegator_reward_claim(snapshot, accounts, validator, delegator)?;
        let delegations = snapshot.delegations.get_mut(&validator).ok_or_else(|| {
            anyhow::anyhow!("Rust FinalChain::finalize DPoS delegation does not exist")
        })?;
        if new_delegation.is_zero() {
            delegations.remove(&delegator);
            remove_delegator_validator(&mut snapshot.delegator_validators, delegator, validator);
            if let Some(cursors) = snapshot.delegation_reward_cursors.get_mut(&validator) {
                cursors.remove(&delegator);
            }
        } else {
            delegations.insert(delegator, u256_to_big_endian(new_delegation));
        }
        let new_stake = current_stake - remove_amount;
        self.set_validator_stake(snapshot, validator, new_stake)?;
        Ok(logs)
    }

    fn apply_dpos_redelegate(
        &self,
        snapshot: &mut DposSnapshot,
        accounts: &mut HashMap<[u8; 20], Account>,
        delegator: [u8; 20],
        from: [u8; 20],
        to: [u8; 20],
        amount: Vec<u8>,
    ) -> Result<Vec<ReceiptLog>, anyhow::Error> {
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
        let mut logs = self
            .apply_dpos_undelegate(snapshot, accounts, delegator, from, amount.clone())?
            .into_iter()
            .filter(is_dpos_rewards_claimed_log)
            .collect::<Vec<_>>();
        logs.extend(
            self.apply_dpos_delegate(snapshot, accounts, delegator, to, amount.clone())?
                .into_iter()
                .filter(is_dpos_rewards_claimed_log),
        );
        logs.push(dpos_redelegated_log(
            delegator,
            from,
            to,
            u256_from_big_endian(&amount),
        )?);
        Ok(logs)
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
    let mut stream = rlp::RlpStream::new_list(20);
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
    append_address_bytes_map(&mut stream, &snapshot.validator_reward_per_stake);
    append_delegations_map(&mut stream, &snapshot.delegation_reward_cursors);
    append_delegator_validators_map(&mut stream, &snapshot.delegator_validators);
    append_address_vec(&mut stream, &snapshot.validator_order);
    append_undelegations_v2_map(&mut stream, &snapshot.undelegations_v2);
    append_address_u64_map(&mut stream, &snapshot.undelegation_v2_last_ids);
    append_address_u64_map(&mut stream, &snapshot.slashing_jail_blocks);
    append_address_vec(&mut stream, &snapshot.slashing_jailed_validators);
    append_fixed_hash_set(&mut stream, &snapshot.slashing_double_voting_proofs);
    stream.out().to_vec()
}

fn decode_dpos_snapshot_rlp(raw: &[u8]) -> Result<DposSnapshot, anyhow::Error> {
    let rlp = Rlp::new(raw);
    let item_count = rlp.item_count()?;
    if item_count != 5
        && item_count != 6
        && item_count != 7
        && item_count != 9
        && item_count != 11
        && item_count != 14
        && item_count != 15
        && item_count != 17
        && item_count != 20
    {
        anyhow::bail!(
            "DPoS snapshot RLP must contain exactly five, six, seven, nine, eleven, fourteen, fifteen, seventeen, or twenty items"
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
    let (total_supply, current_yield) = if item_count >= 11 {
        (rlp.at(9)?.data()?.to_vec(), rlp.val_at(10)?)
    } else {
        (Vec::new(), 0)
    };
    let validator_reward_per_stake = if item_count >= 14 {
        decode_address_bytes_map(&rlp.at(11)?, "validator reward per stake")?
    } else {
        BTreeMap::new()
    };
    let delegation_reward_cursors = if item_count >= 14 {
        decode_delegations_map(&rlp.at(12)?)?
    } else {
        synthesize_empty_delegation_cursors(&delegations)
    };
    let delegator_validators = if item_count >= 14 {
        decode_delegator_validators_map(&rlp.at(13)?)?
    } else {
        delegator_validators_from_delegations(&delegations)
    };
    let validator_order = if item_count >= 15 {
        decode_address_vec(&rlp.at(14)?, "validator order")?
    } else {
        total_stakes.keys().copied().collect()
    };
    let undelegations_v2 = if item_count >= 17 {
        decode_undelegations_v2_map(&rlp.at(15)?)?
    } else {
        BTreeMap::new()
    };
    let undelegation_v2_last_ids = if item_count >= 17 {
        decode_address_u64_map(&rlp.at(16)?, "undelegation V2 last id")?
    } else {
        BTreeMap::new()
    };
    let slashing_jail_blocks = if item_count >= 20 {
        decode_address_u64_map(&rlp.at(17)?, "slashing jail block")?
    } else {
        BTreeMap::new()
    };
    let slashing_jailed_validators = if item_count >= 20 {
        decode_address_vec(&rlp.at(18)?, "slashing jailed validator")?
    } else {
        Vec::new()
    };
    let slashing_double_voting_proofs = if item_count >= 20 {
        decode_fixed_hash_set(&rlp.at(19)?, "slashing double voting proof")?
    } else {
        BTreeSet::new()
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
        validator_order,
        delegator_validators,
        delegation_reward_cursors,
        validator_reward_per_stake,
        minted_tokens,
        total_supply,
        current_yield,
        undelegations_v2,
        undelegation_v2_last_ids,
        slashing_jail_blocks,
        slashing_jailed_validators,
        slashing_double_voting_proofs,
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
        stream.begin_list(6);
        stream.append(&address.as_slice());
        stream.append(&metadata.owner.as_slice());
        stream.append(&metadata.commission);
        stream.append(&metadata.last_commission_change);
        stream.append(&metadata.description.as_str());
        stream.append(&metadata.endpoint.as_str());
    }
}

fn append_address_vec(stream: &mut rlp::RlpStream, addresses: &[[u8; 20]]) {
    stream.begin_list(addresses.len());
    for address in addresses {
        stream.append(&address.as_slice());
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

fn append_delegator_validators_map(stream: &mut rlp::RlpStream, map: &DposDelegatorValidators) {
    stream.begin_list(map.len());
    for (delegator, validators) in map {
        stream.begin_list(2);
        stream.append(&delegator.as_slice());
        stream.begin_list(validators.len());
        for validator in validators {
            stream.append(&validator.as_slice());
        }
    }
}

fn append_undelegations_v2_map(stream: &mut rlp::RlpStream, map: &DposUndelegationsV2) {
    stream.begin_list(map.len());
    for (delegator, validator_groups) in map {
        stream.begin_list(2);
        stream.append(&delegator.as_slice());
        stream.begin_list(validator_groups.len());
        for group in validator_groups {
            stream.begin_list(2);
            stream.append(&group.validator.as_slice());
            stream.begin_list(group.entries.len());
            for entry in &group.entries {
                stream.begin_list(3);
                stream.append(&entry.id);
                stream.append(&entry.amount.as_slice());
                stream.append(&entry.block);
            }
        }
    }
}

fn append_address_u64_map(stream: &mut rlp::RlpStream, map: &BTreeMap<[u8; 20], u64>) {
    stream.begin_list(map.len());
    for (address, value) in map {
        stream.begin_list(2);
        stream.append(&address.as_slice());
        stream.append(value);
    }
}

fn append_fixed_hash_set(stream: &mut rlp::RlpStream, values: &BTreeSet<[u8; 32]>) {
    stream.begin_list(values.len());
    for value in values {
        stream.append(&value.as_slice());
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

fn decode_undelegations_v2_map(rlp: &Rlp<'_>) -> Result<DposUndelegationsV2, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!("DPoS V2 undelegation delegator entry must contain exactly two items");
        }
        let delegator = decode_address(&item.at(0)?, "V2 undelegation delegator")?;
        let mut validator_groups = Vec::new();
        for validator_item in item.at(1)?.iter() {
            if validator_item.item_count()? != 2 {
                anyhow::bail!(
                    "DPoS V2 undelegation validator entry must contain exactly two items"
                );
            }
            let validator = decode_address(&validator_item.at(0)?, "V2 undelegation validator")?;
            let mut entries = Vec::new();
            for entry_item in validator_item.at(1)?.iter() {
                if entry_item.item_count()? != 3 {
                    anyhow::bail!("DPoS V2 undelegation item must contain exactly three items");
                }
                entries.push(DposUndelegationV2Entry {
                    id: entry_item.val_at(0)?,
                    amount: entry_item.val_at(1)?,
                    block: entry_item.val_at(2)?,
                });
            }
            validator_groups.push(DposValidatorUndelegationsV2 { validator, entries });
        }
        map.insert(delegator, validator_groups);
    }
    Ok(map)
}

fn decode_address_u64_map(
    rlp: &Rlp<'_>,
    field: &str,
) -> Result<BTreeMap<[u8; 20], u64>, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!("DPoS snapshot {field} entry must contain exactly two items");
        }
        map.insert(decode_address(&item.at(0)?, field)?, item.val_at(1)?);
    }
    Ok(map)
}

fn decode_fixed_hash_set(rlp: &Rlp<'_>, field: &str) -> Result<BTreeSet<[u8; 32]>, anyhow::Error> {
    rlp.iter()
        .map(|item| decode_fixed_hash(&item, field))
        .collect()
}

fn decode_delegator_validators_map(
    rlp: &Rlp<'_>,
) -> Result<DposDelegatorValidators, anyhow::Error> {
    let mut map = BTreeMap::new();
    for item in rlp.iter() {
        if item.item_count()? != 2 {
            anyhow::bail!(
                "DPoS snapshot delegator validators entry must contain exactly two items"
            );
        }
        let delegator = decode_address(&item.at(0)?, "delegator validators delegator")?;
        let validators = item
            .at(1)?
            .iter()
            .map(|validator| decode_address(&validator, "delegator validator address"))
            .collect::<Result<Vec<_>>>()?;
        map.insert(delegator, validators);
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

fn synthesize_empty_delegation_cursors(
    delegations: &DposDelegations,
) -> DposDelegationRewardCursors {
    delegations
        .iter()
        .map(|(validator, delegators)| {
            (
                *validator,
                delegators
                    .keys()
                    .map(|delegator| (*delegator, Vec::new()))
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect()
}

fn delegator_validators_from_delegations(delegations: &DposDelegations) -> DposDelegatorValidators {
    let mut map = BTreeMap::new();
    for (validator, validator_delegations) in delegations {
        for delegator in validator_delegations.keys() {
            add_delegator_validator(&mut map, *delegator, *validator);
        }
    }
    map
}

fn add_delegator_validator(
    map: &mut DposDelegatorValidators,
    delegator: [u8; 20],
    validator: [u8; 20],
) {
    let validators = map.entry(delegator).or_default();
    if !validators.contains(&validator) {
        validators.push(validator);
    }
}

fn remove_delegator_validator(
    map: &mut DposDelegatorValidators,
    delegator: [u8; 20],
    validator: [u8; 20],
) {
    let Some(validators) = map.get_mut(&delegator) else {
        return;
    };
    if let Some(position) = validators
        .iter()
        .position(|existing| *existing == validator)
    {
        validators.swap_remove(position);
    }
    if validators.is_empty() {
        map.remove(&delegator);
    }
}

fn create_undelegation_v2(
    snapshot: &mut DposSnapshot,
    delegator: [u8; 20],
    validator: [u8; 20],
    amount: Vec<u8>,
    block: u64,
) -> Result<u64, anyhow::Error> {
    let id = snapshot
        .undelegation_v2_last_ids
        .get(&delegator)
        .copied()
        .unwrap_or_default()
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("DPoS undelegation V2 id overflow"))?;
    snapshot.undelegation_v2_last_ids.insert(delegator, id);
    let validators = snapshot.undelegations_v2.entry(delegator).or_default();
    if let Some(group) = validators
        .iter_mut()
        .find(|group| group.validator == validator)
    {
        group
            .entries
            .push(DposUndelegationV2Entry { id, amount, block });
    } else {
        validators.push(DposValidatorUndelegationsV2 {
            validator,
            entries: vec![DposUndelegationV2Entry { id, amount, block }],
        });
    }
    Ok(id)
}

fn remove_undelegation_v2(
    snapshot: &mut DposSnapshot,
    delegator: [u8; 20],
    validator: [u8; 20],
    id: u64,
) -> bool {
    let Some(validators) = snapshot.undelegations_v2.get_mut(&delegator) else {
        return false;
    };
    let Some(validator_position) = validators
        .iter()
        .position(|group| group.validator == validator)
    else {
        return false;
    };
    let group = &mut validators[validator_position];
    let Some(entry_position) = group.entries.iter().position(|entry| entry.id == id) else {
        return false;
    };
    group.entries.swap_remove(entry_position);
    if group.entries.is_empty() {
        validators.swap_remove(validator_position);
    }
    if validators.is_empty() {
        snapshot.undelegations_v2.remove(&delegator);
    }
    true
}

fn cleanup_slashing_jailed_validators(snapshot: &mut DposSnapshot, block_number: u64) {
    snapshot.slashing_jailed_validators.retain(|validator| {
        snapshot
            .slashing_jail_blocks
            .get(validator)
            .is_some_and(|jail_block| *jail_block > block_number)
    });
}

fn find_undelegation_v2(
    snapshot: &DposSnapshot,
    delegator: [u8; 20],
    validator: [u8; 20],
    id: u64,
) -> Option<&DposUndelegationV2Entry> {
    snapshot
        .undelegations_v2
        .get(&delegator)?
        .iter()
        .find(|group| group.validator == validator)?
        .entries
        .iter()
        .find(|entry| entry.id == id)
}

fn dpos_undelegations_v2_for_delegator(
    snapshot: &DposSnapshot,
    delegator: [u8; 20],
) -> Vec<([u8; 20], &DposUndelegationV2Entry)> {
    snapshot
        .undelegations_v2
        .get(&delegator)
        .map(|validators| {
            validators
                .iter()
                .flat_map(|group| group.entries.iter().map(|entry| (group.validator, entry)))
                .collect()
        })
        .unwrap_or_default()
}

fn dpos_undelegations_v2_count_for_validator(snapshot: &DposSnapshot, validator: [u8; 20]) -> u16 {
    let count = snapshot
        .undelegations_v2
        .values()
        .flat_map(|validators| validators.iter())
        .filter(|group| group.validator == validator)
        .map(|group| group.entries.len())
        .sum::<usize>();
    u16::try_from(count).unwrap_or(u16::MAX)
}

fn dpos_undelegations_v2_storage_read_count(
    snapshot: &DposSnapshot,
    delegator: [u8; 20],
    batch: u32,
) -> Result<u64, anyhow::Error> {
    let mut to_skip = u64::from(batch)
        .checked_mul(DPOS_GET_UNDELEGATIONS_MAX_COUNT as u64)
        .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 batch offset overflow"))?;
    let mut reads = 0u64;
    let mut processed = 0u64;
    let Some(validators) = snapshot.undelegations_v2.get(&delegator) else {
        return Ok(0);
    };
    for group in validators {
        reads = reads
            .checked_add(2)
            .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 read count overflow"))?;
        let undelegations_count = group.entries.len() as u64;
        if undelegations_count <= to_skip {
            to_skip -= undelegations_count;
            continue;
        }
        let remaining_page = (DPOS_GET_UNDELEGATIONS_MAX_COUNT as u64)
            .checked_sub(processed)
            .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 processed count overflow"))?;
        let left = undelegations_count - to_skip;
        let to_process = left.min(remaining_page);
        processed = processed
            .checked_add(to_process)
            .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 processed count overflow"))?;
        reads =
            reads
                .checked_add(to_process.checked_mul(2).ok_or_else(|| {
                    anyhow::anyhow!("getUndelegationsV2 item read count overflow")
                })?)
                .ok_or_else(|| anyhow::anyhow!("getUndelegationsV2 read count overflow"))?;
        to_skip = 0;
        if processed == DPOS_GET_UNDELEGATIONS_MAX_COUNT as u64 {
            break;
        }
    }
    Ok(reads)
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
        let item_count = item.item_count()?;
        if item_count != 5 && item_count != 6 {
            anyhow::bail!("DPoS snapshot metadata entry must contain exactly five or six items");
        }
        map.insert(
            decode_address(&item.at(0)?, "validator metadata address")?,
            DposValidatorMetadata {
                owner: decode_address(&item.at(1)?, "validator metadata owner")?,
                commission: item.val_at(2)?,
                last_commission_change: if item_count >= 6 { item.val_at(3)? } else { 0 },
                description: item.val_at(if item_count >= 6 { 4 } else { 3 })?,
                endpoint: item.val_at(if item_count >= 6 { 5 } else { 4 })?,
            },
        );
    }
    Ok(map)
}

fn decode_address_vec(rlp: &Rlp<'_>, field: &str) -> Result<Vec<[u8; 20]>, anyhow::Error> {
    rlp.iter()
        .map(|item| decode_address(&item, field))
        .collect()
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

fn encode_native_receipts(receipts: &[NativeReceipt]) -> Vec<Vec<u8>> {
    receipts.iter().map(encode_receipt_rlp).collect()
}

fn encode_receipt_rlp(receipt: &NativeReceipt) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(5);
    stream.append(&receipt.status_code);
    stream.append(&receipt.gas_used);
    stream.append(&receipt.cumulative_gas_used);
    stream.begin_list(receipt.logs.len());
    for log in &receipt.logs {
        stream.begin_list(3);
        stream.append(&log.address.as_slice());
        stream.begin_list(log.topics.len());
        for topic in &log.topics {
            stream.append(&topic.as_slice());
        }
        stream.append(&log.data.as_slice());
    }
    if let Some(address) = receipt.new_contract_address {
        stream.append(&address.as_slice());
    } else {
        stream.append(&0u8);
    }
    stream.out().to_vec()
}

fn block_log_bloom(receipts: &[NativeReceipt]) -> Vec<u8> {
    let mut bloom = vec![0u8; 256];
    for receipt in receipts {
        for log in &receipt.logs {
            add_bloom_value(&mut bloom, &log.address);
            for topic in &log.topics {
                add_bloom_value(&mut bloom, topic);
            }
        }
    }
    bloom
}

fn final_chain_bloom_index_units(level_count: u64) -> Result<u64, anyhow::Error> {
    let mut units = 1u64;
    for _ in 0..level_count {
        units = units
            .checked_mul(FINAL_CHAIN_BLOOM_INDEX_SIZE as u64)
            .ok_or_else(|| anyhow::anyhow!("final-chain bloom index unit overflow"))?;
    }
    Ok(units)
}

fn log_bloom_contains(stored: &FinalChainLogBloom, query: &FinalChainLogBloom) -> bool {
    stored
        .iter()
        .zip(query.iter())
        .all(|(stored, query)| stored & query == *query)
}

fn add_bloom_value(bloom: &mut [u8], value: &[u8]) {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(value);
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    for offset in [0usize, 2, 4] {
        let index = (((hash[offset] as usize) << 8) | hash[offset + 1] as usize) & 2047;
        bloom[255 - index / 8] |= 1 << (index % 8);
    }
}

fn dpos_delegated_log(
    delegator: [u8; 20],
    validator: [u8; 20],
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_DELEGATED_TOPIC,
        vec![address_topic(delegator), address_topic(validator)],
        amount,
    )
}

fn dpos_undelegated_log(
    delegator: [u8; 20],
    validator: [u8; 20],
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_UNDELEGATED_TOPIC,
        vec![address_topic(delegator), address_topic(validator)],
        amount,
    )
}

fn dpos_undelegated_v2_log(
    delegator: [u8; 20],
    validator: [u8; 20],
    id: u64,
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_UNDELEGATED_V2_TOPIC,
        vec![
            address_topic(delegator),
            address_topic(validator),
            u64_topic(id),
        ],
        amount,
    )
}

fn dpos_undelegate_confirmed_v2_log(
    delegator: [u8; 20],
    validator: [u8; 20],
    id: u64,
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_UNDELEGATE_CONFIRMED_V2_TOPIC,
        vec![
            address_topic(delegator),
            address_topic(validator),
            u64_topic(id),
        ],
        amount,
    )
}

fn dpos_undelegate_canceled_v2_log(
    delegator: [u8; 20],
    validator: [u8; 20],
    id: u64,
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_UNDELEGATE_CANCELED_V2_TOPIC,
        vec![
            address_topic(delegator),
            address_topic(validator),
            u64_topic(id),
        ],
        amount,
    )
}

fn dpos_redelegated_log(
    delegator: [u8; 20],
    from: [u8; 20],
    to: [u8; 20],
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_REDELEGATED_TOPIC,
        vec![
            address_topic(delegator),
            address_topic(from),
            address_topic(to),
        ],
        amount,
    )
}

fn dpos_rewards_claimed_log(
    account: [u8; 20],
    validator: [u8; 20],
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_REWARDS_CLAIMED_TOPIC,
        vec![address_topic(account), address_topic(validator)],
        amount,
    )
}

fn dpos_commission_rewards_claimed_log(
    account: [u8; 20],
    validator: [u8; 20],
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    dpos_amount_log(
        DPOS_COMMISSION_REWARDS_CLAIMED_TOPIC,
        vec![address_topic(account), address_topic(validator)],
        amount,
    )
}

fn dpos_validator_registered_log(validator: [u8; 20]) -> ReceiptLog {
    ReceiptLog {
        address: DPOS_CONTRACT_ADDRESS,
        topics: vec![DPOS_VALIDATOR_REGISTERED_TOPIC, address_topic(validator)],
        data: Vec::new(),
    }
}

fn dpos_validator_info_set_log(validator: [u8; 20]) -> ReceiptLog {
    ReceiptLog {
        address: DPOS_CONTRACT_ADDRESS,
        topics: vec![DPOS_VALIDATOR_INFO_SET_TOPIC, address_topic(validator)],
        data: Vec::new(),
    }
}

fn dpos_commission_set_log(
    validator: [u8; 20],
    commission: u16,
) -> Result<ReceiptLog, anyhow::Error> {
    Ok(ReceiptLog {
        address: DPOS_CONTRACT_ADDRESS,
        topics: vec![DPOS_COMMISSION_SET_TOPIC, address_topic(validator)],
        data: abi_word_from_u64(u64::from(commission)).to_vec(),
    })
}

fn slashing_jailed_log(
    validator: [u8; 20],
    start_block: u64,
    end_block: u64,
) -> Result<ReceiptLog, anyhow::Error> {
    Ok(ReceiptLog {
        address: SLASHING_CONTRACT_ADDRESS,
        topics: vec![
            SLASHING_JAILED_TOPIC,
            address_topic(validator),
            u64_topic(start_block),
            u64_topic(end_block),
        ],
        data: abi_word_from_u64(u64::from(SLASHING_DOUBLE_VOTING_BEHAVIOUR)).to_vec(),
    })
}

fn dpos_amount_log(
    event_topic: [u8; 32],
    mut indexed_topics: Vec<[u8; 32]>,
    amount: U256,
) -> Result<ReceiptLog, anyhow::Error> {
    let mut topics = Vec::with_capacity(indexed_topics.len() + 1);
    topics.push(event_topic);
    topics.append(&mut indexed_topics);
    Ok(ReceiptLog {
        address: DPOS_CONTRACT_ADDRESS,
        topics,
        data: abi_word_from_u256_bytes(&u256_to_big_endian(amount))?.to_vec(),
    })
}

fn u64_topic(value: u64) -> [u8; 32] {
    abi_word_from_u64(value)
}

fn address_topic(address: [u8; 20]) -> [u8; 32] {
    abi_word_from_address(address)
}

fn is_dpos_rewards_claimed_log(log: &ReceiptLog) -> bool {
    log.topics.first() == Some(&DPOS_REWARDS_CLAIMED_TOPIC)
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

fn decode_abi_u32_argument(
    input: &[u8],
    start: usize,
    function_name: &str,
) -> Result<u32, anyhow::Error> {
    if input.len() < start + 32 {
        anyhow::bail!("{function_name} input is shorter than selector plus ABI argument");
    }
    anyhow::ensure!(
        input[start..start + 28].iter().all(|byte| *byte == 0),
        "{function_name} argument does not fit into uint32"
    );
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&input[start + 28..start + 32]);
    Ok(u32::from_be_bytes(bytes))
}

fn decode_abi_word_as_u32(input: &[u8], offset: usize, field: &str) -> Result<u32, anyhow::Error> {
    let word = abi_word(input, offset, field)?;
    if word[..28].iter().any(|byte| *byte != 0) {
        anyhow::bail!("{field} argument does not fit into uint32");
    }
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&word[28..32]);
    Ok(u32::from_be_bytes(bytes))
}

fn decode_abi_word_as_u64(input: &[u8], offset: usize, field: &str) -> Result<u64, anyhow::Error> {
    let word = abi_word(input, offset, field)?;
    if word[..24].iter().any(|byte| *byte != 0) {
        anyhow::bail!("{field} argument does not fit into uint64");
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&word[24..32]);
    Ok(u64::from_be_bytes(bytes))
}

/// Encodes a `u64` as a right-aligned Solidity ABI word.
fn abi_word_from_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn abi_word_from_bool(value: bool) -> [u8; 32] {
    abi_word_from_u64(if value { 1 } else { 0 })
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

fn encode_abi_address_array(addresses: &[[u8; 20]]) -> Vec<u8> {
    let mut output = Vec::with_capacity(64 + addresses.len() * 32);
    output.extend_from_slice(&abi_word_from_u64(32));
    output.extend_from_slice(&abi_word_from_u64(addresses.len() as u64));
    for address in addresses {
        output.extend_from_slice(&abi_word_from_address(*address));
    }
    output
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

fn encode_dpos_undelegation_v2_payload(
    output: &mut Vec<u8>,
    snapshot: &DposSnapshot,
    validator: [u8; 20],
    entry: &DposUndelegationV2Entry,
) -> Result<(), anyhow::Error> {
    output.extend_from_slice(&abi_word_from_u256_bytes(&entry.amount)?);
    output.extend_from_slice(&abi_word_from_u64(entry.block));
    output.extend_from_slice(&abi_word_from_address(validator));
    output.extend_from_slice(&abi_word_from_bool(
        snapshot.total_stakes.contains_key(&validator),
    ));
    output.extend_from_slice(&abi_word_from_u64(entry.id));
    Ok(())
}

/// Formats a four-byte call selector without a `0x` prefix.
fn selector_hex(selector: [u8; 4]) -> String {
    selector
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

/// Decodes Rust-supported slashing contract method payloads.
///
/// The slashing precompile exposes read methods plus
/// `commitDoubleVotingProof(bytes,bytes)`. Finalization stores malformed proof
/// payloads as contract-failure transactions so ordinary bad user input cannot
/// abort block execution.
fn decode_slashing_transaction(input: &[u8]) -> Result<SlashingTransaction, anyhow::Error> {
    if input.len() < 4 {
        return Ok(SlashingTransaction::MethodNotSupported);
    }
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&input[..4]);
    match selector {
        SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR => {
            Ok(SlashingTransaction::CommitDoubleVotingProof(Box::new(
                verify_legacy_double_voting_proof_call_data(input).map_err(|err| err.to_string()),
            )))
        }
        _ => Ok(SlashingTransaction::MethodNotSupported),
    }
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
    block_number: u64,
    fix_claim_all_block_num: u64,
    cornus_period: u64,
) -> Result<DposTransaction, anyhow::Error> {
    if input.len() < 4 {
        anyhow::bail!("Rust FinalChain::finalize DPoS transaction input is missing selector");
    }
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&input[..4]);
    match selector {
        DPOS_CLAIM_REWARDS_SELECTOR => {
            let validator = decode_abi_address_argument(input, "claimRewards(address)")?;
            Ok(DposTransaction::ClaimRewards {
                delegator: owner,
                validator,
            })
        }
        DPOS_CLAIM_COMMISSION_REWARDS_SELECTOR => {
            let validator = decode_abi_address_argument(input, "claimCommissionRewards(address)")?;
            Ok(DposTransaction::ClaimCommissionRewards { owner, validator })
        }
        DPOS_SET_VALIDATOR_INFO_SELECTOR => {
            let (validator, description, endpoint) = decode_dpos_set_validator_info(input)?;
            Ok(DposTransaction::SetValidatorInfo {
                owner,
                validator,
                description,
                endpoint,
            })
        }
        DPOS_SET_COMMISSION_SELECTOR => {
            let validator = decode_abi_address_argument(input, "setCommission(address,uint16)")?;
            let commission = decode_abi_word_as_u16(input, 4 + 32, "setCommission commission")?;
            Ok(DposTransaction::SetCommission {
                owner,
                validator,
                commission,
            })
        }
        DPOS_CLAIM_ALL_REWARDS_SELECTOR => {
            if input.len() != 4 {
                anyhow::bail!("claimAllRewards input is malformed");
            }
            Ok(DposTransaction::ClaimAllRewards {
                delegator: owner,
                batch: None,
            })
        }
        DPOS_CLAIM_ALL_REWARDS_BATCH_SELECTOR => {
            if block_number >= fix_claim_all_block_num {
                anyhow::bail!(
                    "Rust FinalChain::finalize unsupported DPoS selector 0x{}",
                    selector_hex(selector)
                );
            }
            if input.len() != 4 + 32 {
                anyhow::bail!("claimAllRewards(uint32) input is malformed");
            }
            let batch = decode_abi_word_as_u32(input, 4, "claimAllRewards(uint32) batch")?;
            Ok(DposTransaction::ClaimAllRewards {
                delegator: owner,
                batch: Some(batch),
            })
        }
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
        DPOS_UNDELEGATE_V2_SELECTOR => {
            if block_number < cornus_period {
                return Ok(DposTransaction::MethodNotSupported);
            }
            if input.len() < 4 + 2 * 32 {
                anyhow::bail!("undelegateV2 input is shorter than selector plus ABI head");
            }
            let validator = decode_abi_address_argument(input, "undelegateV2(address,...)")?;
            let amount = decode_abi_word_as_vec(input, 4 + 32, "undelegateV2 amount")?;
            Ok(DposTransaction::UndelegateV2 {
                delegator: owner,
                validator,
                amount,
            })
        }
        DPOS_CONFIRM_UNDELEGATE_V2_SELECTOR => {
            if block_number < cornus_period {
                return Ok(DposTransaction::MethodNotSupported);
            }
            if input.len() < 4 + 2 * 32 {
                anyhow::bail!("confirmUndelegateV2 input is shorter than selector plus ABI head");
            }
            let validator =
                decode_abi_address_argument(input, "confirmUndelegateV2(address,uint64)")?;
            let id = decode_abi_word_as_u64(input, 4 + 32, "confirmUndelegateV2 id")?;
            Ok(DposTransaction::ConfirmUndelegateV2 {
                delegator: owner,
                validator,
                id,
            })
        }
        DPOS_CANCEL_UNDELEGATE_V2_SELECTOR => {
            if block_number < cornus_period {
                return Ok(DposTransaction::MethodNotSupported);
            }
            if input.len() < 4 + 2 * 32 {
                anyhow::bail!("cancelUndelegateV2 input is shorter than selector plus ABI head");
            }
            let validator =
                decode_abi_address_argument(input, "cancelUndelegateV2(address,uint64)")?;
            let id = decode_abi_word_as_u64(input, 4 + 32, "cancelUndelegateV2 id")?;
            Ok(DposTransaction::CancelUndelegateV2 {
                delegator: owner,
                validator,
                id,
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
            last_commission_change: 0,
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

fn decode_dpos_set_validator_info(
    input: &[u8],
) -> Result<([u8; 20], String, String), anyhow::Error> {
    if input.len() < 4 + 3 * 32 {
        anyhow::bail!("setValidatorInfo input is shorter than selector plus ABI head");
    }
    let validator =
        decode_abi_address_argument_with_offset(input, 4, "setValidatorInfo validator")?;
    let description_offset =
        decode_abi_word_as_usize(input, 4 + 32, "setValidatorInfo description offset")?;
    let endpoint_offset =
        decode_abi_word_as_usize(input, 4 + 2 * 32, "setValidatorInfo endpoint offset")?;
    let description =
        decode_abi_dynamic_string(input, description_offset, "setValidatorInfo description")?;
    let endpoint = decode_abi_dynamic_string(input, endpoint_offset, "setValidatorInfo endpoint")?;
    Ok((validator, description, endpoint))
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

fn dpos_claim_all_rewards_item_count(
    snapshot: &DposSnapshot,
    delegator: [u8; 20],
    batch: Option<u32>,
) -> Result<u64, anyhow::Error> {
    let validator_count = snapshot
        .delegator_validators
        .get(&delegator)
        .map(|validators| validators.len())
        .unwrap_or(0);
    let validator_count = u64::try_from(validator_count)
        .map_err(|_| anyhow::anyhow!("DPoS delegator validator count does not fit into u64"))?;
    match batch {
        None => Ok(validator_count),
        Some(batch) => dpos_batch_items_count(
            validator_count,
            batch,
            DPOS_CLAIM_ALL_REWARDS_MAX_COUNT as u64,
        ),
    }
}

fn update_dpos_claim_gas_snapshot(
    snapshot: &mut DposSnapshot,
    dpos_tx: &DposTransaction,
) -> Result<(), anyhow::Error> {
    match dpos_tx {
        DposTransaction::Register(registration) => {
            if !u256_from_big_endian(&registration.stake).is_zero() {
                add_delegator_validator(
                    &mut snapshot.delegator_validators,
                    registration.validator,
                    registration.validator,
                );
                snapshot
                    .delegations
                    .entry(registration.validator)
                    .or_default()
                    .insert(registration.validator, registration.stake.clone());
            }
        }
        DposTransaction::Delegate {
            delegator,
            validator,
            amount,
        } => {
            add_delegator_validator(&mut snapshot.delegator_validators, *delegator, *validator);
            let current = snapshot
                .delegations
                .entry(*validator)
                .or_default()
                .get(delegator)
                .map(|bytes| u256_from_big_endian(bytes))
                .unwrap_or_default();
            let next = current
                .checked_add(u256_from_big_endian(amount))
                .ok_or_else(|| {
                    anyhow::anyhow!("claimAllRewards gas snapshot delegation overflow")
                })?;
            snapshot
                .delegations
                .entry(*validator)
                .or_default()
                .insert(*delegator, u256_to_big_endian(next));
        }
        DposTransaction::Undelegate {
            delegator,
            validator,
            amount,
        }
        | DposTransaction::UndelegateV2 {
            delegator,
            validator,
            amount,
        } => {
            update_dpos_claim_gas_snapshot_remove(snapshot, *delegator, *validator, amount)?;
        }
        DposTransaction::Redelegate {
            delegator,
            from,
            to,
            amount,
        } => {
            update_dpos_claim_gas_snapshot_remove(snapshot, *delegator, *from, amount)?;
            add_delegator_validator(&mut snapshot.delegator_validators, *delegator, *to);
            let current = snapshot
                .delegations
                .entry(*to)
                .or_default()
                .get(delegator)
                .map(|bytes| u256_from_big_endian(bytes))
                .unwrap_or_default();
            let next = current
                .checked_add(u256_from_big_endian(amount))
                .ok_or_else(|| {
                    anyhow::anyhow!("claimAllRewards gas snapshot redelegation overflow")
                })?;
            snapshot
                .delegations
                .entry(*to)
                .or_default()
                .insert(*delegator, u256_to_big_endian(next));
        }
        DposTransaction::ClaimRewards { .. }
        | DposTransaction::ConfirmUndelegateV2 { .. }
        | DposTransaction::CancelUndelegateV2 { .. }
        | DposTransaction::ClaimCommissionRewards { .. }
        | DposTransaction::SetValidatorInfo { .. }
        | DposTransaction::SetCommission { .. }
        | DposTransaction::ClaimAllRewards { .. }
        | DposTransaction::MethodNotSupported => {}
    }
    Ok(())
}

fn update_dpos_claim_gas_snapshot_remove(
    snapshot: &mut DposSnapshot,
    delegator: [u8; 20],
    validator: [u8; 20],
    amount: &[u8],
) -> Result<(), anyhow::Error> {
    let Some(delegations) = snapshot.delegations.get_mut(&validator) else {
        return Ok(());
    };
    let Some(current) = delegations
        .get(&delegator)
        .map(|bytes| u256_from_big_endian(bytes))
    else {
        return Ok(());
    };
    let remove = u256_from_big_endian(amount);
    if current <= remove {
        delegations.remove(&delegator);
        remove_delegator_validator(&mut snapshot.delegator_validators, delegator, validator);
    } else {
        delegations.insert(delegator, u256_to_big_endian(current - remove));
    }
    Ok(())
}

fn dpos_batch_items_count(
    actual_count: u64,
    batch: u32,
    max_batch_items_count: u64,
) -> Result<u64, anyhow::Error> {
    if max_batch_items_count == 0 {
        anyhow::bail!("claimAllRewards max batch size must be greater than zero");
    }
    if actual_count == 0 {
        return Ok(1);
    }
    let batch_index = u64::from(batch);
    let start = batch_index
        .checked_mul(max_batch_items_count)
        .ok_or_else(|| anyhow::anyhow!("claimAllRewards batch start index overflow"))?;
    if start >= actual_count {
        return Ok(1);
    }
    Ok(max_batch_items_count.min(actual_count - start))
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
    receipts: Vec<NativeReceipt>,
    gas_used: u64,
    transaction_fees: Vec<([u8; 32], U256)>,
    contract_transactions: Vec<(usize, NativeContractTransaction)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeReceipt {
    status_code: u8,
    gas_used: u64,
    cumulative_gas_used: u64,
    logs: Vec<ReceiptLog>,
    new_contract_address: Option<[u8; 20]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReceiptLog {
    address: [u8; 20],
    topics: Vec<[u8; 32]>,
    data: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DposApplyOutcome {
    status_code: u8,
    logs: Vec<ReceiptLog>,
}

impl DposApplyOutcome {
    fn success(logs: Vec<ReceiptLog>) -> Self {
        Self {
            status_code: 1,
            logs,
        }
    }

    fn contract_failure() -> Self {
        Self {
            status_code: 0,
            logs: Vec::new(),
        }
    }
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
    UndelegateV2 {
        delegator: [u8; 20],
        validator: [u8; 20],
        amount: Vec<u8>,
    },
    ConfirmUndelegateV2 {
        delegator: [u8; 20],
        validator: [u8; 20],
        id: u64,
    },
    CancelUndelegateV2 {
        delegator: [u8; 20],
        validator: [u8; 20],
        id: u64,
    },
    Redelegate {
        delegator: [u8; 20],
        from: [u8; 20],
        to: [u8; 20],
        amount: Vec<u8>,
    },
    ClaimRewards {
        delegator: [u8; 20],
        validator: [u8; 20],
    },
    ClaimCommissionRewards {
        owner: [u8; 20],
        validator: [u8; 20],
    },
    SetValidatorInfo {
        owner: [u8; 20],
        validator: [u8; 20],
        description: String,
        endpoint: String,
    },
    SetCommission {
        owner: [u8; 20],
        validator: [u8; 20],
        commission: u16,
    },
    ClaimAllRewards {
        delegator: [u8; 20],
        batch: Option<u32>,
    },
    MethodNotSupported,
}

enum NativeContractTransaction {
    Dpos(DposTransaction),
    Slashing(SlashingTransaction),
}

enum SlashingTransaction {
    CommitDoubleVotingProof(Box<std::result::Result<VerifiedLegacyDoubleVotingProof, String>>),
    MethodNotSupported,
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

    fn receipt_logs(receipt_rlp: &[u8]) -> Vec<ReceiptLog> {
        let receipt = Rlp::new(receipt_rlp);
        let logs_rlp = receipt.at(3).unwrap();
        let mut logs = Vec::new();
        for index in 0..logs_rlp.item_count().unwrap() {
            let log_rlp = logs_rlp.at(index).unwrap();
            let address_bytes = log_rlp.at(0).unwrap().data().unwrap();
            let mut address = [0u8; 20];
            address.copy_from_slice(address_bytes);
            let topics_rlp = log_rlp.at(1).unwrap();
            let mut topics = Vec::new();
            for topic_index in 0..topics_rlp.item_count().unwrap() {
                let topic_bytes = topics_rlp.at(topic_index).unwrap().data().unwrap();
                let mut topic = [0u8; 32];
                topic.copy_from_slice(topic_bytes);
                topics.push(topic);
            }
            logs.push(ReceiptLog {
                address,
                topics,
                data: log_rlp.at(2).unwrap().data().unwrap().to_vec(),
            });
        }
        logs
    }

    fn bloom_query_for_value(value: &[u8]) -> FinalChainLogBloom {
        let mut bloom = [0u8; 256];
        add_bloom_value(&mut bloom, value);
        bloom
    }

    fn assert_dpos_amount_log(
        log: &ReceiptLog,
        event_topic: [u8; 32],
        indexed_topics: Vec<[u8; 32]>,
        amount: U256,
    ) {
        let mut topics = Vec::with_capacity(indexed_topics.len() + 1);
        topics.push(event_topic);
        topics.extend(indexed_topics);
        assert_eq!(log.address, DPOS_CONTRACT_ADDRESS);
        assert_eq!(log.topics, topics);
        assert_eq!(
            log.data,
            abi_word_from_u256_bytes(&u256_to_big_endian(amount))
                .unwrap()
                .to_vec()
        );
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

    fn slashing_call_request(block_number: u64, input: Vec<u8>) -> FinalChainCallRequest {
        FinalChainCallRequest {
            block_number,
            sender: [0u8; 20],
            receiver: Some(SLASHING_CONTRACT_ADDRESS),
            value: vec![],
            gas_price: vec![],
            gas_limit: 1_000_000,
            input,
        }
    }

    fn legacy_vrf_sortition_rlp(period: u64, round: u32, step: u32, proof_byte: u8) -> Vec<u8> {
        let mut stream = RlpStream::new_list(4);
        stream.append(&period);
        stream.append(&round);
        stream.append(&step);
        stream.append(&vec![proof_byte; 80]);
        stream.out().to_vec()
    }

    fn legacy_pbft_vote_hash(block_hash: H256, vrf_sortition_rlp: &[u8]) -> H256 {
        let mut stream = RlpStream::new_list(2);
        stream.append(&block_hash);
        stream.append(&vrf_sortition_rlp);
        keccak256(&stream.out())
    }

    fn signed_legacy_pbft_vote(
        signing_key: &SigningKey,
        block_hash: H256,
        vrf_sortition_rlp: &[u8],
    ) -> Vec<u8> {
        let vote_hash = legacy_pbft_vote_hash(block_hash, vrf_sortition_rlp);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(vote_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut stream = RlpStream::new_list(3);
        stream.append(&block_hash);
        stream.append(&vrf_sortition_rlp);
        stream.append(&signature_bytes);
        stream.out().to_vec()
    }

    fn abi_bytes_tail(value: &[u8]) -> Vec<u8> {
        let mut output = abi_word_from_u64(value.len() as u64).to_vec();
        output.extend_from_slice(value);
        output.resize(32 + value.len().div_ceil(32) * 32, 0);
        output
    }

    fn commit_double_voting_proof_input(vote_a: &[u8], vote_b: &[u8]) -> Vec<u8> {
        let tail_a = abi_bytes_tail(vote_a);
        let tail_b = abi_bytes_tail(vote_b);
        let offset_a = 64u64;
        let offset_b = offset_a + tail_a.len() as u64;
        let mut input = SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_u64(offset_a));
        input.extend_from_slice(&abi_word_from_u64(offset_b));
        input.extend_from_slice(&tail_a);
        input.extend_from_slice(&tail_b);
        input
    }

    fn get_jail_block_input(validator: [u8; 20]) -> Vec<u8> {
        let mut input = SLASHING_GET_JAIL_BLOCK_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input
    }

    fn get_jailed_validators_input() -> Vec<u8> {
        SLASHING_GET_JAILED_VALIDATORS_SELECTOR.to_vec()
    }

    fn get_validator_input(validator: [u8; 20]) -> Vec<u8> {
        let mut input = DPOS_GET_VALIDATOR_SELECTOR.to_vec();
        input.extend_from_slice(&[0u8; 12]);
        input.extend_from_slice(&validator);
        input
    }

    fn get_total_delegation_input(delegator: [u8; 20]) -> Vec<u8> {
        let mut input = DPOS_GET_TOTAL_DELEGATION_SELECTOR.to_vec();
        input.extend_from_slice(&[0u8; 12]);
        input.extend_from_slice(&delegator);
        input
    }

    fn get_delegations_input(delegator: [u8; 20], batch: u32) -> Vec<u8> {
        let mut input = DPOS_GET_DELEGATIONS_SELECTOR.to_vec();
        input.extend_from_slice(&[0u8; 12]);
        input.extend_from_slice(&delegator);
        input.extend_from_slice(&[0u8; 28]);
        input.extend_from_slice(&batch.to_be_bytes());
        input
    }

    fn get_validators_input(batch: u32) -> Vec<u8> {
        let mut input = DPOS_GET_VALIDATORS_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_u64(u64::from(batch)));
        input
    }

    fn get_validators_for_input(owner: [u8; 20], batch: u32) -> Vec<u8> {
        let mut input = DPOS_GET_VALIDATORS_FOR_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(owner));
        input.extend_from_slice(&abi_word_from_u64(u64::from(batch)));
        input
    }

    fn assert_single_delegation(
        final_chain: &FinalChain,
        block_number: u64,
        delegator: [u8; 20],
        validator: [u8; 20],
        stake: U256,
        rewards: U256,
    ) {
        let total_delegation = final_chain
            .call(dpos_call_request(
                block_number,
                get_total_delegation_input(delegator),
            ))
            .unwrap();
        assert_eq!(u256_from_big_endian(&total_delegation.code_retval), stake);

        let delegations = final_chain
            .call(dpos_call_request(
                block_number,
                get_delegations_input(delegator, 0),
            ))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&delegations.code_retval[0..32]),
            U256::from(64u64)
        );
        assert_eq!(
            u256_from_big_endian(&delegations.code_retval[32..64]),
            U256::one()
        );
        assert_eq!(
            u256_from_big_endian(&delegations.code_retval[64..96]),
            U256::one()
        );
        assert_eq!(&delegations.code_retval[108..128], &validator);
        assert_eq!(
            u256_from_big_endian(&delegations.code_retval[128..160]),
            stake
        );
        assert_eq!(
            u256_from_big_endian(&delegations.code_retval[160..192]),
            rewards
        );
    }

    fn validator_page_addresses(output: &[u8]) -> (Vec<[u8; 20]>, bool) {
        assert_eq!(u256_from_big_endian(&output[0..32]), U256::from(64u64));
        let is_end = u256_from_big_endian(&output[32..64]) == U256::one();
        let array_start = 64usize;
        let len = u256_from_big_endian(&output[array_start..array_start + 32]).as_usize();
        let mut addresses = Vec::with_capacity(len);
        for index in 0..len {
            let offset_start = array_start + 32 + index * 32;
            let payload_offset =
                u256_from_big_endian(&output[offset_start..offset_start + 32]).as_usize();
            let payload_start = array_start + payload_offset;
            let mut address = [0u8; 20];
            address.copy_from_slice(&output[payload_start + 12..payload_start + 32]);
            addresses.push(address);
        }
        (addresses, is_end)
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

    fn claim_rewards_input(validator: [u8; 20]) -> Vec<u8> {
        let mut input = DPOS_CLAIM_REWARDS_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input
    }

    fn claim_commission_rewards_input(validator: [u8; 20]) -> Vec<u8> {
        let mut input = DPOS_CLAIM_COMMISSION_REWARDS_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input
    }

    fn set_commission_input(validator: [u8; 20], commission: u16) -> Vec<u8> {
        let mut input = DPOS_SET_COMMISSION_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u64(u64::from(commission)));
        input
    }

    fn set_validator_info_input(validator: [u8; 20], description: &str, endpoint: &str) -> Vec<u8> {
        let description_offset = 3 * 32;
        let endpoint_offset =
            description_offset + abi_dynamic_string_tail_len(description).unwrap();
        let mut input = DPOS_SET_VALIDATOR_INFO_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_bytes_offset(description_offset));
        input.extend_from_slice(&abi_word_from_bytes_offset(endpoint_offset));
        input.extend_from_slice(&abi_string_tail(description).unwrap());
        input.extend_from_slice(&abi_string_tail(endpoint).unwrap());
        input
    }

    fn claim_all_rewards_input() -> Vec<u8> {
        DPOS_CLAIM_ALL_REWARDS_SELECTOR.to_vec()
    }

    fn claim_all_rewards_batch_input(batch: u32) -> Vec<u8> {
        let mut input = DPOS_CLAIM_ALL_REWARDS_BATCH_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_u64(u64::from(batch)));
        input
    }

    #[test]
    fn decode_dpos_transaction_gates_legacy_claim_all_batch_selector() {
        let owner = [0x44; 20];
        let decoded =
            decode_dpos_transaction(&claim_all_rewards_batch_input(7), owner, 9, 10, 0).unwrap();
        match decoded {
            DposTransaction::ClaimAllRewards {
                delegator,
                batch: Some(7),
            } => assert_eq!(delegator, owner),
            _ => panic!("expected legacy batched claimAllRewards"),
        }

        let err = match decode_dpos_transaction(&claim_all_rewards_batch_input(0), owner, 10, 10, 0)
        {
            Ok(_) => {
                panic!("expected legacy batched claimAllRewards to be rejected at fix boundary")
            }
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("unsupported DPoS selector 0x09b72e00")
        );

        let mut malformed_batch_input = claim_all_rewards_batch_input(1);
        malformed_batch_input.push(0);
        let err = match decode_dpos_transaction(&malformed_batch_input, owner, 9, 10, 0) {
            Ok(_) => panic!("expected malformed legacy batched claimAllRewards to be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("claimAllRewards(uint32) input is malformed")
        );

        let decoded =
            decode_dpos_transaction(&claim_all_rewards_input(), owner, 10, 10, 0).unwrap();
        match decoded {
            DposTransaction::ClaimAllRewards {
                delegator,
                batch: None,
            } => assert_eq!(delegator, owner),
            _ => panic!("expected current no-arg claimAllRewards"),
        }

        let validator = [0x45; 20];
        let decoded =
            decode_dpos_transaction(&claim_commission_rewards_input(validator), owner, 10, 10, 0)
                .unwrap();
        match decoded {
            DposTransaction::ClaimCommissionRewards {
                owner: decoded_owner,
                validator: decoded_validator,
            } => {
                assert_eq!(decoded_owner, owner);
                assert_eq!(decoded_validator, validator);
            }
            _ => panic!("expected claimCommissionRewards"),
        }

        let decoded =
            decode_dpos_transaction(&set_commission_input(validator, 1250), owner, 10, 10, 0)
                .unwrap();
        match decoded {
            DposTransaction::SetCommission {
                owner: decoded_owner,
                validator: decoded_validator,
                commission,
            } => {
                assert_eq!(decoded_owner, owner);
                assert_eq!(decoded_validator, validator);
                assert_eq!(commission, 1250);
            }
            _ => panic!("expected setCommission"),
        }

        let decoded = decode_dpos_transaction(
            &set_validator_info_input(validator, "new description", "new endpoint"),
            owner,
            10,
            10,
            0,
        )
        .unwrap();
        match decoded {
            DposTransaction::SetValidatorInfo {
                owner: decoded_owner,
                validator: decoded_validator,
                description,
                endpoint,
            } => {
                assert_eq!(decoded_owner, owner);
                assert_eq!(decoded_validator, validator);
                assert_eq!(description, "new description");
                assert_eq!(endpoint, "new endpoint");
            }
            _ => panic!("expected setValidatorInfo"),
        }
    }

    #[test]
    fn claim_all_batch_item_count_matches_legacy_page_gas_rules() {
        assert_eq!(dpos_batch_items_count(0, 0, 10).unwrap(), 1);
        assert_eq!(dpos_batch_items_count(12, 0, 10).unwrap(), 10);
        assert_eq!(dpos_batch_items_count(12, 1, 10).unwrap(), 2);
        assert_eq!(dpos_batch_items_count(12, 2, 10).unwrap(), 1);
    }

    fn undelegate_input(validator: [u8; 20], amount: U256) -> Vec<u8> {
        let mut input = DPOS_UNDELEGATE_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u256_bytes(&u256_to_big_endian(amount)).unwrap());
        input
    }

    fn undelegate_v2_input(validator: [u8; 20], amount: U256) -> Vec<u8> {
        let mut input = DPOS_UNDELEGATE_V2_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u256_bytes(&u256_to_big_endian(amount)).unwrap());
        input
    }

    fn confirm_undelegate_v2_input(validator: [u8; 20], id: u64) -> Vec<u8> {
        let mut input = DPOS_CONFIRM_UNDELEGATE_V2_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u64(id));
        input
    }

    fn cancel_undelegate_v2_input(validator: [u8; 20], id: u64) -> Vec<u8> {
        let mut input = DPOS_CANCEL_UNDELEGATE_V2_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u64(id));
        input
    }

    fn get_undelegations_v2_input(delegator: [u8; 20], batch: u32) -> Vec<u8> {
        let mut input = DPOS_GET_UNDELEGATIONS_V2_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(delegator));
        input.extend_from_slice(&abi_word_from_u64(u64::from(batch)));
        input
    }

    fn get_undelegation_v2_input(delegator: [u8; 20], validator: [u8; 20], id: u64) -> Vec<u8> {
        let mut input = DPOS_GET_UNDELEGATION_V2_SELECTOR.to_vec();
        input.extend_from_slice(&abi_word_from_address(delegator));
        input.extend_from_slice(&abi_word_from_address(validator));
        input.extend_from_slice(&abi_word_from_u64(id));
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
    fn pbft_final_chain_hash_preserves_delay_and_missing_header_semantics() {
        let path = temp_db_path("pbft-final-chain-hash");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut batch = storage.create_write_batch();
        let block_number = 7u64;
        let block_hash = [0xFE; 32];

        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainBlkHashByNumber,
                &block_number.to_le_bytes(),
                &block_hash,
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        let final_chain = new_final_chain(storage.clone(), 0, 0, vec![], vec![]);

        assert_eq!(final_chain.pbft_final_chain_hash(0).unwrap(), Some([0; 32]));
        assert_eq!(
            final_chain.pbft_final_chain_hash(block_number).unwrap(),
            Some(block_hash)
        );
        assert_eq!(final_chain.pbft_final_chain_hash(8).unwrap(), None);

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
    fn call_reads_dpos_validator_pages_in_legacy_order() {
        let path = temp_db_path("dpos-validator-pages");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let first_validator = [0x30; 20];
        let second_validator = [0x10; 20];
        let third_validator = [0x20; 20];
        let first_owner = [0x41; 20];
        let second_owner = [0x42; 20];

        let final_chain = new_final_chain_with_dpos(
            storage.clone(),
            vec![
                genesis_validator_with_metadata(
                    first_validator,
                    U256::from(10_000u64),
                    first_owner,
                    100,
                    "first",
                    "first endpoint",
                ),
                genesis_validator_with_metadata(
                    second_validator,
                    U256::from(10_000u64),
                    second_owner,
                    200,
                    "second",
                    "second endpoint",
                ),
                genesis_validator_with_metadata(
                    third_validator,
                    U256::from(10_000u64),
                    first_owner,
                    300,
                    "third",
                    "third endpoint",
                ),
            ],
            U256::from(1_000u64),
            U256::from(1_000u64),
            U256::from(30_000u64),
        );

        let all_validators = final_chain
            .call(dpos_call_request(0, get_validators_input(0)))
            .unwrap();
        assert_eq!(all_validators.gas_used, 15_000);
        assert_eq!(
            validator_page_addresses(&all_validators.code_retval),
            (
                vec![first_validator, second_validator, third_validator],
                true
            )
        );

        let out_of_range = final_chain
            .call(dpos_call_request(0, get_validators_input(1)))
            .unwrap();
        assert_eq!(out_of_range.gas_used, 5_000);
        assert_eq!(
            validator_page_addresses(&out_of_range.code_retval),
            (vec![], true)
        );

        let owner_validators = final_chain
            .call(dpos_call_request(
                0,
                get_validators_for_input(first_owner, 0),
            ))
            .unwrap();
        assert_eq!(owner_validators.gas_used, 100_000);
        assert_eq!(
            validator_page_addresses(&owner_validators.code_retval),
            (vec![first_validator, third_validator], true)
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
    fn finalize_block_executes_slashing_double_vote_and_filters_dpos_votes() {
        let path = temp_db_path("finalize-slashing-double-vote");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator_key = SigningKey::from_slice(&[0x51; 32]).unwrap();
        let validator_h160 = address_from_signing_key(&validator_key);
        let mut validator = [0u8; 20];
        validator.copy_from_slice(validator_h160.as_bytes());
        let submitter = [0x52; 20];
        let period = 1u64;
        let block_signing_key = SigningKey::from_slice(&[0x53; 32]).unwrap();
        let pbft = signed_pbft_block(&block_signing_key, period, 241);
        let sortition = legacy_vrf_sortition_rlp(period, 2, 4, 0x5a);
        let vote_a =
            signed_legacy_pbft_vote(&validator_key, H256::from_low_u64_be(100), &sortition);
        let vote_b =
            signed_legacy_pbft_vote(&validator_key, H256::from_low_u64_be(101), &sortition);
        let input = commit_double_voting_proof_input(&vote_a, &vote_b);
        let first_tx_rlp = vec![0xc1, 0xD1];
        let second_tx_rlp = vec![0xc1, 0xD2];
        let first_tx = test_transaction(
            0xD1,
            submitter,
            Some(SLASHING_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::zero(),
            100_000,
            input.clone(),
            first_tx_rlp.clone(),
        );
        let duplicate_tx = test_transaction(
            0xD2,
            submitter,
            Some(SLASHING_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            input,
            second_tx_rlp.clone(),
        );
        write_period_data(
            &storage,
            period,
            &pbft,
            &[first_tx_rlp.clone(), second_tx_rlp.clone()],
        );
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            300_000,
            0,
            vec![genesis_account(submitter, U256::from(1_000_000u64))],
            vec![genesis_validator(validator, U256::from(10_000u64))],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
            FinalChainRewardsConfig {
                magnolia_period: 1,
                cacti_period: 1,
                magnolia_jail_time: 2,
                cacti_jail_time: 2,
                ..Default::default()
            },
        )
        .unwrap();

        let (_header, receipts) = final_chain
            .finalize_block(pbft, vec![first_tx, duplicate_tx], vec![])
            .unwrap();

        assert_eq!(
            receipt_fields(&receipts[0]),
            (
                1,
                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS,
                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS
            )
        );
        assert_eq!(
            receipt_fields(&receipts[1]),
            (
                0,
                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS,
                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS * 2
            )
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].address, SLASHING_CONTRACT_ADDRESS);
        assert_eq!(
            logs[0].topics,
            vec![
                SLASHING_JAILED_TOPIC,
                address_topic(validator),
                u64_topic(period),
                u64_topic(period + 2)
            ]
        );
        assert_eq!(
            logs[0].data,
            abi_word_from_u64(u64::from(SLASHING_DOUBLE_VOTING_BEHAVIOUR)).to_vec()
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period, validator)
                .unwrap(),
            0
        );
        assert_eq!(
            final_chain.dpos_eligible_total_vote_count(period).unwrap(),
            0
        );
        let jail_block = final_chain
            .call(slashing_call_request(
                period,
                get_jail_block_input(validator),
            ))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&jail_block.code_retval),
            U256::from(period + 2)
        );
        let jailed_validators = final_chain
            .call(slashing_call_request(period, get_jailed_validators_input()))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&jailed_validators.code_retval[0..32]),
            U256::from(32)
        );
        assert_eq!(
            u256_from_big_endian(&jailed_validators.code_retval[32..64]),
            U256::from(1)
        );
        assert_eq!(&jailed_validators.code_retval[76..96], &validator);

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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
        assert_single_delegation(
            &final_chain,
            period,
            dag_author,
            dag_author,
            U256::from(10_000u64),
            U256::from(150u64),
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_claim_commission_rewards() {
        let path = temp_db_path("finalize-dpos-claim-commission-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let first_period = 1u64;
        let second_period = 2u64;
        let sender = [0x24; 20];
        let receiver = [0x25; 20];
        let validator = [0x26; 20];
        let owner = [0x27; 20];
        let genesis_validator = genesis_validator_with_metadata(
            validator,
            U256::from(10_000u64),
            owner,
            0,
            "validator",
            "endpoint",
        );
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            commission_change_delta: 0,
            commission_change_frequency: 0,
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[33u8; 32]).unwrap();
        let first_pbft = signed_pbft_block(&signing_key, first_period, 221);
        let first_transaction_rlp = vec![0xc1, 0x91];
        let first_transaction = test_transaction(
            0x91,
            sender,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::from(2u64),
            50_000,
            vec![],
            first_transaction_rlp.clone(),
        );
        write_period_data(
            &storage,
            first_period,
            &first_pbft,
            std::slice::from_ref(&first_transaction_rlp),
        );
        let final_chain = FinalChain::new(
            storage.clone(),
            100_000,
            0,
            vec![genesis_account(sender, U256::from(1_000_000u64))],
            vec![genesis_validator],
            genesis_dpos_config,
        )
        .unwrap();

        final_chain
            .finalize_block(
                first_pbft,
                vec![first_transaction.clone()],
                vec![FinalizationDagBlock {
                    author: validator,
                    difficulty: 0,
                    transaction_hashes: vec![first_transaction.hash],
                }],
            )
            .unwrap();
        let commission_reward = U256::from(VALUE_TRANSFER_GAS * 2);
        let validator_info = final_chain
            .call(dpos_call_request(
                first_period,
                get_validator_input(validator),
            ))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            commission_reward
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            commission_reward
        );

        let second_pbft = signed_pbft_block(&signing_key, second_period, 222);
        let claim_tx = test_transaction(
            0x92,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::zero(),
            100_000,
            claim_commission_rewards_input(validator),
            vec![0xc1, 0x92],
        );
        write_period_data(
            &storage,
            second_period,
            &second_pbft,
            std::slice::from_ref(&claim_tx.rlp),
        );

        let (_header_rlp, receipts) = final_chain
            .finalize_block(second_pbft, vec![claim_tx], vec![])
            .unwrap();
        assert_eq!(
            receipt_fields(&receipts[0]),
            (
                1,
                DPOS_CLAIM_COMMISSION_REWARDS_GAS,
                DPOS_CLAIM_COMMISSION_REWARDS_GAS,
            )
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_COMMISSION_REWARDS_CLAIMED_TOPIC,
            vec![address_topic(owner), address_topic(validator)],
            commission_reward,
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::zero()
        );
        assert_eq!(balance_of(&final_chain, owner), commission_reward);
        let validator_info = final_chain
            .call(dpos_call_request(
                second_period,
                get_validator_input(validator),
            ))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[64..96]),
            U256::zero()
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_validator_owner_updates_and_failed_receipts() {
        let path = temp_db_path("finalize-dpos-validator-owner-updates");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let period = 1u64;
        let validator = [0x30; 20];
        let owner = [0x31; 20];
        let wrong_owner = [0x32; 20];
        let genesis_validator = genesis_validator_with_metadata(
            validator,
            U256::from(10_000u64),
            owner,
            1_000,
            "old description",
            "old endpoint",
        );
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            commission_change_delta: 500,
            commission_change_frequency: 0,
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let signing_key = SigningKey::from_slice(&[34u8; 32]).unwrap();
        let pbft = signed_pbft_block(&signing_key, period, 231);
        let failed_info_tx = test_transaction(
            0xA0,
            wrong_owner,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::zero(),
            100_000,
            set_validator_info_input(validator, "bad description", "bad endpoint"),
            vec![0xc1, 0xA0],
        );
        let info_tx = test_transaction(
            0xA1,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::zero(),
            100_000,
            set_validator_info_input(validator, "new description", "new endpoint"),
            vec![0xc1, 0xA1],
        );
        let commission_tx = test_transaction(
            0xA2,
            owner,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            set_commission_input(validator, 1_500),
            vec![0xc1, 0xA2],
        );
        write_period_data(
            &storage,
            period,
            &pbft,
            &[
                failed_info_tx.rlp.clone(),
                info_tx.rlp.clone(),
                commission_tx.rlp.clone(),
            ],
        );
        let final_chain = FinalChain::new(
            storage.clone(),
            300_000,
            0,
            vec![
                genesis_account(owner, U256::from(1_000_000u64)),
                genesis_account(wrong_owner, U256::from(1_000_000u64)),
            ],
            vec![genesis_validator.clone()],
            genesis_dpos_config.clone(),
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(pbft, vec![failed_info_tx, info_tx, commission_tx], vec![])
            .unwrap();
        assert_eq!(
            receipt_fields(&receipts[0]),
            (0, DPOS_SET_VALIDATOR_INFO_GAS, DPOS_SET_VALIDATOR_INFO_GAS)
        );
        assert!(receipt_logs(&receipts[0]).is_empty());
        assert_eq!(
            receipt_fields(&receipts[1]),
            (
                1,
                DPOS_SET_VALIDATOR_INFO_GAS,
                DPOS_SET_VALIDATOR_INFO_GAS * 2,
            )
        );
        let info_logs = receipt_logs(&receipts[1]);
        assert_eq!(info_logs.len(), 1);
        assert_eq!(info_logs[0], dpos_validator_info_set_log(validator));
        assert_eq!(
            receipt_fields(&receipts[2]),
            (
                1,
                DPOS_SET_COMMISSION_GAS,
                DPOS_SET_VALIDATOR_INFO_GAS * 2 + DPOS_SET_COMMISSION_GAS,
            )
        );
        let commission_logs = receipt_logs(&receipts[2]);
        assert_eq!(commission_logs.len(), 1);
        assert_eq!(
            commission_logs[0],
            dpos_commission_set_log(validator, 1_500).unwrap()
        );

        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(validator)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[96..128]),
            U256::from(1_500u64)
        );
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[128..160]),
            U256::from(period)
        );
        let description_offset =
            u256_from_big_endian(&validator_info.code_retval[224..256]).as_usize();
        let endpoint_offset =
            u256_from_big_endian(&validator_info.code_retval[256..288]).as_usize();
        assert_abi_string_tail(
            &validator_info.code_retval,
            32,
            description_offset,
            "new description",
        );
        assert_abi_string_tail(
            &validator_info.code_retval,
            32,
            endpoint_offset,
            "new endpoint",
        );

        drop(final_chain);
        let final_chain = FinalChain::new(
            storage.clone(),
            300_000,
            0,
            vec![
                genesis_account(owner, U256::from(1_000_000u64)),
                genesis_account(wrong_owner, U256::from(1_000_000u64)),
            ],
            vec![genesis_validator],
            genesis_dpos_config,
        )
        .unwrap();
        let validator_info = final_chain
            .call(dpos_call_request(period, get_validator_input(validator)))
            .unwrap();
        assert_eq!(
            u256_from_big_endian(&validator_info.code_retval[128..160]),
            U256::from(period)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_auto_claim_on_dpos_stake_mutation() {
        let path = temp_db_path("finalize-dpos-mutation-with-pending-reward");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x68; 20];
        let receiver = [0x69; 20];
        let signing_key = SigningKey::from_slice(&[20u8; 32]).unwrap();
        let first_period = 1u64;
        let first_pbft = signed_pbft_block(&signing_key, first_period, 201);
        let transaction_rlp = vec![0xc1, 0xC1];
        let transaction = test_transaction(
            0xC1,
            validator,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::zero(),
            50_000,
            vec![],
            transaction_rlp.clone(),
        );
        let transaction_hash = transaction.hash;
        write_period_data(
            &storage,
            first_period,
            &first_pbft,
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
                [0x6a; 20],
                2_500,
                "validator",
                "endpoint",
            )],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
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

        final_chain
            .finalize_block(
                first_pbft,
                vec![transaction],
                vec![FinalizationDagBlock {
                    author: validator,
                    difficulty: 0,
                    transaction_hashes: vec![transaction_hash],
                }],
            )
            .unwrap();
        assert_single_delegation(
            &final_chain,
            first_period,
            validator,
            validator,
            U256::from(10_000u64),
            U256::from(150u64),
        );
        let first_period_contract_balance = balance_of(&final_chain, DPOS_CONTRACT_ADDRESS);

        let second_period = 2u64;
        let second_pbft = signed_pbft_block(&signing_key, second_period, 202);
        let delegate_tx = test_transaction(
            0xC2,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::from(1_000u64),
            U256::zero(),
            100_000,
            delegate_input(validator),
            vec![0xc1, 0xC2],
        );
        write_period_data(
            &storage,
            second_period,
            &second_pbft,
            std::slice::from_ref(&delegate_tx.rlp),
        );

        let (_header_rlp, receipts) = final_chain
            .finalize_block(second_pbft, vec![delegate_tx], vec![])
            .unwrap();
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 2);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_REWARDS_CLAIMED_TOPIC,
            vec![address_topic(validator), address_topic(validator)],
            U256::from(150u64),
        );
        assert_dpos_amount_log(
            &logs[1],
            DPOS_DELEGATED_TOPIC,
            vec![address_topic(validator), address_topic(validator)],
            U256::from(1_000u64),
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            first_period_contract_balance - U256::from(150u64) + U256::from(1_000u64)
        );
        assert_eq!(balance_of(&final_chain, validator), U256::from(999_149u64));

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_claim_all_rewards() {
        let path = temp_db_path("finalize-dpos-claim-all-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x70; 20];
        let receiver = [0x71; 20];
        let signing_key = SigningKey::from_slice(&[21u8; 32]).unwrap();
        let first_period = 1u64;
        let first_pbft = signed_pbft_block(&signing_key, first_period, 203);
        let transaction_rlp = vec![0xc1, 0xD1];
        let transaction = test_transaction(
            0xD1,
            validator,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::zero(),
            50_000,
            vec![],
            transaction_rlp.clone(),
        );
        let transaction_hash = transaction.hash;
        write_period_data(
            &storage,
            first_period,
            &first_pbft,
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
                [0x72; 20],
                2_500,
                "validator",
                "endpoint",
            )],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
                fix_claim_all_block_num: u64::MAX,
                ..Default::default()
            },
        )
        .unwrap();

        final_chain
            .finalize_block(
                first_pbft,
                vec![transaction],
                vec![FinalizationDagBlock {
                    author: validator,
                    difficulty: 0,
                    transaction_hashes: vec![transaction_hash],
                }],
            )
            .unwrap();
        assert_single_delegation(
            &final_chain,
            first_period,
            validator,
            validator,
            U256::from(10_000u64),
            U256::from(150u64),
        );
        let first_period_contract_balance = balance_of(&final_chain, DPOS_CONTRACT_ADDRESS);

        let second_period = 2u64;
        let second_pbft = signed_pbft_block(&signing_key, second_period, 204);
        let claim_tx = test_transaction(
            0xD2,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            claim_all_rewards_input(),
            vec![0xc1, 0xD2],
        );
        write_period_data(
            &storage,
            second_period,
            &second_pbft,
            std::slice::from_ref(&claim_tx.rlp),
        );

        let (_header_rlp, receipts) = final_chain
            .finalize_block(second_pbft, vec![claim_tx], vec![])
            .unwrap();
        let claim_all_gas = DPOS_CLAIM_REWARDS_GAS + DPOS_BATCH_GET_REWARDS_GAS;
        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, claim_all_gas, claim_all_gas)
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_REWARDS_CLAIMED_TOPIC,
            vec![address_topic(validator), address_topic(validator)],
            U256::from(150u64),
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            first_period_contract_balance - U256::from(150u64)
        );
        assert_eq!(
            balance_of(&final_chain, validator),
            U256::from(1_000_149u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_legacy_batch_claim_all_rewards() {
        let path = temp_db_path("finalize-dpos-claim-all-rewards-batch");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x70; 20];
        let receiver = [0x71; 20];
        let signing_key = SigningKey::from_slice(&[21u8; 32]).unwrap();
        let first_period = 1u64;
        let first_pbft = signed_pbft_block(&signing_key, first_period, 207);
        let transaction_rlp = vec![0xc1, 0xD3];
        let transaction = test_transaction(
            0xD3,
            validator,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::zero(),
            50_000,
            vec![],
            transaction_rlp.clone(),
        );
        let transaction_hash = transaction.hash;
        write_period_data(
            &storage,
            first_period,
            &first_pbft,
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
                [0x72; 20],
                2_500,
                "validator",
                "endpoint",
            )],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
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

        final_chain
            .finalize_block(
                first_pbft,
                vec![transaction],
                vec![FinalizationDagBlock {
                    author: validator,
                    difficulty: 0,
                    transaction_hashes: vec![transaction_hash],
                }],
            )
            .unwrap();
        assert_single_delegation(
            &final_chain,
            first_period,
            validator,
            validator,
            U256::from(10_000u64),
            U256::from(150u64),
        );
        let first_period_contract_balance = balance_of(&final_chain, DPOS_CONTRACT_ADDRESS);

        let second_period = 2u64;
        let second_pbft = signed_pbft_block(&signing_key, second_period, 208);
        let claim_tx = test_transaction(
            0xD4,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            claim_all_rewards_batch_input(0),
            vec![0xc1, 0xD4],
        );
        write_period_data(
            &storage,
            second_period,
            &second_pbft,
            std::slice::from_ref(&claim_tx.rlp),
        );

        let (_header_rlp, receipts) = final_chain
            .finalize_block(second_pbft, vec![claim_tx], vec![])
            .unwrap();
        let claim_all_gas = DPOS_CLAIM_REWARDS_GAS + DPOS_BATCH_GET_REWARDS_GAS;
        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, claim_all_gas, claim_all_gas)
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_REWARDS_CLAIMED_TOPIC,
            vec![address_topic(validator), address_topic(validator)],
            U256::from(150u64),
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            first_period_contract_balance - U256::from(150u64)
        );
        assert_eq!(
            balance_of(&final_chain, validator),
            U256::from(1_000_149u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_claim_rewards() {
        let path = temp_db_path("finalize-dpos-claim-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x73; 20];
        let receiver = [0x74; 20];
        let signing_key = SigningKey::from_slice(&[22u8; 32]).unwrap();
        let first_period = 1u64;
        let first_pbft = signed_pbft_block(&signing_key, first_period, 205);
        let transaction_rlp = vec![0xc1, 0xE1];
        let transaction = test_transaction(
            0xE1,
            validator,
            Some(receiver),
            0,
            U256::from(1u64),
            U256::zero(),
            50_000,
            vec![],
            transaction_rlp.clone(),
        );
        let transaction_hash = transaction.hash;
        write_period_data(
            &storage,
            first_period,
            &first_pbft,
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
                [0x75; 20],
                2_500,
                "validator",
                "endpoint",
            )],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
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

        final_chain
            .finalize_block(
                first_pbft,
                vec![transaction],
                vec![FinalizationDagBlock {
                    author: validator,
                    difficulty: 0,
                    transaction_hashes: vec![transaction_hash],
                }],
            )
            .unwrap();
        assert_single_delegation(
            &final_chain,
            first_period,
            validator,
            validator,
            U256::from(10_000u64),
            U256::from(150u64),
        );
        let first_period_contract_balance = balance_of(&final_chain, DPOS_CONTRACT_ADDRESS);

        let second_period = 2u64;
        let second_pbft = signed_pbft_block(&signing_key, second_period, 206);
        let claim_tx = test_transaction(
            0xE2,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            claim_rewards_input(validator),
            vec![0xc1, 0xE2],
        );
        write_period_data(
            &storage,
            second_period,
            &second_pbft,
            std::slice::from_ref(&claim_tx.rlp),
        );

        let (_header_rlp, receipts) = final_chain
            .finalize_block(second_pbft, vec![claim_tx], vec![])
            .unwrap();
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_REWARDS_CLAIMED_TOPIC,
            vec![address_topic(validator), address_topic(validator)],
            U256::from(150u64),
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            first_period_contract_balance - U256::from(150u64)
        );
        assert_eq!(
            balance_of(&final_chain, validator),
            U256::from(1_000_149u64)
        );

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn dpos_claim_checkpoint_preserves_other_delegator_rewards() {
        let path = temp_db_path("dpos-claim-preserves-other-delegator-rewards");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x76; 20];
        let delegator = [0x77; 20];
        let final_chain = FinalChain::new(
            storage.clone(),
            100_000,
            0,
            vec![],
            vec![genesis_validator(validator, U256::from(10_000u64))],
            GenesisDposConfig {
                eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
                vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
                validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .unwrap();
        let mut snapshot = final_chain.dpos_snapshot_at_finalized_block(0).unwrap();
        snapshot
            .delegations
            .entry(validator)
            .or_default()
            .insert(delegator, u256_to_big_endian(U256::from(10_000u64)));
        add_delegator_validator(&mut snapshot.delegator_validators, delegator, validator);
        snapshot
            .delegation_reward_cursors
            .entry(validator)
            .or_default()
            .insert(delegator, Vec::new());
        snapshot
            .delegator_rewards
            .insert(validator, u256_to_big_endian(U256::from(150u64)));
        snapshot
            .total_stakes
            .insert(validator, u256_to_big_endian(U256::from(20_000u64)));

        let mut accounts = HashMap::new();
        accounts.insert(
            DPOS_CONTRACT_ADDRESS,
            Account {
                balance: u256_to_big_endian(U256::from(150u64)),
                ..empty_account()
            },
        );

        final_chain
            .apply_dpos_delegator_reward_claim(&mut snapshot, &mut accounts, validator, validator)
            .unwrap();
        final_chain
            .apply_dpos_delegator_reward_claim(&mut snapshot, &mut accounts, validator, delegator)
            .unwrap();

        assert_eq!(
            u256_from_big_endian(&accounts.get(&validator).unwrap().balance),
            U256::from(75u64)
        );
        assert_eq!(
            u256_from_big_endian(&accounts.get(&delegator).unwrap().balance),
            U256::from(75u64)
        );
        assert_eq!(
            u256_from_big_endian(&accounts.get(&DPOS_CONTRACT_ADDRESS).unwrap().balance),
            U256::zero()
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
        assert_single_delegation(
            &final_chain,
            period,
            validator,
            validator,
            U256::from(10_000u64),
            U256::from(750u64),
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
                commission_change_delta: 0,
                commission_change_frequency: 0,
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 2);
        assert_eq!(
            logs[0],
            ReceiptLog {
                address: DPOS_CONTRACT_ADDRESS,
                topics: vec![DPOS_VALIDATOR_REGISTERED_TOPIC, address_topic(validator)],
                data: Vec::new(),
            }
        );
        assert_dpos_amount_log(
            &logs[1],
            DPOS_DELEGATED_TOPIC,
            vec![address_topic(owner), address_topic(validator)],
            stake,
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
        let register_topic_bloom = bloom_query_for_value(&DPOS_VALIDATOR_REGISTERED_TOPIC);
        assert_eq!(
            final_chain
                .with_block_bloom(&register_topic_bloom, period, period)
                .unwrap(),
            vec![period]
        );
        assert!(
            final_chain
                .with_block_bloom(&register_topic_bloom, period + 1, period + 1)
                .unwrap()
                .is_empty()
        );
        let pbft_author: [u8; 20] = address_from_signing_key(&signing_key).into();
        let author_bloom = bloom_query_for_value(&pbft_author);
        assert_eq!(
            final_chain
                .with_block_bloom(&author_bloom, period, period)
                .unwrap(),
            vec![period]
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
        assert_eq!(
            final_chain
                .with_block_bloom(
                    &bloom_query_for_value(&DPOS_VALIDATOR_REGISTERED_TOPIC),
                    period,
                    period
                )
                .unwrap(),
            vec![period]
        );
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
        let delegate_logs = receipt_logs(&receipts[1]);
        assert_eq!(delegate_logs.len(), 1);
        assert_dpos_amount_log(
            &delegate_logs[0],
            DPOS_DELEGATED_TOPIC,
            vec![address_topic(owner), address_topic(validator)],
            U256::from(4_000u64),
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
    fn finalize_block_supports_undelegation_v2_confirm_lifecycle() {
        let path = temp_db_path("finalize-dpos-undelegate-v2-confirm");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x61; 20];
        let signing_key = SigningKey::from_slice(&[21u8; 32]).unwrap();
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            commission_change_delta: 0,
            commission_change_frequency: 0,
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let rewards_config = FinalChainRewardsConfig {
            cornus_period: 0,
            cornus_delegation_locking_period: 2,
            ..Default::default()
        };
        let period_one = 1u64;
        let period_one_block = signed_pbft_block(&signing_key, period_one, 161);
        let undelegate_tx = test_transaction(
            0xC1,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::zero(),
            100_000,
            undelegate_v2_input(validator, U256::from(3_000u64)),
            vec![0xc1, 0xc1],
        );
        write_period_data(
            &storage,
            period_one,
            &period_one_block,
            &[undelegate_tx.rlp.clone()],
        );
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            200_000,
            0,
            vec![
                genesis_account(validator, U256::from(1_000_000u64)),
                genesis_account(DPOS_CONTRACT_ADDRESS, U256::from(10_000u64)),
            ],
            vec![genesis_validator(validator, U256::from(10_000u64))],
            genesis_dpos_config.clone(),
            rewards_config.clone(),
        )
        .unwrap();

        let (_header_rlp, receipts) = final_chain
            .finalize_block(period_one_block, vec![undelegate_tx], vec![])
            .unwrap();

        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, DPOS_UNDELEGATE_GAS, DPOS_UNDELEGATE_GAS)
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_UNDELEGATED_V2_TOPIC,
            vec![
                address_topic(validator),
                address_topic(validator),
                u64_topic(1),
            ],
            U256::from(3_000u64),
        );
        assert_single_delegation(
            &final_chain,
            period_one,
            validator,
            validator,
            U256::from(7_000u64),
            U256::zero(),
        );
        let pending = final_chain
            .call(dpos_call_request(
                period_one,
                get_undelegations_v2_input(validator, 0),
            ))
            .unwrap();
        assert_eq!(pending.gas_used, 20_000);
        assert_eq!(
            u256_from_big_endian(&pending.code_retval[32..64]),
            U256::one()
        );
        assert_eq!(
            u256_from_big_endian(&pending.code_retval[64..96]),
            U256::one()
        );
        assert_eq!(
            u256_from_big_endian(&pending.code_retval[96..128]),
            U256::from(3_000u64)
        );
        assert_eq!(
            u256_from_big_endian(&pending.code_retval[128..160]),
            U256::from(3u64)
        );
        assert_eq!(&pending.code_retval[172..192], &validator);
        assert_eq!(
            u256_from_big_endian(&pending.code_retval[192..224]),
            U256::one()
        );
        assert_eq!(
            u256_from_big_endian(&pending.code_retval[224..256]),
            U256::one()
        );

        drop(final_chain);
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            200_000,
            0,
            vec![
                genesis_account(validator, U256::from(1_000_000u64)),
                genesis_account(DPOS_CONTRACT_ADDRESS, U256::from(10_000u64)),
            ],
            vec![genesis_validator(validator, U256::from(10_000u64))],
            genesis_dpos_config,
            rewards_config,
        )
        .unwrap();
        assert_eq!(
            final_chain
                .call(dpos_call_request(
                    period_one,
                    get_undelegation_v2_input(validator, validator, 1),
                ))
                .unwrap()
                .code_err,
            ""
        );

        let period_two = 2u64;
        let period_two_block = signed_pbft_block(&signing_key, period_two, 162);
        let locked_confirm_tx = test_transaction(
            0xC2,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            confirm_undelegate_v2_input(validator, 1),
            vec![0xc1, 0xc2],
        );
        write_period_data(
            &storage,
            period_two,
            &period_two_block,
            &[locked_confirm_tx.rlp.clone()],
        );
        let (_header_rlp, receipts) = final_chain
            .finalize_block(period_two_block, vec![locked_confirm_tx], vec![])
            .unwrap();
        assert_eq!(
            receipt_fields(&receipts[0]),
            (0, DPOS_DEFAULT_METHOD_GAS, DPOS_DEFAULT_METHOD_GAS)
        );
        assert!(receipt_logs(&receipts[0]).is_empty());

        let period_three = 3u64;
        let period_three_block = signed_pbft_block(&signing_key, period_three, 163);
        let confirm_tx = test_transaction(
            0xC3,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            2,
            U256::zero(),
            U256::zero(),
            100_000,
            confirm_undelegate_v2_input(validator, 1),
            vec![0xc1, 0xc3],
        );
        write_period_data(
            &storage,
            period_three,
            &period_three_block,
            &[confirm_tx.rlp.clone()],
        );
        let (_header_rlp, receipts) = final_chain
            .finalize_block(period_three_block, vec![confirm_tx], vec![])
            .unwrap();
        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, DPOS_DEFAULT_METHOD_GAS, DPOS_DEFAULT_METHOD_GAS)
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_UNDELEGATE_CONFIRMED_V2_TOPIC,
            vec![
                address_topic(validator),
                address_topic(validator),
                u64_topic(1),
            ],
            U256::from(3_000u64),
        );
        assert_eq!(
            balance_of(&final_chain, DPOS_CONTRACT_ADDRESS),
            U256::from(7_000u64)
        );
        assert_eq!(
            balance_of(&final_chain, validator),
            U256::from(1_003_000u64)
        );
        let empty_page = final_chain
            .call(dpos_call_request(
                period_three,
                get_undelegations_v2_input(validator, 0),
            ))
            .unwrap();
        assert_eq!(empty_page.gas_used, 0);
        assert_eq!(
            u256_from_big_endian(&empty_page.code_retval[64..96]),
            U256::zero()
        );
        let missing = final_chain
            .call(dpos_call_request(
                period_three,
                get_undelegation_v2_input(validator, validator, 1),
            ))
            .unwrap();
        assert_eq!(missing.code_err, "Undelegation does not exist");

        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn finalize_block_supports_undelegation_v2_cancel_lifecycle() {
        let path = temp_db_path("finalize-dpos-undelegate-v2-cancel");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = [0x62; 20];
        let signing_key = SigningKey::from_slice(&[22u8; 32]).unwrap();
        let genesis_dpos_config = GenesisDposConfig {
            eligibility_balance_threshold: u256_to_big_endian(U256::from(1_000u64)),
            vote_eligibility_balance_step: u256_to_big_endian(U256::from(1_000u64)),
            validator_maximum_stake: u256_to_big_endian(U256::from(30_000u64)),
            minimum_deposit: vec![],
            commission_change_delta: 0,
            commission_change_frequency: 0,
            delegation_delay: 0,
            dag_vdf_sortition_total_vote_count_until_period: 0,
        };
        let rewards_config = FinalChainRewardsConfig {
            cornus_period: 0,
            cornus_delegation_locking_period: 10,
            ..Default::default()
        };
        let period_one = 1u64;
        let period_one_block = signed_pbft_block(&signing_key, period_one, 171);
        let undelegate_tx = test_transaction(
            0xD1,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            0,
            U256::zero(),
            U256::zero(),
            100_000,
            undelegate_v2_input(validator, U256::from(4_000u64)),
            vec![0xc1, 0xd1],
        );
        write_period_data(
            &storage,
            period_one,
            &period_one_block,
            &[undelegate_tx.rlp.clone()],
        );
        let final_chain = FinalChain::new_with_rewards_config(
            storage.clone(),
            200_000,
            0,
            vec![
                genesis_account(validator, U256::from(1_000_000u64)),
                genesis_account(DPOS_CONTRACT_ADDRESS, U256::from(10_000u64)),
            ],
            vec![genesis_validator(validator, U256::from(10_000u64))],
            genesis_dpos_config,
            rewards_config,
        )
        .unwrap();
        final_chain
            .finalize_block(period_one_block, vec![undelegate_tx], vec![])
            .unwrap();

        let period_two = 2u64;
        let period_two_block = signed_pbft_block(&signing_key, period_two, 172);
        let cancel_tx = test_transaction(
            0xD2,
            validator,
            Some(DPOS_CONTRACT_ADDRESS),
            1,
            U256::zero(),
            U256::zero(),
            100_000,
            cancel_undelegate_v2_input(validator, 1),
            vec![0xc1, 0xd2],
        );
        write_period_data(
            &storage,
            period_two,
            &period_two_block,
            &[cancel_tx.rlp.clone()],
        );
        let (_header_rlp, receipts) = final_chain
            .finalize_block(period_two_block, vec![cancel_tx], vec![])
            .unwrap();
        assert_eq!(
            receipt_fields(&receipts[0]),
            (1, DPOS_UNDELEGATE_GAS, DPOS_UNDELEGATE_GAS)
        );
        let logs = receipt_logs(&receipts[0]);
        assert_eq!(logs.len(), 1);
        assert_dpos_amount_log(
            &logs[0],
            DPOS_UNDELEGATE_CANCELED_V2_TOPIC,
            vec![
                address_topic(validator),
                address_topic(validator),
                u64_topic(1),
            ],
            U256::from(4_000u64),
        );
        assert_single_delegation(
            &final_chain,
            period_two,
            validator,
            validator,
            U256::from(10_000u64),
            U256::zero(),
        );
        assert_eq!(
            final_chain
                .dpos_eligible_vote_count(period_two, validator)
                .unwrap(),
            10
        );
        let empty_page = final_chain
            .call(dpos_call_request(
                period_two,
                get_undelegations_v2_input(validator, 0),
            ))
            .unwrap();
        assert_eq!(empty_page.gas_used, 0);

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
            commission_change_delta: 0,
            commission_change_frequency: 0,
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
