//! Envelope checks against the 16 pinned-Go reference cases.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        CodeExecutionError, CodeExecutionStatus, ConsensusFailure, ExecutionGasPrice,
        ExecutionTransaction, ExecutionTransactionKind, ExecutionValue, TransactionExecutionResult,
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
use serde_json::Value;

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
fn envelope_matches_pinned_wide_nonce_price_and_failure_cases() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .expect("parse feasibility fixture");
    let rows = fixture["envelopes"].as_array().expect("envelope array");
    assert_eq!(rows.len(), 16);

    for row in rows {
        let input_nonce = parse_number(row["input_nonce"].as_str().unwrap());
        let mut journal = ExecutionJournal::new(AccountReader {
            address: SENDER,
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(parse_number(
                row["input_balance"].as_str().unwrap(),
            )),
            exists: true,
        });
        let transaction = ExecutionTransaction {
            position: FinalChainTransactionPosition::from(0_u32),
            hash: [0x11; 32],
            sender: SENDER,
            receiver: (!row["create"].as_bool().unwrap()).then_some(TARGET),
            nonce: nonce(input_nonce),
            gas_price: ExecutionGasPrice::new(parse_number(row["price"].as_str().unwrap())),
            gas_limit: FinalChainGas::new(row["gas_cap"].as_u64().unwrap()),
            value: ExecutionValue::default(),
            input: parse_hex(row["code"].as_str().unwrap()),
            canonical_rlp: None,
            kind: if row["create"].as_bool().unwrap() {
                ExecutionTransactionKind::Create
            } else {
                ExecutionTransactionKind::Call
            },
        };
        let admission = admit(
            &mut journal,
            &transaction,
            EnvelopeRules {
                cornus: row["cornus"].as_bool().unwrap(),
            },
            IntrinsicGasSchedule::PINNED,
        )
        .expect("admit transaction");

        let expected_consensus = row["consensus_error"].as_str().unwrap();
        if !expected_consensus.is_empty() {
            let EnvelopeAdmission::Rejected(result) = admission else {
                panic!("{} should be rejected", row["case"]);
            };
            assert_eq!(result.error, consensus_error(expected_consensus));
            assert_eq!(result.gas_used.as_u64(), row["gas_used"].as_u64().unwrap());
        } else {
            let EnvelopeAdmission::Admitted(admitted) = admission else {
                panic!("{} should be admitted", row["case"]);
            };
            if row["create"].as_bool().unwrap() {
                journal
                    .set_nonce(SENDER, transaction.nonce.next())
                    .expect("frame CREATE nonce increment");
            }
            let gas_used = row["gas_used"].as_u64().unwrap();
            let attempted = parse_address(row["created"].as_str().unwrap());
            let result = settle(
                &mut journal,
                &transaction,
                &admitted,
                FrameSettlement {
                    status: if row["execution_error"].as_str().unwrap().is_empty() {
                        FrameSettlementStatus::Success
                    } else {
                        FrameSettlementStatus::CodeFailure(CodeExecutionError::Revert)
                    },
                    gas_left: FinalChainGas::new(transaction.gas_limit.as_u64() - gas_used),
                    output: parse_hex(row["return"].as_str().unwrap()),
                    attempted_contract_address: attempted,
                },
            )
            .expect("settle transaction");
            let TransactionExecutionResult::Executed(executed) = result else {
                panic!("admitted fixture returned consensus failure");
            };
            assert_eq!(executed.gas_used.as_u64(), gas_used);
            assert_eq!(executed.attempted_contract_address, attempted);
            assert_eq!(
                executed.status,
                if row["execution_error"].as_str().unwrap().is_empty() {
                    CodeExecutionStatus::Success
                } else {
                    CodeExecutionStatus::Failure(CodeExecutionError::Revert)
                }
            );
        }

        let account = journal.account(SENDER).expect("read settled sender");
        assert_eq!(
            account.nonce,
            nonce(parse_number(row["nonce"].as_str().unwrap()))
        );
        assert_eq!(
            account.balance.value(),
            &parse_number(row["balance"].as_str().unwrap()).into()
        );
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
    let mut journal = ExecutionJournal::new(AccountReader {
        address: SENDER,
        nonce: FinalChainNonce::zero(),
        balance: ConcreteAccountBalance::new(BigUint::from(100_000_u64)),
        exists: true,
    });
    let transaction = transaction(SENDER, Some(TARGET), 60_000, 1);
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
        &85_000_u64.into()
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

fn consensus_error(value: &str) -> ConsensusFailure {
    match value {
        "nonce too low" => ConsensusFailure::NonceTooLow,
        "insufficient balance to pay for gas" => ConsensusFailure::InsufficientBalanceForGas,
        "intrinsic gas too low" => ConsensusFailure::IntrinsicGas,
        other => panic!("unknown consensus error {other}"),
    }
}

fn nonce(value: BigUint) -> FinalChainNonce {
    if value == BigUint::default() {
        FinalChainNonce::zero()
    } else {
        FinalChainNonce::from_bytes(&value.to_bytes_be()).unwrap()
    }
}

fn parse_number(value: &str) -> BigUint {
    let (digits, radix) = value
        .strip_prefix("0x")
        .map_or((value, 10), |digits| (digits, 16));
    BigUint::parse_bytes(digits.as_bytes(), radix).expect("integer")
}

fn parse_address(value: &str) -> Option<[u8; 20]> {
    let bytes = parse_hex(value);
    let address: [u8; 20] = bytes.try_into().expect("20-byte address");
    (address != [0_u8; 20]).then_some(address)
}

fn parse_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).expect("hex byte"))
        .collect()
}
