//! Check replay protection for the live reward scheduler.

use super::*;
use rustaxa_storage::{Config, Storage};
use rustaxa_types::GenesisValidatorMetadata;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
        crate::FinalChainGenesisAllocation {
            accounts: Vec::new(),
            validators: vec![
                validator(VALIDATOR_ONE, DELEGATOR_ONE, 1_000, 100),
                validator(VALIDATOR_TWO, DELEGATOR_TWO, 2_000, 2_500),
            ],
        },
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
    crate::test_paths::new_path(&format!("rustaxa-current-rewards-scheduler-{label}"))
}
