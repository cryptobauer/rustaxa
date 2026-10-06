//! Exact redelegate DryRunner outputs over reopened, persisted Go seed rows.
//! Each Rust probe owns a fresh real historical native session. This fixture
//! grants absence authority only for its complete synthetic seed.

use super::*;

/// Independent semantic owner for the Go seed's two-validator zero-reward H=1.
/// Only initial construction finalizes an empty synthetic period through the
/// existing public owner. Restart must load H=1 and never finalize it again.
pub(super) fn history(path: &FixturePath, initialize: bool) -> FinalChain {
    let storage = Arc::new(Storage::new(Config::new(path.0.clone())).unwrap());
    let balance = U256::from(4000);
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        500_000.into(),
        0,
        vec![
            GenesisAccount {
                address: address(0xd1),
                balance: rustaxa_types::FinalChainAccountBalance::new_account(balance),
            },
            GenesisAccount {
                address: address(0xa1),
                balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(2000)),
            },
        ],
        [0x31, 0x32]
            .into_iter()
            .map(|last| GenesisValidator {
                address: address(last),
                vrf_key: [if last == 0x31 { 0x44 } else { 0x55 }; 32],
                total_stake: U256::from(if last == 0x31 { 2000 } else { 1000 })
                    .to_big_endian()
                    .to_vec(),
                delegations: vec![(address(0xa1), U256::from(1000).to_big_endian().to_vec())]
                    .into_iter()
                    .chain(
                        (last == 0x31)
                            .then_some((address(0xd1), U256::from(1000).to_big_endian().to_vec())),
                    )
                    .collect(),
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
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::from(100).into(),
            delegation_delay: 1,
            commission_change_delta: 0,
            commission_change_frequency: 0,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: 0.into(),
            fix_claim_all_block_num: 0.into(),
            phalaenopsis_period: 0.into(),
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            yield_percentage: 0,
            dpos_blocks_per_year: 1,
            aspen_max_supply: U256::from(6000).into(),
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    if initialize {
        assert_eq!(
            chain.last_block_number_typed().unwrap(),
            FinalChainBlockNumber::GENESIS
        );
        fn fields(stream: &mut RlpStream) {
            for value in 10..14 {
                stream.append(&H256::from_low_u64_be(value));
            }
            stream
                .append(&1_u64)
                .append(&1_700_000_001_u64)
                .begin_list(0);
        }
        let mut unsigned = RlpStream::new_list(7);
        fields(&mut unsigned);
        let (signature, recovery) = SigningKey::from_slice(&[9; 32])
            .unwrap()
            .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
            .unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery.to_byte());
        let mut pbft = RlpStream::new_list(8);
        fields(&mut pbft);
        pbft.append(&signature);
        let pbft = pbft.out().to_vec();
        let mut period = RlpStream::new_list(4);
        period
            .append_raw(&pbft, 1)
            .begin_list(0)
            .begin_list(0)
            .begin_list(0);
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PeriodData,
                &1_u64.to_le_bytes(),
                &period.out(),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let (_, receipts) = chain.finalize_block(pbft, vec![], vec![]).unwrap();
        assert!(receipts.is_empty());
    }
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    assert_eq!(
        chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![0x0b, 0xb8]
    );
    chain
}

