use super::*;
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCheckpointReaders, ConcreteStateLifecycle,
    ConcreteStateMutationBatch, ConcreteStorageMutation,
};
use rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog;
use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
use rustaxa_types::concrete_state::{
    ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteStorageKey,
};

fn validators() -> Vec<GenesisValidator> {
    vec![GenesisValidator {
        address: VALIDATOR,
        vrf_key: [0; 32],
        total_stake: U256::from(10_000).to_big_endian().to_vec(),
        delegations: vec![(VALIDATOR, U256::from(10_000).to_big_endian().to_vec())],
        metadata: GenesisValidatorMetadata {
            owner: OWNER,
            commission: 100,
            ..Default::default()
        },
    }]
}

pub(super) fn with_real_owner(
    rewards: FinalChainRewardsConfig,
    dpos: GenesisDposConfig,
    test: impl FnOnce(Arc<FinalChain>, Arc<ConcreteCheckpointReaders>),
) {
    with_arc_chain_dpos_config(
        "owned-delayed-genesis-plan",
        rewards.clone(),
        FinalChainBlockNumber::MAX,
        dpos.clone(),
        |plan| {
            plan.storage
                .metadata()
                .set_genesis_hash_if_empty(&[7; 32])
                .unwrap();
            let chain_id = plan.concrete_chain_identity().unwrap();
            let genesis = plan.dpos_snapshot_at_finalized_block(0.into()).unwrap();
            let mut storage = Vec::new();
            for ((address, key), candidates) in
                canonical_concrete_precompile_storage(&genesis, false).unwrap()
            {
                // Slashing has no genesis account before Magnolia.
                if address == SLASHING_CONTRACT_ADDRESS {
                    assert!(genesis.slashing_jail_blocks.is_empty());
                    assert!(genesis.slashing_jailed_validators.is_empty());
                    assert!(genesis.slashing_double_voting_proofs.is_empty());
                    assert_eq!(key[31], 2);
                    assert_eq!(candidates, vec![vec![0xc0]]);
                    continue;
                }
                let value = candidates.first().unwrap().clone();
                if !value.is_empty() {
                    storage.push(ConcreteStorageMutation {
                        address,
                        key: ConcreteStorageKey(key),
                        value: Some(value),
                    });
                }
            }
            let mut catalog = storage
                .iter()
                .map(|row| ConcreteStorageSlot {
                    address: row.address,
                    key: row.key.0,
                })
                .collect::<Vec<_>>();
            catalog.sort_unstable();
            assert!(catalog.windows(2).all(|pair| pair[0] < pair[1]));
            let accounts = plan
                .account_snapshot_map_at_block(0.into())
                .unwrap()
                .into_iter()
                .map(|(address, account)| {
                    assert_eq!(account.storage_root_hash, [0; 32]);
                    assert_eq!(account.code_hash, [0; 32]);
                    assert_eq!(account.code_size, 0);
                    let mut rlp = rlp::RlpStream::new_list(5);
                    rlp.append(&account.nonce.to_bytes());
                    rlp.append(account.balance.as_u256());
                    rlp.append_empty_data();
                    rlp.append_empty_data();
                    rlp.append(&0_u64);
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
                            physical_rlp: rlp.out().to_vec(),
                        },
                    }
                })
                .collect();
            let native_path = temp_db_path("owned-delayed-native");
            let concrete_path = crate::closeout_test_scratch::new_path(
                native_path.with_extension("concrete"),
                false,
            );
            let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
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
            let genesis_identity = lifecycle.prior_reader().identity();
            let native_storage = Arc::new(Storage::new(Config::new(native_path.clone())).unwrap());
            native_storage
                .metadata()
                .set_genesis_hash_if_empty(&[7; 32])
                .unwrap();
            let reopen = || {
                Arc::new(
                    FinalChain::new_with_genesis_state_root(
                        native_storage.clone(),
                        1_000_000.into(),
                        0,
                        H256(genesis_identity.state_root),
                        true,
                        Vec::new(),
                        validators(),
                        dpos.clone(),
                        rewards.clone(),
                    )
                    .unwrap(),
                )
            };
            let first = reopen();
            assert_eq!(first.concrete_chain_identity().unwrap(), chain_id);
            let lifecycle = commit_empty_period(
                &first,
                lifecycle,
                &concrete_path,
                owned_test_signed_block(1),
            )
            .unwrap();
            let parent_identity = lifecycle.prior_reader().identity();
            assert_eq!(parent_identity.period, 1.into());
            assert_eq!(
                first.committed_state_descriptor().unwrap().state_root,
                parent_identity.state_root
            );
            drop(first);
            let chain = reopen();
            assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            let selected = if dpos.delegation_delay == 0 {
                parent_identity
            } else {
                genesis_identity
            };
            let reader = Arc::new(lifecycle.open_retained_checkpoint(selected).unwrap());
            assert_eq!(reader.committed_identity(), parent_identity);
            assert!(reader.retained_identities().contains(&selected));
            test(chain, reader);
            drop(lifecycle);
            drop(native_storage);
            crate::closeout_test_scratch::dispose(native_path).unwrap();
            crate::closeout_test_scratch::dispose(concrete_path).unwrap();
        },
    );
}

