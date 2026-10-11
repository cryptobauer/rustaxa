use super::*;
use rlp::RlpStream;
use rustaxa_types::{StoredFinalChainBlockHeader, codec::rlp::final_chain::StoredBlockHeaderRlp};
use std::sync::atomic::{AtomicU64, Ordering};

fn identity(period: u64, root: [u8; 32]) -> ConcreteStateIdentity {
    ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(period),
        state_root: root,
    }
}
fn header(root: [u8; 32]) -> Vec<u8> {
    let mut s = RlpStream::new_list(7);
    for hash in [[1; 32], root, [2; 32], [3; 32]] {
        s.append(&hash.as_slice());
    }
    s.append(&[0u8; 256].as_slice());
    s.append(&123u64);
    s.append(&456u64);
    s.out().to_vec()
}
fn plan(root: [u8; 32]) -> ConcreteDiagnosticPlan {
    let delayed = std::array::from_fn(|i| identity(FIRST_DELAYED + i as u64, root));
    let tuples = std::array::from_fn(|i| {
        let mut address = [0; 20];
        address[19] = if i < 48 { 0xfe } else { 0xee };
        let key = if i < 48 {
            ConcreteStorageKey(codec::keccak256(&[4]))
        } else {
            let mut key = [0; 32];
            key[31] = 2;
            ConcreteStorageKey(key)
        };
        ConcreteDiagnosticTuple {
            identity: delayed[i % 48],
            address,
            key,
        }
    });
    ConcreteDiagnosticPlan {
        genesis_hash: [9; 32],
        head_header_sha256: Sha256::digest(header(root)).into(),
        head: identity(HEAD, root),
        parent: identity(PARENT, root),
        delayed,
        tuples,
    }
}
fn raw_descriptor(id: ConcreteStateIdentity) -> Vec<u8> {
    let mut s = RlpStream::new_list(2);
    s.append(&id.period.as_u64());
    s.append(&id.state_root.as_slice());
    s.out().to_vec()
}
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    application: std::path::PathBuf,
    state: std::path::PathBuf,
    base: std::path::PathBuf,
    plan: ConcreteDiagnosticPlan,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}
