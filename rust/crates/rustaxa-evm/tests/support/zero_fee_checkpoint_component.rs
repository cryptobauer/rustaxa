//! Actual driver/journal/typed-writer component; no mainnet admission claim.
//! Declared no-validator genesis is independent of Go output. Real owner
//! EndBlock executes after transaction roots. FinalChain publication is outside scope.
use super::*;
use rustaxa_consensus::{
    FinalChainRewardsConfig, FinalizedRewardsPeriodFact, GenesisAccount, GenesisDposConfig,
    RewardTransactionFact,
};
use rustaxa_evm::contracts::{
    ConsensusFailure, ExecutionGasPrice, ExecutionTransaction, ExecutionValue,
};
use rustaxa_evm::driver::execute_top_level_call;
use rustaxa_storage::{ConcreteStateMutationBatch, Config};
use rustaxa_types::codec::rlp::concrete_lifecycle::{
    concrete_state_bytes_digest, concrete_storage_slot_catalog_hash,
    encode_concrete_execution_marker, encode_concrete_state_provenance,
};
use rustaxa_types::concrete_lifecycle::{
    FinalChainConcreteExecutionMarker, FinalChainConcreteState, FinalChainConcreteStateProvenance,
};
use rustaxa_types::{FinalChainGas, FinalChainTransactionPosition};
use std::sync::Arc;

