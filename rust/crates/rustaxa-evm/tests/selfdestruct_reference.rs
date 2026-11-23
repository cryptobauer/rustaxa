//! Pinned Go SELFDESTRUCT frame, refund, visibility and logical settlement parity.

use std::collections::BTreeMap;

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
        ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
        TransactionExecutionResult,
    },
    driver::{NativeAddressClassifier, execute_top_level_call},
    envelope::EnvelopeRules,
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
fn beneficiary_quote_needs_only_emptiness_metadata() {
    let mut destination = [0; 20];
    destination[19] = 0xdd;
    let mut code = vec![0x73];
    code.extend_from_slice(&destination);
    code.push(0xff);
    let hash = keccak256(&code).0;
    let mut source = account(1, BigUint::default());
    source.code_hash = Some(hash);
    source.code_size = code.len() as u64;
    // The beneficiary's code reference is deliberately incomplete. SELFDESTRUCT
    // only needs its semantic nonemptiness; EXTCODE operations stay strict.
    let mut beneficiary = account(0, BigUint::default());
    beneficiary.code_size = 1;
    let mut journal = ExecutionJournal::new(Reader {
        accounts: BTreeMap::from([
            (SENDER, account(1, BigUint::from(1_000_000_u64))),
            (TARGET, source),
            (destination, beneficiary),
        ]),
        codes: BTreeMap::from([(hash, code)]),
        storage: BTreeMap::new(),
    });
    let TransactionExecutionResult::Executed(result) = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(),
        &transaction(),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap() else {
        panic!("must execute")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used.as_u64(), 13_002);
    let settled = journal.settle_transaction().unwrap();
    assert!(
        !settled
            .writes
            .accounts
            .iter()
            .any(|write| write.address == destination)
    );
}

#[test]
fn signed_suicide_transfer_and_self_transfer_restore_on_revert() {
    use num_bigint::BigInt;
    use rustaxa_evm::contracts::ExecutionBalance;
    let mut destination = [0; 20];
    destination[19] = 0xdd;
    for beneficiary in [destination, TARGET] {
        let mut journal = ExecutionJournal::new(Reader {
            accounts: BTreeMap::from([
                (TARGET, account(1, BigUint::from(7_u8))),
                (destination, account(1, BigUint::from(10_u8))),
            ]),
            codes: BTreeMap::new(),
            storage: BTreeMap::new(),
        });
        journal
            .set_balance(TARGET, ExecutionBalance::new(BigInt::from(-7)))
            .unwrap();
        let checkpoint = journal.checkpoint();
        journal.selfdestruct(TARGET, beneficiary).unwrap();
        assert_eq!(
            journal.account_metadata(TARGET).unwrap().balance.value(),
            &BigInt::from(0)
        );
        if beneficiary == destination {
            assert_eq!(
                journal
                    .account_metadata(destination)
                    .unwrap()
                    .balance
                    .value(),
                &BigInt::from(3)
            );
        }
        journal.revert_checkpoint(checkpoint).unwrap();
        assert_eq!(
            journal.account_metadata(TARGET).unwrap().balance.value(),
            &BigInt::from(-7)
        );
        assert_eq!(
            journal
                .account_metadata(destination)
                .unwrap()
                .balance
                .value(),
            &BigInt::from(10)
        );
    }
}

fn account(nonce: u64, balance: BigUint) -> ConcreteAccount {
    ConcreteAccount {
        nonce: FinalChainNonce::from_u64(nonce),
        balance: ConcreteAccountBalance::new(balance),
        storage_root: None,
        code_hash: None,
        code_size: 0,
    }
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