impl Fixture {
    fn empty() -> Self {
        let base = std::path::PathBuf::from("/home/fry/artifacts/n4-binding-resume-20261011").join(
            format!(
                "synthetic-storage-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
        );
        std::fs::create_dir_all(&base).unwrap();
        let application = base.join("app");
        let state = base.join("state");
        let plan = plan(codec::empty_trie_root());
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        {
            let columns = APPLICATION_COLUMNS
                .into_iter()
                .map(|c| c.descriptor(&options));
            let db = Owner::open_cf_descriptors(&options, &application, columns).unwrap();
            for (column, key, value) in [
                (
                    Column::Genesis,
                    crate::SINGLE_VALUE_KEY.to_vec(),
                    plan.genesis_hash.to_vec(),
                ),
                (
                    Column::FinalChainMeta,
                    1u32.to_le_bytes().to_vec(),
                    HEAD.to_le_bytes().to_vec(),
                ),
                (
                    Column::FinalChainBlkByNumber,
                    HEAD.to_le_bytes().to_vec(),
                    header(plan.head.state_root),
                ),
                (
                    Column::FinalChainBlkByNumber,
                    PARENT.to_le_bytes().to_vec(),
                    header(plan.parent.state_root),
                ),
            ] {
                db.put_cf(&db.cf_handle(column.name()).unwrap(), key, value)
                    .unwrap();
            }
            db.put_cf(
                &db.cf_handle("final_chain_meta").unwrap(),
                0u32.to_le_bytes(),
                7u64.to_le_bytes(),
            )
            .unwrap();
            // The diagnostic opens the fixed subset despite an unrelated CF.
            db.create_cf("unrelated_inventory_forbidden", &Options::default())
                .unwrap();
        }
        {
            let db = Owner::open_cf_descriptors(
                &options,
                &state,
                CONCRETE_COLUMNS.map(|c| ColumnFamilyDescriptor::new(c, Options::default())),
            )
            .unwrap();
            db.put(DESCRIPTOR_KEY, raw_descriptor(plan.head)).unwrap();
        }
        Self {
            application,
            state,
            base,
            plan,
        }
    }
}
#[test]
fn complete_empty_root_family_independently_replays_fixed_subset() {
    let f = Fixture::empty();
    let capture = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert_eq!(capture.usage.used(R::Registrations), 51);
    assert_eq!(capture.usage.used(R::DistinctIdentities), 50);
    assert_eq!(capture.usage.used(R::PayloadTuples), 96);
    let replay =
        replay_diagnostic_bundle(&capture.bundle, &f.plan, ConcreteWitnessLimits::default())
            .unwrap();
    assert!(
        replay
            .logical
            .iter()
            .all(|x| *x == ConcreteWitnessLogical::NonMember)
    );
    assert_eq!(replay.usage.used(R::Registrations), 51);
    assert_eq!(replay.usage.used(R::PayloadTuples), 96);
    assert_eq!(
        replay.usage.admitted_input_bytes,
        capture.bundle.len() as u64
    );
    assert!(capture.usage.used(R::Operations) > replay.usage.used(R::Operations));
}
#[test]
fn pure_header_validator_matches_decoder_and_rejects_noncanonical_fields() {
    let bytes = header([7; 32]);
    let existing =
        StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&bytes)).unwrap();
    assert_eq!(header_root(&bytes).unwrap(), existing.state_root.0);
    for gas in [vec![0], vec![0, 1], vec![1; 9]] {
        let mut s = RlpStream::new_list(7);
        for _ in 0..4 {
            s.append(&[1u8; 32].as_slice());
        }
        s.append(&[0u8; 256].as_slice());
        s.append(&gas);
        s.append(&1u64);
        assert_eq!(header_root(&s.out()), Err(ConcreteWitnessError::Schema));
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(header_root(&trailing), Err(ConcreteWitnessError::Schema));
    assert_eq!(item(&[0x81, 1]), Err(ConcreteWitnessError::Schema));
    assert_eq!(item(&[0xb8, 1, 0]), Err(ConcreteWitnessError::Schema));
}
#[test]
fn invalid_manifest_and_shared_open_limits_stop_before_reads() {
    let mut p = plan(codec::empty_trie_root());
    p.tuples.swap(0, 1);
    let failure = capture_diagnostic_run(
        Path::new("/must-not-open"),
        Path::new("/must-not-open"),
        &p,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(failure.phase, ConcreteDiagnosticPhase::Plan);
    assert_eq!(failure.usage.used(R::Operations), 0);
    let f = Fixture::empty();
    let limits = ConcreteWitnessLimits::default()
        .lowered(R::Operations, 0)
        .unwrap();
    let e = capture_diagnostic_run(&f.application, &f.state, &f.plan, limits)
        .err()
        .unwrap();
    assert_eq!(e.phase, ConcreteDiagnosticPhase::ApplicationOpen);
    assert_eq!(e.usage.used(R::Operations), 0);
    let limits = ConcreteWitnessLimits::default()
        .lowered(R::CanonicalBytes, 0)
        .unwrap();
    let e = capture_diagnostic_run(&f.application, &f.state, &f.plan, limits)
        .err()
        .unwrap();
    assert_eq!(e.usage.used(R::Operations), 0);
    assert_eq!(e.usage.used(R::RawCopyBytes), 0);
}
#[test]
fn envelope_descriptor_constructor_order_and_record_limits_are_rejected() {
    let f = Fixture::empty();
    let capture = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(capture.bundle.len()).unwrap();
    let admitted = bundle::admit(&capture.bundle, &budget).unwrap();
    let pairs = admitted.pair.map(|x| x.to_vec());
    let mut descriptors = admitted.descriptors.map(|x| x.to_vec());
    let mut proofs: Vec<_> = admitted.proofs.iter().map(|x| x.to_vec()).collect();
    descriptors[1] = raw_descriptor(f.plan.parent);
    let altered = bundle::encode(
        &pairs,
        &descriptors,
        &proofs,
        &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
    )
    .unwrap();
    let e = replay_diagnostic_bundle(&altered, &f.plan, ConcreteWitnessLimits::default())
        .err()
        .unwrap();
    assert_eq!(e.phase, ConcreteDiagnosticPhase::SecondDescriptor);
    assert_eq!(e.error, ConcreteWitnessError::Identity);
    descriptors = admitted.descriptors.map(|x| x.to_vec());
    proofs.swap(0, 1);
    let altered = bundle::encode(
        &pairs,
        &descriptors,
        &proofs,
        &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
    )
    .unwrap();
    assert_eq!(
        replay_diagnostic_bundle(&altered, &f.plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap()
            .error,
        ConcreteWitnessError::Identity
    );
    for resource in [
        R::Records,
        R::FrameInspections,
        R::EncodedBytes,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let limits = ConcreteWitnessLimits::default()
            .lowered(resource, 0)
            .unwrap();
        assert!(replay_diagnostic_bundle(&capture.bundle, &f.plan, limits).is_err());
    }
}
#[test]
fn typed_only_point_backend_preserves_missing_empty_error_and_keys() {
    struct Typed {
        value: Option<Vec<u8>>,
        error: bool,
    }
    impl DbPointReader for Typed {
        type Slice<'a> = &'a [u8];
        type Error = u8;
        fn point_get<'a>(
            &'a self,
            column: Column,
            key: &[u8],
        ) -> Result<Option<Self::Slice<'a>>, u8> {
            let expected = match column {
                Column::Genesis => crate::SINGLE_VALUE_KEY.to_vec(),
                Column::FinalChainMeta => 0x01020304u32.to_le_bytes().to_vec(),
                Column::FinalChainBlkByNumber => 0x0102030405060708u64.to_le_bytes().to_vec(),
                _ => panic!("unexpected column"),
            };
            assert_eq!(key, expected);
            if self.error {
                Err(7)
            } else {
                Ok(self.value.as_deref())
            }
        }
    }
    for value in [None, Some(vec![]), Some(vec![1, 2])] {
        let db = Arc::new(Typed {
            value,
            error: false,
        });
        let metadata = MetadataRepository::new(Arc::clone(&db));
        let final_chain = FinalChainRepository::new(Arc::clone(&db));
        assert_eq!(
            metadata.genesis_hash_view().unwrap(),
            db.value.as_deref().filter(|x| !x.is_empty())
        );
        assert_eq!(
            final_chain.meta_value_view(0x01020304).unwrap(),
            db.value.as_deref()
        );
        assert_eq!(
            final_chain
                .block_header_raw_view(0x0102030405060708)
                .unwrap(),
            db.value.as_deref()
        );
    }
    let db = Arc::new(Typed {
        value: None,
        error: true,
    });
    assert_eq!(
        MetadataRepository::new(Arc::clone(&db)).genesis_hash_view(),
        Err(7)
    );
    assert_eq!(
        FinalChainRepository::new(db).meta_value_view(0x01020304),
        Err(7)
    );
}
#[test]
fn missing_and_oversized_pair_rows_fail_without_next_operation_or_copy() {
    let f = Fixture::empty();
    let mut options = Options::default();
    options.create_if_missing(false);
    let db = Owner::open_cf(
        &options,
        &f.application,
        [
            "genesis",
            "final_chain_meta",
            "final_chain_blk_by_number",
            "unrelated_inventory_forbidden",
        ],
    )
    .unwrap();
    db.delete_cf(&db.cf_handle("genesis").unwrap(), crate::SINGLE_VALUE_KEY)
        .unwrap();
    drop(db);
    let e = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.phase, ConcreteDiagnosticPhase::Pair);
    assert_eq!(e.index, 0);
    assert_eq!(e.usage.used(R::Operations), 2);
    assert_eq!(e.usage.used(R::RawCopyBytes), 0);
    assert_eq!(e.last_read.unwrap().outcome, ConcreteDiagnosticOutcome::End);
    let db = Owner::open_cf(
        &options,
        &f.application,
        [
            "genesis",
            "final_chain_meta",
            "final_chain_blk_by_number",
            "unrelated_inventory_forbidden",
        ],
    )
    .unwrap();
    db.put_cf(
        &db.cf_handle("genesis").unwrap(),
        crate::SINGLE_VALUE_KEY,
        vec![0; 1_048_577],
    )
    .unwrap();
    drop(db);
    let e = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert!(matches!(
        e.error,
        ConcreteWitnessError::LimitExceeded {
            resource: R::RowBytes,
            ..
        }
    ));
    assert_eq!(e.usage.used(R::Operations), 2);
    assert_eq!(e.usage.used(R::RawCopyBytes), 0);
    assert_eq!(
        e.last_read.unwrap().outcome,
        ConcreteDiagnosticOutcome::Node { bytes: 1_048_577 }
    );
}

