//! Adversarial port results and real journal checkpoint behavior at native glue.
//! This verifies the S1 application contract, not native business-kernel parity.

use num_bigint::{BigInt, BigUint};
use rlp::RlpStream;
use rustaxa_evm::{
    contracts::*,
    journal::{
        ExecutionJournal, JournalError, JournalLifecycleEvent, validate_lifecycle_receipt_token,
    },
    native::*,
};
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
    let settled = journal.settle_transaction().unwrap();
    assert_eq!(settled.writes.raw_storage[0].operation, put(&[0, 2]));
    assert_eq!(settled.native_invocations.len(), 1);
    let fact = &settled.native_invocations[0];
    assert_eq!(fact.invocation, request(EXISTING, 100));
    assert_eq!(fact.status, completed.status);
    assert_eq!(fact.output, completed.output);
    assert_eq!(fact.logs, vec![log(EXISTING)]);
    assert_eq!(
        fact.disposition,
        ConsensusNativeDisposition::OwnFrameReverted
    );
    assert_eq!(fact.required_gas.as_u64(), 10);
    assert_eq!(fact.gas_used.as_u64(), 10);
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
    let settled = journal.settle_transaction().unwrap();
    assert!(settled.writes.accounts.is_empty());
    assert_eq!(settled.native_invocations.len(), 1);
    let fact = &settled.native_invocations[0];
    assert_eq!(fact.invocation, request(EXISTING, 9));
    assert_eq!(fact.status, completed.status);
    assert_eq!(
        fact.disposition,
        ConsensusNativeDisposition::OwnFrameReverted
    );
    assert_eq!(fact.required_gas.as_u64(), 10);
    assert_eq!(fact.gas_used.as_u64(), 0);
    assert!(fact.output.is_empty());
    assert!(fact.logs.is_empty());
}

#[test]
fn rollback_marks_only_successful_observations_inside_its_checkpoint() {
    let mut journal = ExecutionJournal::new(Prior);
    let invoke = |journal: &mut ExecutionJournal<Prior>, sequence: u64, success: bool| {
        let mut request = request(EXISTING, 100);
        request.id.sequence = sequence;
        let mut result = outcome();
        result.logs.push(log(EXISTING));
        if !success {
            result.status = NativeStatus::ContractFailure(NativeContractFailure {
                error: "own failure".into(),
            });
        }
        invoke_native(journal, &mut port(result), &request).unwrap();
    };
    let first = journal.checkpoint();
    invoke(&mut journal, 0, true);
    journal.commit_checkpoint(first).unwrap();
    let outer = journal.checkpoint();
    let child = journal.checkpoint();
    invoke(&mut journal, 1, true);
    journal.commit_checkpoint(child).unwrap();
    let failed = journal.checkpoint();
    invoke(&mut journal, 2, false);
    journal.revert_checkpoint(failed).unwrap();
    journal.revert_checkpoint(outer).unwrap();
    let settled = journal.settle_transaction().unwrap();
    assert_eq!(settled.logs, vec![log(EXISTING)]);
    assert_eq!(
        settled
            .native_invocations
            .iter()
            .map(|fact| fact.disposition)
            .collect::<Vec<_>>(),
        vec![
            ConsensusNativeDisposition::Normal,
            ConsensusNativeDisposition::OuterFrameReverted,
            ConsensusNativeDisposition::OwnFrameReverted
        ]
    );
    for (sequence, fact) in settled.native_invocations.iter().enumerate() {
        assert_eq!(fact.invocation.id.sequence, sequence as u64);
        assert_eq!(fact.logs, vec![log(EXISTING)]);
        assert_eq!(fact.output, vec![0xee]);
    }
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .native_invocations
            .is_empty()
    );
}

