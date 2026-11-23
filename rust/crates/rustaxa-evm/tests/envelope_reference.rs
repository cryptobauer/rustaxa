//! Envelope checks against the 16 pinned-Go reference cases.

use num_bigint::{BigInt, BigUint};
use rustaxa_evm::{
    contracts::{
        ConsensusFailure, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, TransactionExecutionResult,
    },
    envelope::{
        EnvelopeAdmission, EnvelopeRules, FrameSettlement, FrameSettlementStatus,
        IntrinsicGasSchedule, admit, settle,
    },
    journal::ExecutionJournal,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};

const SENDER: [u8; 20] = {
    let mut address = [0_u8; 20];
    address[19] = 0xaa;
    address
};
const TARGET: [u8; 20] = [0xbb; 20];

#[derive(Clone)]
struct AccountReader {
    address: [u8; 20],
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    exists: bool,
}

impl ConcreteStateRead for AccountReader {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(1),
            state_root: [0x22; 32],
        }
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if address != self.address || !self.exists {
            return Ok(ConcreteRead::Absent);
        }
        Ok(ConcreteRead::Present(ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: self.nonce.clone(),
                balance: self.balance.clone(),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
            physical_rlp: vec![0xc0],
        }))
    }

    fn storage(
        &self,
        _address: [u8; 20],
        _key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }

    fn code(&self, _code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
}

#[test]
fn transfer_failure_retains_attempted_create_address_without_refund() {
    let mut journal = ExecutionJournal::new(AccountReader {
        address: SENDER,
        nonce: FinalChainNonce::zero(),
        balance: ConcreteAccountBalance::new(BigUint::from(100_000_u64)),
        exists: true,
    });
    let transaction = transaction(SENDER, None, 60_000, 1);
    let EnvelopeAdmission::Admitted(admitted) = admit(
        &mut journal,
        &transaction,
        EnvelopeRules { cornus: false },
        IntrinsicGasSchedule::PINNED,
    )
    .unwrap() else {
        panic!("transaction should admit");
    };
    let attempted = [0x55; 20];
    let result = settle(
        &mut journal,
        &transaction,
        &admitted,
        FrameSettlement {
            status: FrameSettlementStatus::InsufficientBalanceForTransfer,
            gas_left: admitted.action_gas,
            output: vec![0xaa],
            attempted_contract_address: Some(attempted),
        },
    )
    .unwrap();
    let TransactionExecutionResult::ConsensusFailure(failure) = result else {
        panic!("transfer failure must remain consensus-shaped");
    };
    assert_eq!(
        failure.error,
        ConsensusFailure::InsufficientBalanceForTransfer
    );
    assert_eq!(failure.gas_used, transaction.gas_limit);
    assert_eq!(failure.attempted_contract_address, Some(attempted));
    assert_eq!(failure.output, vec![0xaa]);
    assert_eq!(
        journal.account(SENDER).unwrap().balance.value(),
        &40_000_u64.into()
    );
}

#[test]
fn zero_sender_can_settle_negative_but_unsigned_persistence_rejects_it() {
    let mut journal = ExecutionJournal::new(AccountReader {
        address: [0_u8; 20],
        nonce: FinalChainNonce::zero(),
        balance: ConcreteAccountBalance::default(),
        exists: false,
    });
    let transaction = transaction([0_u8; 20], Some(TARGET), 21_000, 1);
    let EnvelopeAdmission::Admitted(admitted) = admit(
        &mut journal,
        &transaction,
        EnvelopeRules { cornus: false },
        IntrinsicGasSchedule::PINNED,
    )
    .unwrap() else {
        panic!("zero sender is affordability-exempt");
    };
    settle(
        &mut journal,
        &transaction,
        &admitted,
        FrameSettlement {
            status: FrameSettlementStatus::Success,
            gas_left: FinalChainGas::new(0),
            output: Vec::new(),
            attempted_contract_address: None,
        },
    )
    .unwrap();
    assert_eq!(
        journal.account([0_u8; 20]).unwrap().balance.value(),
        &(-21_000_i64).into()
    );
    assert!(matches!(
        journal.settle_transaction(),
        Err(rustaxa_evm::journal::JournalError::Balance(_))
    ));
}

