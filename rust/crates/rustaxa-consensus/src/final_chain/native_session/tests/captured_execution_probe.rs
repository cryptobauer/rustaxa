use super::*;
use crate::final_chain::native_predecessor::{
    NativePredecessorCapture, NativePredecessorLimits, NativePredecessorUnavailable,
    PreparedNativePredecessor,
};
use rocksdb::{ColumnFamilyDescriptor, DB, IteratorMode, Options};
use rustaxa_storage::{
    Column, ConcreteAccountMutation, ConcreteCheckpointReaders, ConcreteCommitApproval,
    ConcreteStateLifecycle, ConcreteStateMutationBatch, ConcreteStorageMutation,
};
use rustaxa_types::StoredFinalChainBlockHeader;
use rustaxa_types::codec::rlp::concrete_lifecycle::{
    concrete_storage_slot_catalog_hash, encode_concrete_execution_marker,
    encode_concrete_state_provenance,
};
use rustaxa_types::codec::rlp::final_chain::{StoredBlockHeaderRlp, StoredBlockHeaderRlpOwned};
use rustaxa_types::concrete_lifecycle::{
    ConcreteStorageSlot, FinalChainConcreteExecutionMarker, FinalChainConcreteState,
    FinalChainConcreteStateProvenance,
};
use rustaxa_types::concrete_state::{
    ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteStateIdentity,
};
use rustaxa_types::evm_correction::CorrectionStage;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

const EPOCH: u64 = 91;
const REQUEST: [u8; 32] = [0x39; 32];

struct Fixture {
    chain: Option<FinalChain>,
    reader: Option<Arc<ConcreteCheckpointReaders>>,
    concrete: Option<ConcreteStateLifecycle>,
    app: Option<Arc<Storage>>,
    config: Config,
    concrete_path: PathBuf,
    chain_id: [u8; 32],
    delayed: ConcreteStateIdentity,
    native_snapshot: DposSnapshot,
}

impl Fixture {
    fn raw(&self) -> RawState {
        RawState::from_snapshot(&self.native_snapshot, BTreeMap::new())
    }

    fn chain(&self) -> &FinalChain {
        self.chain.as_ref().unwrap()
    }

    fn reader(&self) -> Arc<ConcreteCheckpointReaders> {
        Arc::clone(self.reader.as_ref().unwrap())
    }

    fn capture(&self) -> PreparedNativePredecessor<'_> {
        match self
            .chain()
            .capture_native_predecessor(
                self.chain()
                    .last_block_number_typed()
                    .unwrap()
                    .checked_next()
                    .unwrap(),
                NativePredecessorLimits {
                    max_retained_source_bytes: 1024 * 1024,
                },
            )
            .unwrap()
        {
            NativePredecessorCapture::Ready(value) => value,
            NativePredecessorCapture::Unavailable(reason) => {
                panic!("capture unavailable: {reason:?}")
            }
        }
    }

    fn probe(&self) -> FinalChainNativeExecutionProbe<'_> {
        self.capture()
            .into_execution_probe(REQUEST, EPOCH, 1, self.reader())
            .unwrap()
    }

    fn request(&self, sequence: u64) -> FinalChainNativeRequest {
        simulation_request(
            self.chain()
                .last_block_number_typed()
                .unwrap()
                .checked_next()
                .unwrap(),
            sequence,
            address_word_input(DPOS_GET_VALIDATOR_ELIGIBLE_VOTES_SELECTOR, VALIDATOR),
            DPOS_DEFAULT_METHOD_GAS,
        )
    }

    fn malformed(&self, gas: u64) -> FinalChainNativeRequest {
        let mut input = rlp::RlpStream::new_list(3);
        input
            .append(&vec![1; 32])
            .append(&vec![0xf8])
            .append(&vec![0; 65]);
        let vote = input.out().to_vec();
        let mut request = self.request(0);
        request.contract = SLASHING_CONTRACT_ADDRESS;
        request.state_address = SLASHING_CONTRACT_ADDRESS;
        request.input = crate::slashing::commit_double_voting_proof_call_data(&vote, &vote);
        request.supplied_gas = gas.into();
        request
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.reader.take();
        self.chain.take();
        self.concrete.take();
        self.app.take();
        crate::closeout_test_scratch::preserve(self.config.db_path.parent().unwrap());
        crate::closeout_test_scratch::preserve(&self.concrete_path);
    }
}

fn request(
    sequence: u64,
    caller: [u8; 20],
    commission: u16,
    supplied_gas: u64,
) -> FinalChainNativeRequest {
    let mut request = super::request(sequence, caller, commission, supplied_gas);
    request.depth = 0;
    request
}

fn marker(owner: &ConcreteStateLifecycle) -> FinalChainConcreteExecutionMarker {
    let observed = owner.observation().unwrap();
    FinalChainConcreteExecutionMarker {
        identity: observed.identity,
        generation: observed.generation + 1,
        plan_hash: [0x61; 32],
        period: observed.committed.period.as_u64() + 1,
        prior_state: FinalChainConcreteState {
            period: observed.committed.period.as_u64(),
            root: observed.committed.state_root,
        },
        transactions_hash: [0x62; 32],
        rewards_hash: [0x63; 32],
    }
}

fn commit_empty(
    mut owner: ConcreteStateLifecycle,
    path: &Path,
    chain_id: [u8; 32],
) -> ConcreteStateLifecycle {
    let marker = marker(&owner);
    let encoded = encode_concrete_execution_marker(&marker);
    owner.stage_execution(&encoded).unwrap();
    let prepared = owner
        .prepare(marker.period.into(), ConcreteStateMutationBatch::default())
        .unwrap();
    let observed = owner.observation().unwrap();
    let catalog = observed.catalog_rlp;
    let catalog_hash = concrete_storage_slot_catalog_hash(
        rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog(&catalog)
            .unwrap()
            .into_iter(),
    );
    let projection_hash = [0x64; 32];
    let provenance = FinalChainConcreteStateProvenance {
        identity: marker.identity,
        generation: marker.generation,
        plan_hash: marker.plan_hash,
        committed_state: FinalChainConcreteState {
            period: marker.period,
            root: marker.prior_state.root,
        },
        transactions_hash: marker.transactions_hash,
        rewards_hash: marker.rewards_hash,
        projection_hash,
        catalog_hash,
    };
    let committed = owner
        .commit_approved(
            prepared,
            ConcreteCommitApproval {
                marker_rlp: encoded,
                provenance_rlp: encode_concrete_state_provenance(&provenance),
                catalog_rlp: catalog,
                projection_hash,
                catalog_hash,
            },
        )
        .unwrap();
    ConcreteStateLifecycle::open(path, chain_id, committed.committed).unwrap()
}

