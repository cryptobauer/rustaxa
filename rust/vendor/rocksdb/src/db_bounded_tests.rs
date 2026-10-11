use super::*;
use crate::column_family::UnboundColumnFamily;
use crate::db_options::OptionsMustOutliveDB;
use crate::{Direction, IteratorMode, DB};

fn descriptors(names: &[&str]) -> Vec<ColumnFamilyDescriptor> {
    names
        .iter()
        .map(|n| ColumnFamilyDescriptor::new(*n, Options::default()))
        .collect()
}

#[test]
fn bounded_missing_directory_never_creates_it_and_validates_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("absent");
    let result = DB::open_cf_descriptors_read_only_bounded(
        &Options::default(),
        &missing,
        descriptors(&["default"]),
        false,
    );
    assert_eq!(result.unwrap_err(), BoundedReadError::Open);
    assert!(!missing.exists());
    for names in [
        &[][..],
        &["1"][..],
        &["default", "default"][..],
        &["default", "bad\0name"][..],
    ] {
        assert_eq!(
            DB::open_cf_descriptors_read_only_bounded(
                &Options::default(),
                &missing,
                descriptors(names),
                false
            )
            .unwrap_err(),
            BoundedReadError::InvalidDescriptors
        );
        assert!(!missing.exists());
    }
    assert_eq!(
        DB::open_cf_descriptors_read_only_bounded(
            &Options::default(),
            "",
            descriptors(&["default"]),
            false
        )
        .unwrap_err(),
        BoundedReadError::InvalidPath
    );
    assert_eq!(
        DB::open_cf_descriptors_read_only_bounded(
            &Options::default(),
            "\0",
            descriptors(&["default"]),
            false
        )
        .unwrap_err(),
        BoundedReadError::InvalidPath
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(&[0xff]));
        assert_eq!(
            DB::open_cf_descriptors_read_only_bounded(
                &Options::default(),
                path,
                descriptors(&["default"]),
                false
            )
            .unwrap_err(),
            BoundedReadError::InvalidPath
        );
    }
}

#[test]
fn bounded_fixed_subset_matches_normal_pins_and_reverse_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    {
        let db = DB::open_cf_descriptors(
            &opts,
            directory.path(),
            descriptors(&["default", "selected", "unopened"]),
        )
        .unwrap();
        db.put(b"descriptor", b"identity").unwrap();
        db.put_cf(db.cf_handle("selected").unwrap(), b"a", b"one")
            .unwrap();
        db.put_cf(db.cf_handle("selected").unwrap(), b"c", b"three")
            .unwrap();
    }
    let db = DB::open_cf_descriptors_read_only_bounded(
        &opts,
        directory.path(),
        descriptors(&["default", "selected"]),
        false,
    )
    .unwrap();
    assert!(db.cf_handle("unopened").is_none());
    let readopts = ReadOptions::default();
    assert_eq!(
        db.get_pinned_bounded_opt(b"descriptor", &readopts)
            .unwrap()
            .unwrap()
            .as_ref(),
        db.get_pinned(b"descriptor").unwrap().unwrap().as_ref()
    );
    let cf = db.cf_handle("selected").unwrap();
    assert_eq!(
        db.get_pinned_cf_bounded_opt(cf, b"a", &readopts)
            .unwrap()
            .unwrap()
            .as_ref(),
        b"one"
    );
    assert!(db
        .get_pinned_cf_bounded_opt(cf, b"b", &readopts)
        .unwrap()
        .is_none());
    let mut raw = db.raw_iterator_cf(cf);
    raw.seek_for_prev(b"b");
    assert!(raw.valid());
    let (key, value) = db
        .iterator_cf(cf, IteratorMode::From(b"b", Direction::Reverse))
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(raw.key().unwrap(), key.as_ref());
    assert_eq!(raw.value().unwrap(), value.as_ref());
    raw.seek_for_prev(b"");
    assert!(!raw.valid());
    assert_eq!(raw.status_bounded(), Ok(()));
    assert_eq!(
        crate::bounded_read::iterator_status_with(|error| {
            assert!(error.is_null());
            *error = unsafe { libc::strdup(c"injected invalid cursor error".as_ptr()) };
            assert!(!error.is_null());
        }),
        Err(BoundedReadError::IteratorStatus)
    );
    drop(raw);
    drop(db);
    // Repeated owner cleanup leaves no locks or resource ownership behind.
    for _ in 0..8 {
        drop(
            DB::open_cf_descriptors_read_only_bounded(
                &opts,
                directory.path(),
                descriptors(&["default", "selected"]),
                false,
            )
            .unwrap(),
        );
    }
    drop(
        DB::open_cf_descriptors(
            &opts,
            directory.path(),
            descriptors(&["default", "selected", "unopened"]),
        )
        .unwrap(),
    );
}

