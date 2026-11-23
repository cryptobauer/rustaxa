use super::*;
use crate::consensus_application::{
    ConsensusApplicationBootstrap, consensus_application_test_bootstrap,
};
use ethereum_types::H256;
use num_bigint::BigUint;
use rocksdb::{ColumnFamilyDescriptor, DB, IteratorMode, Options};
use rustaxa_storage::{Column, ConcreteStateMutationBatch};
use rustaxa_types::{
    codec::rlp::concrete_lifecycle::encode_concrete_execution_marker,
    concrete_lifecycle::{FinalChainConcreteExecutionMarker, FinalChainConcreteState},
    concrete_state::{ConcreteRead, execution::ConcreteExecutionRead},
};
use std::path::{Path, PathBuf};

const ACCOUNT: [u8; 20] = [0x44; 20];
const VALIDATOR: [u8; 20] = [0x55; 20];

const LIMITS: ExecutionPredecessorLimits = ExecutionPredecessorLimits {
    maximum_native_source_bytes: 1024 * 1024,
    maximum_retained_bytes: 2 * 1024 * 1024,
};

struct Fixture {
    application: Option<ConsensusApplication>,
    concrete: Option<ConcreteStateLifecycle>,
    root: PathBuf,
    app_path: PathBuf,
    concrete_path: PathBuf,
    genesis: u8,
    delay: u64,
    configured_root: H256,
    conflict: bool,
}

fn bootstrap(path: PathBuf, genesis: u8, delay: u64) -> ConsensusApplicationBootstrap {
    let mut value = consensus_application_test_bootstrap(path, vec![], delay);
    value.final_chain.genesis_accounts = vec![rustaxa_types::GenesisAccount {
        address: ACCOUNT,
        balance: rustaxa_types::FinalChainAccountBalance::from_snapshot_bytes(&[0x12, 0x34])
            .unwrap(),
    }];
    let stake = ethereum_types::U256::from(10_000).to_big_endian().to_vec();
    value.final_chain.genesis_validators = vec![rustaxa_types::GenesisValidator {
        address: VALIDATOR,
        vrf_key: [0; 32],
        total_stake: stake.clone(),
        delegations: vec![(ACCOUNT, stake)],
        metadata: rustaxa_types::GenesisValidatorMetadata {
            owner: ACCOUNT,
            commission: 100,
            ..Default::default()
        },
    }];
    value.storage_genesis_hash = H256::repeat_byte(genesis);
    let rewards = &value.final_chain.rewards;
    let policy = &mut value.consensus.pbft;
    policy.process_synced_policy.chain_id = policy.network_identity.chain_id;
    policy.network_identity.genesis_hash = value.storage_genesis_hash.0;
    policy.magnolia_activation_period = rewards.magnolia_period.as_u64();
    policy.cacti_block = rewards.cacti_period.as_u64();
    policy.cornus_activation_period = rewards.cornus_period.as_u64();
    policy.committee_size = u64::from(rewards.committee_size);
    policy.process_synced_policy.dpos_blocks_per_year = rewards.dpos_blocks_per_year;
    value.consensus.dag_proposer.default_pbft_gas_limit = policy.default_pbft_gas_limit;
    value.consensus.dag_proposer.cornus_activation_period = policy.cornus_activation_period;
    value.consensus.dag_proposer.cornus_pbft_gas_limit = policy.cornus_pbft_gas_limit;
    value
}