fn with_fixture(delay: u64, parent: u64, boundary: Option<u64>, test: impl FnOnce(&mut Fixture)) {
    let rewards = FinalChainRewardsConfig {
        magnolia_period: 0.into(),
        cornus_period: 0.into(),
        ..Default::default()
    };
    with_fixture_config(delay, parent, boundary, rewards, test);
}

fn with_fixture_config(
    delay: u64,
    parent: u64,
    boundary: Option<u64>,
    rewards: FinalChainRewardsConfig,
    test: impl FnOnce(&mut Fixture),
) {
    let dpos = GenesisDposConfig {
        delegation_delay: delay,
        minimum_deposit: U256::from(1).into(),
        eligibility_balance_threshold: U256::from(1000).into(),
        vote_eligibility_balance_step: U256::from(1000).into(),
        validator_maximum_stake: U256::from(30_000).into(),
        ..Default::default()
    };
    with_arc_chain_dpos_config(
        "captured-probe-template",
        rewards.clone(),
        FinalChainBlockNumber::MAX,
        dpos.clone(),
        |template| {
            template
                .storage
                .metadata()
                .set_genesis_hash_if_empty(&[7; 32])
                .unwrap();
            let chain_id = template.concrete_chain_identity().unwrap();
            let initial = template.dpos_snapshot_at_finalized_block(0.into()).unwrap();
            let account_map = template.account_snapshot_map_at_block(0.into()).unwrap();
            let addresses = account_map.keys().copied().collect::<BTreeSet<_>>();
            let accounts = account_map
                .into_iter()
                .map(|(address, account)| {
                    let mut raw = rlp::RlpStream::new_list(5);
                    raw.append(&account.nonce.to_bytes())
                        .append(account.balance.as_u256());
                    raw.append_empty_data().append_empty_data().append(&0_u64);
                    ConcreteAccountMutation::Upsert {
                        address,
                        record: ConcreteAccountRecord {
                            account: ConcreteAccount {
                                nonce: account.nonce,
                                balance: ConcreteAccountBalance::new(BigUint::from_bytes_be(
                                    &account.balance.as_u256().to_big_endian(),
                                )),
                                storage_root: None,
                                code_hash: None,
                                code_size: 0,
                            },
                            physical_rlp: raw.out().to_vec(),
                        },
                    }
                })
                .collect();
            let storage = canonical_concrete_precompile_storage(
                &initial,
                rewards.magnolia_period.is_genesis(),
            )
            .unwrap()
            .into_iter()
            .filter_map(|((address, key), values)| {
                let value = values.into_iter().next().unwrap();
                (addresses.contains(&address) && !value.is_empty()).then_some(
                    ConcreteStorageMutation {
                        address,
                        key: ConcreteStorageKey(key),
                        value: Some(value),
                    },
                )
            })
            .collect::<Vec<_>>();
            let mut catalog = storage
                .iter()
                .map(|row| ConcreteStorageSlot {
                    address: row.address,
                    key: row.key.0,
                })
                .collect::<Vec<_>>();
            catalog.sort_unstable();
            let app_path = temp_db_path("captured-probe-app");
            let concrete_path =
                crate::closeout_test_scratch::new_path(app_path.with_extension("concrete"), false);
            let mut concrete = ConcreteStateLifecycle::create_fresh_exclusive(
                &concrete_path,
                chain_id,
                ConcreteStateMutationBatch {
                    accounts,
                    storage,
                    ..Default::default()
                },
                catalog,
            )
            .unwrap();
            crate::closeout_test_scratch::created(&concrete_path);
            let genesis = concrete.prior_reader().identity();
            let mut config = Config::new(app_path);
            config.db_write_buffer_size = 2 * 1024 * 1024;
            config.max_total_wal_size = 32 * 1024 * 1024;
            config.max_open_files = 32;
            let app = Arc::new(Storage::new(config.clone()).unwrap());
            app.metadata().set_genesis_hash_if_empty(&[7; 32]).unwrap();
            let validators = vec![GenesisValidator {
                address: VALIDATOR,
                vrf_key: [0; 32],
                total_stake: U256::from(10_000).to_big_endian().to_vec(),
                delegations: vec![(VALIDATOR, U256::from(10_000).to_big_endian().to_vec())],
                metadata: GenesisValidatorMetadata {
                    owner: OWNER,
                    commission: 100,
                    ..Default::default()
                },
            }];
            let open = || {
                FinalChain::new_with_genesis_state_root(
                    Arc::clone(&app),
                    1_000_000.into(),
                    0,
                    H256(genesis.state_root),
                    true,
                    vec![],
                    validators.clone(),
                    dpos.clone(),
                    rewards.clone(),
                )
                .unwrap()
                .with_correction_activation(boundary.map(Into::into))
            };
            let first = open();
            let mut state = first.dpos_snapshot_at_finalized_block(0.into()).unwrap();
            let mut identities = vec![genesis];
            for period in 1..=parent {
                concrete = commit_empty(concrete, &concrete_path, chain_id);
                let identity = concrete.prior_reader().identity();
                assert_eq!(identity.period.as_u64(), period);
                identities.push(identity);
                first
                    .advance_reward_reference_graph_block(&mut state, period.into())
                    .unwrap();
                let mut header = stored_header(&first, 0);
                header.state_root = H256(identity.state_root);
                put(
                    &app,
                    Column::FinalChainBlkByNumber,
                    period.to_le_bytes().to_vec(),
                    StoredBlockHeaderRlpOwned::from(&header).into_vec(),
                );
                put(
                    &app,
                    Column::FinalChainMeta,
                    native_key(period),
                    encode_dpos_snapshot_rlp(&state).unwrap(),
                );
            }
            put(
                &app,
                Column::FinalChainMeta,
                FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                parent.to_le_bytes().to_vec(),
            );
            let mut pairing = rlp::RlpStream::new_list(4);
            pairing
                .append(&1_u32)
                .append(&[7; 32].as_slice())
                .append(
                    &concrete
                        .observation()
                        .unwrap()
                        .identity
                        .database_id
                        .as_slice(),
                )
                .append(&chain_id.as_slice());
            put(
                &app,
                Column::FinalChainMeta,
                b"rustaxa:concrete_state_pairing:v1".to_vec(),
                pairing.out().to_vec(),
            );
            drop(first);
            let chain = open();
            assert_eq!(chain.concrete_chain_identity().unwrap(), chain_id);
            chain
                .authorize_reward_scheduler_joint_startup(EPOCH, parent.into())
                .unwrap();
            chain.complete_reward_scheduler_recovery(EPOCH).unwrap();
            let delayed = identities[parent.saturating_sub(delay) as usize];
            let reader = Arc::new(concrete.open_retained_checkpoint(delayed).unwrap());
            let mut fixture = Fixture {
                chain: Some(chain),
                reader: Some(reader),
                concrete: Some(concrete),
                app: Some(app),
                config,
                concrete_path,
                chain_id,
                delayed,
                native_snapshot: state,
            };
            test(&mut fixture);
        },
    );
}

