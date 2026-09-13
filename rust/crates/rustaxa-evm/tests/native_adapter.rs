//! Adversarial port results and real journal checkpoint behavior at native glue.
//! This verifies the S1 application contract, not native business-kernel parity.

use num_bigint::{BigInt, BigUint};
use rlp::RlpStream;
use rustaxa_evm::{contracts::*, journal::ExecutionJournal, native::*};
use rustaxa_types::{FinalChainNonce, concrete_state::*};

const EXISTING: [u8; 20] = [0x11; 20];
const CREATED: [u8; 20] = [0x22; 20];
const KEY: ConcreteStorageKey = ConcreteStorageKey([0x33; 32]);

struct Prior;
impl ConcreteStateRead for Prior {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: 0_u64.into(),
            state_root: [7; 32],
        }
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if address != EXISTING {
            return Ok(ConcreteRead::Absent);
        }
        let mut raw = RlpStream::new_list(5);
        raw.append(&1_u64)
            .append(&10_u64)
            .append_empty_data()
            .append_empty_data()
            .append(&0_u64);
        Ok(ConcreteRead::Present(ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: FinalChainNonce::from(1_u64),
                balance: ConcreteAccountBalance::new(BigUint::from(10_u8)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
            physical_rlp: raw.out().to_vec(),
        }))
    }
    fn storage(
        &self,
        address: [u8; 20],
        _: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(if address == EXISTING {
            ConcreteRead::Present(vec![0x80])
        } else {
            ConcreteRead::Absent
        })
    }
    fn code(&self, _: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        panic!("native result glue must not load bytecode")
    }
}

/// Deliberately untrusted output producer: tests inject invalid quote/result
/// facts so the adapter's validation is exercised independently of a kernel.
struct Port {
    bad_quote: bool,
    calls: usize,
    result: Option<NativeInvocationResult>,
}
impl NativeExecutionPort for Port {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let mut id = invocation.id;
        if self.bad_quote {
            id.sequence += 1;
        }
        Ok(NativeGasQuote {
            invocation: id,
            required_gas: 10_u64.into(),
        })
    }
    fn invoke(
        &mut self,
        _: &NativeInvocation,
        _: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.calls += 1;
        Ok(self.result.take().expect("test port cannot be replayed"))
    }
}
fn request(address: [u8; 20], gas: u64) -> NativeInvocation {
    NativeInvocation {
        id: NativeInvocationId {
            transaction: 0_u32.into(),
            sequence: 0,
        },
        period: 1_u64.into(),
        depth: 1,
        kind: NativeCallKind::Call,
        is_static: false,
        caller: EXISTING,
        contract: address,
        state_address: address,
        value: ExecutionValue::default(),
        input: vec![],
        supplied_gas: gas.into(),
    }
}
fn outcome() -> NativeOutcome {
    NativeOutcome {
        status: NativeStatus::Success,
        gas_used: 10_u64.into(),
        output: vec![0xee],
        account_mutations: vec![],
        raw_mutations: vec![],
        logs: vec![],
        diagnostic: None,
    }
}
fn port(outcome: NativeOutcome) -> Port {
    Port {
        bad_quote: false,
        calls: 0,
        result: Some(NativeInvocationResult::Completed(outcome)),
    }
}
fn put(bytes: &[u8]) -> NativeRawOperation {
    NativeRawOperation::Put(NativeRawValue::new(bytes.to_vec()).unwrap())
}
fn log(address: [u8; 20]) -> ExecutionLog {
    ExecutionLog {
        address,
        topics: vec![[4; 32]],
        data: vec![5],
    }
}

#[test]
fn quote_identity_is_checked_before_port_invocation() {
    let mut journal = ExecutionJournal::new(Prior);
    let mut port = port(outcome());
    port.bad_quote = true;
    assert_eq!(
        invoke_native(&mut journal, &mut port, &request(EXISTING, 100)),
        Err(NativeAdapterError::InvalidResult(
            NativeResultValidationError::InvocationMismatch
        ))
    );
    assert_eq!(port.calls, 0);
    assert_eq!(
        journal.account(EXISTING).unwrap().balance.value(),
        &BigInt::from(10)
    );
}

#[test]
fn invalid_charged_gas_cannot_apply_returned_mutations() {
    let mut journal = ExecutionJournal::new(Prior);
    let mut result = outcome();
    result.gas_used = 9_u64.into();
    result.raw_mutations.push(NativeRawMutation {
        address: EXISTING,
        key: KEY,
        expected: ConcreteRead::Present(vec![0x80]),
        operation: put(&[1]),
    });
    let mut port = port(result);
    assert_eq!(
        invoke_native(&mut journal, &mut port, &request(EXISTING, 100)),
        Err(NativeAdapterError::InvalidResult(
            NativeResultValidationError::ChargedGasMismatch
        ))
    );
    assert_eq!(
        journal.raw_storage(EXISTING, KEY).unwrap(),
        ConcreteRead::Present(vec![0x80])
    );
}

