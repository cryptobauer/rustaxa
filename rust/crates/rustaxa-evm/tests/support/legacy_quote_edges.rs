//! Focused signed legacy quote-edge transaction references.
//! Each mode owns one independently reconstructed B-2 donor and six discarded
//! B-1 overlays. This test does not publish an EndBlock or a FinalChain head.
use super::*;
use rustaxa_evm::envelope::IntrinsicGasSchedule;
use rustaxa_types::FinalChainTransactionPosition;
use rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker;
use rustaxa_types::concrete_lifecycle::{
    FinalChainConcreteExecutionMarker, FinalChainConcreteState,
};

fn checked_file(path: &Path, digest: &str) -> Result<Vec<u8>> {
    ensure!(path.is_absolute() && path.is_file() && !path.is_symlink());
    let bytes = std::fs::read(path)?;
    let hash = revm::precompile::hash::sha256_run(&bytes, u64::MAX)
        .map_err(|e| anyhow::anyhow!("reference hash: {e:?}"))?;
    ensure!(hex::encode(hash.bytes) == digest, "reference source hash");
    Ok(bytes)
}

fn compare_view<R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead>(
    view: &R,
    accounts: &Value,
    catalog: &Value,
) -> Result<()> {
    for row in accounts.as_array().context("actual account projection")? {
        let account = view.account(fixed(&row["address"]))?;
        ensure!(
            account_bytes(&account) == bytes(&row["raw_account"]),
            "actual account RLP"
        );
        ensure!((row["present"] == true) == matches!(account, ConcreteRead::Present(_)));
    }
    for row in catalog["slots"]
        .as_array()
        .context("actual native slot projection")?
    {
        let actual = read_raw_projection(
            view,
            fixed(&row["address"]),
            ConcreteStorageKey(fixed(&row["key"])),
        )?;
        ensure!(actual == bytes(&row["value"]), "actual native slot value");
        ensure!(
            (row["present"] == true) == !actual.is_empty(),
            "actual native slot presence"
        );
    }
    Ok(())
}

fn matching_case(doc: &Value, batch: u64, offset: i64) -> Result<&Value> {
    let cases = doc["cases"].as_array().context("actual cases")?;
    let found = cases
        .iter()
        .filter(|row| {
            row["batch"].as_u64() == Some(batch) && row["offset"].as_i64() == Some(offset)
        })
        .collect::<Vec<_>>();
    ensure!(found.len() == 1, "one actual case per identity");
    Ok(found[0])
}

fn verify_logs(actual: &[rustaxa_evm::contracts::ExecutionLog], expected: &Value) -> Result<()> {
    let expected = expected.as_array().context("actual transaction logs")?;
    ensure!(actual.len() == expected.len(), "actual log count");
    for (a, e) in actual.iter().zip(expected) {
        ensure!(a.address == fixed(&e["address"]) && a.data == bytes(&e["data"]));
        let topics = e["topics"].as_array().context("actual log topics")?;
        ensure!(a.topics.len() == topics.len());
        for (topic, expected) in a.topics.iter().zip(topics) {
            ensure!(*topic == fixed(expected));
        }
    }
    Ok(())
}