fn native_key(period: u64) -> Vec<u8> {
    [b"rustaxa:dpos_snapshot:".as_slice(), &period.to_le_bytes()].concat()
}

fn stored_header(chain: &FinalChain, period: u64) -> StoredFinalChainBlockHeader {
    let raw = chain
        .storage
        .final_chain()
        .block_header_raw(period)
        .unwrap()
        .unwrap();
    StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw)).unwrap()
}

fn put(app: &Storage, column: Column, key: Vec<u8>, value: Vec<u8>) {
    let mut batch = app.create_write_batch();
    app.batch_put_raw(&mut batch, column, &key, &value).unwrap();
    app.commit_write_batch_with_sync(batch, false).unwrap();
}

fn run(
    probe: &mut FinalChainNativeExecutionProbe<'_>,
    request: &FinalChainNativeRequest,
    raw: &RawState,
) -> FinalChainNativeInvocationResult {
    let quote = probe.prepare(request, raw).unwrap();
    probe.invoke(request, quote, raw).unwrap()
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
fn captured_probe_matches_existing_pending() {
    for delay in [0, 2] {
        with_fixture(delay, 2, None, |fixture| {
            let chain = fixture.chain();
            let mut captured = fixture.probe();
            let mut old = chain
                .begin_native_execution_probe(
                    REQUEST,
                    EPOCH,
                    1,
                    3.into(),
                    2.into(),
                    CorrectionStage::Historical,
                    fixture.reader(),
                )
                .unwrap();
            let request = fixture.request(0);
            let a = fixture.raw();
            let b = fixture.raw();
            let left = run(&mut captured, &request, &a);
            let right = run(&mut old, &request, &b);
            assert_eq!(left, right);
            assert_eq!(completed(left).status, FinalChainNativeStatus::Success);
            assert_eq!(
                captured.input_aliases(request.id).unwrap(),
                old.input_aliases(request.id).unwrap()
            );
            assert_eq!(
                captured.input_mutations(request.id).unwrap(),
                old.input_mutations(request.id).unwrap()
            );
        });
    }
}

#[test]
fn captured_probe_ignores_native_cache_changes() {
    for poison in [false, true] {
        with_fixture(2, 2, None, |fixture| {
            let prepared = fixture.capture();
            let cache_before = if poison {
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _guard = fixture.chain().dpos_snapshots.lock().unwrap();
                    panic!("test native cache poison");
                }));
                assert!(caught.is_err());
                assert!(fixture.chain().dpos_snapshots.lock().is_err());
                None
            } else {
                let app = fixture.app.as_ref().unwrap();
                let mut current = decode_dpos_snapshot_rlp(
                    &app.final_chain().dpos_snapshot_raw(2).unwrap().unwrap(),
                )
                .unwrap();
                let mut delayed = decode_dpos_snapshot_rlp(
                    &app.final_chain().dpos_snapshot_raw(0).unwrap().unwrap(),
                )
                .unwrap();
                current
                    .validator_metadata
                    .get_mut(&VALIDATOR)
                    .unwrap()
                    .owner = WRONG_OWNER;
                delayed.vote_counts.insert(VALIDATOR, 999);
                delayed.total_vote_count = 999;
                let mut cache = fixture.chain().dpos_snapshots.lock().unwrap();
                cache.insert(2.into(), current);
                cache.insert(0.into(), delayed);
                Some(cache.clone())
            };
            let mut probe = prepared
                .into_execution_probe(REQUEST, EPOCH, 1, fixture.reader())
                .unwrap();
            let raw = fixture.raw();
            let mut mutation = request(0, OWNER, 200, 100_000);
            mutation.period = 3.into();
            let outcome = completed(run(&mut probe, &mutation, &raw));
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            apply_raw_mutations(&raw, &outcome);
            let query = fixture.request(1);
            assert_eq!(
                BigUint::from_bytes_be(&completed(run(&mut probe, &query, &raw)).output),
                BigUint::from(10_u64)
            );
            if let Some(before) = cache_before {
                assert_eq!(*fixture.chain().dpos_snapshots.lock().unwrap(), before);
                let mut old = fixture
                    .chain()
                    .begin_native_execution_probe(
                        REQUEST,
                        EPOCH,
                        1,
                        3.into(),
                        2.into(),
                        CorrectionStage::Historical,
                        fixture.reader(),
                    )
                    .unwrap();
                let old_raw = fixture.raw();
                let result = old
                    .prepare(&mutation, &old_raw)
                    .and_then(|quote| old.invoke(&mutation, quote, &old_raw));
                assert!(!matches!(
                    result,
                    Ok(FinalChainNativeInvocationResult::Completed(
                        FinalChainNativeOutcome {
                            status: FinalChainNativeStatus::Success,
                            ..
                        }
                    ))
                ));
            }
        });
    }
}

#[test]
fn captured_probe_uses_captured_delayed_header() {
    with_fixture(2, 2, None, |fixture| {
        let prepared = fixture.capture();
        let mut header = stored_header(fixture.chain(), 0);
        header.state_root = H256::repeat_byte(0xe1);
        put(
            fixture.app.as_ref().unwrap(),
            Column::FinalChainBlkByNumber,
            0_u64.to_le_bytes().to_vec(),
            StoredBlockHeaderRlpOwned::from(&header).into_vec(),
        );
        let mut old = prepared
            .into_execution_probe(REQUEST, EPOCH, 1, fixture.reader())
            .unwrap();
        assert_eq!(
            completed(run(&mut old, &fixture.request(0), &fixture.raw())).status,
            FinalChainNativeStatus::Success
        );
        assert!(
            matches!(fixture.capture().into_execution_probe(REQUEST, EPOCH, 1, fixture.reader()), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("FINAL_CHAIN_DELAYED_IDENTITY_MISSING"))
        );
    });
}

