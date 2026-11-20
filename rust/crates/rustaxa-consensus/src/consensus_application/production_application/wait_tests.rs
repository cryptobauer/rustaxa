use super::*;
use crate::consensus_application_runtime::RuntimeWaitOutcome;
use crate::{ConsensusEffectId, ConsensusWaitOutcome, ConsensusWaitReport, ConsensusWaitRequest};
use rustaxa_storage::ConcreteStateLifecycle;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    application: BoundConsensusApplication,
    storage: Arc<Storage>,
    owner: Arc<ApplicationOwner>,
    _concrete: Arc<ApplicationConcreteState>,
}
static NEXT: AtomicU64 = AtomicU64::new(1);

fn fixture() -> Result<Fixture> {
    let root = std::env::var_os("RUN_ARTIFACT_DIR")
        .or_else(|| std::env::var_os("RUSTAXA_OWNER_TEST_SCRATCH_ROOT"))
        .context("retained owner test scratch root")?;
    let path = PathBuf::from(root).join(format!(
        "bound-wait-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    ensure!(!path.exists(), "wait fixture path must be new");
    let path = crate::closeout_test_scratch::new_path(path, true);
    std::fs::create_dir_all(&path)?;
    let chain: [u8; 32] = concrete_chain_config_identity(H256::repeat_byte(1))
        .try_into()
        .unwrap();
    let mut preparation =
        super::super::tests::execution_configuration_bootstrap(path.join("native-inputs"));
    preparation.final_chain.state_api_committed_root = H256::zero();
    let preparation = preparation.bootstrap()?;
    let (batch, catalog) = preparation.final_chain.concrete_genesis_inputs_for_test()?;
    drop(preparation);
    let concrete = ConcreteStateLifecycle::create_fresh_exclusive(
        path.join("state_db"),
        chain,
        batch,
        catalog,
    )?;
    let observed = concrete.observation()?;
    let configuration = || {
        let mut config = super::super::tests::execution_configuration_bootstrap(path.clone());
        config.final_chain.state_api_committed_root = H256(observed.committed.state_root);
        config
    };
    let initial = configuration().bootstrap()?;
    let header = initial
        .consensus_query_api_for_bridge()
        .final_chain_block_by_number(0)?;
    let (native, accounts) = initial
        .final_chain
        .genesis_snapshots_for_integration_test()?;
    drop(initial);
    let storage = Arc::new(Storage::open_existing(Config::new(path.clone()))?);
    let empty = RlpStream::new_list(0).out();
    storage.final_chain().write_block_header_with_snapshots(
        0,
        H256(header.hash),
        &header.stored_header_rlp,
        &empty,
        Some(&native),
        Some(&accounts),
    )?;
    let mut pair = RlpStream::new_list(4);
    pair.append(&1_u64)
        .append(&[1_u8; 32].as_slice())
        .append(&observed.identity.database_id.as_slice())
        .append(&observed.identity.chain_id.as_slice());
    storage
        .final_chain()
        .write_concrete_state_pairing(&pair.out())?;
    let owner = Arc::new(storage.claim_application_owner()?);
    let admission = ProductionAdmissionPolicy {
        transaction_max_gas: 1_000_000.into(),
        transaction_min_gas_price: U256::one(),
    };
    let schedule = BoundConsensusApplication::prepare_existing_component_schedule(
        &configuration(),
        &storage,
        &owner,
        &observed,
        &admission,
    )?;
    let concrete = Arc::new(ApplicationConcreteState::new(Arc::clone(&owner), concrete)?);
    let application = BoundConsensusApplication::restore_existing_components(
        configuration(),
        Arc::clone(&storage),
        Arc::clone(&owner),
        schedule,
        Arc::clone(&concrete),
        admission,
    )?;
    Ok(Fixture {
        application,
        storage,
        owner,
        _concrete: concrete,
    })
}

fn write_status(storage: &Storage, key: &[u8], value: &[u8]) -> Result<()> {
    let mut batch = storage.create_write_batch();
    storage.batch_put_raw(&mut batch, Column::Status, key, value)?;
    storage.commit_write_batch_with_sync(batch, true)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum WaitBehavior {
    SameThread,
    OtherThread,
    Error,
    Panic,
    WrongEffect,
}
struct WaitProcess<'a> {
    fixture: &'a Fixture,
    behavior: WaitBehavior,
}
impl ConsensusProcessPort for WaitProcess<'_> {
    fn now_millis(&self) -> u64 {
        1
    }
    fn unix_time_seconds(&self) -> u64 {
        1
    }
    fn stop_requested(&self, _: u64) -> bool {
        false
    }
    fn wait(&self, request: &ConsensusWaitRequest) -> Result<ConsensusWaitReport> {
        assert!(self.fixture.owner.begin_mutation().is_err());
        assert!(write_status(&self.fixture.storage, b"same-thread-wait", b"blocked").is_err());
        assert!(self.fixture.owner.try_wait_callback().is_err());
        {
            let read = self.fixture.owner.try_read(Duration::from_secs(1))?;
            assert!(read.is_current());
        }
        let mut effect_id = request.effect_id;
        match self.behavior {
            WaitBehavior::SameThread => {}
            WaitBehavior::OtherThread => {
                let owner = Arc::clone(&self.fixture.owner);
                let storage = Arc::clone(&self.fixture.storage);
                std::thread::spawn(move || -> Result<()> {
                    let _operation = owner.begin_mutation()?;
                    write_status(&storage, b"independent-wait", b"completed")?;
                    Ok(())
                })
                .join()
                .expect("independent writer")?;
            }
            WaitBehavior::Error => bail!("declared callback error"),
            WaitBehavior::Panic => panic!("declared callback unwind"),
            WaitBehavior::WrongEffect => {
                effect_id = ConsensusEffectId {
                    generation: effect_id.generation,
                    sequence: effect_id.sequence + 1,
                }
            }
        }
        Ok(ConsensusWaitReport {
            effect_id,
            outcome: ConsensusWaitOutcome::Elapsed,
        })
    }
}

#[test]
fn bound_wait_rejects_ingress_between_preparation_and_callback() -> Result<()> {
    struct NoWait;
    impl ConsensusProcessPort for NoWait {
        fn now_millis(&self) -> u64 {
            1
        }
        fn unix_time_seconds(&self) -> u64 {
            1
        }
        fn stop_requested(&self, _: u64) -> bool {
            false
        }
        fn wait(&self, _: &ConsensusWaitRequest) -> Result<ConsensusWaitReport> {
            bail!("stale preparation must not call process.wait")
        }
    }
    let fixture = fixture()?;
    let result = fixture
        .application
        .core
        .runtime
        .exercise_bound_wait_with_gap(&NoWait, || {
            fixture
                .application
                .network_build_status_packet(NetworkStatusPacketBuildRequest {
                    initial: true,
                    local_pbft_chain_size: 0,
                    local_pbft_round: 1,
                    local_dag_level: 0,
                })?;
            Ok(())
        })?;
    assert_eq!(result, RuntimeWaitOutcome::StaleWork);
    Ok(())
}

#[test]
fn bound_wait_allows_independent_ingress_but_rejects_its_stale_report() -> Result<()> {
    let fixture = fixture()?;
    let process = WaitProcess {
        fixture: &fixture,
        behavior: WaitBehavior::OtherThread,
    };
    let result = fixture
        .application
        .core
        .runtime
        .exercise_bound_wait(&process);
    assert_eq!(result?, RuntimeWaitOutcome::StaleWork);
    assert_eq!(
        fixture
            .storage
            .get_raw(Column::Status, b"independent-wait")?
            .as_deref(),
        Some(b"completed".as_slice())
    );
    assert!(
        fixture
            .storage
            .get_raw(Column::Status, b"same-thread-wait")?
            .is_none()
    );
    Ok(())
}

#[test]
fn bound_wait_refuses_mutation_and_releases_fence_on_every_return() -> Result<()> {
    let fixture = fixture()?;
    for behavior in [
        WaitBehavior::SameThread,
        WaitBehavior::Error,
        WaitBehavior::WrongEffect,
    ] {
        let process = WaitProcess {
            fixture: &fixture,
            behavior,
        };
        let result = fixture
            .application
            .core
            .runtime
            .exercise_bound_wait(&process);
        match behavior {
            WaitBehavior::SameThread => assert!(matches!(result?, RuntimeWaitOutcome::Elapsed(_))),
            WaitBehavior::Error => assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("declared callback error")
            ),
            WaitBehavior::WrongEffect => assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("STALE_EFFECT_REPORT")
            ),
            _ => unreachable!(),
        }
        drop(fixture.owner.begin_mutation()?);
    }
    let process = WaitProcess {
        fixture: &fixture,
        behavior: WaitBehavior::Panic,
    };
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture
            .application
            .core
            .runtime
            .exercise_bound_wait(&process)))
        .is_err()
    );
    drop(fixture.owner.begin_mutation()?);
    Ok(())
}
