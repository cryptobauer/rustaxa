//! Full-width ordinary SSTORE gas and rollback checks.
//!
//! Taraxa persists arbitrary-width positive storage integers even though EVM
//! stack operands remain 256-bit words. These tests verify that the REVM host
//! preserves the full-width equality and zero relations used by Taraxa's gas
//! rules while the journal stores the actual new EVM word.

use std::{collections::BTreeMap, fs, path::PathBuf};

use num_bigint::BigUint;
use revm::{
    context_interface::{Host, context::SStoreResult},
    primitives::{Address, U256, keccak256},
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, TransactionExecutionResult,
    },
    driver::{NativeAddressClassifier, execute_top_level_call},
    envelope::EnvelopeRules,
    host::JournalHost,
    journal::ExecutionJournal,
    profile::TaraxaProfile,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::Value;

const SENDER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
];
const TARGET: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbb,
];
const CHILD: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xcc,
];
const KEY: ConcreteStorageKey = ConcreteStorageKey([0_u8; 32]);

struct Reader {
    accounts: BTreeMap<[u8; 20], ConcreteAccount>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
}

impl ConcreteStateRead for Reader {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(1),
            state_root: [0x44; 32],
        }
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        Ok(self
            .accounts
            .get(&address)
            .cloned()
            .map_or(ConcreteRead::Absent, |account| {
                ConcreteRead::Present(ConcreteAccountRecord {
                    account,
                    physical_rlp: vec![0xc0],
                })
            }))
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .storage
            .get(&(address, key))
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }

    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}

struct NoHistory;

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

impl BlockHashRead for NoHistory {
    fn block_hash(&self, _number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        unreachable!("predicate bytecode does not execute BLOCKHASH")
    }
}

#[test]
fn gas_carrier_preserves_every_full_width_equality_and_zero_relation() {
    let wide = (BigUint::from(1_u8) << 256_usize) + BigUint::from(7_u8);
    let wide_zero_low_word = BigUint::from(1_u8) << 256_usize;
    let other_wide = (BigUint::from(1_u8) << 257_usize) + BigUint::from(7_u8);
    let cases = [
        (BigUint::default(), BigUint::default(), U256::ZERO),
        (BigUint::default(), BigUint::default(), U256::from(7_u8)),
        (BigUint::default(), wide.clone(), U256::ZERO),
        (wide.clone(), BigUint::default(), U256::ZERO),
        (wide.clone(), wide.clone(), U256::from(7_u8)),
        (wide_zero_low_word.clone(), wide_zero_low_word, U256::ZERO),
        (wide.clone(), other_wide, U256::from(7_u8)),
        (wide.clone(), BigUint::from(7_u8), U256::from(7_u8)),
        (wide, BigUint::default(), U256::from(7_u8)),
    ];

    for (original, present, new_value) in cases {
        let (carrier, stored) = sstore(original.clone(), present.clone(), new_value);
        let new = word_biguint(new_value);
        assert_eq!(
            relations(&carrier),
            full_relations(&original, &present, &new),
            "original={original}, present={present}, new={new}"
        );
        assert_eq!(stored, new);
    }
}

