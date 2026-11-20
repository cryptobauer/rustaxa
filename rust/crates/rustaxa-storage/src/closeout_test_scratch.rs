//! Closeout hooks shared by existing storage, consensus and EVM test helpers.
//! Drop only records closure; the source-bound command owner decides disposal.
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn tainted() -> &'static std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>> {
    static ROOTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
    > = std::sync::OnceLock::new();
    ROOTS.get_or_init(Default::default)
}

fn locked_ledger(path: &Path) -> anyhow::Result<std::fs::File> {
    let output = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    std::fs::File::lock(&output)?;
    Ok(output)
}

fn record(path: &Path, state: &str) -> anyhow::Result<()> {
    let launch_path = std::env::var("RUSTAXA_ERA_LAUNCH_MANIFEST")?;
    let launch: serde_json::Value = serde_json::from_slice(&std::fs::read(launch_path)?)?;
    let parent = Path::new(
        launch["scratch_parent"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("scratch parent"))?,
    );
    anyhow::ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
    );
    for part in path.ancestors() {
        if part.exists() || part.is_symlink() {
            anyhow::ensure!(!std::fs::symlink_metadata(part)?.file_type().is_symlink());
        }
    }
    anyhow::ensure!(
        path.parent()
            .unwrap()
            .canonicalize()?
            .starts_with(parent.canonicalize()?)
    );
    let parent_meta = std::fs::metadata(parent)?;
    anyhow::ensure!(
        launch["scratch_parent_identity"]
            == serde_json::json!([parent_meta.dev(), parent_meta.ino()]),
        "scratch parent identity changed"
    );
    let ledger = Path::new(
        launch["unit_scratch_ledger"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("unit ledger"))?,
    );
    if state == "created" && !ledger.exists() {
        return Ok(());
    }
    let mut output = locked_ledger(ledger)?;
    if state == "created" {
        let registered = std::fs::read_to_string(ledger)?
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|row| {
                row["state"] == "registered_before_creation"
                    && row["root"].as_str() == path.to_str()
            });
        if !registered {
            return Ok(());
        }
    }
    if state == "registered_before_creation" {
        anyhow::ensure!(!path.exists(), "unit scratch must be new");
    }
    let metadata = if state != "registered_before_creation" {
        Some(std::fs::symlink_metadata(path)?)
    } else {
        None
    };
    fn usage(path: &Path) -> std::io::Result<u64> {
        let mut total = 0;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let meta = match std::fs::symlink_metadata(entry.path()) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            total += if meta.is_dir() {
                usage(&entry.path())?
            } else {
                meta.len()
            };
        }
        Ok(total)
    }
    let identity_only = state == "created" && std::fs::read_dir(path)?.next().is_none();
    let live = if identity_only {
        None
    } else {
        let resources = &launch["resources"];
        let actual = usage(Path::new(resources["run"].as_str().unwrap()))?
            + usage(Path::new(resources["corpus"].as_str().unwrap()))?;
        let measured = resources["prior_charged_bytes"].as_u64().unwrap()
            + actual.saturating_sub(resources["baseline_actual_bytes"].as_u64().unwrap())
            + 2 * 1024 * 1024;
        anyhow::ensure!(
            measured
                + if state == "registered_before_creation" {
                    64 * 1024 * 1024
                } else {
                    0
                }
                <= resources["retained_live_cap_bytes"]
                    .as_u64()
                    .unwrap_or(1_342_177_280),
            "unit live peak admission"
        );
        Some(measured)
    };
    let row = serde_json::json!({"state":state,"root":path,"test":std::thread::current().name(),"launch":launch["identity"],"source_manifest":launch["source_manifest"],"panicking":std::thread::panicking(),"identity":metadata.map(|meta|(meta.dev(),meta.ino())),"live_charged_bytes":live,"measurement_scope":if identity_only {"identity_only_empty_new_root"} else {"complete_task_usage"},"completion_scope":"whole_source_stable_command","tainted":tainted().lock().unwrap().contains(path)});
    serde_json::to_writer(&mut output, &row)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    std::fs::File::open(ledger.parent().unwrap())?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod ledger_tests {
    #[test]
    fn application_concrete_preparation_ledger_records_are_atomic() {
        use std::io::Write;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = super::new_path(
            std::env::temp_dir().join(format!(
                "rustaxa-closeout-ledger-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )),
            true,
        );
        if !root.exists() {
            std::fs::create_dir(&root).unwrap();
        }
        let path = root.join("records.jsonl");
        std::thread::scope(|scope| {
            for worker in 0..4_u64 {
                let path = &path;
                scope.spawn(move || {
                    for index in 0..64_u64 {
                        let mut output = super::locked_ledger(path).unwrap();
                        let row = serde_json::json!({
                            "worker": worker,
                            "index": index,
                            "payload": "ledger record".repeat(16),
                        });
                        serde_json::to_writer(&mut output, &row).unwrap();
                        output.write_all(b"\n").unwrap();
                    }
                });
            }
        });
        let rows = std::fs::read_to_string(&path).unwrap();
        let identities = rows
            .lines()
            .map(|line| {
                let row: serde_json::Value = serde_json::from_str(line).unwrap();
                (
                    row["worker"].as_u64().unwrap(),
                    row["index"].as_u64().unwrap(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(rows.lines().count(), 256);
        assert_eq!(identities.len(), 256);
        assert_eq!(identities.first(), Some(&(0, 0)));
        assert_eq!(identities.last(), Some(&(3, 63)));
        if !super::preserve(&root) {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
pub fn register(path: &Path) {
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_some() {
        record(path, "registered_before_creation").expect("registered new storage test scratch");
    }
}
pub fn created(path: &Path) {
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_some() {
        record(path, "created").expect("unit scratch creation identity");
    }
}
pub fn preserve(path: &Path) -> bool {
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_none() {
        return false;
    }
    // A missing closure record, including an I/O error during unwind, retains
    // the root. Never panic or erase diagnostic state from this destructor.
    if std::thread::panicking() {
        tainted().lock().unwrap().insert(path.to_owned());
    }
    if record(path, "handles_closed").is_err() {
        // A caught panic must not turn a failed terminal append into success.
        // Exit without unwinding or a core dump; the command owner retains all roots.
        std::process::exit(125);
    }
    true
}

/// Allocate a fresh, registered test root before any database opens. The caller
/// chooses exclusive directory creation when its existing helper needs it.
pub fn new_path(path: std::path::PathBuf, create: bool) -> std::path::PathBuf {
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_some() {
        register(&path);
        if create {
            std::fs::create_dir(&path).expect("exclusive registered test root");
            created(&path);
        }
    }
    path
}

/// Request disposal; the parent waits for source-stable whole-command success.
/// It never interprets this request as proof that dependent handles are closed.
pub fn dispose(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_none() {
        return std::fs::remove_dir_all(path);
    }
    if path.exists() {
        if std::thread::panicking() {
            tainted().lock().unwrap().insert(path.to_owned());
        }
        if record(path, "disposal_requested").is_err() {
            // This also covers a caught panic after an earlier successful
            // request: the whole test child must not return success.
            std::process::exit(125);
        }
    }
    Ok(())
}

/// A pre-open stale-directory clear must never erase prior data in a closeout.
pub fn clear_before_open(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_none() {
        return std::fs::remove_dir_all(path);
    }
    if std::fs::read_dir(path)?.next().is_some() {
        return Err(std::io::Error::other("new test root must be empty"));
    }
    Ok(())
}

#[test]
fn closeout_registered_adapter_defers_panic_and_live_handle_disposal() {
    if std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST").is_none() {
        return;
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = new_path(
        std::env::temp_dir().join(format!("retention-adapter-{}-{nonce}", std::process::id())),
        true,
    );
    clear_before_open(&root).unwrap();
    let data = root.join("live-reader");
    std::fs::write(&data, b"retained through child termination").unwrap();
    let reader = std::fs::File::open(&data).unwrap();
    dispose(&root).unwrap();
    assert!(root.exists());
    drop(reader);
    struct PanickingRoot<'a>(&'a Path);
    impl Drop for PanickingRoot<'_> {
        fn drop(&mut self) {
            dispose(self.0).unwrap();
        }
    }
    let caught = std::panic::catch_unwind(|| {
        let _root = PanickingRoot(&root);
        panic!("expected retention adapter unwind");
    });
    assert!(caught.is_err());
    assert!(root.exists());
    assert!(tainted().lock().unwrap().contains(&root));
    // A later successful request cannot erase the earlier panic or taint.
    dispose(&root).unwrap();
    assert_eq!(
        std::fs::read(data).unwrap(),
        b"retained through child termination"
    );
}

#[test]
fn closeout_terminal_record_failure_exits_and_retains_earlier_request() {
    let Some(launch_path) = std::env::var_os("RUSTAXA_ERA_LAUNCH_MANIFEST") else {
        return;
    };
    if let Some(root) = std::env::var_os("RUSTAXA_RETENTION_FAILURE_CHILD_ROOT") {
        let root = new_path(root.into(), true);
        std::fs::write(root.join("diagnostic"), b"must survive failed panic record").unwrap();
        dispose(&root).unwrap();
        let launch: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&launch_path).unwrap()).unwrap();
        let ledger = std::path::PathBuf::from(launch["unit_scratch_ledger"].as_str().unwrap());
        std::fs::rename(&ledger, ledger.with_extension("saved.jsonl")).unwrap();
        std::fs::create_dir(&ledger).unwrap();
        struct PanicRoot(std::path::PathBuf);
        impl Drop for PanicRoot {
            fn drop(&mut self) {
                dispose(&self.0).unwrap();
            }
        }
        let _caught = std::panic::catch_unwind(|| {
            let _root = PanicRoot(root);
            panic!("expected panic after early disposal request");
        });
        panic!("failed terminal append must terminate this child");
    }
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&launch_path).unwrap()).unwrap();
    let original_ledger =
        std::path::PathBuf::from(original["unit_scratch_ledger"].as_str().unwrap());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let owner = new_path(
        std::env::temp_dir().join(format!(
            "retention-ledger-failure-{}-{nonce}",
            std::process::id()
        )),
        true,
    );
    fn snapshot(path: &Path) -> Vec<u8> {
        let _lock = locked_ledger(path).unwrap();
        std::fs::read(path).unwrap()
    }
    let before = snapshot(&original_ledger);
    // Valid records from another root must not invalidate child isolation.
    let other_root = new_path(owner.join("other-ledger-writer"), true);
    let mut isolated = original.clone();
    let ledger = owner.join("child.jsonl");
    isolated["unit_scratch_ledger"] = serde_json::json!(ledger);
    let isolated_launch = owner.join("child-launch.json");
    std::fs::write(
        &isolated_launch,
        serde_json::to_vec_pretty(&isolated).unwrap(),
    )
    .unwrap();
    let child_root = owner.join("child-root");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            std::thread::current().name().unwrap(),
            "--nocapture",
        ])
        .env("RUSTAXA_ERA_LAUNCH_MANIFEST", &isolated_launch)
        .env("RUSTAXA_RETENTION_FAILURE_CHILD_ROOT", &child_root)
        .output()
        .unwrap();
    std::fs::write(owner.join("child-stdout.raw"), &result.stdout).unwrap();
    std::fs::write(owner.join("child-stderr.raw"), &result.stderr).unwrap();
    let after = snapshot(&original_ledger);
    let prefix_preserved = after.starts_with(&before);
    std::fs::write(owner.join("child-status.json"), serde_json::to_vec(&serde_json::json!({"actual_child_exit":result.status.code(),"expected":125,"root":child_root,"isolated_ledger":ledger,"original_ledger_prefix_preserved":prefix_preserved})).unwrap()).unwrap();
    assert_eq!(result.status.code(), Some(125));
    assert!(prefix_preserved);
    let original_records = std::str::from_utf8(&after)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        original_records
            .iter()
            .all(|row| row["root"] != serde_json::json!(child_root))
    );
    for state in ["registered_before_creation", "created"] {
        assert!(
            original_records.iter().any(|row| {
                row["root"] == serde_json::json!(other_root) && row["state"] == state
            })
        );
    }
    assert_eq!(
        std::fs::read(child_root.join("diagnostic")).unwrap(),
        b"must survive failed panic record"
    );
    let saved = std::fs::read_to_string(ledger.with_extension("saved.jsonl")).unwrap();
    let records = saved
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .filter(|row| row["state"] == "disposal_requested")
            .count(),
        1
    );
    assert!(records.iter().all(|row| row["panicking"] == false));
    assert!(ledger.is_dir());
    // No terminal event for owner: preserve the isolated failed child and captures.
}
