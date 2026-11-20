use rustaxa_types::evm_correction::CorrectionStage;

fn malformed_slashing_request(
    sequence: u64,
    gas: u64,
    sortition: bool,
    position: usize,
) -> FinalChainNativeRequest {
    let vote = if sortition {
        let mut s = rlp::RlpStream::new_list(3);
        s.append(&vec![1; 32]);
        s.append(&vec![0xf8]);
        s.append(&vec![0; 65]);
        s.out().to_vec()
    } else {
        vec![0xf8]
    };
    let good_sortition = {
        let mut s = rlp::RlpStream::new_list(4);
        s.append(&1u64);
        s.append(&2u32);
        s.append(&4u32);
        s.append(&vec![0x5a; 80]);
        s.out().to_vec()
    };
    let good = {
        let mut s = rlp::RlpStream::new_list(3);
        s.append(&vec![2; 32]);
        s.append(&good_sortition);
        s.append(&vec![0; 65]);
        s.out().to_vec()
    };
    let input = if position == 0 {
        crate::slashing::commit_double_voting_proof_call_data(&vote, &good)
    } else {
        crate::slashing::commit_double_voting_proof_call_data(&good, &vote)
    };
    let mut req = custody_request(sequence, OWNER, 0, input, gas);
    req.contract = SLASHING_CONTRACT_ADDRESS;
    req.state_address = SLASHING_CONTRACT_ADDRESS;
    req
}

fn with_slashing_stage(test: impl FnOnce(Arc<FinalChain>)) {
    with_arc_chain_dpos_config(
        "slashing-stage",
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            ..Default::default()
        },
        FinalChainBlockNumber::MAX,
        GenesisDposConfig::default(),
        |chain| {
            let chain = Arc::new(
                Arc::try_unwrap(chain)
                    .ok()
                    .unwrap()
                    .with_correction_stage(CorrectionStage::SlashingHardError),
            );
            test(chain);
        },
    );
}