#[test]
fn intrinsic_overflow_is_a_consensus_failure_after_upfront_charge() {
    let mut journal = ExecutionJournal::new(AccountReader {
        address: SENDER,
        nonce: FinalChainNonce::zero(),
        balance: ConcreteAccountBalance::new(BigUint::from(10_u8)),
        exists: true,
    });
    let mut transaction = transaction(SENDER, Some(TARGET), 1, 1);
    transaction.input.push(1);
    let admission = admit(
        &mut journal,
        &transaction,
        EnvelopeRules { cornus: false },
        IntrinsicGasSchedule {
            transaction: FinalChainGas::new(u64::MAX),
            creation: FinalChainGas::new(u64::MAX),
            zero_byte: FinalChainGas::new(1),
            nonzero_byte: FinalChainGas::new(1),
        },
    )
    .unwrap();
    let EnvelopeAdmission::Rejected(failure) = admission else {
        panic!("overflow must reject");
    };
    assert_eq!(failure.error, ConsensusFailure::IntrinsicGasOverflow);
    assert_eq!(failure.gas_used, FinalChainGas::new(1));
    assert_eq!(
        journal.account(SENDER).unwrap().balance.value(),
        &9_u8.into()
    );
}

#[test]
fn zero_debits_do_not_delete_existing_empty_sender() {
    for (gas, price, balance, expected_error) in [
        (20_000, 0, 0_u64, ConsensusFailure::IntrinsicGas),
        (
            60_000,
            3,
            1_u64,
            ConsensusFailure::InsufficientBalanceForGas,
        ),
    ] {
        let mut journal = ExecutionJournal::new(AccountReader {
            address: SENDER,
            nonce: FinalChainNonce::zero(),
            balance: ConcreteAccountBalance::new(BigUint::from(balance)),
            exists: true,
        });
        let transaction = transaction(SENDER, Some(TARGET), gas, price);
        let EnvelopeAdmission::Rejected(failure) = admit(
            &mut journal,
            &transaction,
            EnvelopeRules { cornus: false },
            IntrinsicGasSchedule::PINNED,
        )
        .unwrap() else {
            panic!("case must reject");
        };
        assert_eq!(failure.error, expected_error);
        assert_eq!(
            journal.settle_transaction().unwrap().writes,
            rustaxa_evm::journal::JournalWritePlan::default()
        );
    }
}

#[test]
fn refund_is_capped_and_credited_at_full_width_price() {
    let price = BigUint::from(1_u8) << 300_usize;
    let mut journal = ExecutionJournal::new(AccountReader {
        address: SENDER,
        nonce: FinalChainNonce::zero(),
        balance: ConcreteAccountBalance::new(&price * BigUint::from(100_000_u64)),
        exists: true,
    });
    let mut transaction = transaction(SENDER, Some(TARGET), 60_000, 1);
    transaction.gas_price = ExecutionGasPrice::new(price.clone());
    let EnvelopeAdmission::Admitted(admitted) = admit(
        &mut journal,
        &transaction,
        EnvelopeRules { cornus: false },
        IntrinsicGasSchedule::PINNED,
    )
    .unwrap() else {
        panic!("case must admit");
    };
    journal.add_refund(20_000).unwrap();
    let result = settle(
        &mut journal,
        &transaction,
        &admitted,
        FrameSettlement {
            status: FrameSettlementStatus::Success,
            gas_left: FinalChainGas::new(30_000),
            output: Vec::new(),
            attempted_contract_address: None,
        },
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("case must execute");
    };
    assert_eq!(result.gas_used, FinalChainGas::new(15_000));
    assert_eq!(
        journal.account(SENDER).unwrap().balance.value(),
        &BigInt::from(price * BigUint::from(85_000_u64))
    );
}

fn transaction(
    sender: [u8; 20],
    receiver: Option<[u8; 20]>,
    gas: u64,
    price: u64,
) -> ExecutionTransaction {
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender,
        receiver,
        nonce: FinalChainNonce::zero(),
        gas_price: ExecutionGasPrice::new(BigUint::from(price)),
        gas_limit: FinalChainGas::new(gas),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: if receiver.is_some() {
            ExecutionTransactionKind::Call
        } else {
            ExecutionTransactionKind::Create
        },
    }
}