impl Fixture {
    fn new(
        name: &str,
        genesis: u8,
        delay: u64,
        paired: bool,
        wrong_root: bool,
        conflict: bool,
    ) -> Self {
        let root = crate::test_paths::new_path(&format!("execution-predecessor-{name}"));
        std::fs::create_dir(&root).unwrap();
        let template = bootstrap(root.join("template"), genesis, delay)
            .bootstrap()
            .unwrap();
        let chain = template.final_chain.concrete_chain_identity().unwrap();
        let (inputs, catalog) = template
            .final_chain
            .concrete_genesis_inputs_for_test()
            .unwrap();
        assert!(!inputs.accounts.is_empty());
        assert!(!inputs.storage.is_empty());
        drop(template);
        let concrete_path = root.join("concrete");
        let concrete =
            ConcreteStateLifecycle::create_fresh_exclusive(&concrete_path, chain, inputs, catalog)
                .unwrap();
        let observed = concrete.observation().unwrap();
        let app_path = root.join("application");
        let mut config = bootstrap(app_path.clone(), genesis, delay);
        config.final_chain.state_api_committed_root = if wrong_root {
            H256::repeat_byte(0xfe)
        } else {
            observed.committed.state_root.into()
        };
        if conflict {
            config.consensus.dag_proposer.default_pbft_gas_limit += 1;
        }
        let configured_root = config.final_chain.state_api_committed_root;
        let application = config.bootstrap().unwrap();
        if paired {
            application
                .final_chain
                .verify_or_initialize_concrete_state_pairing(observed.identity)
                .unwrap();
        }
        Self {
            application: Some(application),
            concrete: Some(concrete),
            root,
            app_path,
            concrete_path,
            genesis,
            delay,
            configured_root,
            conflict,
        }
    }

    fn app(&self) -> &ConsensusApplication {
        self.application.as_ref().unwrap()
    }

    fn owner(&self) -> &ConcreteStateLifecycle {
        self.concrete.as_ref().unwrap()
    }

    fn capture(&self) -> CapturedExecutionPredecessor<'_, '_> {
        captured(
            self.app()
                .capture_execution_predecessor(self.owner(), 1.into(), LIMITS),
        )
    }

    fn contents(&mut self, reopen: bool) -> (LogicalRows, LogicalRows) {
        let reopen_application = reopen && self.application.is_some();
        let chain = self
            .application
            .as_ref()
            .map(|app| app.final_chain.concrete_chain_identity().unwrap());
        let observed = if reopen {
            self.concrete
                .as_ref()
                .map(|owner| owner.observation().unwrap())
        } else {
            None
        };
        self.application.take();
        self.concrete.take();
        let rows = (
            contents(&self.app_path.join("db")),
            contents(&self.concrete_path),
        );
        if let Some(observed) = observed {
            let owner = ConcreteStateLifecycle::open(
                &self.concrete_path,
                chain.unwrap(),
                observed.committed,
            )
            .unwrap();
            assert_eq!(owner.observation().unwrap(), observed);
            self.concrete = Some(owner);
        }
        if reopen_application {
            let mut config = bootstrap(self.app_path.clone(), self.genesis, self.delay);
            config.final_chain.state_api_committed_root = self.configured_root;
            if self.conflict {
                config.consensus.dag_proposer.default_pbft_gas_limit += 1;
            }
            self.application = Some(config.bootstrap().unwrap());
        }
        rows
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.application.take();
        self.concrete.take();
    }
}

fn captured<'a, 'c>(
    result: Result<ExecutionPredecessorCapture<CapturedExecutionPredecessor<'a, 'c>>>,
) -> CapturedExecutionPredecessor<'a, 'c> {
    match result.unwrap() {
        ExecutionPredecessorCapture::Captured(value) => value,
        ExecutionPredecessorCapture::Unavailable(reason) => {
            panic!("capture unavailable: {reason:?}")
        }
    }
}

type LogicalRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;

