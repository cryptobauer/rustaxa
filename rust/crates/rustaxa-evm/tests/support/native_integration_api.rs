//! Historical API checks over the same published and reopened signed workload.
use super::*;
use rustaxa_evm::{
    contracts::{
        ExecutionGasPrice, ExecutionTransaction, ExecutionValue, NativeExecutionPort,
        NativeGasQuote, NativeInvocation, NativeInvocationResult, NativeJournalRead,
        NativePortError,
    },
    estimate::{EstimateProbe, estimate_gas},
    simulation::simulate_with_native,
    trace_runner::run_structured_trace_with_native,
};
use std::rc::Rc;

// This fixture owns complete, unpruned history. A new logical key still needs
// a trie nonmembership proof. Missing bytes for any known row remain errors.
pub(crate) trait StorageProof: ConcreteStateRead {
    fn nonmember(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> std::result::Result<bool, ConcreteReadError>;
}
impl StorageProof for ConcreteStateReader {
    fn nonmember(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> std::result::Result<bool, ConcreteReadError> {
        Ok(matches!(
            self.verify_storage_path(a, k)?,
            rustaxa_storage::ConcreteStoragePath::NonMember
        ))
    }
}
pub(crate) struct CompleteReader<R> {
    pub(crate) inner: R,
    pub(crate) known: BTreeSet<[u8; 32]>,
}
fn inventory(state: &Value, head: u64) -> Result<BTreeSet<[u8; 32]>> {
    state["rows"][4]
        .as_object()
        .context("complete authenticated CF5 physical inventory")?
        .keys()
        .map(|key| {
            let raw = hex::decode(key)?;
            ensure!(raw.len() == 40, "complete physical storage version key");
            ensure!(
                u64::from_be_bytes(raw[32..].try_into().unwrap()) <= head,
                "inventory belongs to selected head"
            );
            Ok(raw[..32].try_into().unwrap())
        })
        .collect()
}
impl<R: StorageProof> ConcreteStateRead for CompleteReader<R> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.inner.identity()
    }
    fn account(
        &self,
        a: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.inner.account(a)
    }
    fn code(&self, h: [u8; 32]) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.inner.code(h)
    }
    fn storage(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match self.inner.storage(a, k) {
            Err(ConcreteReadError::HistoryUnavailable(_))
                if !self
                    .known
                    .contains(&rustaxa_storage::storage_version_prefix(a, k)) =>
            {
                match self.inner.nonmember(a, k)? {
                    true => Ok(ConcreteRead::Absent),
                    false => Err(ConcreteReadError::HistoryUnavailable(self.identity())),
                }
            }
            result => result,
        }
    }
}

impl<R: StorageProof> ConcreteStateRead for &CompleteReader<R> {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateRead::identity(*self)
    }
    fn account(
        &self,
        a: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        ConcreteStateRead::account(*self, a)
    }
    fn code(&self, h: [u8; 32]) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteStateRead::code(*self, h)
    }
    fn storage(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteStateRead::storage(*self, a, k)
    }
}

pub(crate) struct Port<'a> {
    pub(crate) inner: mixed_native::SimulationNativeExecutionPort<'a>,
    pub(crate) outcomes: Rc<RefCell<Vec<NativeOutcome>>>,
}
impl NativeExecutionPort for Port<'_> {
    fn input_mutations(
        &mut self,
        id: rustaxa_evm::contracts::NativeInvocationId,
    ) -> std::result::Result<Vec<rustaxa_evm::contracts::NativeInputMutation>, NativePortError>
    {
        self.inner.input_mutations(id)
    }

    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> std::result::Result<NativeGasQuote, NativePortError> {
        self.inner.prepare(i, j)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> std::result::Result<NativeInvocationResult, NativePortError> {
        let result = self.inner.invoke(i, q, j)?;
        if let NativeInvocationResult::Completed(out) = &result {
            self.outcomes.borrow_mut().push(out.clone());
        }
        Ok(result)
    }
}
fn block(period: u64, fixture: &Value) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: period.into(),
        author: fixed(&fixture["periods"][0]["reward_input"]["block_author"]),
        timestamp: 0,
        gas_limit: 10_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}