#[test]
fn lifecycle_receipt_empty_transaction_and_stale_token_are_checked() {
    let mut first = ExecutionJournal::new(Prior);
    let expected_first = first.begin_lifecycle_receipt().unwrap();
    assert_eq!(
        first.begin_lifecycle_receipt(),
        Err(JournalError::InvalidLifecycleReceipt)
    );
    let first_receipt = first
        .settle_transaction()
        .unwrap()
        .lifecycle_receipt
        .unwrap();
    assert_eq!(
        first_receipt.events(),
        &[JournalLifecycleEvent::TransactionEnd { observations: 0 }]
    );
    validate_lifecycle_receipt_token(expected_first, &first_receipt).unwrap();
    let mut second = ExecutionJournal::new(Prior);
    let expected_second = second.begin_lifecycle_receipt().unwrap();
    let second_receipt = second
        .settle_transaction()
        .unwrap()
        .lifecycle_receipt
        .unwrap();
    assert_ne!(expected_first, expected_second);
    assert_eq!(first_receipt.events(), second_receipt.events());
    assert_eq!(
        validate_lifecycle_receipt_token(expected_second, &first_receipt),
        Err(JournalError::InvalidLifecycleReceipt)
    );
    validate_lifecycle_receipt_token(expected_second, &second_receipt).unwrap();
    assert!(
        first
            .settle_transaction()
            .unwrap()
            .lifecycle_receipt
            .is_none()
    );
    let next = first.begin_lifecycle_receipt().unwrap();
    assert_ne!(next, expected_first);
    let next_receipt = first
        .settle_transaction()
        .unwrap()
        .lifecycle_receipt
        .unwrap();
    assert_eq!(
        validate_lifecycle_receipt_token(expected_first, &next_receipt),
        Err(JournalError::InvalidLifecycleReceipt)
    );
}

#[test]
fn lifecycle_receipt_missing_begin_is_not_accepted_after_execution() {
    let mut journal = ExecutionJournal::new(Prior);
    invoke_native(&mut journal, &mut port(outcome()), &request(EXISTING, 100)).unwrap();
    assert_eq!(
        journal.begin_lifecycle_receipt(),
        Err(JournalError::InvalidLifecycleReceipt)
    );
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .lifecycle_receipt
            .is_none()
    );
}

#[test]
fn lifecycle_receipt_rejects_partial_native_on_expected_mismatch() {
    let mut journal = ExecutionJournal::new(Prior);
    journal.begin_lifecycle_receipt().unwrap();
    let mut result = outcome();
    result.raw_mutations.push(NativeRawMutation {
        address: EXISTING,
        key: KEY,
        expected: ConcreteRead::Absent,
        operation: put(&[7]),
    });
    assert!(matches!(
        invoke_native(&mut journal, &mut port(result), &request(EXISTING, 100)),
        Err(NativeAdapterError::RawExpectation {
            expected: ConcreteRead::Absent,
            observed: ConcreteRead::Present(_),
            ..
        })
    ));
    assert!(matches!(
        journal.settle_transaction(),
        Err(JournalError::InvalidLifecycleReceipt)
    ));
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .lifecycle_receipt
            .is_none()
    );
}

#[test]
fn lifecycle_receipt_records_creation_reset_and_preserves_existing_raw() {
    for (address, prior, should_reset) in [
        (CREATED, ConcreteRead::Absent, true),
        (EXISTING, ConcreteRead::Present(vec![0x80]), false),
    ] {
        let mut journal = ExecutionJournal::new(Prior);
        let expected = journal.begin_lifecycle_receipt().unwrap();
        let outer = journal.checkpoint();
        let mut result = outcome();
        result.raw_mutations.push(NativeRawMutation {
            address,
            key: KEY,
            expected: prior,
            operation: put(&[7]),
        });
        let completed =
            invoke_native(&mut journal, &mut port(result), &request(address, 100)).unwrap();
        assert_eq!(completed.status, CodeExecutionStatus::Success);
        assert_eq!(
            journal.raw_storage(address, KEY).unwrap(),
            ConcreteRead::Present(vec![7])
        );
        journal.revert_checkpoint(outer).unwrap();
        assert_eq!(
            journal.raw_storage(address, KEY).unwrap(),
            if should_reset {
                ConcreteRead::Absent
            } else {
                ConcreteRead::Present(vec![7])
            }
        );
        let settled = journal.settle_transaction().unwrap();
        let receipt = settled.lifecycle_receipt.as_ref().unwrap();
        validate_lifecycle_receipt_token(expected, receipt).unwrap();
        let starts = receipt
            .events()
            .iter()
            .filter(|event| matches!(event, JournalLifecycleEvent::NativeStarted { .. }))
            .count();
        let ends = receipt
            .events()
            .iter()
            .filter(|event| matches!(event, JournalLifecycleEvent::NativeObserved { .. }))
            .count();
        let resets: Vec<_> = receipt.events().iter().filter(|event| matches!(event, JournalLifecycleEvent::AccountReset { address: owner, .. } if *owner == address)).collect();
        assert_eq!(
            (starts, ends, resets.len()),
            (1, 1, usize::from(should_reset))
        );
        if should_reset {
            let events = receipt.events();
            let start = events
                .iter()
                .position(|event| matches!(event, JournalLifecycleEvent::NativeStarted { .. }))
                .unwrap();
            let creation = events.iter().position(|event| matches!(event, JournalLifecycleEvent::AccountCreated { address: owner, saved_absent: true, .. } if *owner == address)).unwrap();
            let observed = events
                .iter()
                .position(|event| matches!(event, JournalLifecycleEvent::NativeObserved { .. }))
                .unwrap();
            let reset = events.iter().position(|event| matches!(event, JournalLifecycleEvent::AccountReset { address: owner, .. } if *owner == address)).unwrap();
            assert!(start < creation && creation < observed && observed < reset);
        }
        assert!(matches!(
            receipt.events().last(),
            Some(JournalLifecycleEvent::TransactionEnd { observations: 1 })
        ));
        assert_eq!(
            settled.native_invocations[0].disposition,
            ConsensusNativeDisposition::OuterFrameReverted
        );
        if should_reset {
            assert!(settled.writes.raw_storage.is_empty());
        } else {
            assert_eq!(settled.writes.raw_storage[0].operation, put(&[7]));
        }
    }
}

