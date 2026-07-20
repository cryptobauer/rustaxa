use crate::pbft::PbftBlockMetadata;
use anyhow::{Result, anyhow};
use ethereum_types::{H160, H256, U256};
use num_bigint::BigUint;
use std::cmp::Ordering;

/// Canonical byte width of an Ethereum/FinalChain log bloom.
pub const FINAL_CHAIN_LOG_BLOOM_BYTES: usize = 256;

/// Error returned when a FinalChain gas price exceeds the EVM `u256` domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalChainGasPriceLengthError {
    actual: usize,
}

impl FinalChainGasPriceLengthError {
    /// Returns the rejected slice length.
    pub const fn actual(self) -> usize {
        self.actual
    }
}

impl std::fmt::Display for FinalChainGasPriceLengthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "FinalChain gas price has {} bytes, expected at most 32",
            self.actual
        )
    }
}

impl std::error::Error for FinalChainGasPriceLengthError {}

/// Canonical `u256` gas price used by FinalChain execution boundaries.
///
/// Big-endian boundary inputs may contain zero through 32 bytes, including
/// leading zeroes. Fixed-width output is always exactly 32 bytes.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FinalChainGasPrice(U256);

impl FinalChainGasPrice {
    /// Returns a zero gas price.
    pub fn zero() -> Self {
        Self(U256::zero())
    }
    /// Wraps a gas price already represented in the EVM integer domain.
    pub const fn from_u256(value: U256) -> Self {
        Self(value)
    }
    /// Decodes an exactly 32-byte big-endian gas price.
    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self(U256::from_big_endian(&bytes))
    }
    /// Decodes zero through 32 big-endian bytes, rejecting wider values.
    pub fn try_from_be_slice(bytes: &[u8]) -> Result<Self, FinalChainGasPriceLengthError> {
        if bytes.len() > 32 {
            return Err(FinalChainGasPriceLengthError {
                actual: bytes.len(),
            });
        }
        Ok(Self(U256::from_big_endian(bytes)))
    }
    /// Returns the wrapped `u256` value.
    pub const fn as_u256(self) -> U256 {
        self.0
    }
    /// Consumes the gas price and returns its `u256` value.
    pub const fn into_u256(self) -> U256 {
        self.0
    }
    /// Encodes the gas price as exactly 32 big-endian bytes.
    pub fn to_fixed_be_bytes(self) -> [u8; 32] {
        self.0.to_big_endian()
    }
    /// Computes `gas_price * gas_used`, returning `None` on `u256` overflow.
    pub fn checked_fee(self, gas_used: u64) -> Option<U256> {
        self.0.checked_mul(U256::from(gas_used))
    }
}

impl From<U256> for FinalChainGasPrice {
    fn from(value: U256) -> Self {
        Self::from_u256(value)
    }
}
impl From<FinalChainGasPrice> for U256 {
    fn from(value: FinalChainGasPrice) -> Self {
        value.into_u256()
    }
}
impl TryFrom<&[u8]> for FinalChainGasPrice {
    type Error = FinalChainGasPriceLengthError;
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from_be_slice(bytes)
    }
}

/// Error returned when a FinalChain transaction value exceeds `u256`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalChainTransactionValueLengthError {
    actual: usize,
}
impl FinalChainTransactionValueLengthError {
    /// Returns the rejected slice length.
    pub const fn actual(self) -> usize {
        self.actual
    }
}
impl std::fmt::Display for FinalChainTransactionValueLengthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FinalChain transaction value has {} bytes, expected at most 32",
            self.actual
        )
    }
}
impl std::error::Error for FinalChainTransactionValueLengthError {}

/// Canonical `u256` value transferred by a FinalChain transaction or call.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FinalChainTransactionValue(U256);
impl FinalChainTransactionValue {
    /// Returns zero value.
    pub fn zero() -> Self {
        Self(U256::zero())
    }
    /// Wraps a value already represented as `u256`.
    pub const fn from_u256(value: U256) -> Self {
        Self(value)
    }
    /// Decodes zero through 32 big-endian bytes, including leading zeroes.
    pub fn try_from_be_slice(bytes: &[u8]) -> Result<Self, FinalChainTransactionValueLengthError> {
        if bytes.len() > 32 {
            return Err(FinalChainTransactionValueLengthError {
                actual: bytes.len(),
            });
        }
        Ok(Self(U256::from_big_endian(bytes)))
    }
    /// Reports whether the transferred value is zero.
    pub fn is_zero(self) -> bool {
        self.0.is_zero()
    }
    /// Returns the wrapped value.
    pub const fn as_u256(self) -> U256 {
        self.0
    }
    /// Consumes the wrapper and returns its value.
    pub const fn into_u256(self) -> U256 {
        self.0
    }
    /// Encodes exactly 32 big-endian bytes.
    pub fn to_fixed_be_bytes(self) -> [u8; 32] {
        self.0.to_big_endian()
    }
    /// Encodes the legacy execution form: zero is `[0]`, otherwise minimal big-endian.
    pub fn to_legacy_minimal_bytes(self) -> Vec<u8> {
        let fixed = self.to_fixed_be_bytes();
        let first = fixed.iter().position(|byte| *byte != 0).unwrap_or(31);
        fixed[first..].to_vec()
    }
}
impl From<U256> for FinalChainTransactionValue {
    fn from(value: U256) -> Self {
        Self(value)
    }
}
impl From<FinalChainTransactionValue> for U256 {
    fn from(value: FinalChainTransactionValue) -> Self {
        value.0
    }
}
impl TryFrom<&[u8]> for FinalChainTransactionValue {
    type Error = FinalChainTransactionValueLengthError;
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from_be_slice(bytes)
    }
}

