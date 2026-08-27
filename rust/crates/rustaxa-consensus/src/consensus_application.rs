//! Native lifetime and composition root for Rust-enabled consensus.
//!
//! The application restores the PBFT and DAG/transaction/sortition graphs over
//! one storage owner and publishes them only as one coherent root. The bridge
//! may invoke task-family operations through this root, but CXX never receives
//! a separately constructible service or a service accessor.

use crate::consensus_application_runtime::{
    ConsensusApplicationRuntime, ConsensusExecutionPort, ConsensusProcessPort, ConsensusRunExit,
    ConsensusSigningPort, ConsensusTransportPort, SigningIdentity,
};
use crate::dag_service::{
    DagRuntimeNonFinalizedSyncPayload, DagServiceConfig, DagVerifyBlockGasReport,
    DagVerifyBlockSessionInput,
};
use crate::dag_transaction_service::{
    DagAddBlockAccountNonceFact, DagAddBlockCompletion, DagAddBlockPrepareRequest,
    DagAddBlockTransactionPayload, DagTransactionService, DagTransactionServiceConfig,
    DagVerifyBlockTransactionCompletionReport, DagVerifyBlockVdfRequest,
    PublicTransactionFinalChainFacts, PublicTransactionSubmissionReport,
    PublicTransactionSubmissionRequest,
};
use crate::final_chain::FinalChain;
use crate::gas_pricer::GasPricerConfig;
use crate::pbft_service::{PbftService, PbftServiceConfig};
use crate::sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams};
use crate::transaction_manager::{
    TransactionManagerVerifyTransactionFact, TransactionManagerVerifyTransactionStatus,
    plan_verify_transaction,
};
use crate::transaction_queue::TransactionQueueInsertStatus;
use crate::transaction_service::TransactionServiceConfig;
use anyhow::{Context, Result, bail};
use ethereum_types::{H256, U256};
use rlp::Rlp;
use rustaxa_storage::{Config, StatusField, Storage};
use rustaxa_types::LegacyTransactionEnvelope;
use rustaxa_types::{
    FinalChainGas, FinalChainRewardsConfig, GenesisAccount, GenesisDposConfig, GenesisValidator,
};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

/// Immutable configuration for one complete native consensus application.
///
/// Both sibling configurations are required. There are no capability flags or
/// partial construction paths; restoration errors prevent root publication.
#[derive(Clone, Debug)]
pub struct ConsensusApplicationConfig {
    /// Configuration for transaction, DAG, and sortition ownership.
    pub dag_transaction: DagTransactionServiceConfig,
    /// Configuration for PBFT, vote, pillar, slashing, and network ownership.
    pub pbft: PbftServiceConfig,
    /// Public identities for host-held signing keys, in stable wallet-index order.
    pub signing_identities: Vec<SigningIdentity>,
    /// Interruptible daemon polling interval used when no wallet is eligible.
    pub polling_interval_ms: u64,
    /// Native DAG proposer policy shared by configured public signing identities.
    pub dag_proposer: DagProposerConfig,
}

/// Production DAG proposer scheduler policy.
///
/// Wallet addresses and VRF public keys come from `signing_identities`; the
/// native bootstrap deterministically derives legacy retry budgets and shards.
/// Constants that are protocol/node policy remain native rather than being
/// duplicated in CXX. Gas limits and configured shard count are explicit
/// dynamic inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DagProposerConfig {
    pub total_transaction_shards: u16,
    /// Node-local cap applied to the active protocol DAG gas limit.
    pub proposal_dag_gas_limit: u64,
    /// Pre-Cornus maximum gas for one DAG block.
    pub default_dag_gas_limit: u64,
    /// Pre-Cornus maximum gas for one PBFT period.
    pub default_pbft_gas_limit: u64,
    /// First proposal period governed by Cornus gas limits.
    pub cornus_activation_period: u64,
    /// Cornus maximum gas for one DAG block.
    pub cornus_dag_gas_limit: u64,
    /// Cornus maximum gas for one PBFT period.
    pub cornus_pbft_gas_limit: u64,
}

impl DagProposerConfig {
    /// Returns canonical DAG/PBFT limits for `proposal_period`.
    ///
    /// Cornus activates inclusively. The proposer weight limit additionally
    /// respects the node-local proposal cap without changing verification.
    pub(crate) const fn gas_limits(self, proposal_period: u64) -> (u64, u64) {
        if proposal_period >= self.cornus_activation_period {
            (self.cornus_dag_gas_limit, self.cornus_pbft_gas_limit)
        } else {
            (self.default_dag_gas_limit, self.default_pbft_gas_limit)
        }
    }

    pub(crate) const fn proposal_weight_limit(self, proposal_period: u64) -> u64 {
        let (dag_gas_limit, _) = self.gas_limits(proposal_period);
        if self.proposal_dag_gas_limit < dag_gas_limit {
            self.proposal_dag_gas_limit
        } else {
            dag_gas_limit
        }
    }
}

/// Immutable native FinalChain construction input owned by application bootstrap.
///
/// The values are consumed exactly once to construct the FinalChain sibling over
/// the bootstrap's shared storage. Genesis accounts and validators are domain
/// inputs rather than bridge carriers, and constructor validation errors prevent
/// publication of the application root.
#[derive(Clone, Debug)]
pub struct ConsensusFinalChainConfig {
    /// Maximum gas available to one finalized block.
    pub block_gas_limit: FinalChainGas,
    /// Timestamp used when materializing the genesis FinalChain header.
    pub genesis_timestamp: u64,
    /// Effective genesis account balances after configured delegations.
    pub genesis_accounts: Vec<GenesisAccount>,
    /// Validators and delegation ledgers used to seed native DPoS state.
    pub genesis_validators: Vec<GenesisValidator>,
    /// DPoS eligibility and delegation policy at genesis.
    pub genesis_dpos: GenesisDposConfig,
    /// Rewards, hardfork, supply, and locking policy for native finalization.
    pub rewards: FinalChainRewardsConfig,
}

/// Consumed native bootstrap for one complete consensus application.
///
/// Bootstrap opens exactly one RocksDB-backed [`Storage`] from `storage_path`,
/// validates or initializes the requested schema version, initializes the
/// genesis hash only when absent and then verifies it exactly, constructs
/// FinalChain, and restores the DAG and PBFT graphs. The value is consumed so
/// configuration vectors cannot be reused to construct a competing root.
/// Any error returns no application, although successfully committed bootstrap
/// metadata remains durable and idempotent for a later retry.
#[derive(Debug)]
pub struct ConsensusApplicationBootstrap {
    /// Node data directory from which the native storage `db/` is opened.
    pub storage_path: PathBuf,
    /// Expected durable database major version; zero is reserved for uninitialized storage.
    pub schema_major: u32,
    /// Expected durable database minor version within `schema_major`.
    pub schema_minor: u32,
    /// Exact full-genesis identity persisted for fresh storage and verified on restart.
    pub storage_genesis_hash: H256,
    /// Native FinalChain construction input.
    pub final_chain: ConsensusFinalChainConfig,
    /// Native DAG/transaction/sortition and PBFT restoration input.
    pub consensus: ConsensusApplicationConfig,
}

impl ConsensusApplicationBootstrap {
    /// Opens storage and constructs one fully restored native application root.
    ///
    /// Existing nonzero schema majors must equal `schema_major`; same-major
    /// minor mismatches fail closed because legacy schema migrations are not a
    /// native bootstrap responsibility. Fresh storage atomically receives both values. Genesis is write-once and
    /// is compared byte-for-byte after the conditional write. The DAG service's
    /// distinct genesis-block hash remains part of its own configuration; it is
    /// intentionally not substituted for the full genesis-configuration identity.
    pub fn bootstrap(self) -> Result<ConsensusApplication> {
        if self.schema_major == 0 {
            bail!("CONSENSUS_APPLICATION_SCHEMA_MAJOR_ZERO");
        }
        let storage = Arc::new(
            Storage::new(Config::new(self.storage_path))
                .context("CONSENSUS_APPLICATION_STORAGE_OPEN_FAILED")?,
        );
        initialize_schema_version(&storage, self.schema_major, self.schema_minor)?;
        initialize_and_verify_genesis(&storage, self.storage_genesis_hash)?;

        let final_chain = Arc::new(
            FinalChain::new_with_rewards_config(
                storage.clone(),
                self.final_chain.block_gas_limit,
                self.final_chain.genesis_timestamp,
                self.final_chain.genesis_accounts,
                self.final_chain.genesis_validators,
                self.final_chain.genesis_dpos,
                self.final_chain.rewards,
            )
            .context("CONSENSUS_APPLICATION_FINAL_CHAIN_RESTORE_FAILED")?,
        );

        ConsensusApplication::restore_with_final_chain(storage, final_chain, self.consensus)
    }
}