#[test]
fn captured_probe_starts_fresh_after_discard() {
    for abort in [false, true] {
        with_fixture(0, 1, None, |fixture| {
            let mut first = fixture.probe();
            let state = fixture.raw();
            let mut request = request(0, OWNER, 200, 100_000);
            request.period = 2.into();
            let result = completed(run(&mut first, &request, &state));
            assert_eq!(result.status, FinalChainNativeStatus::Success);
            assert!(!result.raw_mutations.is_empty());
            if abort {
                let mut invalid = request.clone();
                invalid.id.sequence = 99;
                assert!(matches!(
                    first.prepare(&invalid, &state),
                    Err(FinalChainNativeSessionError::OutOfSequence { .. })
                ));
                assert!(matches!(
                    first.validate_context(),
                    Err(FinalChainNativeSessionError::Aborted)
                ));
            }
            drop(first);
            let mut fresh = fixture.probe();
            assert_eq!(completed(run(&mut fresh, &request, &fixture.raw())), result);
        });
    }
}

#[test]
fn captured_probe_preserves_slashing_error_controls() {
    for boundary in [None, Some(2)] {
        for gas in [0, 19_999, 20_000] {
            with_fixture(0, 1, boundary, |fixture| {
                let mut probe = fixture.probe();
                let request = fixture.malformed(gas);
                let raw = RawState::erroring();
                let quote = probe.prepare(&request, &raw).unwrap();
                let result = probe.invoke(&request, quote, &raw);
                if gas < 20_000 {
                    assert!(matches!(
                        result,
                        Ok(FinalChainNativeInvocationResult::InsufficientGas { .. })
                    ));
                } else if boundary.is_some() {
                    assert!(matches!(
                        result,
                        Err(FinalChainNativeSessionError::MalformedNestedSlashingProof(
                            _
                        ))
                    ));
                } else {
                    assert!(matches!(
                        result,
                        Ok(FinalChainNativeInvocationResult::Completed(_))
                    ));
                }
                assert_eq!(raw.reads.get(), 0);
                assert_eq!(raw.account_reads.get(), 0);
            });
        }
        with_fixture(0, 1, boundary, |fixture| {
            for input in [
                vec![0xde, 0xad, 0xbe, 0xef],
                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR.to_vec(),
            ] {
                let selector_only = input == SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR;
                let mut probe = fixture.probe();
                let mut request = fixture.malformed(20_000);
                request.input = input;
                let result = completed(run(&mut probe, &request, &RawState::erroring()));
                assert!(matches!(
                    result.status,
                    FinalChainNativeStatus::ContractFailure { .. }
                ));
                assert!(result.output.is_empty());
                assert!(result.logs.is_empty());
                assert!(result.account_mutations.is_empty());
                assert!(result.raw_mutations.is_empty());
                if selector_only {
                    assert_eq!(result.gas_used.as_u64(), 20_000);
                }
            }
            let mut probe = fixture.probe();
            let mut request = request(0, OWNER, 200, 100_000);
            request.period = 2.into();
            assert!(
                matches!(probe.prepare(&request, &InvariantRead), Err(FinalChainNativeSessionError::StateRead(FinalChainNativeStateReadError::Invariant(message))) if message.contains("MalformedNestedSlashingProof"))
            );
        });
    }
}

#[test]
fn captured_probe_survives_later_native_row_writes() {
    for changed in [0_u64, 2] {
        with_fixture(2, 2, None, |fixture| {
            let prepared = fixture.capture();
            put(
                fixture.app.as_ref().unwrap(),
                Column::FinalChainMeta,
                native_key(changed),
                vec![0xff],
            );
            let before = contents(&fixture.config.db_path);
            let mut probe = prepared
                .into_execution_probe(REQUEST, EPOCH, 1, fixture.reader())
                .unwrap();
            assert_eq!(
                completed(run(&mut probe, &fixture.request(0), &fixture.raw())).status,
                FinalChainNativeStatus::Success
            );
            assert!(
                fixture
                    .chain()
                    .capture_native_predecessor(
                        3.into(),
                        NativePredecessorLimits {
                            max_retained_source_bytes: 1024 * 1024
                        }
                    )
                    .is_err()
            );
            assert_eq!(contents(&fixture.config.db_path), before);
        });
    }
}

#[test]
fn captured_probe_advances_current_only() {
    for delay in [0, 1, 20] {
        with_fixture(delay, 2, None, |fixture| {
            let current = fixture
                .chain()
                .dpos_snapshot_at_finalized_block(2.into())
                .unwrap();
            let delayed_period = 2_u64.saturating_sub(delay);
            let delayed = fixture
                .chain()
                .dpos_snapshot_at_finalized_block(delayed_period.into())
                .unwrap();
            assert_eq!(current.reward_reference_graph.current_block().unwrap(), 2);
            assert_eq!(
                delayed.reward_reference_graph.current_block().unwrap(),
                delayed_period
            );
            let mut expected = current.clone();
            expected.reward_reference_graph.next_block(3).unwrap();
            let session = fixture
                .chain()
                .native_session_from_captured_states(
                    REQUEST,
                    3.into(),
                    current,
                    delayed_period.into(),
                    delayed.clone(),
                )
                .unwrap();
            assert_eq!(session.dpos_state, expected);
            assert_eq!(
                session
                    .dpos_state
                    .reward_reference_graph
                    .current_block()
                    .unwrap(),
                3
            );
            assert_eq!(session.eligibility_period, delayed_period.into());
            assert_eq!(session.eligibility_state, delayed);
            drop(session);
            let mut probe = fixture.probe();
            let result = completed(run(&mut probe, &fixture.request(0), &fixture.raw()));
            assert_eq!(result.status, FinalChainNativeStatus::Success);
            assert_eq!(
                BigUint::from_bytes_be(&result.output),
                BigUint::from(10_u64)
            );
        });
    }
}