#[test]
fn legacy_quote_edges_actual_signed_top_level_and_transaction_roots() -> Result<()> {
    // The accepted descriptor and both reference documents must be pinned in
    // the source-bound binary launch. The descriptor is produced only after
    // independent review of actual Go outputs.
    let descriptor = std::env::var_os("RUSTAXA_LEGACY_QUOTE_EDGE_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../experiments/evm_feasibility/fixtures/mainnet_era_quote_edges/manifest.json"));
    let descriptor = std::fs::canonicalize(&descriptor)?;
    let launch = std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST")
        .map(|path| -> Result<Value> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
        .transpose()?;
    let check_launch_pin = |path: &Path, digest: &str| -> Result<()> {
        if let Some(launch) = &launch {
            let pins = launch["extra_inputs"]
                .as_array()
                .context("launch input pins")?;
            let matches = pins
                .iter()
                .filter(|p| p["path"].as_str() == path.to_str())
                .collect::<Vec<_>>();
            ensure!(
                matches.len() == 1 && matches[0]["sha256"] == digest,
                "exact launch input pin"
            );
        }
        Ok(())
    };
    let descriptor_bytes = std::fs::read(&descriptor)?;
    let descriptor_hash = revm::precompile::hash::sha256_run(&descriptor_bytes, u64::MAX)
        .map_err(|e| anyhow::anyhow!("descriptor hash: {e:?}"))?;
    check_launch_pin(&descriptor, &hex::encode(descriptor_hash.bytes))?;
    let accepted: Value = serde_json::from_slice(&descriptor_bytes)?;
    ensure!(accepted["decision"] == "ACCEPT_LEGACY_QUOTE_EDGE_PAIRED_ACTUAL");
    let mut docs = Vec::new();
    for mode in ["local", "public"] {
        let row = &accepted[mode];
        let declared = Path::new(string(&row["path"]));
        ensure!(
            declared.is_absolute()
                || (declared.components().count() == 1 && declared.file_name().is_some()),
            "reference path must be absolute or a sibling filename"
        );
        let path = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            descriptor
                .parent()
                .context("descriptor parent")?
                .join(declared)
        };
        let path = std::fs::canonicalize(path)?;
        check_launch_pin(&path, string(&row["sha256"]))?;
        let doc: Value = serde_json::from_slice(&checked_file(&path, string(&row["sha256"]))?)?;
        ensure!(doc["component"] == "claim_all_legacy_direct_quote_edges" && doc["mode"] == mode);
        ensure!(
            number(&doc["basis_period"]) == 7_599_998
                && number(&doc["execution_period"]) == 7_599_999
        );
        ensure!(
            doc["basis_members"]
                .as_array()
                .context("authenticated members")?
                .len()
                == 2
        );
        ensure!(doc["cases"].as_array().context("case list")?.len() == 6);
        docs.push(doc);
    }
    let local = &docs[0];
    for (mode, doc) in [("local", &docs[0]), ("public", &docs[1])] {
        let mut basis = doc["basis_document"].clone();
        if mode == "public" {
            attach_public_witnesses(&mut basis, &local["basis_document"], &local["reference"], 0)?;
        }
        let root = PathBuf::from(std::env::var_os("RUN_ARTIFACT_DIR").context("task scratch")?)
            .join(format!(
                "legacy-quote-edge-{mode}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
        let mut scratch = era_retention::register(&root, &basis)?;
        std::fs::create_dir(&root)?;
        era_retention::created(&mut scratch)?;
        bootstrap(&root, &basis)?;
        native_integration::run_segment(
            &basis,
            basis["periods"].as_array().context("basis warmup")?,
            &root.join("application"),
            &root.join("state_db"),
            false,
        )?;
        let (application, chain) = open_chain(&root.join("application"), &basis)?;
        let identity = chain.concrete_chain_identity()?;
        let state_path = root.join("state_db");
        let initial = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
        ensure!(
            initial.committed.period.as_u64() == 7_599_998 && initial.pending_marker_rlp.is_empty()
        );
        ensure!(
            initial.committed.state_root
                == fixed(&basis["periods"].as_array().unwrap().last().unwrap()["final"]["root"])
        );
        let mut concrete = ConcreteStateLifecycle::open(&state_path, identity, initial.committed)?;
        for batch in [0u64, 1] {
            for offset in [-1i64, 0, 1] {
                for _attempt in 0..2 {
                    let expected = matching_case(doc, batch, offset)?;
                    let local_case = matching_case(local, batch, offset)?;
                    ensure!(expected["basis_digest"] == doc["basis_digest"]);
                    let quote = if batch == 0 { 90_000u64 } else { 45_000u64 };
                    let requested = (quote as i64 + offset) as u64;
                    ensure!(number(&expected["requested_native_gas"]) == requested);
                    ensure!(
                        expected["begin_block_raw_writes"]
                            .as_array()
                            .context("begin-block writes")?
                            .is_empty()
                    );
                    let tx = &expected["transaction"];
                    ensure!(
                        string(&tx["consensus_error"]).is_empty(),
                        "actual reference must pass envelope admission"
                    );
                    let decoded = decode_legacy_input(
                        FinalChainTransactionPosition::new(0),
                        &bytes(&tx["signed_rlp"]),
                        LegacyInputKind::Signed,
                    )?;
                    ensure!(
                        decoded.hash == fixed(&tx["hash"])
                            && decoded.sender == fixed(&expected["sender"])
                    );
                    let mut expected_input = vec![0x09, 0xb7, 0x2e, 0x00];
                    expected_input.extend_from_slice(&[0u8; 32]);
                    expected_input[35] = batch as u8;
                    ensure!(
                        decoded.kind == ExecutionTransactionKind::Call
                            && decoded.receiver
                                == Some([
                                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe
                                ])
                            && decoded.input == expected_input
                            && decoded.value.value() == &BigUint::default()
                    );
                    let intrinsic = IntrinsicGasSchedule::PINNED
                        .calculate(&decoded.input, false)
                        .map_err(|error| anyhow::anyhow!("intrinsic gas: {error:?}"))?
                        .as_u64();
                    ensure!(
                        intrinsic == number(&expected["intrinsic"])
                            && decoded.gas_limit.as_u64() == intrinsic + requested
                    );
                    let current = concrete.observation()?;
                    ensure!(
                        current.committed == initial.committed
                            && current.pending_marker_rlp.is_empty()
                    );
                    let marker = FinalChainConcreteExecutionMarker {
                        identity: current.identity,
                        generation: current.generation + 1,
                        plan_hash: decoded.hash,
                        period: 7_599_999,
                        prior_state: FinalChainConcreteState {
                            period: 7_599_998,
                            root: initial.committed.state_root,
                        },
                        transactions_hash: decoded.hash,
                        rewards_hash: [0; 32],
                    };
                    let marker_bytes = encode_concrete_execution_marker(&marker);
                    concrete.stage_execution(&marker_bytes)?;
                    let source = FixtureView {
                        prior: FixturePrior::Lifecycle(&concrete),
                        identity: initial.committed,
                    };
                    compare_view(
                        &source,
                        &doc["basis_accounts"],
                        &doc["basis_native_catalog"],
                    )?;
                    let mut journal = ExecutionJournal::new(source);
                    let session = chain.begin_native_session_bound(
                        decoded.hash,
                        7_599_999u64.into(),
                        7_599_998u64.into(),
                    )?;
                    let mut native = MixedNativeExecutionPort::new(session);
                    let mut sequence = PeriodConsensusSequence::new(7_599_999u64.into());
                    let block = ExecutionBlockContext {
                        period: 7_599_999u64.into(),
                        author: fixed(&basis["inputs"]["initial_validators"][1]["address"]),
                        timestamp: 0,
                        gas_limit: 315_000_000u64.into(),
                        chain_id: number(&tx["chain_id"]),
                        difficulty: BigUint::default(),
                    };
                    let outcome = execute_top_level_call_with_native(
                        &mut journal,
                        &ChainHashes(&chain),
                        &AllNatives(TaraxaProfile::new(false), &chain),
                        &ConsensusNatives(&chain),
                        &mut native,
                        &mut sequence,
                        &block,
                        &decoded,
                        EnvelopeRules { cornus: false },
                        TaraxaProfile::new(false),
                    )?;
                    let TransactionExecutionResult::Executed(result) = outcome else {
                        bail!("Go admitted signed transaction but Rust did not")
                    };
                    ensure!(code_error(&result.status) == string(&tx["execution_error"]));
                    ensure!(
                        result.output == bytes(&tx["output"])
                            && result.gas_used.as_u64() == number(&tx["gas_used"])
                    );
                    ensure!(
                        c_composed_transaction_fee(
                            decoded.gas_price.value(),
                            result.gas_used.as_u64()
                        )
                        .to_string()
                            == string(&expected["fee"])
                    );
                    verify_logs(&result.logs, &tx["logs"])?;
                    ensure!(
                        result.refund_applied.as_u64() == number(&expected["refund"]["applied"])
                    );
                    let settled = journal.settle_transaction()?;
                    drop(journal);
                    ensure!(
                        settled.logs == result.logs
                            && settled.refund == number(&expected["refund"]["counter"])
                    );
                    let go_native = local_case["native_invocations"]
                        .as_array()
                        .context("actual local native observations")?;
                    ensure!(go_native.len() == 1 && settled.native_invocations.len() == 1);
                    let go = &go_native[0];
                    let observed = &settled.native_invocations[0];
                    ensure!(
                        observed.invocation.supplied_gas.as_u64() == number(&go["SuppliedGas"])
                            && observed.required_gas.as_u64() == number(&go["RequiredGas"])
                            && observed.gas_used.as_u64() == number(&go["GasUsed"])
                    );
                    ensure!(
                        number(&go["SuppliedGas"]) == requested
                            && number(&go["RequiredGas"]) == quote
                    );
                    ensure!(
                        observed.invocation.depth == number(&go["Depth"]) as u16
                            && code_error(&observed.status) == string(&go["Error"])
                            && hex::encode(observed.invocation.caller)
                                == string(&go["Caller"]).trim_start_matches("0x")
                            && observed.invocation.value.value() == &BigUint::default()
                    );
                    let contexts = native.take_contexts();
                    compare_raw_trace(&contexts, &expected["ordered_raw_writes"])?;
                    let report = FinalChainEvmTransactionResult {
                        position: decoded.position,
                        hash: decoded.hash,
                        status: u8::from(result.status == CodeExecutionStatus::Success),
                        gas_used: result.gas_used,
                        cumulative_gas_used: result.gas_used,
                        receipt_rlp: Vec::new(),
                        logs: result.logs.iter().map(project_log).collect(),
                        new_contract_address: result.attempted_contract_address,
                        output: result.output.clone(),
                        code_error: code_error(&result.status),
                        consensus_error: String::new(),
                    };
                    ensure!(
                        encode_external_evm_receipt(&report) == bytes(&tx["receipt_rlp"]),
                        "actual receipt bytes"
                    );
                    let mut accounts = Vec::new();
                    let mut storage = Vec::new();
                    let mut code = Vec::new();
                    for write in settled.writes.accounts {
                        accounts.push(match write.operation {
                            JournalAccountOperation::Upsert {
                                nonce,
                                balance,
                                code_hash,
                                code_size,
                            } => ConcreteAccountMutation::Upsert {
                                address: write.address,
                                record: record(nonce, balance, code_hash, code_size),
                            },
                            JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                                address: write.address,
                            },
                        });
                    }
                    for write in settled.writes.ordinary_storage {
                        storage.push(ConcreteStorageMutation {
                            address: write.address,
                            key: write.key,
                            value: (write.value != BigUint::default())
                                .then(|| write.value.to_bytes_be()),
                        });
                    }
                    for write in settled.writes.raw_storage {
                        storage.push(ConcreteStorageMutation {
                            address: write.address,
                            key: write.key,
                            value: raw_operation_value(&write.operation),
                        });
                    }
                    for write in settled.writes.code {
                        code.push(ConcreteCodeInsertion {
                            code_hash: write.code_hash,
                            code: write.code,
                        });
                    }
                    let output = concrete.apply_observer_phase(ConcreteObserverPhaseDelta {
                        accounts,
                        storage,
                        code,
                    })?;
                    ensure!(
                        output.identity().state_root
                            == fixed(&expected["transaction_checkpoint_root"]),
                        "actual transaction checkpoint root"
                    );
                    let view = FixtureView {
                        prior: FixturePrior::Prepared(concrete.prepared_view(&output)?),
                        identity: output.identity(),
                    };
                    compare_view(
                        &view,
                        &expected["accounts_after"],
                        &expected["native_catalog_after"],
                    )?;
                    concrete.discard_execution(&marker_bytes)?;
                    ensure!(
                        concrete.observation()?.committed == initial.committed
                            && concrete.observation()?.pending_marker_rlp.is_empty()
                    );
                }
            }
        }
        drop(concrete);
        drop(chain);
        drop(application);
        era_retention::finish(
            scratch,
            serde_json::json!({"component":"legacy_quote_edges","mode":mode,"cases":6,"attempts_each":2,"scope":"actual signed top-level transaction and discarded transaction-only checkpoint"}),
            true,
        )?;
    }
    Ok(())
}
