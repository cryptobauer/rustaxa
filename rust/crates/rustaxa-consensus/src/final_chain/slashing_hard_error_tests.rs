use rustaxa_types::evm_correction::CorrectionStage;

type SlashingDatabaseRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;

fn slashing_database_rows(storage: &Storage) -> SlashingDatabaseRows {
    Column::all()
        .iter()
        .map(|&column| {
            let rows = storage
                .iter(column)
                .map(|row| {
                    let (key, value) = row.unwrap();
                    (key.to_vec(), value.to_vec())
                })
                .collect();
            (column.name().to_owned(), rows)
        })
        .collect()
}

fn slashing_gate_chain(storage: Arc<Storage>) -> FinalChain {
    FinalChain::new_with_rewards_config(
        storage,
        1_000_000.into(),
        0,
        vec![genesis_account([0x51; 20], U256::from(1_000_000))],
        vec![],
        GenesisDposConfig::default(),
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 1.into(),
            ..Default::default()
        },
    )
    .unwrap()
    .with_correction_stage(CorrectionStage::SlashingHardError)
}

fn slashing_gate_tx(
    byte: u8,
    nonce: u64,
    receiver: [u8; 20],
    data: Vec<u8>,
    gas: u64,
) -> FinalizationTransaction {
    test_transaction(TestTransactionInput {
        hash_byte: byte,
        sender: [0x51; 20],
        receiver: Some(receiver),
        nonce,
        value: U256::from(7),
        gas_price: U256::one(),
        gas_limit: gas,
        data,
        rlp: vec![0xc1, byte],
    })
}

#[test]
fn slashing_hard_error_discards_memory_prefix_and_persisted_attempt_then_reopens() {
    for nested_sortition in [false, true] {
        let path = temp_db_path("slashing-hard-error-publication");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = slashing_gate_chain(storage.clone());
        let sortition = if nested_sortition {
            vec![0xf8]
        } else {
            legacy_vrf_sortition_rlp(1, 2, 4, 0x5a)
        };
        let key = SigningKey::from_slice(&[0x52; 32]).unwrap();
        let vote = signed_legacy_pbft_vote(&key, H256::from_low_u64_be(1), &sortition);
        let malformed = if nested_sortition {
            vote.clone()
        } else {
            vec![0xf8]
        };
        let input = commit_double_voting_proof_input(&vote, &malformed);
        let prefix = slashing_gate_tx(0xa1, 0, [0x61; 20], vec![], 30_000);
        let middle = slashing_gate_tx(0xa2, 1, SLASHING_CONTRACT_ADDRESS, input, 100_000);
        let suffix = slashing_gate_tx(0xa3, 2, [0x63; 20], vec![], 30_000);
        let block_key = SigningKey::from_slice(&[0x53; 32]).unwrap();
        let pbft = signed_pbft_block(&block_key, 1, 321);
        write_period_data(
            &storage,
            1,
            &pbft,
            &[prefix.rlp.clone(), middle.rlp.clone(), suffix.rlp.clone()],
        );
        let before_rows = slashing_database_rows(&storage);
        let before_accounts = chain.accounts.lock().unwrap().clone();
        let before_account_snapshots = chain.account_snapshots.lock().unwrap().clone();
        let before_dpos = chain.dpos_snapshots.lock().unwrap().clone();
        let prefix_only = chain
            .execute_native_transactions_from_state(
                1.into(),
                std::slice::from_ref(&prefix),
                before_accounts.clone(),
                before_dpos[&0.into()].clone(),
                false,
            )
            .unwrap();
        assert_eq!(prefix_only.receipts.len(), 1);
        assert_eq!(prefix_only.receipts[0].status_code, 1);
        assert_eq!(
            prefix_only.accounts[&[0x61; 20]].balance.as_u256(),
            &U256::from(7)
        );
        assert_eq!(prefix_only.accounts[&[0x51; 20]].nonce, 1);
        let memory_error = chain
            .execute_native_transactions_from_state(
                1.into(),
                &[prefix.clone(), middle.clone(), suffix.clone()],
                before_accounts.clone(),
                before_dpos[&0.into()].clone(),
                false,
            )
            .err()
            .unwrap();
        assert!(
            memory_error
                .to_string()
                .contains("malformed nested slashing proof RLP")
        );
        let error = chain
            .finalize_block(
                pbft.clone(),
                vec![prefix.clone(), middle, suffix.clone()],
                vec![],
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("malformed nested slashing proof RLP")
        );
        assert_eq!(*chain.accounts.lock().unwrap(), before_accounts);
        assert_eq!(
            *chain.account_snapshots.lock().unwrap(),
            before_account_snapshots
        );
        assert_eq!(*chain.dpos_snapshots.lock().unwrap(), before_dpos);
        assert_eq!(chain.last_block_number().unwrap(), 0);
        for hash in [0xa1, 0xa2, 0xa3] {
            assert!(chain.transaction_location([hash; 32]).unwrap().is_none());
        }
        assert_eq!(slashing_database_rows(&storage), before_rows);
        drop(prefix_only);
        drop(chain);
        drop(storage);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = slashing_gate_chain(storage.clone());
        assert_eq!(slashing_database_rows(&storage), before_rows);
        assert_eq!(chain.last_block_number().unwrap(), 0);
        assert_eq!(*chain.accounts.lock().unwrap(), before_accounts);
        assert_eq!(*chain.dpos_snapshots.lock().unwrap(), before_dpos);
        let valid_middle = slashing_gate_tx(0xb2, 1, [0x62; 20], vec![], 30_000);
        write_period_data(
            &storage,
            1,
            &pbft,
            &[
                prefix.rlp.clone(),
                valid_middle.rlp.clone(),
                suffix.rlp.clone(),
            ],
        );
        let (_, receipts) = chain
            .finalize_block(pbft, vec![prefix, valid_middle, suffix], vec![])
            .unwrap();
        assert_eq!(receipts.len(), 3);
        for (i, receipt) in receipts.iter().enumerate() {
            assert_eq!(
                receipt_fields(receipt),
                (1, 21_000, 21_000 * (i as u64 + 1))
            );
            assert!(receipt_logs(receipt).is_empty());
        }
        assert_eq!(
            balance_of(&chain, [0x51; 20]),
            U256::from(1_000_000 - 63_000 - 21)
        );
        assert_eq!(chain.account([0x51; 20]).unwrap().unwrap().nonce, 3);
        for address in [[0x61; 20], [0x62; 20], [0x63; 20]] {
            assert_eq!(balance_of(&chain, address), U256::from(7));
        }
        assert_eq!(chain.last_block_number().unwrap(), 1);
        let published_rows = slashing_database_rows(&storage);
        drop(chain);
        drop(storage);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = slashing_gate_chain(storage.clone());
        assert_eq!(chain.last_block_number().unwrap(), 1);
        assert_eq!(slashing_database_rows(&storage), published_rows);
        drop(chain);
        drop(storage);
    }
}