#[test]
fn bounded_required_cf_and_comparator_failures_remain_errors() {
    let directory = tempfile::tempdir().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let comparator = || {
        let mut options = Options::default();
        options.set_comparator("test.required-comparator", Box::new(|a, b| a.cmp(b)));
        ColumnFamilyDescriptor::new("custom", options)
    };
    {
        let db = DB::open_cf_descriptors(
            &opts,
            directory.path(),
            vec![
                ColumnFamilyDescriptor::new("default", Options::default()),
                comparator(),
            ],
        )
        .unwrap();
        db.put_cf(db.cf_handle("custom").unwrap(), b"key", b"value")
            .unwrap();
    }
    for names in [&["default", "missing"][..], &["default", "custom"][..]] {
        assert_eq!(
            DB::open_cf_descriptors_read_only_bounded(
                &opts,
                directory.path(),
                descriptors(names),
                false
            )
            .unwrap_err(),
            BoundedReadError::Open
        );
    }
    let db = DB::open_cf_descriptors_read_only_bounded(
        &opts,
        directory.path(),
        vec![
            ColumnFamilyDescriptor::new("default", Options::default()),
            comparator(),
        ],
        false,
    )
    .unwrap();
    assert_eq!(
        db.get_pinned_cf_bounded_opt(
            db.cf_handle("custom").unwrap(),
            b"key",
            &ReadOptions::default()
        )
        .unwrap()
        .unwrap()
        .as_ref(),
        b"value"
    );
}

#[test]
fn bounded_partial_output_cleanup_is_null_safe_and_cf_first() {
    use std::cell::RefCell;
    let events = RefCell::new(Vec::new());
    let db = 3_usize as *mut ffi::rocksdb_t;
    let handles = [
        1_usize as *mut ffi::rocksdb_column_family_handle_t,
        ptr::null_mut(),
        2_usize as *mut ffi::rocksdb_column_family_handle_t,
    ];
    cleanup_outputs(
        db,
        &handles,
        |p| events.borrow_mut().push(p as usize),
        |p| events.borrow_mut().push(p as usize),
    );
    assert_eq!(*events.borrow(), [1, 2, 3]);
    events.borrow_mut().clear();
    cleanup_outputs(
        ptr::null_mut(),
        &handles,
        |p| events.borrow_mut().push(p as usize),
        |_| panic!("null DB close"),
    );
    assert_eq!(*events.borrow(), [1, 2]);
    cleanup_outputs(
        ptr::null_mut(),
        &[ptr::null_mut()],
        |_| panic!("null CF destroy"),
        |_| panic!("null DB close"),
    );
    assert_eq!(
        unsafe { open_status(ptr::null_mut(), db, &handles) },
        Err(BoundedReadError::NullHandle)
    );
    assert_eq!(
        unsafe { open_status(ptr::null_mut(), ptr::null_mut(), &handles[..1]) },
        Err(BoundedReadError::NullHandle)
    );
    assert_eq!(
        unsafe { open_status(ptr::null_mut(), db, &handles[..1]) },
        Ok(())
    );
    let error = unsafe { libc::strdup(c"open error with partial outputs".as_ptr()) };
    assert_eq!(
        unsafe { open_status(error, db, &handles) },
        Err(BoundedReadError::Open)
    );
}

#[test]
fn bounded_metadata_ledger_and_ordinary_diagnostics_stay_separate() {
    assert!(std::mem::size_of::<ColumnFamilyDescriptor>() <= 256);
    assert!(std::mem::size_of::<OptionsMustOutliveDB>() <= 256);
    assert!(std::mem::size_of::<CString>() <= 32);
    assert!(std::mem::size_of::<UnboundColumnFamily>() + 2 * std::mem::size_of::<usize>() <= 128);
    for path_bytes in [1, 4096] {
        for descriptor_count in [4, 9] {
            assert!(32768 + 3 * path_bytes + 2048 * descriptor_count <= 65536);
        }
    }
    let text = "ordinary full engine context ".repeat(4096);
    let input = CString::new(text.as_str()).unwrap();
    let error = unsafe { libc::strdup(input.as_ptr()) };
    assert!(!error.is_null());
    assert_eq!(crate::ffi_util::error_message(error), text);
}

