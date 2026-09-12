//! Test-only module compiled in a disposable copy of the existing FinalChain
//! crate. Calls its private staged DPoS kernel directly during an actual REVM
//! CALL/STATICCALL yield. The small serializer handles only setCommission;
//! unknown operations fail. No production API or fallback is introduced.
#![deny(warnings)]
use super::*;
#[path = "host.rs"]
mod host;
use revm::{
    bytecode::Bytecode,
    context_interface::cfg::GasParams,
    interpreter::{
        FrameInput, Gas, InstructionResult, Interpreter, InterpreterAction, instruction_table,
        instructions::gas_table_spec,
    },
    primitives::{Address, hardfork::SpecId, keccak256},
};
use serde_json::{Value, json};
fn decode(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}
fn addr(v: &Value) -> [u8; 20] {
    decode(v).try_into().unwrap()
}
#[test]
fn staged_kernel_inside_reverting_calls_matches_reference() {
    for source in [
        include_str!("../fixtures/local.json"),
        include_str!("../fixtures/public.json"),
    ] {
        let fixtures: Value = serde_json::from_str(source).unwrap();
        for row in fixtures["native_calls"].as_array().unwrap() {
            let name = row["case"].as_str().unwrap();
            let validator = addr(&row["validator"]);
            let owner = addr(&row["owner"]);
            let parent = Address::with_last_byte(0xbb);
            let sender = Address::with_last_byte(0xaa);
            let path = std::env::temp_dir()
                .join(format!("native-kernel-probe-{}-{name}", std::process::id()));
            let storage =
                Arc::new(Storage::new(rustaxa_storage::Config::new(path.clone())).unwrap());
            let chain = FinalChain::new(
                storage.clone(),
                1_000_000.into(),
                0,
                vec![],
                vec![GenesisValidator {
                    address: validator,
                    vrf_key: [0; 32],
                    total_stake: U256::from(10000).to_big_endian().to_vec(),
                    delegations: vec![(validator, U256::from(10000).to_big_endian().to_vec())],
                    metadata: rustaxa_types::GenesisValidatorMetadata {
                        owner,
                        commission: 100,
                        ..Default::default()
                    },
                }],
                GenesisDposConfig {
                    eligibility_balance_threshold: U256::from(1000).into(),
                    vote_eligibility_balance_step: U256::from(1000).into(),
                    validator_maximum_stake: U256::from(30000).into(),
                    ..Default::default()
                },
            )
            .unwrap();
            let mut snapshot = chain
                .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
                .unwrap();
            let mut raw: BTreeMap<String, Vec<u8>> = row["prior_raw"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), decode(v)))
                .collect();
            let mut writes = vec![];
            let mut logs = vec![];
            let mut kernel_calls = 0;
            let code = decode(&row["code"]);
            let mut i =
                Interpreter::default().with_bytecode(Bytecode::new_raw(code.clone().into()));
            i.runtime_flag.spec_id = SpecId::ISTANBUL;
            i.gas = Gas::new(79000);
            i.input.target_address = parent;
            i.input.caller_address = sender;
            let mut host = host::ProbeHost {
                price: revm::primitives::U256::from(1),
                gas: GasParams::new_spec(SpecId::ISTANBUL),
                native_load: true,
                slots: None,
                transient: None,
            };
            let result = loop {
                match i.run_plain(
                    &instruction_table(),
                    &gas_table_spec(SpecId::ISTANBUL),
                    &mut host,
                ) {
                    InterpreterAction::NewFrame(FrameInput::Call(call)) => {
                        assert_eq!(call.bytecode_address, Address::with_last_byte(0xfe));
                        assert!(call.call_value().is_zero());
                        let input = call.input.as_bytes_memory(&i.memory).to_vec();
                        assert_eq!(input, decode(&row["abi"]));
                        // Historical nested-call rejection belongs to the host admission boundary.
                        let ok = if row["pre_fix"] == true {
                            false
                        } else {
                            kernel_calls += 1;
                            let tx = decode_dpos_transaction_for_execution(
                                &input,
                                call.caller.0.0,
                                1.into(),
                                u64::MAX.into(),
                                0.into(),
                                u64::MAX.into(),
                            );
                            let outcome = chain
                                .apply_dpos_mutation_transaction(
                                    1.into(),
                                    tx,
                                    &mut snapshot,
                                    &mut HashMap::new(),
                                )
                                .unwrap();
                            if outcome.status_code == 1 {
                                let meta = &snapshot.validator_metadata[&validator];
                                let mut old = rlp::RlpStream::new_list(4);
                                old.append(&snapshot.total_stakes[&validator].as_u256())
                                    .append(&meta.commission)
                                    .append(&meta.last_commission_change)
                                    .append(
                                        &snapshot
                                            .reward_reference_graph
                                            .read_validator_head(&validator)
                                            .unwrap(),
                                    );
                                let mut extended = rlp::RlpStream::new_list(2);
                                extended.append_raw(&old.out(), 1).append(&0u16);
                                let value = extended.out().to_vec();
                                let key = hex::encode(concrete_storage_key(&[&[0, 0], &validator]));
                                raw.insert(key.clone(), value.clone());
                                writes.push(json!({"key":key,"value":hex::encode(value)}));
                                for log in outcome.logs {
                                    logs.push(json!({"address":hex::encode(log.address),"topics":log.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(log.data)}));
                                }
                            }
                            outcome.status_code == 1
                        };
                        // Go native failures return unused action gas too (not exceptional-halt gas).
                        i.gas.erase_cost(call.gas_limit - 20000);
                        assert!(i.stack.push(revm::primitives::U256::from(ok as u8)));
                        i.return_data.0 = Default::default();
                    }
                    InterpreterAction::Return(result) => break result,
                    _ => panic!("unsupported native probe frame"),
                }
            };
            assert_eq!(kernel_calls, if row["pre_fix"] == true { 0 } else { 1 });
            if result.result == InstructionResult::Revert {
                logs.clear();
            }
            assert_eq!(
                100000 - result.gas.remaining(),
                row["gas_used"].as_u64().unwrap(),
                "{name}: gas"
            );
            assert_eq!(
                hex::encode(&result.output),
                row["return"].as_str().unwrap(),
                "{name}: return"
            );
            assert_eq!(
                if result.result == InstructionResult::Revert {
                    "execution reverted"
                } else {
                    assert!(result.result.is_ok());
                    ""
                },
                row["error"].as_str().unwrap()
            );
            assert_eq!(Value::Array(logs), row["logs"], "{name}: logs");
            let expected_writes = if row["writes"].is_null() {
                vec![]
            } else {
                row["writes"].as_array().unwrap().clone()
            };
            assert_eq!(writes, expected_writes, "{name}: ordered writes");
            for (k, v) in &raw {
                assert_eq!(
                    hex::encode(v),
                    row["raw"][k].as_str().unwrap(),
                    "{name}: raw"
                )
            }
            let root = triehash::trie_root::<keccak_hasher::KeccakHasher, _, _, _>(raw.iter().map(
                |(k, v)| {
                    (
                        keccak256(hex::decode(k).unwrap()).to_vec(),
                        rlp::encode(v).to_vec(),
                    )
                },
            ));
            assert_eq!(hex::encode(root), row["storage_root"].as_str().unwrap());
            let mut leaves = vec![];
            for (address, a) in row["accounts"].as_object().unwrap() {
                let address = hex::decode(address).unwrap();
                let is_dpos = address[19] == 0xfe;
                let is_sender = address[19] == 0xaa;
                let balance = if is_sender {
                    900000 + result.gas.remaining()
                } else if is_dpos {
                    10000
                } else {
                    0
                };
                let nonce = if is_sender { 2u64 } else { 1 };
                let account_code = if address[19] == 0xbb {
                    code.clone()
                } else {
                    vec![]
                };
                let storage_hash = if is_dpos { root } else { keccak256([0x80]).0 };
                let code_hash = keccak256(&account_code);
                let mut leaf = rlp::RlpStream::new_list(4);
                leaf.append(&nonce)
                    .append(&balance)
                    .append(&storage_hash.as_slice())
                    .append(&code_hash.as_slice());
                let leaf = leaf.out().to_vec();
                assert_eq!(
                    hex::encode(&leaf),
                    a["leaf"].as_str().unwrap(),
                    "{name}: account leaf"
                );
                leaves.push((keccak256(address).to_vec(), leaf));
            }
            assert_eq!(
                hex::encode(triehash::trie_root::<keccak_hasher::KeccakHasher, _, _, _>(
                    leaves
                )),
                row["root"].as_str().unwrap(),
                "{name}: account root"
            );
            drop(chain);
            drop(storage);
            std::fs::remove_dir_all(path).unwrap();
        }
    }
}