/// Error returned when a byte slice cannot form a fixed-width log bloom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalChainLogBloomLengthError {
    actual: usize,
}

impl FinalChainLogBloomLengthError {
    /// Returns the rejected slice length.
    pub const fn actual(self) -> usize {
        self.actual
    }
}

impl std::fmt::Display for FinalChainLogBloomLengthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "FinalChain log bloom has {} bytes, expected {FINAL_CHAIN_LOG_BLOOM_BYTES}",
            self.actual
        )
    }
}

impl std::error::Error for FinalChainLogBloomLengthError {}

/// Fixed-width FinalChain log bloom used by headers and bloom indexes.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FinalChainLogBloom([u8; FINAL_CHAIN_LOG_BLOOM_BYTES]);

impl FinalChainLogBloom {
    /// All-zero bloom used by empty blocks and uninitialized index chunks.
    pub const ZERO: Self = Self([0; FINAL_CHAIN_LOG_BLOOM_BYTES]);
    /// Wraps a bloom whose width is proven by its array type.
    pub const fn new(bytes: [u8; FINAL_CHAIN_LOG_BLOOM_BYTES]) -> Self {
        Self(bytes)
    }
    /// Borrows the fixed-width bloom bytes.
    pub const fn as_bytes(&self) -> &[u8; FINAL_CHAIN_LOG_BLOOM_BYTES] {
        &self.0
    }
    /// Mutably borrows the fixed-width bloom bytes.
    pub const fn as_mut_bytes(&mut self) -> &mut [u8; FINAL_CHAIN_LOG_BLOOM_BYTES] {
        &mut self.0
    }
    /// Consumes the wrapper and returns its byte array.
    pub const fn into_bytes(self) -> [u8; FINAL_CHAIN_LOG_BLOOM_BYTES] {
        self.0
    }
}

impl Default for FinalChainLogBloom {
    fn default() -> Self {
        Self::ZERO
    }
}
impl From<[u8; FINAL_CHAIN_LOG_BLOOM_BYTES]> for FinalChainLogBloom {
    fn from(bytes: [u8; FINAL_CHAIN_LOG_BLOOM_BYTES]) -> Self {
        Self::new(bytes)
    }
}
impl From<FinalChainLogBloom> for [u8; FINAL_CHAIN_LOG_BLOOM_BYTES] {
    fn from(bloom: FinalChainLogBloom) -> Self {
        bloom.into_bytes()
    }
}
impl TryFrom<&[u8]> for FinalChainLogBloom {
    type Error = FinalChainLogBloomLengthError;
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let array = <[u8; FINAL_CHAIN_LOG_BLOOM_BYTES]>::try_from(bytes).map_err(|_| {
            FinalChainLogBloomLengthError {
                actual: bytes.len(),
            }
        })?;
        Ok(Self::new(array))
    }
}
impl AsRef<[u8]> for FinalChainLogBloom {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl AsMut<[u8]> for FinalChainLogBloom {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// Zero-based position of a transaction in one finalized FinalChain block.
///
/// The domain is exactly `u32`, matching durable transaction-location and
/// receipt schemas. Construction from wider collection or FFI indices is
/// checked so no position can be silently truncated. The type intentionally
/// exposes no arithmetic or dereference operations; callers must name a
/// checked conversion at boundaries.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FinalChainTransactionPosition(u32);

impl FinalChainTransactionPosition {
    /// Constructs a position already proven to be in the `u32` domain.
    pub const fn new(position: u32) -> Self {
        Self(position)
    }

    /// Returns the schema-width position for persistence and query carriers.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl From<u32> for FinalChainTransactionPosition {
    fn from(position: u32) -> Self {
        Self::new(position)
    }
}

impl From<FinalChainTransactionPosition> for u32 {
    fn from(position: FinalChainTransactionPosition) -> Self {
        position.as_u32()
    }
}

impl TryFrom<u64> for FinalChainTransactionPosition {
    type Error = std::num::TryFromIntError;