/// Exact committed validator rows and d1/a1 ordered delegations, through public
/// read owners. Total principal alone cannot detect a leaked redelegation.
pub(super) fn assert_committed(chain: &FinalChain) {
    let stakes = chain.dpos_validators_total_stakes(1.into()).unwrap();
    assert_eq!(stakes.len(), 2);
    for (row, last) in stakes.iter().zip([0x31, 0x32]) {
        assert_eq!(row.address, address(last));
        assert_eq!(
            U256::from_big_endian(&row.stake),
            U256::from(if last == 0x31 { 2000 } else { 1000 })
        );
    }
    for owner in [0xd1, 0xa1] {
        let mut input = keccak256(b"getDelegations(address,uint32)")[..4].to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&address(owner));
        input.extend_from_slice(&[0; 32]);
        let outcome = chain
            .call(rustaxa_types::FinalChainCallRequest {
                block_number: 1.into(),
                sender: address(owner),
                receiver: Some(address(0xfe)),
                value: U256::zero().into(),
                gas_price: U256::zero().into(),
                gas_limit: 1_000_000.into(),
                input,
            })
            .unwrap();
        let expected_members: &[u8] = if owner == 0xd1 {
            &[0x31]
        } else {
            &[0x31, 0x32]
        };
        assert_eq!(outcome.code_retval.len(), 96 + 96 * expected_members.len());
        assert_eq!(
            U256::from_big_endian(&outcome.code_retval[64..96]),
            U256::from(expected_members.len())
        );
        for (row, last) in outcome.code_retval[96..]
            .as_chunks::<96>()
            .0
            .iter()
            .zip(expected_members.iter().copied())
        {
            assert_eq!(&row[12..32], &address(last));
            assert_eq!(U256::from_big_endian(&row[32..64]), U256::from(1000));
            assert_eq!(U256::from_big_endian(&row[64..96]), U256::zero());
        }
    }
}

