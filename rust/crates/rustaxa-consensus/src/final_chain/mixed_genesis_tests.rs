//! Cross-language witness for the bounded mixed-period genesis and rewards fixture.
//!
//! The fixture starts from the public Go `StateTransition` genesis path. Rust's
//! FinalChain constructor accepts the effective balances left after genesis
//! delegations, so this test derives those balances from the original Go
//! allocations before constructing the Rust snapshot. It then compares the
//! complete observed Go DPoS raw catalog with Rust's canonical raw candidates
//! and feeds transaction, DAG, and certificate facts through the Rust rewards
//! planner. The test also pins the remaining genesis hydration boundary: the
//! current FinalChain account model reproduces nonce and balance semantics but
//! does not yet install the Go Cornus DPoS bytecode or authenticated storage
//! root in its genesis account.

use super::*;
use ethereum_types::H160;
use rustaxa_storage::Config;
use rustaxa_types::{FinalChainAccountBalance, GenesisValidatorMetadata};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const PUBLIC_BATCHED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../experiments/evm_feasibility/fixtures/mixed_public_batched.json"
));
const LOCAL_BATCHED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../experiments/evm_feasibility/fixtures/mixed_local_batched.json"
));
const LOCAL_OBSERVER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../experiments/evm_feasibility/fixtures/mixed_local_observer.json"
));

fn parse_fixture(source: &str) -> Value {
    serde_json::from_str(source).expect("mixed-period fixture is valid JSON")
}

fn string<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} must be a string"))
}

fn decimal(value: &Value, field: &str) -> U256 {
    U256::from_dec_str(string(value, field))
        .unwrap_or_else(|error| panic!("{field} must be a uint256: {error}"))
}

fn fixed_hex<const N: usize>(value: &Value, field: &str) -> [u8; N] {
    fixed_hex_str(string(value, field), field)
}

fn fixed_hex_str<const N: usize>(raw: &str, field: &str) -> [u8; N] {
    let bytes = hex_bytes(raw, field);
    bytes.try_into().unwrap_or_else(|bytes: Vec<u8>| {
        panic!("{field} must contain {N} bytes, got {}", bytes.len())
    })
}

fn hex_bytes(raw: &str, field: &str) -> Vec<u8> {
    assert!(
        raw.len().is_multiple_of(2),
        "{field} must have even hex width"
    );
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex pairs are UTF-8");
            u8::from_str_radix(pair, 16)
                .unwrap_or_else(|error| panic!("{field} must be hex: {error}"))
        })
        .collect()
}

fn temp_db_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time follows the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustaxa-consensus-mixed-genesis-{}-{nanos}",
        std::process::id()
    ))
}

fn account_balance(value: U256) -> FinalChainAccountBalance {
    FinalChainAccountBalance::from_cpp_genesis_bytes(&value.to_big_endian())
        .expect("fixture balance fits the fixed genesis boundary")
}

fn expected_genesis_accounts(genesis: &Value) -> BTreeMap<[u8; 20], (u64, U256)> {
    genesis["accounts"]
        .as_array()
        .expect("genesis accounts are an array")
        .iter()
        .filter(|row| row["present"] == true)
        .map(|row| {
            (
                fixed_hex(row, "address"),
                (
                    string(row, "nonce")
                        .parse()
                        .expect("account nonce fits u64"),
                    decimal(row, "balance"),
                ),
            )
        })
        .collect()
}