#[test]
fn direct_and_nested_wide_sstore_match_both_pinned_go_references() {
    let local: Value = serde_json::from_slice(&fs::read(fixture_path("local")).unwrap()).unwrap();
    let public: Value = serde_json::from_slice(&fs::read(fixture_path("public")).unwrap()).unwrap();
    assert_eq!(local, public);
    let cases = local["wide_sstore"].as_array().unwrap();
    assert_eq!(cases.len(), 6);

    for case in cases {
        let name = case["case"].as_str().unwrap();
        let original = decimal(&case["original"]);
        let target_code = hex::decode(case["code"].as_str().unwrap()).unwrap();
        let mut codes_by_address = vec![(TARGET, target_code)];
        if let Some(child) = case.get("child_code") {
            codes_by_address.push((CHILD, hex::decode(child.as_str().unwrap()).unwrap()));
        }
        let mut accounts = BTreeMap::from([(
            SENDER,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        )]);
        let mut codes = BTreeMap::new();
        for (address, code) in codes_by_address {
            let hash = keccak256(&code).0;
            accounts.insert(
                address,
                ConcreteAccount {
                    nonce: FinalChainNonce::from_u64(1),
                    balance: ConcreteAccountBalance::default(),
                    storage_root: (address == TARGET).then_some([0x55; 32]),
                    code_hash: Some(hash),
                    code_size: code.len() as u64,
                },
            );
            codes.insert(hash, code);
        }
        let mut journal = ExecutionJournal::new(Reader {
            accounts,
            storage: BTreeMap::from([((TARGET, KEY), original.to_bytes_be())]),
            codes,
        });
        let mut transaction = transaction();
        transaction.gas_limit = FinalChainGas::new(case["gas_limit"].as_u64().unwrap());
        let result = execute_top_level_call(
            &mut journal,
            &NoHistory,
            &NoNative,
            &block(),
            &transaction,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("{name}: must execute")
        };
        assert_eq!(case["consensus_error"].as_str().unwrap(), "", "{name}");
        let expected_status = match case["execution_error"].as_str().unwrap() {
            "" => {
                assert_eq!(case["error"].as_str().unwrap(), "", "{name}");
                CodeExecutionStatus::Success
            }
            "execution reverted" => {
                assert_eq!(
                    case["error"].as_str().unwrap(),
                    "execution reverted",
                    "{name}"
                );
                CodeExecutionStatus::Failure(CodeExecutionError::Revert)
            }
            error => panic!("{name}: unsupported fixture execution error {error:?}"),
        };
        assert_eq!(result.status, expected_status, "{name}: status");
        assert_eq!(
            result.gas_used.as_u64(),
            case["gas_used"].as_u64().unwrap(),
            "{name}: gas"
        );
        assert_eq!(
            result.output,
            hex::decode(case["output"].as_str().unwrap()).unwrap(),
            "{name}: output"
        );
        assert!(result.logs.is_empty(), "{name}: logs");
        assert_eq!(
            journal.refund(),
            case["refund"].as_u64().unwrap(),
            "{name}: refund"
        );
        assert_eq!(
            journal.ordinary_storage(TARGET, KEY).unwrap().1,
            decimal(&case["storage"]),
            "{name}: storage"
        );
    }
}

fn sstore(original: BigUint, present: BigUint, new_value: U256) -> (SStoreResult, BigUint) {
    let storage = if original == BigUint::default() {
        BTreeMap::new()
    } else {
        BTreeMap::from([((TARGET, KEY), original.to_bytes_be())])
    };
    let mut journal = ExecutionJournal::new(Reader {
        accounts: BTreeMap::from([(
            TARGET,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: Some([0x55; 32]),
                code_hash: None,
                code_size: 0,
            },
        )]),
        storage,
        codes: BTreeMap::new(),
    });
    if present != original {
        journal.set_ordinary_storage(TARGET, KEY, present).unwrap();
    }
    let result = {
        let block = block();
        let transaction = transaction();
        let mut host = JournalHost::new(
            &mut journal,
            &NoHistory,
            &block,
            &transaction,
            TaraxaProfile::new(false).gas_params(),
        );
        host.sstore_skip_cold_load(Address::from(TARGET), U256::ZERO, new_value, false)
            .unwrap()
            .data
    };
    let stored = journal.ordinary_storage(TARGET, KEY).unwrap().1;
    (result, stored)
}

fn relations(values: &SStoreResult) -> [bool; 7] {
    [
        values.is_original_zero(),
        values.is_present_zero(),
        values.is_new_zero(),
        values.is_original_eq_present(),
        values.is_original_eq_new(),
        values.is_new_eq_present(),
        values.have_changed_from_zero(),
    ]
}

fn full_relations(original: &BigUint, present: &BigUint, new: &BigUint) -> [bool; 7] {
    let zero = BigUint::default();
    [
        original == &zero,
        present == &zero,
        new == &zero,
        original == present,
        original == new,
        new == present,
        original == &zero && new != &zero,
    ]
}

fn word_biguint(value: U256) -> BigUint {
    BigUint::from_bytes_be(&value.to_be_bytes::<32>())
}

fn decimal(value: &Value) -> BigUint {
    BigUint::parse_bytes(value.as_str().unwrap().as_bytes(), 10).unwrap()
}

fn fixture_path(reference: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../../experiments/evm_feasibility/fixtures/wide_sstore_{reference}.json"
    ))
}

fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(1),
        author: [0_u8; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn transaction() -> ExecutionTransaction {
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(TARGET),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}
