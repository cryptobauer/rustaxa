//! Bounded CREATE frame experiment over exact native nonces. It accepts only the
//! fixture's zero-value, no-storage instruction subset and panics on other host
//! work. Clone checkpoints deliberately favor auditability over performance.
//! This is not a general transaction executor or a production state database.
use super::host::ProbeHost;
use revm::interpreter::interpreter_types::Jumps;
use revm::{
    bytecode::Bytecode,
    context_interface::{CreateScheme, cfg::GasParams},
    interpreter::{
        FrameInput, Gas, InputsImpl, InstructionResult, Interpreter, InterpreterAction,
        InterpreterResult, SharedMemory, instruction_table, instructions::gas_table_spec,
        interpreter::ExtBytecode,
    },
    primitives::{Address, U256, hardfork::SpecId, keccak256},
};
use rustaxa_types::final_chain::FinalChainNonce;
use std::collections::BTreeMap;

/// Fixture account state. Balances are limited to the corpus's funded sender;
/// contract value transfer is rejected rather than silently narrowed.
#[derive(Clone, Debug)]
struct Account {
    nonce: FinalChainNonce,
    balance: u64,
    code: Vec<u8>,
}

/// Derives CREATE from exact arbitrary-width RLP; never uses REVM's u64 helper.
fn create_address(caller: Address, nonce: &FinalChainNonce) -> Address {
    let mut rlp = rlp::RlpStream::new_list(2);
    rlp.append(&caller.as_slice()).append(&nonce.to_bytes());
    Address::from_slice(&keccak256(rlp.out())[12..])
}

