use super::*;
use rustaxa_evm::profile::CorrectionStage;

#[derive(Default)]
struct Events {
    entered: Vec<u32>,
    completed: Vec<(FinalChainEvmTransactionResult, ConcreteStateIdentity)>,
    phases: Vec<&'static str>,
}

thread_local! {
    static EVENTS: RefCell<Option<Events>> = const { RefCell::new(None) };
}

pub(super) fn transaction_enter(position: u32) {
    EVENTS.with_borrow_mut(|events| {
        if let Some(events) = events {
            events.entered.push(position);
        }
    });
}

pub(super) fn transaction_complete(
    result: &FinalChainEvmTransactionResult,
    state: ConcreteStateIdentity,
) {
    EVENTS.with_borrow_mut(|events| {
        if let Some(events) = events {
            events.completed.push((result.clone(), state));
        }
    });
}

pub(super) fn phase(name: &'static str) {
    EVENTS.with_borrow_mut(|events| {
        if let Some(events) = events {
            events.phases.push(name);
        }
    });
}

struct Observation;
impl Observation {
    fn start() -> Self {
        EVENTS.with_borrow_mut(|events| {
            assert!(events.is_none());
            *events = Some(Events::default());
        });
        Self
    }

    fn take(self) -> Events {
        EVENTS.with_borrow_mut(|events| events.take().unwrap())
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        EVENTS.with_borrow_mut(|events| events.take());
    }
}

type LogicalRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;

/// All writable owners and retained views must be closed before this read.
fn logical_rows(path: &Path, application: bool) -> Result<LogicalRows> {
    let directory = if application {
        path.join("db")
    } else {
        path.to_path_buf()
    };
    let path = directory.as_path();
    let options = rocksdb::Options::default();
    let mut columns = rocksdb::DB::list_cf(&options, path)?;
    columns.sort();
    let descriptors = columns
        .iter()
        .map(|name| {
            if application && name != "default" {
                Ok(rustaxa_storage::Column::from_name(name)?.descriptor(&options))
            } else {
                if !application {
                    ensure!(
                        matches!(
                            name.as_str(),
                            "default" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8"
                        ),
                        "unknown concrete column"
                    );
                }
                Ok(rocksdb::ColumnFamilyDescriptor::new(name, options.clone()))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
    columns
        .into_iter()
        .map(|column| {
            let handle = db.cf_handle(&column).context("logical column")?;
            let values = db
                .iterator_cf(&handle, rocksdb::IteratorMode::Start)
                .map(|row| row.map(|(key, value)| (key.to_vec(), value.to_vec())))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((column, values))
        })
        .collect()
}

fn malformed_proof(sortition: bool) -> Vec<u8> {
    let vote = if sortition {
        let mut vote = RlpStream::new_list(3);
        vote.append(&vec![1; 32])
            .append(&vec![0xf8])
            .append(&vec![0; 65]);
        vote.out().to_vec()
    } else {
        vec![0xf8]
    };
    let word = |value: usize| {
        let mut word = [0; 32];
        word[24..].copy_from_slice(&(value as u64).to_be_bytes());
        word
    };
    let padded = vote.len().div_ceil(32) * 32;
    let mut input = vec![0xfa, 0xc7, 0xc9, 0x4a];
    input.extend_from_slice(&word(64));
    input.extend_from_slice(&word(96 + padded));
    for _ in 0..2 {
        input.extend_from_slice(&word(vote.len()));
        input.extend_from_slice(&vote);
        input.resize(input.len() + padded - vote.len(), 0);
    }
    input
}

fn sign_call(key: &[u8], chain_id: u64, nonce: u64, input: &[u8]) -> Vec<u8> {
    let mut slash = [0; 20];
    slash[19] = 0xee;
    let fields = |out: &mut RlpStream| {
        out.append(&nonce)
            .append(&1_u64)
            .append(&500_000_u64)
            .append(&slash.as_slice())
            .append(&0_u64)
            .append(&input);
    };
    let mut unsigned = RlpStream::new_list(9);
    fields(&mut unsigned);
    unsigned.append(&chain_id).append(&0_u64).append(&0_u64);
    let key = SigningKey::from_slice(key).unwrap();
    let (signature, recovery) = key
        .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
        .unwrap();
    let signature = signature.to_bytes();
    let mut signed = RlpStream::new_list(9);
    fields(&mut signed);
    signed
        .append(&(chain_id * 2 + 35 + u64::from(recovery.to_byte())))
        .append(&U256::from_big_endian(&signature[..32]))
        .append(&U256::from_big_endian(&signature[32..]));
    signed.out().to_vec()
}

fn committed(
    chain: &FinalChain,
    storage: &Storage,
) -> Result<FinalChainExternalEvmCommittedStateDescriptor> {
    let period = chain.last_block_number_typed()?;
    let raw = storage
        .final_chain()
        .block_header_raw(period.as_u64())?
        .context("committed header")?;
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw))?;
    Ok(FinalChainExternalEvmCommittedStateDescriptor {
        period,
        state_root: header.state_root.0,
    })
}

fn reopen(
    root: &Value,
    application_path: &Path,
    concrete_path: &Path,
) -> Result<(std::sync::Arc<Storage>, FinalChain, ConcreteStateLifecycle)> {
    let (storage, chain) = mixed_genesis::open_chain(application_path, root)?;
    let mut chain = chain.with_correction_stage(CorrectionStage::SlashingHardError);
    let observed =
        ConcreteStateLifecycle::inspect_existing(concrete_path, chain.concrete_chain_identity()?)?;
    let genesis = ConcreteStateIdentity {
        period: 0.into(),
        state_root: fixed(&root["genesis"]["root"]),
    };
    let reader =
        ConcreteStateReader::open_historical_read_only(concrete_path, observed.committed, genesis)?;
    chain.hydrate_concrete_genesis_accounts(&reader)?;
    drop(reader);
    let concrete = ConcreteStateLifecycle::open(
        concrete_path,
        chain.concrete_chain_identity()?,
        observed.committed,
    )?;
    Ok((storage, chain, concrete))
}

fn adapter<'a>(
    chain: &'a FinalChain,
    storage: &'a Storage,
    concrete: ConcreteStateLifecycle,
    period: &'a Value,
) -> Adapter<'a> {
    Adapter {
        chain,
        application: storage,
        concrete: RefCell::new(Some(concrete)),
        staged: RefCell::new(None),
        native: RefCell::new(None),
        native_context: RefCell::new(None),
        physical_receipt: None,
        state_api_epoch: StateApiEpoch::new(),
        ordinary_observer: None,
        fixture: period,
        account_node_publication: AccountNodePublication::ObserverHistory,
    }
}

