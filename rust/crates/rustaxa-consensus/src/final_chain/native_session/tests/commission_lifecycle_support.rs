//! Shared test seed reconstruction. Every owner history follows the Go operations.
use super::lifecycle_consensus;
use ethereum_types::{H256, U256};
use k256::ecdsa::SigningKey;
use lifecycle_consensus::FinalChain;
use rlp::RlpStream;
use rustaxa_storage::{Column, Config, Storage};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce, FinalChainRewardsConfig, FinalizationTransaction,
    GenesisAccount, GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
};
use serde_json::Value;
use std::{path::Path, sync::Arc};
fn keccak(bytes: impl AsRef<[u8]>) -> [u8; 32] {
    super::lifecycle_keccak(bytes.as_ref())
}
pub fn decode(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let (pairs, remainder) = s.as_bytes().as_chunks::<2>();
    assert!(
        remainder.is_empty(),
        "fixture hex must contain complete bytes"
    );
    pairs
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
pub fn address(last: u8) -> [u8; 20] {
    let mut a = [0; 20];
    a[19] = last;
    a
}
pub fn corpus() -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_commission_lifecycle");
    let public = std::fs::read(root.join("public.json")).unwrap();
    let local = std::fs::read(root.join("local.json")).unwrap();
    assert_eq!(public, local);
    let v: Value = serde_json::from_slice(&public).unwrap();
    assert_eq!(v["scenarios"].as_array().unwrap().len(), 56);
    v
}
pub fn expand(corpus: &Value, snapshot: &Value) -> Value {
    let mut s = snapshot.clone();
    s["seed_rows"] = Value::Array(
        snapshot["seed_rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| corpus["concrete_rows"][id.as_str().unwrap()].clone())
            .collect(),
    );
    s
}
pub fn history(
    path: &Path,
    initialize: bool,
    s: &Value,
    selected: u64,
) -> (FinalChain, Arc<Storage>) {
    let storage = Arc::new(Storage::new(Config::new(path.to_path_buf())).unwrap());
    let cfg = &s["configuration"];
    let rewards = FinalChainRewardsConfig {
        magnolia_period: 0.into(),
        phalaenopsis_period: 0.into(),
        fix_claim_all_block_num: 0.into(),
        fix_redelegate_block_num: 0.into(),
        dpos_blocks_per_year: 1,
        dpos_delegation_locking_period: 2,
        cornus_period: 0.into(),
        cornus_delegation_locking_period: 3,
        cacti_period: if s["cacti"].as_bool().unwrap() {
            0.into()
        } else {
            FinalChainBlockNumber::MAX
        },
        cacti_delegation_locking_period: 7,
        aspen_part_one_period: 0.into(),
        aspen_part_two_period: FinalChainBlockNumber::MAX,
        aspen_max_supply: U256::from(10000).into(),
        committee_size: 1,
        rewards_distribution_frequency: vec![(0.into(), 1)],
        yield_percentage: 20,
        max_block_author_reward_percent: 0,
        dag_proposers_reward_percent: 0,
        ..Default::default()
    };
    let accounts = cfg["GenesisBalances"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(a, b)| GenesisAccount {
            address: decode(a).try_into().unwrap(),
            balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(
                b.as_u64().unwrap(),
            )),
        })
        .collect();
    let validators = cfg["DPOS"]["InitialValidators"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let a: [u8; 20] = decode(v["Address"].as_str().unwrap()).try_into().unwrap();
            let delegations: Vec<_> = v["Delegations"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(a, b)| {
                    (
                        decode(a).try_into().unwrap(),
                        U256::from(b.as_u64().unwrap()).to_big_endian().to_vec(),
                    )
                })
                .collect();
            let total = delegations
                .iter()
                .map(|(_, b)| U256::from_big_endian(b))
                .fold(U256::zero(), |a, b| a + b);
            GenesisValidator {
                address: a,
                vrf_key: [match a[19] {
                    0x31 => 0x44,
                    0x32 => 0x55,
                    _ => 0x66,
                }; 32],
                total_stake: total.to_big_endian().to_vec(),
                delegations,
                metadata: GenesisValidatorMetadata {
                    owner: decode(v["Owner"].as_str().unwrap()).try_into().unwrap(),
                    commission: 100,
                    ..Default::default()
                },
            }
        })
        .collect();
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1000000.into(),
        0,
        lifecycle_consensus::FinalChainGenesisAllocation {
            accounts,
            validators,
        },
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1000000).into(),
            minimum_deposit: U256::one().into(),
            delegation_delay: 1,
            ..Default::default()
        },
        rewards,
        0.into(),
    )
    .unwrap();
    if initialize {
        for period in (chain.last_block_number_typed().unwrap().as_u64() + 1)..=selected {
            let mut rows = Vec::new();
            for stage in s["provenance"].as_array().unwrap() {
                if stage["state"]["period"].as_u64() == Some(period)
                    && let Some(txs) = stage["transactions"].as_array()
                {
                    rows.extend(txs.iter());
                }
            }
            rows.extend(
                s["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|case| case["period"].as_u64() == Some(period)),
            );
            let txs = rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| FinalizationTransaction {
                    hash: keccak(format!("{period}/{index}")),
                    sender: decode(row["sender"].as_str().unwrap()).try_into().unwrap(),
                    receiver: Some(address(0xfe)),
                    nonce: FinalChainNonce::from_u64(
                        row["nonce"].as_str().unwrap().parse().unwrap(),
                    ),
                    value: U256::zero().into(),
                    gas_price: U256::zero().into(),
                    gas_limit: 200000.into(),
                    data: decode(row["input"].as_str().unwrap()),
                    rlp: vec![0xc0],
                })
                .collect();
            commit_period(&chain, &storage, period, txs);
        }
    }
    assert_eq!(chain.last_block_number_typed().unwrap(), selected.into());
    (chain, storage)
}
fn commit_period(
    chain: &FinalChain,
    storage: &Storage,
    period: u64,
    txs: Vec<FinalizationTransaction>,
) {
    let transaction_count = txs.len();
    let fields = |s: &mut RlpStream| {
        for value in 10..14 {
            s.append(&H256::from_low_u64_be(value));
        }
        s.append(&period)
            .append(&(1700000000 + period))
            .begin_list(0);
    };
    let mut unsigned = RlpStream::new_list(7);
    fields(&mut unsigned);
    let (signature, recovery) = SigningKey::from_slice(&[9; 32])
        .unwrap()
        .sign_prehash_recoverable(&keccak(unsigned.out()))
        .unwrap();
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte());
    let mut pbft = RlpStream::new_list(8);
    fields(&mut pbft);
    pbft.append(&signature);
    let pbft = pbft.out().to_vec();
    let mut data = RlpStream::new_list(4);
    data.append_raw(&pbft, 1)
        .begin_list(0)
        .begin_list(0)
        .begin_list(txs.len());
    for tx in &txs {
        data.append_raw(&tx.rlp, 1);
    }
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(
            &mut batch,
            Column::PeriodData,
            &period.to_le_bytes(),
            &data.out(),
        )
        .unwrap();
    storage.commit_write_batch_with_sync(batch, false).unwrap();
    let (_, receipts) = if period == 1 {
        chain
            .finalize_block_with_rewards_facts_at(
                pbft,
                txs,
                vec![],
                1,
                vec![lifecycle_consensus::RewardCertVoteFact {
                    voter: ethereum_types::H160(address(0x31)),
                    weight: 1,
                    period: 1,
                }],
                1.into(),
            )
            .unwrap()
    } else {
        chain.finalize_block(pbft, txs, vec![]).unwrap()
    };
    assert_eq!(receipts.len(), transaction_count);
}
