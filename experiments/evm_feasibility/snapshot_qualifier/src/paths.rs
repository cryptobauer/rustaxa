//! Persistent snapshot path policy. Validation uses filesystem metadata only;
//! every DB child must remain in the exact independent copy and reports must be
//! new files outside both database trees. No supplied DB is opened here.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Validated canonical paths. Construction rejects source aliases, symlink
/// components, escaping DB children, output overlap and existing output files.
#[derive(Debug)]
pub struct SnapshotPaths {
    pub input: PathBuf,
    pub application: PathBuf,
    pub state: PathBuf,
    pub output: PathBuf,
}

/// Applies the repository's fixed data/local policy before any database open.
pub fn validate(input: &Path, output: &Path) -> Result<SnapshotPaths> {
    validate_at(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .as_path(),
        input,
        output,
    )
}

fn validate_at(repo: &Path, input: &Path, output: &Path) -> Result<SnapshotPaths> {
    let repo = fs::canonicalize(repo)?;
    let supplied = repo.join("data");
    let expected = repo.join("local/evm-state-db/snapshot-litenode-copy");
    no_symlinks(&supplied)?;
    no_symlinks(&expected)?;
    no_symlinks(input)?;
    let input = fs::canonicalize(input)?;
    ensure!(
        input == expected,
        "input must be the exact independent local snapshot copy"
    );
    let mut children = Vec::new();
    for name in ["db/db", "db/state_db"] {
        let child = input.join(name);
        no_symlinks(&child)?;
        let child = fs::canonicalize(child)?;
        ensure!(
            child.starts_with(&input) && !child.starts_with(&supplied),
            "database child escapes copy"
        );
        children.push(child);
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    no_symlinks(parent)?;
    let output =
        fs::canonicalize(parent)?.join(output.file_name().context("report needs filename")?);
    ensure!(
        !output.starts_with(&supplied) && !output.starts_with(&input),
        "report overlaps snapshot tree"
    );
    match fs::symlink_metadata(&output) {
        Ok(_) => anyhow::bail!("report already exists"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(SnapshotPaths {
        input,
        application: children.remove(0),
        state: children.remove(0),
        output,
    })
}

fn no_symlinks(path: &Path) -> Result<()> {
    // Walk the original spelling: canonicalization alone would hide an alias.
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut walked = PathBuf::new();
    for component in absolute.components() {
        walked.push(component);
        let meta = fs::symlink_metadata(&walked)?;
        ensure!(
            !meta.file_type().is_symlink(),
            "symlink component rejected: {}",
            walked.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_copy_and_new_outputs_only() {
        let root = std::env::temp_dir().join(format!("qualifier-policy-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let copy = root.join("local/evm-state-db/snapshot-litenode-copy");
        fs::create_dir_all(copy.join("db/db")).unwrap();
        fs::create_dir_all(copy.join("db/state_db")).unwrap();
        fs::create_dir(root.join("data")).unwrap();
        let output = root.join("report.json");
        assert!(validate_at(&root, &copy, &output).is_ok());
        assert!(validate_at(&root, &root.join("data"), &output).is_err());
        assert!(validate_at(&root, &copy, &copy.join("out")).is_err());
        assert!(validate_at(&root, &copy, &root.join("data/out")).is_err());
        fs::write(&output, b"preserved").unwrap();
        assert!(validate_at(&root, &copy, &output).is_err());
        fs::remove_file(&output).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let alias = root.join("alias");
            symlink(&copy, &alias).unwrap();
            assert!(validate_at(&root, &alias, &output).is_err());
            let out_alias = root.join("out_alias");
            symlink(root.join("data"), &out_alias).unwrap();
            assert!(validate_at(&root, &copy, &out_alias.join("out")).is_err());
            fs::remove_dir(copy.join("db/state_db")).unwrap();
            symlink(root.join("data"), copy.join("db/state_db")).unwrap();
            assert!(validate_at(&root, &copy, &output).is_err());
            fs::remove_file(copy.join("db/state_db")).unwrap();
            fs::create_dir(copy.join("db/state_db")).unwrap();
            fs::remove_dir_all(&copy).unwrap();
            symlink(root.join("data"), &copy).unwrap();
            assert!(validate_at(&root, &copy, &output).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
