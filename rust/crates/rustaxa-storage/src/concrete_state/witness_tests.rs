use super::*;
use binary::{Frame, Header, Outcome, encode as encoded};
use rlp::RlpStream;

struct Fixture {
    id: ConcreteStateIdentity,
    address: [u8; 20],
    key: ConcreteStorageKey,
    nodes: Vec<(u8, [u8; 32], Vec<u8>)>,
    rows: Vec<(u8, [u8; 40], Vec<u8>)>,
}
struct Recorder<'a> {
    fixture: &'a Fixture,
    budget: ConcreteWitnessBudget,
    records: RefCell<Vec<Frame<'a>>>,
    attempts: Cell<u64>,
    canonical_charges: Cell<u64>,
}
impl Recorder<'_> {
    fn failure(&self, e: ConcreteWitnessError) -> ConcreteReadError {
        self.budget.stop(e);
        codec::corrupt("synthetic witness stopped")
    }
}
impl<'a> Recorder<'a> {
    fn push(
        &self,
        operation: u8,
        column: u8,
        request: [u8; 32],
        outcome: Outcome<'a>,
    ) -> Result<(), ConcreteReadError> {
        self.budget
            .charge(R::Records, 1)
            .map_err(|e| self.failure(e))?;
        let mut records = self.records.borrow_mut();
        if records.len() == records.capacity() {
            self.budget
                .charge(
                    R::CanonicalBytes,
                    ((records.len() + 1) * std::mem::size_of::<Frame<'_>>()) as u64,
                )
                .map_err(|e| self.failure(e))?;
            records
                .try_reserve_exact(1)
                .map_err(|_| self.failure(ConcreteWitnessError::Allocation))?;
        }
        let ordinal = records.len() as u64;
        records.push(Frame {
            ordinal,
            identity: self.fixture.id,
            operation,
            column,
            request,
            period: self.fixture.id.period.as_u64(),
            outcome,
        });
        Ok(())
    }
    fn operation(&self) -> Result<(), ConcreteReadError> {
        self.budget
            .charge(R::Operations, 1)
            .map_err(|e| self.failure(e))?;
        self.attempts.set(self.attempts.get() + 1);
        Ok(())
    }
}
impl PhysicalTrieStore for Recorder<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.fixture.id
    }
    fn allocation_error(&self) -> ConcreteReadError {
        self.failure(ConcreteWitnessError::Allocation)
    }
    fn visit_budget(&self, n: usize) -> Result<(), ConcreteReadError> {
        self.budget
            .charge(R::RowBytes, n as u64)
            .and_then(|()| self.budget.charge(R::Visits, 1))
            .map_err(|e| self.failure(e))
    }
    fn canonical_budget(&self, n: usize) -> Result<(), ConcreteReadError> {
        self.budget
            .charge(R::CanonicalBytes, n as u64)
            .map_err(|e| self.failure(e))?;
        self.canonical_charges.set(self.canonical_charges.get() + 1);
        Ok(())
    }
    fn account_commitment(&self, b: &[u8]) -> Result<Vec<u8>, ConcreteReadError> {
        codec::account_view_commitment(b, |n| self.canonical_budget(n), || self.allocation_error())
    }
    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        self.operation()?;
        let column: u8 = column.parse().unwrap();
        let value = self
            .fixture
            .nodes
            .iter()
            .find(|(c, h, _)| *c == column && *h == hash)
            .map(|(_, _, b)| b.as_slice());
        if let Some(b) = value {
            self.budget
                .charge(R::RowBytes, b.len() as u64)
                .map_err(|e| self.failure(e))?;
        }
        self.push(0, column, hash, value.map_or(Outcome::End, Outcome::Node))?;
        value
            .map(|b| self.budget.copy(b).map_err(|e| self.failure(e)))
            .transpose()
    }
    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        self.operation()?;
        let column: u8 = column.parse().unwrap();
        let seek = codec::versioned_key(prefix, period);
        let row = self
            .fixture
            .rows
            .iter()
            .filter(|(c, k, _)| *c == column && *k <= seek)
            .max_by_key(|(_, k, _)| *k);
        if let Some((_, k, v)) = row {
            self.budget
                .charge(R::KeyBytes, k.len() as u64)
                .and_then(|()| self.budget.charge(R::RowBytes, v.len() as u64))
                .map_err(|e| self.failure(e))?;
            self.push(1, column, prefix, Outcome::Row(k, v))?;
            if k.starts_with(&prefix) {
                return self
                    .budget
                    .copy(v)
                    .map(|value| Some(SelectedVersion { value }))
                    .map_err(|e| self.failure(e));
            }
        } else {
            self.push(1, column, prefix, Outcome::End)?;
        }
        Ok(None)
    }
}

