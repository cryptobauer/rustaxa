//! Joined Go-oracle checks for the process-local reward cleanup scheduler.
//!
//! These tests drive the real FinalChain scheduler basis through the native
//! terminal-rewards serializer and then arm/install its opaque successor. They
//! cover only the constructor-level committed-state reopen witnessed by the Go
//! fixtures. They do not model StateAPI discard durability, publication-fault
//! recovery, or a complete concrete-state publication.

use super::native_session::{
    FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeStateRead,
    FinalChainNativeStateReadError,
};
use super::reward_scheduler::{
    FinalChainRewardSchedulerInstall, FinalChainRewardSchedulerPublicationBinding,
    FinalChainRewardSchedulerRuntime,
};
use super::*;
use rustaxa_storage::{Config, Storage};
use rustaxa_types::GenesisValidatorMetadata;
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const VALIDATOR_ONE: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31,
];
const DELEGATOR_ONE: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x32,
];
const VALIDATOR_TWO: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x41,
];
const DELEGATOR_TWO: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x42,
];
const JAILED_VALIDATOR: [u8; 20] = [
    0x1a, 0x64, 0x2f, 0x0e, 0x3c, 0x3a, 0xf5, 0x45, 0xe7, 0xac, 0xbd, 0x38, 0xb0, 0x72, 0x51, 0xb3,
    0x99, 0x09, 0x14, 0xf1,
];

type RawRows = BTreeMap<([u8; 20], [u8; 32]), ConcreteRead<Vec<u8>>>;

/// Exact logical raw state consumed and updated by the terminal native phase.
struct RewardRawState {
    rows: RefCell<RawRows>,
}

impl RewardRawState {
    fn from_snapshot(snapshot: &DposSnapshot) -> Self {
        let rows = canonical_concrete_precompile_storage(snapshot, true)
            .expect("fixture snapshot has one canonical raw encoding")
            .into_iter()
            .map(|(identity, values)| {
                (
                    identity,
                    ConcreteRead::Present(
                        values
                            .into_iter()
                            .next()
                            .expect("canonical raw row has one encoding"),
                    ),
                )
            })
            .collect();
        Self {
            rows: RefCell::new(rows),
        }
    }

    fn apply(&self, mutations: &[FinalChainNativeRawMutation]) {
        let mut rows = self.rows.borrow_mut();
        for mutation in mutations {
            let value = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => {
                    ConcreteRead::Present(value.as_bytes().to_vec())
                }
                FinalChainNativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
            };
            rows.insert((mutation.address, mutation.key.0), value);
        }
    }

    fn row(&self, address: [u8; 20], key: [u8; 32]) -> ConcreteRead<Vec<u8>> {
        self.rows
            .borrow()
            .get(&(address, key))
            .cloned()
            .unwrap_or(ConcreteRead::Absent)
    }
}

impl FinalChainNativeStateRead for RewardRawState {
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        Ok(self.row(address, key.0))
    }
}

#[test]
fn reopened_cleanup_scheduler_matches_both_go_pins_through_native_rewards_and_install() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/current_rewards_local.json"
    )))
    .unwrap();

    for (label, fixture) in [("public", &public), ("local", &local)] {
        run_reopen_fixture(
            label,
            &fixture["jailed_validator_cleanup"]["reopened_scheduler"],
        );
    }
}

