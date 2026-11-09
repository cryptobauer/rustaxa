//! Prospective success-only disposal of registered integration-test scratch.
//! There is deliberately no Drop cleanup. Errors and unwinding retain data.
use super::*;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::MetadataExt;

pub(super) struct Case {
    root: PathBuf,
    parent: PathBuf,
    ledger: PathBuf,
    launch: Value,
    input: Value,
    inode: Option<(u64, u64)>,
    parent_inode: (u64, u64),
    dependencies: std::rc::Rc<std::cell::Cell<usize>>,
}
pub(super) struct Dependency(std::rc::Rc<std::cell::Cell<usize>>);
impl Drop for Dependency {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

fn durable(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(path.parent().context("ledger parent")?)?.sync_all()?;
    Ok(())
}

fn no_links(path: &Path) -> Result<()> {
    for component in path.ancestors() {
        if component.exists() || component.is_symlink() {
            ensure!(
                !std::fs::symlink_metadata(component)?
                    .file_type()
                    .is_symlink(),
                "scratch symlink"
            );
        }
    }
    Ok(())
}

fn bytes(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = std::fs::symlink_metadata(entry.path())?;
        ensure!(!meta.file_type().is_symlink(), "scratch contains symlink");
        total += if meta.is_dir() {
            bytes(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

fn measure(launch: &Value) -> Result<u64> {
    let run = Path::new(launch["resources"]["run"].as_str().context("task run")?);
    let corpus = Path::new(launch["resources"]["corpus"].as_str().context("corpus")?);
    fn usage(path: &Path) -> Result<u64> {
        let mut total = 0;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let meta = std::fs::symlink_metadata(entry.path())?;
            total += if meta.is_dir() {
                usage(&entry.path())?
            } else {
                meta.len()
            };
        }
        Ok(total)
    }
    let actual = usage(run)? + usage(corpus)?;
    let base = launch["resources"]["baseline_actual_bytes"]
        .as_u64()
        .context("baseline bytes")?;
    let charge = launch["resources"]["prior_charged_bytes"]
        .as_u64()
        .context("prior charge")?;
    let current = charge + actual.saturating_sub(base) + 2 * 1024 * 1024;
    ensure!(
        current
            <= launch["resources"]["retained_live_cap_bytes"]
                .as_u64()
                .unwrap_or(1_342_177_280),
        "retained/live task cap exceeded:{current}"
    );
    Ok(current)
}

impl Case {
    /// Registers the exact absent path before any case creates it. The launch
    /// owner created the exclusive parent, outside all pre-authorization roots.
    pub(super) fn register(root: &Path, input: &Value) -> Result<Option<Self>> {
        let Some(path) = std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST") else {
            return Ok(None);
        };
        let launch: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let parent = PathBuf::from(
            launch["scratch_parent"]
                .as_str()
                .context("registered launch parent")?,
        );
        ensure!(
            root.is_absolute() && !root.exists() && !root.is_symlink(),
            "scratch root must be NEW"
        );
        ensure!(
            root.components()
                .all(|part| !matches!(part, std::path::Component::ParentDir)),
            "scratch path escape"
        );
        no_links(root)?;
        let canonical = std::fs::canonicalize(root.parent().context("scratch parent")?)?;
        ensure!(
            canonical.starts_with(std::fs::canonicalize(&parent)?) && canonical != root,
            "scratch containment"
        );
        let ledger = PathBuf::from(
            launch["scratch_ledger"]
                .as_str()
                .context("scratch ledger")?,
        );
        let input = serde_json::json!({"keccak256":hex::encode(revm::primitives::keccak256(serde_json::to_vec(input)?)),"scenario":input.get("scenario"),"configuration":input.get("configuration")});
        let metadata = std::fs::metadata(&parent)?;
        let parent_inode = (metadata.dev(), metadata.ino());
        ensure!(
            launch["scratch_parent_identity"]
                == serde_json::json!([parent_inode.0, parent_inode.1]),
            "launch parent identity changed"
        );
        let case = Self {
            root: root.to_owned(),
            parent,
            ledger,
            launch,
            input,
            inode: None,
            parent_inode,
            dependencies: Default::default(),
        };
        case.event("registered_before_creation", Value::Null)?;
        Ok(Some(case))
    }

    pub(super) fn created(&mut self) -> Result<()> {
        no_links(&self.root)?;
        let meta = std::fs::symlink_metadata(&self.root)?;
        ensure!(
            meta.is_dir() && self.inode.is_none(),
            "scratch exclusive creation"
        );
        self.inode = Some((meta.dev(), meta.ino()));
        self.event("created", Value::Null)
    }

    pub(super) fn dependent(&self) -> Dependency {
        self.dependencies.set(self.dependencies.get() + 1);
        Dependency(self.dependencies.clone())
    }

    fn event(&self, state: &str, detail: Value) -> Result<()> {
        durable(
            &self.ledger,
            &serde_json::json!({"state":state,"root":self.root,"launch":self.launch["identity"],"source_manifest":self.launch["source_manifest"],"input":self.input,"live_charged_bytes":measure(&self.launch)?,"detail":detail}),
        )
    }

    /// Explicit finish is called only after the complete scoped case returned
    /// successfully and every owner/reader/recovery dependency has closed.
    pub(super) fn finish(self, comparisons: Value, retain: bool) -> Result<()> {
        self.finish_with_scope(comparisons, retain, None)
    }

    fn finish_with_scope(
        self,
        comparisons: Value,
        retain: bool,
        qualification: Option<bool>,
    ) -> Result<()> {
        ensure!(
            self.dependencies.get() == 0 && !comparisons.is_null(),
            "unfinished dependency or missing comparisons"
        );
        no_links(&self.root)?;
        let parent = std::fs::metadata(&self.parent)?;
        ensure!(
            self.parent_inode == (parent.dev(), parent.ino()),
            "scratch parent replaced"
        );
        let meta = std::fs::symlink_metadata(&self.root)?;
        ensure!(
            self.inode == Some((meta.dev(), meta.ino())),
            "scratch root replaced"
        );
        ensure!(
            std::fs::canonicalize(&self.root)?.starts_with(std::fs::canonicalize(&self.parent)?),
            "scratch containment changed"
        );
        let before = bytes(&self.root)?;
        let witness = serde_json::json!({"schema":1,"root":self.root,"inode":self.inode,"launch":self.launch["identity"],"source_manifest":self.launch["source_manifest"],"input":self.input,"comparisons_completed":comparisons,"caller_qualification_flag":qualification,"all_handles_and_dependencies_closed":true,"bytes":before,"disposition":if retain {"retain"}else{"dispose"}});
        let encoded = serde_json::to_vec(&witness)?;
        let hash = revm::primitives::keccak256(&encoded);
        let witness_path = self
            .ledger
            .parent()
            .context("witness parent")?
            .join(format!("case-{}.json", hex::encode(hash)));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&witness_path)?;
        output.write_all(&encoded)?;
        output.sync_all()?;
        drop(output);
        File::open(witness_path.parent().context("witness parent")?)?.sync_all()?;
        ensure!(
            std::fs::read(&witness_path)? == encoded,
            "durable witness reread"
        );
        self.event(
            "checks_complete_handles_closed_witness_saved",
            serde_json::json!({"witness":witness_path,"keccak256":hex::encode(hash)}),
        )?;
        if !retain {
            no_links(&self.root)?;
            let meta = std::fs::symlink_metadata(&self.root)?;
            ensure!(
                self.inode == Some((meta.dev(), meta.ino())),
                "scratch replaced before removal"
            );
            std::fs::remove_dir_all(&self.root)?;
        }
        self.event(if retain {"retained"}else{"disposed"}, serde_json::json!({"before_bytes":before,"after_bytes":if retain {bytes(&self.root)?}else{0}}))
    }
}

pub(super) fn register(root: &Path, input: &Value) -> Result<Option<Case>> {
    Case::register(root, input)
}
pub(super) fn created(case: &mut Option<Case>) -> Result<()> {
    if let Some(case) = case {
        case.created()?;
    }
    Ok(())
}
fn retention_requested(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("qualification"))
}