fn initialize_schema_version(
    storage: &Storage,
    expected_major: u32,
    expected_minor: u32,
) -> Result<()> {
    let stored_major = storage
        .metadata()
        .status_field(StatusField::DbMajorVersion as u8)
        .context("CONSENSUS_APPLICATION_SCHEMA_MAJOR_READ_FAILED")?;
    let stored_minor = storage
        .metadata()
        .status_field(StatusField::DbMinorVersion as u8)
        .context("CONSENSUS_APPLICATION_SCHEMA_MINOR_READ_FAILED")?;

    if stored_major != 0 && stored_major != u64::from(expected_major) {
        bail!(
            "CONSENSUS_APPLICATION_SCHEMA_MAJOR_MISMATCH: stored {stored_major}, expected {expected_major}"
        );
    }
    if stored_major == u64::from(expected_major) && stored_minor == u64::from(expected_minor) {
        return Ok(());
    }
    if stored_major != 0 {
        bail!(
            "CONSENSUS_APPLICATION_SCHEMA_MINOR_MISMATCH: stored {stored_minor}, expected {expected_minor}"
        );
    }

    let mut batch = storage.create_write_batch();
    storage.metadata().write_status_field_in_batch(
        &mut batch,
        StatusField::DbMajorVersion as u8,
        u64::from(expected_major),
    )?;
    storage.metadata().write_status_field_in_batch(
        &mut batch,
        StatusField::DbMinorVersion as u8,
        u64::from(expected_minor),
    )?;
    storage
        .commit_write_batch_with_sync(batch, true)
        .context("CONSENSUS_APPLICATION_SCHEMA_UPDATE_FAILED")
}

fn initialize_and_verify_genesis(storage: &Storage, expected: H256) -> Result<()> {
    storage
        .metadata()
        .set_genesis_hash_if_empty(expected.as_bytes())
        .context("CONSENSUS_APPLICATION_GENESIS_INITIALIZATION_FAILED")?;
    let stored = storage
        .metadata()
        .genesis_hash()
        .context("CONSENSUS_APPLICATION_GENESIS_READ_FAILED")?;
    if stored.as_deref() != Some(expected.as_bytes()) {
        bail!("CONSENSUS_APPLICATION_GENESIS_MISMATCH");
    }
    Ok(())
}

/// Native Rust-enabled consensus composition and lifetime owner.
///
/// The fields are private and share one `Arc<Storage>`. DAG initialization is
/// prepared without durable mutation, PBFT is then restored, and only then is
/// the initial proposal-period mapping published. Consequently a late sibling
/// error returns no root and leaves no initial DAG proposal-period mapping. Other
/// idempotent restoration defaults may already be durable; atomicity here is the
/// publication of the live root, not transactional rollback of bootstrap rows.
pub struct ConsensusApplication {
    storage: Arc<Storage>,
    final_chain: Arc<FinalChain>,
    pbft: Arc<PbftService>,
    dag_transaction: Arc<DagTransactionService>,
    runtime: ConsensusApplicationRuntime,
}

/// Coherent hot PBFT status returned without exposing manager state or guards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsensusLiveStatus {
    pub period: u64,
    pub round: u64,
    pub step: u64,
    pub finalized_chain_size: u64,
    pub syncing_period: u64,
    pub sync_queue_size: u64,
}

/// Cold diagnostic DPoS vote totals for configured public signing identities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsensusVoteStatus {
    pub current_node_votes: Option<u64>,
    pub total_eligible_votes: Option<u64>,
}

/// One ordered observation from the versioned production-root storage conformance scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageConformanceObservation {
    pub key: String,
    pub value: String,
}

/// Canonical transaction-gossip admission entering the application once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionPacketIngressRequest {
    /// Public admission policy plus the exact canonical packet transaction.
    pub submission: PublicTransactionSubmissionRequest,
    /// Stable transport peer identity used only by the caller's effect executor.
    pub peer_id: [u8; 64],
    /// Whether an accepted transaction should be fanned out to other peers.
    pub rebroadcast: bool,
}

/// Native transaction-ingress decision and named leaf effects.
///
/// Observer and gossip effects are emitted only for a newly inserted canonical
/// transaction. Known/finalized/rejected packets remain successful operation
/// reports with both effects false.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionPacketIngressReport {
    pub submission: PublicTransactionSubmissionReport,
    pub peer_id: [u8; 64],
    pub observe_transaction: bool,
    pub gossip_transaction: bool,
    pub transaction_rlp: Vec<u8>,
}

/// Canonical DAG block plus transaction payloads entering native admission once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagBlockIngressRequest {
    pub block_rlp: Vec<u8>,
    pub transaction_rlps: Vec<Vec<u8>>,
    pub proposed: bool,
}

/// Terminal DAG admission and exact public/transport leaf selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagBlockIngressReport {
    pub block_hash: H256,
    /// Decoded DAG level used only for compact peer-progress bookkeeping.
    pub block_level: u64,
    pub accepted: bool,
    pub duplicate: bool,
    pub reject_code: u32,
    pub observe_block: bool,
    pub gossip_block: bool,
    pub block_rlp: Vec<u8>,
}

/// Ordered canonical DAG-sync bundle admitted by one application operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagSyncIngressRequest {
    pub transactions: Vec<TransactionPacketIngressRequest>,
    pub blocks: Vec<DagBlockIngressRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagSyncIngressReport {
    pub transactions: Vec<TransactionPacketIngressReport>,
    pub blocks: Vec<DagBlockIngressReport>,
    pub accepted: bool,
}