#[test]
fn duplicate_old_publication_does_not_rewind_live_account_snapshot() {
    let path = temp_db_path("old-duplicate-publication");
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = current_reward_chain(storage.clone());

    let period_one_dpos = chain
        .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
        .unwrap();
    let period_two_dpos = period_one_dpos.clone();
    let period_one_accounts = HashMap::from([([0x61; 20], empty_account())]);
    let period_two_accounts = HashMap::from([([0x62; 20], empty_account())]);
    let period_one_dpos_rlp = encode_dpos_snapshot_rlp(&period_one_dpos).unwrap();
    let period_two_dpos_rlp = encode_dpos_snapshot_rlp(&period_two_dpos).unwrap();
    let period_one_accounts_rlp = encode_account_snapshot_rlp(&period_one_accounts);
    let period_two_accounts_rlp = encode_account_snapshot_rlp(&period_two_accounts);
    let (period_one_hash, period_one_header) = stored_test_header(1);
    let (period_two_hash, period_two_header) = stored_test_header(2);

    storage
        .final_chain()
        .write_block_header_with_snapshots(
            1,
            period_one_hash,
            period_one_header.as_bytes(),
            &[0xc0],
            Some(&period_one_dpos_rlp),
            Some(&period_one_accounts_rlp),
        )
        .unwrap();
    storage
        .final_chain()
        .write_block_header_with_snapshots(
            2,
            period_two_hash,
            period_two_header.as_bytes(),
            &[0xc0],
            Some(&period_two_dpos_rlp),
            Some(&period_two_accounts_rlp),
        )
        .unwrap();
    chain
        .insert_dpos_snapshot(1.into(), period_one_dpos)
        .unwrap();
    chain
        .insert_dpos_snapshot(2.into(), period_two_dpos)
        .unwrap();
    chain
        .insert_account_snapshot(1.into(), period_one_accounts.clone())
        .unwrap();
    chain
        .insert_account_snapshot(2.into(), period_two_accounts.clone())
        .unwrap();
    assert_eq!(
        chain.current_account_snapshot().unwrap(),
        period_two_accounts
    );

    let request_id = [0x71; 32];
    let mut plan = FinalChainExternalEvmPublicationPlan {
        request_id,
        period: 1.into(),
        block_hash: period_one_hash.into(),
        stored_header_rlp: period_one_header.as_bytes().to_vec(),
        receipts_rlp: vec![0xc0],
        dpos_snapshot_rlp: period_one_dpos_rlp,
        account_snapshot_rlp: period_one_accounts_rlp,
        ..Default::default()
    };
    plan.plan_id = final_chain_external_evm_publication_plan_id(&plan);
    let decision = FinalChainExternalEvmCommitDecision {
        request_id,
        plan_id: plan.plan_id,
        decision_id: final_chain_external_evm_commit_decision_id(
            request_id,
            plan.plan_id,
            plan.period,
            plan.block_hash,
        ),
        period: plan.period,
        publication_block_hash: plan.block_hash,
        status: FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH,
        error_code: String::new(),
    };

    let report = chain
        .publish_external_evm_publication(plan, decision)
        .unwrap();
    assert_eq!(
        report.status,
        FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
    );
    assert!(report.error_code.is_empty());
    assert_eq!(chain.last_block_number_typed().unwrap(), 2.into());
    assert_eq!(
        chain.current_account_snapshot().unwrap(),
        period_two_accounts
    );

    drop(chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
}

fn run_reopen_fixture(label: &str, fixture: &Value) {
    let path = temp_db_path(label);
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = current_reward_chain(storage.clone());

    let mut period_one = chain
        .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
        .unwrap();
    period_one.slashing_jail_blocks.insert(JAILED_VALIDATOR, 5);
    period_one.slashing_jailed_validators.push(JAILED_VALIDATOR);
    chain
        .insert_dpos_snapshot(FinalChainBlockNumber::GENESIS, period_one.clone())
        .unwrap();
    publish_test_parent(&chain, 1, period_one.clone());

    // The Go witness starts this slice after period 1 committed the proof rows.
    // Pair the real FinalChain scheduler with the same committed parent and a
    // live StateAPI epoch; its initial timer is constructor-zero.
    *chain.reward_scheduler_runtime.lock().unwrap() =
        FinalChainRewardSchedulerRuntime::new(1.into());
    chain
        .authorize_reward_scheduler_joint_startup(11, 1.into())
        .unwrap();
    chain.complete_reward_scheduler_recovery(11).unwrap();

    let state = RewardRawState::from_snapshot(&period_one);
    run_period(&chain, &state, fixture, 2, 11);

    // A verified StateAPI replacement resets only the process-local timer.
    chain
        .record_reward_scheduler_verified_discard(11, 12, 2.into())
        .unwrap();
    let reopened_descriptor = chain.committed_state_descriptor().unwrap();
    assert!(
        chain
            .complete_reward_scheduler_verified_reopen(12, reopened_descriptor)
            .unwrap()
    );
    chain
        .reward_scheduler_runtime
        .lock()
        .unwrap()
        .complete_verified_discard_recovery(12)
        .unwrap();
    run_period(&chain, &state, fixture, 3, 12);
    run_period(&chain, &state, fixture, 4, 12);

    drop(chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
}

fn run_period(
    chain: &FinalChain,
    state: &RewardRawState,
    fixture: &Value,
    period: u64,
    state_api_epoch: u64,
) {
    let name = match period {
        2 => "period_2_live_cleanup",
        3 => "period_3_after_reopen",
        4 => "period_4_live_cleanup",
        _ => unreachable!("bounded fixture covers periods 2 through 4"),
    };
    let expected = &fixture[name];
    assert_eq!(expected["period"].as_u64(), Some(period));
    assert_rows_match_fixture(state, &expected["before_end_block"]);

    let request_id = [period as u8; 32];
    let expected_parent = FinalChainBlockNumber::new(period - 1);
    let pending_period = FinalChainBlockNumber::new(period);
    let mut session = chain
        .begin_native_session_bound_at_state_api_epoch(
            request_id,
            state_api_epoch,
            pending_period,
            expected_parent,
        )
        .unwrap();
    let generation = chain.rewards_stats_runtime.lock().unwrap().generation;
    let reward_plan = empty_reward_plan(request_id, pending_period, expected_parent, generation);
    let outcome = session.finish_rewards(&reward_plan, state).unwrap();

    assert_exact_raw_writes(
        &outcome.raw_mutations,
        &expected["ordered_raw_writes"],
        &expected["before_end_block"],
    );
    assert_eq!(
        outcome.dpos_snapshot.slashing_jailed_validators,
        vec![JAILED_VALIDATOR]
    );
    assert_eq!(
        outcome.dpos_snapshot.slashing_jail_blocks[&JAILED_VALIDATOR],
        5
    );
    let successor = outcome
        .scheduler_successor
        .expect("scheduler-bound terminal rewards always return a successor");
    let binding = publication_binding(request_id, expected_parent, pending_period, state_api_epoch);

    // This is the scheduler's real publication transition. The test publishes
    // only the in-memory snapshot/header facts needed by the next bounded
    // session; it makes no durable StateAPI or fault-recovery claim.
    {
        let mut scheduler = chain.reward_scheduler_runtime.lock().unwrap();
        scheduler.arm(successor, binding).unwrap();
        state.apply(&outcome.raw_mutations);
        publish_test_parent(chain, period, outcome.dpos_snapshot);
        assert_eq!(
            scheduler.install_applied(binding).unwrap(),
            FinalChainRewardSchedulerInstall::Installed
        );
    }
    assert_rows_match_fixture(state, &expected["after"]["slots"]);
}

fn assert_exact_raw_writes(
    actual: &[FinalChainNativeRawMutation],
    expected: &Value,
    before: &Value,
) {
    let expected = expected
        .as_array()
        .expect("fixture ordered_raw_writes is an array");
    assert_eq!(actual.len(), expected.len());
    for (mutation, row) in actual.iter().zip(expected) {
        let value = match &mutation.operation {
            FinalChainNativeRawOperation::Put(value) => value.as_bytes(),
            FinalChainNativeRawOperation::Delete => &[],
        };
        assert_eq!(hex(mutation.address), row["address"].as_str().unwrap());
        assert_eq!(hex(mutation.key.0), row["key"].as_str().unwrap());
        assert_eq!(hex(value), row["value"].as_str().unwrap());
        let prior = before
            .as_array()
            .unwrap()
            .iter()
            .find(|prior| prior["key"] == row["key"])
            .expect("every cleanup write has an authenticated prior row");
        let prior = if prior["present"].as_bool().unwrap() {
            ConcreteRead::Present(hex_decode(prior["value"].as_str().unwrap()))
        } else {
            ConcreteRead::Absent
        };
        assert_eq!(mutation.expected, prior);
    }
}

fn assert_rows_match_fixture(state: &RewardRawState, rows: &Value) {
    for row in rows.as_array().expect("fixture slots are an array") {
        let key = fixed_hex::<32>(row["key"].as_str().unwrap());
        let actual = state.row(SLASHING_CONTRACT_ADDRESS, key);
        let expected = if row["present"].as_bool().unwrap() {
            ConcreteRead::Present(hex_decode(row["value"].as_str().unwrap()))
        } else {
            ConcreteRead::Absent
        };
        assert_eq!(actual, expected, "fixture row {}", row["name"]);
    }
}

fn current_reward_chain(storage: Arc<Storage>) -> FinalChain {
    let validator = |address, delegator, stake: u64, commission| GenesisValidator {
        address,
        vrf_key: [0; 32],
        total_stake: U256::from(stake).to_big_endian().to_vec(),
        delegations: vec![(delegator, U256::from(stake).to_big_endian().to_vec())],
        metadata: GenesisValidatorMetadata {
            owner: delegator,
            commission,
            ..Default::default()
        },
    };
    FinalChain::new_with_rewards_config_and_ficus_activation(
        storage,
        1_000_000.into(),
        0,
        Vec::new(),
        vec![
            validator(VALIDATOR_ONE, DELEGATOR_ONE, 1_000, 100),
            validator(VALIDATOR_TWO, DELEGATOR_TWO, 2_000, 2_500),
        ],
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::one().into(),
            ..Default::default()
        },
        FinalChainRewardsConfig {
            committee_size: 10,
            magnolia_period: FinalChainBlockNumber::GENESIS,
            aspen_part_one_period: FinalChainBlockNumber::GENESIS,
            aspen_part_two_period: 1.into(),
            fix_redelegate_block_num: FinalChainBlockNumber::MAX,
            max_block_author_reward_percent: 10,
            dag_proposers_reward_percent: 50,
            yield_percentage: 1,
            dpos_blocks_per_year: 10,
            cornus_period: FinalChainBlockNumber::GENESIS,
            genesis_balance_sum: Some(DposTokenAmount::from(U256::from(5_000))),
            aspen_max_supply: DposTokenAmount::from(U256::from(6_000)),
            aspen_generated_rewards: DposTokenAmount::zero(),
            cacti_period: FinalChainBlockNumber::GENESIS,
            magnolia_jail_time: 4,
            cacti_jail_time: 4,
            rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
            ..Default::default()
        },
        FinalChainBlockNumber::GENESIS,
    )
    .unwrap()
}

fn empty_reward_plan(
    request_id: [u8; 32],
    period: FinalChainBlockNumber,
    expected_parent: FinalChainBlockNumber,
    expected_runtime_generation: u64,
) -> FinalChainPreparedExternalEvmRewardsStatsPlan {
    FinalChainPreparedExternalEvmRewardsStatsPlan {
        request_id,
        period,
        expected_prior_head: expected_parent,
        expected_runtime_generation,
        distribution_stats: Vec::new(),
        storage_update: FinalChainExternalEvmRewardsStatsUpdate {
            current_period: period,
            ..Default::default()
        },
    }
}

fn publication_binding(
    request_id: [u8; 32],
    expected_parent: FinalChainBlockNumber,
    period: FinalChainBlockNumber,
    state_api_epoch: u64,
) -> FinalChainRewardSchedulerPublicationBinding {
    FinalChainRewardSchedulerPublicationBinding {
        request_id,
        expected_parent,
        period,
        state_api_epoch,
        concrete_database_id: [0x31; 32],
        concrete_generation: period.as_u64(),
        concrete_projection_hash: [period.as_u64() as u8; 32],
        publication_plan_id: [0x51 ^ period.as_u64() as u8; 32],
    }
}

fn publish_test_parent(chain: &FinalChain, period: u64, snapshot: DposSnapshot) {
    chain.insert_dpos_snapshot(period.into(), snapshot).unwrap();
    let (hash, raw_header) = stored_test_header(period);
    chain
        .storage
        .final_chain()
        .write_block_header(period, hash, raw_header.as_bytes(), &[0xc0])
        .unwrap();
    let mut rewards = chain.rewards_stats_runtime.lock().unwrap();
    rewards.durable_head = period.into();
    rewards.generation = rewards.generation.checked_add(1).unwrap();
}

fn stored_test_header(period: u64) -> (H256, StoredBlockHeaderRlpOwned) {
    let stored_header = StoredFinalChainBlockHeader {
        parent_hash: H256::zero(),
        state_root: H256::from_low_u64_be(period),
        transactions_root: empty_trie_root(),
        receipts_root: empty_trie_root(),
        log_bloom: FinalChainLogBloom::ZERO,
        gas_used: FinalChainGas::ZERO,
        total_reward: DposTokenAmount::zero(),
    };
    let raw_header = StoredBlockHeaderRlpOwned::from(&stored_header);
    (H256::from_low_u64_be(period), raw_header)
}

fn temp_db_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustaxa-current-rewards-scheduler-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_decode(value: &str) -> Vec<u8> {
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|chunk| u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap())
        .collect()
}

fn fixed_hex<const N: usize>(value: &str) -> [u8; N] {
    hex_decode(value)
        .try_into()
        .unwrap_or_else(|_| panic!("fixture hexadecimal value is not {N} bytes"))
}
