//! Write-composition prerequisite only; session admission remains unsupported.
use super::*;
use rustaxa_storage::Config;
use rustaxa_types::GenesisValidatorMetadata;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

fn address(last: u8) -> [u8; 20] {
    let mut result = [0; 20];
    result[19] = last;
    result
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct RawOnly(BTreeMap<ConcreteStorageKey, Vec<u8>>);
impl FinalChainNativeStateRead for RawOnly {
    fn raw_storage(
        &self,
        contract: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        assert_eq!(contract, DPOS_CONTRACT_ADDRESS);
        Ok(self
            .0
            .get(key)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("zero-reward redelegation must not read or mutate accounts")
    }
}

#[test]
fn redelegate_kernel_and_composed_serializers_match_actual_go_repeat() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    let case = &public["cases"][0];
    assert_eq!(case["name"], "partial_and_repeat");
    let mut raw = RawOnly(BTreeMap::new());
    for read in case["attempts"][0]["ordered_reads"].as_array().unwrap() {
        if read["present"].as_bool().unwrap() {
            raw.0.insert(
                ConcreteStorageKey(unhex(read["key"].as_str().unwrap()).try_into().unwrap()),
                unhex(read["value"].as_str().unwrap()),
            );
        }
    }
    let (chain, storage, path) = kernel_chain(1_000_000);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    for attempt in case["attempts"].as_array().unwrap() {
        let before = session.dpos_state.clone();
        let mut after = before.clone();
        let mut accounts = StagedDposAccountPort::from_state(&raw);
        let outcome = chain
            .apply_dpos_redelegate(
                &mut after,
                &mut accounts,
                address(0xd1),
                address(0x31),
                address(0x32),
                vec![1, 0x2c],
                1.into(),
            )
            .unwrap();
        assert_eq!(outcome.status_code, 1);
        assert!(outcome.contract_error.is_none());
        assert_eq!(
            after
                .total_stakes
                .values()
                .fold(U256::zero(), |total, stake| total + stake.as_u256()),
            U256::from(2000)
        );
        assert_eq!(after.total_vote_count, before.total_vote_count);
        assert_eq!(after.validator_order, before.validator_order);
        assert_eq!(after.delegator_validators, before.delegator_validators);
        assert_eq!(
            outcome.code_retval,
            unhex(attempt["output"].as_str().unwrap())
        );
        assert!(accounts.into_mutations().is_empty());
        let logs = outcome.logs.iter().map(|log| json!({"address": hex(&log.address), "topics": log.topics.iter().map(|topic| hex(topic)).collect::<Vec<_>>(), "data": hex(&log.data)})).collect::<Vec<_>>();
        assert_eq!(logs, *attempt["logs"].as_array().unwrap());
        let mut trace = FinalChainNativeRawTrace::new(&raw);
        session
            .serialize_undelegate_principal(
                address(0xd1),
                address(0x31),
                &before,
                &after,
                &mut trace,
            )
            .unwrap();
        session
            .serialize_delegate(address(0xd1), address(0x32), &before, &after, &mut trace)
            .unwrap();
        let mutations = trace.finish();
        let actual = mutations.iter().map(|mutation| {
            let value = match &mutation.operation { FinalChainNativeRawOperation::Put(value) => value.as_bytes(), FinalChainNativeRawOperation::Delete => &[] };
            json!({"address": hex(&mutation.address), "key": hex(&mutation.key.0), "value": hex(value)})
        }).collect::<Vec<_>>();
        assert_eq!(actual, *attempt["ordered_raw_writes"].as_array().unwrap());
        for mutation in mutations {
            let current = raw
                .0
                .get(&mutation.key)
                .cloned()
                .map_or(ConcreteRead::Absent, ConcreteRead::Present);
            assert_eq!(current, mutation.expected);
            match mutation.operation {
                FinalChainNativeRawOperation::Put(value) => {
                    raw.0.insert(mutation.key, value.into_bytes());
                }
                FinalChainNativeRawOperation::Delete => {
                    raw.0.insert(mutation.key, Vec::new());
                }
            }
        }
        session.dpos_state = after;
    }
    for (validator, field) in [(0x31, "source_stake"), (0x32, "destination_stake")] {
        assert_eq!(
            session.dpos_state.total_stakes[&address(validator)]
                .as_u256()
                .to_string(),
            case[field].as_str().unwrap()
        );
    }
    drop(session);
    assert_eq!(
        chain.dpos_total_amount_delegated(0.into()).unwrap(),
        vec![7, 0xd0]
    );
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

fn kernel_chain(maximum: u64) -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "redelegate-composition-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        Vec::new(),
        [0x31, 0x32]
            .into_iter()
            .map(|validator| GenesisValidator {
                address: address(validator),
                vrf_key: [if validator == 0x31 { 0x44 } else { 0x55 }; 32],
                total_stake: U256::from(1000).to_big_endian().to_vec(),
                delegations: vec![(address(0xd1), U256::from(1000).to_big_endian().to_vec())],
                metadata: GenesisValidatorMetadata {
                    owner: address(0xa1),
                    commission: 100,
                    ..Default::default()
                },
            })
            .collect(),
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(maximum).into(),
            minimum_deposit: U256::from(100).into(),
            delegation_delay: 1,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            yield_percentage: 0,
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    (chain, storage, path)
}

#[test]
fn redelegate_kernel_normal_preflight_errors_match_go_without_advancement() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/public.json"
    )))
    .unwrap();
    let names = [
        "destination_cap_before_insufficient_source",
        "missing_source",
        "missing_destination",
        "missing_source_delegation",
        "insufficient_source",
        "remainder_below_minimum",
        "same_validator",
    ];
    let raw = RawOnly(BTreeMap::new());
    for name in names {
        let case = public["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let maximum = if name == "destination_cap_before_insufficient_source" {
            1500
        } else {
            1_000_000
        };
        let (chain, storage, path) = kernel_chain(maximum);
        let session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let committed_before = chain.dpos_snapshot(0.into()).unwrap();
        let before = session.dpos_state.clone();
        let mut after = before.clone();
        let input = unhex(case["input"].as_str().unwrap());
        let mut accounts = StagedDposAccountPort::from_state(&raw);
        let outcome = chain
            .apply_dpos_redelegate(
                &mut after,
                &mut accounts,
                unhex(case["caller"].as_str().unwrap()).try_into().unwrap(),
                input[16..36].try_into().unwrap(),
                input[48..68].try_into().unwrap(),
                input[68..100].to_vec(),
                1.into(),
            )
            .unwrap();
        let expected = &case["attempts"][0];
        assert_eq!(outcome.status_code, 0, "{name}");
        assert_eq!(
            outcome.contract_error.unwrap().legacy_message(),
            expected["execution_error"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            outcome.code_retval,
            unhex(expected["output"].as_str().unwrap()),
            "{name}"
        );
        assert!(outcome.logs.is_empty(), "{name}");
        assert!(expected["logs"].as_array().unwrap().is_empty());
        assert!(
            expected["ordered_raw_writes"].is_null()
                || expected["ordered_raw_writes"]
                    .as_array()
                    .unwrap()
                    .is_empty()
        );
        assert!(accounts.into_mutations().is_empty(), "{name}");
        assert_eq!(after, before, "{name}");
        assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed_before);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
