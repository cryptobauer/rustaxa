//! Same-H1 rewarded claim errors through the public simulation API.
//! Independent complete Go state and public finalized/reopened Rust owner.
use super::*;
use rustaxa_evm::contracts::{
    NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
    NativeJournalRead, NativePortError,
};
use std::{cell::Cell, rc::Rc};
struct Probe<'a> {
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    drops: Rc<Cell<usize>>,
}
impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
impl NativeExecutionPort for Probe<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        assert_eq!(i.period, 1.into());
        self.inner.prepare(i, j)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        let result = self.inner.invoke(i, q, j)?;
        if let NativeInvocationResult::Completed(out) = &result {
            assert!(out.raw_mutations.is_empty());
            assert!(out.account_mutations.is_empty());
            assert!(out.logs.is_empty());
        }
        Ok(result)
    }
}
#[test]
fn rewarded_claim_failures_match_go_and_dispose_h1_simulations() {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claims_h1_failure/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_reward_claims_h1_failure/local.json")).unwrap();
    assert_eq!(public, local);
    let seed:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1/public.json")).unwrap();
    assert_eq!(public["state_before"], seed["state_before"]);
    let concrete = FixturePath::new("claims-failure-h1-concrete");
    let identity = materialize(&public, &concrete);
    let before = concrete_rows(&concrete);
    let app = FixturePath::new("claims-failure-h1-owner");
    let mut attempts = 0;
    for restart in 0..2 {
        let chain = super::redelegate_rewarded_partial::history(&app, restart == 0, &seed);
        let owner_before = super::redelegate_rewarded_partial::committed(&chain, &seed);
        for _ in 0..2 {
            let reader = CompleteSeedReader::open(&concrete, &public, identity);
            for case in public["cases"].as_array().unwrap() {
                for _ in 0..2 {
                    let original = FinalChainNonce::from_bytes(
                        &(BigUint::from(1u8) << 512usize).to_bytes_be(),
                    )
                    .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                    let request = ExecutionTransaction {
                        position: 0.into(),
                        hash: [0; 32],
                        sender: bytes(&case["sender"]).try_into().unwrap(),
                        receiver: Some(address(0xfe)),
                        nonce: original.clone(),
                        gas_price: ExecutionGasPrice::default(),
                        gas_limit: case["gas"].as_u64().unwrap().into(),
                        value: ExecutionValue::new(number(&case["value"])),
                        input: bytes(&case["input"]),
                        canonical_rlp: None,
                        kind: ExecutionTransactionKind::Call,
                    };
                    let drops = Rc::new(Cell::new(0));
                    let factories = Rc::new(Cell::new(0));
                    let simulated = simulate_with_native(
                        &reader,
                        &NoHistory,
                        &Dpos,
                        &Dpos,
                        |selected| {
                            factories.set(factories.get() + 1);
                            assert_eq!(selected, identity);
                            Ok(Probe {
                                inner: mixed_native::SimulationNativeExecutionPort::new(
                                    chain.begin_native_simulation(selected.period).unwrap(),
                                ),
                                drops: drops.clone(),
                            })
                        },
                        &ExecutionBlockContext {
                            period: 1.into(),
                            author: address(0x31),
                            timestamp: 1700000001,
                            gas_limit: 500000.into(),
                            chain_id: 666,
                            difficulty: BigUint::default(),
                        },
                        &request,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                    )
                    .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
                    attempts += 1;
                    assert_eq!(drops.get(), factories.get());
                    assert_eq!(request.nonce, original);
                    assert_eq!(simulated.state, identity);
                    assert_eq!(case["output"]["effective_nonce"], "1");
                    match simulated.execution {
                        TransactionExecutionResult::ConsensusFailure(result) => {
                            assert_eq!(
                                result.error,
                                rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
                            );
                            assert_eq!(case["output"]["consensus_error"], "intrinsic gas too low");
                            assert_eq!(case["output"]["execution_error"], "");
                            assert_eq!(
                                result.gas_used.as_u64(),
                                case["output"]["gas_used"].as_u64().unwrap()
                            );
                            assert_eq!(result.output, bytes(&case["output"]["return"]));
                        }
                        TransactionExecutionResult::Executed(result) => {
                            assert_eq!(case["output"]["consensus_error"], "");
                            let CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) =
                                &result.status
                            else {
                                panic!("{}: {:?}", case["name"], result.status)
                            };
                            assert_eq!(
                                error.error,
                                case["output"]["execution_error"].as_str().unwrap()
                            );
                            assert_eq!(
                                result.gas_used.as_u64(),
                                case["output"]["gas_used"].as_u64().unwrap()
                            );
                            assert_eq!(result.output, bytes(&case["output"]["return"]));
                            assert!(result.logs.is_empty());
                        }
                    }
                    assert_eq!(concrete_rows(&concrete), before);
                    assert_eq!(
                        super::redelegate_rewarded_partial::committed(&chain, &seed),
                        owner_before
                    );
                }
            }
        }
    }
    assert_eq!(attempts, 48);
    let reopened = super::redelegate_rewarded_partial::history(&app, false, &seed);
    super::redelegate_rewarded_partial::committed(&reopened, &seed);
    assert_eq!(concrete_rows(&concrete), before);
}
