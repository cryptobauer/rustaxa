//! Complete logical snapshots of the public Rust owner, separate from Go rows.

use super::FixturePath;
use rustaxa_storage::Column;

type OwnerRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;

/// Copies every column name and comparator-ordered key/value row from `path/db`.
/// Empty columns are retained. The fresh read-only view replays the live owner's
/// WAL without flushing it; public descriptors preserve numeric comparators.
/// Unknown columns, open failures and iterator errors fail the test immediately.
pub(super) fn owner_rows(path: &FixturePath) -> OwnerRows {
    let path = path.0.join("db");
    let options = rocksdb::Options::default();
    let mut columns = rocksdb::DB::list_cf(&options, &path).unwrap();
    columns.sort();
    let descriptors = columns
        .iter()
        .map(|name| Column::from_name(name).unwrap().descriptor(&options));
    let db =
        rocksdb::DB::open_cf_descriptors_read_only(&options, &path, descriptors, false).unwrap();
    columns
        .into_iter()
        .map(|column| {
            let rows = db
                .iterator_cf(
                    &db.cf_handle(&column).unwrap(),
                    rocksdb::IteratorMode::Start,
                )
                .map(|row| {
                    let (key, value) = row.unwrap();
                    (key.to_vec(), value.to_vec())
                })
                .collect();
            (column, rows)
        })
        .collect()
}

/// Receipt changes must be visible before owner close, even when account state
/// stays fixed. Numeric keys distinguish the public comparator from byte order.
#[test]
fn complete_owner_snapshot_sees_wal_non_account_rows_and_reopen() {
    use rustaxa_storage::{Config, Storage};

    let path = FixturePath::new("owner-snapshot");
    let storage = Storage::new(Config::new(path.0.clone())).unwrap();
    let empty = owner_rows(&path);
    let mut names: Vec<_> = Column::all()
        .iter()
        .map(|column| column.name().to_string())
        .collect();
    names.sort();
    assert_eq!(
        empty
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>(),
        names
    );
    assert!(empty.iter().all(|(_, rows)| rows.is_empty()));

    let put = |column, key: &[u8], value: &[u8]| {
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, column, key, value)
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
    };
    put(Column::DefaultColumn, b"sentinel", b"default row");
    put(
        Column::FinalChainReceiptByPeriod,
        &256u64.to_le_bytes(),
        b"receipt256",
    );
    put(
        Column::FinalChainReceiptByPeriod,
        &1u64.to_le_bytes(),
        b"receipt1",
    );
    let before = owner_rows(&path);
    let receipt_index = before
        .iter()
        .position(|(name, _)| name == Column::FinalChainReceiptByPeriod.name())
        .unwrap();
    assert_eq!(
        before[receipt_index].1,
        vec![
            (1u64.to_le_bytes().to_vec(), b"receipt1".to_vec()),
            (256u64.to_le_bytes().to_vec(), b"receipt256".to_vec()),
        ]
    );
    assert_eq!(
        before.iter().filter(|(_, rows)| rows.is_empty()).count(),
        names.len() - 2
    );
    put(
        Column::FinalChainReceiptByPeriod,
        &1u64.to_le_bytes(),
        b"changed receipt",
    );
    let after = owner_rows(&path);
    assert_ne!(after, before, "a changed non-account row must be detected");
    let mut expected = before;
    expected[receipt_index].1[0].1 = b"changed receipt".to_vec();
    assert_eq!(
        after, expected,
        "all other rows and empty columns stay fixed"
    );
    drop(storage);
    let reopened = Storage::new(Config::new(path.0.clone())).unwrap();
    assert_eq!(owner_rows(&path), after);
    drop(reopened);
}