#[test]
fn lifecycle_receipt_preserves_native_failure_and_outer_failure_effects() {
    for native_failure in [false, true] {
        let mut journal = ExecutionJournal::new(Prior);
        let expected = journal.begin_lifecycle_receipt().unwrap();
        let outer = journal.checkpoint();
        let mut result = outcome();
        let failure = NativeContractFailure {
            error: "native failure".into(),
        };
        if native_failure {
            result.status = NativeStatus::ContractFailure(failure.clone());
        }
        result.raw_mutations = vec![
            NativeRawMutation {
                address: EXISTING,
                key: KEY,
                expected: ConcreteRead::Present(vec![0x80]),
                operation: put(&[1]),
            },
            NativeRawMutation {
                address: EXISTING,
                key: KEY,
                expected: ConcreteRead::Present(vec![1]),
                operation: put(&[2]),
            },
        ];
        let completed =
            invoke_native(&mut journal, &mut port(result), &request(EXISTING, 100)).unwrap();
        assert_eq!(
            completed.status,
            if native_failure {
                CodeExecutionStatus::Failure(CodeExecutionError::Native(failure))
            } else {
                CodeExecutionStatus::Success
            }
        );
        assert_eq!(
            journal.raw_storage(EXISTING, KEY).unwrap(),
            ConcreteRead::Present(vec![2])
        );
        journal.revert_checkpoint(outer).unwrap();
        assert_eq!(
            journal.raw_storage(EXISTING, KEY).unwrap(),
            ConcreteRead::Present(vec![2])
        );
        let settled = journal.settle_transaction().unwrap();
        validate_lifecycle_receipt_token(expected, settled.lifecycle_receipt.as_ref().unwrap())
            .unwrap();
        assert_eq!(settled.writes.raw_storage[0].operation, put(&[2]));
        assert_eq!(settled.native_invocations[0].status, completed.status);
        assert_eq!(
            settled.native_invocations[0].disposition,
            if native_failure {
                ConsensusNativeDisposition::OwnFrameReverted
            } else {
                ConsensusNativeDisposition::OuterFrameReverted
            }
        );
        assert!(
            !settled
                .lifecycle_receipt
                .unwrap()
                .events()
                .iter()
                .any(|event| matches!(event, JournalLifecycleEvent::AccountReset { .. }))
        );
    }
}

