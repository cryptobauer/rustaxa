//! Isolated compatibility probes, never linked by a production target.
//! Tests compare independently encoded commitments with pinned Go evidence and
//! exercise REVM interface boundaries. They do not implement a Taraxa backend.
#[cfg(test)]
mod host;

#[cfg(test)]
mod tests {
    use super::host::ProbeHost;
    use revm::{
        bytecode::Bytecode,
        context_interface::cfg::GasParams,
        interpreter::{
            FrameInput, Interpreter, InterpreterAction, instruction_table,
            instructions::gas_table_spec,
        },
        primitives::{U256, hardfork::SpecId},
    };
    use rustaxa_types::final_chain::FinalChainNonce;
    use serde_json::Value;

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../fixtures/local.json")).unwrap()
    }
    fn bytes(v: &Value) -> Vec<u8> {
        hex::decode(v.as_str().unwrap()).unwrap()
    }
    fn root(leaves: Vec<(Vec<u8>, Vec<u8>)>) -> String {
        hex::encode(triehash::trie_root::<keccak_hasher::KeccakHasher, _, _, _>(
            leaves,
        ))
    }

    #[test]
    fn independent_commitments_match_go() {
        for c in fixtures()["commitments"].as_array().unwrap() {
            let leaves = if c["kind"] == "account" {
                let n =
                    num_bigint::BigUint::parse_bytes(c["nonce"].as_str().unwrap().as_bytes(), 10)
                        .unwrap();
                let nonce = FinalChainNonce::from_bytes(&n.to_bytes_be()).unwrap();
                let mut disk = rlp::RlpStream::new_list(5);
                disk.append(&nonce.to_bytes())
                    .append(&100u64)
                    .append_empty_data()
                    .append_empty_data()
                    .append(&0u64);
                assert_eq!(disk.out().as_ref(), bytes(&c["disk"]));
                let mut leaf = rlp::RlpStream::new_list(4);
                leaf.append(&nonce.to_bytes()).append(&100u64);
                leaf.append(
                    &hex::decode(
                        "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                    )
                    .unwrap(),
                );
                leaf.append(
                    &hex::decode(
                        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
                    )
                    .unwrap(),
                );
                let leaf = leaf.out().to_vec();
                assert_eq!(leaf, bytes(&c["leaf"]));
                vec![(bytes(&c["key"]), leaf)]
            } else {
                c["values"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| {
                        let leaf = rlp::encode(&bytes(v)).to_vec();
                        assert_eq!(leaf, bytes(&c["leaves"][k]));
                        (hex::decode(k).unwrap(), leaf)
                    })
                    .collect()
            };
            assert_eq!(root(leaves), c["root"].as_str().unwrap());
        }
    }

    #[test]
    fn native_ordered_mutations_commit_exact_bytes() {
        use std::collections::BTreeMap;
        let mut state = BTreeMap::new();
        for step in fixtures()["native_iterable"].as_array().unwrap() {
            for write in step["writes"].as_array().unwrap() {
                state.insert(
                    write["key"].as_str().unwrap().to_owned(),
                    bytes(&write["value"]),
                );
            }
            let expected = step["rows_including_tombstones"].as_object().unwrap();
            assert_eq!(state.len(), expected.len());
            for (key, value) in &state {
                assert_eq!(*value, bytes(&expected[key]));
            }
            let leaves = state
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| {
                    (
                        revm::primitives::keccak256(hex::decode(k).unwrap()).to_vec(),
                        rlp::encode(v).to_vec(),
                    )
                })
                .collect();
            assert_eq!(root(leaves), step["storage_root"].as_str().unwrap());
        }
        // Zero count is four actual zero bytes, not a deletion. The empty map
        // therefore retains a nonempty commitment. Never normalize raw bytes.
        let live: Vec<_> = state.values().filter(|v| !v.is_empty()).collect();
        assert_eq!(live, vec![&vec![0, 0, 0, 0]]);
    }

    #[test]
    fn wide_create_address_matches_reference_using_native_nonce() {
        for row in fixtures()["envelopes"].as_array().unwrap() {
            if row["case"] != "create-wide" {
                continue;
            }
            let nonce =
                FinalChainNonce::from_bytes(&hex::decode("010000000000000000").unwrap()).unwrap();
            let mut rlp = rlp::RlpStream::new_list(2);
            let mut address = vec![0; 20];
            address[19] = 0xaa;
            rlp.append(&address).append(&nonce.to_bytes());
            let hash = revm::primitives::keccak256(rlp.out());
            assert_eq!(&hash[12..], bytes(&row["created"]));
        }
    }

    fn run(code: &[u8], price: U256) -> (Interpreter, InterpreterAction) {
        let mut host = ProbeHost {
            price,
            gas: GasParams::new_spec(SpecId::ISTANBUL),
        };
        let mut i = Interpreter::default().with_bytecode(Bytecode::new_raw(code.to_vec().into()));
        i.runtime_flag.spec_id = SpecId::ISTANBUL;
        let action = i.run_plain(
            &instruction_table(),
            &gas_table_spec(SpecId::ISTANBUL),
            &mut host,
        );
        (i, action)
    }

    #[test]
    fn interpreter_preserves_price_above_u128() {
        let price = U256::from(1) << 128;
        let (i, a) = run(&[0x3a, 0x00], price);
        assert!(a.is_return());
        assert_eq!(i.stack.data(), &[price]);
    }

    #[test]
    fn interpreter_yields_create_without_narrowing_nonce() {
        for code in [
            &[0x60, 0, 0x60, 0, 0x60, 0, 0xf0][..],
            &[0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0xf5][..],
        ] {
            let (_, action) = run(code, U256::ZERO);
            assert!(matches!(
                action,
                InterpreterAction::NewFrame(FrameInput::Create(_))
            ));
            // No account load happened: the fail-closed host would panic. Frame
            // owner can retain the existing native nonce without a shadow u64.
            let nonce = FinalChainNonce::from_bytes(&[0xff; 32]).unwrap().next();
            assert_eq!(nonce.to_bytes(), [vec![1], vec![0; 32]].concat());
        }
    }

    #[test]
    fn framework_rejects_nonce_skipping_before_execution() {
        use revm::context_interface::result::{EVMError, InvalidTransaction};
        use revm::{Context, ExecuteEvm, MainBuilder, MainContext};
        let mut evm = Context::mainnet().build_mainnet();
        let tx = revm::context::TxEnv::builder().nonce(7).build().unwrap();
        let err = evm.transact(tx).unwrap_err();
        assert!(
            matches!(
                err,
                EVMError::Transaction(InvalidTransaction::NonceTooHigh { .. })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn framework_fields_require_lossy_adapters_for_valid_taraxa_values() {
        use revm::context_interface::Transaction;
        let tx = revm::context::TxEnv::default();
        let _: u64 = tx.nonce();
        let _: u128 = tx.gas_price();
        let account = revm::state::AccountInfo::default();
        let _: u64 = account.nonce;
        // Checked conversion rejection is the evidence; do not truncate values
        // and then claim a successful framework transaction.
        assert!(u64::try_from(1u128 << 64).is_err());
        assert!(u128::try_from(U256::from(1) << 128).is_err());
    }
}
