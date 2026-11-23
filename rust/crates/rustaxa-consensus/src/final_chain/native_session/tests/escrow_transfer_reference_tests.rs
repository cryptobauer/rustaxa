use super::*;

struct NoEscrowRead;

impl FinalChainNativeStateRead for NoEscrowRead {
    fn raw_storage(
        &self,
        _: [u8; 20],
        _: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        panic!("escrow operation must not read raw state")
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        panic!("escrow operation must not read accounts")
    }
}

fn assert_lookup_failure(
    session: &mut FinalChainNativeSession<'_>,
    request: &FinalChainNativeRequest,
) {
    let before = session.dpos_state.clone();
    let quote = session.prepare(request, &NoEscrowRead).unwrap();
    assert_eq!(quote.required_gas, FinalChainGas::ZERO);
    let outcome = completed(session.invoke(request, quote, &NoEscrowRead).unwrap());
    assert_eq!(
        outcome.status,
        FinalChainNativeStatus::ContractFailure {
            error: "no method with id: 0x44df8e70".to_owned(),
        }
    );
    assert_eq!(outcome.gas_used, FinalChainGas::ZERO);
    assert!(outcome.output.is_empty());
    assert!(outcome.logs.is_empty());
    assert!(outcome.raw_mutations.is_empty());
    assert!(outcome.account_mutations.is_empty());
    assert_eq!(session.dpos_state, before);
}

#[test]
fn escrow_activation_exact_input_quote_binding_and_failure_continuation() {
    for activation in [1_u64, 2] {
        with_chain_config(
            "escrow-activation",
            FinalChainRewardsConfig {
                cornus_period: 0.into(),
                phalaenopsis_period: activation.into(),
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            |chain| {
                let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let mut request =
                    simulation_request(1.into(), 0, vec![0x44, 0xdf, 0x8e, 0x70], 1000);
                if activation == 2 {
                    assert_lookup_failure(&mut session, &request);
                    request.id.sequence = 1;
                    request.input.push(0);
                    assert_lookup_failure(&mut session, &request);
                    return;
                }
                let mut trailing = request.clone();
                trailing.input.push(0);
                assert_lookup_failure(&mut session, &trailing);
                request.id.sequence = 1;
                request.supplied_gas = 999.into();
                let quote = session.prepare(&request, &NoEscrowRead).unwrap();
                let mut mismatch = quote;
                mismatch.required_gas = 0.into();
                assert_eq!(
                    session.invoke(&request, mismatch, &NoEscrowRead),
                    Err(FinalChainNativeSessionError::QuoteMismatch)
                );
                assert_eq!(
                    session.invoke(&request, quote, &NoEscrowRead).unwrap(),
                    FinalChainNativeInvocationResult::InsufficientGas {
                        required_gas: 1000.into()
                    }
                );
                request.id.sequence = 2;
                request.supplied_gas = 1000.into();
                // No machine-word conversion is allowed in this pure payable kernel.
                request.value = FinalChainNativeValue::new(BigUint::from(1_u8) << 300);
                let quote = session.prepare(&request, &NoEscrowRead).unwrap();
                let outcome = completed(session.invoke(&request, quote, &NoEscrowRead).unwrap());
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(outcome.gas_used, 1000.into());
                assert!(outcome.account_mutations.is_empty());
                assert!(outcome.raw_mutations.is_empty());
            },
        );
    }
}
