//! Retained P2 gas estimation from actual Go probes and the unchanged C++ search.
use super::*;
use retained_commission::{AuthPort, committed, corpus, history};
use rustaxa_evm::{
    contracts::{NativeInvocationResult, NativeJournalReadError, NativePortError},
    driver::ExecutionDriverError,
    estimate::{EstimateError, EstimateProbe, estimate_gas},
    native::NativeAdapterError,
    simulation::SimulationError,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Attempt<'a> {
    chain: &'a FinalChain,
    concrete: &'a FixturePath,
    simulation: &'a Value,
    identity: ConcreteStateIdentity,
    owner_before: &'a Value,
    rows_before: &'a [(String, Vec<u8>, Vec<u8>)],
}

impl Attempt<'_> {
    fn probe(&self, probe: &Value, gas: u64, fault: u8) -> Result<EstimateProbe, SimulationError> {
        assert_eq!(gas, probe["gas"].as_u64().unwrap());
        // Every callback reopens all concrete rows and owns a new native port.
        let reader = CompleteSeedReader::open(self.concrete, self.simulation, self.identity);
        let nonce =
            FinalChainNonce::from_bytes(&number(&probe["supplied_nonce"]).to_bytes_be()).unwrap();
        let caller = bytes(
            &self.simulation[if probe["name"] == "wrong_owner" {
                "delegator"
            } else {
                "owner"
            }],
        )
        .try_into()
        .unwrap();
        let request = ExecutionTransaction {
            position: 0.into(),
            hash: [0; 32],
            sender: caller,
            receiver: Some(bytes(&probe["to"]).try_into().unwrap()),
            nonce: nonce.clone(),
            gas_price: ExecutionGasPrice::new(number(&probe["gas_price"])),
            gas_limit: gas.into(),
            value: ExecutionValue::new(number(&probe["value"])),
            input: bytes(&probe["input"]),
            canonical_rlp: None,
            kind: ExecutionTransactionKind::Call,
        };
        let ConcreteRead::Present(sender) = reader.account(caller).unwrap() else {
            panic!("sender")
        };
        // This is a check of the DryRunner input rule, not a returned native nonce.
        assert_eq!(
            BigUint::from_bytes_be(&sender.account.nonce.next().to_bytes()),
            number(&probe["output"]["effective_nonce"])
        );
        let reads = Rc::new(RefCell::new(Vec::new()));
        let drops = Rc::new(Cell::new(0));
        let outcomes = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let result = simulate_with_native(
            &reader,
            &NoHistory,
            &Dpos,
            &Dpos,
            |selected| {
                assert_eq!(selected, self.identity);
                if fault == 2 {
                    let mut unavailable = selected;
                    unavailable.period = 1.into();
                    // Actual historical reader refusal; no synthetic contract error.
                    let readers = ConcreteCheckpointReaders::open_read_only(
                        &self.concrete.0,
                        selected,
                        [selected],
                    )
                    .unwrap();
                    let error =
                        ConcreteCheckpointNativeStateRead::new(&readers, unavailable).unwrap_err();
                    let rustaxa_consensus::native_session::FinalChainNativeStateReadError::State(
                        error,
                    ) = error
                    else {
                        panic!("typed unavailable history")
                    };
                    assert_eq!(error, ConcreteReadError::HistoryUnavailable(unavailable));
                    return Err(NativePortError::Journal(NativeJournalReadError::State(
                        error,
                    )));
                }
                Ok(AuthPort {
                    inner: mixed_native::SimulationNativeExecutionPort::new(
                        self.chain.begin_native_simulation(selected.period).unwrap(),
                    ),
                    reads: reads.clone(),
                    drops: drops.clone(),
                    fail: (fault == 1).then_some(0),
                    mode: 1,
                    outcomes: outcomes.clone(),
                    seen: seen.clone(),
                    fail_prepare: false,
                    identity: selected,
                })
            },
            &ExecutionBlockContext {
                period: 2.into(),
                author: address(0x31),
                timestamp: 1700000002,
                gas_limit: 1000000.into(),
                chain_id: 666,
                difficulty: BigUint::default(),
            },
            &request,
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(TaraxaPhase::Ficus),
        );
        assert_eq!(drops.get(), usize::from(fault != 2));
        assert_eq!(request.nonce, nonce);
        assert_eq!(&committed(self.chain, self.simulation), self.owner_before);
        assert_eq!(concrete_rows(self.concrete), self.rows_before);
        if fault == 1 {
            assert_eq!(reads.borrow().len(), 1);
            // The failed read is the exact consumed reward-row key, not a spare row.
            assert_eq!(
                hex::encode(reads.borrow()[0].0),
                self.simulation["cases"][1]["ordered_raw_writes"][0]["key"]
            );
            assert!(outcomes.borrow().is_empty());
            assert_eq!(
                result,
                Err(SimulationError::Execution(ExecutionDriverError::Native(
                    NativeAdapterError::Port(NativePortError::Journal(
                        NativeJournalReadError::State(ConcreteReadError::HistoryUnavailable(
                            self.identity
                        ))
                    ))
                )))
            );
        } else if fault == 2 {
            assert!(
                reads.borrow().is_empty()
                    && seen.borrow().is_empty()
                    && outcomes.borrow().is_empty()
            );
            let mut unavailable = self.identity;
            unavailable.period = 1.into();
            assert_eq!(
                result,
                Err(SimulationError::NativeSession(NativePortError::Journal(
                    NativeJournalReadError::State(ConcreteReadError::HistoryUnavailable(
                        unavailable
                    ))
                )))
            );
        }
        let result = result?;
        assert_eq!(result.state, self.identity);
        let TransactionExecutionResult::Executed(execution) = result.execution else {
            panic!("executed retained probe")
        };
        let error = match &execution.status {
            CodeExecutionStatus::Success => "",
            CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) => e.error.as_str(),
            CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas",
            s => panic!("{s:?}"),
        };
        assert_eq!(probe["output"]["consensus_error"], "");
        assert_eq!(error, probe["output"]["execution_error"].as_str().unwrap());
        assert_eq!(
            execution.gas_used.as_u64(),
            probe["output"]["gas_used"].as_u64().unwrap()
        );
        assert_eq!(execution.output, bytes(&probe["output"]["return"]));
        let logs = execution.logs.iter().map(|l| json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
        assert_eq!(logs, *probe["output"]["logs"].as_array().unwrap());
        if error.is_empty() {
            assert_eq!(logs.len(), 1);
            assert_eq!(
                U256::from_big_endian(&bytes(&logs[0]["data"])),
                U256::from(2)
            );
            assert_eq!(outcomes.borrow().len(), 1);
            let outcomes = outcomes.borrow();
            let NativeInvocationResult::Completed(out) = &outcomes[0] else {
                panic!("complete")
            };
            assert_eq!(out.raw_mutations.len(), 1);
            Ok(EstimateProbe::Success {
                gas_used: execution.gas_used.as_u64(),
            })
        } else {
            Ok(EstimateProbe::CodeFailure {
                error: error.into(),
            })
        }
    }
}