#[test]
fn go_genesis_catalog_and_reward_facts_match_rust_native_semantics() {
    let public = parse_fixture(PUBLIC_BATCHED);
    let local = parse_fixture(LOCAL_BATCHED);
    let oracle = parse_fixture(LOCAL_OBSERVER);

    for peer in [&public, &local] {
        assert_eq!(peer["configuration"], oracle["configuration"]);
        assert_eq!(peer["inputs"], oracle["inputs"]);
        assert_eq!(peer["genesis"]["root"], oracle["genesis"]["root"]);
        assert_eq!(
            peer["genesis"]["native_storage_by_hashed_path"],
            oracle["genesis"]["native_storage_by_hashed_path"]
        );
        assert_eq!(
            peer["period"]["reward_input"],
            oracle["period"]["reward_input"]
        );
        assert_eq!(
            peer["period"]["reward_output"],
            oracle["period"]["reward_output"]
        );
    }

    let inputs = &oracle["inputs"];
    let configuration = &oracle["configuration"];
    let dpos = &configuration["dpos"];
    let genesis = &oracle["genesis"];
    let initial_validator = &inputs["initial_validator"];
    let validator: [u8; 20] = fixed_hex(initial_validator, "address");
    let owner: [u8; 20] = fixed_hex(initial_validator, "owner");

    // Go's input allocation is before ApplyGenesis debits each delegation.
    // FinalChain deliberately accepts the effective post-debit allocation.
    let original_allocations = inputs["genesis_allocations"]
        .as_array()
        .expect("genesis allocations are an array")
        .iter()
        .map(|row| (fixed_hex(row, "address"), decimal(row, "balance")))
        .collect::<BTreeMap<[u8; 20], U256>>();
    let mut effective_allocations = original_allocations.clone();
    let delegations = initial_validator["delegations"]
        .as_array()
        .expect("validator delegations are an array")
        .iter()
        .map(|row| (fixed_hex(row, "delegator"), decimal(row, "amount")))
        .collect::<Vec<([u8; 20], U256)>>();
    let total_stake = delegations
        .iter()
        .try_fold(U256::zero(), |total, (_, amount)| {
            total.checked_add(*amount)
        })
        .expect("genesis stake sum fits uint256");
    for (delegator, amount) in &delegations {
        let balance = effective_allocations
            .get_mut(delegator)
            .expect("every delegator has a genesis allocation");
        *balance = balance
            .checked_sub(*amount)
            .expect("genesis allocation covers delegation");
    }

    let expected_accounts = expected_genesis_accounts(genesis);
    let delegator: [u8; 20] = fixed_hex(inputs, "delegator");
    assert_eq!(original_allocations[&delegator], U256::from(2_000u64));
    assert_eq!(effective_allocations[&delegator], U256::from(1_000u64));
    assert_eq!(
        expected_accounts[&delegator].1,
        effective_allocations[&delegator]
    );

    let genesis_accounts = effective_allocations
        .iter()
        .map(|(address, balance)| GenesisAccount {
            address: *address,
            balance: account_balance(*balance),
        })
        .collect::<Vec<_>>();
    let genesis_validator = GenesisValidator {
        address: validator,
        vrf_key: fixed_hex(initial_validator, "vrf_key"),
        total_stake: total_stake.to_big_endian().to_vec(),
        delegations: delegations
            .iter()
            .map(|(delegator, amount)| (*delegator, amount.to_big_endian().to_vec()))
            .collect(),
        metadata: GenesisValidatorMetadata {
            owner,
            commission: initial_validator["commission"]
                .as_u64()
                .expect("commission is an integer")
                .try_into()
                .expect("commission fits uint16"),
            description: String::new(),
            endpoint: String::new(),
        },
    };
    let dpos_config = GenesisDposConfig {
        eligibility_balance_threshold: decimal(dpos, "eligibility_balance_threshold").into(),
        vote_eligibility_balance_step: decimal(dpos, "vote_eligibility_balance_step").into(),
        validator_maximum_stake: decimal(dpos, "validator_maximum_stake").into(),
        minimum_deposit: decimal(dpos, "minimum_deposit").into(),
        commission_change_delta: dpos["commission_change_delta"]
            .as_u64()
            .expect("commission delta is an integer")
            .try_into()
            .expect("commission delta fits uint16"),
        commission_change_frequency: dpos["commission_change_frequency"]
            .as_u64()
            .expect("commission frequency is an integer")
            .try_into()
            .expect("commission frequency fits uint32"),
        delegation_delay: dpos["delegation_delay"]
            .as_u64()
            .expect("delegation delay is an integer"),
        dag_vdf_sortition_total_vote_count_until_period: FinalChainBlockNumber::GENESIS,
    };
    let hardforks = &configuration["hardforks"];
    let rewards_config = FinalChainRewardsConfig {
        committee_size: configuration["committee_size"]
            .as_u64()
            .expect("committee size is an integer")
            .try_into()
            .expect("committee size fits uint32"),
        magnolia_period: hardforks["magnolia"]["block"]
            .as_u64()
            .expect("Magnolia period is an integer")
            .into(),
        phalaenopsis_period: hardforks["phalaenopsis_block"]
            .as_u64()
            .expect("Phalaenopsis period is an integer")
            .into(),
        aspen_part_one_period: hardforks["aspen"]["part_one_block"]
            .as_u64()
            .expect("Aspen part one period is an integer")
            .into(),
        fix_claim_all_block_num: hardforks["fix_claim_all_block"]
            .as_u64()
            .expect("fix-claim-all period is an integer")
            .into(),
        fix_redelegate_block_num: hardforks["fix_redelegate_block"]
            .as_u64()
            .expect("fix-redelegate period is an integer")
            .into(),
        aspen_part_two_period: string(&hardforks["aspen"], "part_two_block")
            .parse::<u64>()
            .expect("Aspen part two period fits u64")
            .into(),
        max_block_author_reward_percent: dpos["max_block_author_reward_percent"]
            .as_u64()
            .expect("author reward is an integer")
            .try_into()
            .expect("author reward fits uint16"),
        dag_proposers_reward_percent: dpos["dag_proposers_reward_percent"]
            .as_u64()
            .expect("DAG reward is an integer")
            .try_into()
            .expect("DAG reward fits uint16"),
        yield_percentage: dpos["yield_percentage"]
            .as_u64()
            .expect("yield is an integer")
            .try_into()
            .expect("yield fits uint16"),
        dpos_blocks_per_year: dpos["blocks_per_year"]
            .as_u64()
            .expect("blocks per year is an integer")
            .try_into()
            .expect("blocks per year fits uint32"),
        dpos_delegation_locking_period: dpos["delegation_locking_period"]
            .as_u64()
            .expect("base locking period is an integer"),
        cornus_period: hardforks["cornus"]["block"]
            .as_u64()
            .expect("Cornus period is an integer")
            .into(),
        cornus_delegation_locking_period: hardforks["cornus"]["delegation_locking_period"]
            .as_u64()
            .expect("Cornus locking period is an integer"),
        genesis_balance_sum: Some(
            original_allocations
                .values()
                .try_fold(U256::zero(), |total, balance| total.checked_add(*balance))
                .expect("genesis allocation sum fits uint256")
                .into(),
        ),
        aspen_max_supply: decimal(&hardforks["aspen"], "max_supply").into(),
        aspen_generated_rewards: decimal(&hardforks["aspen"], "generated_rewards").into(),
        cacti_period: string(&hardforks["cacti"], "block")
            .parse::<u64>()
            .expect("Cacti period fits u64")
            .into(),
        cacti_delegation_locking_period: hardforks["cacti"]["delegation_locking_period"]
            .as_u64()
            .expect("Cacti locking period is an integer"),
        magnolia_jail_time: hardforks["magnolia"]["jail_time"]
            .as_u64()
            .expect("Magnolia jail time is an integer"),
        cacti_jail_time: hardforks["cacti"]["jail_time"]
            .as_u64()
            .expect("Cacti jail time is an integer"),
        rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
        ..Default::default()
    };

    let path = temp_db_path();
    let storage = Arc::new(Storage::new(Config::new(path.clone())).expect("create temporary DB"));
    let final_chain = FinalChain::new_with_rewards_config(
        storage.clone(),
        configuration["block_gas_limit"]
            .as_u64()
            .expect("block gas limit is an integer")
            .into(),
        0,
        genesis_accounts,
        vec![genesis_validator],
        dpos_config,
        rewards_config,
    )
    .expect("construct Rust FinalChain from effective genesis inputs");

    let rust_accounts = final_chain.accounts.lock().expect("account lock");
    assert_eq!(rust_accounts.len(), expected_accounts.len());
    for (address, (expected_nonce, expected_balance)) in &expected_accounts {
        let actual = rust_accounts
            .get(address)
            .expect("Go-present semantic account exists in Rust");
        assert_eq!(actual.nonce.as_u64(), Some(*expected_nonce));
        assert_eq!(*actual.balance.as_u256(), *expected_balance);
    }

    // This deliberately records the exact M1 boundary rather than hiding it
    // behind the nonce/balance comparison. Go installs the Cornus DPoS code
    // and authenticates its raw slots at genesis; FinalChain's typed genesis
    // account currently has no code/root inputs.
    let go_dpos = genesis["accounts"]
        .as_array()
        .expect("genesis accounts are an array")
        .iter()
        .find(|row| fixed_hex::<20>(row, "address") == DPOS_CONTRACT_ADDRESS)
        .expect("Go fixture contains the DPoS account");
    let rust_dpos = &rust_accounts[&DPOS_CONTRACT_ADDRESS];
    assert!(go_dpos["code_size"].as_u64().expect("Go code size") > 0);
    assert!(go_dpos["code_hash"].as_str().is_some());
    assert!(go_dpos["storage_root"].as_str().is_some());
    assert_eq!(rust_dpos.code_size, 0);
    assert_eq!(rust_dpos.code_hash, [0; 32]);
    assert_eq!(rust_dpos.storage_root_hash, [0; 32]);
    drop(rust_accounts);

    let snapshot = final_chain
        .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
        .expect("Rust genesis DPoS snapshot");
    assert_eq!(snapshot.total_stakes[&validator].as_u256(), total_stake);
    assert_eq!(
        snapshot.delegations[&validator][&delegator].as_u256(),
        delegations[0].1
    );
    assert_eq!(
        snapshot.total_vote_count,
        oracle["period"]["planner_facts"]["total_eligible_vote_count"]
            .as_u64()
            .expect("eligible vote count is an integer")
    );
    assert_eq!(
        snapshot.vote_counts[&validator],
        oracle["period"]["planner_facts"]["validator_eligible_vote_count"]
            .as_u64()
            .expect("validator vote count is an integer")
    );
    assert_eq!(
        snapshot.vote_counts.len() as u64,
        oracle["period"]["planner_facts"]["eligible_validator_count"]
            .as_u64()
            .expect("eligible validator count is an integer")
    );
    assert_eq!(
        snapshot.total_vote_count,
        oracle["period"]["reward_input"]["eligible_vote_count"]
            .as_u64()
            .expect("reward input vote count is an integer")
    );

    let canonical = canonical_concrete_precompile_storage(&snapshot, true)
        .expect("canonical Rust genesis raw candidates");
    let go_slots = genesis["native_catalog"]["slots"]
        .as_array()
        .expect("observer genesis catalog slots are an array")
        .iter()
        .map(|row| {
            assert_eq!(row["present"], true);
            (
                (fixed_hex(row, "address"), fixed_hex(row, "key")),
                hex_bytes(string(row, "value"), "catalog value"),
            )
        })
        .collect::<BTreeMap<([u8; 20], [u8; 32]), Vec<u8>>>();
    assert_eq!(go_slots.len(), 15);
    let mismatches = go_slots
        .iter()
        .filter(|(key, value)| {
            !canonical
                .get(key)
                .is_some_and(|candidates| candidates.contains(value))
        })
        .map(|(key, value)| (*key, value.clone(), canonical.get(key).cloned()))
        .collect::<Vec<_>>();
    assert!(
        mismatches.is_empty(),
        "raw candidate mismatches: {mismatches:?}"
    );
    let rust_live_dpos_keys = canonical
        .iter()
        .filter(|((contract, _), candidates)| {
            *contract == DPOS_CONTRACT_ADDRESS
                && candidates.iter().any(|candidate| !candidate.is_empty())
        })
        .map(|(key, _)| *key)
        .collect::<BTreeSet<_>>();
    assert_eq!(rust_live_dpos_keys, go_slots.keys().copied().collect());

    let period = &oracle["period"];
    let planner_facts = &period["planner_facts"];
    let transaction = &period["transaction"];
    let reward_input = &period["reward_input"];
    let transaction_hash = H256::from(fixed_hex::<32>(transaction, "hash"));
    let block_author = H160::from(fixed_hex::<20>(reward_input, "block_author"));
    let transaction_fact = RewardTransactionFact {
        hash: transaction_hash,
        gas_price: decimal(transaction, "gas_price"),
        gas_used: transaction["gas_used"]
            .as_u64()
            .expect("transaction gas used is an integer")
            .into(),
    };
    let validator_facts = reward_input["validators"]
        .as_array()
        .expect("reward validators are an array");
    let dag_blocks = planner_facts["dag_blocks"]
        .as_array()
        .expect("planner DAG blocks are an array")
        .iter()
        .map(|block| RewardDagBlockFact {
            author: H160::from(fixed_hex::<20>(block, "author")),
            difficulty: string(block, "difficulty")
                .parse()
                .expect("DAG difficulty fits uint16"),
            transaction_hashes: block["transaction_hashes"]
                .as_array()
                .expect("DAG transaction hashes are an array")
                .iter()
                .map(|hash| {
                    H256::from(fixed_hex_str::<32>(
                        hash.as_str().expect("DAG transaction hash is a string"),
                        "DAG transaction hash",
                    ))
                })
                .collect(),
        })
        .collect();
    let cert_votes = planner_facts["certificate_votes"]
        .as_array()
        .expect("planner certificate votes are an array")
        .iter()
        .map(|vote| RewardCertVoteFact {
            voter: H160::from(fixed_hex::<20>(vote, "validator")),
            weight: vote["weight"].as_u64().expect("vote weight is an integer"),
            period: vote["period"].as_u64().expect("vote period is an integer"),
        })
        .collect();
    let fact = FinalizedRewardsPeriodFact {
        period: period["number"]
            .as_u64()
            .expect("period number is an integer"),
        block_author,
        blocks_per_year: reward_input["blocks_per_year"]
            .as_u64()
            .expect("blocks per year is an integer")
            .try_into()
            .expect("blocks per year fits uint32"),
        dpos_eligible_total_vote_count: snapshot.total_vote_count,
        transactions: vec![transaction_fact],
        dag_blocks,
        cert_votes,
    };
    let rewards_plan = final_chain
        .plan_external_evm_rewards_stats([0x4d; 32], fact)
        .expect("Rust rewards-stats planner accepts period one facts");
    let distributions = decode_rewards_block_distributions(&rewards_plan.distribution_stats)
        .expect("decode Rust-planned distribution stats");
    assert_eq!(distributions.len(), 1);
    let distribution = &distributions[0];
    assert_eq!(distribution.period, period["number"].as_u64().unwrap());
    assert_eq!(distribution.block_author, block_author);
    assert_eq!(
        distribution.total_dag_blocks_count,
        reward_input["total_dag_blocks_count"].as_u64().unwrap() as u32
    );
    assert_eq!(
        distribution.total_votes_weight,
        reward_input["total_votes_weight"].as_u64().unwrap()
    );
    assert_eq!(
        distribution.max_votes_weight,
        reward_input["max_votes_weight"].as_u64().unwrap()
    );
    let expected_validator_keys = validator_facts
        .iter()
        .map(|expected| fixed_hex(expected, "validator"))
        .collect::<BTreeSet<[u8; 20]>>();
    assert_eq!(
        distribution
            .validators_stats
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        expected_validator_keys
    );
    for expected in validator_facts {
        let address: [u8; 20] = fixed_hex(expected, "validator");
        let actual = &distribution.validators_stats[&address];
        assert_eq!(
            u64::from(actual.dag_blocks_count),
            expected["dag_blocks_count"].as_u64().unwrap()
        );
        assert_eq!(
            actual.vote_weight,
            expected["vote_weight"].as_u64().unwrap()
        );
        assert_eq!(
            actual.fees_rewards.as_u256(),
            decimal(expected, "fees_reward")
        );
    }

    let minted = final_chain
        .plan_minted_rewards(1.into(), &distributions, &snapshot)
        .expect("Rust fixed-yield planner accepts the Go-derived distribution");
    assert_eq!(
        minted.total_minted_reward.as_u256(),
        decimal(&period["reward_output"], "minted_reward")
    );
    assert!(!minted.total_minted_reward.is_zero());
    assert_eq!(
        distribution.validators_stats[&validator]
            .fees_rewards
            .as_u256(),
        decimal(&period["reward_output"], "actual_transaction_fee")
    );

    drop(final_chain);
    drop(storage);
    std::fs::remove_dir_all(path).expect("remove temporary DB");
}