pub(super) fn finish(case: Option<Case>, comparisons: Value, qualification: bool) -> Result<()> {
    if let Some(case) = case {
        let retain = retention_requested(std::env::var_os("RUSTAXA_ERA_RETAIN").as_deref());
        case.finish_with_scope(comparisons, retain, Some(qualification))?;
    }
    Ok(())
}

pub(super) fn evidence(fixture: &Value, checks: &str) -> Result<Value> {
    let digest = |value: &Value| -> Result<String> {
        Ok(hex::encode(revm::primitives::keccak256(
            serde_json::to_vec(value)?,
        )))
    };
    let periods = fixture["periods"].as_array().map(Vec::as_slice).unwrap_or_default().iter().map(|row| Ok(serde_json::json!({"period":row["number"],"root":row["final"]["root"],"transactions_receipts_observations_keccak256":digest(&row["transactions"])? ,"native_history_keccak256":digest(&row["final"])?}))).collect::<Result<Vec<_>>>()?;
    Ok(
        serde_json::json!({"completed_assertions":checks,"scenario":fixture["scenario"],"lifecycle":fixture["oracle_lifecycle"],"compared_period_inputs":periods,"fixture_api_cases":fixture["api_cases"].as_array().map_or(0,Vec::len),"api_transcript_keccak256":digest(&fixture["api_cases"])?,"fixture_history_keccak256":digest(&fixture["history_rows"])?}),
    )
}

