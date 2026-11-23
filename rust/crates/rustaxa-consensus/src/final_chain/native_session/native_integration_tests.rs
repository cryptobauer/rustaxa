//! Native session checks over stored genesis and transaction inputs.
use super::*;
use serde_json::Value;
use std::{cell::RefCell, path::PathBuf, sync::Arc};

fn decode(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .unwrap()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn corpus() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/native_session_inputs.json"
    )))
    .unwrap()
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        Self(crate::test_paths::new_path("rustaxa-native-integration"))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.exists() {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn chain(
    path: &Scratch,
    fixture: &Value,
    activation: FinalChainBlockNumber,
    generated: u64,
) -> FinalChain {
    chain_with_threshold(path, fixture, activation, generated, 100)
}
fn chain_with_threshold(
    path: &Scratch,
    fixture: &Value,
    activation: FinalChainBlockNumber,
    generated: u64,
    threshold: u64,
) -> FinalChain {
    chain_with_economics(
        path,
        fixture,
        activation,
        generated,
        threshold,
        U256::from(10_000_000_000u64),
    )
}
fn chain_with_economics(
    path: &Scratch,
    fixture: &Value,
    activation: FinalChainBlockNumber,
    generated: u64,
    threshold: u64,
    max_supply: U256,
) -> FinalChain {
    let storage = Arc::new(Storage::new(rustaxa_storage::Config::new(path.0.clone())).unwrap());
    let accounts = fixture["inputs"]["genesis_allocations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| rustaxa_types::GenesisAccount {
            address: decode(&row["address"]).try_into().unwrap(),
            balance: rustaxa_types::FinalChainAccountBalance::new_account(
                U256::from_dec_str(row["balance"].as_str().unwrap()).unwrap(),
            ),
        })
        .collect();
    FinalChain::new_with_rewards_config_and_ficus_activation(
        storage,
        10_000_000.into(),
        0,
        crate::FinalChainGenesisAllocation {
            accounts,
            validators: Vec::new(),
        },
        rustaxa_types::GenesisDposConfig {
            eligibility_balance_threshold: U256::from(threshold).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::one().into(),
            commission_change_delta: 100,
            commission_change_frequency: 2,
            delegation_delay: 1,
            ..Default::default()
        },
        rustaxa_types::FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            phalaenopsis_period: 0.into(),
            cornus_period: 0.into(),
            fix_claim_all_block_num: 0.into(),
            fix_redelegate_block_num: 0.into(),
            cacti_period: 0.into(),
            cacti_jail_time: 2,
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: activation,
            genesis_balance_sum: Some(
                fixture["inputs"]["genesis_allocations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .fold(U256::zero(), |total, row| {
                        total + U256::from_dec_str(row["balance"].as_str().unwrap()).unwrap()
                    })
                    .into(),
            ),
            aspen_max_supply: max_supply.into(),
            aspen_generated_rewards: U256::from(generated).into(),
            dpos_blocks_per_year: 100,
            yield_percentage: 1,
            committee_size: 1,
            rewards_distribution_frequency: vec![(0.into(), 1)],
            ..Default::default()
        },
        0.into(),
    )
    .unwrap()
}
#[derive(Clone)]
struct State {
    rows: BTreeMap<([u8; 20], ConcreteStorageKey), ConcreteRead<Vec<u8>>>,
    reads: RefCell<Vec<([u8; 20], ConcreteStorageKey)>>,
    fault: Option<(usize, bool)>,
}
impl State {
    fn genesis(fixture: &Value) -> Self {
        Self {
            rows: fixture["genesis"]["native_catalog"]["slots"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    (
                        (
                            decode(&row["address"]).try_into().unwrap(),
                            ConcreteStorageKey(decode(&row["key"]).try_into().unwrap()),
                        ),
                        if row["present"] == true {
                            ConcreteRead::Present(decode(&row["value"]))
                        } else {
                            ConcreteRead::Absent
                        },
                    )
                })
                .collect(),
            reads: RefCell::new(Vec::new()),
            fault: None,
        }
    }
}
impl FinalChainNativeStateRead for State {
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        let index = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, *key));
        if let Some((at, malformed)) = self.fault
            && index == at
        {
            return if malformed {
                Ok(ConcreteRead::Present(vec![0xff]))
            } else {
                Err(FinalChainNativeStateReadError::Invariant(
                    "native integration consumed read".into(),
                ))
            };
        }
        Ok(self
            .rows
            .get(&(address, *key))
            .cloned()
            .unwrap_or(ConcreteRead::Absent))
    }
}
fn request(row: &Value, sequence: u64) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: (sequence as u32).into(),
            sequence,
        },
        period: 1.into(),
        depth: 0,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: decode(&row["sender"]).try_into().unwrap(),
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(
            BigUint::parse_bytes(row["value"].as_str().unwrap().as_bytes(), 10).unwrap(),
        ),
        input: decode(&row["input"]),
        supplied_gas: 500_000.into(),
    }
}
fn completed(result: FinalChainNativeInvocationResult) -> FinalChainNativeOutcome {
    let FinalChainNativeInvocationResult::Completed(outcome) = result else {
        panic!("funded reference invocation")
    };
    outcome
}
#[test]
fn native_integration_aspen_genesis_is_initialized_without_minting() {
    let fixture = corpus();
    for activation in [
        FinalChainBlockNumber::GENESIS,
        2.into(),
        FinalChainBlockNumber::MAX,
    ] {
        let path = Scratch::new();
        let chain = chain(&path, &fixture, activation, 7);
        let genesis = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
        assert!(genesis.delegator_rewards.is_empty() && genesis.commission_rewards.is_empty());
        assert!(genesis.validator_order.is_empty());
        if activation == FinalChainBlockNumber::GENESIS {
            let supply = DposTokenAmount::from(U256::from(5_000_000_007_u64));
            assert_eq!(
                genesis.aspen_supply_state.total_supply().unwrap().amount(),
                supply
            );
            assert_eq!(
                genesis.aspen_supply_state.current_yield(),
                Some(chain.aspen_current_yield(supply).unwrap())
            );
        } else {
            assert!(matches!(
                genesis.aspen_supply_state,
                AspenSupplyState::Unmigrated { .. }
            ));
        }
    }
}
#[test]
fn native_integration_slashing_marker_matches_literal_go_key_and_reversal() {
    let fixture = corpus();
    let row = fixture["periods"][3]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "slash_a")
        .unwrap();
    let mut input = decode(&row["input"]);
    let proof = verify_legacy_double_voting_proof_call_data(&input).unwrap();
    assert_eq!(
        hex(proof.proof_key.as_bytes()),
        "dc35efc475e49bffe215eea886063ea790bde40e358685c728d52129f16ae9db"
    );
    let left = input[4..36].to_vec();
    let right = input[36..68].to_vec();
    input[4..36].copy_from_slice(&right);
    input[36..68].copy_from_slice(&left);
    assert_eq!(
        verify_legacy_double_voting_proof_call_data(&input)
            .unwrap()
            .proof_key,
        proof.proof_key
    );
}