fn compact(nibbles: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let odd = nibbles.len() % 2 == 1;
    result.push(if odd { 0x30 | nibbles[0] } else { 0x20 });
    for pair in nibbles[usize::from(odd)..].as_chunks::<2>().0 {
        result.push(pair[0] * 16 + pair[1]);
    }
    result
}
fn leaf(key: [u8; 32], commitment: &[u8], depth: usize) -> (Vec<u8>, Vec<u8>) {
    let nibbles: Vec<_> = key.into_iter().flat_map(|b| [b >> 4, b & 15]).collect();
    let compact = compact(&nibbles[depth..]);
    let mut canonical = RlpStream::new_list(2);
    canonical.append(&compact);
    canonical.append(&commitment);
    let canonical = canonical.out().to_vec();
    let hash = codec::keccak256(&canonical);
    let mut physical = RlpStream::new_list(2);
    physical.append(&compact);
    physical.append(&hash.as_slice());
    (canonical, physical.out().to_vec())
}
fn account_trie(
    entries: &[([u8; 32], Vec<u8>)],
    depth: usize,
    nodes: &mut Vec<([u8; 32], Vec<u8>)>,
) -> [u8; 32] {
    if entries.len() == 1 {
        let (canonical, physical) = leaf(entries[0].0, &entries[0].1, depth);
        let hash = codec::keccak256(&canonical);
        nodes.push((hash, physical));
        return hash;
    }
    let mut canonical = RlpStream::new_list(17);
    let mut physical = RlpStream::new_list(16);
    for nibble in 0..16 {
        let children: Vec<_> = entries
            .iter()
            .filter(|(path, _)| {
                let b = path[depth / 2];
                let n = if depth.is_multiple_of(2) {
                    b >> 4
                } else {
                    b & 15
                };
                n == nibble
            })
            .cloned()
            .collect();
        if children.is_empty() {
            canonical.append_empty_data();
            physical.append_empty_data();
        } else {
            let hash = account_trie(&children, depth + 1, nodes);
            canonical.append(&hash.as_slice());
            physical.append(&hash.as_slice());
        }
    }
    canonical.append_empty_data();
    let hash = codec::keccak256(&canonical.out());
    nodes.push((hash, physical.out().to_vec()));
    hash
}
impl Fixture {
    fn members() -> Self {
        let mut f = Self::empty();
        let mut entries = Vec::new();
        let mut nodes = Vec::new();
        let mut rows = Vec::new();
        for (index, raw_value) in [(0, vec![0x01]), (48, vec![0xc0])] {
            let tuple = f.plan.tuples[index];
            let path = codec::storage_trie_path(tuple.key);
            let commitment = rlp::encode(&raw_value).to_vec();
            let (canonical, physical) = leaf(path, &commitment, 0);
            let storage_root = codec::keccak256(&canonical);
            nodes.push((4, storage_root, physical));
            rows.push((
                5,
                codec::versioned_key(
                    codec::storage_version_prefix(tuple.address, tuple.key),
                    FinalChainBlockNumber::new(FIRST_DELAYED),
                ),
                raw_value,
            ));
            let mut raw = RlpStream::new_list(5);
            raw.append_empty_data();
            raw.append_empty_data();
            raw.append(&storage_root.as_slice());
            raw.append_empty_data();
            raw.append(&0u64);
            let raw = raw.out().to_vec();
            let prefix = codec::account_version_prefix(tuple.address);
            let commitment =
                codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap())
                    .unwrap();
            entries.push((prefix, commitment));
            rows.push((
                3,
                codec::versioned_key(prefix, FinalChainBlockNumber::new(FIRST_DELAYED)),
                raw,
            ));
        }
        let mut account_nodes = Vec::new();
        let root = account_trie(&entries, 0, &mut account_nodes);
        nodes.extend(account_nodes.into_iter().map(|(h, n)| (2, h, n)));
        f.plan = plan(root);
        let options = Options::default();
        {
            let db = Owner::open_cf(&options, &f.state, CONCRETE_COLUMNS).unwrap();
            db.put(DESCRIPTOR_KEY, raw_descriptor(f.plan.head)).unwrap();
            for (column, hash, node) in nodes {
                db.put_cf(&db.cf_handle(&column.to_string()).unwrap(), hash, node)
                    .unwrap();
            }
            for (column, key, value) in rows {
                db.put_cf(&db.cf_handle(&column.to_string()).unwrap(), key, value)
                    .unwrap();
            }
        }
        {
            let db = Owner::open_cf(
                &options,
                &f.application,
                [
                    "genesis",
                    "final_chain_meta",
                    "final_chain_blk_by_number",
                    "unrelated_inventory_forbidden",
                ],
            )
            .unwrap();
            for id in [f.plan.head, f.plan.parent] {
                db.put_cf(
                    &db.cf_handle("final_chain_blk_by_number").unwrap(),
                    id.period.as_u64().to_le_bytes(),
                    header(root),
                )
                .unwrap();
            }
        }
        f
    }
}
#[test]
fn authenticated_native_members_replay_exact_selected_rows_and_shared_copies() {
    let f = Fixture::members();
    let captured = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let replay =
        replay_diagnostic_bundle(&captured.bundle, &f.plan, ConcreteWitnessLimits::default())
            .unwrap();
    for (index, logical) in replay.logical.iter().enumerate() {
        assert_eq!(
            *logical,
            ConcreteWitnessLogical::Member(if index < 48 { vec![1] } else { vec![0xc0] })
        );
    }
    // Ordinary-reader parity is outside the diagnostic budget. The same fixed
    // synthetic subset must expose the exact authenticated first/last D values.
    let ordinary = crate::ConcreteCheckpointReaders::open_read_only(
        &f.state,
        f.plan.head,
        [f.plan.delayed[0], f.plan.delayed[47], f.plan.head],
    )
    .unwrap();
    for index in [0, 47, 48, 95] {
        let tuple = f.plan.tuples[index];
        let rustaxa_types::concrete_state::ConcreteRead::Present(raw) = ordinary
            .storage_at(tuple.identity, tuple.address, tuple.key)
            .unwrap()
        else {
            panic!("expected ordinary physical Member");
        };
        let crate::ConcreteStoragePath::Member(proved) = ordinary
            .verify_storage_path_at(tuple.identity, tuple.address, tuple.key)
            .unwrap()
        else {
            panic!("expected ordinary authenticated Member");
        };
        assert_eq!(raw, proved);
        assert_eq!(
            replay.logical[index],
            ConcreteWitnessLogical::Member(proved)
        );
    }
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(captured.bundle.len()).unwrap();
    let envelope = bundle::admit(&captured.bundle, &budget).unwrap();
    let proof = envelope.proofs[51];
    let (frame, _) = binary::frame_at(proof, binary::HEADER_BYTES, &budget).unwrap();
    let binary::Outcome::Row(key, value) = frame.outcome else {
        panic!("expected selected row")
    };
    assert_eq!(
        key,
        codec::versioned_key(
            codec::storage_version_prefix(f.plan.tuples[0].address, f.plan.tuples[0].key),
            FinalChainBlockNumber::new(FIRST_DELAYED)
        )
    );
    assert_eq!(value, &[1]);
    for resource in [R::RawCopyBytes, R::CanonicalBytes, R::Visits] {
        let limit = replay.usage.used(resource);
        assert!(limit > 0);
        let limits = ConcreteWitnessLimits::default()
            .lowered(resource, limit - 1)
            .unwrap();
        assert!(replay_diagnostic_bundle(&captured.bundle, &f.plan, limits).is_err());
    }
}
#[test]
fn shared_registration_payload_and_encoder_failure_keep_first_context() {
    let f = Fixture::members();
    for (resource, limit, phase) in [
        (
            R::Registrations,
            2,
            ConcreteDiagnosticPhase::SecondConstructor,
        ),
        (R::PayloadTuples, 0, ConcreteDiagnosticPhase::Payload),
        (
            R::FrameInspections,
            0,
            ConcreteDiagnosticPhase::FirstConstructor,
        ),
    ] {
        let limits = ConcreteWitnessLimits::default()
            .lowered(resource, limit)
            .unwrap();
        let e = capture_diagnostic_run(&f.application, &f.state, &f.plan, limits)
            .err()
            .unwrap();
        assert_eq!(e.phase, phase);
        assert!(
            matches!(e.error,ConcreteWitnessError::LimitExceeded {resource:r,..} if r==resource)
        );
        if resource == R::FrameInspections {
            assert_eq!(e.usage.used(R::EncodedBytes), 0);
            assert!(e.last_read.is_some());
        }
    }
    let run = Run::new(
        ConcreteWitnessLimits::default()
            .lowered(R::Operations, 0)
            .unwrap(),
    );
    assert!(run.operation().is_err());
    let before = run.budget.usage();
    assert!(run.operation().is_err());
    assert_eq!(run.budget.usage(), before);
}
#[test]
fn descriptor_schema_and_missing_proof_dependency_stop_the_family() {
    let f = Fixture::members();
    let options = Options::default();
    let db = Owner::open_cf(&options, &f.state, CONCRETE_COLUMNS).unwrap();
    db.delete_cf(&db.cf_handle("2").unwrap(), f.plan.head.state_root)
        .unwrap();
    drop(db);
    let e = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.phase, ConcreteDiagnosticPhase::FirstConstructor);
    assert_eq!(e.index, 0);
    assert_eq!(e.error, ConcreteWitnessError::Proof);
    assert_eq!(e.last_read.unwrap().outcome, ConcreteDiagnosticOutcome::End);
    assert_eq!(e.usage.used(R::PayloadTuples), 0);
    for bytes in [vec![0xc0], vec![0x80], vec![0xc2, 0x00, 0x80]] {
        assert!(descriptor(&bytes, f.plan.head).is_err());
    }
}
#[test]
fn ledger_size_count_and_metadata_bound_are_fixed() {
    assert_eq!(APPLICATION_COLUMNS.len(), 4);
    assert_eq!(CONCRETE_COLUMNS.len(), 9);
    assert!(size_of::<ColumnFamilyDescriptor>() <= 256);
    assert!(size_of::<ReadOptions>() <= 256);
    assert_eq!(32768 + 3 * 4096 + 2048 * 9, 63488);
    assert_eq!(32768 + 3 * 4096 + 2048 * 4, 53248);
    for column in APPLICATION_COLUMNS {
        assert!(!column.uses_uint64_comparator());
        assert!(column.name().len() <= 64);
    }
}