    fn try_from(position: u64) -> Result<Self, Self::Error> {
        Ok(Self::new(u32::try_from(position)?))
    }
}

impl TryFrom<usize> for FinalChainTransactionPosition {
    type Error = std::num::TryFromIntError;

    fn try_from(position: usize) -> Result<Self, Self::Error> {
        Ok(Self::new(u32::try_from(position)?))
    }
}

/// Canonical arbitrary-width FinalChain account/transaction nonce.
///
/// Nonces are encoded as minimal unsigned big-endian bytes: zero is the empty
/// byte string and non-zero values may not begin with `0`. This keeps existing
/// RLP snapshots byte-identical for values representable by `u64`, while also
/// preserving account state above the EVM `u256` range.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FinalChainNonce(BigUint);

impl FinalChainNonce {
    /// Returns the zero nonce.
    pub fn zero() -> Self {
        Self(BigUint::default())
    }

    /// Constructs a nonce from a machine-width value.
    pub fn from_u64(value: u64) -> Self {
        Self(BigUint::from(value))
    }

    /// Decodes canonical minimal big-endian bytes. Empty bytes represent zero.
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes.first() == Some(&0) {
            anyhow::bail!("non-canonical FinalChain nonce with leading zero")
        }
        Ok(Self(BigUint::from_bytes_be(bytes)))
    }

    /// Returns canonical minimal big-endian bytes. Zero is encoded as empty.
    pub fn to_bytes(&self) -> Vec<u8> {
        if self.is_zero() {
            Vec::new()
        } else {
            self.0.to_bytes_be()
        }
    }

    /// Returns the nonce as `u64`, or `None` when it does not fit.
    pub fn as_u64(&self) -> Option<u64> {
        self.0.clone().try_into().ok()
    }

    /// Returns the successor nonce. Big integer growth is unbounded.
    pub fn next(&self) -> Self {
        Self(&self.0 + BigUint::from(1u8))
    }

    /// Returns whether this nonce is zero.
    pub fn is_zero(&self) -> bool {
        self.0 == BigUint::default()
    }
}

impl From<u64> for FinalChainNonce {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl PartialEq<u64> for FinalChainNonce {
    fn eq(&self, other: &u64) -> bool {
        self.0 == BigUint::from(*other)
    }
}

impl PartialOrd<u64> for FinalChainNonce {
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        Some(self.0.cmp(&BigUint::from(*other)))
    }
}

#[cfg(test)]
mod nonce_tests {
    use super::FinalChainNonce;

    #[test]
    fn nonce_encoding_is_minimal_and_unbounded() {
        assert_eq!(FinalChainNonce::zero().to_bytes(), Vec::<u8>::new());
        assert_eq!(
            FinalChainNonce::from_bytes(&[]).unwrap(),
            FinalChainNonce::zero()
        );
        assert!(FinalChainNonce::from_bytes(&[0]).is_err());
        assert_eq!(
            FinalChainNonce::from_u64(u64::MAX).to_bytes(),
            u64::MAX.to_be_bytes()
        );
        let max_u256 = vec![0xff; 32];
        let nonce = FinalChainNonce::from_bytes(&max_u256).unwrap();
        assert_eq!(nonce.to_bytes(), max_u256);
        assert_eq!(
            nonce.next().to_bytes(),
            vec![1].into_iter().chain([0u8; 32]).collect::<Vec<_>>()
        );
        assert!(nonce.as_u64().is_none());
    }

    #[test]
    fn nonce_preserves_legacy_u64_ordering() {
        let nonce = FinalChainNonce::from_u64(7);
        assert_eq!(nonce, 7);
        assert!(nonce > 6);
        assert!(nonce < 8);
    }
}

#[cfg(test)]
mod transaction_position_tests {
    use super::FinalChainTransactionPosition;

    #[test]
    fn position_accepts_exact_u32_domain() {
        let maximum = FinalChainTransactionPosition::try_from(u64::from(u32::MAX)).unwrap();
        assert_eq!(maximum.as_u32(), u32::MAX);
        assert_eq!(u32::from(maximum), u32::MAX);
        assert_eq!(FinalChainTransactionPosition::from(7u32).as_u32(), 7);
    }

    #[test]
    fn position_rejects_wider_values() {
        assert!(FinalChainTransactionPosition::try_from(u64::from(u32::MAX) + 1).is_err());
        if usize::BITS > u32::BITS {
            assert!(FinalChainTransactionPosition::try_from(u32::MAX as usize + 1).is_err());
        }
    }
}

#[cfg(test)]
mod log_bloom_tests {
    use super::{FINAL_CHAIN_LOG_BLOOM_BYTES, FinalChainLogBloom};