use crate::concrete_state_projection::*;
use crate::final_chain_execution::*;
use anyhow::{Result, ensure};
use rustaxa_storage::{ConcreteCommitApproval, PreparedConcreteState};
use std::path::Path;

struct EmptyStaged {
    prepared: PreparedConcreteState,
    projection: FinalChainConcreteStateProjection,
    provenance: Vec<u8>,
}
struct EmptyLeaf {
    path: PathBuf,
    chain_id: [u8; 32],
    owner: RefCell<Option<ConcreteStateLifecycle>>,
    staged: RefCell<Option<EmptyStaged>>,
}
impl FinalChainExecutionLeaf for EmptyLeaf {
    fn load_committed_state_descriptor(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        let owner = self.owner.borrow();
        let observed = owner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner unavailable"))?
            .observation()?;
        ensure!(
            observed.identity.chain_id == request.concrete_chain_identity
                && self.chain_id == request.concrete_chain_identity,
            "chain identity"
        );
        Ok(FinalChainExternalEvmPreflightReport {
            request_id: request.request_id,
            state_api_epoch: 91,
            committed: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
            concrete_provenance_rlp: observed.provenance_rlp,
            pending_concrete_marker_rlp: observed.pending_marker_rlp,
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn load_system_transaction_facts(
        &self,
        request: &FinalChainSystemTransactionFactsRequest,
    ) -> Result<FinalChainSystemTransactionPlanFact> {
        ensure!(!request.is_pillar_block_period, "pillar period unsupported");
        let owner = self.owner.borrow();
        let owner = owner
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner unavailable"))?;
        ensure!(
            matches!(
                owner
                    .prior_reader()
                    .account(request.bridge_contract_address)?,
                ConcreteRead::Absent | ConcreteRead::Tombstone
            ),
            "bridge account must be absent"
        );
        Ok(FinalChainSystemTransactionPlanFact {
            request_id: request.request_id,
            period: request.period,
            is_pillar_block_period: false,
            bridge_contract_address: request.bridge_contract_address,
            bridge_contract_found: false,
            bridge_contract_has_code: false,
            should_finalize_epoch: false,
            system_account_nonce: FinalChainNonce::zero(),
            block_gas_limit: request.block_gas_limit,
        })
    }
    fn execute_transactions(
        &self,
        request: &FinalChainEvmExecutionRequest,
    ) -> Result<FinalChainEvmExecutionReport> {
        ensure!(
            request.transactions.is_empty(),
            "empty fixture received transactions"
        );
        ensure!(self.staged.borrow().is_none(), "fixture already staged");
        let mut owner = self.owner.borrow_mut();
        let owner = owner
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("owner unavailable"))?;
        owner.stage_execution(&request.concrete_marker_rlp)?;
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        let prior_identity = owner.prior_reader().identity();
        let prepared = owner.prepare(request.period, ConcreteStateMutationBatch::default())?;
        let identity = prepared.next_identity();
        ensure!(
            identity.state_root == prior_identity.state_root,
            "empty fixture batch changed the prior root"
        );
        let catalog = decode_concrete_storage_catalog(&owner.observation()?.catalog_rlp)?;
        let reader = owner.prior_reader();
        ensure!(
            reader.identity() == prior_identity,
            "fixture prior reader changed"
        );
        let mut storage = Vec::with_capacity(catalog.len());
        for slot in &catalog {
            let value = match reader.storage(slot.address, ConcreteStorageKey(slot.key))? {
                ConcreteRead::Present(value) if !value.is_empty() => value,
                _ => anyhow::bail!("fresh fixture catalog slot is not live"),
            };
            storage.push(
                crate::concrete_state_projection::FinalChainConcreteStorageProjection {
                    contract: slot.address,
                    key: slot.key,
                    value,
                },
            );
        }
        let projection = FinalChainConcreteStateProjection {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            prior_state: marker.prior_state,
            post_transaction_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: identity.state_root,
            },
            post_rewards_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: identity.state_root,
            },
            transaction_effects: Vec::new(),
            accounts: Vec::new(),
            storage,
            invocations: Vec::new(),
            rewards_input: Vec::new(),
            catalog_hash:
                rustaxa_types::codec::rlp::concrete_lifecycle::concrete_storage_slot_catalog_hash(
                    catalog.iter().copied(),
                ),
        };
        *self.staged.borrow_mut() = Some(EmptyStaged {
            prepared,
            projection,
            provenance: Vec::new(),
        });
        Ok(FinalChainEvmExecutionReport {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            post_transaction_state_root: identity.state_root,
            cumulative_gas_used: 0.into(),
            results: Vec::new(),
        })
    }
    fn distribute_rewards(
        &self,
        request: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport> {
        validate_zero_effect_reward_request(request)?;
        let mut staged = self.staged.borrow_mut();
        let staged = staged
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture not staged"))?;
        ensure!(
            staged.projection.post_transaction_state.root == request.post_transaction_state_root,
            "rewards prior root"
        );
        staged.projection.rewards_input =
            encode_concrete_rewards_input(&request.distribution_stats);
        let projection = encode_concrete_state_projection(&staged.projection);
        let projection_hash = concrete_state_bytes_digest(&projection);
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        staged.provenance = encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: staged.projection.post_rewards_state,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash: staged.projection.catalog_hash,
        });
        Ok(FinalChainEvmRewardsReport {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            period: request.period,
            status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: staged.projection.post_rewards_state.root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            concrete_projection_rlp: projection,
            concrete_projection_hash: projection_hash,
            concrete_provenance_rlp: staged.provenance.clone(),
            total_reward: Vec::new(),
        })
    }
    fn commit_staged_state(
        &self,
        request: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult> {
        let staged = self
            .staged
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("fixture not staged"))?;
        ensure!(
            encode_concrete_state_projection(&staged.projection) == request.concrete_projection_rlp
                && staged.provenance == request.concrete_provenance_rlp,
            "projection mismatch"
        );
        let owner = self
            .owner
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("owner unavailable"))?;
        let approval = ConcreteCommitApproval {
            marker_rlp: request.concrete_marker_rlp.clone(),
            provenance_rlp: request.concrete_provenance_rlp.clone(),
            catalog_rlp: owner.observation()?.catalog_rlp,
            projection_hash: request.concrete_projection_hash,
            catalog_hash: staged.projection.catalog_hash,
        };
        let observed = owner.commit_approved(staged.prepared, approval)?;
        ensure!(
            observed.pending_marker_rlp.is_empty()
                && observed.provenance_rlp == request.concrete_provenance_rlp,
            "commit observation mismatch"
        );
        let reopened = ConcreteStateLifecycle::open(&self.path, self.chain_id, observed.committed)?;
        *self.owner.borrow_mut() = Some(reopened);
        Ok(FinalChainExternalEvmStateCommitResult {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            plan_id: request.plan_id,
            period: request.period,
            publication_block_hash: request.publication_block_hash,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: request.post_rewards_state_root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_projection_rlp: request.concrete_projection_rlp.clone(),
            concrete_projection_hash: request.concrete_projection_hash,
            concrete_provenance_rlp: request.concrete_provenance_rlp.clone(),
            committed_state: Some(FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            }),
            status: FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
            error_code: String::new(),
        })
    }
}