#[test]
fn persisted_full_new_simulations_match_actual_go_dry_runner_and_reopen() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_full_new_simulation/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_full_new_simulation/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["repeat_identical"], true);
    assert_eq!(public["committed_state_unchanged"], true);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    assert_eq!(public["cases"][0]["name"], "full_new");
    assert_eq!(
        number(&public["cases"][0]["supplied_nonce"]),
        BigUint::from(1_u8) << 512
    );
    assert_eq!(public["cases"][0]["value"], "0");
    assert_eq!(public["cases"][0]["gas"], 200_000);
    assert_eq!(public["cases"][0]["gas_price"], "0");
    assert_eq!(public["cases"][0]["to"], hex::encode(address(0xfe)));
    let config = &public["state_before"]["configuration"];
    assert_eq!(config["EVMChainConfig"]["chainId"], 666);
    assert_eq!(
        config["GenesisBalances"][format!("0x{}", hex::encode(address(0xd1)))],
        4000
    );
    assert_eq!(
        config["GenesisBalances"][format!("0x{}", hex::encode(address(0xa1)))],
        2000
    );
    assert_eq!(config["Hardforks"]["AspenHf"]["MaxSupply"], 6000);
    assert_eq!(config["DPOS"]["YieldPercentage"], 0);
    let facts = &public["state_before"]["native_facts"];
    for owner in [0xd1, 0xa1] {
        let members = if owner == 0xd1 {
            vec![0x31]
        } else {
            vec![0x31, 0x32]
        };
        assert_eq!(
            facts["memberships"][hex::encode(address(owner))],
            json!(
                members
                    .iter()
                    .map(|last| hex::encode(address(*last)))
                    .collect::<Vec<_>>()
            )
        );
        for last in [0x31, 0x32] {
            let pair = &facts["delegations"][format!(
                "{}/{}",
                hex::encode(address(owner)),
                hex::encode(address(last))
            )];
            if owner == 0xd1 && last == 0x32 {
                assert!(pair.is_null());
            } else {
                assert_eq!(pair["stake"], "1000");
                assert_eq!(pair["last_updated"], 0);
            }
        }
    }
    let input = bytes(&public["cases"][0]["input"]);
    assert_eq!(
        &input[..4],
        &keccak256(b"reDelegate(address,address,uint256)")[..4]
    );
    assert_eq!(&input[16..36], &address(0x31));
    assert_eq!(&input[48..68], &address(0x32));
    assert_eq!(U256::from_big_endian(&input[68..]), U256::from(1000));
    assert_eq!(
        public["cases"][0]["output"]["logs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        bytes(&public["cases"][0]["output"]["logs"][0]["data"]),
        U256::from(1000).to_big_endian()
    );
    let concrete = FixturePath::new("full-new-concrete");
    let app = FixturePath::new("full-new-app");
    let identity = materialize(&public, &concrete);
    assert_eq!(identity.period, 1.into());
    assert_eq!(
        public["state_before"]["seed_rows"]
            .as_array()
            .unwrap()
            .len(),
        79
    );
    assert_eq!(
        hex::encode(identity.state_root),
        public["state_before"]["root"]
    );
    let before = concrete_rows(&concrete);
    let mut sessions = 0;
    for owner_restart in 0..2 {
        let chain = history(&app, owner_restart == 0);
        assert_committed(&chain);
        for _ in 0..2 {
            // Reopen the physical reader independently of semantic-owner reconstruction.
            let reader = CompleteSeedReader::open(&concrete, &public, identity);
            assert_eq!(reader.identity(), identity);
            for (node, last) in public["state_before"]["current_reward_nodes"]
                .as_array()
                .unwrap()
                .iter()
                .zip([0x31, 0x32])
            {
                let key =
                    ConcreteStorageKey(keccak256([&[1][..], &address(last), &[1]].concat()).0);
                assert_eq!(node["block"], 1);
                assert_eq!(node["validator"], hex::encode(address(last)));
                assert_eq!(node["key"], hex::encode(key.0));
                assert_eq!(node["present"], false);
                assert_eq!(node["value"], "");
                assert_eq!(
                    reader.storage(address(0xfe), key).unwrap(),
                    ConcreteRead::Absent
                );
            }
            for case in public["cases"].as_array().unwrap() {
                for _ in 0..2 {
                    let ConcreteRead::Present(sender) = reader.account(address(0xd1)).unwrap()
                    else {
                        panic!("missing sender")
                    };
                    assert_eq!(
                        BigUint::from_bytes_be(&sender.account.nonce.next().to_bytes()),
                        number(&case["output"]["effective_nonce"])
                    );
                    let supplied_nonce =
                        FinalChainNonce::from_bytes(&number(&case["supplied_nonce"]).to_bytes_be())
                            .unwrap();
                    let request = ExecutionTransaction {
                        position: 0.into(),
                        hash: [0; 32],
                        sender: address(0xd1),
                        receiver: Some(bytes(&case["to"]).try_into().unwrap()),
                        nonce: supplied_nonce.clone(),
                        gas_price: ExecutionGasPrice::new(number(&case["gas_price"])),
                        gas_limit: case["gas"].as_u64().unwrap().into(),
                        value: ExecutionValue::new(number(&case["value"])),
                        input: bytes(&case["input"]),
                        canonical_rlp: None,
                        kind: ExecutionTransactionKind::Call,
                    };
                    sessions += 1;
                    let simulated = simulate_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected| {
                            assert_eq!(selected, identity);
                            Ok(mixed_native::SimulationNativeExecutionPort::new(
                                chain.begin_native_simulation(selected.period).unwrap(),
                            ))
                        },
                        &ExecutionBlockContext {
                            period: identity.period,
                            author: address(0x31),
                            timestamp: 1_700_000_001,
                            gas_limit: 500_000.into(),
                            chain_id: 666,
                            difficulty: BigUint::default(),
                        },
                        &request,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    )
                    .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                    assert_eq!(simulated.state, identity);
                    assert_eq!(request.nonce, supplied_nonce);
                    assert_eq!(case["output"]["consensus_error"], "");
                    let TransactionExecutionResult::Executed(result) = simulated.execution else {
                        panic!("Go admitted {}", case["name"])
                    };
                    assert_eq!(result.status, CodeExecutionStatus::Success);
                    assert_eq!(case["output"]["execution_error"], "");
                    assert_eq!(
                        result.gas_used.as_u64(),
                        case["output"]["gas_used"].as_u64().unwrap(),
                        "{}",
                        case["name"]
                    );
                    assert_eq!(
                        result.output,
                        bytes(&case["output"]["return"]),
                        "{}",
                        case["name"]
                    );
                    let logs = result.logs.iter().map(|log| json!({
                    "address": hex::encode(log.address), "topics": log.topics.iter().map(hex::encode).collect::<Vec<_>>(), "data": hex::encode(&log.data),
                })).collect::<Vec<_>>();
                    assert_eq!(
                        logs,
                        *case["output"]["logs"].as_array().unwrap(),
                        "{}",
                        case["name"]
                    );
                }
            }
            assert_eq!(
                chain.dpos_total_amount_delegated(1.into()).unwrap(),
                vec![0x0b, 0xb8]
            );
            assert_committed(&chain);
            assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
            // Reading exact physical rows after disposing both readers is below.
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
        drop(chain);
    }
    assert_eq!(sessions, 8);
}
