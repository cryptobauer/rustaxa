//! Bounded complete images of task-owned integration databases.
use anyhow::{Result, ensure};
use std::{
    io::{Read, Write},
    path::Path,
};

pub type Rows = Vec<(Vec<u8>, Vec<u8>)>;
#[derive(Debug, PartialEq, Eq)]
pub struct DatabaseImage {
    pub columns: Vec<(String, Rows)>,
    pub wal_files: Vec<(String, Vec<u8>)>,
}
pub(crate) fn collect_rows(
    iterator: impl IntoIterator<Item = std::result::Result<(Box<[u8]>, Box<[u8]>), rocksdb::Error>>,
    mut remaining: usize,
    maximum_rows: usize,
) -> Result<Rows> {
    let mut rows = Vec::new();
    for row in iterator {
        ensure!(rows.len() < maximum_rows, "integration row count limit");
        let (k, v) = row?;
        let size = k
            .len()
            .checked_add(v.len())
            .ok_or_else(|| anyhow::anyhow!("integration image size overflow"))?;
        ensure!(size <= remaining, "integration image row limit");
        remaining -= size;
        rows.push((k.into_vec(), v.into_vec()));
    }
    Ok(rows)
}
pub(crate) fn capture(
    path: &Path,
    read: impl Fn(&str, usize, usize) -> Result<Rows>,
) -> Result<DatabaseImage> {
    let mut names = rocksdb::DB::list_cf(&rocksdb::Options::default(), path)?;
    names.sort();
    ensure!(
        names.len() <= 256 && names.iter().all(|n| n.len() <= 4096),
        "integration column limits"
    );
    let mut used = 0usize;
    let mut columns = Vec::new();
    let mut row_count = 0usize;
    for name in names {
        let rows = read(&name, 64 * 1024 * 1024 - used, 262_144 - row_count)?;
        row_count += rows.len();
        for (key, value) in &rows {
            used = used
                .checked_add(key.len() + value.len())
                .ok_or_else(|| anyhow::anyhow!("integration image size overflow"))?;
        }
        ensure!(used <= 64 * 1024 * 1024, "integration image byte limit");
        columns.push((name, rows));
    }
    let mut paths = std::fs::read_dir(path)?
        .map(|p| p.map(|p| p.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    ensure!(paths.len() <= 1024, "integration file count limit");
    let mut wal_files = Vec::new();
    for file in paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "log"))
    {
        let size = usize::try_from(file.metadata()?.len())?;
        used = used
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("integration image size overflow"))?;
        ensure!(used <= 64 * 1024 * 1024, "integration image byte limit");
        let mut input = std::fs::File::open(&file)?;
        let mut bytes = vec![0; size];
        input.read_exact(&mut bytes)?;
        let mut extra = [0];
        ensure!(
            input.read(&mut extra)? == 0,
            "integration WAL grew during read"
        );
        ensure!(bytes.len() == size, "integration WAL changed during read");
        wal_files.push((
            file.file_name().unwrap().to_string_lossy().into_owned(),
            bytes,
        ));
    }
    Ok(DatabaseImage { columns, wal_files })
}
impl DatabaseImage {
    pub fn save(&self, path: &Path) -> Result<()> {
        fn prefix(n: usize) -> usize {
            if n < 56 {
                1
            } else {
                1 + (usize::BITS - n.leading_zeros()).div_ceil(8) as usize
            }
        }
        fn data(v: &[u8]) -> usize {
            if v.len() == 1 && v[0] < 128 {
                1
            } else {
                prefix(v.len()).saturating_add(v.len())
            }
        }
        fn list(n: usize) -> usize {
            prefix(n).saturating_add(n)
        }
        let columns = self
            .columns
            .iter()
            .map(|(name, rows)| {
                let rows = rows
                    .iter()
                    .map(|(k, v)| list(data(k).saturating_add(data(v))))
                    .fold(0usize, usize::saturating_add);
                list(data(name.as_bytes()).saturating_add(list(rows)))
            })
            .fold(0usize, usize::saturating_add);
        let wals = self
            .wal_files
            .iter()
            .map(|(name, v)| list(data(name.as_bytes()).saturating_add(data(v))))
            .fold(0usize, usize::saturating_add);
        let encoded_size = list(list(columns).saturating_add(list(wals)));
        ensure!(
            encoded_size <= 80 * 1024 * 1024,
            "integration image encoding limit"
        );
        let mut r = rlp::RlpStream::new_list(2);
        r.begin_list(self.columns.len());
        for (name, rows) in &self.columns {
            r.begin_list(2).append(name).begin_list(rows.len());
            for (k, v) in rows {
                r.begin_list(2).append(k).append(v);
            }
        }
        r.begin_list(self.wal_files.len());
        for (n, v) in &self.wal_files {
            r.begin_list(2).append(n).append(v);
        }
        let bytes = r.out();
        ensure!(
            bytes.len() == encoded_size,
            "integration image encoding size mismatch"
        );
        ensure!(
            bytes.len() <= 80 * 1024 * 1024,
            "integration image encoding limit"
        );
        let mut f = std::fs::File::create_new(path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        Ok(())
    }
}