pub(super) fn final_evidence(root: &Path, fixture: &Value, checks: &str) -> Result<Value> {
    let actual = mixed_recovery::final_facts(root, fixture)?;
    let mut result = evidence(fixture, checks)?;
    result["actual_final_facts_keccak256"] = Value::String(hex::encode(
        revm::primitives::keccak256(format!("{actual:?}").as_bytes()),
    ));
    Ok(result)
}

#[test]
fn retention_rejects_unsafe_completion_and_preserves_panics() -> Result<()> {
    // Independent miniature ledger; exercise the actual state/ownership methods
    // without changing process-wide environment used by the integration cases.
    let parent = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("persistent test parent")?
        .join(format!("retention-contract-{}", std::process::id()));
    let mut parent_case = register(&parent, &serde_json::json!({"test":"retention contract"}))?;
    std::fs::create_dir(&parent)?;
    created(&mut parent_case)?;
    let launch: Value = serde_json::from_slice(&std::fs::read(std::env::var(
        "RUSTAXA_ERA_LAUNCH_MANIFEST",
    )?)?)?;
    let make = |name: &str| -> Result<Case> {
        let root = parent.join(name);
        let metadata = std::fs::metadata(&parent)?;
        let mut case = Case {
            root: root.clone(),
            parent: parent.clone(),
            ledger: parent.join("ledger"),
            launch: launch.clone(),
            input: serde_json::json!({"test":name}),
            inode: None,
            parent_inode: (metadata.dev(), metadata.ino()),
            dependencies: Default::default(),
        };
        case.event("registered_before_creation", Value::Null)?;
        std::fs::create_dir(&root)?;
        case.created()?;
        Ok(case)
    };
    make("success")?.finish(serde_json::json!({"assertions":"passed"}), false)?;
    ensure!(!parent.join("success").exists() && parent.join("ledger").is_file());
    make("retain")?.finish(serde_json::json!({"assertions":"passed"}), true)?;
    ensure!(parent.join("retain").exists());
    let missing = make("missing-witness")?;
    ensure!(missing.finish(Value::Null, false).is_err() && parent.join("missing-witness").exists());
    let dependent = make("dependent")?;
    let _reader = dependent.dependent();
    ensure!(
        dependent
            .finish(serde_json::json!({"checks":true}), false)
            .is_err()
            && parent.join("dependent").exists()
    );
    let panic_case = make("panic")?;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _owned = panic_case;
        panic!("expected unwind");
    }));
    ensure!(parent.join("panic").exists());
    let unknown = make("unknown")?;
    drop(unknown);
    ensure!(parent.join("unknown").exists());
    let unexpected = make("unexpected-error")?;
    drop(unexpected);
    ensure!(parent.join("unexpected-error").exists());
    ensure!(Case::register(&parent, &Value::Null).is_err());
    ensure!(Case::register(&parent.join("../escape"), &Value::Null).is_err());
    let mut unwritable = make("unwritable-witness")?;
    unwritable.ledger = parent.join("missing-parent/ledger");
    ensure!(
        unwritable
            .finish(serde_json::json!({"checks":true}), false)
            .is_err()
            && parent.join("unwritable-witness").exists()
    );
    let link = make("link")?;
    std::os::unix::fs::symlink("/", link.root.join("escape"))?;
    ensure!(
        link.finish(serde_json::json!({"checks":true}), false)
            .is_err()
    );
    let replaced = make("replaced")?;
    std::fs::rename(&replaced.root, parent.join("original"))?;
    std::fs::create_dir(&replaced.root)?;
    ensure!(
        replaced
            .finish(serde_json::json!({"checks":true}), false)
            .is_err()
    );
    ensure!(no_links(&parent.join("link/escape/tmp")).is_err());
    std::os::unix::fs::symlink(&parent, parent.join("root-symlink"))?;
    ensure!(Case::register(&parent.join("root-symlink"), &Value::Null).is_err());
    let mut parent_replaced = make("parent-replaced-child")?;
    let child_inode = parent_replaced.inode;
    let container = parent.join("replaceable-parent");
    std::fs::create_dir(&container)?;
    let moved = container.join("child");
    std::fs::rename(&parent_replaced.root, &moved)?;
    parent_replaced.root = moved;
    parent_replaced.parent = container.clone();
    let original = std::fs::metadata(&container)?;
    parent_replaced.parent_inode = (original.dev(), original.ino());
    let saved = parent.join("saved-parent");
    std::fs::rename(&container, &saved)?;
    std::fs::create_dir(&container)?;
    std::fs::rename(saved.join("child"), container.join("child"))?;
    ensure!(parent_replaced.inode == child_inode);
    ensure!(
        parent_replaced
            .finish(serde_json::json!({"checks":true}), false)
            .is_err()
            && container.join("child").exists()
    );
    let isolated = make("expected-error-isolated")?;
    let baseline = parent.join("baseline-input");
    std::fs::write(&baseline, b"original")?;
    std::fs::write(isolated.root.join("private-copy"), b"fault")?;
    let rejection = (|| -> Result<()> {
        ensure!(
            std::fs::read(isolated.root.join("private-copy"))? != b"fault",
            "injected rejection"
        );
        Ok(())
    })();
    let error = rejection.unwrap_err();
    ensure!(error.to_string() == "injected rejection" && std::fs::read(&baseline)? == b"original");
    isolated.finish(serde_json::json!({"actual_error":error.to_string(),"mutation":"private-copy=fault","baseline_actual":"original","isolation_assertion":true}), false)?;
    let _ = File::open(parent.join("ledger"))?;
    durable(
        &parent.join("contract-result.jsonl"),
        &serde_json::json!({"launch":launch["identity"],"all_ownership_completion_and_witness_negative_assertions_passed":true,"unsafe_roots_retained":true}),
    )?;
    Ok(())
}