impl ConsensusApplication {
    /// Runs the closed v1 storage conformance scenario against this production-composed root.
    ///
    /// The operation requires a fresh fixture root and returns an ordered transcript. It intentionally exposes no
    /// storage handle, column selector, iterator, or caller-owned batch; native repositories own every write group.
    pub fn run_storage_conformance_v1(&self) -> Result<Vec<StorageConformanceObservation>> {
        let storage = self.storage.as_ref();
        let mut observations = Vec::new();
        let mut observe = |key: &str, value: String| {
            observations.push(StorageConformanceObservation {
                key: key.to_owned(),
                value,
            });
        };
        let boolean = |value: bool| value.to_string();
        let optional_u64 =
            |value: Option<u64>| value.map_or_else(|| "none".into(), |v| v.to_string());
        let optional_u32 =
            |value: Option<u32>| value.map_or_else(|| "none".into(), |v| v.to_string());

        let dag_hash_1 = H256::repeat_byte(0x11);
        let dag_hash_2 = H256::repeat_byte(0x22);
        let dag_hash_3 = H256::repeat_byte(0x33);
        let dag_missing = H256::repeat_byte(0xee);
        if storage.dag().exists(dag_hash_1)?
            || storage.transaction().exists(H256::repeat_byte(0x51))?
        {
            bail!("STORAGE_CONFORMANCE_V1_REQUIRES_FRESH_ROOT");
        }

        observe(
            "status_default_executed_blk",
            storage
                .metadata()
                .status_field(StatusField::ExecutedBlkCount as u8)?
                .to_string(),
        );
        observe(
            "pbft_mgr_field_default_round",
            storage.pbft().manager_field(0)?.unwrap_or(1).to_string(),
        );
        observe(
            "pbft_mgr_status_default_executed_block",
            boolean(storage.pbft().manager_status(0)?.unwrap_or(false)),
        );
        observe(
            "proposal_period_missing",
            optional_u64(storage.dag().proposal_period_at_level(1_000_001)?),
        );
        observe(
            "period_lambda_missing",
            optional_u32(storage.metadata().period_lambda(7, false)?),
        );
        observe(
            "rounds_count_dynamic_lambda_default",
            storage
                .metadata()
                .rounds_count_dynamic_lambda()?
                .to_string(),
        );
        observe(
            "genesis_present_before",
            boolean(storage.metadata().genesis_hash()?.is_some()),
        );

        let mut batch = storage.create_write_batch();
        storage
            .metadata()
            .write_status_field_in_batch(&mut batch, 2, 11)?;
        storage
            .pbft()
            .write_manager_field_in_batch(&mut batch, 0, 17)?;
        storage
            .pbft()
            .write_manager_status_in_batch(&mut batch, 2, true)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        storage.dag().write_proposal_period_at_level(100, 50)?;
        storage.metadata().write_period_lambda(7, 42)?;
        storage.metadata().write_rounds_count_dynamic_lambda(23)?;
        observe(
            "status_trx_count_after_save",
            storage.metadata().status_field(2)?.to_string(),
        );
        observe(
            "pbft_mgr_field_round_after_save",
            storage.pbft().manager_field(0)?.unwrap_or(1).to_string(),
        );
        observe(
            "pbft_mgr_status_next_voted_soft_after_save",
            boolean(storage.pbft().manager_status(2)?.unwrap_or(false)),
        );
        observe(
            "proposal_period_level_100_after_save",
            optional_u64(storage.dag().proposal_period_at_level(100)?),
        );
        observe(
            "period_lambda_exact_after_save",
            optional_u32(storage.metadata().period_lambda(7, false)?),
        );
        observe(
            "period_lambda_closest_after_save",
            optional_u32(storage.metadata().period_lambda(8, true)?),
        );
        observe(
            "rounds_count_dynamic_lambda_after_save",
            storage
                .metadata()
                .rounds_count_dynamic_lambda()?
                .to_string(),
        );

        observe(
            "dag_missing_block",
            boolean(!storage.dag().exists(dag_missing)?),
        );
        observe(
            "dag_missing_period",
            boolean(storage.dag().period_optional(dag_missing)?.is_none()),
        );
        let mut batch = storage.create_write_batch();
        storage
            .dag()
            .write_in_batch(&mut batch, dag_hash_1, 1, &[0xc0], 1, 1)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        let mut batch = storage.create_write_batch();
        storage
            .dag()
            .write_in_batch(&mut batch, dag_hash_2, 1, &[0xc0], 2, 3)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "dag_saved_primary",
            boolean(storage.dag().exists(dag_hash_1)?),
        );
        observe(
            "dag_saved_batch",
            boolean(storage.dag().exists(dag_hash_2)?),
        );
        observe(
            "dag_level_1_count",
            storage.dag().hashes_at_level(1)?.len().to_string(),
        );
        let mut batch = storage.create_write_batch();
        storage
            .dag()
            .write_period_in_batch(&mut batch, dag_hash_1, 7, 2)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        let dag_period = storage
            .dag()
            .period_optional(dag_hash_1)?
            .context("STORAGE_CONFORMANCE_V1_DAG_PERIOD")?;
        observe("dag_period_lookup_found", "true".into());
        observe("dag_period_lookup_period", dag_period.0.to_string());
        observe("dag_period_lookup_position", dag_period.1.to_string());
        let mut batch = storage.create_write_batch();
        storage
            .dag()
            .update_counters_in_batch(&mut batch, &[(dag_hash_3, 2, 2)], 3, 6)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "dag_counters_nonzero",
            boolean(
                storage.metadata().status_field(3)? > 0 && storage.metadata().status_field(4)? > 0,
            ),
        );
        let mut batch = storage.create_write_batch();
        storage.dag().remove_in_batch(&mut batch, dag_hash_2)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "dag_removed_batch_hash",
            boolean(!storage.dag().exists(dag_hash_2)?),
        );
        observe("dag_last_level", storage.dag().last_level()?.to_string());
        observe(
            "dag_blocks_at_level_span_count",
            storage.dag().at_level_range(1, 2)?.len().to_string(),
        );

        let pbft_hash = H256::repeat_byte(0x44);
        let pbft_missing = H256::repeat_byte(0x45);
        let mut batch = storage.create_write_batch();
        storage
            .period()
            .write_pbft_period_in_batch(&mut batch, pbft_hash, 99)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "pbft_period_lookup_found",
            boolean(storage.period().by_pbft_hash(pbft_hash)?.is_some()),
        );
        observe(
            "pbft_period_lookup_value",
            optional_u64(storage.period().by_pbft_hash(pbft_hash)?),
        );
        observe(
            "pbft_period_lookup_missing",
            boolean(storage.period().by_pbft_hash(pbft_missing)?.is_none()),
        );
        observe(
            "pbft_block_in_db_found",
            boolean(storage.pbft().exists(pbft_hash)?),
        );
        observe(
            "pbft_block_in_db_missing",
            boolean(storage.pbft().exists(pbft_missing)?),
        );
        let pbft_head_hash = H256::repeat_byte(0x71);
        observe(
            "pbft_head_missing_len",
            storage
                .pbft()
                .head(pbft_head_hash)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );
        storage.pbft().write_head(pbft_head_hash, b"head")?;
        observe(
            "pbft_head_after_save_len",
            storage
                .pbft()
                .head(pbft_head_hash)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );

        let tx_hash_1 = H256::repeat_byte(0x51);
        let tx_hash_2 = H256::repeat_byte(0x52);
        let system_hash = H256::repeat_byte(0x53);
        let mut batch = storage.create_write_batch();
        storage
            .transaction()
            .write_in_batch(&mut batch, tx_hash_1, &[0xc0])?;
        storage
            .transaction()
            .write_in_batch(&mut batch, tx_hash_2, &[0xc0])?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "tx_hash_1_in_db",
            boolean(storage.transaction().exists(tx_hash_1)?),
        );
        observe(
            "tx_hash_1_finalized_before",
            boolean(storage.transaction().finalized(tx_hash_1)?),
        );
        let mut batch = storage.create_write_batch();
        storage
            .transaction()
            .write_location_in_batch(&mut batch, tx_hash_1, 12, 0, false)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "tx_hash_1_finalized_after",
            boolean(storage.transaction().finalized(tx_hash_1)?),
        );
        observe(
            "tx_hash_1_location_present",
            boolean(storage.transaction().location_rlp(tx_hash_1)?.is_some()),
        );
        observe(
            "tx_hash_1_lookup_nonempty",
            boolean(storage.transaction().rlp(tx_hash_1)?.is_some()),
        );
        observe(
            "tx_period_map_size",
            storage.transaction().all_with_period()?.len().to_string(),
        );
        let mut batch = storage.create_write_batch();
        storage
            .transaction()
            .remove_in_batch(&mut batch, tx_hash_2)?;
        storage.commit_write_batch_with_sync(batch, false)?;
        observe(
            "tx_hash_2_removed",
            boolean(!storage.transaction().exists(tx_hash_2)?),
        );
        observe(
            "tx_nonfinalized_count",
            storage
                .transaction()
                .all_nonfinalized_rlp()?
                .len()
                .to_string(),
        );
        observe(
            "tx_finalized_vector",
            format!(
                "{}{}",
                storage.transaction().finalized(tx_hash_1)? as u8,
                storage.transaction().finalized(tx_hash_2)? as u8
            ),
        );
        storage.transaction().write_system(system_hash, &[0xc0])?;
        observe(
            "system_tx_lookup_nonempty",
            boolean(storage.transaction().system_rlp(system_hash)?.is_some()),
        );
        let mut hashes = rlp::RlpStream::new_list(1);
        hashes.append(&system_hash);
        storage
            .transaction()
            .write_period_system_hashes(12, &hashes.out())?;
        observe("period_system_hashes_count", "1".into());
        storage
            .period()
            .write(33, &[0xc6, 0xc0, 0xc0, 0xc0, 0xe1, 0xc0, 0xc0])?;
        observe(
            "period_data_raw_len",
            storage.period().data_raw(33)?.len().to_string(),
        );

        let block_hash = H256::repeat_byte(0x61);
        let receipt_hash = H256::repeat_byte(0x62);
        let bloom_chunk = H256::repeat_byte(0x63);
        storage.final_chain().write_conformance_lookup_rows(
            99,
            b"meta",
            42,
            block_hash,
            b"blk",
            receipt_hash,
            b"rcp",
            bloom_chunk,
            b"blm",
            15,
            &[0xc0],
        )?;
        observe(
            "final_chain_meta_len",
            storage
                .final_chain()
                .meta_value(99)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );
        observe(
            "final_chain_block_len",
            storage
                .final_chain()
                .block_header_raw(42)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );
        observe(
            "final_chain_hash_len",
            storage
                .final_chain()
                .block_hash_by_number(42)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );
        let number = storage
            .final_chain()
            .block_number_by_hash(block_hash)?
            .context("STORAGE_CONFORMANCE_V1_FINAL_CHAIN_NUMBER")?;
        let number = u64::from_le_bytes(
            number
                .as_slice()
                .try_into()
                .context("STORAGE_CONFORMANCE_V1_FINAL_CHAIN_NUMBER_SIZE")?,
        );
        observe("final_chain_number_by_hash", number.to_string());
        observe(
            "final_chain_receipt_len",
            storage
                .final_chain()
                .receipt_by_trx_hash(receipt_hash)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );
        observe(
            "final_chain_blooms_len",
            storage
                .final_chain()
                .log_blooms_chunk_raw(bloom_chunk)?
                .unwrap_or_default()
                .len()
                .to_string(),
        );
        observe(
            "final_chain_receipts_by_period_count",
            usize::from(!storage.period().receipt(15)?.is_empty()).to_string(),
        );
        Ok(observations)
    }

    /// Executes one atomic, root-owned light-history pruning task.
    ///
    /// Exact exclusive cutoffs are validated by storage. The operation exposes no storage handle and is idempotent
    /// for repeated requests with the same policy.
    pub fn prune_light_history(
        &self,
        request: rustaxa_storage::LightHistoryPruneRequest,
    ) -> Result<rustaxa_storage::LightHistoryPruneReport> {
        self.storage.prune_light_history(request)
    }

    /// Validates one DAG-sync transaction without publishing it to the live
    /// queue. Known transactions preserve the legacy verification fast path;
    /// new transactions run canonical envelope policy only and become durable
    /// solely if a sequentially accepted DAG block references them.
    fn validate_dag_sync_transaction(
        &self,
        request: TransactionPacketIngressRequest,
    ) -> Result<TransactionPacketIngressReport> {
        let transaction_rlp = request.submission.transaction_rlp.clone();
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)
            .context("DAG_SYNC_TRANSACTION_DECODE_FAILED")?;
        let known = self.dag_transaction.transaction_is_known(envelope.hash.0)?;
        let verification = if known {
            TransactionManagerVerifyTransactionStatus::Accepted
        } else {
            plan_verify_transaction(TransactionManagerVerifyTransactionFact {
                tx_hash: envelope.hash,
                chain_id: envelope.chain_id,
                expected_chain_id: request.submission.expected_chain_id,
                gas_limit: envelope.gas,
                max_gas_limit: request.submission.maximum_gas_limit,
                last_block_number: request.submission.last_block_number,
                cornus_active: request.submission.cornus_active,
                intrinsic_gas_covered: envelope.intrinsic_gas_covered,
                signature_valid: envelope.signature_valid,
                gas_price: envelope.gas_price,
                minimum_gas_price: request.submission.minimum_gas_price,
            })?
            .status
        };
        let accepted = verification == TransactionManagerVerifyTransactionStatus::Accepted;
        Ok(TransactionPacketIngressReport {
            submission: PublicTransactionSubmissionReport {
                transaction_hash: envelope.hash,
                accepted,
                message: (!accepted)
                    .then_some("DAG_SYNC_TRANSACTION_REJECTED".to_owned())
                    .unwrap_or_default(),
                verification_status: verification,
                queue_status: known.then_some(TransactionQueueInsertStatus::Known),
                transaction_observed: false,
            },
            peer_id: request.peer_id,
            observe_transaction: false,
            gossip_transaction: false,
            transaction_rlp,
        })
    }

    /// Admits one canonical transaction packet and selects exact transport and
    /// public-observer leaves without exposing transaction-manager state.
    pub fn ingest_transaction_packet<E: ConsensusExecutionPort>(
        &self,
        request: TransactionPacketIngressRequest,
        execution: &E,
    ) -> Result<TransactionPacketIngressReport> {
        let transaction_rlp = request.submission.transaction_rlp.clone();
        let peer_id = request.peer_id;
        let rebroadcast = request.rebroadcast;
        let submission =
            self.submit_public_transaction_with_execution(request.submission, execution)?;
        let newly_inserted = submission.transaction_observed;
        Ok(TransactionPacketIngressReport {
            submission,
            peer_id,
            observe_transaction: newly_inserted,
            gossip_transaction: newly_inserted && rebroadcast,
            transaction_rlp,
        })
    }

    /// Verifies and publishes one canonical DAG block through native services.
    ///
    /// Transactions carried by DAG sync must first enter the canonical
    /// transaction ingress operation. This call then resolves them from native
    /// queue/sidecar/storage state, owns signature/DPoS/VDF/gas verification,
    /// and atomically publishes the block and transaction persistence.
    pub fn ingest_dag_block_packet<E: ConsensusExecutionPort>(
        &self,
        request: DagBlockIngressRequest,
        execution: &E,
    ) -> Result<DagBlockIngressReport> {
        let block = rustaxa_types::dag::DagBlock::try_from(
            rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&request.block_rlp),
        )?;
        let block_hash = crate::dag::dag_manager_block_from_rlp(&request.block_rlp)?.hash;
        let supplied_payloads = request
            .transaction_rlps
            .iter()
            .map(|bytes| {
                LegacyTransactionEnvelope::decode(bytes).map(|value| (value.hash, bytes.clone()))
            })
            .collect::<Result<std::collections::HashMap<_, _>>>()?;
        let supplied = supplied_payloads.keys().copied().collect::<Vec<_>>();
        self.dag_transaction
            .begin_verify_block_session(DagVerifyBlockSessionInput {
                block_hash: block_hash.0,
                block_level: block.level,
                pivot: block.pivot.0,
                tips: block.tips.clone(),
                block_transaction_hashes: block.transactions.clone(),
                supplied_transaction_hashes: supplied.clone(),
                block_rlp: request.block_rlp.clone(),
            })?;
        let transaction_query = self.dag_transaction.prepare_verify_block_transactions()?;
        let views = transaction_query.transactions.clone();
        let native_payloads = views
            .iter()
            .filter(|view| view.found && !view.tx_rlp.is_empty())
            .map(|view| (H256::from(view.hash), view.tx_rlp.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let resolved_transactions = resolve_dag_block_transaction_payloads(
            &block.transactions,
            &supplied_payloads,
            &native_payloads,
        );
        let mut step = self.dag_transaction.complete_verify_block_transactions(
            DagVerifyBlockTransactionCompletionReport {
                cursor_id: transaction_query.cursor_id,
                proposal_period: transaction_query.proposal_period,
                account_nonce_facts: Vec::new(),
            },
        )?;
        if !step.complete {
            step = self
                .dag_transaction
                .report_verify_block_authorization_with_final_chain(&self.final_chain)?;
        }
        if !step.complete {
            let period_hash = self
                .dag_transaction
                .dag_period_block_hash(step.proposal_period)?;
            if !period_hash.found {
                bail!("DAG_BLOCK_INGRESS_PROPOSAL_PERIOD_HASH_MISSING");
            }
            step = self
                .dag_transaction
                .verify_block_vdf(DagVerifyBlockVdfRequest {
                    cursor_id: step.cursor_id,
                    block_rlp: request.block_rlp.clone(),
                    block_level: block.level,
                    proposal_period_hash: period_hash.hash,
                })?;
        }
        if !step.complete {
            let effect_id = self.runtime.next_operation_effect()?;
            let gas = execution.estimate_dag_transaction_gas(&crate::DagGasEstimateRequest {
                effect_id,
                proposal_period: step.proposal_period,
                transactions: resolved_transactions
                    .iter()
                    .map(|transaction| crate::DagGasEstimateInput {
                        hash: transaction.hash.0,
                        transaction_rlp: transaction.transaction_rlp.clone(),
                    })
                    .collect(),
            })?;
            self.runtime
                .validate_operation_report(effect_id, gas.effect_id)?;
            if !gas.succeeded {
                bail!("DAG_BLOCK_INGRESS_GAS_FAILED: {}", gas.error_code);
            }
            let estimated_transactions_weight = gas
                .estimates
                .iter()
                .try_fold(0_u64, |sum, value| sum.checked_add(value.gas_used))
                .context("DAG_BLOCK_INGRESS_GAS_OVERFLOW")?;
            step = self
                .dag_transaction
                .report_verify_block_gas(DagVerifyBlockGasReport {
                    block_gas_estimation: block.gas_estimation,
                    estimated_transactions_weight,
                    dag_gas_limit: self.runtime.dag_gas_limit(step.proposal_period),
                    pbft_gas_limit: self.runtime.pbft_gas_limit(step.proposal_period),
                })?;
        }
        if step.reject_code != 0 {
            return Ok(DagBlockIngressReport {
                block_hash,
                block_level: block.level,
                accepted: false,
                duplicate: false,
                reject_code: step.reject_code,
                observe_block: false,
                gossip_block: false,
                block_rlp: request.block_rlp,
            });
        }
        let prepared = self
            .dag_transaction
            .prepare_add_block(DagAddBlockPrepareRequest {
                expected_hash: block_hash,
                block_rlp: request.block_rlp.clone(),
                validate_hash: true,
                save: true,
                proposed: request.proposed,
                transactions: resolved_transactions,
            })?;
        if prepared.cursor_id == 0 {
            let reject_code = if prepared.expired {
                crate::dag::DAG_VERIFY_REJECT_EXPIRED_BLOCK
            } else if !prepared.accepted && !prepared.missing_references.is_empty() {
                crate::dag::DAG_VERIFY_REJECT_MISSING_TIP
            } else if !prepared.accepted {
                crate::dag::DAG_VERIFY_REJECT_ADD_BLOCK_METADATA
            } else {
                0
            };
            return Ok(DagBlockIngressReport {
                block_hash,
                block_level: block.level,
                accepted: prepared.accepted,
                duplicate: prepared.duplicate,
                reject_code,
                observe_block: false,
                gossip_block: false,
                block_rlp: request.block_rlp,
            });
        }
        let nonces = self.load_operation_account_nonces(
            prepared
                .account_requests
                .iter()
                .map(|value| value.sender.0)
                .collect(),
            execution,
        )?;
        let commit = self
            .dag_transaction
            .complete_add_block(DagAddBlockCompletion {
                cursor_id: prepared.cursor_id,
                account_nonce_facts: nonces
                    .into_iter()
                    .enumerate()
                    .map(|(input_index, account_nonce)| DagAddBlockAccountNonceFact {
                        input_index: input_index as u64,
                        account_nonce,
                    })
                    .collect(),
            })?;
        Ok(DagBlockIngressReport {
            block_hash,
            block_level: block.level,
            accepted: commit.accepted,
            duplicate: false,
            reject_code: 0,
            observe_block: commit.emit_verified,
            gossip_block: commit.gossip,
            block_rlp: request.block_rlp,
        })
    }

    fn load_operation_account_nonces<E: ConsensusExecutionPort>(
        &self,
        addresses: Vec<[u8; 20]>,
        execution: &E,
    ) -> Result<Vec<U256>> {
        let effect_id = self.runtime.next_operation_effect()?;
        let report = execution.load_final_chain_account_facts(
            &crate::consensus_application_runtime::FinalChainAccountFactsRequest {
                effect_id,
                addresses: addresses.clone(),
            },
        )?;
        self.runtime
            .validate_operation_report(effect_id, report.effect_id)?;
        if !report.succeeded || report.accounts.len() != addresses.len() {
            bail!("DAG_BLOCK_INGRESS_ACCOUNT_FACTS_FAILED");
        }
        report
            .accounts
            .into_iter()
            .zip(addresses)
            .map(|(account, expected)| {
                if account.address != expected {
                    bail!("DAG_BLOCK_INGRESS_ACCOUNT_FACTS_ORDER_MISMATCH");
                }
                Ok(U256::from_big_endian(&account.nonce))
            })
            .collect()
    }

    /// Validates bundle transactions, then commits only transactions referenced
    /// by each sequentially accepted DAG block.
    ///
    /// The first invalid transaction rejects the packet before any block-side
    /// mutation. A later block rejection retains prior block commits but never
    /// publishes transactions belonging only to that or subsequent blocks.
    pub fn ingest_dag_sync_packet<E: ConsensusExecutionPort>(
        &self,
        request: DagSyncIngressRequest,
        execution: &E,
    ) -> Result<DagSyncIngressReport> {
        let mut transactions = Vec::with_capacity(request.transactions.len());
        let mut payloads = std::collections::HashMap::new();
        for transaction in request.transactions {
            let report = self.validate_dag_sync_transaction(transaction)?;
            payloads
                .entry(report.submission.transaction_hash)
                .or_insert_with(|| report.transaction_rlp.clone());
            let rejected = !report.submission.accepted;
            transactions.push(report);
            if rejected {
                return Ok(DagSyncIngressReport {
                    transactions,
                    blocks: Vec::new(),
                    accepted: false,
                });
            }
        }
        let mut blocks = Vec::with_capacity(request.blocks.len());
        let mut accepted = true;
        for mut block in request.blocks {
            block.transaction_rlps = select_dag_sync_transaction_payloads(
                &crate::dag::dag_block_transaction_hashes(&block.block_rlp)?,
                &payloads,
            );
            let report = self.ingest_dag_block_packet(block, execution)?;
            accepted &= report.accepted;
            let rejected = !report.accepted;
            blocks.push(report);
            if rejected {
                break;
            }
        }
        Ok(DagSyncIngressReport {
            transactions,
            blocks,
            accepted,
        })
    }

    /// Returns canonical non-finalized DAG sync bytes from one native snapshot.
    ///
    /// Known hashes are excluded natively; blocks and first-seen transactions
    /// preserve deterministic DAG order. No manager or storage handle crosses
    /// the transport boundary.
    pub fn prepare_dag_sync_egress(
        &self,
        known_hashes: Vec<H256>,
    ) -> Result<DagRuntimeNonFinalizedSyncPayload> {
        self.dag_transaction.dag_non_finalized_sync(known_hashes)
    }

    /// Returns the bounded canonical transaction-gossip snapshot.
    pub fn prepare_transaction_gossip(
        &self,
        max_count: u64,
    ) -> Result<Vec<crate::TransactionGossipAccount>> {
        self.dag_transaction.transaction_gossip_snapshot(max_count)
    }

    /// Loads slashing submitter nonce/balance facts through one exact external
    /// account operation without exposing FinalChain or manager handles.
    pub fn slashing_submitters_with_execution<E: ConsensusExecutionPort>(
        &self,
        execution: &E,
    ) -> Result<Vec<crate::SlashingSubmitterIdentity>> {
        let addresses = self
            .runtime
            .signing_identities()
            .iter()
            .map(|identity| identity.address)
            .collect::<Vec<_>>();
        let effect_id = self.runtime.next_operation_effect()?;
        let report = execution.load_final_chain_account_facts(
            &crate::consensus_application_runtime::FinalChainAccountFactsRequest {
                effect_id,
                addresses: addresses.clone(),
            },
        )?;
        self.runtime
            .validate_operation_report(effect_id, report.effect_id)?;
        if !report.succeeded || report.accounts.len() != addresses.len() {
            bail!("CONSENSUS_SLASHING_SUBMITTER_FACTS_FAILED");
        }
        report
            .accounts
            .iter()
            .zip(addresses)
            .enumerate()
            .map(|(wallet_index, (account, address))| {
                if account.address != address {
                    bail!("CONSENSUS_SLASHING_SUBMITTER_FACTS_ORDER_MISMATCH");
                }
                Ok(crate::SlashingSubmitterIdentity {
                    wallet_index,
                    address,
                    nonce: account
                        .found
                        .then(|| U256::from_big_endian(&account.nonce))
                        .unwrap_or_default(),
                    balance: account
                        .found
                        .then(|| U256::from_big_endian(&account.balance))
                        .unwrap_or_default(),
                })
            })
            .collect()
    }
    /// Submits canonical public transaction bytes through the native owner.
    ///
    /// The caller supplies only exact external-EVM account facts; Rust owns
    /// envelope decoding, verification, duplicate handling, queue mutation,
    /// and observer-effect selection.
    pub fn submit_public_transaction(
        &self,
        request: PublicTransactionSubmissionRequest,
        final_chain_facts: PublicTransactionFinalChainFacts,
    ) -> Result<PublicTransactionSubmissionReport> {
        self.dag_transaction
            .submit_public_transaction(request, final_chain_facts)
    }

    /// Submits canonical transaction bytes using the configured external-EVM leaf.
    ///
    /// Rust decodes the sender before issuing one exact account query, validates
    /// report identity/order/head, resolves finalized membership from native
    /// storage, and only then enters queue admission. Host failure or malformed
    /// reports leave queue and observer state unchanged.
    pub fn submit_public_transaction_with_execution<E: ConsensusExecutionPort>(
        &self,
        request: PublicTransactionSubmissionRequest,
        execution: &E,
    ) -> Result<PublicTransactionSubmissionReport> {
        let envelope = LegacyTransactionEnvelope::decode(&request.transaction_rlp)
            .context("PUBLIC_TRANSACTION_DECODE_FAILED")?;
        let sender = envelope
            .sender
            .context("PUBLIC_TRANSACTION_SENDER_MISSING")?;
        let effect_id = self.runtime.next_operation_effect()?;
        let report = execution.load_final_chain_account_facts(
            &crate::consensus_application_runtime::FinalChainAccountFactsRequest {
                effect_id,
                addresses: vec![sender.0],
            },
        )?;
        self.runtime
            .validate_operation_report(effect_id, report.effect_id)?;
        ensure_public_account_report(&report, sender.0, request.last_block_number)?;
        let finalized_period = self
            .final_chain
            .transaction_location(envelope.hash.0)?
            .map(|bytes| {
                Rlp::new(&bytes)
                    .val_at::<u64>(0)
                    .context("PUBLIC_TRANSACTION_FINALIZED_LOCATION_DECODE")
            })
            .transpose()?;
        let account = &report.accounts[0];
        self.submit_public_transaction(
            request,
            PublicTransactionFinalChainFacts {
                sender: sender.0,
                account_found: account.found,
                account_nonce: U256::from_big_endian(&account.nonce),
                account_balance: U256::from_big_endian(&account.balance),
                finalized_period,
            },
        )
    }
    /// Returns one coherent application-root PBFT/queue status snapshot.
    pub fn consensus_live_status(&self) -> Result<ConsensusLiveStatus> {
        let status = self.pbft.application_status_snapshot()?;
        Ok(ConsensusLiveStatus {
            period: status.period,
            round: status.round,
            step: status.step,
            finalized_chain_size: status.finalized_chain_size,
            syncing_period: status.syncing_period,
            sync_queue_size: status.sync_queue_size,
        })
    }

    /// Computes local and total eligible vote diagnostics at the finalized head.
    pub fn consensus_vote_status(&self) -> Result<ConsensusVoteStatus> {
        let period = self.pbft.pbft_chain_head().size;
        let addresses: Vec<_> = self
            .runtime
            .signing_identities()
            .iter()
            .map(|identity| identity.address)
            .collect();
        let total_eligible_votes = self
            .final_chain
            .pbft_dpos_eligible_total_vote_count(period)?;
        let current_node_votes = self
            .final_chain
            .pbft_dpos_eligible_wallet_vote_counts(period, &addresses)?
            .map(|votes| votes.into_iter().map(|vote| vote.vote_count).sum());
        Ok(ConsensusVoteStatus {
            current_node_votes,
            total_eligible_votes,
        })
    }
    /// Creates the read-only client API bound to this application's storage and live PBFT owner.
    ///
    /// The returned API extends the lifetime of existing root-owned services;
    /// it cannot construct or mutate a competing consensus runtime.
    #[doc(hidden)]
    pub fn consensus_query_api_for_bridge(&self) -> crate::ConsensusQueryApi {
        crate::ConsensusQueryApi::new_live(
            Arc::clone(&self.storage),
            Arc::clone(&self.pbft),
            Arc::clone(&self.final_chain),
            Arc::clone(&self.dag_transaction),
        )
    }

    fn restore_with_final_chain(
        storage: Arc<Storage>,
        final_chain: Arc<FinalChain>,
        config: ConsensusApplicationConfig,
    ) -> Result<Self> {
        let (dag_transaction, max_levels_per_period) =
            DagTransactionService::restore_deferred_mapping(
                storage.clone(),
                config.dag_transaction,
            )?;
        let pbft = PbftService::restore(storage.clone(), config.pbft)?;
        dag_transaction.complete_restore_mapping(max_levels_per_period)?;
        let dag_proposers = dag_proposer_inputs(&config.signing_identities, config.dag_proposer);
        let runtime = ConsensusApplicationRuntime::new_with_proposers(
            config.signing_identities,
            config.polling_interval_ms,
            dag_proposers,
            config.dag_proposer,
        )?;
        Ok(Self {
            storage,
            final_chain,
            pbft: Arc::new(pbft),
            dag_transaction: Arc::new(dag_transaction),
            runtime,
        })
    }

    /// Runs the restartable native consensus daemon on the calling thread.
    pub fn run_consensus<P, S, T, E, V, O>(
        &self,
        process: &P,
        signer: &S,
        transport: &T,
        evm: &E,
        vdf: &V,
        observer: &O,
    ) -> Result<ConsensusRunExit>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        E: ConsensusExecutionPort,
        V: crate::ConsensusVdfPort,
        O: crate::ConsensusObserverPort,
    {
        self.runtime.run(
            self.pbft.as_ref(),
            self.dag_transaction.as_ref(),
            self.final_chain.as_ref(),
            process,
            signer,
            transport,
            evm,
            vdf,
            observer,
        )
    }

    /// Borrows the application-owned FinalChain for thin Rust bridge dispatch.
    ///
    /// This accessor is not exported through CXX. External-EVM and query
    /// adapters may invoke exact native operations while the opaque application
    /// root retains lifetime and construction authority.
    #[doc(hidden)]
    pub fn final_chain_for_bridge(&self) -> &FinalChain {
        self.final_chain.as_ref()
    }

    /// Clones the private FinalChain owner for a root-bound Rust adapter.
    ///
    /// The clone remains inside Rust and cannot construct a competing runtime;
    /// it only extends the lifetime of the root's existing FinalChain sibling.
    #[doc(hidden)]
    pub fn final_chain_arc_for_bridge(&self) -> Arc<FinalChain> {
        Arc::clone(&self.final_chain)
    }

    /// Borrows PBFT ownership for thin Rust bridge task dispatch.
    ///
    /// This Rust-only migration seam is intentionally not exported through
    /// CXX. It must disappear as operation-shaped application tasks absorb the
    /// remaining manager facade calls.
    #[doc(hidden)]
    pub fn pbft_for_bridge(&self) -> &PbftService {
        self.pbft.as_ref()
    }

    /// Clones the private PBFT owner for a root-bound Rust adapter.
    ///
    /// The clone never crosses CXX and cannot construct a competing runtime;
    /// it lets the network bridge compose packet ingress with the exact vote
    /// service owned by this application root.
    #[doc(hidden)]
    pub fn pbft_arc_for_bridge(&self) -> Arc<PbftService> {
        Arc::clone(&self.pbft)
    }
}