pub(crate) fn request(row: &Value, gas: u64, index: usize) -> ExecutionTransaction {
    ExecutionTransaction {
        position: (index as u32).into(),
        hash: [0; 32],
        sender: fixed(&row["sender"]),
        receiver: if row["to"].is_null() {
            None
        } else {
            Some(fixed(&row["to"]))
        },
        nonce: FinalChainNonce::from_bytes(
            &BigUint::parse_bytes(string(&row["supplied_nonce"]).as_bytes(), 10)
                .unwrap()
                .to_bytes_be(),
        )
        .unwrap(),
        gas_price: ExecutionGasPrice::new(
            BigUint::parse_bytes(string(&row["gas_price"]).as_bytes(), 10)
                .expect("declared API gas price"),
        ),
        gas_limit: gas.into(),
        value: ExecutionValue::new(
            BigUint::parse_bytes(string(&row["value"]).as_bytes(), 10).unwrap(),
        ),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: if row["to"].is_null() {
            ExecutionTransactionKind::Create
        } else {
            ExecutionTransactionKind::Call
        },
    }
}
pub(crate) fn compare(result: &TransactionExecutionResult, row: &Value) -> Result<EstimateProbe> {
    match result {
        TransactionExecutionResult::ConsensusFailure(out) => {
            ensure!(
                out.error == rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas,
                "API consensus error {:?}",
                out.error
            );
            ensure!(row["consensus_error"] == "intrinsic gas too low");
            ensure!(out.gas_used.as_u64() == number(&row["gas_used"]));
            ensure!(out.output == bytes(&row["output"]));
            Ok(EstimateProbe::ConsensusFailure {
                error: "intrinsic gas too low".into(),
            })
        }
        TransactionExecutionResult::Executed(out) => {
            let err = match &out.status {
                CodeExecutionStatus::Success => String::new(),
                CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) => e.error.clone(),
                CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas".into(),
                CodeExecutionStatus::Failure(CodeExecutionError::Revert) => {
                    "execution reverted".into()
                }
                CodeExecutionStatus::Failure(CodeExecutionError::InvalidOpcode(opcode)) => {
                    format!("invalid opcode 0x{opcode:02x}")
                }
                other => bail!("API status {other:?}"),
            };
            ensure!(row["consensus_error"] == "");
            ensure!(
                err == string(&row["execution_error"]),
                "API error {err:?}, expected {}",
                row["execution_error"]
            );
            ensure!(
                out.gas_used.as_u64() == number(&row["gas_used"]),
                "API gas {} expected {}",
                out.gas_used.as_u64(),
                row["gas_used"]
            );
            ensure!(out.output == bytes(&row["output"]), "API output");
            let logs=out.logs.iter().map(|l|serde_json::json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
            ensure!(
                logs == *row["logs"].as_array().context("API logs")?,
                "API logs"
            );
            Ok(if err.is_empty() {
                EstimateProbe::Success {
                    gas_used: out.gas_used.as_u64(),
                }
            } else {
                EstimateProbe::CodeFailure { error: err }
            })
        }
    }
}
pub(crate) fn compare_raw(outcomes: &[NativeOutcome], expected: &Value) -> Result<()> {
    let rows=outcomes.iter().flat_map(|o|o.raw_mutations.iter()).map(|w|serde_json::json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
    ensure!(
        rows == *expected.as_array().context("API raw writes")?,
        "API ordered native writes: actual {} expected {}",
        Value::Array(rows.clone()),
        expected
    );
    Ok(())
}
pub(super) fn verify(fixture: &Value, application: &Path, concrete: &Path) -> Result<()> {
    let api = fixture_stream("local.observer.traced.api")?;
    let cpp = fixture_stream("cpp.api")?;
    let (_storage, chain) = open_chain(application, fixture)?;
    let committed =
        ConcreteStateLifecycle::inspect_existing(concrete, chain.concrete_chain_identity()?)?
            .committed;
    let final_period = fixture["periods"].as_array().unwrap().last().unwrap();
    let mut probes = 0usize;
    let mut traces = 0usize;
    let mut estimate_index = 0usize;
    for case in api["cases"].as_array().context("API cases")? {
        let head = number(&case["head"]);
        let selected = ConcreteStateIdentity {
            period: head.into(),
            state_root: if head == 0 {
                fixed(&fixture["genesis"]["root"])
            } else {
                fixed(&fixture["periods"][head as usize - 1]["final"]["root"])
            },
        };
        let known = inventory(
            if head == 0 {
                &fixture["genesis"]
            } else {
                &fixture["periods"][head as usize - 1]["final"]
            },
            head,
        )?;
        let reader = CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(concrete, committed, selected)?,
            known,
        };
        let call = |probe: &Value| -> Result<EstimateProbe> {
            let req = request(&case["request"], number(&probe["gas"]), 0);
            let original = req.clone();
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let result = simulate_with_native(
                &reader,
                &ChainHashes(&chain),
                &AllNatives(TaraxaProfile::new(false)),
                &ConsensusNatives,
                |identity| {
                    assert_eq!(identity, selected);
                    Ok(Port {
                        inner: mixed_native::SimulationNativeExecutionPort::new(
                            chain
                                .begin_native_simulation(identity.period)
                                .map_err(|e| NativePortError::Domain(e.to_string()))?,
                        ),
                        outcomes: outcomes.clone(),
                    })
                },
                &block(head, fixture),
                &req,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .map_err(|e| anyhow::anyhow!("{e:?}"))
            .with_context(|| format!("historical call {} gas {}", case["name"], probe["gas"]))?;
            ensure!(
                result.state == selected && req == original,
                "API state/request identity"
            );
            compare_raw(&outcomes.borrow(), &probe["ordered_raw_writes"]).with_context(|| {
                format!(
                    "call {} selector {} gas {}",
                    case["name"],
                    &string(&case["request"]["input"])[..8],
                    probe["gas"]
                )
            })?;
            compare(&result.execution, &probe["output"])
        };
        call(&case["call"])?;
        for estimate in case["estimates"].as_array().unwrap() {
            let rows = estimate["probes"].as_array().unwrap();
            let mut at = 0;
            let result = estimate_gas(number(&estimate["cap"]), |gas| {
                let row = rows
                    .get(at)
                    .context("complete estimator probe transcript")?;
                at += 1;
                ensure!(gas == number(&row["gas"]), "estimator probe gas");
                call(row)
            });
            ensure!(at == rows.len(), "all estimator probes consumed");
            probes += at;
            // The C++ transcript is checked by the producer; the Rust search
            // must consume exactly the same probes and terminate identically.
            let expected = cpp["cases"]
                .as_array()
                .unwrap()
                .get(estimate_index)
                .context("ordered C++ estimate case")?;
            estimate_index += 1;
            ensure!(
                expected["name"] == estimate["name"],
                "C++ transcript identity"
            );
            match result {
                Ok(value) => ensure!(
                    expected["result"] == value,
                    "C++ estimate value {} actual {value} expected {}",
                    estimate["name"],
                    expected
                ),
                Err(rustaxa_evm::estimate::EstimateError::CodeFailure { error })
                | Err(rustaxa_evm::estimate::EstimateError::ConsensusFailure { error }) => ensure!(
                    expected["error"] == error,
                    "C++ estimate error {} {error}",
                    expected["error"]
                ),
                Err(e) => bail!("estimation {e:?}"),
            }
        }
        for name in ["trace", "trace_sequence"] {
            let trace = &case[name];
            let prefix = trace["prefix"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, r)| request(r, 500_000, i))
                .collect::<Vec<_>>();
            let targets = [request(&trace["request"], 500_000, prefix.len())];
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let result = run_structured_trace_with_native(
                &reader,
                &ChainHashes(&chain),
                &AllNatives(TaraxaProfile::new(false)),
                &ConsensusNatives,
                |identity, execution| {
                    assert_eq!(identity, selected);
                    Ok(Port {
                        inner: mixed_native::SimulationNativeExecutionPort::new(
                            chain
                                .begin_native_trace(identity.period, execution)
                                .map_err(|e| NativePortError::Domain(e.to_string()))?,
                        ),
                        outcomes: outcomes.clone(),
                    })
                },
                &block(head + 1, fixture),
                &prefix,
                &targets,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .with_context(|| format!("historical trace {} {name}", case["name"]))?;
            ensure!(result.state == selected);
            ensure!(
                serde_json::from_slice::<Value>(&result.output)? == trace["result"],
                "default trace {} {name}: actual {} expected {}",
                case["name"],
                String::from_utf8_lossy(&result.output),
                trace["result"]
            );
            let expected = trace["calls"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|c| c["ordered_raw_writes"].as_array().unwrap().iter().cloned())
                .collect::<Vec<_>>();
            compare_raw(&outcomes.borrow(), &Value::Array(expected))
                .with_context(|| format!("trace {} {name}", case["name"]))?;
            traces += 1;
        }
        if head > 0 {
            verify_final_reader(&reader, &fixture["periods"][head as usize - 1]["final"])?;
        }
    }
    ensure!(traces == 622 && probes > 933, "complete API workload");
    for extra in api["interacting_traces"]
        .as_array()
        .context("interacting trace oracle")?
    {
        let head = number(&extra["head"]);
        let state = &fixture["periods"][head as usize - 1]["final"];
        let selected = ConcreteStateIdentity {
            period: head.into(),
            state_root: fixed(&state["root"]),
        };
        let reader = CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(concrete, committed, selected)?,
            known: inventory(state, head)?,
        };
        let trace = &extra["trace"];
        let prefix = trace["prefix"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, r)| request(r, 500_000, i))
            .collect::<Vec<_>>();
        let targets = [request(&trace["request"], 500_000, prefix.len())];
        let outcomes = Rc::new(RefCell::new(Vec::new()));
        let result = run_structured_trace_with_native(
            &reader,
            &ChainHashes(&chain),
            &AllNatives(TaraxaProfile::new(false)),
            &ConsensusNatives,
            |identity, execution| {
                assert_eq!(identity, selected);
                Ok(Port {
                    inner: mixed_native::SimulationNativeExecutionPort::new(
                        chain
                            .begin_native_trace(identity.period, execution)
                            .map_err(|e| NativePortError::Domain(e.to_string()))?,
                    ),
                    outcomes: outcomes.clone(),
                })
            },
            &block(head + 1, fixture),
            &prefix,
            &targets,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .with_context(|| format!("interacting trace {}", extra["name"]))?;
        ensure!(
            serde_json::from_slice::<Value>(&result.output)? == trace["result"],
            "interacting trace result {}: actual {} expected {}",
            extra["name"],
            String::from_utf8_lossy(&result.output),
            trace["result"]
        );
        let expected = trace["calls"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["ordered_raw_writes"].as_array().unwrap().iter().cloned())
            .collect::<Vec<_>>();
        compare_raw(&outcomes.borrow(), &Value::Array(expected))
            .with_context(|| format!("interacting trace {}", extra["name"]))?;
        if extra["creation_prefix"] != true {
            ensure!(
                outcomes.borrow().len() == trace["calls"].as_array().unwrap().len(),
                "complete prefix/target outcomes"
            );
            for (out, row) in outcomes
                .borrow()
                .iter()
                .zip(trace["calls"].as_array().unwrap())
            {
                let error = match &out.status {
                    rustaxa_evm::contracts::NativeStatus::Success => "",
                    rustaxa_evm::contracts::NativeStatus::ContractFailure(e) => &e.error,
                };
                ensure!(error == string(&row["result"]["execution_error"]));
                ensure!(out.output == bytes(&row["result"]["output"]));
            }
        } else {
            ensure!(
                outcomes.borrow().len()
                    == extra["native_invocations"].as_u64().unwrap_or(1) as usize,
                "measured nested native invocation count"
            );
        }
        if extra["untraced"] == true {
            let mut journal = ExecutionJournal::new(&reader);
            let mut sequence = PeriodConsensusSequence::new((head + 1).into());
            let untraced_outcomes = Rc::new(RefCell::new(Vec::new()));
            let mut port = Port {
                inner: mixed_native::SimulationNativeExecutionPort::new(
                    chain.begin_native_trace(selected.period, (head + 1).into())?,
                ),
                outcomes: untraced_outcomes.clone(),
            };
            let requests = prefix
                .iter()
                .chain(targets.iter())
                .cloned()
                .collect::<Vec<_>>();
            let original = requests.clone();
            for (req, row) in requests.iter().zip(trace["calls"].as_array().unwrap()) {
                let out = match req.kind {
                    ExecutionTransactionKind::Call => execute_top_level_call_with_native(
                        &mut journal,
                        &ChainHashes(&chain),
                        &AllNatives(TaraxaProfile::new(false)),
                        &ConsensusNatives,
                        &mut port,
                        &mut sequence,
                        &block(head + 1, fixture),
                        req,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::new(false),
                    ),
                    ExecutionTransactionKind::Create => execute_top_level_create_with_native(
                        &mut journal,
                        &ChainHashes(&chain),
                        &AllNatives(TaraxaProfile::new(false)),
                        &ConsensusNatives,
                        &mut port,
                        &mut sequence,
                        &block(head + 1, fixture),
                        req,
                        EnvelopeRules { cornus: true },
                        TaraxaProfile::new(false),
                    ),
                    _ => unreachable!(),
                }?;
                compare(&out, &row["result"])?;
            }
            ensure!(requests == original, "canonical caller input is immutable");
            let expected = trace["calls"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|c| c["ordered_raw_writes"].as_array().unwrap().iter().cloned())
                .collect::<Vec<_>>();
            compare_raw(&untraced_outcomes.borrow(), &Value::Array(expected))?;
        }
        verify_final_reader(&reader, state)?;
    }
    let reader = ConcreteStateReader::open_read_only(concrete, committed)?;
    verify_final_reader(&reader, &final_period["final"])?;
    drop(reader);
    concrete_reader_regressions(fixture, concrete, committed)?;
    Ok(())
}