#[test]
fn native_application_observations_bind_actual_order_and_mutations() {
    use rustaxa_evm::journal::{duration_journal_arm, duration_journal_take};
    duration_journal_arm(1_048_576).unwrap();
    let mut journal = ExecutionJournal::new(Prior);
    let invocation = request(EXISTING, 100);
    let mut result = outcome();
    result
        .account_mutations
        .push(NativeOrdinaryAccountMutation::Nonce {
            address: EXISTING,
            expected_exists: true,
            expected: FinalChainNonce::from(1_u64),
            replacement: FinalChainNonce::from(2_u64),
        });
    result.raw_mutations.push(NativeRawMutation {
        address: EXISTING,
        key: KEY,
        expected: ConcreteRead::Present(vec![0x80]),
        operation: put(&[7]),
    });
    result.raw_mutations.push(NativeRawMutation {
        address: EXISTING,
        key: KEY,
        expected: ConcreteRead::Present(vec![7]),
        operation: put(&[7]),
    });
    invoke_native(&mut journal, &mut port(result), &invocation).unwrap();
    let events = duration_journal_take().unwrap();
    let mut identity = u64::from(invocation.id.transaction.as_u32())
        .to_be_bytes()
        .to_vec();
    identity.extend_from_slice(&invocation.id.sequence.to_be_bytes());
    identity.extend_from_slice(&invocation.depth.to_be_bytes());
    let markers = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind.starts_with("native_apply_"))
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 6);
    for (position, pair) in markers.as_chunks::<2>().0.iter().enumerate() {
        let (begin_index, begin) = pair[0];
        let (end_index, end) = pair[1];
        let raw = position > 0;
        assert_eq!(
            begin.kind,
            if raw {
                "native_apply_raw_begin"
            } else {
                "native_apply_account_begin"
            }
        );
        assert_eq!(
            end.kind,
            if raw {
                "native_apply_raw_end"
            } else {
                "native_apply_account_end"
            }
        );
        assert_eq!(begin.address, EXISTING);
        assert_eq!(begin.key, if raw { KEY.0.to_vec() } else { Vec::new() });
        assert_eq!(begin.value, end.value);
        assert_eq!(begin.value[0], 1);
        assert_eq!(&begin.value[1..19], identity);
        assert_eq!(
            u64::from_be_bytes(begin.value[19..27].try_into().unwrap()),
            if raw { (position - 1) as u64 } else { 0 }
        );
        let applied = &events[end_index - 1];
        assert_eq!(
            applied.kind,
            if raw {
                "applied_raw"
            } else {
                "applied_account"
            }
        );
        assert_eq!(applied.address, begin.address);
        assert_eq!(applied.key, begin.key);
        assert_eq!(applied.value, begin.value[27..]);
        assert!(begin_index < end_index);
        assert!(
            events[begin_index + 1..end_index]
                .iter()
                .any(|event| event.kind == "read_account")
        );
        if raw {
            assert!(
                events[begin_index + 1..end_index]
                    .iter()
                    .any(|event| event.kind == "read_raw_present")
            );
            assert!(
                events[begin_index + 1..end_index]
                    .iter()
                    .any(|event| event.kind == "raw_write" && event.value == [7])
            );
        }
    }
    assert_eq!(
        journal.account(EXISTING).unwrap().nonce,
        FinalChainNonce::from(2_u64)
    );
    assert_eq!(
        journal.raw_storage(EXISTING, KEY).unwrap(),
        ConcreteRead::Present(vec![7])
    );
}

#[test]
fn native_application_failure_retains_begin_without_success_terminal() {
    use rustaxa_evm::journal::{duration_journal_arm, duration_journal_take};
    duration_journal_arm(1_048_576).unwrap();
    let mut journal = ExecutionJournal::new(Prior);
    let mut result = outcome();
    result.raw_mutations.push(NativeRawMutation {
        address: EXISTING,
        key: KEY,
        expected: ConcreteRead::Absent,
        operation: put(&[7]),
    });
    assert!(matches!(
        invoke_native(&mut journal, &mut port(result), &request(EXISTING, 100)),
        Err(NativeAdapterError::RawExpectation { .. })
    ));
    let events = duration_journal_take().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "native_apply_raw_begin")
            .count(),
        1
    );
    assert!(!events.iter().any(|event| matches!(
        event.kind,
        "native_apply_raw_end" | "applied_raw" | "raw_write"
    )));
    assert!(
        events
            .iter()
            .any(|event| event.kind == "read_raw_present" && event.value == [0x80])
    );
    assert_eq!(events.last().unwrap().kind, "native_end");
}

