//! Full-width ordinary SSTORE gas and rollback checks.
//!
//! Taraxa persists arbitrary-width positive storage integers even though EVM
//! stack operands remain 256-bit words. These tests verify that the REVM host
//! preserves the full-width equality and zero relations used by Taraxa's gas
//! rules while the journal stores the actual new EVM word.

use std::collections::BTreeMap;

use num_bigint::BigUint;
use revm::{
    context_interface::{Host, context::SStoreResult},
    primitives::{Address, U256},
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, ExecutionBlockContext, ExecutionGasPrice,
        ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
    },
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

const SENDER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
];
const TARGET: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbb,
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