#[cfg(test)]
mod retention_scope_controls {
    use super::*;

    fn case(parent: &Path) -> Result<Case> {
        std::fs::create_dir(parent)?;
        let root = parent.join("case");
        std::fs::create_dir(&root)?;
        let meta = std::fs::metadata(parent)?;
        let mut case = Case {
            root,
            parent: parent.to_owned(),
            ledger: parent.join("ledger.jsonl"),
            launch: serde_json::json!({"identity":"control","source_manifest":"control",
                "resources":{"run":parent,"corpus":parent,"baseline_actual_bytes":0,
                    "prior_charged_bytes":0,"retained_live_cap_bytes":u64::MAX}}),
            input: serde_json::json!({"scenario":"retention-control"}),
            inode: None,
            parent_inode: (meta.dev(), meta.ino()),
            dependencies: Default::default(),
        };
        case.created()?;
        Ok(case)
    }

    #[test]
    fn retention_request_is_separate_from_qualification_scope() -> Result<()> {
        use std::ffi::OsStr;
        ensure!(
            retention_requested(Some(OsStr::new("qualification"))),
            "retain request missing"
        );
        ensure!(
            !retention_requested(None) && !retention_requested(Some(OsStr::new("other"))),
            "unexpected retain request"
        );
        let base = std::env::var_os("RUN_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let parent = base.join(format!(
            "retention-scope-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&parent)?;
        let retained = case(&parent.join("retain"))?;
        let retained_root = retained.root.clone();
        std::fs::write(retained_root.join("owned-data"), b"actual control bytes")?;
        retained.finish_with_scope(
            serde_json::json!({"checks":"recovery only"}),
            true,
            Some(false),
        )?;
        ensure!(
            std::fs::read(retained_root.join("owned-data"))? == b"actual control bytes",
            "retained data missing"
        );
        let witnesses = std::fs::read_dir(parent.join("retain"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("case-"))
            .collect::<Vec<_>>();
        ensure!(witnesses.len() == 1, "retention witness count");
        let witness: Value = serde_json::from_slice(&std::fs::read(witnesses[0].path())?)?;
        ensure!(
            witness["caller_qualification_flag"] == false && witness["disposition"] == "retain",
            "scope and disposition conflated"
        );
        let disposed = case(&parent.join("dispose"))?;
        let disposed_root = disposed.root.clone();
        disposed.finish_with_scope(serde_json::json!({"checks":"complete"}), false, Some(true))?;
        ensure!(!disposed_root.exists(), "unrequested retention");
        let unfinished = case(&parent.join("unfinished"))?;
        let unfinished_root = unfinished.root.clone();
        let dependency = unfinished.dependent();
        ensure!(
            unfinished
                .finish_with_scope(
                    serde_json::json!({"checks":"incomplete"}),
                    false,
                    Some(false)
                )
                .is_err(),
            "open dependency accepted"
        );
        drop(dependency);
        ensure!(unfinished_root.exists(), "unfinished case removed");
        Ok(())
    }
}