struct NoNative;
impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _: FinalChainBlockNumber, _: [u8; 20]) -> bool {
        false
    }
}
struct NoHash;
impl BlockHashRead for NoHash {
    fn block_hash(
        &self,
        p: FinalChainBlockNumber,
    ) -> std::result::Result<[u8; 32], BlockHashReadError> {
        Err(BlockHashReadError::HistoryUnavailable(p))
    }
}
fn address(n: u16) -> [u8; 20] {
    let mut a = [0; 20];
    a[18..].copy_from_slice(&n.to_be_bytes());
    a
}
fn facts(
    reader: &(impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead + ?Sized),
    addresses: &[[u8; 20]],
) -> Result<Vec<Value>> {
    addresses.iter().map(|a|Ok(match reader.account(*a)? {
        ConcreteRead::Present(row)=>serde_json::json!({"address":hex::encode(a),"present":true,"nonce":BigUint::from_bytes_be(&row.account.nonce.to_bytes()).to_string(),"balance":row.account.balance.value().to_string(),"raw_account":hex::encode(row.physical_rlp)}),
        ConcreteRead::Absent|ConcreteRead::Tombstone=>serde_json::json!({"address":hex::encode(a),"present":false}),
    })).collect()
}
fn run_case(name: &str, credit: bool) -> Result<Value> {
    let sender: [u8; 20] = hex::decode("1a642f0e3c3af545e7acbd38b07251b3990914f1")?
        .try_into()
        .unwrap();
    let author = address(0xfa1c);
    let recipient = address(0x0777);
    let dpos = address(0xfe);
    let addresses = [sender, author, recipient, dpos];
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent RUN_ARTIFACT_DIR required")?;
    let path = base.join(format!(
        "zero-fee-component-{name}-{credit}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    // Task-owned new path only; retain it for review, never open original DBs.
    std::fs::create_dir(&path)?;
    let genesis_batch = ConcreteStateMutationBatch {
        accounts: vec![
            ConcreteAccountMutation::Upsert {
                address: sender,
                record: record(
                    FinalChainNonce::zero(),
                    ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                    None,
                    0,
                ),
            },
            ConcreteAccountMutation::Upsert {
                address: dpos,
                record: record(
                    FinalChainNonce::from_u64(1),
                    ConcreteAccountBalance::new(BigUint::default()),
                    None,
                    0,
                ),
            },
        ],
        ..Default::default()
    };
    // Compute the declared genesis root without consulting either Go output or
    // the owner, then bind the owner and the actual writer to that exact root.
    let probe = ConcreteStateLifecycle::create_fresh_exclusive(
        path.join("genesis-probe"),
        [0x54; 32],
        genesis_batch.clone(),
        vec![],
    )?;
    let independent_genesis_root = probe.observation()?.committed.state_root;
    drop(probe);
    let storage = Arc::new(Storage::new(Config::new(path.join("application")))?);
    storage.metadata().set_genesis_hash_if_empty(&[0x54; 32])?;
    let owner = FinalChain::new_with_genesis_state_root(
        storage.clone(),
        1_000_000.into(),
        0,
        H256(independent_genesis_root),
        true,
        vec![GenesisAccount {
            address: sender,
            balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(1_000_000)),
        }],
        vec![],
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::from(1).into(),
            commission_change_delta: 0,
            commission_change_frequency: 0,
            delegation_delay: 1,
            dag_vdf_sortition_total_vote_count_until_period: FinalChainBlockNumber::GENESIS,
        },
        FinalChainRewardsConfig {
            committee_size: 0,
            magnolia_period: 10.into(),
            phalaenopsis_period: 0.into(),
            fix_claim_all_block_num: 0.into(),
            fix_redelegate_block_num: 0.into(),
            aspen_part_one_period: u64::MAX.into(),
            aspen_part_two_period: u64::MAX.into(),
            cornus_period: u64::MAX.into(),
            cacti_period: u64::MAX.into(),
            max_block_author_reward_percent: 10,
            dag_proposers_reward_percent: 50,
            yield_percentage: 0,
            dpos_blocks_per_year: 1,
            dpos_delegation_locking_period: 1,
            cornus_delegation_locking_period: 1,
            cacti_delegation_locking_period: 1,
            magnolia_jail_time: 1,
            cacti_jail_time: 1,
            genesis_balance_sum: Some(U256::from(1_000_000).into()),
            aspen_max_supply: U256::from(1_200_000).into(),
            aspen_generated_rewards: U256::zero().into(),
            rewards_distribution_frequency: vec![(0.into(), 1)],
            ..Default::default()
        },
    )?;
    let state_path = path.join("state-db");
    let mut concrete = ConcreteStateLifecycle::create_fresh_exclusive(
        &state_path,
        owner.concrete_chain_identity()?,
        genesis_batch,
        vec![],
    )?;
    let genesis = concrete.observation()?;
    ensure!(genesis.committed.state_root == independent_genesis_root);
    ensure!(owner.last_block_number()? == 0);
    let genesis_accounts = facts(concrete.prior_reader(), &addresses)?;
    let marker = FinalChainConcreteExecutionMarker {
        identity: genesis.identity,
        generation: 1,
        plan_hash: [0x51; 32],
        period: 1,
        prior_state: FinalChainConcreteState {
            period: 0,
            root: genesis.committed.state_root,
        },
        transactions_hash: [0x52; 32],
        rewards_hash: [0x53; 32],
    };
    let marker_rlp = encode_concrete_execution_marker(&marker);
    concrete.stage_execution(&marker_rlp)?;
    let block = ExecutionBlockContext {
        period: 1.into(),
        author,
        timestamp: 0,
        gas_limit: 1_000_000.into(),
        chain_id: 841,
        difficulty: BigUint::default(),
    };
    let count = if name == "second_value_to_author" {
        2
    } else {
        1
    };
    let mut prepared = None;
    let mut rows = vec![];
    let mut transaction_facts = vec![];
    for i in 0..count {
        let view = FixtureView {
            prior: match prepared.as_ref() {
                Some(p) => FixturePrior::Prepared(concrete.prepared_view(p)?),
                None => FixturePrior::Lifecycle(&concrete),
            },
            identity: prepared
                .as_ref()
                .map_or(genesis.committed, |p: &ConcreteObserverPhaseOutput| {
                    p.identity()
                }),
        };
        let mut journal = ExecutionJournal::new(view);
        let failure = name == "zero_gas_intrinsic_failure";
        let tx = ExecutionTransaction {
            position: FinalChainTransactionPosition::new(i),
            hash: [i as u8; 32],
            sender,
            receiver: Some(if i == 1 { author } else { recipient }),
            nonce: FinalChainNonce::from_u64(u64::from(i)),
            gas_price: ExecutionGasPrice::new(BigUint::from(u8::from(failure))),
            gas_limit: FinalChainGas::new(if failure { 0 } else { 21000 }),
            value: ExecutionValue::new(BigUint::from(u8::from(!failure))),
            input: vec![],
            canonical_rlp: None,
            kind: ExecutionTransactionKind::Call,
        };
        let result = execute_top_level_call(
            &mut journal,
            &NoHash,
            &NoNative,
            &block,
            &tx,
            EnvelopeRules { cornus: false },
            TaraxaProfile::new(false),
        )?;
        let (gas, consensus) = match result {
            TransactionExecutionResult::Executed(ref r) => {
                ensure!(!failure && r.status == CodeExecutionStatus::Success);
                (r.gas_used.as_u64(), None)
            }
            TransactionExecutionResult::ConsensusFailure(ref r) => {
                ensure!(failure && r.error == ConsensusFailure::IntrinsicGas);
                (r.gas_used.as_u64(), Some(format!("{:?}", r.error)))
            }
        };
        ensure!(gas == if failure { 0 } else { 21000 });
        // Actual executed result and declared price, not an invented zero fee.
        transaction_facts.push(RewardTransactionFact {
            hash: H256(tx.hash),
            gas_price: U256::from_big_endian(&tx.gas_price.value().to_bytes_be()),
            gas_used: gas.into(),
        });
        if credit {
            credit_c_composed_transaction_fee(&mut journal, author, tx.gas_price.value(), gas)?;
        }
        let transient = journal.account(author)?;
        let transient = serde_json::json!({"present":transient.exists,"nonce":BigUint::from_bytes_be(&transient.nonce.to_bytes()).to_string(),"balance":transient.balance.value().to_string()});
        let settled = journal.settle_transaction()?;
        drop(journal);
        ensure!(
            settled.writes.ordinary_storage.is_empty()
                && settled.writes.raw_storage.is_empty()
                && settled.writes.code.is_empty()
        );
        let account_events = settled.writes.accounts.iter().map(|row| serde_json::json!({"address":hex::encode(row.address),"operation":format!("{:?}",row.operation)})).collect::<Vec<_>>();
        if credit {
            ensure!(
                settled
                    .writes
                    .accounts
                    .iter()
                    .any(|row| row.address == author)
            );
        }
        let accounts = settled
            .writes
            .accounts
            .into_iter()
            .map(|row| match row.operation {
                JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                    address: row.address,
                },
                JournalAccountOperation::Upsert {
                    nonce,
                    balance,
                    code_hash,
                    code_size,
                } => ConcreteAccountMutation::Upsert {
                    address: row.address,
                    record: record(nonce, balance, code_hash, code_size),
                },
            })
            .collect();
        let next = concrete.apply_observer_phase(ConcreteObserverPhaseDelta {
            accounts,
            storage: vec![],
            code: vec![],
        })?;
        let actual = facts(&concrete.prepared_view(&next)?, &addresses)?;
        rows.push(serde_json::json!({"index":i,"gas_used":gas,"consensus_class":consensus,"after_c_credit":transient,"transaction_root":hex::encode(next.identity().state_root),"accounts":actual,"account_events":account_events}));
        prepared = Some(next);
    }
    let transactions_output = prepared.context("at least one actual transaction")?;
    let transaction_root = transactions_output.identity().state_root;
    let request_id = [0x59; 32];
    let fact = FinalizedRewardsPeriodFact {
        period: 1,
        block_author: H160(author),
        blocks_per_year: 1,
        dpos_eligible_total_vote_count: 0,
        transactions: transaction_facts,
        dag_blocks: vec![],
        cert_votes: vec![],
    };
    match owner.plan_synthetic_external_evm_rewards_stats(
        request_id,
        FinalizedRewardsPeriodFact {
            period: 2,
            ..fact.clone()
        },
    ) {
        Err(error) => ensure!(error.to_string() == "FINAL_CHAIN_EVM_REWARDS_HEAD_MISMATCH"),
        Ok(_) => bail!("wrong-period synthetic rewards plan was admitted"),
    }
    let plan = owner.plan_synthetic_external_evm_rewards_stats(request_id, fact)?;
    ensure!(
        owner.last_block_number()? == 0,
        "planning must not publish the head"
    );
    let view = FixtureView {
        prior: FixturePrior::Prepared(concrete.prepared_view(&transactions_output)?),
        identity: transactions_output.identity(),
    };
    let mut journal = ExecutionJournal::new(view);
    // A mismatched request must fail; a fresh matching session is independent.
    let mut wrong = MixedNativeExecutionPort::new(owner.begin_native_session_bound(
        [0x58; 32],
        1.into(),
        0.into(),
    )?);
    ensure!(matches!(wrong.finish(&plan, &journal),
        Err(rustaxa_evm::contracts::NativePortError::Domain(message))
            if message == "FinalChain native session: RewardsPlanMismatch"));
    drop(wrong);
    let mut native = MixedNativeExecutionPort::new(owner.begin_native_session_bound(
        request_id,
        1.into(),
        0.into(),
    )?);
    let (native_rewards, projected, reward_context) = native.finish(&plan, &journal)?;
    ensure!(projected.status == rustaxa_evm::contracts::NativeStatus::Success);
    apply_native_outcome(&mut journal, &projected)?;
    let settled = journal.settle_transaction()?;
    drop(journal);
    ensure!(settled.logs.is_empty() && settled.native_invocations.is_empty());
    let raw_events = projected
        .raw_mutations
        .iter()
        .map(|m| {
            serde_json::json!({"address":hex::encode(m.address),"key":hex::encode(m.key.0),
            "expected":format!("{:?}",m.expected),"operation":format!("{:?}",m.operation)})
        })
        .collect::<Vec<_>>();
    let account_events = settled.writes.accounts.iter().map(|m|
        serde_json::json!({"address":hex::encode(m.address),"operation":format!("{:?}",m.operation)}))
        .collect::<Vec<_>>();
    let accounts = settled
        .writes
        .accounts
        .into_iter()
        .map(|row| match row.operation {
            JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                address: row.address,
            },
            JournalAccountOperation::Upsert {
                nonce,
                balance,
                code_hash,
                code_size,
            } => ConcreteAccountMutation::Upsert {
                address: row.address,
                record: record(nonce, balance, code_hash, code_size),
            },
        })
        .collect();
    let mut storage = vec![];
    let mut catalog = BTreeSet::new();
    for write in settled.writes.ordinary_storage {
        catalog.insert(ConcreteStorageSlot {
            address: write.address,
            key: write.key.0,
        });
        storage.push(ConcreteStorageMutation {
            address: write.address,
            key: write.key,
            value: (write.value != BigUint::default()).then(|| write.value.to_bytes_be()),
        });
    }
    for write in settled.writes.raw_storage {
        catalog.insert(ConcreteStorageSlot {
            address: write.address,
            key: write.key.0,
        });
        storage.push(ConcreteStorageMutation {
            address: write.address,
            key: write.key,
            value: raw_operation_value(&write.operation),
        });
    }
    let code = settled
        .writes
        .code
        .into_iter()
        .map(|write| ConcreteCodeInsertion {
            code_hash: write.code_hash,
            code: write.code,
        })
        .collect();
    let output = concrete.apply_observer_phase(ConcreteObserverPhaseDelta {
        accounts,
        storage,
        code,
    })?;
    let public_final_accounts = facts(&concrete.prepared_view(&output)?, &addresses)?;
    ensure!(
        owner.last_block_number()? == 0,
        "component finish does not publish FinalChain"
    );
    let state = output.identity();
    let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
    let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
    // Component-only lifecycle approval, not FinalChain or production adoption.
    let projection_hash = concrete_state_bytes_digest(b"zero-fee real-owner EndBlock component");
    let provenance = FinalChainConcreteStateProvenance {
        identity: marker.identity,
        generation: 1,
        plan_hash: marker.plan_hash,
        committed_state: FinalChainConcreteState {
            period: 1,
            root: state.state_root,
        },
        transactions_hash: marker.transactions_hash,
        rewards_hash: marker.rewards_hash,
        projection_hash,
        catalog_hash,
    };
    let approval = ConcreteCommitApproval {
        marker_rlp,
        provenance_rlp: encode_concrete_state_provenance(&provenance),
        catalog_rlp,
        projection_hash,
        catalog_hash,
    };
    let committed = concrete.commit_observer_approved(output, approval)?;
    // commit consumes/drops every writable handle before this physical reader.
    let reopened = ConcreteStateReader::open_read_only(&state_path, committed.committed)?;
    let reopened_accounts = facts(&reopened, &addresses)?;
    match reopened.account(author)? {
        ConcreteRead::Present(row) => ensure!(
            name == "second_value_to_author"
                && row.account.balance.value() == &BigUint::from(1_u8)
                && row.account.nonce.is_zero()
        ),
        ConcreteRead::Absent | ConcreteRead::Tombstone => ensure!(name != "second_value_to_author"),
    }
    Ok(
        serde_json::json!({"component":"zero-fee-checkpoint-v1","case":name,"credit":credit,"scope":"REAL_OWNER_ENDBLOCK_COMPONENT_NOT_FINALCHAIN_PUBLICATION","genesis_root":hex::encode(genesis.committed.state_root),"genesis_accounts":genesis_accounts,"transactions":rows,"transaction_component_committed_root":hex::encode(transaction_root),"public_final_root":hex::encode(state.state_root),"public_final_accounts":public_final_accounts,"end_block_account_events":account_events,"end_block_raw_events":raw_events,"minted_reward":native_rewards.total_reward.as_u256().to_string(),"recorded_reward_context":format!("{:?}",reward_context),"reopened_accounts":reopened_accounts,"retained_task_db":path}),
    )
}
#[test]
fn zero_fee_actual_driver_typed_writer_and_real_owner_end_block() -> Result<()> {
    for case in [
        "zero_price_call",
        "second_value_to_author",
        "zero_gas_intrinsic_failure",
    ] {
        let credited = run_case(case, true)?;
        let control = run_case(case, false)?;
        ensure!(
            credited["genesis_root"] == control["genesis_root"]
                && credited["transaction_component_committed_root"]
                    == control["transaction_component_committed_root"]
                && credited["public_final_root"] == control["public_final_root"]
        );
        println!(
            "ZERO_FEE_RUST_EVIDENCE {}",
            serde_json::json!({"credited":credited,"omitted_control":control})
        );
    }
    Ok(())
}