fn dag_proposer_inputs(
    identities: &[SigningIdentity],
    config: DagProposerConfig,
) -> Vec<crate::dag_service::DagProposerSessionBeginInput> {
    let shards = config.total_transaction_shards.max(1);
    identities
        .iter()
        .map(|identity| {
            let max_retry_count = 20 + u64::from(identity.address[0]) % 200;
            let shard_prefix = u32::from_be_bytes([
                0,
                identity.address[0],
                identity.address[1],
                identity.address[2],
            ]);
            crate::dag_service::DagProposerSessionBeginInput {
                max_non_finalized_transactions: 1_000_000,
                dag_expiry_level_limit: 1_000,
                wallet_vrf_public_key: identity.vrf_public_key,
                proposer_address: identity.address,
                max_non_finalized_dag_blocks: 100,
                max_non_finalized_dag_blocks_low_difficulty: 5,
                max_retry_count,
                proposal_weight_limit: config.proposal_weight_limit(0),
                total_transaction_shards: shards,
                node_transaction_shard: (shard_prefix % u32::from(shards)) as u16,
                shard_period_interval: 10,
                pbft_gas_limit: config.gas_limits(0).1,
                dag_gas_limit: config.gas_limits(0).0,
                max_tips: 16,
            }
        })
        .collect()
}