    #[test]
    fn log_bloom_enforces_exact_width() {
        let exact = [0xabu8; FINAL_CHAIN_LOG_BLOOM_BYTES];
        assert_eq!(
            FinalChainLogBloom::try_from(exact.as_slice())
                .unwrap()
                .into_bytes(),
            exact
        );
        assert_eq!(
            FinalChainLogBloom::try_from(&exact[..FINAL_CHAIN_LOG_BLOOM_BYTES - 1])
                .unwrap_err()
                .actual(),
            255
        );
        let oversized = [0u8; FINAL_CHAIN_LOG_BLOOM_BYTES + 1];
        assert_eq!(
            FinalChainLogBloom::try_from(oversized.as_slice())
                .unwrap_err()
                .actual(),
            257
        );
    }

    #[test]
    fn log_bloom_zero_array_and_mutation_are_explicit() {
        assert_eq!(
            FinalChainLogBloom::ZERO.as_bytes(),
            &[0; FINAL_CHAIN_LOG_BLOOM_BYTES]
        );
        let mut bloom = FinalChainLogBloom::new([0; FINAL_CHAIN_LOG_BLOOM_BYTES]);
        bloom.as_mut_bytes()[17] = 0x80;
        assert_eq!(bloom.as_ref()[17], 0x80);
        let array: [u8; FINAL_CHAIN_LOG_BLOOM_BYTES] = bloom.into();
        assert_eq!(array[17], 0x80);
    }
}

/// Genesis account input passed from C++ configuration into the Rust final-chain domain.
///
/// Balances are stored as big-endian unsigned integer bytes so bridge code can
/// preserve the exact C++ `u256` representation without assigning numeric
/// semantics at the FFI boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenesisAccount {
    /// Account address bytes in canonical Ethereum/Taraxa address order.
    pub address: [u8; 20],
    /// Initial account balance as an unsigned big-endian integer byte string.
    pub balance: Vec<u8>,
}

/// Genesis validator metadata passed from genesis configuration into Rust.
///
/// These fields mirror the user-visible DPoS validator info returned by
/// `getValidator(address)`. The owner address is encoded in canonical
/// Ethereum/Taraxa address order, `commission` is the Solidity `uint16`
/// percentage value, and the text fields are stored as UTF-8 strings supplied
/// by the external genesis configuration boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GenesisValidatorMetadata {
    /// Validator owner address bytes in canonical address order.
    pub owner: [u8; 20],
    /// Validator commission value represented as the contract's `uint16`.
    pub commission: u16,
    /// Human-readable validator description encoded as UTF-8.
    pub description: String,
    /// Validator endpoint encoded as UTF-8.
    pub endpoint: String,
}

/// Genesis validator input passed from configuration into Rust.
///
/// The address identifies the validator account, the VRF key is kept as raw
/// bytes because DAG verification currently consumes the C++ VRF wrapper format
/// through the bridge, `total_stake` is the effective genesis stake used by the
/// initial Rust DPoS vote-count model, and `metadata` seeds the DPoS read model
/// returned by `getValidator(address)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenesisValidator {
    /// Validator account address bytes in canonical address order.
    pub address: [u8; 20],
    /// Validator VRF public key bytes.
    pub vrf_key: [u8; 32],
    /// Effective genesis validator stake as an unsigned big-endian integer byte string.
    pub total_stake: Vec<u8>,
    /// Per-delegator genesis stakes for this validator.
    ///
    /// Each tuple carries `(delegator_address, stake)` where stake is an
    /// unsigned big-endian integer byte string. The Rust DPoS snapshot uses this
    /// ledger to validate undelegation and redelegation ownership without
    /// routing contract semantics back through C++.
    pub delegations: Vec<([u8; 20], Vec<u8>)>,
    /// Genesis-seeded user-visible validator metadata.
    pub metadata: GenesisValidatorMetadata,
}

/// Validator metadata stored in a block-keyed DPoS snapshot.
///
/// Snapshot metadata is separated from stake and reward counters because stake
/// and rewards change through finalization. Owner-controlled DPoS contract
/// calls mutate these fields in the same block snapshot as stake/reward state,
/// so historical reads observe the validator info that was current at the
/// requested finalized block.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DposValidatorMetadata {
    /// Validator owner address bytes in canonical address order.
    pub owner: [u8; 20],
    /// Validator commission value represented as the contract's `uint16`.
    pub commission: u16,
    /// Finalized block number of the latest accepted commission change.
    pub last_commission_change: u64,
    /// Validator description as raw bytes.
    pub description: Vec<u8>,
    /// Validator endpoint as raw bytes.
    pub endpoint: Vec<u8>,
}

impl From<&GenesisValidator> for DposValidatorMetadata {
    fn from(validator: &GenesisValidator) -> Self {
        Self {
            owner: validator.metadata.owner,
            commission: validator.metadata.commission,
            last_commission_change: 0,
            description: validator.metadata.description.as_bytes().to_vec(),
            endpoint: validator.metadata.endpoint.as_bytes().to_vec(),
        }
    }
}