fn advance_fixture(fixture: &mut Fixture) {
    fixture.reader.take();
    let chain = fixture.chain.as_ref().unwrap();
    let parent = chain.last_block_number_typed().unwrap();
    let period = parent.checked_next().unwrap();
    let mut state = chain.dpos_snapshot_at_finalized_block(parent).unwrap();
    let concrete = commit_empty(
        fixture.concrete.take().unwrap(),
        &fixture.concrete_path,
        fixture.chain_id,
    );
    let current = concrete.prior_reader().identity();
    let mut header = stored_header(chain, parent.as_u64());
    header.state_root = H256(current.state_root);
    chain
        .advance_reward_reference_graph_block(&mut state, period)
        .unwrap();
    let app = fixture.app.as_ref().unwrap();
    put(
        app,
        Column::FinalChainBlkByNumber,
        period.as_u64().to_le_bytes().to_vec(),
        StoredBlockHeaderRlpOwned::from(&header).into_vec(),
    );
    put(
        app,
        Column::FinalChainMeta,
        native_key(period.as_u64()),
        encode_dpos_snapshot_rlp(&state).unwrap(),
    );
    put(
        app,
        Column::FinalChainMeta,
        FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
        period.as_u64().to_le_bytes().to_vec(),
    );
    *chain.reward_scheduler_runtime.lock().unwrap() =
        reward_scheduler::FinalChainRewardSchedulerRuntime::new(period);
    chain
        .authorize_reward_scheduler_joint_startup(EPOCH, period)
        .unwrap();
    chain.complete_reward_scheduler_recovery(EPOCH).unwrap();
    fixture.native_snapshot = state.clone();
    chain.dpos_snapshots.lock().unwrap().insert(period, state);
    fixture.delayed = ConcreteStateIdentity {
        period: period.saturating_sub_distance(chain.dpos_delegation_delay),
        state_root: current.state_root,
    };
    fixture.reader = Some(Arc::new(
        concrete.open_retained_checkpoint(fixture.delayed).unwrap(),
    ));
    fixture.concrete = Some(concrete);
}

#[test]
fn captured_probe_preserves_period_stage_selection() {
    with_fixture(0, 1, Some(3), |fixture| {
        {
            let mut before = fixture.probe();
            assert_eq!(before.correction_stage(), CorrectionStage::Historical);
            let request = fixture.malformed(20_000);
            let raw = RawState::erroring();
            let quote = before.prepare(&request, &raw).unwrap();
            assert!(matches!(
                before.invoke(&request, quote, &raw),
                Ok(FinalChainNativeInvocationResult::Completed(_))
            ));
        }
        let owner = fixture.chain() as *const FinalChain;
        advance_fixture(fixture);
        assert_eq!(fixture.chain() as *const FinalChain, owner);
        let mut at = fixture.probe();
        assert_eq!(at.correction_stage(), CorrectionStage::SlashingHardError);
        let request = fixture.malformed(20_000);
        let raw = RawState::erroring();
        let quote = at.prepare(&request, &raw).unwrap();
        assert!(matches!(
            at.invoke(&request, quote, &raw),
            Err(FinalChainNativeSessionError::MalformedNestedSlashingProof(
                _
            ))
        ));
        assert_eq!(
            fixture.chain().correction_stage_at(2.into()),
            CorrectionStage::Historical
        );
        let historical = fixture.chain().begin_native_simulation(2.into()).unwrap();
        assert_eq!(historical.correction_stage(), CorrectionStage::Historical);
        let mut historical = historical;
        let mut replay = fixture.malformed(20_000);
        replay.period = 2.into();
        let quote = historical.prepare(&replay, &raw).unwrap();
        assert!(matches!(
            historical.invoke(&replay, quote, &raw),
            Ok(FinalChainNativeInvocationResult::Completed(_))
        ));
    });
}

fn legacy_five(raw: &[u8]) -> Vec<u8> {
    let source = rlp::Rlp::new(raw);
    let mut output = rlp::RlpStream::new_list(5);
    for index in [0, 1, 2, 4, 5] {
        output.append_raw(source.at(index).unwrap().as_raw(), 1);
    }
    output.out().to_vec()
}

#[test]
fn captured_probe_preserves_pending_layout_rules() {
    for regime in ["complete", "magnolia", "cornus"] {
        let compatible = regime != "complete";
        for source_period in [0_u64, 2] {
            let rewards = FinalChainRewardsConfig {
                magnolia_period: if regime == "magnolia" {
                    0.into()
                } else {
                    100.into()
                },
                cornus_period: if regime == "cornus" {
                    0.into()
                } else {
                    100.into()
                },
                ..Default::default()
            };
            with_fixture_config(2, 2, None, rewards, |fixture| {
                let app = fixture.app.as_ref().unwrap();
                let raw = app
                    .final_chain()
                    .dpos_snapshot_raw(source_period)
                    .unwrap()
                    .unwrap();
                put(
                    app,
                    Column::FinalChainMeta,
                    native_key(source_period),
                    legacy_five(&raw),
                );
                let captured = fixture
                    .chain()
                    .capture_native_predecessor(
                        3.into(),
                        NativePredecessorLimits {
                            max_retained_source_bytes: 1024 * 1024,
                        },
                    )
                    .unwrap();
                if compatible {
                    let NativePredecessorCapture::Ready(value) = captured else {
                        panic!("native layout compatibility must remain available")
                    };
                    let result = value.into_execution_probe(REQUEST, EPOCH, 1, fixture.reader());
                    if source_period == 2 {
                        let expected =
                            crate::dpos_reward_graph::DposRewardGraphError::GraphHistoryIncomplete
                                .to_string();
                        assert!(
                            matches!(result, Err(FinalChainNativeSessionError::Domain(message)) if message == expected)
                        );
                    } else {
                        let mut probe = result.unwrap();
                        assert_eq!(
                            completed(run(&mut probe, &fixture.request(0), &fixture.raw())).status,
                            FinalChainNativeStatus::Success
                        );
                    }
                } else {
                    assert!(
                        matches!(captured, NativePredecessorCapture::Unavailable(NativePredecessorUnavailable::UnsupportedNativeLayout(period)) if period.as_u64() == source_period)
                    );
                }
            });
        }
    }
    let rewards = FinalChainRewardsConfig {
        magnolia_period: 100.into(),
        cornus_period: 100.into(),
        ..Default::default()
    };
    with_fixture_config(2, 2, None, rewards, |fixture| {
        {
            let mut complete = fixture.probe();
            assert_eq!(
                completed(run(&mut complete, &fixture.request(0), &fixture.raw())).status,
                FinalChainNativeStatus::Success
            );
        }
        let app = fixture.app.as_ref().unwrap();
        let raw = app.final_chain().dpos_snapshot_raw(2).unwrap().unwrap();
        put(
            app,
            Column::FinalChainMeta,
            native_key(2),
            legacy_five(&raw),
        );
        let mut batch = app.create_write_batch();
        app.batch_delete_raw(&mut batch, Column::FinalChainMeta, &native_key(0))
            .unwrap();
        app.commit_write_batch_with_sync(batch, false).unwrap();
        fixture.chain().dpos_snapshots.lock().unwrap().clear();
        assert!(matches!(
            fixture.chain().native_session_from_state(
                Some(REQUEST),
                3.into(),
                decode_dpos_snapshot_rlp(&legacy_five(&raw)).unwrap(),
                NativeSessionOrigin::PendingPeriod,
            ),
            Err(FinalChainNativeSessionError::PreMagnoliaUnsupported)
        ));
    });
}

