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
fn vrf_plan(context: ConcreteDiagnosticPlan) -> ConcreteVrfPlan {
    ConcreteVrfPlan::new(context, std::array::from_fn(|i| [i as u8 + 1; 20])).unwrap()
}

#[test]
fn vrf_complete_nonmember_family_replays_and_rejects_profile_confusion() {
    let f = Fixture::empty();
    let plan = vrf_plan(plan(codec::empty_trie_root()));
    let capture = capture_vrf_run(
        &f.application,
        &f.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert_eq!(&capture.bundle[..4], b"RCV1");
    assert_eq!(capture.usage.used(R::PayloadTuples), 20);
    assert_eq!(capture.usage.used(R::Registrations), 51);
    assert_eq!(capture.usage.used(R::DistinctIdentities), 50);
    let replay =
        replay_vrf_bundle(&capture.bundle, &plan, ConcreteWitnessLimits::default()).unwrap();
    assert!(
        replay
            .logical
            .iter()
            .all(|v| *v == ConcreteWitnessLogical::NonMember)
    );
    assert!(
        replay
            .physical
            .iter()
            .all(|v| matches!(v, ConcreteWitnessPhysical::HistoryUnavailable(_)))
    );
    assert!(
        replay_diagnostic_bundle(&capture.bundle, &f.plan, ConcreteWitnessLimits::default())
            .is_err()
    );
    let native = capture_diagnostic_run(
        &f.application,
        &f.state,
        &f.plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert!(replay_vrf_bundle(&native.bundle, &plan, ConcreteWitnessLimits::default()).is_err());
    for offset in [0, 4, 8, 10, 12, 14] {
        let mut wrong = capture.bundle.clone();
        wrong[offset] ^= 1;
        assert!(replay_vrf_bundle(&wrong, &plan, ConcreteWitnessLimits::default()).is_err());
    }
    let mut extra = capture.bundle.clone();
    extra.push(0);
    assert!(replay_vrf_bundle(&extra, &plan, ConcreteWitnessLimits::default()).is_err());
}

#[test]
fn vrf_key_order_and_context_fail_before_any_open() {
    let voters = std::array::from_fn(|i| [i as u8 + 1; 20]);
    let p = vrf_plan(plan(codec::empty_trie_root()));
    for (i, t) in p.tuples().iter().enumerate() {
        let mut raw = [0; 22];
        raw[1] = 4;
        raw[2..].copy_from_slice(&voters[i]);
        assert_eq!(t.key.0, codec::keccak256(&raw));
        assert_eq!(t.identity, p.context.delayed[0]);
        assert_eq!(
            t.address,
            [
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe
            ]
        );
    }
    let mut reordered = voters;
    reordered.swap(0, 1);
    assert!(ConcreteVrfPlan::new(plan(codec::empty_trie_root()), reordered).is_err());
    let mut duplicate = voters;
    duplicate[1] = duplicate[0];
    assert!(ConcreteVrfPlan::new(plan(codec::empty_trie_root()), duplicate).is_err());
    let mut wrong = plan(codec::empty_trie_root());
    wrong.delayed[0].period = (FIRST_DELAYED + 1).into();
    assert!(ConcreteVrfPlan::new(wrong, voters).is_err());
}

#[test]
fn vrf_history_error_never_becomes_proven_nonmembership_and_keeps_first_failure() {
    let f = Fixture::empty();
    let plan = vrf_plan(plan(codec::empty_trie_root()));
    let captured = capture_vrf_run(
        &f.application,
        &f.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(captured.bundle.len()).unwrap();
    let env =
        bundle::admit_profile::<71>(&captured.bundle, &budget, bundle::Profile::FirstVrf).unwrap();
    let mut proofs: Vec<_> = env.proofs.iter().map(|p| p.to_vec()).collect();
    let header = binary::admit_transcript(
        &proofs[51],
        &budget,
        Some((
            plan.tuples[0].identity,
            plan.tuples[0].address,
            plan.tuples[0].key,
        )),
        false,
    )
    .unwrap();
    let mut offset = binary::HEADER_BYTES;
    let mut frames = Vec::new();
    while offset < proofs[51].len() {
        let (frame, next) = binary::frame_at(&proofs[51], offset, &budget).unwrap();
        frames.push(frame);
        offset = next;
    }
    assert_eq!(frames[0].operation, 1);
    assert_eq!(frames[0].column, 5);
    frames[0].outcome = binary::Outcome::Error(3);
    let encoding = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    encoding.charge(R::Records, frames.len() as u64).unwrap();
    let altered = binary::encode_transcript(header, &frames, &encoding, 0, false).unwrap();
    proofs[51] = altered;
    let bytes = bundle::encode_profile(
        &env.pair.map(|p| p.to_vec()),
        &env.descriptors.map(|p| p.to_vec()),
        &proofs,
        &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
        bundle::Profile::FirstVrf,
    )
    .unwrap();
    let failure = replay_vrf_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
        .err()
        .unwrap();
    assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
    assert_eq!(failure.index, 0);
    assert_eq!(
        failure.last_read.unwrap().outcome,
        ConcreteDiagnosticOutcome::Error(3)
    );
    assert_eq!(failure.usage.used(R::PayloadTuples), 1);
}

#[test]
fn vrf_all_thirteen_bounds_fail_without_accepted_prefix() {
    let f = Fixture::with_vrf_member(vec![9; 32]);
    let plan = vrf_plan(plan(f.plan.head.state_root));
    for resource in [
        R::KeyBytes,
        R::RowBytes,
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::EncodedBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::ConstructorIdentities,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let limits = ConcreteWitnessLimits::default()
            .lowered(resource, 0)
            .unwrap();
        let failure = capture_vrf_run(&f.application, &f.state, &plan, limits)
            .err()
            .unwrap();
        assert!(
            matches!(failure.error,ConcreteWitnessError::LimitExceeded {resource:r,..} if r==resource),
            "{resource:?}: {failure:?}"
        );
        assert_eq!(failure.usage.used(resource), 0);
    }
}
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
    fn with_vrf_member(value: Vec<u8>) -> Self {
        Self::with_member_at(value, vrf_plan(plan(codec::empty_trie_root())).tuples[0])
    }
    fn with_residual_member(value: Vec<u8>) -> Self {
        Self::with_member_at(
            value,
            residual_first(plan(codec::empty_trie_root())).tuples[2],
        )
    }
    fn with_member_at(value: Vec<u8>, tuple: ConcreteDiagnosticTuple) -> Self {
        fn leaf(path: &[u8], value: &[u8]) -> Vec<u8> {
            let mut s = RlpStream::new_list(2);
            s.append(&path);
            s.append(&value);
            s.out().to_vec()
        }
        let mut f = Self::empty();
        let mut compact = vec![0x20];
        compact.extend_from_slice(&codec::storage_trie_path(tuple.key));
        let storage_root = codec::keccak256(&leaf(&compact, &rlp::encode(&value)));
        let storage_node = leaf(&compact, &storage_root);
        let mut raw = RlpStream::new_list(5);
        raw.append_empty_data();
        raw.append(&vec![1u8]);
        raw.append(&storage_root.as_slice());
        raw.append_empty_data();
        raw.append(&0u64);
        let raw = raw.out().to_vec();
        let commitment =
            codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap()).unwrap();
        let prefix = codec::account_version_prefix(tuple.address);
        let mut compact = vec![0x20];
        compact.extend_from_slice(&prefix);
        let root = codec::keccak256(&leaf(&compact, &commitment));
        let account_node = leaf(&compact, &root);
        f.plan = plan(root);
        {
            let db = Owner::open_cf_descriptors(
                &Options::default(),
                &f.application,
                APPLICATION_COLUMNS
                    .into_iter()
                    .map(|c| c.descriptor(&Options::default()))
                    .chain(std::iter::once(ColumnFamilyDescriptor::new(
                        "unrelated_inventory_forbidden",
                        Options::default(),
                    ))),
            )
            .unwrap();
            for period in [HEAD, PARENT] {
                db.put_cf(
                    &db.cf_handle(Column::FinalChainBlkByNumber.name()).unwrap(),
                    period.to_le_bytes(),
                    header(root),
                )
                .unwrap();
            }
        }
        {
            let db = Owner::open_cf_descriptors(
                &Options::default(),
                &f.state,
                CONCRETE_COLUMNS.map(|c| ColumnFamilyDescriptor::new(c, Options::default())),
            )
            .unwrap();
            db.put(DESCRIPTOR_KEY, raw_descriptor(f.plan.head)).unwrap();
            for (column, key, value) in [
                ("2", root.to_vec(), account_node),
                ("4", storage_root.to_vec(), storage_node),
                (
                    "3",
                    codec::versioned_key(prefix, tuple.identity.period).to_vec(),
                    raw,
                ),
                (
                    "5",
                    codec::versioned_key(
                        codec::storage_version_prefix(tuple.address, tuple.key),
                        tuple.identity.period,
                    )
                    .to_vec(),
                    value,
                ),
            ] {
                db.put_cf(&db.cf_handle(column).unwrap(), key, value)
                    .unwrap();
            }
        }
        f
    }
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
fn vrf_present_selected_version_and_tombstone_require_matching_paths() {
    let f = Fixture::with_vrf_member(vec![9; 32]);
    let plan = vrf_plan(plan(f.plan.head.state_root));
    let capture = capture_vrf_run(
        &f.application,
        &f.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let replay =
        replay_vrf_bundle(&capture.bundle, &plan, ConcreteWitnessLimits::default()).unwrap();
    assert_eq!(
        replay.logical[0],
        ConcreteWitnessLogical::Member(vec![9; 32])
    );
    assert_eq!(
        replay.physical[0],
        ConcreteWitnessPhysical::Present(vec![9; 32])
    );
    assert!(
        replay.logical[1..]
            .iter()
            .all(|v| *v == ConcreteWitnessLogical::NonMember)
    );
    let tuple = plan.tuples[0];
    {
        let db = Owner::open_cf_descriptors(
            &Options::default(),
            &f.state,
            CONCRETE_COLUMNS.map(|c| ColumnFamilyDescriptor::new(c, Options::default())),
        )
        .unwrap();
        db.put_cf(
            &db.cf_handle("5").unwrap(),
            codec::versioned_key(
                codec::storage_version_prefix(tuple.address, tuple.key),
                tuple.identity.period,
            ),
            [],
        )
        .unwrap();
    }
    let failed = capture_vrf_run(
        &f.application,
        &f.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(failed.phase, ConcreteDiagnosticPhase::Payload);
    assert_eq!(failed.index, 0);
    assert_eq!(failed.error, ConcreteWitnessError::Proof);
}

#[test]
fn vrf_malformed_present_width_stops_capture_and_replay_at_first_payload() {
    for width in [1, 31, 33, 64] {
        let f = Fixture::with_vrf_member(vec![9; width]);
        let plan = vrf_plan(plan(f.plan.head.state_root));
        let failure = capture_vrf_run(
            &f.application,
            &f.state,
            &plan,
            ConcreteWitnessLimits::default(),
        )
        .err()
        .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(failure.index, 0);
        assert_eq!(failure.error, ConcreteWitnessError::Schema);
        assert_eq!(failure.usage.used(R::PayloadTuples), 1);
        // Synthetic malformed-value fixture: the private test-only invocation
        // omits VRF semantics while retaining genuine root/path reconstruction.
        let captured = capture_family(
            &f.application,
            &f.state,
            &plan.context,
            &plan.tuples,
            bundle::Profile::FirstVrf,
            false,
            ConcreteWitnessLimits::default(),
        )
        .unwrap();
        let failure = replay_vrf_bundle(&captured.bundle, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(failure.index, 0);
        assert_eq!(failure.error, ConcreteWitnessError::Schema);
        assert_eq!(failure.usage.used(R::PayloadTuples), 1);
    }
}

#[test]
fn vrf_tombstone_with_independent_nonmembership_preserves_physical_provenance() {
    let f = Fixture::empty();
    let plan = vrf_plan(plan(codec::empty_trie_root()));
    let tuple = plan.tuples[0];
    {
        let db = Owner::open_cf_descriptors(
            &Options::default(),
            &f.state,
            CONCRETE_COLUMNS.map(|c| ColumnFamilyDescriptor::new(c, Options::default())),
        )
        .unwrap();
        db.put_cf(
            &db.cf_handle("5").unwrap(),
            codec::versioned_key(
                codec::storage_version_prefix(tuple.address, tuple.key),
                tuple.identity.period,
            ),
            [],
        )
        .unwrap();
    }
    let capture = capture_vrf_run(
        &f.application,
        &f.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let replay =
        replay_vrf_bundle(&capture.bundle, &plan, ConcreteWitnessLimits::default()).unwrap();
    assert_eq!(replay.physical[0], ConcreteWitnessPhysical::Tombstone);
    assert_eq!(replay.logical[0], ConcreteWitnessLogical::NonMember);
}

#[test]
fn voter_window_fixed_order_and_profile_independently_replay_all_remaining_periods() {
    let f = Fixture::with_vrf_member(vec![9; 32]);
    let first = vrf_plan(plan(f.plan.head.state_root));
    let old = capture_vrf_run(
        &f.application,
        &f.state,
        &first,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let window = ConcreteVrfVoterPlan::new(first, [1; 20]).unwrap();
    assert_eq!(window.voter(), [1; 20]);
    for (i, t) in window.tuples().iter().enumerate() {
        assert_eq!(t.identity.period.as_u64(), FIRST_DELAYED + 1 + i as u64);
        assert_eq!(t.key, window.tuples[0].key);
        assert_eq!(t.address, window.tuples[0].address);
    }
    let capture = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &window,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert_eq!(&capture.bundle[..4], b"RCV2");
    assert_eq!(capture.usage.used(R::PayloadTuples), 47);
    let replay =
        replay_vrf_voter_bundle(&capture.bundle, &window, ConcreteWitnessLimits::default())
            .unwrap();
    assert!(
        replay
            .logical
            .iter()
            .all(|v| *v == ConcreteWitnessLogical::Member(vec![9; 32]))
    );
    assert!(
        replay
            .physical
            .iter()
            .all(|v| *v == ConcreteWitnessPhysical::Present(vec![9; 32]))
    );
    assert_eq!(replay.usage.used(R::Registrations), 51);
    assert_eq!(replay.usage.used(R::DistinctIdentities), 50);
    assert!(
        replay_vrf_voter_bundle(&old.bundle, &window, ConcreteWitnessLimits::default()).is_err()
    );
    let first = vrf_plan(plan(f.plan.head.state_root));
    assert!(replay_vrf_bundle(&capture.bundle, &first, ConcreteWitnessLimits::default()).is_err());
    for i in [0, 4, 8, 10, 12, 14] {
        let mut b = capture.bundle.clone();
        b[i] ^= 1;
        assert!(replay_vrf_voter_bundle(&b, &window, ConcreteWitnessLimits::default()).is_err());
    }
    let mut trailing = capture.bundle.clone();
    trailing.push(0);
    assert!(replay_vrf_voter_bundle(&trailing, &window, ConcreteWitnessLimits::default()).is_err());
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(capture.bundle.len()).unwrap();
    let env =
        bundle::admit_profile::<98>(&capture.bundle, &budget, bundle::Profile::VrfVoterWindow)
            .unwrap();
    for offset in [8, 16, 48, 68, usize::MAX] {
        let mut proofs: Vec<_> = env.proofs.iter().map(|v| v.to_vec()).collect();
        if offset == usize::MAX {
            proofs.swap(51, 52);
        } else {
            proofs[51][offset] ^= 1;
        }
        let b = bundle::encode_profile(
            &env.pair.map(|v| v.to_vec()),
            &env.descriptors.map(|v| v.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfVoterWindow,
        )
        .unwrap();
        let failure = replay_vrf_voter_bundle(&b, &window, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(failure.index, 0);
    }
}

#[test]
fn voter_windows_with_same_rcv2_framing_reject_each_others_fixed_keys() {
    let fixture = Fixture::empty();
    let plans = [1, 2].map(|voter| {
        ConcreteVrfVoterPlan::new(vrf_plan(plan(codec::empty_trie_root())), [voter; 20]).unwrap()
    });
    let captures = plans.each_ref().map(|plan| {
        capture_vrf_voter_run(
            &fixture.application,
            &fixture.state,
            plan,
            ConcreteWitnessLimits::default(),
        )
        .unwrap()
    });
    assert_eq!(&captures[0].bundle[..16], &captures[1].bundle[..16]);
    assert_eq!(&captures[0].bundle[..4], b"RCV2");
    for index in 0..2 {
        let verified = replay_vrf_voter_bundle(
            &captures[index].bundle,
            &plans[index],
            ConcreteWitnessLimits::default(),
        )
        .unwrap();
        assert_eq!(verified.usage.used(R::PayloadTuples), 47);
        assert!(
            verified
                .logical
                .iter()
                .all(|v| *v == ConcreteWitnessLogical::NonMember)
        );
        let error = replay_vrf_voter_bundle(
            &captures[index].bundle,
            &plans[1 - index],
            ConcreteWitnessLimits::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.error, ConcreteWitnessError::Identity);
        assert_eq!(error.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(error.index, 0);
    }
}

#[test]
fn voter_window_missing_voter_wrong_root_and_late_frame_bound_fail_closed() {
    assert!(ConcreteVrfVoterPlan::new(vrf_plan(plan(codec::empty_trie_root())), [0; 20]).is_err());
    let f = Fixture::with_vrf_member(vec![9; 32]);
    let p = ConcreteVrfVoterPlan::new(vrf_plan(plan(f.plan.head.state_root)), [1; 20]).unwrap();
    let captured = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let limit = captured.usage.used(R::FrameInspections) - 1;
    let failure = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default()
            .lowered(R::FrameInspections, limit)
            .unwrap(),
    )
    .err()
    .unwrap();
    assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
    assert!(failure.index > 0);
    assert!(matches!(
        failure.error,
        ConcreteWitnessError::LimitExceeded {
            resource: R::FrameInspections,
            ..
        }
    ));
    let mut wrong = plan(f.plan.head.state_root);
    wrong.delayed[10].state_root = [7; 32];
    wrong.tuples[10].identity = wrong.delayed[10];
    wrong.tuples[58].identity = wrong.delayed[10];
    let p = ConcreteVrfVoterPlan::new(vrf_plan(wrong), [1; 20]).unwrap();
    let failure = replay_vrf_voter_bundle(&captured.bundle, &p, ConcreteWitnessLimits::default())
        .err()
        .unwrap();
    assert_eq!(failure.phase, ConcreteDiagnosticPhase::SecondConstructor);
    assert_eq!(failure.error, ConcreteWitnessError::Identity);
}

#[test]
fn voter_window_all_bounds_and_malformed_member_stop_before_later_payloads() {
    let f = Fixture::with_vrf_member(vec![9; 32]);
    let p = ConcreteVrfVoterPlan::new(vrf_plan(plan(f.plan.head.state_root)), [1; 20]).unwrap();
    for r in [
        R::KeyBytes,
        R::RowBytes,
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::EncodedBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::ConstructorIdentities,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let e = capture_vrf_voter_run(
            &f.application,
            &f.state,
            &p,
            ConcreteWitnessLimits::default().lowered(r, 0).unwrap(),
        )
        .err()
        .unwrap();
        assert!(matches!(e.error,ConcreteWitnessError::LimitExceeded {resource,..} if resource==r));
        assert_eq!(e.usage.used(r), 0);
    }
    let f = Fixture::with_vrf_member(vec![9; 31]);
    let p = ConcreteVrfVoterPlan::new(vrf_plan(plan(f.plan.head.state_root)), [1; 20]).unwrap();
    let e = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.phase, ConcreteDiagnosticPhase::Payload);
    assert_eq!(e.index, 0);
    assert_eq!(e.error, ConcreteWitnessError::Schema);
    assert_eq!(e.usage.used(R::PayloadTuples), 1);
}

#[test]
fn voter_window_history_error_and_unused_frames_stop_at_the_bound_payload() {
    let f = Fixture::empty();
    let p = ConcreteVrfVoterPlan::new(vrf_plan(plan(codec::empty_trie_root())), [1; 20]).unwrap();
    let captured = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(captured.bundle.len()).unwrap();
    let env =
        bundle::admit_profile::<98>(&captured.bundle, &budget, bundle::Profile::VrfVoterWindow)
            .unwrap();
    for extra in [false, true] {
        let mut proofs: Vec<_> = env.proofs.iter().map(|v| v.to_vec()).collect();
        let index = 17;
        let target = 51 + index;
        let tuple = p.tuples[index];
        let mut header = binary::admit_transcript(
            &proofs[target],
            &budget,
            Some((tuple.identity, tuple.address, tuple.key)),
            false,
        )
        .unwrap();
        let mut offset = binary::HEADER_BYTES;
        let mut frames = Vec::new();
        while offset < proofs[target].len() {
            let (frame, next) = binary::frame_at(&proofs[target], offset, &budget).unwrap();
            frames.push(frame);
            offset = next;
        }
        if extra {
            let source = &frames[1];
            let frame = binary::Frame {
                ordinal: frames.len() as u64,
                identity: source.identity,
                operation: source.operation,
                column: source.column,
                request: source.request,
                period: source.period,
                outcome: binary::Outcome::End,
            };
            frames.push(frame);
            header.records += 1;
        } else {
            frames[0].outcome = binary::Outcome::Error(3);
        }
        let encoder = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        encoder.charge(R::Records, frames.len() as u64).unwrap();
        let altered = binary::encode_transcript(header, &frames, &encoder, 0, false).unwrap();
        proofs[target] = altered;
        let bytes = bundle::encode_profile(
            &env.pair.map(|v| v.to_vec()),
            &env.descriptors.map(|v| v.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfVoterWindow,
        )
        .unwrap();
        let e = replay_vrf_voter_bundle(&bytes, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(e.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(e.index, index as u16);
        assert_eq!(e.usage.used(R::PayloadTuples), 18);
        if extra {
            assert_eq!(e.error, ConcreteWitnessError::UnusedRecords);
        } else {
            assert_eq!(
                e.last_read.unwrap().outcome,
                ConcreteDiagnosticOutcome::Error(3)
            );
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

fn residual_first(context: ConcreteDiagnosticPlan) -> ConcreteVrfPlan {
    let mut voters = std::array::from_fn(|i| [i as u8 + 1; 20]);
    voters[2] = ConcreteVrfResidualPlan::VOTER;
    ConcreteVrfPlan::new(context, voters).unwrap()
}
#[test]
fn residual_fixed_order_and_profile_independently_replay_all_remaining_periods() {
    let f = Fixture::with_residual_member(vec![9; 32]);
    let first = residual_first(plan(f.plan.head.state_root));
    let old = capture_vrf_run(
        &f.application,
        &f.state,
        &first,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let window = ConcreteVrfResidualPlan::new(first).unwrap();
    assert_eq!(window.voter(), ConcreteVrfResidualPlan::VOTER);
    for (i, t) in window.tuples().iter().enumerate() {
        assert_eq!(
            t.identity.period.as_u64(),
            FIRST_DELAYED + if i < 4 { i as u64 + 1 } else { i as u64 + 2 }
        );
        assert_eq!(t.key, window.tuples[0].key);
        assert_eq!(t.address, window.tuples[0].address);
    }
    let capture = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &window,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert_eq!(&capture.bundle[..4], b"RCV3");
    assert_eq!(capture.usage.used(R::PayloadTuples), 46);
    let replay =
        replay_vrf_residual_bundle(&capture.bundle, &window, ConcreteWitnessLimits::default())
            .unwrap();
    assert!(
        replay
            .logical
            .iter()
            .all(|v| *v == ConcreteWitnessLogical::Member(vec![9; 32]))
    );
    assert!(
        replay
            .physical
            .iter()
            .all(|v| *v == ConcreteWitnessPhysical::Present(vec![9; 32]))
    );
    assert_eq!(replay.usage.used(R::Registrations), 51);
    assert_eq!(replay.usage.used(R::DistinctIdentities), 50);
    assert!(
        replay_vrf_residual_bundle(&old.bundle, &window, ConcreteWitnessLimits::default()).is_err()
    );
    let first = residual_first(plan(f.plan.head.state_root));
    assert!(replay_vrf_bundle(&capture.bundle, &first, ConcreteWitnessLimits::default()).is_err());
    let complete = ConcreteVrfVoterPlan::new(
        residual_first(plan(f.plan.head.state_root)),
        ConcreteVrfResidualPlan::VOTER,
    )
    .unwrap();
    let complete_capture = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &complete,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert!(
        replay_vrf_residual_bundle(
            &complete_capture.bundle,
            &window,
            ConcreteWitnessLimits::default()
        )
        .is_err()
    );
    assert!(
        replay_vrf_voter_bundle(&capture.bundle, &complete, ConcreteWitnessLimits::default())
            .is_err()
    );
    assert!(
        replay_diagnostic_bundle(
            &capture.bundle,
            window.context(),
            ConcreteWitnessLimits::default()
        )
        .is_err()
    );
    for length in [
        0,
        15,
        16,
        capture.bundle.len() / 2,
        capture.bundle.len() - 1,
    ] {
        assert!(
            replay_vrf_residual_bundle(
                &capture.bundle[..length],
                &window,
                ConcreteWitnessLimits::default()
            )
            .is_err()
        );
    }
    for i in [0, 4, 8, 10, 12, 14] {
        let mut b = capture.bundle.clone();
        b[i] ^= 1;
        assert!(replay_vrf_residual_bundle(&b, &window, ConcreteWitnessLimits::default()).is_err());
    }
    let mut trailing = capture.bundle.clone();
    trailing.push(0);
    assert!(
        replay_vrf_residual_bundle(&trailing, &window, ConcreteWitnessLimits::default()).is_err()
    );
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(capture.bundle.len()).unwrap();
    let env = bundle::admit_profile::<97>(&capture.bundle, &budget, bundle::Profile::VrfResidual)
        .unwrap();
    for offset in [8, 16, 48, 68, usize::MAX] {
        let mut proofs: Vec<_> = env.proofs.iter().map(|v| v.to_vec()).collect();
        if offset == usize::MAX {
            proofs.swap(51, 52);
        } else {
            proofs[51][offset] ^= 1;
        }
        let b = bundle::encode_profile(
            &env.pair.map(|v| v.to_vec()),
            &env.descriptors.map(|v| v.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfResidual,
        )
        .unwrap();
        let failure = replay_vrf_residual_bundle(&b, &window, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(failure.index, 0);
    }
}

#[test]
fn residual_missing_voter_wrong_root_and_late_frame_bound_fail_closed() {
    assert!(ConcreteVrfResidualPlan::new(vrf_plan(plan(codec::empty_trie_root()))).is_err());
    let f = Fixture::with_residual_member(vec![9; 32]);
    let p = ConcreteVrfResidualPlan::new(residual_first(plan(f.plan.head.state_root))).unwrap();
    let captured = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let limit = captured.usage.used(R::FrameInspections) - 1;
    let failure = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default()
            .lowered(R::FrameInspections, limit)
            .unwrap(),
    )
    .err()
    .unwrap();
    assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
    assert!(failure.index > 0);
    assert!(matches!(
        failure.error,
        ConcreteWitnessError::LimitExceeded {
            resource: R::FrameInspections,
            ..
        }
    ));
    let mut wrong = plan(f.plan.head.state_root);
    wrong.delayed[10].state_root = [7; 32];
    wrong.tuples[10].identity = wrong.delayed[10];
    wrong.tuples[58].identity = wrong.delayed[10];
    let p = ConcreteVrfResidualPlan::new(residual_first(wrong)).unwrap();
    let failure =
        replay_vrf_residual_bundle(&captured.bundle, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
    assert_eq!(failure.phase, ConcreteDiagnosticPhase::SecondConstructor);
    assert_eq!(failure.error, ConcreteWitnessError::Identity);
}

#[test]
fn residual_all_bounds_and_malformed_member_stop_before_later_payloads() {
    let f = Fixture::with_residual_member(vec![9; 32]);
    let p = ConcreteVrfResidualPlan::new(residual_first(plan(f.plan.head.state_root))).unwrap();
    for r in [
        R::KeyBytes,
        R::RowBytes,
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::EncodedBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::ConstructorIdentities,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let e = capture_vrf_residual_run(
            &f.application,
            &f.state,
            &p,
            ConcreteWitnessLimits::default().lowered(r, 0).unwrap(),
        )
        .err()
        .unwrap();
        assert!(matches!(e.error,ConcreteWitnessError::LimitExceeded {resource,..} if resource==r));
        assert_eq!(e.usage.used(r), 0);
    }
    let f = Fixture::with_residual_member(vec![9; 31]);
    let p = ConcreteVrfResidualPlan::new(residual_first(plan(f.plan.head.state_root))).unwrap();
    let e = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.phase, ConcreteDiagnosticPhase::Payload);
    assert_eq!(e.index, 0);
    assert_eq!(e.error, ConcreteWitnessError::Schema);
    assert_eq!(e.usage.used(R::PayloadTuples), 1);
}

#[test]
fn residual_history_error_and_unused_frames_stop_at_the_bound_payload() {
    let f = Fixture::empty();
    let p = ConcreteVrfResidualPlan::new(residual_first(plan(codec::empty_trie_root()))).unwrap();
    let captured = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.admit_input(captured.bundle.len()).unwrap();
    let env = bundle::admit_profile::<97>(&captured.bundle, &budget, bundle::Profile::VrfResidual)
        .unwrap();
    for extra in [false, true] {
        let mut proofs: Vec<_> = env.proofs.iter().map(|v| v.to_vec()).collect();
        let index = 17;
        let target = 51 + index;
        let tuple = p.tuples[index];
        let mut header = binary::admit_transcript(
            &proofs[target],
            &budget,
            Some((tuple.identity, tuple.address, tuple.key)),
            false,
        )
        .unwrap();
        let mut offset = binary::HEADER_BYTES;
        let mut frames = Vec::new();
        while offset < proofs[target].len() {
            let (frame, next) = binary::frame_at(&proofs[target], offset, &budget).unwrap();
            frames.push(frame);
            offset = next;
        }
        if extra {
            let source = &frames[1];
            let frame = binary::Frame {
                ordinal: frames.len() as u64,
                identity: source.identity,
                operation: source.operation,
                column: source.column,
                request: source.request,
                period: source.period,
                outcome: binary::Outcome::End,
            };
            frames.push(frame);
            header.records += 1;
        } else {
            frames[0].outcome = binary::Outcome::Error(3);
        }
        let encoder = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        encoder.charge(R::Records, frames.len() as u64).unwrap();
        let altered = binary::encode_transcript(header, &frames, &encoder, 0, false).unwrap();
        proofs[target] = altered;
        let bytes = bundle::encode_profile(
            &env.pair.map(|v| v.to_vec()),
            &env.descriptors.map(|v| v.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfResidual,
        )
        .unwrap();
        let e = replay_vrf_residual_bundle(&bytes, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(e.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(e.index, index as u16);
        assert_eq!(e.usage.used(R::PayloadTuples), 18);
        if extra {
            assert_eq!(e.error, ConcreteWitnessError::UnusedRecords);
        } else {
            assert_eq!(
                e.last_read.unwrap().outcome,
                ConcreteDiagnosticOutcome::Error(3)
            );
        }
    }
}

#[test]
fn residual_context_shape_prior_root_and_omitted_period_retention() {
    let f = Fixture::empty();
    let p = ConcreteVrfResidualPlan::new(residual_first(plan(codec::empty_trie_root()))).unwrap();
    assert_eq!(p.tuples().len(), 46);
    assert_eq!(p.context().delayed.len(), 48);
    assert!(
        p.tuples()
            .iter()
            .all(|t| t.identity != p.context().delayed[5])
    );
    let preimage = hex::decode("000403bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b").unwrap();
    assert_eq!(p.tuples()[0].key.0, codec::keccak256(&preimage));
    assert_eq!(
        hex::encode(p.tuples()[0].key.0),
        "89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b"
    );
    let mut malformed = residual_first(plan(codec::empty_trie_root()));
    malformed.context.delayed.swap(0, 1);
    assert!(ConcreteVrfResidualPlan::new(malformed).is_err());
    let captured = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    for index in [0, 5] {
        let mut context = plan(codec::empty_trie_root());
        context.delayed[index].state_root = [7; 32];
        context.tuples[index].identity = context.delayed[index];
        context.tuples[index + 48].identity = context.delayed[index];
        let wrong = ConcreteVrfResidualPlan::new(residual_first(context)).unwrap();
        let failed =
            replay_vrf_residual_bundle(&captured.bundle, &wrong, ConcreteWitnessLimits::default())
                .err()
                .unwrap();
        assert_eq!(failed.phase, ConcreteDiagnosticPhase::SecondConstructor);
        assert_eq!(failed.index, index as u16);
        assert_eq!(failed.error, ConcreteWitnessError::Identity);
        assert_eq!(failed.usage.used(R::PayloadTuples), 0);
    }
}

#[test]
fn residual_tombstone_and_absence_preserve_independent_nonmembership() {
    let f = Fixture::empty();
    let p = ConcreteVrfResidualPlan::new(residual_first(plan(codec::empty_trie_root()))).unwrap();
    let absent = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let replay =
        replay_vrf_residual_bundle(&absent.bundle, &p, ConcreteWitnessLimits::default()).unwrap();
    assert!(
        replay
            .logical
            .iter()
            .all(|v| *v == ConcreteWitnessLogical::NonMember)
    );
    for (physical, tuple) in replay.physical.iter().zip(p.tuples()) {
        assert_eq!(
            *physical,
            ConcreteWitnessPhysical::HistoryUnavailable(tuple.identity)
        );
    }
    let tuple = p.tuples[0];
    {
        let db = Owner::open_cf_descriptors(
            &Options::default(),
            &f.state,
            CONCRETE_COLUMNS.map(|c| ColumnFamilyDescriptor::new(c, Options::default())),
        )
        .unwrap();
        db.put_cf(
            &db.cf_handle("5").unwrap(),
            codec::versioned_key(
                codec::storage_version_prefix(tuple.address, tuple.key),
                tuple.identity.period,
            ),
            [],
        )
        .unwrap();
    }
    let captured = capture_vrf_residual_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let replay =
        replay_vrf_residual_bundle(&captured.bundle, &p, ConcreteWitnessLimits::default()).unwrap();
    assert!(
        replay
            .logical
            .iter()
            .all(|v| *v == ConcreteWitnessLogical::NonMember)
    );
    assert!(
        replay
            .physical
            .iter()
            .all(|v| *v == ConcreteWitnessPhysical::Tombstone)
    );
}

#[test]
fn residual_omitted_period_history_and_missing_node_stop_before_payload() {
    let fixture = Fixture::with_residual_member(vec![9; 32]);
    let plan =
        ConcreteVrfResidualPlan::new(residual_first(self::plan(fixture.plan.head.state_root)))
            .unwrap();
    let omitted = plan.context().delayed[5];
    assert!(plan.tuples().iter().all(|tuple| tuple.identity != omitted));
    let captured = capture_vrf_residual_run(
        &fixture.application,
        &fixture.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let admission = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    admission.admit_input(captured.bundle.len()).unwrap();
    let envelope =
        bundle::admit_profile::<97>(&captured.bundle, &admission, bundle::Profile::VrfResidual)
            .unwrap();
    // The two first-constructor proofs precede all 48 delayed identities.
    let target = 2 + 5;
    for history_error in [true, false] {
        let mut proofs: Vec<_> = envelope.proofs.iter().map(|proof| proof.to_vec()).collect();
        let decode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        let header = binary::admit_transcript(
            &proofs[target],
            &decode,
            Some((omitted, [0; 20], ConcreteStorageKey([0; 32]))),
            true,
        )
        .unwrap();
        let mut frames = Vec::new();
        let mut offset = binary::HEADER_BYTES;
        while offset < proofs[target].len() {
            let (frame, next) = binary::frame_at(&proofs[target], offset, &decode).unwrap();
            frames.push(frame);
            offset = next;
        }
        let node = frames
            .iter_mut()
            .find(|frame| frame.operation == 0 && frame.column == 2)
            .unwrap();
        assert!(matches!(node.outcome, binary::Outcome::Node(_)));
        node.outcome = if history_error {
            binary::Outcome::Error(3)
        } else {
            binary::Outcome::End
        };
        let encode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        encode.charge(R::Records, frames.len() as u64).unwrap();
        let altered = binary::encode_transcript(header, &frames, &encode, 0, true).unwrap();
        proofs[target] = altered;
        let bytes = bundle::encode_profile(
            &envelope.pair.map(|row| row.to_vec()),
            &envelope.descriptors.map(|row| row.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfResidual,
        )
        .unwrap();
        let failure = replay_vrf_residual_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::SecondConstructor);
        assert_eq!(failure.index, 5);
        assert_eq!(failure.error, ConcreteWitnessError::Proof);
        assert_eq!(failure.usage.used(R::PayloadTuples), 0);
        assert_eq!(
            failure.last_read.unwrap().outcome,
            if history_error {
                ConcreteDiagnosticOutcome::Error(3)
            } else {
                ConcreteDiagnosticOutcome::End
            }
        );
    }
}

fn residual_b92a_first(context: ConcreteDiagnosticPlan) -> ConcreteVrfPlan {
    let mut voters = std::array::from_fn(|i| [i as u8 + 1; 20]);
    voters[2] = ConcreteVrfResidualPlan::VOTER;
    voters[19] = ConcreteVrfResidualPlan::B92A;
    ConcreteVrfPlan::new(context, voters).unwrap()
}

#[test]
fn residual_b92a_fixed_order_key_and_first_instance_cross_rejection() {
    let fixture = Fixture::empty();
    let first =
        ConcreteVrfResidualPlan::new(residual_b92a_first(plan(codec::empty_trie_root()))).unwrap();
    let second =
        ConcreteVrfResidualPlan::new_b92a(residual_b92a_first(plan(codec::empty_trie_root())))
            .unwrap();
    assert_eq!(first.voter(), ConcreteVrfResidualPlan::VOTER);
    assert_eq!(second.voter(), ConcreteVrfResidualPlan::B92A);
    for (p, omitted, expected_key) in [
        (
            &first,
            5,
            "89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b",
        ),
        (
            &second,
            36,
            "92f670429dc4076aa7771488f1512225a3bbf93eb17aa2e37a8cedcd463fff65",
        ),
    ] {
        assert_eq!(p.tuples().len(), 46);
        assert_eq!(p.context().delayed.len(), 48);
        let mut preimage = [0; 22];
        preimage[1] = 4;
        preimage[2..].copy_from_slice(&p.voter());
        for (i, tuple) in p.tuples().iter().enumerate() {
            let delayed = if i < omitted - 1 { i + 1 } else { i + 2 };
            assert_eq!(tuple.identity, p.context().delayed[delayed]);
            assert_eq!(tuple.key.0, codec::keccak256(&preimage));
            assert_eq!(hex::encode(tuple.key.0), expected_key);
            let mut account = [0; 20];
            account[19] = 0xfe;
            assert_eq!(tuple.address, account);
        }
    }
    let captures = [&first, &second].map(|p| {
        capture_vrf_residual_run(
            &fixture.application,
            &fixture.state,
            p,
            ConcreteWitnessLimits::default(),
        )
        .unwrap()
    });
    assert_eq!(&captures[0].bundle[..16], &captures[1].bundle[..16]);
    for (i, p) in [&first, &second].iter().enumerate() {
        let valid =
            replay_vrf_residual_bundle(&captures[i].bundle, p, ConcreteWitnessLimits::default())
                .unwrap();
        assert_eq!(valid.usage.used(R::PayloadTuples), 46);
        let wrong = replay_vrf_residual_bundle(
            &captures[1 - i].bundle,
            p,
            ConcreteWitnessLimits::default(),
        )
        .err()
        .unwrap();
        assert_eq!(wrong.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(wrong.index, 0);
        assert_eq!(wrong.error, ConcreteWitnessError::Identity);
        assert_eq!(wrong.usage.used(R::PayloadTuples), 0);
    }
}

#[test]
fn residual_b92a_missing_voter_malformed_context_caps_and_eof() {
    assert!(
        ConcreteVrfResidualPlan::new_b92a(residual_first(plan(codec::empty_trie_root()))).is_err()
    );
    let mut malformed = residual_b92a_first(plan(codec::empty_trie_root()));
    malformed.context.delayed.swap(35, 36);
    assert!(ConcreteVrfResidualPlan::new_b92a(malformed).is_err());
    let fixture = Fixture::with_residual_member(vec![9; 32]);
    let p =
        ConcreteVrfResidualPlan::new_b92a(residual_b92a_first(plan(fixture.plan.head.state_root)))
            .unwrap();
    for resource in [
        R::KeyBytes,
        R::RowBytes,
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::EncodedBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::ConstructorIdentities,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let failed = capture_vrf_residual_run(
            &fixture.application,
            &fixture.state,
            &p,
            ConcreteWitnessLimits::default()
                .lowered(resource, 0)
                .unwrap(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(failed.error, ConcreteWitnessError::LimitExceeded { resource: r, .. } if r == resource)
        );
        assert_eq!(failed.usage.used(resource), 0);
    }
    let captured = capture_vrf_residual_run(
        &fixture.application,
        &fixture.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    for length in [
        0,
        15,
        16,
        captured.bundle.len() / 2,
        captured.bundle.len() - 1,
    ] {
        assert!(
            replay_vrf_residual_bundle(
                &captured.bundle[..length],
                &p,
                ConcreteWitnessLimits::default()
            )
            .is_err()
        );
    }
    let mut trailing = captured.bundle.clone();
    trailing.push(0);
    assert!(replay_vrf_residual_bundle(&trailing, &p, ConcreteWitnessLimits::default()).is_err());
}
#[test]
fn residual_b92a_omitted_period_history_and_missing_node_stop_before_payload() {
    let fixture = Fixture::with_residual_member(vec![9; 32]);
    let plan = ConcreteVrfResidualPlan::new_b92a(residual_b92a_first(self::plan(
        fixture.plan.head.state_root,
    )))
    .unwrap();
    let omitted = plan.context().delayed[36];
    assert!(plan.tuples().iter().all(|tuple| tuple.identity != omitted));
    let captured = capture_vrf_residual_run(
        &fixture.application,
        &fixture.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let admission = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    admission.admit_input(captured.bundle.len()).unwrap();
    let envelope =
        bundle::admit_profile::<97>(&captured.bundle, &admission, bundle::Profile::VrfResidual)
            .unwrap();
    // The two first-constructor proofs precede all 48 delayed identities.
    let target = 2 + 36;
    for history_error in [true, false] {
        let mut proofs: Vec<_> = envelope.proofs.iter().map(|proof| proof.to_vec()).collect();
        let decode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        let header = binary::admit_transcript(
            &proofs[target],
            &decode,
            Some((omitted, [0; 20], ConcreteStorageKey([0; 32]))),
            true,
        )
        .unwrap();
        let mut frames = Vec::new();
        let mut offset = binary::HEADER_BYTES;
        while offset < proofs[target].len() {
            let (frame, next) = binary::frame_at(&proofs[target], offset, &decode).unwrap();
            frames.push(frame);
            offset = next;
        }
        let node = frames
            .iter_mut()
            .find(|frame| frame.operation == 0 && frame.column == 2)
            .unwrap();
        assert!(matches!(node.outcome, binary::Outcome::Node(_)));
        node.outcome = if history_error {
            binary::Outcome::Error(3)
        } else {
            binary::Outcome::End
        };
        let encode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        encode.charge(R::Records, frames.len() as u64).unwrap();
        let altered = binary::encode_transcript(header, &frames, &encode, 0, true).unwrap();
        proofs[target] = altered;
        let bytes = bundle::encode_profile(
            &envelope.pair.map(|row| row.to_vec()),
            &envelope.descriptors.map(|row| row.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfResidual,
        )
        .unwrap();
        let failure = replay_vrf_residual_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::SecondConstructor);
        assert_eq!(failure.index, 36);
        assert_eq!(failure.error, ConcreteWitnessError::Proof);
        assert_eq!(failure.usage.used(R::PayloadTuples), 0);
        assert_eq!(
            failure.last_read.unwrap().outcome,
            if history_error {
                ConcreteDiagnosticOutcome::Error(3)
            } else {
                ConcreteDiagnosticOutcome::End
            }
        );
    }
}

fn residual_e50b_first(context: ConcreteDiagnosticPlan) -> ConcreteVrfPlan {
    let mut voters = std::array::from_fn(|i| [i as u8 + 1; 20]);
    voters[2] = ConcreteVrfResidualPlan::VOTER;
    voters[18] = ConcreteVrfResidualPlan::B92A;
    voters[19] = ConcreteVrfResidualPlan::E50B;
    ConcreteVrfPlan::new(context, voters).unwrap()
}

#[test]
fn residual_e50b_fixed_key_order_full_context_and_all_instance_cross_rejection() {
    let fixture = Fixture::empty();
    let root = codec::empty_trie_root();
    let plans = [
        ConcreteVrfResidualPlan::new(residual_e50b_first(plan(root))).unwrap(),
        ConcreteVrfResidualPlan::new_b92a(residual_e50b_first(plan(root))).unwrap(),
        ConcreteVrfResidualPlan::new_e50b(residual_e50b_first(plan(root))).unwrap(),
    ];
    assert_eq!(plans[2].voter(), ConcreteVrfResidualPlan::E50B);
    assert_eq!(
        hex::encode(plans[2].voter()),
        "e50b5452b2e8435404dbe06e6a05410c47b7583d"
    );
    for (p, omitted, key) in [
        (
            &plans[0],
            5,
            "89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b",
        ),
        (
            &plans[1],
            36,
            "92f670429dc4076aa7771488f1512225a3bbf93eb17aa2e37a8cedcd463fff65",
        ),
        (
            &plans[2],
            12,
            "97c2d7a6a0dc3ae78ab1fe9147bd5db10088db1c01850d9b8bce75ae287361d2",
        ),
    ] {
        assert_eq!(p.context().delayed.len(), 48);
        assert_eq!(p.tuples().len(), 46);
        let mut preimage = [0; 22];
        preimage[1] = 4;
        preimage[2..].copy_from_slice(&p.voter());
        let mut account = [0; 20];
        account[19] = 0xfe;
        for (i, tuple) in p.tuples().iter().enumerate() {
            let delayed = if i < omitted - 1 { i + 1 } else { i + 2 };
            assert_eq!(tuple.identity, p.context().delayed[delayed]);
            assert_ne!(tuple.identity, p.context().delayed[omitted]);
            assert_eq!(tuple.key.0, codec::keccak256(&preimage));
            assert_eq!(hex::encode(tuple.key.0), key);
            assert_eq!(tuple.address, account);
        }
    }
    let captures = plans.each_ref().map(|p| {
        capture_vrf_residual_run(
            &fixture.application,
            &fixture.state,
            p,
            ConcreteWitnessLimits::default(),
        )
        .unwrap()
    });
    for (i, capture) in captures.iter().enumerate() {
        assert_eq!(&capture.bundle[..16], &captures[0].bundle[..16]);
        let admission = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        admission.admit_input(capture.bundle.len()).unwrap();
        let envelope =
            bundle::admit_profile::<97>(&capture.bundle, &admission, bundle::Profile::VrfResidual)
                .unwrap();
        assert_eq!(envelope.proofs.len(), 97);
        for (j, p) in plans.iter().enumerate() {
            if i == j {
                let valid = replay_vrf_residual_bundle(
                    &capture.bundle,
                    p,
                    ConcreteWitnessLimits::default(),
                )
                .unwrap();
                assert_eq!(valid.usage.used(R::Registrations), 51);
                assert_eq!(valid.usage.used(R::PayloadTuples), 46);
            } else {
                let failure = replay_vrf_residual_bundle(
                    &capture.bundle,
                    p,
                    ConcreteWitnessLimits::default(),
                )
                .err()
                .unwrap();
                assert_eq!(failure.phase, ConcreteDiagnosticPhase::Payload);
                assert_eq!(failure.index, 0);
                assert_eq!(failure.error, ConcreteWitnessError::Identity);
                assert_eq!(failure.usage.used(R::PayloadTuples), 0);
            }
        }
    }
}
#[test]
fn residual_e50b_missing_voter_malformed_context_caps_and_eof() {
    assert!(
        ConcreteVrfResidualPlan::new_e50b(residual_first(plan(codec::empty_trie_root()))).is_err()
    );
    let mut malformed = residual_e50b_first(plan(codec::empty_trie_root()));
    malformed.context.delayed.swap(11, 12);
    assert!(ConcreteVrfResidualPlan::new_e50b(malformed).is_err());
    let fixture = Fixture::with_residual_member(vec![9; 32]);
    let p =
        ConcreteVrfResidualPlan::new_e50b(residual_e50b_first(plan(fixture.plan.head.state_root)))
            .unwrap();
    for resource in [
        R::KeyBytes,
        R::RowBytes,
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::EncodedBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::ConstructorIdentities,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let failed = capture_vrf_residual_run(
            &fixture.application,
            &fixture.state,
            &p,
            ConcreteWitnessLimits::default()
                .lowered(resource, 0)
                .unwrap(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(failed.error, ConcreteWitnessError::LimitExceeded { resource: r, .. } if r == resource)
        );
        assert_eq!(failed.usage.used(resource), 0);
    }
    let captured = capture_vrf_residual_run(
        &fixture.application,
        &fixture.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    for length in [
        0,
        15,
        16,
        captured.bundle.len() / 2,
        captured.bundle.len() - 1,
    ] {
        assert!(
            replay_vrf_residual_bundle(
                &captured.bundle[..length],
                &p,
                ConcreteWitnessLimits::default()
            )
            .is_err()
        );
    }
    let mut trailing = captured.bundle.clone();
    trailing.push(0);
    assert!(replay_vrf_residual_bundle(&trailing, &p, ConcreteWitnessLimits::default()).is_err());
}
#[test]
fn residual_e50b_omitted_period_history_and_missing_node_stop_before_payload() {
    let fixture = Fixture::with_residual_member(vec![9; 32]);
    let plan = ConcreteVrfResidualPlan::new_e50b(residual_e50b_first(self::plan(
        fixture.plan.head.state_root,
    )))
    .unwrap();
    let omitted = plan.context().delayed[12];
    assert!(plan.tuples().iter().all(|tuple| tuple.identity != omitted));
    let captured = capture_vrf_residual_run(
        &fixture.application,
        &fixture.state,
        &plan,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    let admission = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    admission.admit_input(captured.bundle.len()).unwrap();
    let envelope =
        bundle::admit_profile::<97>(&captured.bundle, &admission, bundle::Profile::VrfResidual)
            .unwrap();
    // The two first-constructor proofs precede all 48 delayed identities.
    let target = 2 + 12;
    for history_error in [true, false] {
        let mut proofs: Vec<_> = envelope.proofs.iter().map(|proof| proof.to_vec()).collect();
        let decode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        let header = binary::admit_transcript(
            &proofs[target],
            &decode,
            Some((omitted, [0; 20], ConcreteStorageKey([0; 32]))),
            true,
        )
        .unwrap();
        let mut frames = Vec::new();
        let mut offset = binary::HEADER_BYTES;
        while offset < proofs[target].len() {
            let (frame, next) = binary::frame_at(&proofs[target], offset, &decode).unwrap();
            frames.push(frame);
            offset = next;
        }
        let node = frames
            .iter_mut()
            .find(|frame| frame.operation == 0 && frame.column == 2)
            .unwrap();
        assert!(matches!(node.outcome, binary::Outcome::Node(_)));
        node.outcome = if history_error {
            binary::Outcome::Error(3)
        } else {
            binary::Outcome::End
        };
        let encode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        encode.charge(R::Records, frames.len() as u64).unwrap();
        let altered = binary::encode_transcript(header, &frames, &encode, 0, true).unwrap();
        proofs[target] = altered;
        let bytes = bundle::encode_profile(
            &envelope.pair.map(|row| row.to_vec()),
            &envelope.descriptors.map(|row| row.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfResidual,
        )
        .unwrap();
        let failure = replay_vrf_residual_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(failure.phase, ConcreteDiagnosticPhase::SecondConstructor);
        assert_eq!(failure.index, 12);
        assert_eq!(failure.error, ConcreteWitnessError::Proof);
        assert_eq!(failure.usage.used(R::PayloadTuples), 0);
        assert_eq!(
            failure.last_read.unwrap().outcome,
            if history_error {
                ConcreteDiagnosticOutcome::Error(3)
            } else {
                ConcreteDiagnosticOutcome::End
            }
        );
    }
}

fn residual_49f8_first(context: ConcreteDiagnosticPlan) -> ConcreteVrfPlan {
    let mut voters = std::array::from_fn(|i| [i as u8 + 1; 20]);
    voters[2] = ConcreteVrfResidualPlan::VOTER;
    voters[17] = ConcreteVrfResidual49f8Plan::VOTER;
    voters[18] = ConcreteVrfResidualPlan::B92A;
    voters[19] = ConcreteVrfResidualPlan::E50B;
    ConcreteVrfPlan::new(context, voters).unwrap()
}

#[test]
fn residual_49f8_exact_shape_key_rcv4_and_old_profiles_fail_closed() {
    let f = Fixture::empty();
    let p = ConcreteVrfResidual49f8Plan::new(residual_49f8_first(plan(codec::empty_trie_root())))
        .unwrap();
    assert_eq!(
        hex::encode(p.voter()),
        "49f85534bbcf461d44fd3fc96feecf9abb6d38ad"
    );
    let selected: Vec<_> = (1..48).filter(|i| ![12, 45].contains(i)).collect();
    assert_eq!(selected.len(), 45);
    assert_eq!(p.context().delayed.len(), 48);
    let preimage = hex::decode("000449f85534bbcf461d44fd3fc96feecf9abb6d38ad").unwrap();
    for (tuple, delayed) in p.tuples().iter().zip(selected) {
        assert_eq!(tuple.identity, p.context().delayed[delayed]);
        assert_eq!(tuple.key.0, codec::keccak256(&preimage));
        assert_eq!(
            hex::encode(tuple.key.0),
            "446a40c15b86d1effc8e92cf6bb7db72c56131bd9491c6a6bcb70cd0c0bac9f2"
        );
        let mut account = [0; 20];
        account[19] = 0xfe;
        assert_eq!(tuple.address, account);
    }
    let captured = capture_vrf_residual_49f8_run(
        &f.application,
        &f.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert_eq!(
        &captured.bundle[..16],
        &[82, 67, 86, 52, 1, 0, 0, 0, 4, 0, 2, 0, 51, 0, 45, 0]
    );
    let valid =
        replay_vrf_residual_49f8_bundle(&captured.bundle, &p, ConcreteWitnessLimits::default())
            .unwrap();
    assert_eq!(valid.usage.used(R::PayloadTuples), 45);
    assert_eq!(valid.usage.used(R::Registrations), 51);
    let old = [
        ConcreteVrfResidualPlan::new(residual_49f8_first(plan(codec::empty_trie_root()))).unwrap(),
        ConcreteVrfResidualPlan::new_b92a(residual_49f8_first(plan(codec::empty_trie_root())))
            .unwrap(),
        ConcreteVrfResidualPlan::new_e50b(residual_49f8_first(plan(codec::empty_trie_root())))
            .unwrap(),
    ];
    for prior in &old {
        let bytes = capture_vrf_residual_run(
            &f.application,
            &f.state,
            prior,
            ConcreteWitnessLimits::default(),
        )
        .unwrap();
        let fail =
            replay_vrf_residual_49f8_bundle(&bytes.bundle, &p, ConcreteWitnessLimits::default())
                .err()
                .unwrap();
        assert_eq!(fail.phase, ConcreteDiagnosticPhase::Admission);
        assert_eq!(fail.usage.used(R::PayloadTuples), 0);
        let fail =
            replay_vrf_residual_bundle(&captured.bundle, prior, ConcreteWitnessLimits::default())
                .err()
                .unwrap();
        assert_eq!(fail.phase, ConcreteDiagnosticPhase::Admission);
        assert_eq!(fail.usage.used(R::PayloadTuples), 0);
    }
    let complete = ConcreteVrfVoterPlan::new(
        residual_49f8_first(plan(codec::empty_trie_root())),
        p.voter(),
    )
    .unwrap();
    let bytes = capture_vrf_voter_run(
        &f.application,
        &f.state,
        &complete,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    assert!(
        replay_vrf_residual_49f8_bundle(&bytes.bundle, &p, ConcreteWitnessLimits::default())
            .is_err()
    );
    assert!(
        replay_vrf_voter_bundle(
            &captured.bundle,
            &complete,
            ConcreteWitnessLimits::default()
        )
        .is_err()
    );
    let admission = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    admission.admit_input(captured.bundle.len()).unwrap();
    let env = bundle::admit_profile::<96>(
        &captured.bundle,
        &admission,
        bundle::Profile::VrfResidual49f8,
    )
    .unwrap();
    for offset in [8, 16, 48, 68, usize::MAX] {
        let mut proofs: Vec<_> = env.proofs.iter().map(|v| v.to_vec()).collect();
        if offset == usize::MAX {
            proofs.swap(51, 52);
        } else {
            proofs[51][offset] ^= 1;
        }
        let altered = bundle::encode_profile(
            &env.pair.map(|v| v.to_vec()),
            &env.descriptors.map(|v| v.to_vec()),
            &proofs,
            &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            bundle::Profile::VrfResidual49f8,
        )
        .unwrap();
        let fail = replay_vrf_residual_49f8_bundle(&altered, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(fail.phase, ConcreteDiagnosticPhase::Payload);
        assert_eq!(fail.index, 0);
        assert_eq!(fail.usage.used(R::PayloadTuples), 0);
    }
    for index in [0, 12, 45] {
        let mut wrong = plan(codec::empty_trie_root());
        wrong.delayed[index].state_root = [7; 32];
        wrong.tuples[index].identity = wrong.delayed[index];
        wrong.tuples[index + 48].identity = wrong.delayed[index];
        let wrong = ConcreteVrfResidual49f8Plan::new(residual_49f8_first(wrong)).unwrap();
        let fail = replay_vrf_residual_49f8_bundle(
            &captured.bundle,
            &wrong,
            ConcreteWitnessLimits::default(),
        )
        .err()
        .unwrap();
        assert_eq!(fail.phase, ConcreteDiagnosticPhase::SecondConstructor);
        assert_eq!(fail.index, index as u16);
        assert_eq!(fail.usage.used(R::PayloadTuples), 0);
    }
}
#[test]
fn residual_49f8_missing_voter_malformed_context_caps_and_eof() {
    assert!(
        ConcreteVrfResidual49f8Plan::new(residual_first(plan(codec::empty_trie_root()))).is_err()
    );
    let mut malformed = residual_49f8_first(plan(codec::empty_trie_root()));
    malformed.context.delayed.swap(11, 12);
    assert!(ConcreteVrfResidual49f8Plan::new(malformed).is_err());
    let fixture = Fixture::with_residual_member(vec![9; 32]);
    let p =
        ConcreteVrfResidual49f8Plan::new(residual_49f8_first(plan(fixture.plan.head.state_root)))
            .unwrap();
    for resource in [
        R::KeyBytes,
        R::RowBytes,
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::EncodedBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::ConstructorIdentities,
        R::Registrations,
        R::DistinctIdentities,
        R::PayloadTuples,
    ] {
        let failed = capture_vrf_residual_49f8_run(
            &fixture.application,
            &fixture.state,
            &p,
            ConcreteWitnessLimits::default()
                .lowered(resource, 0)
                .unwrap(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(failed.error, ConcreteWitnessError::LimitExceeded { resource: r, .. } if r == resource)
        );
        assert_eq!(failed.usage.used(resource), 0);
    }
    let captured = capture_vrf_residual_49f8_run(
        &fixture.application,
        &fixture.state,
        &p,
        ConcreteWitnessLimits::default(),
    )
    .unwrap();
    for length in [
        0,
        15,
        16,
        captured.bundle.len() / 2,
        captured.bundle.len() - 1,
    ] {
        assert!(
            replay_vrf_residual_49f8_bundle(
                &captured.bundle[..length],
                &p,
                ConcreteWitnessLimits::default()
            )
            .is_err()
        );
    }
    let mut trailing = captured.bundle.clone();
    trailing.push(0);
    assert!(
        replay_vrf_residual_49f8_bundle(&trailing, &p, ConcreteWitnessLimits::default()).is_err()
    );
}
#[test]
fn residual_49f8_both_omitted_periods_history_and_missing_node_stop_before_payload() {
    for omitted_index in [12, 45] {
        let fixture = Fixture::with_residual_member(vec![9; 32]);
        let plan = ConcreteVrfResidual49f8Plan::new(residual_49f8_first(self::plan(
            fixture.plan.head.state_root,
        )))
        .unwrap();
        let omitted = plan.context().delayed[omitted_index];
        assert!(plan.tuples().iter().all(|tuple| tuple.identity != omitted));
        let captured = capture_vrf_residual_49f8_run(
            &fixture.application,
            &fixture.state,
            &plan,
            ConcreteWitnessLimits::default(),
        )
        .unwrap();
        let admission = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        admission.admit_input(captured.bundle.len()).unwrap();
        let envelope = bundle::admit_profile::<96>(
            &captured.bundle,
            &admission,
            bundle::Profile::VrfResidual49f8,
        )
        .unwrap();
        // The two first-constructor proofs precede all 48 delayed identities.
        let target = 2 + omitted_index;
        for history_error in [true, false] {
            let mut proofs: Vec<_> = envelope.proofs.iter().map(|proof| proof.to_vec()).collect();
            let decode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
            let header = binary::admit_transcript(
                &proofs[target],
                &decode,
                Some((omitted, [0; 20], ConcreteStorageKey([0; 32]))),
                true,
            )
            .unwrap();
            let mut frames = Vec::new();
            let mut offset = binary::HEADER_BYTES;
            while offset < proofs[target].len() {
                let (frame, next) = binary::frame_at(&proofs[target], offset, &decode).unwrap();
                frames.push(frame);
                offset = next;
            }
            let node = frames
                .iter_mut()
                .find(|frame| frame.operation == 0 && frame.column == 2)
                .unwrap();
            assert!(matches!(node.outcome, binary::Outcome::Node(_)));
            node.outcome = if history_error {
                binary::Outcome::Error(3)
            } else {
                binary::Outcome::End
            };
            let encode = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
            encode.charge(R::Records, frames.len() as u64).unwrap();
            let altered = binary::encode_transcript(header, &frames, &encode, 0, true).unwrap();
            proofs[target] = altered;
            let bytes = bundle::encode_profile(
                &envelope.pair.map(|row| row.to_vec()),
                &envelope.descriptors.map(|row| row.to_vec()),
                &proofs,
                &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
                bundle::Profile::VrfResidual49f8,
            )
            .unwrap();
            let failure =
                replay_vrf_residual_49f8_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
                    .err()
                    .unwrap();
            assert_eq!(failure.phase, ConcreteDiagnosticPhase::SecondConstructor);
            assert_eq!(failure.index, omitted_index as u16);
            assert_eq!(failure.error, ConcreteWitnessError::Proof);
            assert_eq!(failure.usage.used(R::PayloadTuples), 0);
            assert_eq!(
                failure.last_read.unwrap().outcome,
                if history_error {
                    ConcreteDiagnosticOutcome::Error(3)
                } else {
                    ConcreteDiagnosticOutcome::End
                }
            );
        }
    }
}