#[test]
fn corrected_application_rejects_complete_attempt_and_publishes_fresh_control() -> Result<()> {
    let fixture = load_fixture()?;
    let path = closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!(
            "corrected-application-publication-{}",
            std::process::id()
        )),
        true,
    );
    let application_path = path.join("application");
    let concrete_path = path.join("state_db");
    run_period(
        &fixture,
        &fixture["periods"][0],
        &application_path,
        &concrete_path,
        true,
        None,
    )?;
    let baseline_application = logical_rows(&application_path, true)?;
    let baseline_concrete = logical_rows(&concrete_path, false)?;
    assert!(
        baseline_application
            .iter()
            .any(|(_, values)| values.is_empty())
    );
    let valid = &fixture["periods"][1];

    for sortition in [false, true] {
        let mut invalid = valid.clone();
        let mut middle = valid["transactions"][2].clone();
        middle["signed_rlp"] = Value::String(hex::encode(sign_call(
            &bytes(&fixture["inputs"]["sender_private_key"]),
            number(&fixture["configuration"]["chain_id"]),
            12,
            &malformed_proof(sortition),
        )));
        middle["name"] = Value::String("malformed_slashing".into());
        invalid["transactions"] =
            serde_json::json!([valid["transactions"][0], middle, valid["transactions"][3]]);
        let hashes = invalid["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tx| {
                let decoded = LegacyTransactionEnvelope::decode(&bytes(&tx["signed_rlp"]))?;
                ensure!(decoded.signature_valid);
                Ok(Value::String(hex::encode(decoded.hash.0)))
            })
            .collect::<Result<Vec<_>>>()?;
        let blocks = invalid["planner_facts"]["dag_blocks"]
            .as_array_mut()
            .unwrap();
        for block in blocks.iter_mut() {
            block["transaction_hashes"] = serde_json::json!([]);
        }
        blocks[0]["transaction_hashes"] = Value::Array(hashes);
        let (storage, chain, concrete) = reopen(&fixture, &application_path, &concrete_path)?;
        let leaf = adapter(&chain, &storage, concrete, &invalid);
        initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &leaf)?;
        let prior = committed(&chain, &storage)?;
        let epoch = leaf.state_api_epoch.current();
        let request = request(&fixture, &invalid)?;
        let hashes = request
            .transactions
            .iter()
            .map(|tx| tx.hash)
            .collect::<Vec<_>>();
        let observation = Observation::start();
        let error = execute_final_chain_application_task(
            &chain,
            request,
            FinalChainProposalPeriodDagLevelUpdate::default(),
            false,
            [0; 20],
            &leaf,
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<rustaxa_evm::driver::ExecutionDriverError>(),
            Some(rustaxa_evm::driver::ExecutionDriverError::Native(
                rustaxa_evm::native::NativeAdapterError::Port(
                    rustaxa_evm::contracts::NativePortError::MalformedNestedSlashingProof(_)
                )
            ))
        ));
        let events = observation.take();
        assert_eq!(events.entered, vec![0, 1]);
        assert_eq!(events.completed.len(), 1);
        let (prefix, state) = &events.completed[0];
        assert_eq!(prefix.status, 1);
        assert!(!prefix.logs.is_empty());
        assert!(!prefix.receipt_rlp.is_empty());
        assert!(prefix.gas_used.as_u64() > 20_000);
        assert_ne!(state.state_root, prior.state_root);
        assert_eq!(events.phases, vec!["discard"]);
        assert!(leaf.staged.borrow().is_none());
        assert!(leaf.native.borrow().is_none());
        assert!(leaf.native_context.borrow().is_none());
        assert_ne!(leaf.state_api_epoch.current(), epoch);
        assert_eq!(committed(&chain, &storage)?, prior);
        assert_eq!(
            chain.last_block_number_typed()?,
            FinalChainBlockNumber::new(1)
        );
        assert!(
            storage
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none()
        );
        let concrete = leaf.concrete.borrow();
        let observed = concrete.as_ref().unwrap().observation()?;
        assert_eq!(observed.committed.period, prior.period);
        assert_eq!(observed.committed.state_root, prior.state_root);
        assert!(observed.pending_marker_rlp.is_empty());
        drop(concrete);
        for hash in hashes {
            assert!(chain.transaction_location(hash)?.is_none());
        }
        drop(leaf);
        drop(chain);
        drop(storage);
        assert_eq!(logical_rows(&application_path, true)?, baseline_application);
        assert_eq!(logical_rows(&concrete_path, false)?, baseline_concrete);
    }

    let (storage, chain, concrete) = reopen(&fixture, &application_path, &concrete_path)?;
    let leaf = adapter(&chain, &storage, concrete, valid);
    initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &leaf)?;
    let request = request(&fixture, valid)?;
    chain.ensure_period_data(2.into(), &period_data(&request))?;
    let observation = Observation::start();
    let report = execute_final_chain_application_task(
        &chain,
        request,
        FinalChainProposalPeriodDagLevelUpdate::default(),
        false,
        [0; 20],
        &leaf,
    )?;
    let events = observation.take();
    assert_eq!(report.period, FinalChainBlockNumber::new(2));
    assert_eq!(
        events.entered.len(),
        valid["transactions"].as_array().unwrap().len()
    );
    assert_eq!(events.completed.len(), events.entered.len());
    assert_eq!(events.phases, vec!["rewards", "commit"]);
    verify_receipts(&storage, valid)?;
    assert_eq!(
        committed(&chain, &storage)?.state_root,
        fixed(&valid["final"]["root"])
    );
    drop(leaf);
    drop(chain);
    drop(storage);
    let published_application = logical_rows(&application_path, true)?;
    let published_concrete = logical_rows(&concrete_path, false)?;
    assert_ne!(published_application, baseline_application);
    assert_ne!(published_concrete, baseline_concrete);
    let (storage, chain, concrete) = reopen(&fixture, &application_path, &concrete_path)?;
    let leaf = adapter(&chain, &storage, concrete, valid);
    initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &leaf)?;
    assert_eq!(
        chain.last_block_number_typed()?,
        FinalChainBlockNumber::new(2)
    );
    verify_receipts(&storage, valid)?;
    drop(leaf);
    drop(chain);
    drop(storage);
    assert_eq!(
        logical_rows(&application_path, true)?,
        published_application
    );
    assert_eq!(logical_rows(&concrete_path, false)?, published_concrete);
    closeout_test_scratch::dispose(path)?;
    Ok(())
}