#[test]
fn captured_probe_rejects_checkpoint_mismatch() {
    with_fixture(2, 2, None, |fixture| {
        let current = fixture.reader().committed_identity();
        assert!(
            ConcreteCheckpointReaders::open_read_only(&fixture.concrete_path, current, []).is_err()
        );
        let no_delayed = Arc::new(
            ConcreteCheckpointReaders::open_read_only(&fixture.concrete_path, current, [current])
                .unwrap(),
        );
        assert!(
            matches!(fixture.capture().into_execution_probe(REQUEST, EPOCH, 1, no_delayed), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("FINAL_CHAIN_DELAYED_IDENTITY_MISSING"))
        );
        for identity in [
            ConcreteStateIdentity {
                period: 1.into(),
                ..current
            },
            ConcreteStateIdentity {
                state_root: [0xee; 32],
                ..current
            },
        ] {
            assert!(
                ConcreteCheckpointReaders::open_read_only(
                    &fixture.concrete_path,
                    identity,
                    [identity]
                )
                .is_err()
            );
        }
        let chain = fixture.chain();
        let scheduler = chain.reward_scheduler_runtime.lock().unwrap().clone();
        put(
            fixture.app.as_ref().unwrap(),
            Column::FinalChainMeta,
            FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
            1_u64.to_le_bytes().to_vec(),
        );
        *chain.reward_scheduler_runtime.lock().unwrap() =
            reward_scheduler::FinalChainRewardSchedulerRuntime::new(1.into());
        chain
            .authorize_reward_scheduler_joint_startup(EPOCH, 1.into())
            .unwrap();
        chain.complete_reward_scheduler_recovery(EPOCH).unwrap();
        assert!(
            matches!(fixture.capture().into_execution_probe(REQUEST, EPOCH, 1, fixture.reader()), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("committed checkpoint mismatch"))
        );
        put(
            fixture.app.as_ref().unwrap(),
            Column::FinalChainMeta,
            FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
            2_u64.to_le_bytes().to_vec(),
        );
        *chain.reward_scheduler_runtime.lock().unwrap() = scheduler;
        let mut header = stored_header(fixture.chain(), 2);
        header.state_root = H256::repeat_byte(0xef);
        put(
            fixture.app.as_ref().unwrap(),
            Column::FinalChainBlkByNumber,
            2_u64.to_le_bytes().to_vec(),
            StoredBlockHeaderRlpOwned::from(&header).into_vec(),
        );
        assert!(
            matches!(fixture.capture().into_execution_probe(REQUEST, EPOCH, 1, fixture.reader()), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("committed checkpoint mismatch"))
        );
    });
}

#[test]
fn captured_probe_preserves_scheduler_refusals() {
    for mode in 0..8 {
        with_fixture(0, 1, None, |fixture| {
            let chain = fixture.chain();
            if mode < 3 {
                let parent = if mode == 2 { 2.into() } else { 1.into() };
                *chain.reward_scheduler_runtime.lock().unwrap() =
                    reward_scheduler::FinalChainRewardSchedulerRuntime::new(parent);
                if mode != 0 {
                    chain
                        .authorize_reward_scheduler_joint_startup(EPOCH, parent)
                        .unwrap();
                }
                if mode == 2 {
                    chain.complete_reward_scheduler_recovery(EPOCH).unwrap();
                }
            }
            if mode == 5 {
                let _session = chain
                    .begin_native_session_bound_at_state_api_epoch(
                        [0x77; 32],
                        EPOCH,
                        2.into(),
                        1.into(),
                    )
                    .unwrap();
            }
            if mode == 6 {
                let mut runtime = chain.reward_scheduler_runtime.lock().unwrap();
                let basis = runtime.basis(EPOCH, 1.into(), 2.into(), REQUEST).unwrap();
                runtime
                    .arm(
                        basis.successor(3),
                        reward_scheduler::FinalChainRewardSchedulerPublicationBinding {
                            request_id: REQUEST,
                            expected_parent: 1.into(),
                            period: 2.into(),
                            state_api_epoch: EPOCH,
                            concrete_database_id: fixture
                                .concrete
                                .as_ref()
                                .unwrap()
                                .observation()
                                .unwrap()
                                .identity
                                .database_id,
                            concrete_generation: 2,
                            concrete_projection_hash: [0x78; 32],
                            publication_plan_id: [0x79; 32],
                        },
                    )
                    .unwrap();
            }
            if mode == 7 {
                chain
                    .reward_scheduler_runtime
                    .lock()
                    .unwrap()
                    .record_verified_discard(EPOCH, EPOCH + 1, 1.into())
                    .unwrap();
            }
            let before = chain.reward_scheduler_runtime.lock().unwrap().clone();
            let epoch = if mode == 3 {
                0
            } else if mode == 4 {
                EPOCH + 1
            } else {
                EPOCH
            };
            assert!(
                fixture
                    .capture()
                    .into_execution_probe(REQUEST, epoch, 1, fixture.reader())
                    .is_err()
            );
            assert_eq!(*chain.reward_scheduler_runtime.lock().unwrap(), before);
        });
    }
}

struct InvariantRead;

impl FinalChainNativeStateRead for InvariantRead {
    fn account(
        &self,
        _: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        Err(FinalChainNativeStateReadError::Invariant(
            "MalformedNestedSlashingProof is only a test message".into(),
        ))
    }

    fn raw_storage(
        &self,
        _: [u8; 20],
        _: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        Err(FinalChainNativeStateReadError::Invariant(
            "MalformedNestedSlashingProof is only a test message".into(),
        ))
    }
}

struct ChangedRead<'a> {
    owner: &'a FinalChain,
    raw: RawState,
    armed: Cell<bool>,
    fail: bool,
}

impl FinalChainNativeStateRead for ChangedRead<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.raw.account(address)
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        if self.armed.replace(false) {
            let mut header = stored_header(self.owner, 1);
            header.state_root = H256::repeat_byte(0xed);
            put(
                &self.owner.storage,
                Column::FinalChainBlkByNumber,
                1_u64.to_le_bytes().to_vec(),
                StoredBlockHeaderRlpOwned::from(&header).into_vec(),
            );
            if self.fail {
                return Err(FinalChainNativeStateReadError::Invariant(
                    "test invocation read failed".into(),
                ));
            }
        }
        self.raw.raw_storage(address, key)
    }
}