/// DPoS genesis parameters used to derive the initial vote-count model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GenesisDposConfig {
    /// Minimum stake required for a validator to be eligible.
    pub eligibility_balance_threshold: Vec<u8>,
    /// Stake amount represented by one vote.
    pub vote_eligibility_balance_step: Vec<u8>,
    /// Maximum allowed effective stake for a genesis validator.
    pub validator_maximum_stake: Vec<u8>,
    /// Minimum delegation amount accepted by the DPoS contract, encoded as an
    /// unsigned big-endian integer byte string.
    pub minimum_deposit: Vec<u8>,
    /// Maximum absolute commission change accepted by `setCommission`.
    pub commission_change_delta: u16,
    /// Minimum block distance between accepted commission changes.
    pub commission_change_frequency: u32,
    /// Number of finalized blocks by which DPoS state reads lag newly applied
    /// delegation changes.
    pub delegation_delay: u64,
    /// Exclusive period boundary below which legacy DAG VDF sortition uses the
    /// total eligible vote count as denominator.
    pub dag_vdf_sortition_total_vote_count_until_period: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RedelegationCorrection {
    /// Validator address receiving the redelegation correction amount.
    pub validator: [u8; 20],
    /// Delegator address whose historical redelegation is being corrected.
    pub delegator: [u8; 20],
    /// Correction amount as an unsigned big-endian integer byte string.
    pub amount: Vec<u8>,
}

/// Rewards and hardfork configuration used by Rust native finalization.
///
/// This is intentionally separate from `GenesisDposConfig`: DPoS genesis
/// fields describe validator/stake initialization, while rewards configuration
/// controls post-execution reward-stat planning and fixed-yield reward
/// distribution for finalized blocks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainRewardsConfig {
    /// Committee size used by legacy rewards stats for max vote-weight bounds.
    pub committee_size: u32,
    /// First period where Magnolia fee rewards are active.
    pub magnolia_period: u64,
    /// First period where the exact legacy Phalaenopsis DPoS escrow-transfer
    /// selector is active.
    ///
    /// A value of `u64::MAX` disables the compatibility action outside
    /// configured networks.
    pub phalaenopsis_period: u64,
    /// First period where Aspen part-one DAG reward counting is active.
    pub aspen_part_one_period: u64,
    /// First period where the legacy pre-Aspen `claimAllRewards(uint32)` ABI is disabled.
    ///
    /// A value of `u64::MAX` keeps the compatibility ABI enabled for local
    /// rewrite tests that do not configure the hardfork boundary.
    pub fix_claim_all_block_num: u64,
    /// First period where a one-time redelegation hardfork stake correction is applied.
    ///
    /// A value of `u64::MAX` disables correction outside configured hardforks.
    pub fix_redelegate_block_num: u64,
    /// First period where Aspen part-two dynamic-yield rewards are active.
    ///
    /// Rust native finalization currently distributes fixed-yield rewards only.
    /// A zero value keeps the part-two path disabled for rewrite tests and
    /// local configurations that do not provide the hardfork boundary.
    pub aspen_part_two_period: u64,
    /// Ordered redelegation stake corrections applied when
    /// `fix_redelegate_block_num` is reached.
    pub redelegations: Vec<RedelegationCorrection>,
    /// Maximum percentage of a block reward paid to the PBFT block author as a
    /// cert-vote inclusion bonus.
    pub max_block_author_reward_percent: u16,
    /// Percentage of a block reward allocated to DAG proposers when the
    /// finalized period has cert-vote weight.
    pub dag_proposers_reward_percent: u16,
    /// Fixed annual DPoS yield percentage used before Aspen part two.
    pub yield_percentage: u16,
    /// Configured fixed-yield block count per year.
    pub dpos_blocks_per_year: u32,
    /// Legacy DPoS undelegation locking period before Cornus overrides it.
    pub dpos_delegation_locking_period: u64,
    /// First period where Cornus DPoS V2 undelegation methods are active.
    pub cornus_period: u64,
    /// DPoS undelegation locking period after Cornus and before Cacti.
    pub cornus_delegation_locking_period: u64,
    /// Genesis account balance sum encoded as an unsigned big-endian integer.
    ///
    /// Aspen part-two supply migration adds this value to the durable
    /// part-one minted-token counter and generated rewards.
    pub genesis_balance_sum: Vec<u8>,
    /// Aspen part-two maximum supply encoded as an unsigned big-endian integer.
    pub aspen_max_supply: Vec<u8>,
    /// Aspen part-one generated rewards encoded as an unsigned big-endian
    /// integer. Aspen part-two migration adds it to the genesis balance sum and
    /// the pre-migration minted-token counter.
    pub aspen_generated_rewards: Vec<u8>,
    /// First period where Cacti reward stats provide dynamic blocks-per-year.
    pub cacti_period: u64,
    /// DPoS undelegation locking period after Cacti.
    pub cacti_delegation_locking_period: u64,
    /// Number of finalized blocks a validator remains jailed before Cacti.
    pub magnolia_jail_time: u64,
    /// Number of finalized blocks a validator remains jailed after Cacti.
    pub cacti_jail_time: u64,
    /// Rewards distribution frequency changes keyed by starting period.
    pub rewards_distribution_frequency: Vec<(u64, u32)>,
}

