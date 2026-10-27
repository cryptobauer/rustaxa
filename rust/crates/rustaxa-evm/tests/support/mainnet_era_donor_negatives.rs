//! Actual Cacti donor/proof faults in newly registered private copies only.
use super::*;

#[test]
fn cacti_donor_proof_and_initialization_rejections() -> Result<()> {
    let fixture = corpus("cacti")?;
    let source_fixture = &fixture["seed"]["donor_document"];
    let base = PathBuf::from(std::env::var_os("RUN_ARTIFACT_DIR").context("persistent test root")?);
    let root = base.join(format!("donor-negative-baseline-{}", std::process::id()));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    bootstrap(&root, &fixture)?;
    let source_path = root.join("donor/state_db");
    let (application, chain) = open_chain(&root.join("donor/application"), source_fixture)?;
    let source =
        ConcreteStateLifecycle::inspect_existing(&source_path, chain.concrete_chain_identity()?)?;
    let known = api::complete_inventory(source_fixture, source.committed.period.as_u64())?;
    let proof = ConcreteStateReader::open_read_only(&source_path, source.committed)?;
    let owner = ConcreteStateLifecycle::open(
        &source_path,
        chain.concrete_chain_identity()?,
        source.committed,
    )?;
    let token = chain.capture_synthetic_fixture_donor(&owner, &proof, &known)?;
    for inventory in [
        BTreeSet::new(),
        {
            let mut wrong = known.clone();
            wrong.insert([0x42; 32]);
            wrong
        },
        {
            let mut wrong = known.clone();
            wrong.pop_first();
            wrong.insert([0x43; 32]);
            wrong
        },
    ] {
        let error = chain
            .capture_synthetic_fixture_donor(&owner, &proof, &inventory)
            .err()
            .context("forged inventory must fail")?;
        ensure!(
            error
                .to_string()
                .contains("SYNTHETIC_DONOR_HISTORY_INVENTORY")
        );
    }
    let prior = source_fixture["periods"].as_array().unwrap();
    let prior = &prior[prior.len() - 2];
    let wrong = ConcreteStateReader::open_historical_read_only(
        &source_path,
        source.committed,
        ConcreteStateIdentity {
            period: number(&prior["number"]).into(),
            state_root: fixed(&prior["final"]["root"]),
        },
    )?;
    ensure!(
        chain
            .capture_synthetic_fixture_donor(&owner, &wrong, &known)
            .err()
            .context("wrong proof identity")?
            .to_string()
            .contains("SYNTHETIC_DONOR_READER_IDENTITY")
    );
    drop(wrong);
    let wrong_root = ConcreteStateReader::open_historical_read_only(
        &source_path,
        source.committed,
        ConcreteStateIdentity {
            period: source.committed.period,
            state_root: [0xff; 32],
        },
    );
    ensure!(
        wrong_root
            .err()
            .context("wrong source proof root")?
            .to_string()
            .contains("root")
    );
    ensure!(
        chain
            .initialize_synthetic_fixture_snapshot_from_donor(&owner, &[], &[], vec![], &token)
            .unwrap_err()
            .to_string()
            .contains("SYNTHETIC_SEED_CONCRETE_ALREADY_EXECUTED")
    );
    drop(owner);
    let native = source_fixture["periods"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["final"]["native_catalog"]["slots"]
        .as_array()
        .unwrap();
    let row = native
        .iter()
        .find(|row| row["present"] == true && fixed::<20>(&row["address"])[19] == 0xfe)
        .context("actual DPoS member")?;
    let address = fixed(&row["address"]);
    let key = ConcreteStorageKey(fixed(&row["key"]));
    let prefix = rustaxa_storage::storage_version_prefix(address, key);
    let ConcreteRead::Present(account) = proof.account(address)? else {
        bail!("actual donor account")
    };
    let storage_root = account
        .account
        .storage_root
        .context("actual storage trie")?;
    ensure!(matches!(
        proof.verify_storage_path(address, key)?,
        rustaxa_storage::ConcreteStoragePath::Member(_)
    ));
    let absent = ConcreteStorageKey([0x42; 32]);
    ensure!(matches!(
        proof.verify_storage_path(address, absent)?,
        rustaxa_storage::ConcreteStoragePath::NonMember
    ));
    let mut outcomes = Vec::new();
    for fault in [
        "missing-account-proof",
        "corrupt-account-proof",
        "missing-storage-proof",
        "corrupt-storage-proof",
        "lost-known-history",
        "forged-none-member",
        "physical-orphan",
        "missing-code",
    ] {
        let case_root = base.join(format!("donor-fault-{fault}-{}", std::process::id()));
        let mut case = era_retention::register(&case_root, &fixture)?;
        std::fs::create_dir(&case_root)?;
        era_retention::created(&mut case)?;
        let path = case_root.join("state_db");
        api::copy_owned_application(&source_path, &path)?;
        let mutation = {
            let options = rocksdb::Options::default();
            let names = rocksdb::DB::list_cf(&options, &path)?;
            let db = rocksdb::DB::open_cf(&options, &path, names)?;
            let (column, target) = match fault {
                "missing-account-proof" | "corrupt-account-proof" => {
                    ("2", source.committed.state_root.to_vec())
                }
                "missing-storage-proof" | "corrupt-storage-proof" => ("4", storage_root.to_vec()),
                "missing-code" => (
                    "1",
                    account
                        .account
                        .code_hash
                        .context("actual DPoS code")?
                        .to_vec(),
                ),
                _ => ("5", Vec::new()),
            };
            let cf = db.cf_handle(column).context("actual physical column")?;
            let mut changed = Vec::new();
            if column == "5" {
                if fault == "physical-orphan" {
                    let mut key = rustaxa_storage::storage_version_prefix(address, absent).to_vec();
                    key.extend_from_slice(&source.committed.period.as_u64().to_be_bytes());
                    db.put_cf(cf, &key, [0xab])?;
                    changed.push(serde_json::json!({"key":hex::encode(key),"old":null,"new":"ab"}));
                } else {
                    let rows = db
                        .iterator_cf(cf, rocksdb::IteratorMode::Start)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    for (physical, value) in rows {
                        if physical.starts_with(&prefix) {
                            if fault == "forged-none-member" {
                                db.put_cf(cf, &physical, [])?;
                            } else {
                                db.delete_cf(cf, &physical)?;
                            }
                            changed.push(serde_json::json!({"key":hex::encode(physical),"old":hex::encode(value),"new":if fault=="forged-none-member" {Some("")}else{None}}));
                        }
                    }
                    ensure!(!changed.is_empty(), "actual known history mutation");
                }
            } else {
                let old = db
                    .get_cf(cf, &target)?
                    .context("actual proof/code target")?;
                if fault.starts_with("corrupt") {
                    db.put_cf(cf, &target, [0xff])?;
                } else {
                    db.delete_cf(cf, &target)?;
                }
                changed.push(serde_json::json!({"key":hex::encode(target),"old":hex::encode(old),"new":if fault.starts_with("corrupt") {Some("ff")}else{None}}));
            }
            db.flush()?;
            serde_json::json!({"column":column,"rows":changed})
        };
        let failure = (|| -> Result<()> {
            let reader = ConcreteStateReader::open_read_only(&path, source.committed)?;
            if fault == "lost-known-history" {
                ensure!(
                    matches!(
                        reader.storage(address, key),
                        Err(ConcreteReadError::HistoryUnavailable(_))
                    ),
                    "known physical loss remains unavailable"
                );
            }
            if fault.contains("storage-proof") {
                reader.verify_storage_path(address, key)?;
                bail!("proof fault unexpectedly returned a proof");
            }
            if fault == "missing-code" {
                let value = reader.code(account.account.code_hash.unwrap())?;
                ensure!(
                    matches!(value, ConcreteRead::Present(_)),
                    "missing actual code"
                );
            } else {
                if fault == "physical-orphan" {
                    ensure!(
                        matches!(reader.storage(address,absent)?,ConcreteRead::Present(value) if value==[0xab])
                    );
                    ensure!(matches!(
                        reader.verify_storage_path(address, absent)?,
                        rustaxa_storage::ConcreteStoragePath::NonMember
                    ));
                }
                let private = ConcreteStateLifecycle::open(
                    &path,
                    chain.concrete_chain_identity()?,
                    source.committed,
                )?;
                chain.capture_synthetic_fixture_donor(&private, &reader, &known)?;
            }
            Ok(())
        })();
        let error = failure.expect_err("actual donor physical fault must fail");
        ensure!(
            !error.to_string().contains("unexpectedly returned"),
            "proof fault was reached"
        );
        if matches!(fault, "lost-known-history" | "physical-orphan") {
            ensure!(
                error
                    .to_string()
                    .contains("SYNTHETIC_DONOR_HISTORY_INVENTORY")
            );
        } else if fault == "forged-none-member" {
            ensure!(
                error.to_string().contains("SYNTHETIC_DONOR_ABSENCE_PROOF")
                    || matches!(
                        error.downcast_ref::<ConcreteReadError>(),
                        Some(
                            ConcreteReadError::Corrupt(_)
                                | ConcreteReadError::HistoryUnavailable(_)
                        )
                    ),
                "logical-member/physical-tombstone conflict must reject:{error:#}"
            );
        } else {
            ensure!(
                error
                    .downcast_ref::<ConcreteReadError>()
                    .is_some_and(|error| matches!(
                        error,
                        ConcreteReadError::HistoryUnavailable(_) | ConcreteReadError::Corrupt(_)
                    )),
                "specific proof/code error:{error:#}"
            );
        }
        ensure!(
            proof.synthetic_storage_history_prefixes()? == known
                && proof.verify_storage_path(address, key)?.eq(
                    &rustaxa_storage::ConcreteStoragePath::Member(bytes(&row["value"]))
                ),
            "immutable baseline proof/history unchanged"
        );
        let evidence = serde_json::json!({"fault":fault,"source_period":source.committed.period.as_u64(),"source_root":hex::encode(source.committed.state_root),"address":hex::encode(address),"logical_key":hex::encode(key.0),"mutation":mutation,"actual_error":format!("{error:#}"),"baseline_proof_history_unchanged":true,"all_fault_handles_closed":true});
        outcomes.push(evidence.clone());
        era_retention::finish(case, evidence, false)?;
    }
    let seed = &fixture["seed"];
    let accounts = seed["native_catalog"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["present"] == true)
        .map(|r| fixed(&r["address"]))
        .collect::<Vec<_>>();
    let slots = seed["native_catalog"]["slots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["present"] == true)
        .map(|r| ConcreteStorageSlot {
            address: fixed(&r["address"]),
            key: fixed(&r["key"]),
        })
        .collect::<Vec<_>>();
    let source_reader = native_integration::api::CompleteReader {
        inner: proof,
        known: known.clone(),
    };
    for fault in [
        "wrong-root",
        "wrong-era",
        "wrong-policy",
        "partial-inventory",
        "missing-account",
        "known-target-history",
    ] {
        let case_root = base.join(format!("target-fault-{fault}-{}", std::process::id()));
        let mut case = era_retention::register(&case_root, &fixture)?;
        std::fs::create_dir(&case_root)?;
        era_retention::created(&mut case)?;
        let mut target_fixture = fixture.clone();
        if fault == "wrong-policy" {
            target_fixture["oracle_lifecycle"] = Value::String("public_batched".into());
        }
        let (app, target_chain) = open_chain(&case_root.join("application"), &target_fixture)?;
        let at = if fault == "wrong-era" {
            24_350_801
        } else {
            number(&seed["history"][0]["period"])
        };
        let owner = if fault == "wrong-root" {
            ConcreteStateLifecycle::create_synthetic_exclusive(
                case_root.join("state_db"),
                target_chain.concrete_chain_identity()?,
                at.into(),
                Default::default(),
                vec![],
            )?
        } else {
            concrete_from_donor(
                &case_root.join("state_db"),
                &target_chain,
                seed,
                at.into(),
                &source_reader,
            )?
        };
        let selected = owner.observation()?.committed;
        drop(owner);
        let mut mutation = serde_json::json!({"argument_fault":fault});
        if fault == "known-target-history" {
            let path = case_root.join("state_db");
            let options = rocksdb::Options::default();
            let db = rocksdb::DB::open_cf(&options, &path, rocksdb::DB::list_cf(&options, &path)?)?;
            let cf = db.cf_handle("5").unwrap();
            let rows = db
                .iterator_cf(cf, rocksdb::IteratorMode::Start)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut removed = Vec::new();
            for (k, value) in rows {
                if k.starts_with(&prefix) {
                    removed.push(serde_json::json!({"key":hex::encode(&k),"old":hex::encode(value),"new":null}));
                    db.delete_cf(cf, k)?;
                }
            }
            db.flush()?;
            ensure!(!removed.is_empty());
            mutation = serde_json::json!({"column":"5","rows":removed});
        }
        let owner = ConcreteStateLifecycle::open(
            case_root.join("state_db"),
            target_chain.concrete_chain_identity()?,
            selected,
        )?;
        let before = owner.observation()?;
        let present_accounts = if fault == "missing-account" {
            vec![]
        } else {
            accounts.clone()
        };
        let mut present_slots = slots.clone();
        if fault == "partial-inventory" {
            present_slots.remove(0);
        }
        let pending = fixture["pending_reward_facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| number(&r["number"]) <= at)
            .map(pending_fact)
            .collect::<Result<Vec<_>>>()?;
        let error = target_chain
            .initialize_synthetic_fixture_snapshot_from_donor(
                &owner,
                &present_accounts,
                &present_slots,
                pending,
                &token,
            )
            .expect_err("invalid target initialization must fail");
        let expected = match fault {
            "partial-inventory" => "SYNTHETIC_DONOR_TARGET_SLOT_INVENTORY",
            "missing-account" => "SYNTHETIC_DONOR_TARGET_ACCOUNT_INVENTORY",
            "known-target-history" => "SYNTHETIC_TARGET_HISTORY_UNAVAILABLE",
            _ => "SYNTHETIC_DONOR_POLICY_ERA_OR_ROOT",
        };
        ensure!(
            error.to_string().contains(expected),
            "intended target rejection {expected}:{error:#}"
        );
        ensure!(
            owner.observation()? == before,
            "invalid initialization preserves concrete facts"
        );
        ensure!(
            target_chain.last_block_number_typed()?.is_genesis(),
            "invalid private target never accepted"
        );
        drop(owner);
        drop(target_chain);
        drop(app);
        let (reopened_app, reopened) = open_chain(&case_root.join("application"), &target_fixture)?;
        ensure!(
            reopened.last_block_number_typed()?.is_genesis()
                && reopened_app
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    .is_none(),
            "incomplete target cannot reopen as accepted native head"
        );
        drop(reopened);
        drop(reopened_app);
        let evidence = serde_json::json!({"fault":fault,"mutation":mutation,"target_period":at,"target_root":hex::encode(selected.state_root),"actual_error":format!("{error:#}"),"expected_rejection":expected,"accepted_native_head":0,"physical_reopen_checked":true,"all_handles_closed":true});
        outcomes.push(evidence.clone());
        era_retention::finish(case, evidence, false)?;
    }
    let pending_root = base.join(format!("target-pending-{}", std::process::id()));
    let mut pending_case = era_retention::register(&pending_root, &fixture)?;
    std::fs::create_dir(&pending_root)?;
    era_retention::created(&mut pending_case)?;
    api::copy_owned_application(&root.join("application"), &pending_root.join("application"))?;
    api::copy_owned_application(&root.join("state_db"), &pending_root.join("state_db"))?;
    native_integration::run_segment_fault(
        &fixture,
        &fixture["periods"].as_array().unwrap()[..1],
        &pending_root.join("application"),
        &pending_root.join("state_db"),
        false,
        mixed_recovery::Fault::BeforeCommit,
    )?;
    let (app, pending_chain) = open_chain(&pending_root.join("application"), &fixture)?;
    let pending = ConcreteStateLifecycle::inspect_existing(
        pending_root.join("state_db"),
        pending_chain.concrete_chain_identity()?,
    )?;
    ensure!(!pending.pending_marker_rlp.is_empty());
    let pending_owner = ConcreteStateLifecycle::open(
        pending_root.join("state_db"),
        pending_chain.concrete_chain_identity()?,
        pending.committed,
    )?;
    let error = pending_chain
        .initialize_synthetic_fixture_snapshot_from_donor(
            &pending_owner,
            &accounts,
            &slots,
            vec![],
            &token,
        )
        .unwrap_err();
    ensure!(
        error
            .to_string()
            .contains("SYNTHETIC_SEED_CONCRETE_ALREADY_EXECUTED")
            && pending_owner.observation()? == pending
    );
    let head = pending_chain.last_block_number_typed()?.as_u64();
    ensure!(head == number(&seed["period"]));
    drop(pending_owner);
    drop(pending_chain);
    drop(app);
    era_retention::finish(
        pending_case,
        serde_json::json!({"mutation":"actual BeforeCommit execution left pending marker","expected_rejection":"SYNTHETIC_SEED_CONCRETE_ALREADY_EXECUTED","actual_error":error.to_string(),"concrete_marker_provenance_unchanged":true,"native_head_unchanged":head,"all_handles_closed":true}),
        false,
    )?;
    ensure!(
        ConcreteStateLifecycle::inspect_existing(&source_path, chain.concrete_chain_identity()?)?
            == source,
        "baseline lifecycle unchanged"
    );
    drop(source_reader);
    drop(chain);
    drop(application);
    era_retention::finish(
        scratch,
        serde_json::json!({"actual_cacti_donor":true,"source_inventory_and_identity_negatives":5,"post_execution_rejection":true,"pending_execution_rejection":true,"physical_and_initialization_faults":outcomes,"baseline_isolation":true}),
        false,
    )?;
    Ok(())
}