fn validate_zero_effect_reward_request(request: &FinalChainEvmRewardsRequest) -> Result<()> {
    ensure!(
        request.transaction_fees.is_empty() && request.transaction_gas_used.is_empty(),
        "nonempty transaction rewards"
    );
    ensure!(
        request.period.as_u64() == 1
            && request.block_gas_used.is_zero()
            && request.finalized_dag_block_count == 0
            && request.distribution_stats.len() == 1,
        "nonzero fixture reward work: {:?}",
        request.distribution_stats
    );
    let distributions =
        crate::rewards_stats::decode_rewards_block_distributions(&request.distribution_stats)?;
    let distribution = distributions
        .first()
        .context("missing fixture distribution")?;
    ensure!(
        distributions.len() == 1
            && distribution.period == request.period.as_u64()
            && distribution.block_author.0 == request.block_author
            && distribution.blocks_per_year == 0
            && distribution.validators_stats.is_empty()
            && distribution.total_dag_blocks_count == 0
            && distribution.total_votes_weight == 0
            && distribution.max_votes_weight == 0,
        "nonzero fixture distribution: {:?}",
        distributions
    );
    Ok(())
}

pub(super) fn commit_empty_period(
    chain: &FinalChain,
    owner: ConcreteStateLifecycle,
    path: &Path,
    pbft_block_rlp: Vec<u8>,
) -> Result<ConcreteStateLifecycle> {
    let prior = owner.observation()?;
    ensure!(
        prior.committed.period.is_genesis()
            && prior.generation == 0
            && prior.pending_marker_rlp.is_empty()
            && chain.last_block_number_typed()?.is_genesis()
            && chain.rewards_config.magnolia_period.as_u64() == 7
            && chain.rewards_config.yield_percentage == 0,
        "fixture requires fresh genesis and zero-yield pre-Magnolia period one"
    );
    let chain_id = chain.concrete_chain_identity()?;
    let leaf = EmptyLeaf {
        path: path.to_owned(),
        chain_id,
        owner: RefCell::new(Some(owner)),
        staged: RefCell::new(None),
    };
    initialize_and_recover_final_chain_application_state_at_joint_startup(chain, &leaf)?;
    let request = FinalChainExecutionRequest {
        pbft_block_rlp,
        transactions: Vec::<FinalizationTransaction>::new(),
        finalized_dag_blocks: Vec::<FinalizationDagBlock>::new(),
        blocks_per_year: 0,
        cert_votes: Vec::new(),
        block_gas_limit: 1_000_000.into(),
        mode: FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
    };
    let publication = execute_final_chain_application_task(
        chain,
        request,
        Default::default(),
        false,
        [0; 20],
        &leaf,
    )?;
    ensure!(
        publication.period.as_u64() == 1 && publication.error_code.is_empty(),
        "period one publication failed"
    );
    ensure!(leaf.staged.borrow().is_none(), "staged fixture remains");
    let result = leaf
        .owner
        .into_inner()
        .ok_or_else(|| anyhow::anyhow!("owner unavailable"))?;
    let observed = result.observation()?;
    let descriptor = chain.committed_state_descriptor()?;
    ensure!(
        observed.identity.chain_id == chain_id && observed.committed.period.as_u64() == 1,
        "period one identity mismatch"
    );
    ensure!(
        observed.committed.period == descriptor.period
            && observed.committed.state_root == descriptor.state_root,
        "published descriptor mismatch"
    );
    ensure!(
        observed.pending_marker_rlp.is_empty(),
        "pending marker remains"
    );
    let provenance = decode_concrete_state_provenance(&observed.provenance_rlp)?;
    ensure!(
        provenance.identity == observed.identity
            && provenance.committed_state.period == observed.committed.period.as_u64()
            && provenance.committed_state.root == observed.committed.state_root,
        "committed provenance mismatch"
    );
    Ok(result)
}