impl Default for FinalChainRewardsConfig {
    fn default() -> Self {
        Self {
            committee_size: 0,
            magnolia_period: 0,
            phalaenopsis_period: u64::MAX,
            aspen_part_one_period: 0,
            fix_claim_all_block_num: u64::MAX,
            fix_redelegate_block_num: u64::MAX,
            aspen_part_two_period: 0,
            max_block_author_reward_percent: 0,
            dag_proposers_reward_percent: 0,
            yield_percentage: 0,
            dpos_blocks_per_year: 0,
            dpos_delegation_locking_period: 0,
            cornus_period: 0,
            cornus_delegation_locking_period: 0,
            genesis_balance_sum: Vec::new(),
            aspen_max_supply: Vec::new(),
            aspen_generated_rewards: Vec::new(),
            cacti_period: 0,
            cacti_delegation_locking_period: 0,
            magnolia_jail_time: 0,
            cacti_jail_time: 0,
            rewards_distribution_frequency: Vec::new(),
            redelegations: Vec::new(),
        }
    }
}

/// Transient FinalChain call request routed from C++ into Rust.
///
/// This type intentionally models the execution-facing fields Rust needs for
/// deterministic native/precompile reads and DPoS mutation simulation. Values
/// are kept as big-endian byte strings at the boundary so the bridge does not
/// need to interpret C++ `u256` layouts. Mutation calls execute against cloned
/// requested-block snapshots and never publish their staged state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainCallRequest {
    /// Block number used for historical reads.
    pub block_number: u64,
    /// Caller address bytes in canonical Ethereum/Taraxa address order.
    pub sender: [u8; 20],
    /// Optional receiver address. `None` represents contract creation.
    pub receiver: Option<[u8; 20]>,
    /// Transaction value as an unsigned big-endian integer byte string.
    pub value: FinalChainTransactionValue,
    /// Gas price in the EVM `u256` domain.
    pub gas_price: FinalChainGasPrice,
    /// Gas limit supplied by the caller.
    pub gas_limit: u64,
    /// Call input data.
    pub input: Vec<u8>,
}

/// Result of a Rust-backed transient FinalChain call.
///
/// EVM-style failures are represented as error strings in the result to match
/// the C++ `state_api::ExecutionResult` contract. Infrastructure failures still
/// use `anyhow::Result` at the API boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinalChainCallOutcome {
    /// Returned call data bytes.
    pub code_retval: Vec<u8>,
    /// Contract logs produced by the transient call. These are returned to the
    /// caller but are never persisted as finalized receipts.
    pub logs: Vec<FinalChainCallLog>,
    /// Gas used by the transient call.
    pub gas_used: u64,
    /// EVM/code-level error text, if any.
    pub code_err: String,
    /// Consensus/account-level error text, if any.
    pub consensus_err: String,
}

/// One EVM-compatible log produced by a transient FinalChain call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainCallLog {
    /// Emitting contract address.
    pub address: [u8; 20],
    /// Ordered event signature and indexed argument topics.
    pub topics: Vec<[u8; 32]>,
    /// ABI-encoded non-indexed event data.
    pub data: Vec<u8>,
}

/// Finalized DAG block summary needed by Rust finalization reward accounting.
///
/// The full DAG block remains a C++ type during this slice. Rust only needs the
/// author and ordered transaction hashes to reproduce deterministic fee reward
/// assignment for native transaction finalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizationDagBlock {
    /// DAG block author address bytes.
    pub author: [u8; 20],
    /// Legacy VDF difficulty used by Aspen DAG reward counting.
    pub difficulty: u16,
    /// Transaction hashes carried by this DAG block.
    pub transaction_hashes: Vec<[u8; 32]>,
}

/// Validator stake entry returned from the Rust DPoS read model.
///
/// The stake bytes keep the unsigned big-endian shape used by C++ `u256`
/// conversions at the bridge boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DposValidatorStake {
    /// Validator account address bytes in canonical address order.
    pub address: [u8; 20],
    /// Validator total stake as an unsigned big-endian integer byte string.
    pub stake: Vec<u8>,
}

/// Eligible validator vote-count entry returned from the Rust DPoS read model.
///
/// Entries represent validators with nonzero eligible vote count at the queried
/// block. Addresses are sorted by the Rust caller before crossing the bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DposValidatorVoteCount {
    /// Validator account address bytes in canonical address order.
    pub address: [u8; 20],
    /// Eligible vote count for this validator.
    pub vote_count: u64,
}