#[test]
fn slashing_hard_error_direct_admission_and_readonly_precedence() {
    let path = temp_db_path("slashing-hard-error-admission");
    let storage = Arc::new(Storage::new(Config::new(path)).unwrap());
    let chain = slashing_gate_chain(storage.clone());
    let malformed = commit_double_voting_proof_input(&[0xf8], &[0xf8]);
    let intrinsic = intrinsic_gas(&malformed, false).unwrap();
    let initial_accounts = chain.accounts.lock().unwrap().clone();
    let initial_dpos = chain.dpos_snapshot(0.into()).unwrap();
    for (name, nonce, balance, gas, expected_gas, expected_nonce) in [
        ("stale", 0, 1_000_000, 100_000, 100_000, 1),
        ("unfunded cap", 1, 100, 100_000, 100, 2),
        (
            "unfunded action",
            1,
            1_000_000,
            intrinsic + 19_999,
            intrinsic,
            2,
        ),
    ] {
        let mut accounts = initial_accounts.clone();
        let sender = accounts.get_mut(&[0x51; 20]).unwrap();
        sender.nonce = 1.into();
        sender.balance = rustaxa_types::FinalChainAccountBalance::new_account(U256::from(balance));
        let tx = slashing_gate_tx(
            0xc1,
            nonce,
            SLASHING_CONTRACT_ADDRESS,
            malformed.clone(),
            gas,
        );
        let result = chain
            .execute_native_transactions_from_state(
                1.into(),
                &[tx],
                accounts,
                initial_dpos.clone(),
                false,
            )
            .unwrap();
        assert_eq!(result.receipts[0].status_code, 0, "{name}");
        assert_eq!(result.receipts[0].gas_used.as_u64(), expected_gas, "{name}");
        assert_eq!(result.accounts[&[0x51; 20]].nonce, expected_nonce, "{name}");
        assert!(result.receipts[0].logs.is_empty());
        assert_eq!(result.dpos_snapshot, initial_dpos);
    }
    let tx = slashing_gate_tx(
        0xc2,
        0,
        SLASHING_CONTRACT_ADDRESS,
        malformed.clone(),
        intrinsic + 20_000,
    );
    assert!(
        chain
            .execute_native_transactions_from_state(
                1.into(),
                &[tx],
                initial_accounts,
                initial_dpos,
                false
            )
            .err()
            .unwrap()
            .to_string()
            .contains("malformed nested slashing proof RLP")
    );
    for (gas, hard) in [(20_999, false), (21_000, true)] {
        let mut request = slashing_call_request(0, malformed.clone());
        request.gas_limit = gas.into();
        if hard {
            assert!(
                chain
                    .call(request)
                    .unwrap_err()
                    .to_string()
                    .contains("malformed nested slashing proof RLP")
            );
        } else {
            let result = chain.call(request).unwrap();
            assert_eq!(result.code_err, "intrinsic gas too low");
            assert_eq!(result.gas_used.as_u64(), gas);
        }
    }
}

