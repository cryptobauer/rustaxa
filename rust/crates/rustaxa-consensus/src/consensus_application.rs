//! Native lifetime and composition root for Rust-enabled consensus.
//!
//! The application restores the PBFT and DAG/transaction/sortition graphs over
//! one storage owner and publishes them only as one coherent root. The bridge
//! may invoke task-family operations through this root, but CXX never receives
//! a separately constructible service or a service accessor.

use crate::dag_transaction_service::{DagTransactionService, DagTransactionServiceConfig};
use crate::final_chain::FinalChain;
use crate::pbft_service::{PbftService, PbftServiceConfig};
use anyhow::{Context, Result, bail};
use ethereum_types::H256;
use rustaxa_storage::{Config, StatusField, Storage};
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
    /// Exact genesis identity persisted for fresh storage and verified on restart.
    pub genesis_hash: H256,
    /// Native FinalChain construction input.
    pub final_chain: ConsensusFinalChainConfig,
    /// Native DAG/transaction/sortition and PBFT restoration input.
    pub consensus: ConsensusApplicationConfig,
}

impl ConsensusApplicationBootstrap {
    /// Opens storage and constructs one fully restored native application root.
    ///
    /// Existing nonzero schema majors must equal `schema_major`; same-major
    /// minor changes are accepted and atomically publish the requested major and
    /// minor pair. Fresh storage receives both values. Genesis is write-once and
    /// is compared byte-for-byte after the conditional write. The configured DAG
    /// genesis must equal the durable application genesis, preventing sibling
    /// services from starting with different chain identities.
    pub fn bootstrap(self) -> Result<ConsensusApplication> {
        if self.schema_major == 0 {
            bail!("CONSENSUS_APPLICATION_SCHEMA_MAJOR_ZERO");
        }
        if self.consensus.dag_transaction.dag.genesis_hash != self.genesis_hash {
            bail!("CONSENSUS_APPLICATION_DAG_GENESIS_MISMATCH");
        }

        let storage = Arc::new(
            Storage::new(Config::new(self.storage_path))
                .context("CONSENSUS_APPLICATION_STORAGE_OPEN_FAILED")?,
        );
        initialize_schema_version(&storage, self.schema_major, self.schema_minor)?;
        initialize_and_verify_genesis(&storage, self.genesis_hash)?;

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
}

impl ConsensusApplication {
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
        Ok(Self {
            storage,
            final_chain,
            pbft: Arc::new(pbft),
            dag_transaction: Arc::new(dag_transaction),
        })
    }

    /// Borrows the application-owned storage handle for thin Rust bridge dispatch.
    ///
    /// This accessor is not exported through CXX. It lets operation-specific
    /// adapters bind to the root's sole storage owner without opening RocksDB a
    /// second time or publishing a separately constructible storage service.
    #[doc(hidden)]
    pub fn storage_for_bridge(&self) -> &Arc<Storage> {
        &self.storage
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

    /// Borrows DAG/transaction ownership for thin Rust bridge task dispatch.
    ///
    /// This Rust-only migration seam is intentionally not exported through
    /// CXX. It must disappear as operation-shaped application tasks absorb the
    /// remaining manager facade calls.
    #[doc(hidden)]
    pub fn dag_transaction_for_bridge(&self) -> &DagTransactionService {
        self.dag_transaction.as_ref()
    }

    /// Clones the private DAG owner for the temporary in-process bridge adapter.
    ///
    /// The clone never crosses CXX and cannot construct a second runtime; it
    /// only lets the single opaque bridge receiver dispatch existing DAG tasks.
    #[doc(hidden)]
    pub fn dag_transaction_arc_for_bridge(&self) -> Arc<DagTransactionService> {
        Arc::clone(&self.dag_transaction)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag_service::DagServiceConfig;
    use crate::gas_pricer::GasPricerConfig;
    use crate::sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams};
    use crate::transaction_service::TransactionServiceConfig;
    use ethereum_types::{H256, U256};
    use rustaxa_storage::{Config, StatusField};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config() -> ConsensusApplicationConfig {
        ConsensusApplicationConfig {
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
                slashing_submitters: Vec::new(),
            },
        }
    }

    fn bootstrap(path: std::path::PathBuf) -> ConsensusApplicationBootstrap {
        ConsensusApplicationBootstrap {
            storage_path: path,
            schema_major: 7,
            schema_minor: 3,
            genesis_hash: H256::repeat_byte(1),
            final_chain: ConsensusFinalChainConfig {
                block_gas_limit: 1_000_000.into(),
                genesis_timestamp: 42,
                genesis_accounts: Vec::new(),
                genesis_validators: Vec::new(),
                genesis_dpos: GenesisDposConfig::default(),
                rewards: FinalChainRewardsConfig::default(),
            },
            consensus: config(),
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
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
        assert_eq!(
            root.dag_transaction_for_bridge()
                .transaction_count()
                .unwrap(),
            0
        );
        assert!(Arc::ptr_eq(root.storage_for_bridge(), &root.storage));
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
    fn native_bootstrap_restarts_and_updates_same_major_minor_version() {
        let path = temp_path("consensus_application_native_restart");
        bootstrap(path.clone())
            .bootstrap()
            .expect("fresh bootstrap");

        let mut restarted = bootstrap(path.clone());
        restarted.schema_minor = 4;
        let root = restarted.bootstrap().expect("restart bootstrap");
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
            4
        );
        assert_eq!(
            root.storage.dag().proposal_period_at_level(100).unwrap(),
            Some(0)
        );

        drop(root);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn native_bootstrap_rejects_exact_genesis_mismatch() {
        let path = temp_path("consensus_application_native_genesis_mismatch");
        bootstrap(path.clone())
            .bootstrap()
            .expect("fresh bootstrap");

        let mut mismatched = bootstrap(path.clone());
        mismatched.genesis_hash = H256::repeat_byte(2);
        mismatched.consensus.dag_transaction.dag.genesis_hash = H256::repeat_byte(2);
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
}