fn leaf(compact: &[u8], commitment: &[u8]) -> Vec<u8> {
    let mut r = RlpStream::new_list(2);
    r.append(&compact);
    r.append(&commitment);
    r.out().to_vec()
}
fn account(storage: Option<[u8; 32]>) -> Vec<u8> {
    let mut r = RlpStream::new_list(5);
    r.append_empty_data();
    r.append(&vec![1_u8]);
    if let Some(root) = storage {
        r.append(&root.as_slice());
    } else {
        r.append_empty_data();
    }
    r.append_empty_data();
    r.append(&0_u64);
    r.out().to_vec()
}
fn fixture(member: bool, embedded: bool) -> Fixture {
    let period = FinalChainBlockNumber::new(10);
    let address = [7; 20];
    let key = ConcreteStorageKey([4; 32]);
    let path = codec::storage_trie_path(key);
    let mut nodes = Vec::new();
    let mut rows = Vec::new();
    let value = vec![9_u8];
    let storage_root = if !member {
        codec::empty_trie_root()
    } else if !embedded {
        let mut compact = vec![0x20];
        compact.extend_from_slice(&path);
        let canonical = leaf(&compact, &rlp::encode(&value));
        let root = codec::keccak256(&canonical);
        nodes.push((4, root, leaf(&compact, &root)));
        rows.push((
            5,
            codec::versioned_key(codec::storage_version_prefix(address, key), period),
            value,
        ));
        root
    } else {
        let target = path[31] & 15;
        let sibling = target ^ 1;
        let a = leaf(&[0x20], &rlp::encode(&value));
        let other = vec![8_u8];
        let b = leaf(&[0x20], &rlp::encode(&other));
        let pa = leaf(&[0x20], &codec::keccak256(&a));
        let pb = leaf(&[0x20], &codec::keccak256(&b));
        let mut canonical = RlpStream::new_list(17);
        let mut physical = RlpStream::new_list(16);
        for n in 0..16 {
            if n == target {
                canonical.append_raw(&a, 1);
                physical.append_raw(&pa, 1);
            } else if n == sibling {
                canonical.append_raw(&b, 1);
                physical.append_raw(&pb, 1);
            } else {
                canonical.append_empty_data();
                physical.append_empty_data();
            }
        }
        canonical.append_empty_data();
        let canonical = canonical.out();
        let physical = physical.out();
        assert!(canonical.len() < 32);
        let mut compact = vec![0x10 | (path[0] >> 4)];
        for i in 0..31 {
            compact.push((path[i] & 15) << 4 | (path[i + 1] >> 4));
        }
        let mut c = RlpStream::new_list(2);
        c.append(&compact);
        c.append_raw(&canonical, 1);
        let root = codec::keccak256(&c.out());
        let mut p = RlpStream::new_list(2);
        p.append(&compact);
        p.append_raw(&physical, 1);
        nodes.push((4, root, p.out().to_vec()));
        rows.push((
            5,
            codec::versioned_key(codec::storage_version_prefix(address, key), period),
            value,
        ));
        let mut other_path = path;
        other_path[31] = (other_path[31] & 0xf0) | sibling;
        rows.push((
            5,
            codec::versioned_key(codec::storage_prefix_for_path(address, other_path), period),
            other,
        ));
        root
    };
    let raw = account(Some(storage_root));
    let old =
        codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap()).unwrap();
    let prefix = codec::account_version_prefix(address);
    let mut compact = vec![0x20];
    compact.extend_from_slice(&prefix);
    let canonical = leaf(&compact, &old);
    let state_root = codec::keccak256(&canonical);
    nodes.push((2, state_root, leaf(&compact, &state_root)));
    rows.push((3, codec::versioned_key(prefix, period), raw));
    Fixture {
        id: ConcreteStateIdentity { period, state_root },
        address,
        key,
        nodes,
        rows,
    }
}
fn recorder(f: &Fixture, limits: ConcreteWitnessLimits) -> Recorder<'_> {
    let budget = ConcreteWitnessBudget::new(limits);
    budget.charge(R::CanonicalBytes, 1024).unwrap();
    Recorder {
        fixture: f,
        budget,
        records: RefCell::new(Vec::new()),
        attempts: Cell::new(0),
        canonical_charges: Cell::new(0),
    }
}
fn capture(f: &Fixture) -> (Vec<u8>, ConcreteWitnessUsage) {
    let r = recorder(f, ConcreteWitnessLimits::default());
    r.budget.tuple(f.id, f.address, f.key).unwrap();
    let (raw, proof) = tuple_proof(&r, f.address, f.key).unwrap();
    reconcile(f.id, raw, proof).unwrap();
    let bytes = encoded(
        Header {
            identity: f.id,
            address: f.address,
            key: f.key,
            records: r.records.borrow().len() as u32,
        },
        &r.records.borrow(),
        &r.budget,
    )
    .unwrap();
    (bytes, r.budget.usage())
}
fn replay(f: &Fixture, bytes: &[u8]) -> Result<ConcreteWitnessReplay, ConcreteWitnessError> {
    replay_storage_witness(
        bytes,
        f.id,
        f.address,
        f.key,
        ConcreteWitnessLimits::default(),
    )
}