#[test]
fn slashing_hard_error_native_poison_clears_preparation_and_retains_no_effects() {
    with_slashing_stage(|chain| {
        for sortition in [false, true] {
            for position in 0..2 {
                let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let before = session.dpos_state.clone();
                let state = RawState::erroring();
                let request = malformed_slashing_request(0, 20_000, sortition, position);
                let quote = session.prepare(&request, &state).unwrap();
                assert_eq!(quote.required_gas.as_u64(), 20_000);
                assert!(
                    matches!(session.invoke(&request, quote, &state), Err(FinalChainNativeSessionError::MalformedNestedSlashingProof(message)) if !message.is_empty())
                );
                assert!(session.aborted);
                assert!(session.prepared.is_none());
                assert_eq!(session.next_sequence, 0);
                assert_eq!(session.completed_input_invocation, None);
                assert!(session.input_aliases.is_empty());
                assert!(session.input_mutations.is_empty());
                assert!(session.borrowed_vrf_keys.is_empty());
                assert!(session.verified_slashing.is_none());
                assert_eq!(session.dpos_state, before);
                assert_eq!(state.reads.get(), 0);
                assert_eq!(state.account_reads.get(), 0);
                assert!(matches!(
                    session.prepare(&request, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                ));
                assert!(matches!(
                    session.invoke(&request, quote, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                ));
                drop(session);
                let mut fresh = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let mut outer = request.clone();
                outer.input = SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR.to_vec();
                let quote = fresh.prepare(&outer, &state).unwrap();
                let FinalChainNativeInvocationResult::Completed(result) =
                    fresh.invoke(&outer, quote, &state).unwrap()
                else {
                    panic!("funded outer ABI must complete")
                };
                assert!(matches!(
                    result.status,
                    FinalChainNativeStatus::ContractFailure { .. }
                ));
                assert_eq!(result.gas_used.as_u64(), 20_000);
                assert!(result.output.is_empty());
                assert!(result.logs.is_empty());
                assert_eq!(fresh.next_sequence, 1);
                assert!(!fresh.aborted);
                assert_eq!(fresh.dpos_state, before);
            }
        }
    });
}

#[test]
fn slashing_hard_error_native_action_gas_and_readonly_owner_identity() {
    with_slashing_stage(|chain| {
        let state = RawState::erroring();
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let request = malformed_slashing_request(0, 19_999, false, 0);
        let quote = session.prepare(&request, &state).unwrap();
        assert!(
            matches!(session.invoke(&request, quote, &state).unwrap(), FinalChainNativeInvocationResult::InsufficientGas { required_gas } if required_gas.as_u64() == 20_000)
        );
        assert_eq!(session.next_sequence, 1);
        assert!(!session.aborted);
        assert_eq!(state.reads.get(), 0);
        let funded = malformed_slashing_request(1, 20_000, false, 0);
        let quote = session.prepare(&funded, &state).unwrap();
        assert!(session.invoke(&funded, quote, &state).is_err());
        assert_eq!(session.next_sequence, 1);
        assert!(session.aborted);
        let mut simulation = chain.begin_native_simulation(0.into()).unwrap();
        assert_eq!(
            simulation.correction_stage(),
            CorrectionStage::SlashingHardError
        );
        let mut req = malformed_slashing_request(0, 20_000, true, 1);
        req.period = 0.into();
        let quote = simulation.prepare(&req, &state).unwrap();
        assert!(
            matches!(simulation.invoke(&req, quote, &state), Err(FinalChainNativeSessionError::MalformedNestedSlashingProof(message)) if !message.is_empty())
        );
        assert!(simulation.session.aborted);
        chain
            .authorize_reward_scheduler_joint_startup(91, 0.into())
            .unwrap();
        chain.complete_reward_scheduler_recovery(91).unwrap();
        let owned = chain
            .begin_owned_native_session_bound_at_state_api_epoch([0x39; 32], 91, 1.into(), 0.into())
            .unwrap();
        assert_eq!(owned.correction_stage(), CorrectionStage::SlashingHardError);
    });
}

#[test]
fn slashing_hard_error_concrete_replay_checks_action_gas_before_decode() {
    with_slashing_stage(|chain| {
        let mut invocation = FinalChainConcreteInvocation {
            transaction_index: 0,
            sequence: 0,
            depth: 1,
            call_type: 0,
            caller: OWNER,
            contract: SLASHING_CONTRACT_ADDRESS,
            value: vec![],
            input: malformed_slashing_request(0, 19_999, true, 0).input,
            output: vec![],
            supplied_gas: 19_999,
            required_gas: 20_000,
            gas_used: 0,
            error: "out of gas".into(),
            logs: vec![],
            disposition: FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED,
        };
        replay_invocation(&chain, &invocation).unwrap();
        invocation.required_gas += 1;
        assert!(
            replay_invocation(&chain, &invocation)
                .unwrap_err()
                .to_string()
                .contains("REQUIRED_GAS_MISMATCH")
        );
        invocation.required_gas = 20_000;
        invocation.supplied_gas = 20_000;
        invocation.gas_used = 20_000;
        assert!(
            replay_invocation(&chain, &invocation)
                .unwrap_err()
                .to_string()
                .contains("malformed nested slashing proof RLP")
        );
    });
}

fn with_period_correction(test: impl FnOnce(Arc<FinalChain>)) {
    with_slashing_stage(|chain| {
        test(Arc::new(
            Arc::try_unwrap(chain)
                .ok()
                .unwrap()
                .with_correction_activation(Some(1.into())),
        ));
    });
}

type PolicyRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;
fn policy_rows(chain: &FinalChain) -> PolicyRows {
    rustaxa_storage::Column::all()
        .iter()
        .copied()
        .map(|column| {
            let rows = chain
                .storage
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

#[test]
fn correction_policy_preserves_fixed_stages() {
    with_slashing_stage(|chain| {
        let mut chain = Arc::try_unwrap(chain).ok().unwrap();
        let before = policy_rows(&chain);
        for stage in [
            CorrectionStage::Historical,
            CorrectionStage::TransientRollback,
            CorrectionStage::SingleSelfDestructRefund,
            CorrectionStage::CheckedReturnDataCopy,
            CorrectionStage::BoundedFalconAbi,
            CorrectionStage::SlashingHardError,
        ] {
            chain = chain.with_correction_stage(stage);
            for period in [0, 1, 7, u64::MAX] {
                assert_eq!(chain.correction_stage_at(period.into()), stage);
            }
        }
        assert_eq!(policy_rows(&chain), before);
    });
}

#[test]
fn correction_policy_selects_inclusive_boundary() {
    with_slashing_stage(|chain| {
        let mut chain = Arc::try_unwrap(chain).ok().unwrap();
        let before = policy_rows(&chain);
        for boundary in [None, Some(0), Some(7), Some(u64::MAX)] {
            let boundary = boundary.map(FinalChainBlockNumber::new);
            let schedule = crate::pbft_execution_validation::PbftExecutionSchedule::new(
                841, [1; 32], [2; 32], boundary,
            );
            chain = chain.with_correction_activation(boundary);
            for period in [0, 1, 6, 7, 8, u64::MAX - 1, u64::MAX] {
                assert_eq!(
                    chain.correction_stage_at(period.into()),
                    schedule.correction_stage(period.into()),
                );
            }
        }
        assert_eq!(policy_rows(&chain), before);
    });
}

#[test]
fn correction_policy_preserves_constructor_fork_fields() {
    use crate::pbft_execution_validation::PbftInstructionPhase as Phase;
    for (ficus, cacti, cornus, gas) in [
        (2, 4, 3, 1_000_000),
        (u64::MAX, u64::MAX, u64::MAX, 2_000_000),
    ] {
        let path = temp_db_path("correction-policy-forks");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
            storage.clone(),
            gas.into(),
            0,
            crate::FinalChainGenesisAllocation {
                accounts: vec![],
                validators: vec![],
            },
            GenesisDposConfig::default(),
            FinalChainRewardsConfig {
                magnolia_period: 0.into(),
                cacti_period: cacti.into(),
                cornus_period: cornus.into(),
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            ficus.into(),
        )
        .unwrap();
        for period in 0..6 {
            assert_eq!(
                chain.correction_stage_at(period.into()),
                CorrectionStage::Historical
            );
        }
        let chain = chain.with_correction_activation(Some(3.into()));
        for period in 0..6 {
            let expected = if period >= cacti {
                Phase::Cacti
            } else if period >= ficus {
                Phase::Ficus
            } else {
                Phase::Californicum
            };
            assert_eq!(
                chain.pbft_execution_instruction_phase(period.into()),
                expected
            );
            assert_eq!(chain.pbft_execution_cornus(period.into()), period >= cornus);
            assert_eq!(chain.pbft_execution_block_gas_limit().as_u64(), gas);
            assert_eq!(
                chain.correction_stage_at(period.into()),
                if period < 3 {
                    CorrectionStage::Historical
                } else {
                    CorrectionStage::SlashingHardError
                }
            );
        }
        drop(chain);
        drop(storage);
        crate::closeout_test_scratch::dispose(path).unwrap();
    }
}

#[test]
fn native_correction_policy_replays_historical_after_active_attempt() {
    with_period_correction(|chain| {
        let before = policy_rows(&chain);
        let state = RawState::erroring();
        for _ in 0..2 {
            let mut active = chain.begin_native_session(1.into(), 0.into()).unwrap();
            let prior = active.dpos_state.clone();
            let request = malformed_slashing_request(0, 20_000, false, 0);
            let quote = active.prepare(&request, &state).unwrap();
            assert_eq!(
                active.correction_stage(),
                CorrectionStage::SlashingHardError
            );
            assert!(matches!(
                active.invoke(&request, quote, &state),
                Err(FinalChainNativeSessionError::MalformedNestedSlashingProof(
                    _
                ))
            ));
            assert!(active.aborted);
            assert!(active.prepared.is_none());
            assert_eq!(active.next_sequence, 0);
            assert_eq!(active.dpos_state, prior);
            drop(active);
            let mut historical = chain.begin_native_simulation(0.into()).unwrap();
            let historical_prior = historical.session.dpos_state.clone();
            assert_eq!(historical.correction_stage(), CorrectionStage::Historical);
            let mut request = malformed_slashing_request(0, 20_000, false, 0);
            request.period = 0.into();
            let quote = historical.prepare(&request, &state).unwrap();
            let FinalChainNativeInvocationResult::Completed(outcome) =
                historical.invoke(&request, quote, &state).unwrap()
            else {
                panic!("historical malformed proof must complete");
            };
            assert!(matches!(
                outcome.status,
                FinalChainNativeStatus::ContractFailure { .. }
            ));
            assert_eq!(outcome.gas_used.as_u64(), 20_000);
            assert!(outcome.output.is_empty());
            assert!(outcome.logs.is_empty());
            assert_eq!(historical.session.next_sequence, 1);
            assert!(!historical.session.aborted);
            assert_eq!(historical.session.dpos_state, historical_prior);
        }
        assert_eq!(state.reads.get(), 0);
        assert_eq!(state.account_reads.get(), 0);
        assert_eq!(policy_rows(&chain), before);
    });
}

#[test]
fn native_correction_policy_preserves_action_gas_and_owned_identity() {
    with_period_correction(|chain| {
        chain
            .authorize_reward_scheduler_joint_startup(91, 0.into())
            .unwrap();
        chain.complete_reward_scheduler_recovery(91).unwrap();
        let before = policy_rows(&chain);
        let state = RawState::erroring();
        let mut borrowed = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut owned = chain
            .begin_owned_native_session_bound_at_state_api_epoch([0x39; 32], 91, 1.into(), 0.into())
            .unwrap();
        assert_eq!(
            borrowed.correction_stage(),
            CorrectionStage::SlashingHardError
        );
        assert_eq!(owned.correction_stage(), borrowed.correction_stage());
        let request = malformed_slashing_request(0, 19_999, true, 1);
        for session in [&mut borrowed, &mut owned] {
            let prior = session.dpos_state.clone();
            let quote = session.prepare(&request, &state).unwrap();
            assert!(matches!(session.invoke(&request, quote, &state).unwrap(),
                FinalChainNativeInvocationResult::InsufficientGas { required_gas } if required_gas.as_u64() == 20_000));
            assert_eq!(session.next_sequence, 1);
            assert!(!session.aborted);
            assert_eq!(session.dpos_state, prior);
        }
        let simulation = chain.begin_native_simulation(0.into()).unwrap();
        assert_eq!(simulation.correction_stage(), CorrectionStage::Historical);
        assert_eq!(state.reads.get(), 0);
        assert_eq!(state.account_reads.get(), 0);
        assert_eq!(policy_rows(&chain), before);
    });
}

#[test]
fn concrete_replay_and_call_use_requested_period() {
    with_period_correction(|chain| {
        let mut chain = Arc::try_unwrap(chain).ok().unwrap();
        let before = policy_rows(&chain);
        let input = malformed_slashing_request(0, 20_000, true, 0).input;
        for period in [0_u64, 1] {
            let direct = |chain: &FinalChain| {
                chain
                    .call(rustaxa_types::FinalChainCallRequest {
                        block_number: period.into(),
                        sender: [0; 20],
                        receiver: Some(SLASHING_CONTRACT_ADDRESS),
                        value: U256::zero().into(),
                        gas_price: U256::zero().into(),
                        gas_limit: 100_000.into(),
                        input: input.clone(),
                    })
                    .map_err(|error| format!("{error:#}"))
            };
            let replay = |chain: &FinalChain| {
                let mut snapshot = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
                let mut gas_snapshot = snapshot.clone();
                let mut accounts = chain.account_snapshot_map_at_block(0.into()).unwrap();
                let invocation = FinalChainConcreteInvocation {
                    transaction_index: 0,
                    sequence: 0,
                    depth: 1,
                    call_type: 0,
                    caller: OWNER,
                    contract: SLASHING_CONTRACT_ADDRESS,
                    value: vec![],
                    input: input.clone(),
                    output: vec![],
                    supplied_gas: 19_999,
                    required_gas: 20_000,
                    gas_used: 0,
                    error: "out of gas".into(),
                    logs: vec![],
                    disposition: FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED,
                };
                let result = chain
                    .replay_concrete_precompile_invocation(
                        period.into(),
                        &invocation,
                        ConcreteNativeReplayState {
                            dpos_snapshot: &mut snapshot,
                            dpos_gas_snapshot: &mut gas_snapshot,
                            accounts: &mut accounts,
                            head_block_number: 0.into(),
                            dpos_eligibility_read_snapshot: &mut None,
                            slashing_validator_snapshot: None,
                            slashing_read_snapshot: &mut None,
                        },
                    )
                    .map_err(|error| format!("{error:#}"));
                (result, snapshot, gas_snapshot, accounts)
            };
            let envelope = |chain: &FinalChain| {
                let mut accounts = chain.account_snapshot_map_at_block(0.into()).unwrap();
                accounts.insert(
                    OWNER,
                    rustaxa_types::Account {
                        nonce: 0.into(),
                        balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(
                            1_000_000,
                        )),
                        storage_root_hash: [0; 32],
                        code_hash: [0; 32],
                        code_size: 0,
                    },
                );
                let tx = rustaxa_types::FinalizationTransaction {
                    hash: [0x17; 32],
                    sender: OWNER,
                    receiver: Some(SLASHING_CONTRACT_ADDRESS),
                    nonce: 0.into(),
                    value: U256::zero().into(),
                    gas_price: U256::zero().into(),
                    gas_limit: (crate::final_chain::intrinsic_gas(&input, false).unwrap() + 19_999)
                        .into(),
                    data: input.clone(),
                    rlp: vec![0xc0],
                };
                chain
                    .execute_native_transactions_from_state(
                        period.into(),
                        &[tx],
                        accounts,
                        chain.dpos_snapshot_at_finalized_block(0.into()).unwrap(),
                        false,
                    )
                    .map(|result| {
                        (
                            result.accounts,
                            result.receipts,
                            result.gas_used,
                            result.transaction_fees,
                            result.dpos_snapshot,
                        )
                    })
                    .map_err(|error| format!("{error:#}"))
            };
            chain = chain.with_correction_activation(Some(1.into()));
            let actual_call = direct(&chain);
            let actual_replay = replay(&chain);
            let actual_envelope = envelope(&chain);
            let stage = if period == 0 {
                CorrectionStage::Historical
            } else {
                CorrectionStage::SlashingHardError
            };
            chain = chain.with_correction_stage(stage);
            assert_eq!(actual_call, direct(&chain));
            assert_eq!(actual_replay, replay(&chain));
            assert_eq!(actual_envelope, envelope(&chain));
            if period == 0 {
                assert!(actual_call.is_ok());
                assert!(actual_replay.0.is_err());
                assert!(actual_envelope.is_err());
            } else {
                assert!(
                    actual_call
                        .unwrap_err()
                        .contains("malformed nested slashing proof RLP")
                );
                assert!(actual_replay.0.is_ok());
                assert!(actual_envelope.is_ok());
            }
        }
        assert_eq!(policy_rows(&chain), before);
    });
}