fn ensure_public_account_report(
    report: &crate::consensus_application_runtime::FinalChainAccountFactsReport,
    sender: [u8; 20],
    expected_block: u64,
) -> Result<()> {
    if !report.succeeded {
        bail!(
            "PUBLIC_TRANSACTION_ACCOUNT_FACTS_FAILED: {}",
            report.error_code
        );
    }
    if report.observed_block != expected_block {
        bail!("PUBLIC_TRANSACTION_ACCOUNT_FACTS_HEAD_MISMATCH");
    }
    if report.accounts.len() != 1 || report.accounts[0].address != sender {
        bail!("PUBLIC_TRANSACTION_ACCOUNT_FACTS_SHAPE_MISMATCH");
    }
    Ok(())
}

/// Temporary Rust-only PBFT dispatch compatibility for bridge migration.
///
/// CXX sees only the opaque application root and cannot retrieve or construct
/// the PBFT owner. This implementation disappears as the remaining PBFT facade
/// calls become operation-shaped application tasks.
impl Deref for ConsensusApplication {
    type Target = PbftService;

    fn deref(&self) -> &Self::Target {
        self.pbft.as_ref()
    }
}

/// Returns the deterministic native bootstrap shared by downstream boundary tests.
///
/// The fixture uses one storage owner and the production bootstrap path. Callers
/// may vary validators and the DPoS delegation delay; all other consensus inputs
/// are fixed. This helper does not open storage or publish a partial application.
#[doc(hidden)]
pub fn consensus_application_test_bootstrap(
    storage_path: PathBuf,
    genesis_validators: Vec<GenesisValidator>,
    delegation_delay: u64,
) -> ConsensusApplicationBootstrap {
    let mut consensus = deterministic_test_config();
    consensus
        .dag_transaction
        .transaction
        .gas_pricer_config
        .minimum_price = U256::zero();
    ConsensusApplicationBootstrap {
        storage_path,
        schema_major: 1,
        schema_minor: 0,
        storage_genesis_hash: H256::repeat_byte(1),
        final_chain: ConsensusFinalChainConfig {
            block_gas_limit: FinalChainGas::ZERO,
            genesis_timestamp: 0,
            genesis_accounts: Vec::new(),
            genesis_validators,
            genesis_dpos: GenesisDposConfig {
                eligibility_balance_threshold: U256::from(1_000).into(),
                vote_eligibility_balance_step: U256::from(1_000).into(),
                validator_maximum_stake: U256::from(30_000).into(),
                delegation_delay,
                ..GenesisDposConfig::default()
            },
            rewards: FinalChainRewardsConfig {
                aspen_part_one_period: u64::MAX.into(),
                ..FinalChainRewardsConfig::default()
            },
        },
        consensus,
    }
}