#[test]
fn full_account_storage_capture_replay_and_embedded_siblings() {
    for embedded in [false, true] {
        let f = fixture(true, embedded);
        let (bytes, capture_usage) = capture(&f);
        let result = replay(&f, &bytes).unwrap();
        assert_eq!(result.physical, ConcreteWitnessPhysical::Present(vec![9]));
        assert_eq!(result.logical, ConcreteWitnessLogical::Member(vec![9]));
        assert!(capture_usage.used(R::EncodedBytes) > 0);
        assert_eq!(result.usage.admitted_input_bytes, bytes.len() as u64);
        assert_eq!(result.usage.used(R::EncodedBytes), 0);
        assert_eq!(
            result.usage.used(R::Operations),
            capture_usage.used(R::Operations)
        );
        assert!(
            result.usage.used(R::RawCopyBytes) > 0
                && result.usage.used(R::CanonicalBytes) > 1024
                && result.usage.used(R::Visits) > 0
        );
        let mut changed = f.id;
        changed.period = FinalChainBlockNumber::new(11);
        assert_eq!(
            replay_storage_witness(
                &bytes,
                changed,
                f.address,
                f.key,
                ConcreteWitnessLimits::default()
            )
            .unwrap_err(),
            ConcreteWitnessError::Identity
        );
        let mut bad = bytes.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(replay(&f, &bad).is_err());
    }
}
#[test]
fn empty_storage_nonmember_unavailable_and_tombstone() {
    let mut f = fixture(false, false);
    let (bytes, _) = capture(&f);
    let result = replay(&f, &bytes).unwrap();
    assert_eq!(
        result.physical,
        ConcreteWitnessPhysical::HistoryUnavailable(f.id)
    );
    assert_eq!(result.logical, ConcreteWitnessLogical::NonMember);
    f.rows.push((
        5,
        codec::versioned_key(codec::storage_version_prefix(f.address, f.key), f.id.period),
        vec![],
    ));
    let (bytes, _) = capture(&f);
    assert_eq!(
        replay(&f, &bytes).unwrap().physical,
        ConcreteWitnessPhysical::Tombstone
    );
}
#[test]
fn exact_byte_operation_visit_limits_and_sticky_failure() {
    let f = fixture(true, true);
    let (bytes, _) = capture(&f);
    let actual = replay(&f, &bytes).unwrap().usage;
    for resource in [
        R::RawCopyBytes,
        R::CanonicalBytes,
        R::Operations,
        R::Visits,
        R::Records,
        R::FrameInspections,
        R::KeyBytes,
        R::RowBytes,
    ] {
        let limit = actual.used(resource);
        assert!(limit > 0);
        let limits = ConcreteWitnessLimits::default()
            .lowered(resource, limit)
            .unwrap();
        assert!(
            replay_storage_witness(&bytes, f.id, f.address, f.key, limits).is_ok(),
            "{resource:?}"
        );
        let limits = limits.lowered(resource, limit - 1).unwrap();
        assert!(
            matches!(replay_storage_witness(&bytes,f.id,f.address,f.key,limits),Err(ConcreteWitnessError::LimitExceeded {resource:r,..}) if r==resource)
        );
    }
    let budget = ConcreteWitnessBudget::new(
        ConcreteWitnessLimits::default()
            .lowered(R::Operations, 0)
            .unwrap(),
    );
    assert!(budget.charge(R::Operations, 1).is_err());
    assert!(budget.charge(R::RawCopyBytes, 1).is_err());
    assert_eq!(budget.usage().used(R::RawCopyBytes), 0);
    let r = recorder(
        &f,
        ConcreteWitnessLimits::default()
            .lowered(R::Operations, 1)
            .unwrap(),
    );
    assert!(tuple_proof(&r, f.address, f.key).is_err());
    assert_eq!(r.attempts.get(), 1);
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.state.borrow_mut().usage.counters[R::RawCopyBytes as usize] = u64::MAX;
    assert_eq!(
        budget.charge(R::RawCopyBytes, 1).unwrap_err(),
        ConcreteWitnessError::Overflow
    );
}
#[test]
fn shared_constructor_registration_and_tuple_guards() {
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let id = |n| ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(n),
        state_root: [1; 32],
    };
    budget.register_constructor(&[id(100), id(99)]).unwrap();
    let payload: Vec<_> = std::iter::once(id(100)).chain((0..48).map(id)).collect();
    budget.register_constructor(&payload).unwrap();
    assert_eq!(budget.usage().used(R::Registrations), 51);
    assert_eq!(budget.usage().used(R::DistinctIdentities), 50);
    for _ in 0..4 {
        budget.charge(R::Operations, 1).unwrap();
    }
    budget
        .tuple(id(1), [1; 20], ConcreteStorageKey([1; 32]))
        .unwrap();
    assert!(
        budget
            .tuple(id(1), [1; 20], ConcreteStorageKey([1; 32]))
            .is_err()
    );
    assert_eq!(budget.usage().used(R::Operations), 4);
    let budget = ConcreteWitnessBudget::new(
        ConcreteWitnessLimits::default()
            .lowered(R::Registrations, 50)
            .unwrap(),
    );
    budget.register_constructor(&[id(100), id(99)]).unwrap();
    assert!(budget.register_constructor(&payload).is_err());
}