#[derive(Clone)]
struct FaultReader {
    identity: ConcreteStateIdentity,
    row: std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
    proof: std::result::Result<bool, ConcreteReadError>,
    proof_reads: Rc<std::cell::Cell<usize>>,
}
impl ConcreteStateRead for FaultReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }
    fn account(
        &self,
        _: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
    fn code(&self, _: [u8; 32]) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
    fn storage(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.row.clone()
    }
}
impl StorageProof for FaultReader {
    fn nonmember(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> std::result::Result<bool, ConcreteReadError> {
        self.proof_reads.set(self.proof_reads.get() + 1);
        self.proof.clone()
    }
}
#[test]
fn native_integration_complete_inventory_keeps_tombstones_members_orphans_and_proof_errors()
-> Result<()> {
    let identity = ConcreteStateIdentity {
        period: 5.into(),
        state_root: [17; 32],
    };
    let unavailable = ConcreteReadError::HistoryUnavailable(identity);
    let ordinary = [7; 20];
    let mut native = [0; 20];
    native[19] = 0xfe;
    let slots = [
        (ordinary, ConcreteStorageKey([0; 32])),
        (native, ConcreteStorageKey([1; 32])),
        (native, ConcreteStorageKey([2; 32])),
    ];
    // CF5 records the final tombstone even when creation and deletion occurred
    // in one period, and includes physical orphans absent from the trie/catalog.
    let mut physical = serde_json::Map::new();
    for (a, k) in slots {
        let version =
            rustaxa_storage::versioned_key(rustaxa_storage::storage_version_prefix(a, k), 5.into());
        physical.insert(hex::encode(version), Value::String(String::new()));
    }
    let state = serde_json::json!({"rows":[{},{},{},{},physical]});
    let known = inventory(&state, 5)?;
    ensure!(known.len() == 3);
    for (a, k) in slots {
        let count = Rc::new(std::cell::Cell::new(0));
        let reader = CompleteReader {
            inner: FaultReader {
                identity,
                row: Err(unavailable.clone()),
                proof: Ok(true),
                proof_reads: count.clone(),
            },
            known: known.clone(),
        };
        ensure!(
            reader.storage(a, k) == Err(unavailable.clone()),
            "missing known tombstone/orphan must remain unavailable"
        );
        ensure!(
            count.get() == 0,
            "known missing row must not use a nonmembership proof"
        );
        for row in [ConcreteRead::Tombstone, ConcreteRead::Present(Vec::new())] {
            let retry = CompleteReader {
                inner: FaultReader {
                    row: Ok(row.clone()),
                    ..reader.inner.clone()
                },
                known: known.clone(),
            };
            ensure!(
                retry.storage(a, k) == Ok(row),
                "fresh retry retains exact empty-row classification"
            );
        }
    }
    let key = ConcreteStorageKey([9; 32]);
    let fresh = |proof| CompleteReader {
        inner: FaultReader {
            identity,
            row: Err(unavailable.clone()),
            proof,
            proof_reads: Rc::new(std::cell::Cell::new(0)),
        },
        known: known.clone(),
    };
    ensure!(
        fresh(Ok(false)).storage(native, key) == Err(unavailable.clone()),
        "missing member bytes"
    );
    let proof_error = ConcreteReadError::Io("consumed proof read".into());
    ensure!(
        fresh(Err(proof_error.clone())).storage(native, key) == Err(proof_error),
        "proof failure is not absence"
    );
    ensure!(
        fresh(Ok(true)).storage(native, key) == Ok(ConcreteRead::Absent),
        "fresh verified nonmembership retry"
    );
    let corrupt = ConcreteReadError::Corrupt("malformed row".into());
    let reader = CompleteReader {
        inner: FaultReader {
            identity,
            row: Err(corrupt.clone()),
            proof: Ok(true),
            proof_reads: Rc::new(std::cell::Cell::new(0)),
        },
        known,
    };
    ensure!(reader.storage(native, key) == Err(corrupt));
    ensure!(reader.inner.proof_reads.get() == 0);
    ensure!(
        inventory(&state, 4).is_err(),
        "future physical inventory rejected"
    );
    for malformed in [
        serde_json::json!({}),
        serde_json::json!({"rows":[{},{},{},{},[]]}),
        serde_json::json!({"rows":[{},{},{},{},{"xx":""}]}),
        serde_json::json!({"rows":[{},{},{},{},{"00":""}]}),
    ] {
        ensure!(
            inventory(&malformed, 5).is_err(),
            "malformed physical inventory rejected"
        );
    }
    Ok(())
}

fn copy_owned_database(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file(),
            "owned concrete database flat files"
        );
        std::fs::copy(entry.path(), destination.join(entry.file_name()))?;
    }
    Ok(())
}
fn edit_owned_database(path: &Path, edit: impl FnOnce(&rocksdb::DB) -> Result<()>) -> Result<()> {
    let options = rocksdb::Options::default();
    let columns = rocksdb::DB::list_cf(&options, path)?;
    let db = rocksdb::DB::open_cf(&options, path, columns)?;
    edit(&db)?;
    db.flush()?;
    drop(db);
    Ok(())
}
fn concrete_reader_regressions(
    fixture: &Value,
    path: &Path,
    committed: ConcreteStateIdentity,
) -> Result<()> {
    let head = committed.period.as_u64();
    let known = inventory(&fixture["periods"][head as usize - 1]["final"], head)?;
    let ordinary = fixed(
        &fixture["periods"][1]["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "ordinary_storage_value_log")
            .unwrap()["to"],
    );
    let mut native = [0; 20];
    native[19] = 0xfe;
    let never = ConcreteStorageKey([0xa9; 32]);
    let reader = CompleteReader {
        inner: ConcreteStateReader::open_read_only(path, committed)?,
        known: known.clone(),
    };
    ensure!(
        reader.inner.storage(native, never)
            == Err(ConcreteReadError::HistoryUnavailable(committed))
    );
    ensure!(
        reader
            .inner
            .nonmember(native, never)
            .context("base never-written proof")?
    );
    ensure!(reader.storage(native, never) == Ok(ConcreteRead::Absent));
    drop(reader);
    let parent = path.parent().unwrap().join("reader-faults");
    std::fs::create_dir(&parent)?;
    let owned = parent.join("physical-empty-orphan");
    copy_owned_database(path, &owned)?;
    let samples = [
        (ordinary, ConcreteStorageKey([0xb1; 32])),
        (native, ConcreteStorageKey([0xb2; 32])),
        (native, ConcreteStorageKey([0xb3; 32])),
    ];
    let mut extended = known.clone();
    edit_owned_database(&owned, |db| {
        let cf = db.cf_handle("5").context("CF5")?;
        for (i, (a, k)) in samples.iter().enumerate() {
            let prefix = rustaxa_storage::storage_version_prefix(*a, *k);
            extended.insert(prefix);
            let physical = rustaxa_storage::versioned_key(prefix, committed.period);
            db.put_cf(cf, physical, [0xaa])?;
            if i < 2 {
                db.put_cf(cf, physical, [])?;
            }
        }
        Ok(())
    })?;
    let reopened = CompleteReader {
        inner: ConcreteStateReader::open_read_only(&owned, committed)?,
        known: extended.clone(),
    };
    for (i, (a, k)) in samples.iter().enumerate() {
        ensure!(
            reopened
                .inner
                .nonmember(*a, *k)
                .with_context(|| format!("owned empty/orphan proof {i}"))?
        );
        ensure!(
            reopened.storage(*a, *k)
                == Ok(if i < 2 {
                    ConcreteRead::Tombstone
                } else {
                    ConcreteRead::Present(vec![0xaa])
                })
        );
    }
    drop(reopened);
    let missing = parent.join("missing-physical");
    copy_owned_database(&owned, &missing)?;
    edit_owned_database(&missing, |db| {
        let cf = db.cf_handle("5").context("CF5")?;
        for (a, k) in samples {
            db.delete_cf(
                cf,
                rustaxa_storage::versioned_key(
                    rustaxa_storage::storage_version_prefix(a, k),
                    committed.period,
                ),
            )?;
        }
        Ok(())
    })?;
    let broken = CompleteReader {
        inner: ConcreteStateReader::open_read_only(&missing, committed)?,
        known: extended,
    };
    for (a, k) in samples {
        ensure!(
            broken
                .inner
                .nonmember(a, k)
                .context("missing empty/orphan proof")?
        );
        ensure!(broken.storage(a, k) == Err(ConcreteReadError::HistoryUnavailable(committed)));
    }
    drop(broken);
    // An actual ordinary trie member whose physical version bytes disappear.
    let member = parent.join("member-missing");
    copy_owned_database(path, &member)?;
    let slot = ConcreteStorageKey([0; 32]);
    let prefix = rustaxa_storage::storage_version_prefix(ordinary, slot);
    edit_owned_database(&member, |db| {
        let cf = db.cf_handle("5").context("CF5")?;
        let keys = db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .map(|row| row.map(|(k, _)| k))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for key in keys {
            if key.starts_with(&prefix) {
                db.delete_cf(cf, key)?;
            }
        }
        Ok(())
    })?;
    let mut broken = CompleteReader {
        inner: ConcreteStateReader::open_read_only(&member, committed)?,
        known: known.clone(),
    };
    let member_error = broken
        .inner
        .nonmember(ordinary, slot)
        .expect_err("member path cannot resolve missing physical value");
    ensure!(broken.storage(ordinary, slot) == Err(member_error));
    broken.known.remove(&prefix);
    ensure!(
        broken.storage(ordinary, slot) == Err(ConcreteReadError::HistoryUnavailable(committed))
    );
    drop(broken);
    let proof = parent.join("proof-missing");
    copy_owned_database(path, &proof)?;
    edit_owned_database(&proof, |db| {
        let cf = db.cf_handle("4").context("CF4")?;
        let keys = db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .map(|row| row.map(|(k, _)| k))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for key in keys {
            db.delete_cf(cf, key)?;
        }
        Ok(())
    })?;
    let broken = CompleteReader {
        inner: ConcreteStateReader::open_read_only(&proof, committed)?,
        known: known.clone(),
    };
    let error = broken
        .inner
        .nonmember(native, never)
        .expect_err("actual missing proof dependency");
    ensure!(broken.storage(native, never) == Err(error));
    drop(broken);
    let fresh = CompleteReader {
        inner: ConcreteStateReader::open_read_only(path, committed)?,
        known,
    };
    ensure!(fresh.storage(native, never) == Ok(ConcreteRead::Absent));
    ensure!(!fresh.inner.nonmember(ordinary, slot)?);
    ensure!(matches!(
        fresh.storage(ordinary, slot)?,
        ConcreteRead::Present(_)
    ));
    drop(fresh);
    let first = &fixture["periods"][0]["transactions"][0]["ordered_raw_writes"][0];
    let key = ConcreteStorageKey(fixed(&first["key"]));
    for h in [0u64, 1] {
        let state = if h == 0 {
            &fixture["genesis"]
        } else {
            &fixture["periods"][0]["final"]
        };
        let selected = ConcreteStateIdentity {
            period: h.into(),
            state_root: fixed(&state["root"]),
        };
        let reader = CompleteReader {
            inner: ConcreteStateReader::open_historical_read_only(path, committed, selected)?,
            known: inventory(state, h)?,
        };
        ensure!(reader.identity() == selected);
        ensure!(
            reader
                .known
                .contains(&rustaxa_storage::storage_version_prefix(native, key))
                == (h == 1)
        );
        if h == 0 {
            ensure!(reader.storage(native, key) == Ok(ConcreteRead::Absent));
        } else {
            ensure!(matches!(
                reader.storage(native, key)?,
                ConcreteRead::Present(_)
            ));
        }
    }
    ensure!(
        ConcreteStateReader::open_historical_read_only(
            path,
            committed,
            ConcreteStateIdentity {
                period: (head + 1).into(),
                state_root: [0; 32]
            }
        )
        .is_err()
    );
    Ok(())
}