/// Final-chain account view returned to C++ callers through the bridge.
///
/// This is intentionally a data carrier rather than an EVM account object. It
/// represents the fields currently needed by Rust-enabled DAG and final-chain
/// tests while keeping storage roots, code hashes, and balances byte-exact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Account {
    /// Account nonce.
    pub nonce: FinalChainNonce,
    /// Account balance as an unsigned big-endian integer byte string.
    pub balance: Vec<u8>,
    /// State storage root hash bytes.
    pub storage_root_hash: [u8; 32],
    /// Contract code hash bytes.
    pub code_hash: [u8; 32],
    /// Contract code size in bytes.
    pub code_size: u64,
}

/// Transaction data needed by Rust finalization while transaction ownership is
/// still held by the C++ `Transaction` type at the bridge boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizationTransaction {
    /// Canonical transaction hash bytes.
    pub hash: [u8; 32],
    /// Recovered sender address bytes.
    pub sender: [u8; 20],
    /// Receiver address bytes for calls and value transfers.
    pub receiver: Option<[u8; 20]>,
    /// Transaction nonce.
    pub nonce: FinalChainNonce,
    /// Transaction value as unsigned big-endian integer bytes.
    pub value: FinalChainTransactionValue,
    /// Gas price in the EVM `u256` domain.
    pub gas_price: FinalChainGasPrice,
    /// Gas limit supplied by the transaction.
    pub gas_limit: u64,
    /// Transaction input data.
    pub data: Vec<u8>,
    /// Canonical transaction RLP.
    pub rlp: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFinalChainBlockHeader {
    pub parent_hash: H256,
    pub state_root: H256,
    pub transactions_root: H256,
    pub receipts_root: H256,
    pub log_bloom: FinalChainLogBloom,
    pub gas_used: u64,
    pub total_reward: U256,
}