fn deterministic_test_config() -> ConsensusApplicationConfig {
    ConsensusApplicationConfig {
        signing_identities: Vec::new(),
        polling_interval_ms: 100,
        dag_proposer: DagProposerConfig {
            total_transaction_shards: 1,
            proposal_dag_gas_limit: 1_000_000,
            default_dag_gas_limit: 1_000_000,
            default_pbft_gas_limit: 1_000_000,
            cornus_activation_period: u64::MAX,
            cornus_dag_gas_limit: 1_000_000,
            cornus_pbft_gas_limit: 1_000_000,
        },
        dag_transaction: DagTransactionServiceConfig {
            transaction: TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
            dag: DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
            sortition: SortitionConfig {
                params: SortitionParams {
                    vrf: VrfParams {
                        threshold_upper: 0x100,
                    },
                    vdf: VdfParams {
                        difficulty_min: 1,
                        difficulty_max: 10,
                        difficulty_stale: 5,
                        lambda_bound: 100,
                    },
                },
                changes_count_for_average: 8,
                dag_efficiency_targets: (5_000, 10_000),
                changing_interval: 10,
                computation_interval: 5,
            },
        },
        pbft: PbftServiceConfig {
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            cacti_block: 1,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
            report_malicious_behaviour: true,
            magnolia_activation_period: 0,
            ficus_activation_period: 0,
            pillar_blocks_interval: 10,
            sync_level_size: 10,
            is_light_node: false,
            light_node_history: 0,
            committee_size: 1,
            number_of_proposers: 1,
            dag_blocks_size: 50,
            ghost_path_move_back: 0,
            node_version: (0, 0, 0, 0),
            node_version_suffix: b"T".to_vec(),
            default_pbft_gas_limit: 1_000_000,
            cornus_activation_period: u64::MAX,
            cornus_pbft_gas_limit: 1_000_000,
            process_synced_policy: crate::pbft_service::PbftProcessSyncedPolicy {
                chain_id: 2999,
                lambda_min_ms: 100,
                lambda_change_interval: 10,
                lambda_change_ms: 10,
                consensus_delay_ms: 400,
                dpos_blocks_per_year: 500,
                recently_finalized_factor: 3,
            },
        },
    }
}