#[test]
fn native_integration_existing_checkpoint_keeps_later_reward_pool() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain(&path, &fixture, 0.into(), 0);
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let state = State::genesis(&fixture);
    let row = &fixture["periods"][0]["transactions"][0];
    let request = request(row, 0);
    let quote = session.prepare(&request, &state).unwrap();
    completed(session.invoke(&request, quote, &state).unwrap());
    let mut snapshot = session.dpos_state.clone();
    let validator = decode(&fixture["periods"][0]["transactions"][0]["input"])[16..36]
        .try_into()
        .unwrap();
    snapshot.delegator_rewards.insert(
        validator,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(11)),
    );
    let existing = chain
        .checkpoint_validator_reward_per_stake(&mut snapshot, validator)
        .unwrap();
    assert_eq!(
        snapshot.delegator_rewards[&validator].as_u256(),
        U256::from(11)
    );
    chain
        .advance_reward_reference_graph_block(&mut snapshot, 2.into())
        .unwrap();
    let next = chain
        .checkpoint_validator_reward_per_stake(&mut snapshot, validator)
        .unwrap();
    assert!(next > existing);
    assert!(snapshot.delegator_rewards[&validator].is_zero());
}

#[test]
fn native_integration_zero_threshold_accepts_absent_stake_as_go_does() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain_with_threshold(&path, &fixture, 0.into(), 0, 0);
    let row = fixture["periods"][3]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "slash_a")
        .unwrap();
    let proof = verify_legacy_double_voting_proof_call_data(&decode(&row["input"])).unwrap();
    let mut state = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
    assert!(state.total_stakes.is_empty());
    let result = chain
        .apply_slashing_double_voting_proof(&mut state, None, 1.into(), proof)
        .unwrap();
    assert_eq!(result.status_code, 1);
    assert_eq!(state.slashing_jail_blocks.len(), 1);
}

#[test]
fn native_integration_failed_prepare_discards_old_input_metadata() {
    let fixture = corpus();
    let path = Scratch::new();
    let chain = chain(&path, &fixture, 0.into(), 0);
    for invalid in 0..3 {
        let state = State::genesis(&fixture);
        let original = request(&fixture["periods"][0]["transactions"][0], 0);
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let quote = session.prepare(&original, &state).unwrap();
        completed(session.invoke(&original, quote, &state).unwrap());
        assert_eq!(session.input_mutations.len(), 1);
        let before = session.dpos_state.clone();
        let mut bad = original.clone();
        bad.id.sequence = 1;
        match invalid {
            0 => bad.period = 2.into(),
            1 => {
                bad.contract = [7; 20];
                bad.state_address = [7; 20];
            }
            _ => bad.id.sequence = 3,
        }
        assert!(session.prepare(&bad, &state).is_err());
        assert_eq!(session.dpos_state, before);
        assert_eq!(
            session.input_mutations(original.id),
            Err(FinalChainNativeSessionError::NotPrepared)
        );
        assert!(session.input_mutations.is_empty());
    }
}
