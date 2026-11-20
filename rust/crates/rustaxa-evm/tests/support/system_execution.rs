use super::*;
use ethereum_types::{H160, U256};
use rustaxa_evm::{
    driver::execute_top_level_system_call_with_native,
    input::{LegacyInputKind, decode_legacy_input},
};
use rustaxa_types::{
    LegacySystemTransactionInput, TARAXA_SYSTEM_ACCOUNT, encode_legacy_system_transaction,
};

fn system_input(target: [u8; 20], nonce: u64, gas: u64, input: Vec<u8>) -> ExecutionTransaction {
    let bytes = encode_legacy_system_transaction(&LegacySystemTransactionInput {
        nonce: U256::from(nonce),
        value: U256::zero(),
        gas_price: U256::zero(),
        gas,
        data: input,
        receiver: Some(H160(target)),
        chain_id: 0,
    });
    decode_legacy_input(0_u32.into(), &bytes, LegacyInputKind::System).unwrap()
}

fn execute(
    reader: Reader,
    owner: &Owner,
    transaction: &ExecutionTransaction,
    context: &ExecutionBlockContext,
) -> (TransactionExecutionResult, ExecutionJournal<Reader>) {
    let mut journal = ExecutionJournal::new_with_correction_stage(reader, STAGE);
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(context.period);
    let result = execute_top_level_system_call_with_native(
        &mut journal,
        &External,
        &External,
        &External,
        &mut port,
        &mut sequence,
        context,
        transaction,
        RULES,
        profile(STAGE),
        transaction.canonical_rlp.as_ref().unwrap(),
    )
    .unwrap();
    (result, journal)
}

#[test]
fn system_call_keeps_normal_gas_nonce_output_and_settlement() {
    let owner = Owner::new();
    let mut reader = Reader::funded();
    reader.accounts.insert(TARAXA_SYSTEM_ACCOUNT, account(7, 0));
    reader.add_code(
        PARENT,
        vec![
            0x60, 9, 0x60, 1, 0x55, 0x60, 0x2a, 0x60, 0, 0x52, 0x60, 32, 0x60, 0, 0xf3,
        ],
    );
    let transaction = system_input(PARENT, 7, block().gas_limit.as_u64(), vec![]);
    let (result, mut system) = execute(reader.clone(), &owner, &transaction, &block());
    let execution = completed(result.clone());
    assert_eq!(execution.status, CodeExecutionStatus::Success);
    assert!(execution.gas_used.as_u64() > 21_000);
    assert_eq!(execution.output.len(), 32);
    assert_eq!(execution.output[31], 0x2a);
    assert_eq!(
        system.account(TARAXA_SYSTEM_ACCOUNT).unwrap().nonce,
        FinalChainNonce::from_u64(8)
    );
    assert_eq!(
        system
            .account(TARAXA_SYSTEM_ACCOUNT)
            .unwrap()
            .balance
            .value(),
        &0.into()
    );
    assert_eq!(
        system.ordinary_storage(PARENT, slot_one()).unwrap().1,
        BigUint::from(9_u8)
    );
    let mut ordinary = ExecutionJournal::new_with_correction_stage(reader, STAGE);
    let mut call = transaction;
    call.kind = ExecutionTransactionKind::Call;
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    assert_eq!(
        result,
        run(
            &mut ordinary,
            &mut port,
            &mut PeriodConsensusSequence::new(1.into()),
            &call
        )
        .unwrap()
    );
    assert_eq!(
        system.settle_transaction().unwrap(),
        ordinary.settle_transaction().unwrap()
    );
}

