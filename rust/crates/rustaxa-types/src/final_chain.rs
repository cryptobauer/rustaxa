use crate::pbft::PbftBlockMetadata;
use anyhow::{Result, anyhow};
use ethereum_types::{H160, H256, U256};

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
/// percentage value, and the text fields are stored as UTF-8 strings so the
/// FinalChain read model can ABI-encode them without crossing back into C++.
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
    /// Genesis-seeded user-visible validator metadata.
    pub metadata: GenesisValidatorMetadata,
}

/// Validator metadata stored in a block-keyed DPoS snapshot.
///
/// Snapshot metadata is separated from stake and reward counters because stake
/// and rewards change through finalization, while this slice only supports
/// genesis-seeded metadata. Future DPoS transactions can update these fields in
/// the snapshot without changing the read-only ABI encoder.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DposValidatorMetadata {
    /// Validator owner address bytes in canonical address order.
    pub owner: [u8; 20],
    /// Validator commission value represented as the contract's `uint16`.
    pub commission: u16,
    /// Human-readable validator description encoded as UTF-8.
    pub description: String,
    /// Validator endpoint encoded as UTF-8.
    pub endpoint: String,
}

impl From<&GenesisValidator> for DposValidatorMetadata {
    fn from(validator: &GenesisValidator) -> Self {
        Self {
            owner: validator.metadata.owner,
            commission: validator.metadata.commission,
            description: validator.metadata.description.clone(),
            endpoint: validator.metadata.endpoint.clone(),
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
    /// Exclusive period boundary below which legacy DAG VDF sortition uses the
    /// total eligible vote count as denominator.
    pub dag_vdf_sortition_total_vote_count_until_period: u64,
}

/// Read-only FinalChain call request routed from C++ into Rust.
///
/// This type intentionally models the execution-facing fields Rust needs for
/// deterministic native/precompile reads. Values are kept as big-endian byte
/// strings at the boundary so the bridge does not need to interpret C++ `u256`
/// layouts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainCallRequest {
    /// Block number used for historical reads.
    pub block_number: u64,
    /// Caller address bytes in canonical Ethereum/Taraxa address order.
    pub sender: [u8; 20],
    /// Optional receiver address. `None` represents contract creation.
    pub receiver: Option<[u8; 20]>,
    /// Transaction value as an unsigned big-endian integer byte string.
    pub value: Vec<u8>,
    /// Gas price as an unsigned big-endian integer byte string.
    pub gas_price: Vec<u8>,
    /// Gas limit supplied by the caller.
    pub gas_limit: u64,
    /// Call input data.
    pub input: Vec<u8>,
}

/// Result of a Rust-backed read-only FinalChain call.
///
/// EVM-style failures are represented as error strings in the result to match
/// the C++ `state_api::ExecutionResult` contract. Infrastructure failures still
/// use `anyhow::Result` at the API boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinalChainCallOutcome {
    /// Returned call data bytes.
    pub code_retval: Vec<u8>,
    /// Gas used by the read-only call.
    pub gas_used: u64,
    /// EVM/code-level error text, if any.
    pub code_err: String,
    /// Consensus/account-level error text, if any.
    pub consensus_err: String,
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
    pub nonce: u64,
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
    pub nonce: u64,
    /// Transaction value as unsigned big-endian integer bytes.
    pub value: Vec<u8>,
    /// Gas price as unsigned big-endian integer bytes.
    pub gas_price: Vec<u8>,
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
    pub log_bloom: Vec<u8>,
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
    pub log_bloom: Vec<u8>,
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
            log_bloom: self.stored_header.log_bloom.clone(),
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
    fn builder_materializes_genesis_header_defaults() {
        let stored_header = StoredFinalChainBlockHeader {
            parent_hash: H256::from_low_u64_be(1),
            state_root: H256::from_low_u64_be(2),
            transactions_root: H256::from_low_u64_be(3),
            receipts_root: H256::from_low_u64_be(4),
            log_bloom: vec![0; 256],
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
            log_bloom: vec![0; 256],
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