fn contents(path: &Path) -> LogicalRows {
    let options = Options::default();
    let mut names = DB::list_cf(&options, path).unwrap();
    names.sort();
    let descriptors = names.iter().map(|name| match Column::from_name(name) {
        Ok(column) => column.descriptor(&options),
        Err(_) => ColumnFamilyDescriptor::new(name, options.clone()),
    });
    let db = DB::open_cf_descriptors_read_only(&options, path, descriptors, false).unwrap();
    let snapshot = db.snapshot();
    let mut bytes = 0;
    names
        .into_iter()
        .map(|name| {
            let handle = db.cf_handle(&name).unwrap();
            let rows = snapshot
                .iterator_cf(&handle, IteratorMode::Start)
                .map(|row| {
                    let (key, value) = row.unwrap();
                    bytes += key.len() + value.len();
                    assert!(bytes <= 16 * 1024 * 1024);
                    (key.to_vec(), value.to_vec())
                })
                .collect();
            (name, rows)
        })
        .collect()
}

#[test]
fn captures_actual_bootstrap_sources() {
    let mut fixture = Fixture::new("actual", 1, 0, true, false, false);
    assert!(fixture.app().execution_configuration().is_ok());
    let before = fixture.contents(true);
    let capture = fixture.capture();
    assert_eq!(capture.observation, fixture.owner().observation().unwrap());
    assert!(capture.recheck_sources(LIMITS).unwrap().is_none());
    assert_eq!(
        capture.native.committed_identity(),
        capture.observation.committed
    );
    assert_eq!(
        capture.native.pairing_identity(),
        capture.observation.identity
    );
    let account = capture
        .concrete_probe
        .current_view()
        .unwrap()
        .account(ACCOUNT)
        .unwrap();
    assert_eq!(
        account,
        capture
            .checkpoint
            .account_at(capture.native.delayed_identity(), ACCOUNT)
            .unwrap()
    );
    match account {
        ConcreteRead::Present(record) => {
            assert_eq!(record.account.nonce.as_u64(), Some(0));
            assert_eq!(record.account.balance.value(), &BigUint::from(0x1234_u64));
        }
        _ => panic!("declared genesis account is absent"),
    }
    drop(capture);
    assert_eq!(before, fixture.contents(false));
}

#[test]
fn retains_saturated_delayed_identity() {
    for delay in [0, 8] {
        let fixture = Fixture::new("delay", 1, delay, true, false, false);
        let capture = fixture.capture();
        assert_eq!(
            capture.native.delayed_identity(),
            capture.observation.committed
        );
        assert!(
            capture
                .checkpoint
                .retained_identities()
                .contains(&capture.observation.committed)
        );
    }
}

#[test]
fn rejects_other_database_with_same_root() {
    let fixture = Fixture::new("database", 1, 0, true, false, false);
    let (inputs, catalog) = fixture
        .app()
        .final_chain
        .concrete_genesis_inputs_for_test()
        .unwrap();
    let other = ConcreteStateLifecycle::create_fresh_exclusive(
        fixture.root.join("other"),
        fixture.owner().observation().unwrap().identity.chain_id,
        inputs,
        catalog,
    )
    .unwrap();
    assert_eq!(
        other.observation().unwrap().committed,
        fixture.owner().observation().unwrap().committed
    );
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(&other, 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::PairingMismatch)
    ));
}

#[test]
fn rejects_other_chain_or_genesis() {
    let first = Fixture::new("chain-one", 1, 0, true, false, false);
    let other = Fixture::new("chain-two", 2, 0, true, false, false);
    assert!(matches!(
        first
            .app()
            .capture_execution_predecessor(other.owner(), 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::PairingMismatch)
    ));
}

#[test]
fn rejects_head_and_root_mismatch() {
    let wrong = Fixture::new("wrong-root", 1, 0, true, true, false);
    assert!(matches!(
        wrong
            .app()
            .capture_execution_predecessor(wrong.owner(), 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::StateMismatch)
    ));
    let fixture = Fixture::new("wrong-period", 1, 0, true, false, false);
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(fixture.owner(), 2.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::Native(
            NativePredecessorUnavailable::HeadMismatch { .. }
        ))
    ));
}