#[test]
fn system_empty_code_revert_and_out_of_gas_keep_call_rules() {
    let owner = Owner::new();
    for (code, expected) in [
        (vec![], CodeExecutionStatus::Success),
        (
            vec![0x60, 9, 0x60, 1, 0x55, 0x60, 0, 0x60, 0, 0xfd],
            CodeExecutionStatus::Failure(CodeExecutionError::Revert),
        ),
        (
            vec![0x60, 0, 0x56],
            CodeExecutionStatus::Failure(CodeExecutionError::InvalidJump),
        ),
    ] {
        let mut reader = Reader::funded();
        reader.accounts.insert(TARAXA_SYSTEM_ACCOUNT, account(7, 0));
        if !code.is_empty() {
            reader.add_code(PARENT, code);
        }
        let transaction = system_input(PARENT, 7, block().gas_limit.as_u64(), vec![]);
        let (result, journal) = execute(reader, &owner, &transaction, &block());
        let execution = completed(result);
        assert_eq!(execution.status, expected);
        assert_eq!(
            journal.account(TARAXA_SYSTEM_ACCOUNT).unwrap().nonce,
            FinalChainNonce::from_u64(8)
        );
        assert_eq!(
            journal.ordinary_storage(PARENT, slot_one()).unwrap().1,
            BigUint::default()
        );
        if expected == CodeExecutionStatus::Success {
            assert_eq!(execution.gas_used.as_u64(), 21_000);
        }
    }
    let mut reader = Reader::funded();
    reader.accounts.insert(TARAXA_SYSTEM_ACCOUNT, account(7, 0));
    reader.add_code(PARENT, vec![0x5b, 0x60, 0, 0x56]);
    let transaction = system_input(PARENT, 7, 22_000, vec![]);
    let mut context = block();
    context.gas_limit = 22_000.into();
    let (result, journal) = execute(reader, &owner, &transaction, &context);
    assert_eq!(
        completed(result).status,
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
    );
    assert_eq!(
        journal.account(TARAXA_SYSTEM_ACCOUNT).unwrap().nonce,
        FinalChainNonce::from_u64(8)
    );
}

#[test]
fn system_envelope_failure_keeps_nonce_and_intrinsic_rules() {
    let owner = Owner::new();
    for (nonce, gas, expected, next) in [
        (6, 1_000_000, ConsensusFailure::NonceTooLow, 7),
        (7, 20_000, ConsensusFailure::IntrinsicGas, 8),
    ] {
        let mut reader = Reader::funded();
        reader.accounts.insert(TARAXA_SYSTEM_ACCOUNT, account(7, 0));
        let transaction = system_input(PARENT, nonce, gas, vec![]);
        let mut context = block();
        context.gas_limit = gas.into();
        let (result, journal) = execute(reader, &owner, &transaction, &context);
        let TransactionExecutionResult::ConsensusFailure(result) = result else {
            panic!("must reject envelope")
        };
        assert_eq!(result.error, expected);
        assert_eq!(
            journal.account(TARAXA_SYSTEM_ACCOUNT).unwrap().nonce,
            FinalChainNonce::from_u64(next)
        );
    }
}

#[test]
fn inconsistent_system_facts_fail_before_state_reads() {
    let owner = Owner::new();
    let good = system_input(PARENT, 7, block().gas_limit.as_u64(), vec![]);
    let mut bad = Vec::new();
    let mut change = |edit: fn(&mut ExecutionTransaction)| {
        let mut tx = good.clone();
        edit(&mut tx);
        bad.push(tx);
    };
    change(|tx| tx.kind = ExecutionTransactionKind::Call);
    change(|tx| tx.sender = [0; 20]);
    change(|tx| tx.receiver = None);
    change(|tx| tx.gas_price = ExecutionGasPrice::new(1_u8.into()));
    change(|tx| tx.value = ExecutionValue::new(1_u8.into()));
    change(|tx| tx.gas_limit = 99.into());
    change(|tx| tx.canonical_rlp = None);
    change(|tx| tx.hash = [0; 32]);
    change(|tx| tx.nonce = FinalChainNonce::from_u64(8));
    change(|tx| tx.input.push(1));
    for transaction in bad {
        let reader = Reader::funded();
        let reads = reader.reads.clone();
        let mut journal = ExecutionJournal::new_with_correction_stage(reader, STAGE);
        let before = reads.load(Ordering::Relaxed);
        let mut port = mixed_native::MixedNativeExecutionPort::new(
            owner
                .chain
                .begin_native_session(1.into(), 0.into())
                .unwrap(),
        );
        let result = execute_top_level_system_call_with_native(
            &mut journal,
            &External,
            &External,
            &External,
            &mut port,
            &mut PeriodConsensusSequence::new(1.into()),
            &block(),
            &transaction,
            RULES,
            profile(STAGE),
            good.canonical_rlp.as_ref().unwrap(),
        );
        assert!(matches!(
            result,
            Err(ExecutionDriverError::InvalidSystemTransaction(_))
        ));
        assert_eq!(reads.load(Ordering::Relaxed), before);
    }
}