fn resolve_dag_block_transaction_payloads(
    block_hashes: &[H256],
    supplied: &std::collections::HashMap<H256, Vec<u8>>,
    native: &std::collections::HashMap<H256, Vec<u8>>,
) -> Vec<DagAddBlockTransactionPayload> {
    block_hashes
        .iter()
        .filter_map(|hash| {
            supplied
                .get(hash)
                .or_else(|| native.get(hash))
                .map(|transaction_rlp| DagAddBlockTransactionPayload {
                    hash: *hash,
                    transaction_rlp: transaction_rlp.clone(),
                })
        })
        .collect()
}

fn select_dag_sync_transaction_payloads(
    block_hashes: &[H256],
    packet_payloads: &std::collections::HashMap<H256, Vec<u8>>,
) -> Vec<Vec<u8>> {
    block_hashes
        .iter()
        .filter_map(|hash| packet_payloads.get(hash).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, StatusField};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn dag_gas_policy_activates_cornus_at_proposal_period_and_preserves_cap() {
        let policy = DagProposerConfig {
            total_transaction_shards: 1,
            proposal_dag_gas_limit: 700,
            default_dag_gas_limit: 1_000,
            default_pbft_gas_limit: 10_000,
            cornus_activation_period: 42,
            cornus_dag_gas_limit: 600,
            cornus_pbft_gas_limit: 6_000,
        };

        assert_eq!(policy.gas_limits(41), (1_000, 10_000));
        assert_eq!(policy.proposal_weight_limit(41), 700);
        assert_eq!(policy.gas_limits(42), (600, 6_000));
        assert_eq!(policy.proposal_weight_limit(42), 600);
        assert_eq!(policy.gas_limits(43), (600, 6_000));
    }

    struct UnusedExecution;

    impl ConsensusExecutionPort for UnusedExecution {
        fn execute_finalization(
            &self,
            _request: &crate::consensus_application_runtime::EvmFinalizationRequest,
        ) -> Result<crate::consensus_application_runtime::EvmFinalizationReport> {
            bail!("unused execution finalization")
        }

        fn load_pillar_anchor_state(
            &self,
            _request: &crate::consensus_application_runtime::PillarAnchorStateRequest,
        ) -> Result<crate::consensus_application_runtime::PillarAnchorStateReport> {
            bail!("unused pillar lookup")
        }

        fn load_final_chain_account_facts(
            &self,
            _request: &crate::consensus_application_runtime::FinalChainAccountFactsRequest,
        ) -> Result<crate::consensus_application_runtime::FinalChainAccountFactsReport> {
            bail!("unused account lookup")
        }
    }

    fn bootstrap(path: std::path::PathBuf) -> ConsensusApplicationBootstrap {
        ConsensusApplicationBootstrap {
            storage_path: path,
            schema_major: 7,
            schema_minor: 3,
            storage_genesis_hash: H256::repeat_byte(1),
            final_chain: ConsensusFinalChainConfig {
                block_gas_limit: 1_000_000.into(),
                genesis_timestamp: 42,
                genesis_accounts: Vec::new(),
                genesis_validators: Vec::new(),
                genesis_dpos: GenesisDposConfig::default(),
                rewards: FinalChainRewardsConfig::default(),
            },
            consensus: deterministic_test_config(),
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn signed_public_transaction() -> (Vec<u8>, [u8; 20]) {
        use tiny_keccak::{Hasher, Keccak};
        let key = SigningKey::from_slice(&[0x33; 32]).expect("fixed signing key");
        let nonce = U256::from(1);
        let gas_price = U256::from(2);
        let gas = 21_000_u64;
        let receiver = H160::repeat_byte(0x44);
        let value = U256::from(3);
        let chain_id = 2_999_u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&nonce);
        unsigned.append(&gas_price);
        unsigned.append(&gas);
        unsigned.append(&receiver);
        unsigned.append(&value);
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let mut digest = [0; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("sign transaction");
        let signature = signature.to_bytes();
        let mut signed = RlpStream::new_list(9);
        signed.append(&nonce);
        signed.append(&gas_price);
        signed.append(&gas);
        signed.append(&receiver);
        signed.append(&value);
        signed.append(&Vec::<u8>::new());
        signed.append(&U256::from(
            chain_id * 2 + 35 + u64::from(recovery_id.to_byte()),
        ));
        signed.append(&U256::from_big_endian(&signature[..32]));
        signed.append(&U256::from_big_endian(&signature[32..]));
        let bytes = signed.out().to_vec();
        let sender = LegacyTransactionEnvelope::decode(&bytes)
            .expect("decode signed transaction")
            .sender
            .expect("recover sender");
        (bytes, sender.0)
    }

    fn public_submission_request(transaction_rlp: Vec<u8>) -> PublicTransactionSubmissionRequest {
        PublicTransactionSubmissionRequest {
            transaction_rlp,
            expected_chain_id: 2_999,
            maximum_gas_limit: 1_000_000,
            minimum_gas_price: U256::one(),
            last_block_number: 0,
            cornus_active: false,
        }
    }

    fn public_account_facts(sender: [u8; 20]) -> PublicTransactionFinalChainFacts {
        PublicTransactionFinalChainFacts {
            sender,
            account_found: true,
            account_nonce: U256::zero(),
            account_balance: U256::MAX,
            finalized_period: None,
        }
    }

    #[test]
    fn native_bootstrap_initializes_fresh_storage_and_owns_all_siblings() {
        let path = temp_path("consensus_application_native_fresh");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");

        assert_eq!(
            root.storage
                .metadata()
                .status_field(StatusField::DbMajorVersion as u8)
                .unwrap(),
            7
        );
        assert_eq!(
            root.storage
                .metadata()
                .status_field(StatusField::DbMinorVersion as u8)
                .unwrap(),
            3
        );
        assert_eq!(
            root.storage.metadata().genesis_hash().unwrap(),
            Some(vec![1; 32])
        );
        assert_eq!(root.dag_transaction.transaction_count().unwrap(), 0);
        let final_chain = root.final_chain_arc_for_bridge();
        assert!(Arc::ptr_eq(&final_chain, &root.final_chain));
        assert!(std::ptr::eq(
            root.final_chain_for_bridge(),
            final_chain.as_ref()
        ));

        drop(root);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn root_bound_query_observes_live_pbft_progress() {
        let path = temp_path("consensus_application_live_pbft_query");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");
        let query = root.consensus_query_api_for_bridge();

        assert_eq!(query.pbft_progress().unwrap().finalized_period, 0);
        root.pbft_chain_update(H256::repeat_byte(2), H256::repeat_byte(3))
            .expect("PBFT head advances");
        let progress = query.pbft_progress().expect("live progress is readable");
        assert_eq!(progress.finalized_period, 1);
        assert_eq!(progress.non_empty_finalized_periods, 1);

        drop(query);
        drop(root);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn public_submission_decodes_admits_observes_and_deduplicates() {
        let path = temp_path("consensus_application_public_submission");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");
        let (rlp, sender) = signed_public_transaction();

        let first = root
            .submit_public_transaction(
                public_submission_request(rlp.clone()),
                public_account_facts(sender),
            )
            .expect("first submission");
        assert!(first.accepted);
        assert!(first.transaction_observed);
        assert_eq!(
            root.consensus_query_api_for_bridge()
                .transaction_pool_status()
                .unwrap()
                .queue_size,
            1
        );

        let duplicate = root
            .submit_public_transaction(public_submission_request(rlp), public_account_facts(sender))
            .expect("duplicate report");
        assert!(!duplicate.accepted);
        assert!(!duplicate.transaction_observed);
        assert!(duplicate.message.contains("already in transactions pool"));

        drop(root);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn public_submission_failure_does_not_publish_queue_state() {
        let path = temp_path("consensus_application_public_submission_failure");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");
        let (rlp, sender) = signed_public_transaction();
        let mut mismatched = public_account_facts(sender);
        mismatched.sender[0] ^= 1;

        let error = root
            .submit_public_transaction(public_submission_request(rlp), mismatched)
            .expect_err("mismatched facts reject");
        assert!(
            error
                .to_string()
                .contains("PUBLIC_TRANSACTION_FINAL_CHAIN_SENDER_MISMATCH")
        );
        assert_eq!(
            root.consensus_query_api_for_bridge()
                .transaction_pool_status()
                .unwrap()
                .queue_size,
            0
        );

        drop(root);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn public_submission_restart_does_not_restore_ephemeral_queue() {
        let path = temp_path("consensus_application_public_submission_restart");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");
        let (rlp, sender) = signed_public_transaction();
        root.submit_public_transaction(
            public_submission_request(rlp.clone()),
            public_account_facts(sender),
        )
        .expect("submission");
        drop(root);

        let restarted = bootstrap(path.clone()).bootstrap().expect("root restarts");
        let status = restarted
            .consensus_query_api_for_bridge()
            .transaction_pool_status()
            .expect("pool status");
        assert_eq!(status.queue_size, 0);
        let replay = restarted
            .submit_public_transaction(public_submission_request(rlp), public_account_facts(sender))
            .expect("replay after restart");
        assert!(replay.accepted);
        assert!(replay.transaction_observed);

        drop(restarted);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn native_bootstrap_rejects_same_major_minor_mismatch_without_migrating() {
        let path = temp_path("consensus_application_native_restart");
        bootstrap(path.clone())
            .bootstrap()
            .expect("fresh bootstrap");

        let mut restarted = bootstrap(path.clone());
        restarted.schema_minor = 4;
        let error = match restarted.bootstrap() {
            Ok(_) => panic!("minor mismatch must fail closed"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("CONSENSUS_APPLICATION_SCHEMA_MINOR_MISMATCH")
        );

        let storage = Storage::new(Config::new(path.clone())).expect("storage reopens");
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::DbMinorVersion as u8)
                .unwrap(),
            3
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn native_bootstrap_rejects_exact_genesis_mismatch() {
        let path = temp_path("consensus_application_native_genesis_mismatch");
        bootstrap(path.clone())
            .bootstrap()
            .expect("fresh bootstrap");

        let mut mismatched = bootstrap(path.clone());
        mismatched.storage_genesis_hash = H256::repeat_byte(2);
        let error = match mismatched.bootstrap() {
            Ok(_) => panic!("genesis mismatch must reject bootstrap"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("CONSENSUS_APPLICATION_GENESIS_MISMATCH")
        );

        let storage = Storage::new(Config::new(path.clone())).expect("storage reopens");
        assert_eq!(
            storage.metadata().genesis_hash().unwrap(),
            Some(vec![1; 32])
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn native_bootstrap_rejects_existing_schema_major_mismatch() {
        let path = temp_path("consensus_application_native_schema_mismatch");
        bootstrap(path.clone())
            .bootstrap()
            .expect("fresh bootstrap");

        let mut mismatched = bootstrap(path.clone());
        mismatched.schema_major = 8;
        let error = match mismatched.bootstrap() {
            Ok(_) => panic!("schema major mismatch must reject bootstrap"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("CONSENSUS_APPLICATION_SCHEMA_MAJOR_MISMATCH")
        );

        let storage = Storage::new(Config::new(path.clone())).expect("storage reopens");
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::DbMajorVersion as u8)
                .unwrap(),
            7
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn native_bootstrap_failure_publishes_no_root_or_dag_mapping() {
        let path = temp_path("consensus_application_native_failure");
        let mut invalid = bootstrap(path.clone());
        invalid.consensus.pbft.pillar_blocks_interval = 1;
        let error = match invalid.bootstrap() {
            Ok(_) => panic!("invalid PBFT config must reject bootstrap"),
            Err(error) => error,
        };
        assert!(!error.to_string().is_empty());

        let storage = Storage::new(Config::new(path.clone())).expect("storage reopens");
        assert_eq!(storage.dag().proposal_period_at_level(100).unwrap(), None);
        assert_eq!(
            storage.metadata().genesis_hash().unwrap(),
            Some(vec![1; 32])
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn resolved_dag_transactions_merge_packet_and_native_views_in_block_order() {
        let first = H256::repeat_byte(1);
        let second = H256::repeat_byte(2);
        let supplied = std::collections::HashMap::from([(second, vec![0x22])]);
        let native = std::collections::HashMap::from([(first, vec![0x11])]);
        let resolved = resolve_dag_block_transaction_payloads(&[second, first], &supplied, &native);
        assert_eq!(
            resolved
                .iter()
                .map(|value| (value.hash, value.transaction_rlp.clone()))
                .collect::<Vec<_>>(),
            vec![(second, vec![0x22]), (first, vec![0x11])]
        );
    }

    #[test]
    fn dag_sync_staging_selects_only_each_blocks_referenced_transactions() {
        let first = H256::repeat_byte(1);
        let second = H256::repeat_byte(2);
        let unreferenced = H256::repeat_byte(3);
        let payloads = std::collections::HashMap::from([
            (first, vec![0x11]),
            (second, vec![0x22]),
            (unreferenced, vec![0x33]),
        ]);
        assert_eq!(
            select_dag_sync_transaction_payloads(&[first], &payloads),
            vec![vec![0x11]]
        );
        assert_eq!(
            select_dag_sync_transaction_payloads(&[second], &payloads),
            vec![vec![0x22]]
        );
    }

    #[test]
    fn dag_sync_rejects_invalid_unreferenced_transaction_without_queue_mutation() {
        let path = temp_path("consensus_application_dag_sync_invalid_unreferenced");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");
        let (rlp, _) = signed_public_transaction();
        let mut submission = public_submission_request(rlp);
        submission.expected_chain_id += 1;
        let report = root
            .ingest_dag_sync_packet(
                DagSyncIngressRequest {
                    transactions: vec![TransactionPacketIngressRequest {
                        submission,
                        peer_id: [7; 64],
                        rebroadcast: false,
                    }],
                    blocks: Vec::new(),
                },
                &UnusedExecution,
            )
            .expect("typed rejection");
        assert!(!report.accepted);
        assert!(!report.transactions[0].submission.accepted);
        assert_eq!(root.dag_transaction.transaction_queue_size().unwrap(), 0);
        drop(root);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn dag_sync_does_not_publish_valid_unreferenced_transaction() {
        let path = temp_path("consensus_application_dag_sync_valid_unreferenced");
        let root = bootstrap(path.clone())
            .bootstrap()
            .expect("root bootstraps");
        let (rlp, _) = signed_public_transaction();
        let report = root
            .ingest_dag_sync_packet(
                DagSyncIngressRequest {
                    transactions: vec![TransactionPacketIngressRequest {
                        submission: public_submission_request(rlp),
                        peer_id: [8; 64],
                        rebroadcast: false,
                    }],
                    blocks: Vec::new(),
                },
                &UnusedExecution,
            )
            .expect("validation succeeds");
        assert!(report.accepted);
        assert!(report.transactions[0].submission.accepted);
        assert_eq!(root.dag_transaction.transaction_queue_size().unwrap(), 0);
        drop(root);
        let _ = fs::remove_dir_all(path);
    }
}
