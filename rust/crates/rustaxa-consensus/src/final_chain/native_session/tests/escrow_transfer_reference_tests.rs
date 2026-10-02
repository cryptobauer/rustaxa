use super::*;
use serde_json::Value;

struct NoEscrowRead;

fn unhex(value: &Value) -> Vec<u8> {
    let text = value.as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}
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

fn request_for(row: &Value, period: FinalChainBlockNumber) -> FinalChainNativeRequest {
    let mut request = simulation_request(
        period,
        0,
        unhex(&row["input"]),
        row["supplied_native_gas"].as_u64().unwrap(),
    );
    request.caller = unhex(&row["caller"]).try_into().unwrap();
    request.depth = row["depth"].as_u64().unwrap() as u16;
    request.value = FinalChainNativeValue::new(row["value"].as_u64().unwrap().into());
    request.is_static = row["static"].as_bool().unwrap();
    if request.is_static {
        request.kind = FinalChainNativeCallKind::StaticCall;
    }
    request
}

fn assert_outcome(
    row: &Value,
    quote: FinalChainNativeGasQuote,
    result: FinalChainNativeInvocationResult,
) {
    assert_eq!(
        quote.required_gas.as_u64(),
        row["required_gas"].as_u64().unwrap()
    );
    if !row["native_called"].as_bool().unwrap() {
        assert_eq!(
            result,
            FinalChainNativeInvocationResult::InsufficientGas {
                required_gas: quote.required_gas
            }
        );
    } else {
        let outcome = completed(result);
        let error = row["native_error"].as_str().unwrap();
        assert_eq!(
            outcome.status,
            if error.is_empty() {
                FinalChainNativeStatus::Success
            } else {
                FinalChainNativeStatus::ContractFailure {
                    error: error.into(),
                }
            }
        );
        assert_eq!(outcome.gas_used, quote.required_gas);
        assert!(outcome.output.is_empty());
        assert!(outcome.logs.is_empty());
        assert!(outcome.raw_mutations.is_empty());
        assert!(outcome.account_mutations.is_empty());
    }
}

#[test]
fn escrow_transfer_pending_and_historical_sessions_match_actual_go_without_reads() {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_escrow_transfer/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_escrow_transfer/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    for row in public["cases"].as_array().unwrap() {
        if matches!(
            row["name"].as_str().unwrap(),
            "before_phala" | "at_phala" | "trailing" | "inactive_after_cornus"
        ) {
            continue;
        }
        let pre_fix = row["fix"] == 2;
        with_chain_config(
            "escrow-reference",
            FinalChainRewardsConfig {
                // Deliberately no Magnolia/custody activation gate for this selector.
                magnolia_period: FinalChainBlockNumber::MAX,
                cornus_period: 0.into(),
                phalaenopsis_period: 0.into(),
                fix_redelegate_block_num: if pre_fix { 2.into() } else { 0.into() },
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            |chain| {
                let mut pending = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let request = request_for(row, 1.into());
                let quote = pending.prepare(&request, &NoEscrowRead).unwrap();
                assert_outcome(
                    row,
                    quote,
                    pending.invoke(&request, quote, &NoEscrowRead).unwrap(),
                );
                let mut history = chain.begin_native_simulation(0.into()).unwrap();
                let request = request_for(row, 0.into());
                let quote = history.prepare(&request, &NoEscrowRead).unwrap();
                assert_outcome(
                    row,
                    quote,
                    history.invoke(&request, quote, &NoEscrowRead).unwrap(),
                );
            },
        );
    }
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
                    assert_eq!(
                        session.prepare(&request, &NoEscrowRead),
                        Err(FinalChainNativeSessionError::UnsupportedOperation)
                    );
                    return;
                }
                let mut trailing = request.clone();
                trailing.input.push(0);
                assert_eq!(
                    session.prepare(&trailing, &NoEscrowRead),
                    Err(FinalChainNativeSessionError::UnsupportedOperation)
                );
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
                request.id.sequence = 1;
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
