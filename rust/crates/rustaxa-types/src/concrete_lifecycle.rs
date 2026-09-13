//! Shared concrete database identity and lifecycle facts.
//!
//! These are existing StateAPI contracts shared by FinalChain authorization and
//! concrete storage. They carry facts, not publication authority. Canonical RLP
//! belongs to `codec::rlp::concrete_lifecycle`; callers validate lineage and the
//! application-approved intent before writing.

/// Concrete-root policy and projection codec version accepted by Rust.
pub const FINAL_CHAIN_CONCRETE_PROJECTION_VERSION: u64 = 1;
/// Stable identity of the concrete state database paired with one chain.
///
/// Strict codecs require the supported policy and nonzero database/chain IDs.
/// Construction and `Default` alone do not prove fresh-database ownership or
/// authorize pairing an imported database with an application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteIdentity {
    pub policy_version: u64,
    pub database_id: [u8; 32],
    pub chain_id: [u8; 32],
}

/// Concrete committed or staged state descriptor encoded in StateAPI bytes.
///
/// The period and root are an exact pair. A descriptor neither proves retained
/// history nor authorizes advancing a published head; the lifecycle owner checks
/// consecutive periods and the independently prepared root.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteState {
    pub period: u64,
    pub root: [u8; 32],
}

/// Durable exact staged-execution marker owned by StateAPI.
///
/// Binds one next period to its prior state, database identity and execution
/// input digests. The strict decoder checks consecutive periods; the owner must
/// additionally match generation, identity and digests to the accepted plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteExecutionMarker {
    pub identity: FinalChainConcreteIdentity,
    pub generation: u64,
    pub plan_hash: [u8; 32],
    pub period: u64,
    pub prior_state: FinalChainConcreteState,
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
}

/// Concrete StateAPI provenance for one exact staged or committed plan.
///
/// The committed descriptor and projection/catalog digests bind one generation.
/// Storage persists the application-approved canonical bytes atomically with the
/// descriptor. A well-formed value alone is not approval to commit or publish.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteStateProvenance {
    pub identity: FinalChainConcreteIdentity,
    pub generation: u64,
    pub plan_hash: [u8; 32],
    pub committed_state: FinalChainConcreteState,
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
    pub projection_hash: [u8; 32],
    pub catalog_hash: [u8; 32],
}
