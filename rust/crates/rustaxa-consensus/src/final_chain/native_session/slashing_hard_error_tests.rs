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
                    matches!(session.invoke(&request, quote, &state), Err(FinalChainNativeSessionError::Domain(message)) if message.starts_with("malformed nested slashing proof RLP:"))
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
            matches!(simulation.invoke(&req, quote, &state), Err(FinalChainNativeSessionError::Domain(message)) if message.starts_with("malformed nested slashing proof RLP:"))
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