#[test]
fn zero_effect_reward_request_rejects_nonzero_or_mismatched_actual_rows() -> Result<()> {
    let data = vec![
        0xda, 0x94, 0x1a, 0x64, 0x2f, 0x0e, 0x3c, 0x3a, 0xf5, 0x45, 0xe7, 0xac, 0xbd, 0x38, 0xb0,
        0x72, 0x51, 0xb3, 0x99, 0x09, 0x14, 0xf1, 0x80, 0xc0, 0x80, 0x80, 0x80,
    ];
    let row = crate::rewards_stats::RewardsStatsPeriodRlp { period: 1, data };
    let decoded =
        crate::rewards_stats::decode_rewards_block_distributions(std::slice::from_ref(&row))?;
    let request = FinalChainEvmRewardsRequest {
        period: 1.into(),
        block_author: decoded[0].block_author.0,
        distribution_stats: vec![row],
        ..Default::default()
    };
    validate_zero_effect_reward_request(&request)?;
    let mut wrong = request.clone();
    wrong.period = 2.into();
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.distribution_stats[0].period = 2;
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.block_author = [0; 20];
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.block_gas_used = 1.into();
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.finalized_dag_block_count = 1;
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.transaction_gas_used.push(0.into());
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.transaction_fees.push(Vec::new());
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.distribution_stats.clear();
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong
        .distribution_stats
        .push(wrong.distribution_stats[0].clone());
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    let mut wrong = request.clone();
    wrong.distribution_stats[0].data = vec![0xc0];
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    for field in [1, 3, 4, 5] {
        let source = rlp::Rlp::new(&request.distribution_stats[0].data);
        let mut changed = rlp::RlpStream::new_list(6);
        for index in 0..6 {
            if index == field {
                changed.append(&1_u64);
            } else {
                changed.append_raw(source.at(index)?.as_raw(), 1);
            }
        }
        let mut wrong = request.clone();
        wrong.distribution_stats[0].data = changed.out().to_vec();
        assert!(validate_zero_effect_reward_request(&wrong).is_err());
    }
    let source = rlp::Rlp::new(&request.distribution_stats[0].data);
    let mut changed = rlp::RlpStream::new_list(6);
    for index in 0..6 {
        if index == 2 {
            changed
                .begin_list(1)
                .begin_list(2)
                .append(&H160::zero())
                .begin_list(3)
                .append(&0_u32)
                .append(&0_u64)
                .append(&U256::zero());
        } else {
            changed.append_raw(source.at(index)?.as_raw(), 1);
        }
    }
    let mut wrong = request;
    wrong.distribution_stats[0].data = changed.out().to_vec();
    assert!(validate_zero_effect_reward_request(&wrong).is_err());
    Ok(())
}