#[test]
fn native_application_account_ordinals_and_partial_failure_are_actual() {
    use rustaxa_evm::journal::{duration_journal_arm, duration_journal_take};
    for wrong_second_expectation in [false, true] {
        duration_journal_arm(1_048_576).unwrap();
        let mut journal = ExecutionJournal::new(Prior);
        let mut result = outcome();
        result.account_mutations = vec![
            NativeOrdinaryAccountMutation::Nonce {
                address: EXISTING,
                expected_exists: true,
                expected: FinalChainNonce::from(1_u64),
                replacement: FinalChainNonce::from(2_u64),
            },
            NativeOrdinaryAccountMutation::Nonce {
                address: EXISTING,
                expected_exists: true,
                expected: FinalChainNonce::from(if wrong_second_expectation {
                    1_u64
                } else {
                    2_u64
                }),
                replacement: FinalChainNonce::from(3_u64),
            },
        ];
        result.raw_mutations.push(NativeRawMutation {
            address: EXISTING,
            key: KEY,
            expected: ConcreteRead::Present(vec![0x80]),
            operation: put(&[7]),
        });
        let completed = invoke_native(&mut journal, &mut port(result), &request(EXISTING, 100));
        let events = duration_journal_take().unwrap();
        let begins = events
            .iter()
            .filter(|event| event.kind == "native_apply_account_begin")
            .collect::<Vec<_>>();
        let ends = events
            .iter()
            .filter(|event| event.kind == "native_apply_account_end")
            .collect::<Vec<_>>();
        assert_eq!(begins.len(), 2);
        assert_eq!(
            u64::from_be_bytes(begins[0].value[19..27].try_into().unwrap()),
            0
        );
        assert_eq!(
            u64::from_be_bytes(begins[1].value[19..27].try_into().unwrap()),
            1
        );
        if wrong_second_expectation {
            assert!(matches!(
                completed,
                Err(NativeAdapterError::Journal(
                    JournalError::NativeExpectation { .. }
                ))
            ));
            assert_eq!(ends.len(), 1);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event.kind == "applied_account")
                    .count(),
                1
            );
            assert!(
                !events
                    .iter()
                    .any(|event| event.kind.starts_with("native_apply_raw_")
                        || event.kind == "raw_write")
            );
            assert_eq!(
                journal.account(EXISTING).unwrap().nonce,
                FinalChainNonce::from(2_u64)
            );
            assert_eq!(
                journal.raw_storage(EXISTING, KEY).unwrap(),
                ConcreteRead::Present(vec![0x80])
            );
        } else {
            completed.unwrap();
            assert_eq!(ends.len(), 2);
            assert_eq!(ends[0].value, begins[0].value);
            assert_eq!(ends[1].value, begins[1].value);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event.kind == "native_apply_raw_end")
                    .count(),
                1
            );
            assert_eq!(
                journal.account(EXISTING).unwrap().nonce,
                FinalChainNonce::from(3_u64)
            );
            assert_eq!(
                journal.raw_storage(EXISTING, KEY).unwrap(),
                ConcreteRead::Present(vec![7])
            );
        }
        assert_eq!(events.last().unwrap().kind, "native_end");
    }
}

#[test]
fn native_outcome_status_observes_success_failure_and_insufficient_gas() {
    use rustaxa_evm::journal::{duration_journal_arm, duration_journal_take};
    for tag in 0_u8..=2 {
        duration_journal_arm(1_048_576).unwrap();
        let mut journal = ExecutionJournal::new(Prior);
        let invocation = request(EXISTING, if tag == 2 { 9 } else { 100 });
        let mut result = outcome();
        if tag == 1 {
            result.status = NativeStatus::ContractFailure(NativeContractFailure {
                error: String::new(),
            });
        }
        let mut native_port = port(result);
        if tag == 2 {
            native_port.result = Some(NativeInvocationResult::InsufficientGas {
                required_gas: 10_u64.into(),
            });
        }
        let completed = invoke_native(&mut journal, &mut native_port, &invocation).unwrap();
        let events = duration_journal_take().unwrap();
        let statuses = events
            .iter()
            .enumerate()
            .filter(|(_, event)| event.kind == "outcome_status")
            .collect::<Vec<_>>();
        if tag == 2 {
            assert!(statuses.is_empty());
            assert!(
                events
                    .iter()
                    .any(|event| event.kind == "outcome_insufficient_gas")
            );
            assert_eq!(
                completed.status,
                CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
            );
        } else {
            assert_eq!(statuses.len(), 1);
            let (index, status) = statuses[0];
            let mut identity = u64::from(invocation.id.transaction.as_u32())
                .to_be_bytes()
                .to_vec();
            identity.extend_from_slice(&invocation.id.sequence.to_be_bytes());
            identity.extend_from_slice(&invocation.depth.to_be_bytes());
            assert_eq!(status.address, [0; 20]);
            assert_eq!(status.key, identity);
            assert_eq!(status.value, [1, tag]);
            assert_eq!(events[index - 1].kind, "outcome_begin");
            assert_eq!(completed.status == CodeExecutionStatus::Success, tag == 0);
        }
    }
}