fn admitted_frames(bytes: &[u8]) -> Vec<Frame<'_>> {
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let h = binary::admit(bytes, &budget, None).unwrap();
    let mut offset = binary::HEADER_BYTES;
    (0..h.records)
        .map(|_| {
            let (f, next) = binary::frame_at(bytes, offset, &budget).unwrap();
            offset = next;
            f
        })
        .collect()
}
fn reframe(f: &Fixture, frames: &[Frame<'_>]) -> Result<Vec<u8>, ConcreteWitnessError> {
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.charge(R::Records, frames.len() as u64).unwrap();
    encoded(
        Header {
            identity: f.id,
            address: f.address,
            key: f.key,
            records: frames.len() as u32,
        },
        frames,
        &budget,
    )
}
#[test]
fn strict_schema_order_request_unused_conflicts_and_identity_admission() {
    let f = fixture(true, true);
    let (bytes, _) = capture(&f);
    for (offset, value) in [(0, b'X'), (4, 2), (6, 1), (156, 9), (157, 9), (198, 9)] {
        let mut bad = bytes.clone();
        bad[offset] = value;
        assert!(replay(&f, &bad).is_err(), "offset {offset}");
    }
    for cut in [0, 4, 103, 104, bytes.len() - 1] {
        assert!(replay(&f, &bytes[..cut]).is_err());
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(replay(&f, &bad).is_err());
    let mut frames = admitted_frames(&bytes);
    frames.swap(0, 1);
    assert!(reframe(&f, &frames).is_err());
    let mut frames = admitted_frames(&bytes);
    frames[0].request[0] ^= 1;
    assert_eq!(
        replay(&f, &reframe(&f, &frames).unwrap()).unwrap_err(),
        ConcreteWitnessError::Request
    );
    let mut frames = admitted_frames(&bytes);
    let mut extra = admitted_frames(&bytes).remove(0);
    extra.ordinal = frames.len() as u64;
    extra.request = [0; 32];
    frames.push(extra);
    assert_eq!(
        replay(&f, &reframe(&f, &frames).unwrap()).unwrap_err(),
        ConcreteWitnessError::UnusedRecords
    );
    let mut frames = admitted_frames(&bytes);
    let duplicate = frames
        .iter()
        .enumerate()
        .find(|(i, x)| {
            frames[..*i].iter().any(|p| {
                p.operation == x.operation && p.column == x.column && p.request == x.request
            })
        })
        .map(|(i, _)| i)
        .unwrap();
    frames[duplicate].outcome = Outcome::End;
    assert_eq!(
        reframe(&f, &frames).unwrap_err(),
        ConcreteWitnessError::Conflict
    );
    let mut wrong = f.id;
    wrong.state_root[0] ^= 1;
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    assert!(
        ConcreteStorageWitness::admit_budget(&bytes, &budget, Some((wrong, f.address, f.key)))
            .is_err()
    );
    assert_eq!(budget.usage().used(R::FrameInspections), 0);
    for (address, key) in [([8; 20], f.key), (f.address, ConcreteStorageKey([5; 32]))] {
        assert_eq!(
            replay_storage_witness(&bytes, f.id, address, key, ConcreteWitnessLimits::default())
                .unwrap_err(),
            ConcreteWitnessError::Identity
        );
    }
}
#[test]
fn raw_selected_history_and_reconciliation_are_separate() {
    let f = fixture(true, false);
    let (bytes, _) = capture(&f);
    let mut frames = admitted_frames(&bytes);
    frames[0].outcome = Outcome::End;
    assert_eq!(
        reframe(&f, &frames).unwrap_err(),
        ConcreteWitnessError::Conflict
    );
    let mut frames = admitted_frames(&bytes);
    let Outcome::Row(key, value) = frames[0].outcome else {
        panic!()
    };
    let mut newer = key.to_vec();
    newer[39] = 11;
    let target = frames[0].request;
    for frame in frames
        .iter_mut()
        .filter(|x| x.operation == 1 && x.column == 5 && x.request == target)
    {
        frame.outcome = Outcome::Row(&newer, value);
    }
    assert_eq!(
        replay(&f, &reframe(&f, &frames).unwrap()).unwrap_err(),
        ConcreteWitnessError::Request
    );
    let mut frames = admitted_frames(&bytes);
    let target = frames[0].request;
    for frame in frames
        .iter_mut()
        .filter(|x| x.operation == 1 && x.column == 5 && x.request == target)
    {
        frame.outcome = Outcome::Row(&key[..32], value);
    }
    assert_eq!(
        replay(&f, &reframe(&f, &frames).unwrap()).unwrap_err(),
        ConcreteWitnessError::Schema
    );
    let mut frames = admitted_frames(&bytes);
    frames[0].outcome = Outcome::Row(key, &[10]);
    assert!(reframe(&f, &frames).is_err() || replay(&f, &reframe(&f, &frames).unwrap()).is_err());
    let f = fixture(false, false);
    let (bytes, _) = capture(&f);
    let mut frames = admitted_frames(&bytes);
    frames[0].outcome = Outcome::Error(3);
    assert_eq!(
        replay(&f, &reframe(&f, &frames).unwrap()).unwrap().logical,
        ConcreteWitnessLogical::NonMember
    );
    frames[0].outcome = Outcome::Error(0);
    assert_eq!(
        replay(&f, &reframe(&f, &frames).unwrap()).unwrap_err(),
        ConcreteWitnessError::Read(0)
    );
    let mut wrong = f.id;
    wrong.period = FinalChainBlockNumber::new(11);
    assert_eq!(
        reconcile(
            f.id,
            ConcreteWitnessPhysical::HistoryUnavailable(wrong),
            PathProof::NonMember
        )
        .unwrap_err(),
        ConcreteWitnessError::Reconciliation
    );
    assert_eq!(
        reconcile(
            f.id,
            ConcreteWitnessPhysical::Tombstone,
            PathProof::Member(vec![1])
        )
        .unwrap_err(),
        ConcreteWitnessError::Reconciliation
    );
    assert_eq!(
        reconcile(
            f.id,
            ConcreteWitnessPhysical::Present(vec![1]),
            PathProof::NonMember
        )
        .unwrap_err(),
        ConcreteWitnessError::Reconciliation
    );
}
#[test]
fn absence_account_empty_roots_divergent_path_and_required_dependency_failures() {
    let mut f = fixture(false, false);
    f.id.state_root = codec::empty_trie_root();
    f.rows.clear();
    f.nodes.clear();
    let (bytes, _) = capture(&f);
    assert_eq!(
        replay(&f, &bytes).unwrap().logical,
        ConcreteWitnessLogical::NonMember
    );
    let mut f = fixture(false, false);
    let raw = account(None);
    let old =
        codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap()).unwrap();
    let prefix = codec::account_version_prefix(f.address);
    let mut compact = vec![0x20];
    compact.extend_from_slice(&prefix);
    let root = codec::keccak256(&leaf(&compact, &old));
    f.id.state_root = root;
    f.nodes = vec![(2, root, leaf(&compact, &root))];
    f.rows = vec![(3, codec::versioned_key(prefix, f.id.period), raw)];
    let (bytes, _) = capture(&f);
    assert_eq!(
        replay(&f, &bytes).unwrap().logical,
        ConcreteWitnessLogical::NonMember
    );
    let f = fixture(true, true);
    let (bytes, _) = capture(&f);
    for index in 0..admitted_frames(&bytes).len() {
        let mut frames = admitted_frames(&bytes);
        if frames[index].operation == 0 {
            frames[index].outcome = Outcome::End;
            if let Ok(b) = reframe(&f, &frames) {
                assert!(replay(&f, &b).is_err());
            }
        }
    }
    // Query a different slot through the same real root, without inventing a
    // storage root or ignoring the divergent embedded sibling value.
    let mut f = fixture(true, true);
    f.key = ConcreteStorageKey([5; 32]);
    let (bytes, _) = capture(&f);
    assert_eq!(
        replay(&f, &bytes).unwrap().logical,
        ConcreteWitnessLogical::NonMember
    );
    let f = fixture(true, true);
    let r = recorder(&f, ConcreteWitnessLimits::default());
    r.budget.tuple(f.id, f.address, f.key).unwrap();
    tuple_proof(&r, f.address, f.key).unwrap();
    let mut frames = r.records.borrow_mut();
    let sibling = frames
        .iter()
        .position(|x| {
            x.operation == 1
                && x.column == 5
                && x.request != codec::storage_version_prefix(f.address, f.key)
        })
        .unwrap();
    frames[sibling].outcome = Outcome::End;
    let bytes = reframe(&f, &frames).unwrap();
    assert!(replay(&f, &bytes).is_err());
}
#[test]
fn output_preflight_borrowed_widths_account_codec_parity_and_allocation_error() {
    let f = fixture(true, false);
    let (bytes, _) = capture(&f);
    let mut frames = admitted_frames(&bytes);
    let huge = vec![0; 1_048_577];
    frames[0].outcome = Outcome::Row(&[0; 65], &huge);
    assert!(matches!(
        reframe(&f, &frames),
        Err(ConcreteWitnessError::LimitExceeded {
            resource: R::KeyBytes,
            ..
        })
    ));
    frames[0].outcome = Outcome::Row(&[0; 40], &huge);
    assert!(matches!(
        reframe(&f, &frames),
        Err(ConcreteWitnessError::LimitExceeded {
            resource: R::RowBytes,
            ..
        })
    ));
    let limit = bytes.len() as u64;
    let policy = ConcreteWitnessLimits::default()
        .lowered(R::EncodedBytes, limit)
        .unwrap();
    assert!(replay_storage_witness(&bytes, f.id, f.address, f.key, policy).is_ok());
    assert!(
        replay_storage_witness(
            &bytes,
            f.id,
            f.address,
            f.key,
            policy.lowered(R::EncodedBytes, limit - 1).unwrap()
        )
        .is_err()
    );
    let r = recorder(&f, ConcreteWitnessLimits::default());
    r.budget.tuple(f.id, f.address, f.key).unwrap();
    tuple_proof(&r, f.address, f.key).unwrap();
    let limits = ConcreteWitnessLimits::default()
        .lowered(R::EncodedBytes, limit - 1)
        .unwrap();
    let b = ConcreteWitnessBudget::new(limits);
    b.charge(R::Records, r.records.borrow().len() as u64)
        .unwrap();
    assert!(
        encoded(
            Header {
                identity: f.id,
                address: f.address,
                key: f.key,
                records: r.records.borrow().len() as u32
            },
            &r.records.borrow(),
            &b
        )
        .is_err()
    );
    assert_eq!(b.usage().used(R::EncodedBytes), 0);
    for raw in [account(None), account(Some([3; 32]))] {
        let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        let actual = codec::account_view_commitment(
            &raw,
            |n| {
                budget
                    .charge(R::CanonicalBytes, n as u64)
                    .map_err(|_| codec::corrupt("limit"))
            },
            || codec::corrupt("allocation"),
        )
        .unwrap();
        assert_eq!(
            actual,
            codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap()).unwrap()
        );
        assert_eq!(budget.usage().used(R::CanonicalBytes), actual.len() as u64);
    }
    let r = recorder(&f, ConcreteWitnessLimits::default());
    r.allocation_error();
    assert_eq!(r.budget.error(), Some(ConcreteWitnessError::Allocation));
    assert!(r.node("2", f.id.state_root).is_err());
    assert_eq!(r.attempts.get(), 0);
    let mut f = fixture(false, false);
    let malformed = leaf(&[0x20; 34], &[1]);
    f.id.state_root = codec::keccak256(&malformed);
    f.nodes = vec![(2, f.id.state_root, malformed)];
    let r = recorder(&f, ConcreteWitnessLimits::default());
    assert!(tuple_proof(&r, f.address, f.key).is_err());
    // Metadata charges remain separate from canonical payload allocations.
    assert_eq!(r.canonical_charges.get(), 0);
}