#[test]
fn captured_probe_rejects_context_changes() {
    for scheduler in [false, true] {
        with_fixture(0, 1, None, |fixture| {
            let mut probe = fixture.probe();
            let mut before_invocation = fixture.probe();
            if scheduler {
                *fixture.chain().reward_scheduler_runtime.lock().unwrap() =
                    reward_scheduler::FinalChainRewardSchedulerRuntime::new(1.into());
            } else {
                let mut header = stored_header(fixture.chain(), 1);
                header.state_root = H256::repeat_byte(0xec);
                put(
                    fixture.app.as_ref().unwrap(),
                    Column::FinalChainBlkByNumber,
                    1_u64.to_le_bytes().to_vec(),
                    StoredBlockHeaderRlpOwned::from(&header).into_vec(),
                );
            }
            assert!(
                matches!(probe.validate_context(), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("context changed"))
            );
            let raw = RawState::erroring();
            assert!(
                matches!(before_invocation.prepare(&fixture.request(0), &raw), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("context changed"))
            );
            assert_eq!(raw.reads.get(), 0);
            assert_eq!(raw.account_reads.get(), 0);
            assert!(matches!(
                probe.prepare(&fixture.request(0), &fixture.raw()),
                Err(FinalChainNativeSessionError::Aborted)
            ));
        });
    }
    for fail in [false, true] {
        with_fixture(0, 1, None, |fixture| {
            let mut probe = fixture.probe();
            let raw = ChangedRead {
                owner: fixture.chain(),
                raw: fixture.raw(),
                armed: Cell::new(false),
                fail,
            };
            let mut request = request(0, OWNER, 200, 100_000);
            request.period = 2.into();
            let quote = probe.prepare(&request, &raw).unwrap();
            raw.armed.set(true);
            assert!(
                matches!(probe.invoke(&request, quote, &raw), Err(FinalChainNativeSessionError::Domain(message)) if message == "native probe committed context changed")
            );
            assert!(!raw.armed.get());
        });
    }
}

#[test]
fn captured_probe_preserves_ordered_native_effects() {
    with_fixture(0, 1, None, |fixture| {
        let app_before = contents(&fixture.config.db_path);
        let cache_before = fixture.chain().dpos_snapshots.lock().unwrap().clone();
        let mut probe = fixture.probe();
        let raw = fixture.raw();
        let initial_row = vec![0xc8, 0xc6, 0x82, 0x27, 0x10, 0x64, 0x80, 0x80, 0x80];
        assert_eq!(
            raw.raw_storage(DPOS_CONTRACT_ADDRESS, &ConcreteStorageKey(VALIDATOR_KEY))
                .unwrap(),
            ConcreteRead::Present(initial_row.clone()),
        );
        let mut expected_row = EXTENDED_VALIDATOR_200.to_vec();
        expected_row[7] = 2;
        for sequence in 0..2 {
            let mut request = request(sequence, OWNER, 200, 100_000);
            request.period = 2.into();
            let outcome = completed(run(&mut probe, &request, &raw));
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            assert_eq!(outcome.gas_used.as_u64(), 20_000);
            assert!(outcome.output.is_empty());
            assert!(outcome.account_mutations.is_empty());
            assert_eq!(
                outcome.raw_mutations,
                vec![FinalChainNativeRawMutation {
                    address: DPOS_CONTRACT_ADDRESS,
                    key: ConcreteStorageKey(VALIDATOR_KEY),
                    expected: ConcreteRead::Present(if sequence == 0 {
                        initial_row.clone()
                    } else {
                        expected_row.clone()
                    }),
                    operation: FinalChainNativeRawOperation::Put(
                        FinalChainNativeRawValue::new(expected_row.clone()).unwrap()
                    ),
                }]
            );
            assert_eq!(
                outcome.logs,
                vec![FinalChainCallLog {
                    address: DPOS_CONTRACT_ADDRESS,
                    topics: vec![DPOS_COMMISSION_SET_TOPIC, address_topic(VALIDATOR)],
                    data: abi_word_from_u64(200).to_vec(),
                }]
            );
            assert!(probe.input_aliases(request.id).unwrap().is_empty());
            assert!(probe.input_mutations(request.id).unwrap().is_empty());
            apply_raw_mutations(&raw, &outcome);
        }
        let completed_id = FinalChainNativeInvocationId {
            transaction: FinalChainTransactionPosition::new(0),
            sequence: 1,
        };
        assert!(probe.input_mutations(completed_id).is_err());
        assert!(matches!(
            probe.validate_context(),
            Err(FinalChainNativeSessionError::Aborted)
        ));
        assert_eq!(
            *fixture.chain().dpos_snapshots.lock().unwrap(),
            cache_before
        );
        assert_eq!(contents(&fixture.config.db_path), app_before);
    });
}