#[test]
fn bounded_pinned_errors_are_not_missing_and_release_returned_pin() {
    let directory = tempfile::tempdir().unwrap();
    let db = DB::open_default(directory.path()).unwrap();
    db.put(b"key", b"value").unwrap();
    // A real owned pin plus a synthetic engine error tests the same cleanup
    // adapter used by both bounded pinned entry points.
    let mut err = ptr::null_mut();
    let opts = ReadOptions::default();
    let pin = unsafe {
        ffi::rocksdb_get_pinned(
            db.inner.inner(),
            opts.inner,
            b"key".as_ptr().cast(),
            3,
            &mut err,
        )
    };
    assert!(err.is_null());
    assert!(!pin.is_null());
    let error = unsafe { libc::strdup(c"synthetic read failure".as_ptr()) };
    assert_eq!(
        unsafe { pinned_result(pin, error) }.err().unwrap(),
        BoundedReadError::PinnedRead
    );
    assert!(unsafe { pinned_result(ptr::null_mut(), ptr::null_mut()) }
        .unwrap()
        .is_none());
    let error = unsafe { libc::strdup(c"synthetic read failure".as_ptr()) };
    assert_eq!(
        unsafe { pinned_result(ptr::null_mut(), error) }
            .err()
            .unwrap(),
        BoundedReadError::PinnedRead
    );
    let destroys = std::cell::Cell::new(0);
    let error = unsafe { libc::strdup(c"injected error plus pin".as_ptr()) };
    let fake = std::ptr::dangling_mut();
    assert_eq!(
        unsafe {
            pinned_status(fake, error, |p| {
                assert_eq!(p, fake);
                destroys.set(destroys.get() + 1);
            })
        },
        Err(BoundedReadError::PinnedRead)
    );
    assert_eq!(destroys.get(), 1);
    let error = unsafe { libc::strdup(c"injected error without pin".as_ptr()) };
    assert_eq!(
        unsafe { pinned_status(ptr::null_mut(), error, |_| destroys.set(2)) },
        Err(BoundedReadError::PinnedRead)
    );
    assert_eq!(destroys.get(), 1);
}

#[test]
fn bounded_descriptor_and_path_ceilings_apply_before_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("absent");
    let invalid_name = "a".repeat(65);
    assert_eq!(
        DB::open_cf_descriptors_read_only_bounded(
            &Options::default(),
            &path,
            descriptors(&["default", &invalid_name]),
            false
        )
        .unwrap_err(),
        BoundedReadError::InvalidDescriptors
    );
    let long_path = "a".repeat(4097);
    assert_eq!(
        DB::open_cf_descriptors_read_only_bounded(
            &Options::default(),
            &long_path,
            descriptors(&["default"]),
            false
        )
        .unwrap_err(),
        BoundedReadError::InvalidPath
    );
    assert!(!path.exists());
    let accepted_name = "a".repeat(64);
    let accepted_directory = tempfile::tempdir().unwrap();
    let mut accepted_options = Options::default();
    accepted_options.create_if_missing(true);
    accepted_options.create_missing_column_families(true);
    drop(
        DB::open_cf_descriptors(
            &accepted_options,
            accepted_directory.path(),
            descriptors(&["default", &accepted_name]),
        )
        .unwrap(),
    );
    drop(
        DB::open_cf_descriptors_read_only_bounded(
            &accepted_options,
            accepted_directory.path(),
            descriptors(&["default", &accepted_name]),
            false,
        )
        .unwrap(),
    );
    let names = ["default", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    drop(DB::open_cf_descriptors(&opts, directory.path(), descriptors(&names)).unwrap());
    assert_eq!(
        DB::open_cf_descriptors_read_only_bounded(
            &opts,
            directory.path(),
            descriptors(&names),
            false
        )
        .unwrap_err(),
        BoundedReadError::InvalidDescriptors
    );
    drop(
        DB::open_cf_descriptors_read_only_bounded(
            &opts,
            directory.path(),
            descriptors(&names[..9]),
            false,
        )
        .unwrap(),
    );
    drop(
        DBWithThreadMode::<MultiThreaded>::open_cf_descriptors_read_only_bounded(
            &opts,
            directory.path(),
            descriptors(&names[..9]),
            false,
        )
        .unwrap(),
    );
}