#[test]
fn cursor_width_admission_precedes_both_copies_and_malformed_pair_stops_early() {
    for (key, value, resource) in [
        (vec![0; 65], vec![1], R::KeyBytes),
        (vec![0; 40], vec![1; 1_048_577], R::RowBytes),
    ] {
        let f = Fixture::empty();
        let db = Owner::open_cf(&Options::default(), &f.state, CONCRETE_COLUMNS).unwrap();
        db.put_cf(&db.cf_handle("3").unwrap(), &key, &value)
            .unwrap();
        drop(db);
        let e = capture_diagnostic_run(
            &f.application,
            &f.state,
            &f.plan,
            ConcreteWitnessLimits::default(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(e.error,ConcreteWitnessError::LimitExceeded {resource:r,..} if r==resource)
        );
        assert_eq!(e.phase, ConcreteDiagnosticPhase::FirstConstructor);
        assert_eq!(e.usage.used(R::Operations), 10);
        assert_eq!(
            e.last_read.unwrap().outcome,
            ConcreteDiagnosticOutcome::Row {
                key_bytes: key.len() as u64,
                value_bytes: value.len() as u64
            }
        );
        let expected_raw = 32
            + 8
            + header(f.plan.head.state_root).len()
            + header(f.plan.parent.state_root).len()
            + raw_descriptor(f.plan.head).len();
        assert_eq!(e.usage.used(R::RawCopyBytes), expected_raw as u64);
    }
    let f = Fixture::empty();
    let db = Owner::open_cf(
        &Options::default(),
        &f.application,
        [
            "genesis",
            "final_chain_meta",
            "final_chain_blk_by_number",
            "unrelated_inventory_forbidden",
        ],
    )
    .unwrap();
    db.put_cf(
        &db.cf_handle("final_chain_blk_by_number").unwrap(),
        HEAD.to_le_bytes(),
        [0xc0],
    )
    .unwrap();
    drop(db);
    let e = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.error, ConcreteWitnessError::Schema);
    assert_eq!(e.phase, ConcreteDiagnosticPhase::Pair);
    assert_eq!(e.index, 2);
    assert_eq!(e.usage.used(R::Operations), 4);
}
#[test]
fn explicit_backend_error_is_sticky_and_does_not_become_missing() {
    let run = Run::new(ConcreteWitnessLimits::default());
    run.phase(ConcreteDiagnosticPhase::Pair, 0, None, None);
    run.read(0, 1, [0; 32]);
    let error = pair_read::<&[u8]>(&run, || Err(BoundedReadError::PinnedRead)).unwrap_err();
    assert_eq!(error, ConcreteWitnessError::Read(0));
    assert_eq!(
        run.last.get().unwrap().outcome,
        ConcreteDiagnosticOutcome::Error(0)
    );
    let before = run.budget.usage();
    assert_eq!(run.operation(), Err(error));
    assert_eq!(run.budget.usage(), before);
}
#[test]
fn same_identity_cross_transcript_conflict_and_explicit_error_are_rejected() {
    let f = Fixture::members();
    let capture = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(capture.bundle.len()).unwrap();
    let envelope = bundle::admit(&capture.bundle, &budget).unwrap();
    let pairs = envelope.pair.map(|x| x.to_vec());
    let descriptors = envelope.descriptors.map(|x| x.to_vec());
    let mut proofs: Vec<_> = envelope.proofs.iter().map(|x| x.to_vec()).collect();
    let original = envelope.proofs[50];
    let header = binary::admit_transcript(
        original,
        &budget,
        Some((f.plan.head, [0; 20], ConcreteStorageKey([0; 32]))),
        true,
    )
    .unwrap();
    let mut frames = Vec::new();
    let mut offset = binary::HEADER_BYTES;
    while offset < original.len() {
        let (frame, next) = binary::frame_at(original, offset, &budget).unwrap();
        frames.push(frame);
        offset = next;
    }
    assert!(matches!(frames[0].outcome, binary::Outcome::End));
    frames[0].outcome = binary::Outcome::Row(&[0; 40], &[9]);
    let encode_budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    encode_budget
        .charge(R::Records, frames.len() as u64)
        .unwrap();
    proofs[50] = binary::encode_transcript(header, &frames, &encode_budget, 0, true).unwrap();
    let altered = bundle::encode(
        &pairs,
        &descriptors,
        &proofs,
        &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
    )
    .unwrap();
    let e = replay_diagnostic_bundle(&altered, &f.plan, ConcreteWitnessLimits::default())
        .err()
        .unwrap();
    assert_eq!(e.error, ConcreteWitnessError::Conflict);
    assert_eq!(e.phase, ConcreteDiagnosticPhase::SecondConstructor);
    assert_eq!(e.index, 48);
    // An explicit backend failure in the first transcript is a read error,
    // whereas a missing required dependency is the separate proof failure.
    let original = envelope.proofs[0];
    let header = binary::admit_transcript(
        original,
        &budget,
        Some((f.plan.parent, [0; 20], ConcreteStorageKey([0; 32]))),
        true,
    )
    .unwrap();
    let mut frames = Vec::new();
    let mut offset = binary::HEADER_BYTES;
    while offset < original.len() {
        let (frame, next) = binary::frame_at(original, offset, &budget).unwrap();
        frames.push(frame);
        offset = next;
    }
    frames[0].outcome = binary::Outcome::Error(0);
    let encode_budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    encode_budget
        .charge(R::Records, frames.len() as u64)
        .unwrap();
    proofs[0] = binary::encode_transcript(header, &frames, &encode_budget, 0, true).unwrap();
    let altered = bundle::encode(
        &pairs,
        &descriptors,
        &proofs,
        &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
    )
    .unwrap();
    let e = replay_diagnostic_bundle(&altered, &f.plan, ConcreteWitnessLimits::default())
        .err()
        .unwrap();
    assert_eq!(e.error, ConcreteWitnessError::Read(0));
    assert_eq!(
        e.last_read.unwrap().outcome,
        ConcreteDiagnosticOutcome::Error(0)
    );
}
