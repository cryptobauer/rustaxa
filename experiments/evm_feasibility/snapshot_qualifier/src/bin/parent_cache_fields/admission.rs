//! Bounded fixed artifact admission and duplicate-key rejecting JSON codec.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{env, fs, io::Read, path::Path};
pub(super) fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(super) fn outside_db(path: &Path) -> Result<()> {
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))?;
    ensure!(
        !path.starts_with(repo.join("data"))
            && !path.starts_with(repo.join("local/evm-state-db/snapshot-litenode-copy")),
        "path overlaps database tree"
    );
    Ok(())
}
pub(super) fn evidence(
    path: &Path,
    expected_path: &Path,
    pin: &str,
    size: usize,
    cap: usize,
) -> Result<Vec<u8>> {
    let path = fs::canonicalize(path)?;
    ensure!(
        path == fs::canonicalize(expected_path)?,
        "source path differs"
    );
    outside_db(&path)?;
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() == size as u64 && size <= cap,
        "source size/cap drift"
    );
    let bytes = read_capped(fs::File::open(path)?, cap)?;
    ensure!(
        bytes.len() == size && sha(&bytes) == pin,
        "source byte pin drift"
    );
    Ok(bytes)
}
pub(super) fn read_capped(reader: impl Read, cap: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(cap as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= cap, "growing/oversized input");
    Ok(bytes)
}
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("integer-only JSON with unique keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Bool(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::from(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::from(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::from(v)))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::from(v)))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = Vec::new();
                while let Some(Unique(v)) = a.next_element()? {
                    out.push(v);
                }
                Ok(Unique(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some((k, Unique(v))) = a.next_entry::<String, Unique>()? {
                    if out.insert(k, v).is_some() {
                        return Err(de::Error::custom("duplicate JSON field"));
                    }
                }
                Ok(Unique(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}
pub(super) fn parse_unique(bytes: &[u8]) -> Result<Value> {
    Ok(serde_json::from_slice::<Unique>(bytes)?.0)
}
/// Sync a bounded temporary before atomic exclusive publication; cleanup on errors.
pub(super) fn publish_exclusive(output: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output parent"))?;
    let name = output
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("output name"))?
        .to_string_lossy();
    let pending = parent.join(format!(".{name}.pending-{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(pending.clone());
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::hard_link(&pending, output)?;
    Ok(())
}