#[test]
fn completed_failure_preserves_exact_raw_order_through_frame_revert() {
    let mut journal = ExecutionJournal::new(Prior);
    let checkpoint = journal.checkpoint();
    let mut result = outcome();
    let failure = NativeContractFailure {
        error: "exact native error".into(),
    };
    result.status = NativeStatus::ContractFailure(failure.clone());
    result
        .account_mutations
        .push(NativeOrdinaryAccountMutation::Balance {
            address: EXISTING,
            expected_exists: true,
            expected: ExecutionBalance::new(BigInt::from(10)),
            replacement: ExecutionBalance::new(BigInt::from(4)),
        });
    result.raw_mutations = vec![
        NativeRawMutation {
            address: EXISTING,
            key: KEY,
            expected: ConcreteRead::Present(vec![0x80]),
            operation: put(&[0]),
        },
        NativeRawMutation {
            address: EXISTING,
            key: KEY,
            expected: ConcreteRead::Present(vec![0]),
            operation: NativeRawOperation::Delete,
        },
        NativeRawMutation {
            address: EXISTING,
            key: KEY,
            expected: ConcreteRead::Present(vec![]),
            operation: put(&[0, 2]),
        },
    ];
    result.logs.push(log(EXISTING));
    let completed =
        invoke_native(&mut journal, &mut port(result), &request(EXISTING, 100)).unwrap();
    assert_eq!(
        completed.status,
        CodeExecutionStatus::Failure(CodeExecutionError::Native(failure))
    );
    assert_eq!(completed.gas_left.as_u64(), 90);
    assert_eq!(completed.required_gas.as_u64(), 10);
    assert_eq!(completed.output, vec![0xee]);
    assert_eq!(
        journal.account(EXISTING).unwrap().balance.value(),
        &BigInt::from(4)
    );
    assert_eq!(journal.logs().len(), 1);
    journal.revert_checkpoint(checkpoint).unwrap();
    assert_eq!(
        journal.account(EXISTING).unwrap().balance.value(),
        &BigInt::from(10)
    );
    assert!(journal.logs().is_empty());
    assert_eq!(
        journal.raw_storage(EXISTING, KEY).unwrap(),
        ConcreteRead::Present(vec![0, 2])
    );
    assert_eq!(
        journal.settle_transaction().unwrap().writes.raw_storage[0].operation,
        put(&[0, 2])
    );
}

#[test]
fn new_account_rollback_removes_its_native_raw_overlay() {
    let mut journal = ExecutionJournal::new(Prior);
    let checkpoint = journal.checkpoint();
    // Account creation/touch belongs to the frame before native preparation.
    journal.touch_account(CREATED).unwrap();
    let mut result = outcome();
    result.raw_mutations.push(NativeRawMutation {
        address: CREATED,
        key: KEY,
        expected: ConcreteRead::Absent,
        operation: put(&[0]),
    });
    result.logs.push(log(CREATED));
    invoke_native(&mut journal, &mut port(result), &request(CREATED, 100)).unwrap();
    assert_eq!(
        journal.raw_storage(CREATED, KEY).unwrap(),
        ConcreteRead::Present(vec![0])
    );
    journal.revert_checkpoint(checkpoint).unwrap();
    assert!(!journal.account(CREATED).unwrap().exists);
    assert_eq!(
        journal.raw_storage(CREATED, KEY).unwrap(),
        ConcreteRead::Absent
    );
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .writes
            .raw_storage
            .is_empty()
    );
}

#[test]
fn raw_expectations_do_not_normalize_empty_or_tombstoned_values() {
    for expected in [
        ConcreteRead::Absent,
        ConcreteRead::Tombstone,
        ConcreteRead::Present(vec![]),
    ] {
        let mut journal = ExecutionJournal::new(Prior);
        let mut result = outcome();
        result.raw_mutations.push(NativeRawMutation {
            address: EXISTING,
            key: KEY,
            expected: expected.clone(),
            operation: put(&[9]),
        });
        assert_eq!(
            invoke_native(&mut journal, &mut port(result), &request(EXISTING, 100)),
            Err(NativeAdapterError::RawExpectation {
                address: EXISTING,
                key: KEY,
                expected,
                observed: ConcreteRead::Present(vec![0x80])
            })
        );
        assert_eq!(
            journal.raw_storage(EXISTING, KEY).unwrap(),
            ConcreteRead::Present(vec![0x80])
        );
    }
}

#[test]
fn insufficient_native_gas_retains_all_supplied_child_gas() {
    let mut journal = ExecutionJournal::new(Prior);
    let mut port = Port {
        bad_quote: false,
        calls: 0,
        result: Some(NativeInvocationResult::InsufficientGas {
            required_gas: 10_u64.into(),
        }),
    };
    let completed = invoke_native(&mut journal, &mut port, &request(EXISTING, 9)).unwrap();
    assert_eq!(
        completed.status,
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
    );
    assert_eq!(completed.gas_left.as_u64(), 9);
    assert_eq!(completed.required_gas.as_u64(), 10);
    assert!(completed.output.is_empty());
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .writes
            .accounts
            .is_empty()
    );
}