#[test]
fn retained_commission_estimate_matches_cpp_go_and_disposes_faults() {
    let public: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_retained_commission_estimate/public.json")).unwrap();
    let local: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_retained_commission_estimate/local.json")).unwrap();
    let cpp: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_retained_commission_estimate/cpp.json")).unwrap();
    let simulation = corpus();
    assert_eq!(public, local);
    assert_eq!(public["state_before"], simulation["state_before"]);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["configuration"], simulation["configuration"]);
    assert_eq!(public["cases"].as_array().unwrap().len(), 4);
    assert_eq!(cpp["cases"].as_array().unwrap().len(), 4);
    let concrete = FixturePath::new("retained-estimate-concrete");
    let identity = materialize(&simulation, &concrete);
    assert_eq!(identity.period, 2.into());
    let baseline = concrete_rows(&concrete);
    let app = FixturePath::new("retained-estimate-owner");
    let mut owner_original = None;
    let mut probes_total = 0;
    let mut searches = 0;
    let mut faults = 0;
    for restart in 0..2 {
        let chain = history(&app, restart == 0, &simulation);
        let owner_before = committed(&chain, &simulation);
        assert_eq!(
            owner_original.get_or_insert(owner_before.clone()),
            &owner_before
        );
        let attempt = Attempt {
            chain: &chain,
            concrete: &concrete,
            simulation: &simulation,
            identity,
            owner_before: &owner_before,
            rows_before: &baseline,
        };
        // Repeat full searches on each semantic-owner start, reopening per callback.
        for _ in 0..2 {
            for (case, expected) in public["cases"]
                .as_array()
                .unwrap()
                .iter()
                .zip(cpp["cases"].as_array().unwrap())
            {
                assert_eq!(case["name"], expected["name"]);
                let probes = case["probes"].as_array().unwrap();
                let mut index = 0;
                let result = estimate_gas(case["cap"].as_u64().unwrap(), |gas| {
                    let p = probes.get(index).expect("transcript available");
                    index += 1;
                    probes_total += 1;
                    attempt.probe(p, gas, 0)
                });
                searches += 1;
                assert_eq!(index, probes.len());
                assert_eq!(index as u64, expected["consumed"].as_u64().unwrap());
                if let Some(value) = expected["result"].as_u64() {
                    assert_eq!(result, Ok(value));
                } else {
                    assert_eq!(
                        result,
                        Err(EstimateError::CodeFailure {
                            error: expected["error"].as_str().unwrap().into()
                        })
                    );
                }
            }
        }
        // Infrastructure errors stop after the first callback. Every failure is
        // followed by an entire fresh successful search, not an injected result.
        let case = &public["cases"][0];
        for mode in [1, 2] {
            let mut count = 0;
            let result = estimate_gas(200000, |gas| {
                count += 1;
                attempt.probe(&case["probes"][0], gas, mode)
            });
            assert_eq!(count, 1);
            let expected = if mode == 1 {
                SimulationError::Execution(ExecutionDriverError::Native(NativeAdapterError::Port(
                    NativePortError::Journal(NativeJournalReadError::State(
                        ConcreteReadError::HistoryUnavailable(identity),
                    )),
                )))
            } else {
                let mut unavailable = identity;
                unavailable.period = 1.into();
                SimulationError::NativeSession(NativePortError::Journal(
                    NativeJournalReadError::State(ConcreteReadError::HistoryUnavailable(
                        unavailable,
                    )),
                ))
            };
            assert_eq!(result, Err(EstimateError::Callback(expected)));
            faults += 1;
            let mut index = 0;
            let retry = estimate_gas(200000, |gas| {
                let p = &case["probes"][index];
                index += 1;
                probes_total += 1;
                attempt.probe(p, gas, 0)
            });
            assert_eq!(retry, Ok(cpp["cases"][0]["result"].as_u64().unwrap()));
            assert_eq!(index as u64, cpp["cases"][0]["consumed"].as_u64().unwrap());
            searches += 1;
        }
    }
    assert_eq!((searches, probes_total, faults), (20, 76, 4));
    assert_eq!(concrete_rows(&concrete), baseline);
    println!(
        "retained estimate: {searches} successful/completed searches, {probes_total} ordinary probes, {faults} one-probe infrastructure failures; two owner starts"
    );
}