#[test]
fn pinned_go_storage_root_with_authenticated_account_wrapper() {
    // Same external Go node_history[2] commitment as the unchanged path test.
    let mut f = fixture(false, false);
    f.nodes.retain(|(c, _, _)| *c != 4);
    f.rows.retain(|(c, _, _)| *c != 5);
    let hash = |s: &str| -> [u8; 32] { hex::decode(s).unwrap().try_into().unwrap() };
    let root = hash("acde2a8675590a1f104ae5db7c4ab5ef0be55ebdd63b6b63f0aa5474fd5620dc");
    for (h, n) in [
        (
            "11eec08482c5316b7562422fade7059df9794c5b5fc0eddfe52d985f6d00b146",
            "f843a1200100000000000000000000000000000000000000000000000000000000000001a011eec08482c5316b7562422fade7059df9794c5b5fc0eddfe52d985f6d00b146",
        ),
        (
            "1b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15",
            "f89680f842a02000000000000000000000000000000000000000000000000000000000000001a0f9aa104e4c11dab9f96b33feb13e7ef0d37e8bcf2d6920172bc6d1c60392167bf842a02000000000000000000000000000000000000000000000000000000000000002a0a9d2c714abcf26a6371624afc6baf84e67dd9287f8f1408b07b99a30d5ed2b6e80808080808080808080808080",
        ),
        (
            "5469ad332e5de42c6030829cfaad5080425441be8066edfaaf63d6bf90d06150",
            "e210a01b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15",
        ),
        (
            "acde2a8675590a1f104ae5db7c4ab5ef0be55ebdd63b6b63f0aa5474fd5620dc",
            "f873a01b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf158080808080808080808080808080f842a031000000000000000000000000000000000000000000000000000000000000f1a0593d9d0f1d5cc62870cc75287b06bc5c1cb3b78cc91bd783234ee39d9f6ede14",
        ),
    ] {
        f.nodes.push((4, hash(h), hex::decode(n).unwrap()));
    }
    for (path, value) in [
        (
            "0100000000000000000000000000000000000000000000000000000000000001",
            vec![1],
        ),
        (
            "0200000000000000000000000000000000000000000000000000000000000002",
            vec![0; 80],
        ),
        (
            "f1000000000000000000000000000000000000000000000000000000000000f1",
            vec![3],
        ),
    ] {
        f.rows.push((
            5,
            codec::versioned_key(
                codec::storage_prefix_for_path(f.address, hash(path)),
                f.id.period,
            ),
            value,
        ));
    }
    let raw = account(Some(root));
    let committed =
        codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap()).unwrap();
    let account_prefix = codec::account_version_prefix(f.address);
    let mut compact = vec![0x20];
    compact.extend_from_slice(&account_prefix);
    let account_root = codec::keccak256(&leaf(&compact, &committed));
    f.id.state_root = account_root;
    f.nodes.retain(|(c, _, _)| *c != 2);
    f.nodes
        .push((2, account_root, leaf(&compact, &account_root)));
    f.rows.retain(|(c, _, _)| *c != 3);
    f.rows
        .push((3, codec::versioned_key(account_prefix, f.id.period), raw));
    let (bytes, _) = capture(&f);
    assert_eq!(
        replay(&f, &bytes).unwrap().logical,
        ConcreteWitnessLogical::NonMember
    );
    // The off-path hashed root child remains unopened; embedded siblings have
    // real selected-version dependencies and reproduce the external commitment.
    let frames = admitted_frames(&bytes);
    assert!(!frames.iter().any(|x| x.operation == 0
        && x.column == 4
        && x.request == hash("1b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15")));
}
#[test]
fn borrowed_account_view_preserves_widths_and_all_routing_boundaries() {
    for (nonce, balance, storage, code, size) in [
        (vec![], vec![0, 0, 1], vec![2], vec![3], 0_u64),
        (vec![1; 128], vec![0; 4096], vec![], vec![], u64::MAX),
    ] {
        let mut r = RlpStream::new_list(5);
        r.append(&nonce);
        r.append(&balance);
        r.append(&storage);
        r.append(&code);
        r.append(&size);
        let bytes = r.out();
        let expected =
            codec::account_commitment_rlp(&codec::decode_physical_account(&bytes).unwrap())
                .unwrap();
        assert_eq!(
            codec::account_view_commitment(&bytes, |_| Ok(()), || codec::corrupt("allocation"))
                .unwrap(),
            expected
        );
    }
    let f = fixture(true, false);
    let (bytes, _) = capture(&f);
    let records = admitted_frames(&bytes);
    for resource in [R::Records, R::FrameInspections, R::RowBytes, R::KeyBytes] {
        let budget = ConcreteWitnessBudget::new(
            ConcreteWitnessLimits::default()
                .lowered(resource, 0)
                .unwrap(),
        );
        assert!(
            ConcreteStorageWitness::admit_budget(&bytes, &budget, Some((f.id, f.address, f.key)))
                .is_err()
        );
        assert_eq!(budget.usage().used(R::RawCopyBytes), 0);
        assert_eq!(budget.usage().used(R::CanonicalBytes), 0);
    }
    let id = |n| ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(n),
        state_root: [1; 32],
    };
    let payload: Vec<_> = std::iter::once(id(100)).chain((0..48).map(id)).collect();
    let budget = ConcreteWitnessBudget::new(
        ConcreteWitnessLimits::default()
            .lowered(R::DistinctIdentities, 49)
            .unwrap(),
    );
    budget.register_constructor(&[id(100), id(99)]).unwrap();
    assert!(budget.register_constructor(&payload).is_err());
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    assert!(
        budget
            .register_constructor(&(0..50).map(id).collect::<Vec<_>>())
            .is_err()
    );
    assert!(
        ConcreteWitnessLimits::default()
            .lowered(R::RowBytes, 1_048_577)
            .is_err()
    );
    let budget = ConcreteWitnessBudget::new(
        ConcreteWitnessLimits::default()
            .lowered(R::PayloadTuples, 1)
            .unwrap(),
    );
    budget.tuple(f.id, f.address, f.key).unwrap();
    assert!(budget.tuple(f.id, [8; 20], f.key).is_err());
    assert!(!records.is_empty());
    assert!(
        reconcile(
            f.id,
            ConcreteWitnessPhysical::Present(vec![1]),
            PathProof::Member(vec![2])
        )
        .is_err()
    );
}

