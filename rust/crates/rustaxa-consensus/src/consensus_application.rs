//! Native lifetime and composition root for Rust-enabled consensus.
//!
//! The application restores the PBFT and DAG/transaction/sortition graphs over
//! one storage owner and publishes them only as one coherent root. The bridge
//! may invoke task-family operations through this root, but CXX never receives
//! a separately constructible service or a service accessor.

use crate::dag_transaction_service::{DagTransactionService, DagTransactionServiceConfig};
use crate::pbft_service::{PbftService, PbftServiceConfig};
use anyhow::Result;
use rustaxa_storage::Storage;
use std::ops::Deref;
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

/// Native Rust-enabled consensus composition and lifetime owner.
///
/// The fields are private and share one `Arc<Storage>`. DAG initialization is
/// prepared without durable mutation, PBFT is then restored, and only then is
/// the initial proposal-period mapping published. Consequently a late sibling
/// error returns no root and leaves no initial DAG proposal-period mapping. Other
/// idempotent restoration defaults may already be durable; atomicity here is the
/// publication of the live root, not transactional rollback of bootstrap rows.
pub struct ConsensusApplication {
    pbft: Arc<PbftService>,
    dag_transaction: Arc<DagTransactionService>,
}

impl ConsensusApplication {
    /// Restores and atomically publishes one complete native application root.
    ///
    /// Transaction, DAG, and sortition restoration precede PBFT restoration to
    /// preserve startup error ordering. The sole DAG initialization write is
    /// deferred until every sibling has restored successfully.
    pub fn restore(storage: Arc<Storage>, config: ConsensusApplicationConfig) -> Result<Self> {
        let (dag_transaction, max_levels_per_period) =
            DagTransactionService::restore_deferred_mapping(
                storage.clone(),
                config.dag_transaction,
            )?;
        let pbft = PbftService::restore(storage, config.pbft)?;
        dag_transaction.complete_restore_mapping(max_levels_per_period)?;
        Ok(Self {
            pbft: Arc::new(pbft),
            dag_transaction: Arc::new(dag_transaction),
        })
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
    use rustaxa_storage::Config;
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

    fn storage(name: &str) -> (std::path::PathBuf, Arc<Storage>) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).expect("storage opens"));
        (path, storage)
    }

    #[test]
    fn restore_publishes_both_clusters_and_restores_again() {
        let (path, storage) = storage("consensus_application_restore");
        let root = ConsensusApplication::restore(storage.clone(), config()).expect("root restores");
        assert_eq!(
            root.dag_transaction_for_bridge()
                .transaction_count()
                .unwrap(),
            0
        );
        drop(root);
        ConsensusApplication::restore(storage.clone(), config()).expect("root restarts");
        assert_eq!(
            storage.dag().proposal_period_at_level(100).unwrap(),
            Some(0)
        );
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn late_pbft_failure_leaves_no_dag_initialization_mapping() {
        let (path, storage) = storage("consensus_application_atomic_failure");
        let mut invalid = config();
        invalid.pbft.pillar_blocks_interval = 1;
        assert!(ConsensusApplication::restore(storage.clone(), invalid).is_err());
        assert_eq!(storage.dag().proposal_period_at_level(100).unwrap(), None);
        drop(storage);
        let _ = fs::remove_dir_all(path);
    }
}