#[test]
fn system_real_nested_native_hard_error_stays_typed() {
    let owner = Owner::new();
    let mut reader = Reader::funded();
    reader.accounts.insert(TARAXA_SYSTEM_ACCOUNT, account(7, 0));
    reader.add_code(PARENT, parent_code(20_000));
    let transaction = system_input(
        PARENT,
        7,
        block().gas_limit.as_u64(),
        malformed_inputs()[0].clone(),
    );
    let mut journal = ExecutionJournal::new_with_correction_stage(reader, STAGE);
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(1.into());
    let error = execute_top_level_system_call_with_native(
        &mut journal,
        &External,
        &External,
        &External,
        &mut port,
        &mut sequence,
        &block(),
        &transaction,
        RULES,
        profile(STAGE),
        transaction.canonical_rlp.as_ref().unwrap(),
    )
    .unwrap_err();
    let ExecutionDriverError::Native(error) = error else {
        panic!("must propagate native error")
    };
    assert_hard(error);
    assert_eq!(sequence.next_sequence(), 1);
    assert_eq!(
        journal.ordinary_storage(PARENT, slot_one()).unwrap().1,
        BigUint::default()
    );
}

#[test]
fn system_unsigned_encoding_and_ordinary_entry_stay_strict() {
    let owner = Owner::new();
    let good = system_input(PARENT, 7, block().gas_limit.as_u64(), vec![]);
    let bytes = good.canonical_rlp.as_ref().unwrap();
    let parsed = rlp::Rlp::new(bytes);
    for signature in [[1_u64, 0, 0], [27, 1, 0], [27, 0, 1]] {
        let mut encoded = RlpStream::new_list(9);
        for index in 0..6 {
            encoded.append_raw(parsed.at(index).unwrap().as_raw(), 1);
        }
        for value in signature {
            encoded.append(&value);
        }
        let encoded = encoded.out().to_vec();
        let transaction =
            decode_legacy_input(0_u32.into(), &encoded, LegacyInputKind::System).unwrap();
        let reader = Reader::funded();
        let reads = reader.reads.clone();
        let mut journal = ExecutionJournal::new_with_correction_stage(reader, STAGE);
        let before = reads.load(Ordering::Relaxed);
        let mut port = mixed_native::MixedNativeExecutionPort::new(
            owner
                .chain
                .begin_native_session(1.into(), 0.into())
                .unwrap(),
        );
        assert!(matches!(
            execute_top_level_system_call_with_native(
                &mut journal,
                &External,
                &External,
                &External,
                &mut port,
                &mut PeriodConsensusSequence::new(1.into()),
                &block(),
                &transaction,
                RULES,
                profile(STAGE),
                &encoded,
            ),
            Err(ExecutionDriverError::InvalidSystemTransaction(_))
        ));
        assert_eq!(reads.load(Ordering::Relaxed), before);
    }
    let mut journal = ExecutionJournal::new_with_correction_stage(Reader::funded(), STAGE);
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    assert!(matches!(
        run(
            &mut journal,
            &mut port,
            &mut PeriodConsensusSequence::new(1.into()),
            &good
        ),
        Err(ExecutionDriverError::UnsupportedTransactionKind(
            ExecutionTransactionKind::System
        ))
    ));
}