#[test]
fn review_edges_unrelated_predecessor_empty_branch_and_shared_exhaustion() {
    let f = fixture(false, false);
    let (bytes, _) = capture(&f);
    let baseline = replay(&f, &bytes).unwrap();
    for key in [&[0_u8; 40][..], &[0_u8; 3][..]] {
        let mut frames = admitted_frames(&bytes);
        frames[0].outcome = Outcome::Row(key, &[99; 8]);
        let changed = reframe(&f, &frames).unwrap();
        let admitted = admitted_frames(&changed);
        let Outcome::Row(borrowed_key, borrowed_value) = admitted[0].outcome else {
            panic!()
        };
        assert!(borrowed_key.as_ptr() >= changed.as_ptr());
        assert!(borrowed_value.as_ptr() < changed.as_ptr().wrapping_add(changed.len()));
        let result = replay(&f, &changed).unwrap();
        assert_eq!(result.logical, ConcreteWitnessLogical::NonMember);
        assert_eq!(
            result.physical,
            ConcreteWitnessPhysical::HistoryUnavailable(f.id)
        );
        assert_eq!(
            result.usage.used(R::RawCopyBytes),
            baseline.usage.used(R::RawCopyBytes)
        );
    }
    assert_eq!(
        reconcile(
            f.id,
            ConcreteWitnessPhysical::HistoryUnavailable(f.id),
            PathProof::Member(vec![1])
        )
        .unwrap_err(),
        ConcreteWitnessError::Reconciliation
    );

    // An authenticated branch with no child is different from the empty-root
    // shortcut. Reconstruct its real canonical root through the account wrapper.
    let mut f = fixture(false, false);
    let mut canonical = RlpStream::new_list(17);
    let mut physical = RlpStream::new_list(16);
    for _ in 0..16 {
        canonical.append_empty_data();
        physical.append_empty_data();
    }
    canonical.append_empty_data();
    let storage_root = codec::keccak256(&canonical.out());
    let raw = account(Some(storage_root));
    let commitment =
        codec::account_commitment_rlp(&codec::decode_physical_account(&raw).unwrap()).unwrap();
    let prefix = codec::account_version_prefix(f.address);
    let mut compact = vec![0x20];
    compact.extend_from_slice(&prefix);
    f.id.state_root = codec::keccak256(&leaf(&compact, &commitment));
    f.nodes = vec![
        (2, f.id.state_root, leaf(&compact, &f.id.state_root)),
        (4, storage_root, physical.out().to_vec()),
    ];
    f.rows = vec![(3, codec::versioned_key(prefix, f.id.period), raw)];
    let (bytes, _) = capture(&f);
    assert_eq!(
        replay(&f, &bytes).unwrap().logical,
        ConcreteWitnessLogical::NonMember
    );

    let r = recorder(
        &f,
        ConcreteWitnessLimits::default()
            .lowered(R::Operations, 4)
            .unwrap(),
    );
    let id = |n| ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(n),
        state_root: [n as u8; 32],
    };
    r.budget.register_constructor(&[id(100), id(99)]).unwrap();
    let payload: Vec<_> = (51..100).map(id).collect();
    r.budget.register_constructor(&payload).unwrap();
    for _ in 0..4 {
        r.operation().unwrap();
    }
    r.budget.tuple(f.id, f.address, f.key).unwrap();
    assert!(tuple_proof(&r, f.address, f.key).is_err());
    assert_eq!(r.attempts.get(), 4);
    assert_eq!(r.records.borrow().len(), 0);
    assert_eq!(r.budget.usage().used(R::RawCopyBytes), 0);
    assert!(r.node("2", f.id.state_root).is_err());
    assert_eq!(r.attempts.get(), 4);

    let frames = admitted_frames(&bytes);
    let budget = ConcreteWitnessBudget::new(
        ConcreteWitnessLimits::default()
            .lowered(R::FrameInspections, 1)
            .unwrap(),
    );
    budget.charge(R::Records, frames.len() as u64).unwrap();
    assert!(matches!(
        encoded(
            Header {
                identity: f.id,
                address: f.address,
                key: f.key,
                records: frames.len() as u32
            },
            &frames,
            &budget
        ),
        Err(ConcreteWitnessError::LimitExceeded {
            resource: R::FrameInspections,
            ..
        })
    ));
    assert_eq!(budget.usage().used(R::RawCopyBytes), 0);
    assert_eq!(budget.usage().used(R::EncodedBytes), 0);
}
