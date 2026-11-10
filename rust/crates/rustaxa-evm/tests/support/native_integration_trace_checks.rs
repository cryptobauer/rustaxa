use super::*;
use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;

fn live<R: ConcreteExecutionRead>(journal: &ExecutionJournal<R>, expected: &Value) -> Result<()> {
    for row in expected["accounts"]
        .as_array()
        .context("trace live accounts")?
    {
        let account = journal.account(fixed(&row["address"]))?;
        ensure!(
            BigUint::from_bytes_be(&account.nonce.to_bytes()).to_string() == string(&row["nonce"]),
            "trace account nonce"
        );
        ensure!(
            account.balance.value().to_string() == string(&row["balance"]),
            "trace account balance"
        );
    }
    for row in expected["slots"].as_array().context("trace live slots")? {
        let value = journal.raw_storage(
            fixed(&row["address"]),
            ConcreteStorageKey(fixed(&row["key"])),
        )?;
        let (callback, raw) = match value {
            ConcreteRead::Present(raw) => (true, raw),
            ConcreteRead::Absent | ConcreteRead::Tombstone => (false, Vec::new()),
        };
        let expected_raw = bytes(&row["value"]);
        ensure!(
            row["callback"] == callback && expected_raw == raw,
            "trace raw slot address {} key {}: callback {callback} expected {}, value {} ({} bytes) expected {} ({} bytes)",
            row["address"],
            row["key"],
            row["callback"],
            hex::encode(&raw[..raw.len().min(128)]),
            raw.len(),
            hex::encode(&expected_raw[..expected_raw.len().min(128)]),
            expected_raw.len()
        );
    }
    Ok(())
}

fn row_result(actual: &TransactionExecutionResult, expected: &Value) -> Result<()> {
    compare(actual, expected)?;
    let mut encoded = RlpStream::new_list(6);
    match actual {
        TransactionExecutionResult::Executed(out) => {
            encoded.append(&out.output);
            encoded.append(
                &out.attempted_contract_address
                    .unwrap_or_default()
                    .as_slice(),
            );
            encoded.begin_list(out.logs.len());
            for log in &out.logs {
                encoded.begin_list(3);
                encoded.append(&log.address.as_slice());
                encoded.begin_list(log.topics.len());
                for topic in &log.topics {
                    encoded.append(&topic.as_slice());
                }
                encoded.append(&log.data);
            }
            encoded.append(&out.gas_used.as_u64());
            let code = string(&expected["execution_error"]);
            encoded.append(&code);
            encoded.append(&"");
        }
        TransactionExecutionResult::ConsensusFailure(out) => {
            ensure!(
                expected["logs"]
                    .as_array()
                    .context("consensus logs")?
                    .is_empty()
                    && expected["execution_error"] == "",
                "consensus failure shape"
            );
            encoded.append(&out.output);
            encoded.append(
                &out.attempted_contract_address
                    .unwrap_or_default()
                    .as_slice(),
            );
            encoded.begin_list(0);
            encoded.append(&out.gas_used.as_u64());
            encoded.append(&"");
            let consensus = string(&expected["consensus_error"]);
            encoded.append(&consensus);
        }
    }
    ensure!(
        encoded.out().as_ref() == bytes(&expected["execution_result_rlp"]),
        "trace execution result RLP"
    );
    Ok(())
}

fn row_error(row: &Value) -> Result<()> {
    let result = &row["result"];
    let execution = string(&result["execution_error"]);
    let consensus = string(&result["consensus_error"]);
    let error = if execution.is_empty() {
        consensus
    } else {
        execution
    };
    ensure!(row["error"] == error, "trace call error");
    Ok(())
}

pub(super) fn verify(
    reader: &CompleteReader<ConcreteStateReader>,
    chain: &FinalChain,
    fixture: &Value,
    head: u64,
    trace: &Value,
) -> Result<()> {
    let prefix = trace["prefix"].as_array().context("trace prefix")?;
    let rows = trace["calls"].as_array().context("trace calls")?;
    ensure!(rows.len() == prefix.len() + 1, "trace call count");
    let mut journal = ExecutionJournal::new(reader);
    let mut sequence = PeriodConsensusSequence::new((head + 1).into());
    let outcomes = Rc::new(RefCell::new(Vec::new()));
    let mut port = Port {
        inner: mixed_native::SimulationNativeExecutionPort::new(
            chain.begin_native_trace(head.into(), (head + 1).into())?,
        ),
        outcomes: outcomes.clone(),
    };
    for (index, row) in rows.iter().enumerate() {
        let input = if index < prefix.len() {
            &prefix[index]
        } else {
            &trace["request"]
        };
        let req = request(input, 500_000, index);
        live(&journal, &row["before"]).with_context(|| format!("trace call {index} before"))?;
        if index == 0 {
            let mut changed = row["before"].clone();
            let accounts = changed["accounts"]
                .as_array_mut()
                .context("mutation accounts")?;
            ensure!(!accounts.is_empty(), "mutation account exists");
            accounts[0]["balance"] = Value::String("invalid balance".into());
            ensure!(
                live(&journal, &changed).is_err(),
                "before observation mutation rejected"
            );
        }
        let first_outcome = outcomes.borrow().len();
        let actual = match req.kind {
            ExecutionTransactionKind::Call => execute_top_level_call_with_native(
                &mut journal,
                &ChainHashes(chain),
                &AllNatives(TaraxaProfile::new(false), chain),
                &ConsensusNatives(chain),
                &mut port,
                &mut sequence,
                &block(head + 1, fixture),
                &req,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            ),
            ExecutionTransactionKind::Create => execute_top_level_create_with_native(
                &mut journal,
                &ChainHashes(chain),
                &AllNatives(TaraxaProfile::new(false), chain),
                &ConsensusNatives(chain),
                &mut port,
                &mut sequence,
                &block(head + 1, fixture),
                &req,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            ),
            _ => unreachable!(),
        }?;
        row_result(&actual, &row["result"])?;
        row_error(row)?;
        compare_raw(
            &outcomes.borrow()[first_outcome..],
            &row["ordered_raw_writes"],
        )?;
        live(&journal, &row["after"]).with_context(|| format!("trace call {index} after"))?;
        if index == 0 {
            let mut changed = row["after"].clone();
            let accounts = changed["accounts"]
                .as_array_mut()
                .context("mutation accounts")?;
            ensure!(!accounts.is_empty(), "mutation account exists");
            accounts[0]["balance"] = Value::String("invalid balance".into());
            ensure!(
                live(&journal, &changed).is_err(),
                "after observation mutation rejected"
            );
            let mut error = row.clone();
            error["error"] = Value::String("invalid error".into());
            ensure!(row_error(&error).is_err(), "error mutation rejected");
            let mut result = row["result"].clone();
            result["gas_used"] = Value::from(number(&result["gas_used"]) + 1);
            ensure!(
                row_result(&actual, &result).is_err(),
                "gas mutation rejected"
            );
            let mut result = row["result"].clone();
            result["logs"]
                .as_array_mut()
                .context("logs")?
                .push(serde_json::json!({"address":"00","topics":[],"data":""}));
            ensure!(
                row_result(&actual, &result).is_err(),
                "log mutation rejected"
            );
        }
    }
    Ok(())
}