impl StoredFinalChainBlockHeader {
    pub fn materialize(&self, context: BlockHeaderContext<'_>) -> FinalChainBlockHeader {
        FinalChainBlockHeaderBuilder::new(self)
            .hash(context.hash)
            .pbft(context.pbft)
            .block_gas_limit(context.block_gas_limit)
            .genesis_timestamp(context.genesis_timestamp)
            .build()
            .expect("context provides all required final-chain block header fields")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockHeaderContext<'a> {
    pub hash: H256,
    pub pbft: Option<&'a PbftBlockMetadata>,
    pub block_gas_limit: u64,
    pub genesis_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalChainBlockHeader {
    pub hash: H256,
    pub parent_hash: H256,
    pub author: H160,
    pub state_root: H256,
    pub transactions_root: H256,
    pub receipts_root: H256,
    pub log_bloom: FinalChainLogBloom,
    pub number: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub total_reward: U256,
    pub extra_data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FinalChainBlockHeaderBuilder<'a> {
    stored_header: &'a StoredFinalChainBlockHeader,
    hash: Option<H256>,
    pbft: Option<&'a PbftBlockMetadata>,
    block_gas_limit: Option<u64>,
    genesis_timestamp: Option<u64>,
}

impl<'a> FinalChainBlockHeaderBuilder<'a> {
    pub fn new(stored_header: &'a StoredFinalChainBlockHeader) -> Self {
        Self {
            stored_header,
            hash: None,
            pbft: None,
            block_gas_limit: None,
            genesis_timestamp: None,
        }
    }

    pub fn hash(mut self, hash: H256) -> Self {
        self.hash = Some(hash);
        self
    }

    pub fn pbft(mut self, pbft: Option<&'a PbftBlockMetadata>) -> Self {
        self.pbft = pbft;
        self
    }

    pub fn block_gas_limit(mut self, block_gas_limit: u64) -> Self {
        self.block_gas_limit = Some(block_gas_limit);
        self
    }

    pub fn genesis_timestamp(mut self, genesis_timestamp: u64) -> Self {
        self.genesis_timestamp = Some(genesis_timestamp);
        self
    }

    pub fn build(self) -> Result<FinalChainBlockHeader> {
        let hash = self
            .hash
            .ok_or_else(|| anyhow!("missing block header hash"))?;
        let block_gas_limit = self
            .block_gas_limit
            .ok_or_else(|| anyhow!("missing block gas limit"))?;
        let genesis_timestamp = self
            .genesis_timestamp
            .ok_or_else(|| anyhow!("missing genesis timestamp"))?;
        let author = self.pbft.map(|pbft| pbft.author).unwrap_or_default();
        let number = self.pbft.map(|pbft| pbft.period).unwrap_or_default();
        let timestamp = self
            .pbft
            .map(|pbft| pbft.timestamp)
            .unwrap_or(genesis_timestamp);
        let extra_data = self
            .pbft
            .map(|pbft| pbft.extra_data.clone())
            .unwrap_or_default();

        Ok(FinalChainBlockHeader {
            hash,
            parent_hash: self.stored_header.parent_hash,
            author,
            state_root: self.stored_header.state_root,
            transactions_root: self.stored_header.transactions_root,
            receipts_root: self.stored_header.receipts_root,
            log_bloom: self.stored_header.log_bloom,
            number,
            gas_limit: block_gas_limit,
            gas_used: self.stored_header.gas_used,
            timestamp,
            total_reward: self.stored_header.total_reward,
            extra_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gas_price_accepts_zero_through_32_bytes_and_preserves_value() {
        for length in [0, 1, 31, 32] {
            let mut bytes = vec![0; length];
            if let Some(last) = bytes.last_mut() {
                *last = 7;
            }
            let price = FinalChainGasPrice::try_from(bytes.as_slice()).unwrap();
            assert_eq!(
                price.as_u256(),
                if length == 0 {
                    U256::zero()
                } else {
                    U256::from(7)
                }
            );
            assert_eq!(price.to_fixed_be_bytes().len(), 32);
        }
        let leading_zero = FinalChainGasPrice::try_from(&[0, 1][..]).unwrap();
        assert_eq!(leading_zero.as_u256(), U256::one());
        let maximum = FinalChainGasPrice::from_u256(U256::MAX);
        assert_eq!(
            FinalChainGasPrice::from_be_bytes(maximum.to_fixed_be_bytes()),
            maximum
        );
    }

    #[test]
    fn gas_price_rejects_33_bytes_with_actual_length() {
        let error = FinalChainGasPrice::try_from(&[0; 33][..]).unwrap_err();
        assert_eq!(error.actual(), 33);
    }

    #[test]
    fn gas_price_checked_fee_handles_zero_success_and_overflow() {
        assert_eq!(
            FinalChainGasPrice::zero().checked_fee(u64::MAX),
            Some(U256::zero())
        );
        assert_eq!(
            FinalChainGasPrice::from(U256::from(3)).checked_fee(7),
            Some(U256::from(21))
        );
        assert_eq!(FinalChainGasPrice::from(U256::MAX).checked_fee(2), None);
    }

    #[test]
    fn transaction_value_accepts_boundary_widths_and_encodes_both_forms() {
        for length in [0, 1, 31, 32] {
            let mut bytes = vec![0; length];
            if let Some(last) = bytes.last_mut() {
                *last = 9;
            }
            let value = FinalChainTransactionValue::try_from(bytes.as_slice()).unwrap();
            assert_eq!(value.to_fixed_be_bytes().len(), 32);
            assert_eq!(value.is_zero(), length == 0);
        }
        assert_eq!(
            FinalChainTransactionValue::zero().to_legacy_minimal_bytes(),
            vec![0]
        );
        assert_eq!(
            FinalChainTransactionValue::try_from(&[0, 1][..])
                .unwrap()
                .to_legacy_minimal_bytes(),
            vec![1]
        );
        let maximum = FinalChainTransactionValue::from_u256(U256::MAX);
        assert_eq!(maximum.to_fixed_be_bytes(), [0xff; 32]);
    }

    #[test]
    fn transaction_value_rejects_33_bytes_with_actual_length() {
        let error = FinalChainTransactionValue::try_from(&[0; 33][..]).unwrap_err();
        assert_eq!(error.actual(), 33);
    }

    #[test]
    fn builder_materializes_genesis_header_defaults() {
        let stored_header = StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(1),
            state_root: H256::from_low_u64_be(2),
            transactions_root: H256::from_low_u64_be(3),
            receipts_root: H256::from_low_u64_be(4),
            log_bloom: FinalChainLogBloom::ZERO,
            gas_used: 5,
            total_reward: U256::from(6u64),
        };

        let header = FinalChainBlockHeaderBuilder::new(&stored_header)
            .hash(H256::from_low_u64_be(99))
            .block_gas_limit(1000)
            .genesis_timestamp(1234)
            .build()
            .unwrap();

        assert_eq!(header.parent_hash, stored_header.parent_hash);
        assert_eq!(header.author, H160::zero());
        assert_eq!(header.number, 0);
        assert_eq!(header.gas_limit, 1000);
        assert_eq!(header.gas_used, stored_header.gas_used);
        assert_eq!(header.timestamp, 1234);
        assert_eq!(header.total_reward, stored_header.total_reward);
        assert!(header.extra_data.is_empty());
    }

    #[test]
    fn builder_reports_missing_required_fields() {
        let stored_header = StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(1),
            state_root: H256::from_low_u64_be(2),
            transactions_root: H256::from_low_u64_be(3),
            receipts_root: H256::from_low_u64_be(4),
            log_bloom: FinalChainLogBloom::ZERO,
            gas_used: 5,
            total_reward: U256::from(6u64),
        };

        let err = FinalChainBlockHeaderBuilder::new(&stored_header)
            .block_gas_limit(1000)
            .genesis_timestamp(1234)
            .build()
            .unwrap_err();

        assert!(err.to_string().contains("missing block header hash"));
    }
}