#[test]
fn slashing_hard_error_valid_duplicate_gas_logs_and_jail_controls() {
    finalize_block_does_not_double_charge_duplicate_slashing_proof_value_for_stage(
        CorrectionStage::SlashingHardError,
    );
    finalize_block_does_not_charge_value_on_invalid_slashing_proof_and_preserves_state_for_stage(
        CorrectionStage::SlashingHardError,
    );

    assert_finalize_block_executes_slashing_for_stage(1, 1, CorrectionStage::SlashingHardError);
    assert_finalize_block_executes_slashing_for_stage(0, 0, CorrectionStage::SlashingHardError);
}

#[test]
fn slashing_hard_error_outer_and_signature_failures_keep_normal_settlement() {
    let path = temp_db_path("slashing-hard-error-normal");
    let storage = Arc::new(Storage::new(Config::new(path)).unwrap());
    let chain = slashing_gate_chain(storage);
    let sortition = legacy_vrf_sortition_rlp(1, 2, 4, 0x5a);
    let unsigned = |block: u64| {
        let mut s = RlpStream::new_list(3);
        s.append(&H256::from_low_u64_be(block));
        s.append(&sortition);
        s.append(&vec![0; 65]);
        s.out().to_vec()
    };
    let invalid_signature = commit_double_voting_proof_input(&unsigned(1), &unsigned(2));
    let identical = commit_double_voting_proof_input(&unsigned(1), &unsigned(1));
    for input in [
        SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR.to_vec(),
        invalid_signature,
        identical,
    ] {
        let accounts = chain.accounts.lock().unwrap().clone();
        let dpos = chain.dpos_snapshot(0.into()).unwrap();
        let expected_gas = intrinsic_gas(&input, false).unwrap() + 20_000;
        let tx = slashing_gate_tx(0xd1, 0, SLASHING_CONTRACT_ADDRESS, input.clone(), 100_000);
        let result = chain
            .execute_native_transactions_from_state(1.into(), &[tx], accounts, dpos.clone(), false)
            .unwrap();
        assert_eq!(result.receipts[0].status_code, 0);
        assert_eq!(result.receipts[0].gas_used.as_u64(), expected_gas);
        assert_eq!(
            result.receipts[0].cumulative_gas_used.as_u64(),
            expected_gas
        );
        assert!(result.receipts[0].logs.is_empty());
        assert_eq!(result.accounts[&[0x51; 20]].nonce, 1);
        assert_eq!(
            result.accounts[&[0x51; 20]].balance.as_u256(),
            &U256::from(1_000_000 - expected_gas)
        );
        assert!(!result.accounts.contains_key(&SLASHING_CONTRACT_ADDRESS));
        assert_eq!(result.dpos_snapshot, dpos);
        let call = chain.call(slashing_call_request(0, input)).unwrap();
        assert_eq!(call.gas_used.as_u64(), 20_000);
        assert!(call.code_err.contains("unsupported slashing selector"));
        assert!(call.code_retval.is_empty());
        assert!(call.logs.is_empty());
    }
}