fn sst_files(path: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut files = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "sst"))
        .map(|entry| {
            let meta = entry.metadata().unwrap();
            (
                entry.file_name().into_string().unwrap(),
                meta.len(),
                meta.modified().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn captured_probe_leaves_stores_unchanged() {
    for mode in 0..4 {
        with_fixture(0, 1, Some(2), |fixture| {
            let app_sst = sst_files(&fixture.config.db_path);
            let concrete_sst = sst_files(&fixture.concrete_path);
            let before_sentinel = contents(&fixture.config.db_path);
            assert!(
                !before_sentinel
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|(key, _)| key == b"captured_probe_wal_sentinel")
            );
            put(
                fixture.app.as_ref().unwrap(),
                Column::FinalChainMeta,
                b"captured_probe_wal_sentinel".to_vec(),
                vec![0xa1],
            );
            let marker =
                encode_concrete_execution_marker(&marker(fixture.concrete.as_ref().unwrap()));
            fixture
                .concrete
                .as_mut()
                .unwrap()
                .stage_execution(&marker)
                .unwrap();
            assert_eq!(sst_files(&fixture.config.db_path), app_sst);
            assert_eq!(sst_files(&fixture.concrete_path), concrete_sst);
            let before_app = contents(&fixture.config.db_path);
            let before_concrete = contents(&fixture.concrete_path);
            assert!(
                before_app
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|(key, value)| key == b"captured_probe_wal_sentinel" && value == &[0xa1])
            );
            assert!(
                before_concrete
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .any(
                        |(key, value)| key == b"rustaxa_concrete_execution_pending_v1"
                            && value == &marker
                    )
            );
            assert!(before_app.iter().any(|(_, rows)| rows.is_empty()));
            assert!(before_concrete.iter().any(|(_, rows)| rows.is_empty()));
            {
                let mut probe = fixture.probe();
                match mode {
                    0 => assert_eq!(
                        completed(run(&mut probe, &fixture.request(0), &fixture.raw())).status,
                        FinalChainNativeStatus::Success
                    ),
                    1 => {
                        let request = fixture.malformed(20_000);
                        let raw = RawState::erroring();
                        let quote = probe.prepare(&request, &raw).unwrap();
                        assert!(matches!(
                            probe.invoke(&request, quote, &raw),
                            Err(FinalChainNativeSessionError::MalformedNestedSlashingProof(
                                _
                            ))
                        ));
                    }
                    2 => {
                        let mut request = fixture.request(0);
                        request.period = 3.into();
                        assert!(matches!(
                            probe.prepare(&request, &fixture.raw()),
                            Err(FinalChainNativeSessionError::PeriodMismatch)
                        ));
                    }
                    _ => assert!(matches!(
                        run(&mut probe, &fixture.malformed(0), &RawState::erroring()),
                        FinalChainNativeInvocationResult::InsufficientGas { .. }
                    )),
                }
            }
            assert_eq!(contents(&fixture.config.db_path), before_app);
            assert_eq!(contents(&fixture.concrete_path), before_concrete);
            let committed = fixture.reader().committed_identity();
            fixture.reader.take();
            fixture.chain.take();
            fixture.concrete.take();
            fixture.app.take();
            let mut config = fixture.config.clone();
            config.create_if_missing = false;
            config.create_missing_column_families = false;
            let app = Storage::new(config).unwrap();
            let concrete =
                ConcreteStateLifecycle::open(&fixture.concrete_path, fixture.chain_id, committed)
                    .unwrap();
            assert_eq!(contents(&fixture.config.db_path), before_app);
            assert_eq!(contents(&fixture.concrete_path), before_concrete);
            drop(concrete);
            drop(app);
            assert_eq!(contents(&fixture.config.db_path), before_app);
            assert_eq!(contents(&fixture.concrete_path), before_concrete);
        });
    }
}

#[test]
fn captured_probe_preserves_registration_alias_order() {
    let input = "d6fdc12700000000000000000000000058da990a8f4a3a6ca7cb6315d68a14010591735200000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000064000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000001c00000000000000000000000000000000000000000000000000000000000000041453b4f2f4620000c250e185c6e035f9326c4eeba287dfa7ac87cddef4ac7486677ee4603f5b09aac162103465e46ff0a980a6149db1782c9577f61cc92711e3d1b0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000204444444444444444444444444444444444444444444444444444444444444444000000000000000000000000000000000000000000000000000000000000000976616c696461746f7200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008656e64706f696e74000000000000000000000000000000000000000000000000";
    let input = input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(input.len(), 516);
    assert_eq!(input[292], 27);
    assert_eq!(&input[356..388], &[0x44; 32]);
    let validator = [
        0x58, 0xda, 0x99, 0x0a, 0x8f, 0x4a, 0x3a, 0x6c, 0xa7, 0xcb, 0x63, 0x15, 0xd6, 0x8a, 0x14,
        0x01, 0x05, 0x91, 0x73, 0x52,
    ];
    let expected_alias = FinalChainNativeInputAlias {
        scope: FinalChainNativeInputAliasScope::OwnedInputOnly,
        address: DPOS_CONTRACT_ADDRESS,
        key: ConcreteStorageKey(concrete_storage_key(&[&[0, 4], &validator])),
        offset: 356,
        length: 32,
    };
    let expected_mutation = FinalChainNativeInputMutation {
        offset: 292,
        expected: 27,
        replacement: 0,
    };
    with_fixture(0, 1, Some(2), |fixture| {
        let initial = fixture
            .chain()
            .dpos_snapshot_at_finalized_block(1.into())
            .unwrap();
        let raw = RawState::from_snapshot(&initial, BTreeMap::new());
        let cache_before = fixture.chain().dpos_snapshots.lock().unwrap().clone();
        let app_before = contents(&fixture.config.db_path);
        let concrete_before = contents(&fixture.concrete_path);
        let mut probe = fixture.probe();
        let mut request = fixture.request(0);
        request.caller = OWNER;
        request.value = FinalChainNativeValue::new(BigUint::from(1000_u64));
        request.input = input.clone();
        request.supplied_gas = 1_000_000.into();
        let quote = probe.prepare(&request, &raw).unwrap();
        assert!(quote.required_gas.as_u64() > 0);
        assert!(quote.required_gas <= request.supplied_gas);
        let result = completed(probe.invoke(&request, quote, &raw).unwrap());
        assert_eq!(result.status, FinalChainNativeStatus::Success);
        assert_eq!(result.gas_used, quote.required_gas);
        assert!(result.account_mutations.is_empty());
        assert_eq!(
            probe.input_aliases(request.id).unwrap(),
            vec![expected_alias.clone()]
        );
        assert_eq!(
            probe.input_mutations(request.id).unwrap(),
            vec![expected_mutation]
        );
        assert_eq!(request.input, input);
        apply_raw_mutations(&raw, &result);
        let vrf = raw
            .raw_storage(DPOS_CONTRACT_ADDRESS, &expected_alias.key)
            .unwrap();
        assert_eq!(vrf, ConcreteRead::Present(vec![0x44; 32]));
        request.id.sequence = 1;
        let quote = probe.prepare(&request, &raw).unwrap();
        let duplicate = completed(probe.invoke(&request, quote, &raw).unwrap());
        assert!(matches!(
            duplicate.status,
            FinalChainNativeStatus::ContractFailure { .. }
        ));
        assert!(duplicate.account_mutations.is_empty());
        assert!(duplicate.raw_mutations.is_empty());
        assert!(duplicate.logs.is_empty());
        assert!(probe.input_aliases(request.id).unwrap().is_empty());
        assert_eq!(
            probe.input_mutations(request.id).unwrap(),
            vec![expected_mutation]
        );
        assert!(probe.input_mutations(request.id).is_err());
        drop(probe);
        request.id.sequence = 0;
        let mut fresh = fixture.probe();
        let fresh_raw = RawState::from_snapshot(&initial, BTreeMap::new());
        assert_eq!(completed(run(&mut fresh, &request, &fresh_raw)), result);
        assert_eq!(
            fresh.input_aliases(request.id).unwrap(),
            vec![expected_alias.clone()]
        );
        assert_eq!(
            fresh.input_mutations(request.id).unwrap(),
            vec![expected_mutation]
        );
        assert_eq!(
            *fixture.chain().dpos_snapshots.lock().unwrap(),
            cache_before
        );
        assert_eq!(contents(&fixture.config.db_path), app_before);
        assert_eq!(contents(&fixture.concrete_path), concrete_before);
    });
}