/// Runs a single frame and recursively settles CREATE children. Only the small
/// allowed instruction set is supported, depth is bounded to eight, and every
/// infrastructure/unsupported-operation failure aborts rather than making a receipt.
fn execute(
    accounts: &mut BTreeMap<Address, Account>,
    address: Address,
    caller: Address,
    code: Vec<u8>,
    gas: u64,
    depth: usize,
) -> InterpreterResult {
    assert!(depth < 8, "probe depth bound");
    let mut interpreter: Interpreter = Interpreter::new(
        SharedMemory::new(),
        ExtBytecode::new(Bytecode::new_raw(code.into())),
        InputsImpl {
            target_address: address,
            caller_address: caller,
            depth,
            ..Default::default()
        },
        false,
        SpecId::ISTANBUL,
        gas,
    );
    let mut host = ProbeHost {
        native_load: false,
        slots: None,
        transient: None,
        price: U256::from(1),
        gas: GasParams::new_spec(SpecId::ISTANBUL),
    };
    let table = instruction_table();
    let gas_table = gas_table_spec(SpecId::ISTANBUL);
    loop {
        // Validate executed opcodes, not data bytes: init/runtime data may contain
        // any byte. This subset has no jumps; instructions and immediate pushes
        // are handled by REVM, and CREATE yields the only allowed frame action.
        let action = loop {
            let opcode = interpreter.bytecode.opcode();
            assert!(
                matches!(
                    opcode,
                    0x00 | 0x39
                        | 0x3d
                        | 0x3e
                        | 0x50
                        | 0x52
                        | 0x53
                        | 0x60
                        | 0xf0
                        | 0xf3
                        | 0xf5
                        | 0xfd
                        | 0xfe
                ),
                "unsupported probe opcode {opcode:02x}"
            );
            if let Err(result) = interpreter.step(&table, &gas_table, &mut host) {
                use revm::interpreter::interpreter_types::LoopControl;
                if interpreter.bytecode.action().is_none() {
                    interpreter.halt(result);
                }
                break interpreter.take_next_action();
            }
        };
        match action {
            InterpreterAction::Return(mut result) => {
                if !result.result.is_ok_or_revert() {
                    result.gas.spend_all();
                }
                return result;
            }
            InterpreterAction::NewFrame(FrameInput::Create(inputs)) => {
                assert_eq!(inputs.value(), U256::ZERO, "value transfer outside probe");
                assert_eq!(inputs.caller(), address);
                let old_nonce = accounts[&address].nonce.clone();
                let child = match inputs.scheme() {
                    CreateScheme::Create => create_address(address, &old_nonce),
                    CreateScheme::Create2 { salt } => {
                        address.create2(salt.to_be_bytes::<32>(), keccak256(inputs.init_code()))
                    }
                    CreateScheme::Custom { .. } => panic!("custom create outside probe"),
                };
                // Taraxa increments the caller before collision checking and
                // before the child's checkpoint, so child failure retains it.
                accounts.get_mut(&address).unwrap().nonce = old_nonce.next();
                let collision = accounts
                    .get(&child)
                    .is_some_and(|a| !a.nonce.is_zero() || !a.code.is_empty());
                let result = if collision {
                    InterpreterResult {
                        result: InstructionResult::CreateCollision,
                        output: Default::default(),
                        gas: Gas::new_spent_with_reservoir(inputs.gas_limit(), 0),
                    }
                } else {
                    let checkpoint = accounts.clone();
                    accounts.insert(
                        child,
                        Account {
                            nonce: FinalChainNonce::from_u64(1),
                            balance: 0,
                            code: vec![],
                        },
                    );
                    let mut result = execute(
                        accounts,
                        child,
                        address,
                        inputs.init_code().to_vec(),
                        inputs.gas_limit(),
                        depth + 1,
                    );
                    if result.result.is_ok() {
                        if result.output.len() > 24_576 {
                            result.result = InstructionResult::CreateContractSizeLimit;
                            result.gas.spend_all();
                        } else if !result
                            .gas
                            .record_regular_cost(200 * result.output.len() as u64)
                        {
                            result.result = InstructionResult::OutOfGas;
                            result.gas.spend_all();
                        } else {
                            accounts.get_mut(&child).unwrap().code = result.output.to_vec();
                        }
                    }
                    if !result.result.is_ok() {
                        *accounts = checkpoint;
                    }
                    result
                };
                // Return bytes only survive failed CREATE with REVERT. The parent
                // gets unused child gas on success/revert, and an address or zero.
                interpreter.return_data.0 = if result.result == InstructionResult::Revert {
                    result.output.clone()
                } else {
                    Default::default()
                };
                if result.result.is_ok_or_revert() {
                    interpreter.gas.erase_cost(result.gas.remaining());
                }
                assert!(interpreter.stack.push(if result.result.is_ok() {
                    U256::from_be_slice(child.as_slice())
                } else {
                    U256::ZERO
                }));
            }
            _ => panic!("unsupported frame action"),
        }
    }
}