#[test]
fn does_not_initialize_missing_pairing() {
    let mut fixture = Fixture::new("missing-pairing", 1, 0, false, false, false);
    let before = fixture.contents(true);
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(fixture.owner(), 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::Native(
            NativePredecessorUnavailable::MissingPairing
        ))
    ));
    assert_eq!(before, fixture.contents(false));
}

#[test]
fn rejects_pending_or_prepared_concrete_owner() {
    let mut fixture = Fixture::new("pending", 1, 0, true, false, false);
    let observed = fixture.owner().observation().unwrap();
    let marker = FinalChainConcreteExecutionMarker {
        identity: observed.identity,
        generation: observed.generation + 1,
        plan_hash: [0x71; 32],
        period: 1,
        prior_state: FinalChainConcreteState {
            period: 0,
            root: observed.committed.state_root,
        },
        transactions_hash: [0x72; 32],
        rewards_hash: [0x73; 32],
    };
    fixture
        .concrete
        .as_mut()
        .unwrap()
        .stage_execution(&encode_concrete_execution_marker(&marker))
        .unwrap();
    let before = fixture.contents(true);
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(fixture.owner(), 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::Concrete(_))
    ));
    fixture
        .concrete
        .as_mut()
        .unwrap()
        .prepare(1.into(), ConcreteStateMutationBatch::default())
        .unwrap();
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(fixture.owner(), 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::Concrete(_))
    ));
    assert_eq!(before, fixture.contents(false));
}

#[test]
fn rejects_configuration_conflict() {
    let fixture = Fixture::new("configuration", 1, 0, true, false, true);
    assert!(fixture.app().execution_configuration().is_err());
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(fixture.owner(), 1.into(), LIMITS)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::Configuration(_))
    ));
}

#[test]
fn recheck_detects_changed_application_source() {
    let fixture = Fixture::new("recheck", 1, 0, true, false, false);
    let capture = fixture.capture();
    let digest = capture.source_digest;
    let storage = &fixture.app().storage;
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(
            &mut batch,
            Column::FinalChainMeta,
            &crate::FinalChain::DB_META_LAST_NUMBER.to_le_bytes(),
            &9_u64.to_le_bytes(),
        )
        .unwrap();
    storage.commit_write_batch_with_sync(batch, false).unwrap();
    assert!(capture.recheck_sources(LIMITS).unwrap().is_some());
    assert_eq!(capture.source_digest, digest);
    assert_eq!(
        capture.native.committed_identity().period,
        FinalChainBlockNumber::GENESIS
    );
}

#[test]
fn limits_and_drop_preserve_complete_stores() {
    let mut fixture = Fixture::new("limits", 1, 0, true, false, false);
    let before = fixture.contents(true);
    let capture = fixture.capture();
    let exact = ExecutionPredecessorLimits {
        maximum_retained_bytes: capture.retained_bytes,
        ..LIMITS
    };
    let digest = capture.source_digest;
    drop(capture);
    for _ in 0..2 {
        let capture = captured(fixture.app().capture_execution_predecessor(
            fixture.owner(),
            1.into(),
            exact,
        ));
        assert_eq!(capture.source_digest, digest);
        assert!(capture.recheck_sources(exact).unwrap().is_none());
    }
    let too_small = ExecutionPredecessorLimits {
        maximum_retained_bytes: exact.maximum_retained_bytes - 1,
        ..exact
    };
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(fixture.owner(), 1.into(), too_small)
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::LocalLimit)
    ));
    assert!(matches!(
        fixture
            .app()
            .capture_execution_predecessor(
                fixture.owner(),
                1.into(),
                ExecutionPredecessorLimits {
                    maximum_native_source_bytes: 0,
                    ..LIMITS
                }
            )
            .unwrap(),
        ExecutionPredecessorCapture::Unavailable(ExecutionPredecessorUnavailable::LocalLimit)
    ));
    assert_eq!(before, fixture.contents(false));
    fixture.application.take();
    fixture.concrete.take();
    assert_eq!(before, fixture.contents(false));
}