/// Executes every pinned creation case and compares gas, exact output/errors,
/// all account fields, disk/commitment encoding and complete post-EVM state root.
/// The envelope here is deliberately fixed to the valid fixture transaction.
#[test]
fn nested_creation_matches_both_references() {
    for source in [
        include_str!("../fixtures/local.json"),
        include_str!("../fixtures/public.json"),
    ] {
        let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
        for row in fixture["creation_frames"].as_array().unwrap() {
            let name = row["case"].as_str().unwrap();
            let gas_cap = row["gas_cap"].as_u64().unwrap();
            let sender = Address::with_last_byte(0xaa);
            let parent = Address::with_last_byte(0xbb);
            let nonce_text = row["parent_nonce"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x");
            let nonce = num_bigint::BigUint::parse_bytes(nonce_text.as_bytes(), 16).unwrap();
            let mut accounts = BTreeMap::from([
                (
                    sender,
                    Account {
                        nonce: FinalChainNonce::from_u64(2),
                        balance: 1_000_000 - gas_cap,
                        code: vec![],
                    },
                ),
                (
                    parent,
                    Account {
                        nonce: FinalChainNonce::from_bytes(&nonce.to_bytes_be()).unwrap(),
                        balance: 0,
                        code: hex::decode(row["parent_code"].as_str().unwrap()).unwrap(),
                    },
                ),
            ]);
            let child = Address::from_slice(&hex::decode(row["child"].as_str().unwrap()).unwrap());
            if row["collision"] == true {
                accounts.insert(
                    child,
                    Account {
                        nonce: FinalChainNonce::from_u64(1),
                        balance: 0,
                        code: vec![],
                    },
                );
            }
            let checkpoint = accounts.clone();
            let code = accounts[&parent].code.clone();
            let result = execute(&mut accounts, parent, sender, code, gas_cap - 21_000, 0);
            if !result.result.is_ok() {
                accounts = checkpoint;
            }
            let gas_used = gas_cap - result.gas.remaining();
            accounts.get_mut(&sender).unwrap().balance += result.gas.remaining();
            assert_eq!(gas_used, row["gas_used"].as_u64().unwrap(), "{name}: gas");
            assert_eq!(
                hex::encode(&result.output),
                row["return"].as_str().unwrap(),
                "{name}: output"
            );
            let error = match result.result {
                InstructionResult::Revert => "execution reverted",
                r if r.is_ok() => "",
                r => panic!("{name}: unexpected top-level result {r:?}"),
            };
            for key in ["execution_error", "error"] {
                assert_eq!(error, row[key].as_str().unwrap(), "{name}: {key}");
            }
            assert_eq!(row["consensus_error"], "");
            let expected = row["accounts"].as_object().unwrap();
            assert_eq!(
                accounts.len(),
                expected.values().filter(|a| !a.is_null()).count(),
                "{name}: account count"
            );
            let mut leaves = vec![];
            for (key, value) in expected {
                let address = Address::from_slice(&hex::decode(key).unwrap());
                if value.is_null() {
                    assert!(!accounts.contains_key(&address), "{name}: absent {key}");
                    continue;
                }
                let actual = &accounts[&address];
                assert_eq!(
                    num_bigint::BigUint::from_bytes_be(&actual.nonce.to_bytes()).to_string(),
                    value["nonce"].as_str().unwrap(),
                    "{name}: nonce {key}"
                );
                assert_eq!(
                    actual.balance.to_string(),
                    value["balance"].as_str().unwrap(),
                    "{name}: balance {key}"
                );
                assert_eq!(
                    hex::encode(&actual.code),
                    value["code"].as_str().unwrap(),
                    "{name}: code {key}"
                );
                let empty_root = keccak256([0x80]);
                let code_hash = keccak256(&actual.code);
                let mut disk = rlp::RlpStream::new_list(5);
                disk.append(&actual.nonce.to_bytes())
                    .append(&actual.balance)
                    .append_empty_data();
                if actual.code.is_empty() {
                    disk.append_empty_data();
                } else {
                    disk.append(&code_hash.as_slice());
                }
                disk.append(&(actual.code.len() as u64));
                assert_eq!(
                    hex::encode(disk.out()),
                    value["disk"].as_str().unwrap(),
                    "{name}: disk {key}"
                );
                let mut leaf = rlp::RlpStream::new_list(4);
                leaf.append(&actual.nonce.to_bytes())
                    .append(&actual.balance)
                    .append(&empty_root.as_slice())
                    .append(&code_hash.as_slice());
                let leaf = leaf.out().to_vec();
                assert_eq!(
                    hex::encode(&leaf),
                    value["leaf"].as_str().unwrap(),
                    "{name}: leaf {key}"
                );
                leaves.push((keccak256(address.as_slice()).to_vec(), leaf));
            }
            let root = triehash::trie_root::<keccak_hasher::KeccakHasher, _, _, _>(leaves);
            assert_eq!(
                hex::encode(root),
                row["root"].as_str().unwrap(),
                "{name}: root"
            );
        }
    }
}
