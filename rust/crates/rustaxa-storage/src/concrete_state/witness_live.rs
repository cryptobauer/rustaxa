//! Consuming bounded diagnostic family. Roots and manifest are external authority.
use super::*;
use crate::{Column, FinalChainRepository, MetadataRepository, db::DbPointReader};
use rocksdb::{
    BoundedReadError, ColumnFamilyDescriptor, DBPinnableSlice, DBWithThreadMode, MultiThreaded,
    Options, ReadOptions,
};
use sha2::{Digest, Sha256};
use std::{mem::size_of, path::Path, sync::Arc};

pub(super) const HEAD: u64 = 25_706_949;
pub(super) const PARENT: u64 = 25_706_948;
pub(super) const FIRST_DELAYED: u64 = 25_706_894;
const CONCRETE_COLUMNS: [&str; 9] = ["default", "1", "2", "3", "4", "5", "6", "7", "8"];
const APPLICATION_COLUMNS: [Column; 4] = [
    Column::DefaultColumn,
    Column::Genesis,
    Column::FinalChainMeta,
    Column::FinalChainBlkByNumber,
];
const DESCRIPTOR_KEY: &[u8] = b"last_committed_descriptor";
type Owner = DBWithThreadMode<MultiThreaded>;

/// Exact externally approved tuple, with no captured verdict or authority bit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcreteDiagnosticTuple {
    pub identity: ConcreteStateIdentity,
    pub address: [u8; 20],
    pub key: ConcreteStorageKey,
}
/// Independently authenticated comparison roots and fixed native request family.
/// Validation requires P,H and 48 ascending D identities, then totals and jailed
/// tuples in ascending D order. Supplying this plan does not authenticate it.
pub struct ConcreteDiagnosticPlan {
    pub genesis_hash: [u8; 32],
    pub head_header_sha256: [u8; 32],
    pub head: ConcreteStateIdentity,
    pub parent: ConcreteStateIdentity,
    pub delayed: [ConcreteStateIdentity; 48],
    pub tuples: [ConcreteDiagnosticTuple; 96],
}
/// Fixed capture/replay phase for first-failure retention evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcreteDiagnosticPhase {
    Plan,
    ApplicationOpen,
    Pair,
    FirstOpen,
    FirstDescriptor,
    FirstConstructor,
    SecondOpen,
    SecondDescriptor,
    SecondConstructor,
    Payload,
    Envelope,
    Admission,
}
/// Borrowed read disposition and admitted lengths; no engine error text or raw
/// payload is copied solely to retain a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcreteDiagnosticOutcome {
    Pending,
    End,
    Node { bytes: u64 },
    Row { key_bytes: u64, value_bytes: u64 },
    Error(u8),
}
/// Fixed last-read evidence. Request holds a node hash or version-seek prefix;
/// identity.period supplies the exact seek period. No engine text is retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcreteDiagnosticRead {
    pub operation: u8,
    pub column: u8,
    pub request: [u8; 32],
    pub outcome: ConcreteDiagnosticOutcome,
}
/// Sticky first failure and finite retention evidence; never an accepted family.
#[derive(Debug)]
pub struct ConcreteDiagnosticFailure {
    pub error: ConcreteWitnessError,
    pub phase: ConcreteDiagnosticPhase,
    pub index: u16,
    pub identity: Option<ConcreteStateIdentity>,
    pub tuple: Option<ConcreteDiagnosticTuple>,
    pub last_read: Option<ConcreteDiagnosticRead>,
    pub usage: ConcreteWitnessUsage,
}
impl std::fmt::Display for ConcreteDiagnosticFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}[{}]: {}", self.phase, self.index, self.error)
    }
}
impl std::error::Error for ConcreteDiagnosticFailure {}
/// Successful raw finite family. No captured logical verdict is serialized.
pub struct ConcreteDiagnosticCapture {
    pub bundle: Vec<u8>,
    pub usage: ConcreteWitnessUsage,
}
/// Independently reconstructed logical bytes in exact manifest order. Native
/// total/jailed decoding stays with the caller's reviewed business kernel.
pub struct ConcreteDiagnosticReplay {
    pub logical: [ConcreteWitnessLogical; 96],
    pub usage: ConcreteWitnessUsage,
}

pub(super) struct Run {
    pub budget: ConcreteWitnessBudget,
    phase: Cell<ConcreteDiagnosticPhase>,
    index: Cell<u16>,
    identity: Cell<Option<ConcreteStateIdentity>>,
    tuple: Cell<Option<ConcreteDiagnosticTuple>>,
    last: Cell<Option<ConcreteDiagnosticRead>>,
}
impl Run {
    fn new(limits: ConcreteWitnessLimits) -> Self {
        Self {
            budget: ConcreteWitnessBudget::new(limits),
            phase: Cell::new(ConcreteDiagnosticPhase::Plan),
            index: Cell::new(0),
            identity: Cell::new(None),
            tuple: Cell::new(None),
            last: Cell::new(None),
        }
    }
    pub(super) fn phase(
        &self,
        phase: ConcreteDiagnosticPhase,
        index: usize,
        identity: Option<ConcreteStateIdentity>,
        tuple: Option<ConcreteDiagnosticTuple>,
    ) {
        self.phase.set(phase);
        self.index.set(index as u16);
        self.identity.set(identity);
        self.tuple.set(tuple);
        self.last.set(None);
    }
    fn failure(&self, e: ConcreteWitnessError) -> ConcreteDiagnosticFailure {
        ConcreteDiagnosticFailure {
            error: self.budget.stop(e),
            phase: self.phase.get(),
            index: self.index.get(),
            identity: self.identity.get(),
            tuple: self.tuple.get(),
            last_read: self.last.get(),
            usage: self.budget.usage(),
        }
    }
    fn operation(&self) -> Result<(), ConcreteWitnessError> {
        self.budget.charge(R::Operations, 1)
    }
    fn read(&self, operation: u8, column: u8, request: [u8; 32]) {
        self.last.set(Some(ConcreteDiagnosticRead {
            operation,
            column,
            request,
            outcome: ConcreteDiagnosticOutcome::Pending,
        }));
    }
    fn outcome(&self, outcome: ConcreteDiagnosticOutcome) {
        if let Some(mut read) = self.last.get() {
            read.outcome = outcome;
            self.last.set(Some(read));
        }
    }
    fn backend<T>(&self, result: Result<T, BoundedReadError>) -> Result<T, ConcreteWitnessError> {
        result.map_err(|_| {
            self.outcome(ConcreteDiagnosticOutcome::Error(0));
            self.budget.stop(ConcreteWitnessError::Read(0))
        })
    }
}

pub(super) fn validate_plan(plan: &ConcreteDiagnosticPlan) -> Result<(), ConcreteWitnessError> {
    if plan.head.period.as_u64() != HEAD || plan.parent.period.as_u64() != PARENT {
        return Err(ConcreteWitnessError::Identity);
    }
    for (i, identity) in plan.delayed.iter().enumerate() {
        if identity.period.as_u64() != FIRST_DELAYED + i as u64 {
            return Err(ConcreteWitnessError::Identity);
        }
    }
    let mut total_address = [0; 20];
    total_address[19] = 0xfe;
    let mut jailed_address = [0; 20];
    jailed_address[19] = 0xee;
    let total_key = ConcreteStorageKey(codec::keccak256(&[4]));
    let mut jailed_key = [0; 32];
    jailed_key[31] = 2;
    for (index, tuple) in plan.tuples.iter().enumerate() {
        let (address, key) = if index < 48 {
            (total_address, total_key)
        } else {
            (jailed_address, ConcreteStorageKey(jailed_key))
        };
        if tuple.identity != plan.delayed[index % 48]
            || tuple.address != address
            || tuple.key != key
        {
            return Err(ConcreteWitnessError::Request);
        }
    }
    Ok(())
}

/// Borrow one canonical RLP item without an allocating decoder or error path.
/// Long lengths, singleton bytes and every scalar are checked canonically.
pub(super) fn item(bytes: &[u8]) -> Result<(bool, &[u8], usize), ConcreteWitnessError> {
    let first = *bytes.first().ok_or(ConcreteWitnessError::Schema)?;
    if first < 0x80 {
        return Ok((false, &bytes[..1], 1));
    }
    let (list, short, long) = if first < 0xc0 {
        (false, 0x80, 0xb7)
    } else {
        (true, 0xc0, 0xf7)
    };
    let (offset, length) = if first <= long {
        (1, usize::from(first - short))
    } else {
        let width = usize::from(first - long);
        let raw = bytes
            .get(1..1 + width)
            .ok_or(ConcreteWitnessError::Schema)?;
        if raw.first() == Some(&0) || width > size_of::<usize>() {
            return Err(ConcreteWitnessError::Schema);
        }
        let length = raw.iter().try_fold(0usize, |n, b| {
            n.checked_mul(256)
                .and_then(|n| n.checked_add(usize::from(*b)))
                .ok_or(ConcreteWitnessError::Overflow)
        })?;
        if length < 56 {
            return Err(ConcreteWitnessError::Schema);
        }
        (1 + width, length)
    };
    let end = offset
        .checked_add(length)
        .ok_or(ConcreteWitnessError::Overflow)?;
    let payload = bytes.get(offset..end).ok_or(ConcreteWitnessError::Schema)?;
    if !list && length == 1 && payload[0] < 0x80 {
        return Err(ConcreteWitnessError::Schema);
    }
    Ok((list, payload, end))
}
fn fields<const N: usize>(bytes: &[u8]) -> Result<[&[u8]; N], ConcreteWitnessError> {
    let (list, mut payload, length) = item(bytes)?;
    if !list || length != bytes.len() {
        return Err(ConcreteWitnessError::Schema);
    }
    let mut result = [&[][..]; N];
    for field in &mut result {
        let (list, data, length) = item(payload)?;
        if list {
            return Err(ConcreteWitnessError::Schema);
        }
        *field = data;
        payload = &payload[length..];
    }
    if !payload.is_empty() {
        return Err(ConcreteWitnessError::Schema);
    }
    Ok(result)
}
fn integer(bytes: &[u8], width: usize) -> Result<(), ConcreteWitnessError> {
    if bytes.len() > width || bytes.first() == Some(&0) {
        Err(ConcreteWitnessError::Schema)
    } else {
        Ok(())
    }
}
/// Strict seven-field borrowed header validation. No Vec, String, anyhow or
/// typed-header conversion occurs; only the fixed root leaves this function.
pub(super) fn header_root(bytes: &[u8]) -> Result<[u8; 32], ConcreteWitnessError> {
    let f = fields::<7>(bytes)?;
    if f[..4].iter().any(|v| v.len() != 32) || f[4].len() != 256 {
        return Err(ConcreteWitnessError::Schema);
    }
    integer(f[5], 8)?;
    integer(f[6], 32)?;
    f[1].try_into().map_err(|_| ConcreteWitnessError::Schema)
}
pub(super) fn descriptor(
    bytes: &[u8],
    expected: ConcreteStateIdentity,
) -> Result<(), ConcreteWitnessError> {
    let f = fields::<2>(bytes)?;
    integer(f[0], 8)?;
    let period = f[0].iter().fold(0u64, |n, b| n * 256 + u64::from(*b));
    if period != expected.period.as_u64() || f[1] != expected.state_root {
        return Err(ConcreteWitnessError::Identity);
    }
    Ok(())
}
fn pair_row(
    index: usize,
    row: &[u8],
    plan: &ConcreteDiagnosticPlan,
    budget: &ConcreteWitnessBudget,
) -> Result<(), ConcreteWitnessError> {
    budget.charge(R::RowBytes, row.len() as u64)?;
    match index {
        0 if row == plan.genesis_hash => Ok(()),
        1 if row == HEAD.to_le_bytes() => Ok(()),
        0 | 1 => Err(ConcreteWitnessError::Identity),
        2 | 3 => {
            budget.charge(R::CanonicalBytes, 2048)?;
            let root = header_root(row)?;
            let expected = if index == 2 { plan.head } else { plan.parent };
            if root != expected.state_root
                || index == 2 && <[u8; 32]>::from(Sha256::digest(row)) != plan.head_header_sha256
            {
                return Err(ConcreteWitnessError::Identity);
            }
            Ok(())
        }
        _ => Err(ConcreteWitnessError::Schema),
    }
}
// This backend exposes only typed borrowed point reads and never owns a budget.
struct PointBackend {
    db: Owner,
    options: ReadOptions,
}
impl DbPointReader for PointBackend {
    type Slice<'a> = DBPinnableSlice<'a>;
    type Error = BoundedReadError;
    fn point_get<'a>(
        &'a self,
        column: Column,
        key: &[u8],
    ) -> Result<Option<Self::Slice<'a>>, Self::Error> {
        if !APPLICATION_COLUMNS.contains(&column) {
            return Err(BoundedReadError::InvalidDescriptors);
        }
        let cf = self
            .db
            .cf_handle(column.name())
            .ok_or(BoundedReadError::InvalidDescriptors)?;
        self.db.get_pinned_cf_bounded_opt(&cf, key, &self.options)
    }
}
fn open(path: &Path, application: bool, run: &Run) -> Result<Owner, ConcreteWitnessError> {
    let text = path
        .to_str()
        .ok_or_else(|| run.budget.stop(ConcreteWitnessError::Schema))?;
    if text.is_empty() || text.len() > 4096 || text.as_bytes().contains(&0) {
        return Err(run.budget.stop(ConcreteWitnessError::Schema));
    }
    let count = if application { 4 } else { 9 };
    let metadata = 32768usize
        .checked_add(3 * text.len())
        .and_then(|n| n.checked_add(2048 * count))
        .ok_or_else(|| run.budget.stop(ConcreteWitnessError::Overflow))?;
    if metadata > 65536 || size_of::<ColumnFamilyDescriptor>() > 256 {
        return Err(run.budget.stop(ConcreteWitnessError::Schema));
    }
    run.budget.charge(R::CanonicalBytes, metadata as u64)?;
    run.operation()?;
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    options.set_max_open_files(128);
    let mut columns = Vec::new();
    columns
        .try_reserve_exact(count)
        .map_err(|_| run.budget.stop(ConcreteWitnessError::Allocation))?;
    if application {
        for column in APPLICATION_COLUMNS {
            columns.push(column.descriptor(&options));
        }
    } else {
        for name in CONCRETE_COLUMNS {
            columns.push(ColumnFamilyDescriptor::new(name, Options::default()));
        }
    }
    run.backend(Owner::open_cf_descriptors_read_only_bounded(
        &options, path, columns, false,
    ))
}
fn pair_read<T: AsRef<[u8]>>(
    run: &Run,
    read: impl FnOnce() -> Result<Option<T>, BoundedReadError>,
) -> Result<Vec<u8>, ConcreteWitnessError> {
    run.budget.charge(R::CanonicalBytes, 1024)?;
    run.operation()?;
    let pin = run.backend(read())?.ok_or_else(|| {
        run.outcome(ConcreteDiagnosticOutcome::End);
        run.budget.stop(ConcreteWitnessError::Read(3))
    })?;
    let bytes = pin.as_ref();
    run.outcome(ConcreteDiagnosticOutcome::Node {
        bytes: bytes.len() as u64,
    });
    run.budget.charge(R::RowBytes, bytes.len() as u64)?;
    run.budget.copy(bytes)
}
fn read_descriptor(
    db: &Owner,
    run: &Run,
    expected: ConcreteStateIdentity,
) -> Result<Vec<u8>, ConcreteWitnessError> {
    run.read(0, 0, [0; 32]);
    run.operation()?;
    let pin = run
        .backend(db.get_pinned_bounded_opt(DESCRIPTOR_KEY, &ReadOptions::default()))?
        .ok_or_else(|| {
            run.outcome(ConcreteDiagnosticOutcome::End);
            run.budget.stop(ConcreteWitnessError::Read(3))
        })?;
    run.outcome(ConcreteDiagnosticOutcome::Node {
        bytes: pin.len() as u64,
    });
    run.budget.charge(R::RowBytes, pin.len() as u64)?;
    descriptor(&pin, expected)?;
    run.budget.copy(&pin)
}

struct OwnedFrame {
    operation: u8,
    column: u8,
    request: [u8; 32],
    outcome: OwnedOutcome,
}
enum OwnedOutcome {
    End,
    Node(Vec<u8>),
    Row(Vec<u8>, Vec<u8>),
}
struct Recorder<'a> {
    db: &'a Owner,
    run: &'a Run,
    identity: ConcreteStateIdentity,
    records: RefCell<Vec<OwnedFrame>>,
}
impl Recorder<'_> {
    fn failure(&self, e: ConcreteWitnessError) -> ConcreteReadError {
        self.run.budget.stop(e);
        codec::corrupt("bounded live witness stopped")
    }
    fn push(
        &self,
        operation: u8,
        column: u8,
        request: [u8; 32],
        outcome: OwnedOutcome,
    ) -> Result<(), ConcreteWitnessError> {
        self.run.budget.charge(R::Records, 1)?;
        let mut records = self.records.borrow_mut();
        if records.len() == records.capacity() {
            let n = records
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(size_of::<OwnedFrame>()))
                .ok_or_else(|| self.run.budget.stop(ConcreteWitnessError::Overflow))?;
            self.run.budget.charge(R::CanonicalBytes, n as u64)?;
            records
                .try_reserve_exact(1)
                .map_err(|_| self.run.budget.stop(ConcreteWitnessError::Allocation))?;
        }
        records.push(OwnedFrame {
            operation,
            column,
            request,
            outcome,
        });
        Ok(())
    }
    fn code(column: &str) -> Result<u8, ConcreteWitnessError> {
        match column {
            "2" => Ok(2),
            "3" => Ok(3),
            "4" => Ok(4),
            "5" => Ok(5),
            _ => Err(ConcreteWitnessError::Request),
        }
    }
    fn encode(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
        baseline: u64,
        account: bool,
        prior: &[Vec<u8>],
    ) -> Result<Vec<u8>, ConcreteWitnessError> {
        let records = self.records.borrow();
        let n = records
            .len()
            .checked_mul(size_of::<binary::Frame<'_>>())
            .ok_or_else(|| self.run.budget.stop(ConcreteWitnessError::Overflow))?;
        self.run.budget.charge(R::CanonicalBytes, n as u64)?;
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(records.len())
            .map_err(|_| self.run.budget.stop(ConcreteWitnessError::Allocation))?;
        for (ordinal, record) in records.iter().enumerate() {
            let outcome = match &record.outcome {
                OwnedOutcome::End => binary::Outcome::End,
                OwnedOutcome::Node(v) => binary::Outcome::Node(v),
                OwnedOutcome::Row(k, v) => binary::Outcome::Row(k, v),
            };
            frames.push(binary::Frame {
                ordinal: ordinal as u64,
                identity: self.identity,
                operation: record.operation,
                column: record.column,
                request: record.request,
                period: self.identity.period.as_u64(),
                outcome,
            });
        }
        bundle::consistent_frames(self.identity, &frames, prior, &self.run.budget)?;
        let header = binary::Header {
            identity: self.identity,
            address,
            key,
            records: u32::try_from(frames.len())
                .map_err(|_| self.run.budget.stop(ConcreteWitnessError::Overflow))?,
        };
        if baseline == 0 && !account {
            binary::encode(header, &frames, &self.run.budget)
        } else {
            binary::encode_transcript(header, &frames, &self.run.budget, baseline, account)
        }
    }
}
impl PhysicalTrieStore for Recorder<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }
    fn allocation_error(&self) -> ConcreteReadError {
        self.failure(ConcreteWitnessError::Allocation)
    }
    fn visit_budget(&self, n: usize) -> Result<(), ConcreteReadError> {
        self.run
            .budget
            .charge(R::RowBytes, n as u64)
            .and_then(|()| self.run.budget.charge(R::Visits, 1))
            .map_err(|e| self.failure(e))
    }
    fn canonical_budget(&self, n: usize) -> Result<(), ConcreteReadError> {
        self.run
            .budget
            .charge(R::CanonicalBytes, n as u64)
            .map_err(|e| self.failure(e))
    }
    fn account_commitment(&self, b: &[u8]) -> Result<Vec<u8>, ConcreteReadError> {
        codec::account_view_commitment(b, |n| self.canonical_budget(n), || self.allocation_error())
    }
    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        let work = || -> Result<_, ConcreteWitnessError> {
            let code = Self::code(column)?;
            self.run.read(0, code, hash);
            self.run.operation()?;
            let cf = self
                .db
                .cf_handle(column)
                .ok_or(ConcreteWitnessError::Schema)?;
            let pin = self.run.backend(self.db.get_pinned_cf_bounded_opt(
                &cf,
                hash,
                &ReadOptions::default(),
            ))?;
            match pin {
                None => {
                    self.run.outcome(ConcreteDiagnosticOutcome::End);
                    self.push(0, code, hash, OwnedOutcome::End)?;
                    Ok(None)
                }
                Some(pin) => {
                    self.run.outcome(ConcreteDiagnosticOutcome::Node {
                        bytes: pin.len() as u64,
                    });
                    self.run.budget.charge(R::RowBytes, pin.len() as u64)?;
                    let raw = self.run.budget.copy(&pin)?;
                    let returned = self.run.budget.copy(&pin)?;
                    self.push(0, code, hash, OwnedOutcome::Node(raw))?;
                    Ok(Some(returned))
                }
            }
        };
        work().map_err(|e| self.failure(e))
    }
    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        let work = || -> Result<_, ConcreteWitnessError> {
            let code = Self::code(column)?;
            self.run.read(1, code, prefix);
            self.run.operation()?;
            let cf = self
                .db
                .cf_handle(column)
                .ok_or(ConcreteWitnessError::Schema)?;
            let mut cursor = self.db.raw_iterator_cf(&cf);
            self.run.operation()?;
            cursor.seek_for_prev(codec::versioned_key(prefix, period));
            self.run.operation()?;
            self.run.backend(cursor.status_bounded())?;
            match (cursor.key(), cursor.value()) {
                (None, None) => {
                    self.run.outcome(ConcreteDiagnosticOutcome::End);
                    self.push(1, code, prefix, OwnedOutcome::End)?;
                    Ok(None)
                }
                (Some(key), Some(value)) => {
                    self.run.outcome(ConcreteDiagnosticOutcome::Row {
                        key_bytes: key.len() as u64,
                        value_bytes: value.len() as u64,
                    });
                    self.run.budget.charge(R::KeyBytes, key.len() as u64)?;
                    self.run.budget.charge(R::RowBytes, value.len() as u64)?;
                    let raw_key = self.run.budget.copy(key)?;
                    let raw_value = self.run.budget.copy(value)?;
                    self.push(1, code, prefix, OwnedOutcome::Row(raw_key, raw_value))?;
                    if !key.starts_with(&prefix) {
                        return Ok(None);
                    }
                    if key.len() != 40 {
                        return Err(ConcreteWitnessError::Schema);
                    }
                    Ok(Some(SelectedVersion {
                        value: self.run.budget.copy(value)?,
                    }))
                }
                _ => Err(ConcreteWitnessError::Schema),
            }
        };
        work().map_err(|e| self.failure(e))
    }
}

pub(super) fn account_proof<S: PhysicalTrieStore>(store: &S) -> Result<(), ConcreteReadError> {
    let prefix = codec::account_version_prefix([0; 20]);
    let selected = store.value("3", prefix, store.identity().period)?;
    let proof = verify_path(
        store,
        store.identity().state_root,
        prefix,
        "2",
        "3",
        |p| p,
        TrieSchema::Account,
    )?;
    match (proof, selected) {
        (PathProof::Member(proved), Some(selected))
            if !proved.is_empty() && proved == selected.value =>
        {
            codec::account_view(&proved)?;
            Ok(())
        }
        (PathProof::NonMember, None) => Ok(()),
        (PathProof::NonMember, Some(selected)) if selected.value.is_empty() => Ok(()),
        _ => Err(codec::corrupt("constructor account reconciliation failed")),
    }
}
fn capture_proof(
    db: &Owner,
    run: &Run,
    identity: ConcreteStateIdentity,
    tuple: Option<ConcreteDiagnosticTuple>,
    prior: &[Vec<u8>],
) -> Result<Vec<u8>, ConcreteWitnessError> {
    run.budget.charge(R::CanonicalBytes, 1024)?;
    let baseline = run.budget.usage().used(R::Records);
    let store = Recorder {
        db,
        run,
        identity,
        records: RefCell::new(Vec::new()),
    };
    if let Some(tuple) = tuple {
        run.budget.tuple(identity, tuple.address, tuple.key)?;
        let (physical, proof) = tuple_proof(&store, tuple.address, tuple.key)
            .map_err(|_| run.budget.error().unwrap_or(ConcreteWitnessError::Proof))?;
        reconcile(identity, physical, proof)?;
        store.encode(tuple.address, tuple.key, baseline, false, prior)
    } else {
        account_proof(&store)
            .map_err(|_| run.budget.error().unwrap_or(ConcreteWitnessError::Proof))?;
        store.encode([0; 20], ConcreteStorageKey([0; 32]), baseline, true, prior)
    }
}

/// Open only the fixed read-only subsets, validate the pair and both descriptors,
/// authenticate both ordered constructors and capture exactly the approved96
/// tuples. One budget owns the entire run; a failure prevents subsequent reads.
/// This diagnostic is not a production reader or historical authority token.
#[allow(
    clippy::result_large_err,
    reason = "fixed failure evidence stays on the stack after budget exhaustion"
)]
pub fn capture_diagnostic_run(
    application: &Path,
    state: &Path,
    plan: &ConcreteDiagnosticPlan,
    limits: ConcreteWitnessLimits,
) -> Result<ConcreteDiagnosticCapture, ConcreteDiagnosticFailure> {
    let run = Run::new(limits);
    let work = || -> Result<_, ConcreteWitnessError> {
        validate_plan(plan)?;
        run.phase(ConcreteDiagnosticPhase::ApplicationOpen, 0, None, None);
        let point = Arc::new(PointBackend {
            db: open(application, true, &run)?,
            options: ReadOptions::default(),
        });
        let metadata = MetadataRepository::new(Arc::clone(&point));
        let final_chain = FinalChainRepository::new(point);
        let mut pair_rows: [Vec<u8>; 4] = std::array::from_fn(|_| Vec::new());
        for (i, pair_row_bytes) in pair_rows.iter_mut().enumerate() {
            run.phase(ConcreteDiagnosticPhase::Pair, i, None, None);
            let (column, key): (u8, [u8; 32]) = match i {
                0 => (1, [0; 32]),
                1 => {
                    let mut k = [0; 32];
                    k[..4].copy_from_slice(&1u32.to_le_bytes());
                    (2, k)
                }
                2 => {
                    let mut k = [0; 32];
                    k[..8].copy_from_slice(&HEAD.to_le_bytes());
                    (3, k)
                }
                _ => {
                    let mut k = [0; 32];
                    k[..8].copy_from_slice(&PARENT.to_le_bytes());
                    (3, k)
                }
            };
            run.read(0, column, key);
            *pair_row_bytes = match i {
                0 => pair_read(&run, || metadata.genesis_hash_view()),
                1 => pair_read(&run, || final_chain.meta_value_view(1)),
                2 => pair_read(&run, || final_chain.block_header_raw_view(HEAD)),
                _ => pair_read(&run, || final_chain.block_header_raw_view(PARENT)),
            }?;
            pair_row(i, pair_row_bytes, plan, &run.budget)?;
        }

        let mut descriptors: [Vec<u8>; 2] = std::array::from_fn(|_| Vec::new());
        let mut proofs = bundle::proof_vector(&run.budget)?;
        for (constructor, descriptor_bytes) in descriptors.iter_mut().enumerate() {
            let (open_phase, descriptor_phase, proof_phase) = if constructor == 0 {
                (
                    ConcreteDiagnosticPhase::FirstOpen,
                    ConcreteDiagnosticPhase::FirstDescriptor,
                    ConcreteDiagnosticPhase::FirstConstructor,
                )
            } else {
                (
                    ConcreteDiagnosticPhase::SecondOpen,
                    ConcreteDiagnosticPhase::SecondDescriptor,
                    ConcreteDiagnosticPhase::SecondConstructor,
                )
            };
            run.phase(open_phase, 0, Some(plan.head), None);
            let db = open(state, false, &run)?;
            run.phase(descriptor_phase, 0, Some(plan.head), None);
            *descriptor_bytes = read_descriptor(&db, &run, plan.head)?;
            let mut ids = [plan.head; 49];
            let count = if constructor == 0 {
                ids[0] = plan.parent;
                ids[1] = plan.head;
                2
            } else {
                ids[..48].copy_from_slice(&plan.delayed);
                49
            };
            run.phase(proof_phase, 0, Some(ids[0]), None);
            run.budget.register_constructor(&ids[..count])?;
            for (index, id) in ids[..count].iter().enumerate() {
                run.phase(proof_phase, index, Some(*id), None);
                proofs.push(capture_proof(&db, &run, *id, None, &proofs)?);
            }
            if constructor == 1 {
                for (index, tuple) in plan.tuples.iter().enumerate() {
                    run.phase(
                        ConcreteDiagnosticPhase::Payload,
                        index,
                        Some(tuple.identity),
                        Some(*tuple),
                    );
                    proofs.push(capture_proof(
                        &db,
                        &run,
                        tuple.identity,
                        Some(*tuple),
                        &proofs,
                    )?);
                }
            }
        }
        run.phase(ConcreteDiagnosticPhase::Envelope, 0, None, None);
        let bundle = bundle::encode(&pair_rows, &descriptors, &proofs, &run.budget)?;
        Ok(ConcreteDiagnosticCapture {
            bundle,
            usage: run.budget.usage(),
        })
    };
    work().map_err(|e| run.failure(e))
}

/// Offline verification has no path, owner or network input. Admit the complete
/// envelope before decoding, compare external pair/roots and replay every proof
/// in exact order on one independent family budget. No partial family escapes.
#[allow(
    clippy::result_large_err,
    reason = "fixed failure evidence stays on the stack after budget exhaustion"
)]
pub fn replay_diagnostic_bundle(
    bytes: &[u8],
    plan: &ConcreteDiagnosticPlan,
    limits: ConcreteWitnessLimits,
) -> Result<ConcreteDiagnosticReplay, ConcreteDiagnosticFailure> {
    let run = Run::new(limits);
    let work = || -> Result<_, ConcreteWitnessError> {
        validate_plan(plan)?;
        run.phase(ConcreteDiagnosticPhase::Admission, 0, None, None);
        run.budget.admit_input(bytes.len())?;
        let envelope = bundle::admit(bytes, &run.budget)?;
        for (index, row) in envelope.pair.iter().enumerate() {
            run.phase(ConcreteDiagnosticPhase::Pair, index, None, None);
            let mut request = [0; 32];
            if index == 1 {
                request[..4].copy_from_slice(&1u32.to_le_bytes());
            }
            if index >= 2 {
                request[..8].copy_from_slice(&if index == 2 { HEAD } else { PARENT }.to_le_bytes());
            }
            run.read(
                0,
                if index == 0 {
                    1
                } else if index == 1 {
                    2
                } else {
                    3
                },
                request,
            );
            run.outcome(ConcreteDiagnosticOutcome::Node {
                bytes: row.len() as u64,
            });
            pair_row(index, row, plan, &run.budget)?;
        }
        for (index, raw) in envelope.descriptors.iter().enumerate() {
            run.phase(
                if index == 0 {
                    ConcreteDiagnosticPhase::FirstDescriptor
                } else {
                    ConcreteDiagnosticPhase::SecondDescriptor
                },
                0,
                Some(plan.head),
                None,
            );
            run.read(0, 0, [0; 32]);
            run.outcome(ConcreteDiagnosticOutcome::Node {
                bytes: raw.len() as u64,
            });
            descriptor(raw, plan.head)?;
        }
        let mut cursor = 0;
        for constructor in 0..2 {
            let mut ids = [plan.head; 49];
            let count = if constructor == 0 {
                ids[0] = plan.parent;
                ids[1] = plan.head;
                2
            } else {
                ids[..48].copy_from_slice(&plan.delayed);
                49
            };
            run.phase(
                if constructor == 0 {
                    ConcreteDiagnosticPhase::FirstConstructor
                } else {
                    ConcreteDiagnosticPhase::SecondConstructor
                },
                0,
                Some(ids[0]),
                None,
            );
            run.budget.register_constructor(&ids[..count])?;
            for (index, id) in ids[..count].iter().enumerate() {
                run.phase(
                    if constructor == 0 {
                        ConcreteDiagnosticPhase::FirstConstructor
                    } else {
                        ConcreteDiagnosticPhase::SecondConstructor
                    },
                    index,
                    Some(*id),
                    None,
                );
                replay_proof(
                    envelope.proofs[cursor],
                    &run,
                    *id,
                    None,
                    &envelope.proofs[..cursor],
                )?;
                cursor += 1;
            }
        }
        let mut logical = std::array::from_fn(|_| ConcreteWitnessLogical::NonMember);
        for (index, tuple) in plan.tuples.iter().enumerate() {
            run.phase(
                ConcreteDiagnosticPhase::Payload,
                index,
                Some(tuple.identity),
                Some(*tuple),
            );
            logical[index] = replay_proof(
                envelope.proofs[cursor],
                &run,
                tuple.identity,
                Some(*tuple),
                &envelope.proofs[..cursor],
            )?
            .ok_or(ConcreteWitnessError::Schema)?;
            cursor += 1;
        }
        if cursor != envelope.proofs.len() {
            return Err(ConcreteWitnessError::UnusedRecords);
        }
        Ok(ConcreteDiagnosticReplay {
            logical,
            usage: run.budget.usage(),
        })
    };
    work().map_err(|e| run.failure(e))
}
fn replay_proof(
    bytes: &[u8],
    run: &Run,
    identity: ConcreteStateIdentity,
    tuple: Option<ConcreteDiagnosticTuple>,
    prior: &[&[u8]],
) -> Result<Option<ConcreteWitnessLogical>, ConcreteWitnessError> {
    let (address, key) = tuple
        .map(|t| (t.address, t.key))
        .unwrap_or(([0; 20], ConcreteStorageKey([0; 32])));
    let header = binary::admit_transcript(
        bytes,
        &run.budget,
        Some((identity, address, key)),
        tuple.is_none(),
    )
    .map_err(|error| run.budget.stop(error))?;
    bundle::consistent_transcript(bytes, header, prior, &run.budget)?;
    let witness = ConcreteStorageWitness { bytes, header };
    run.budget.charge(R::CanonicalBytes, 1024)?;
    let store = ReplayStore {
        witness: &witness,
        budget: &run.budget,
        offset: Cell::new(binary::HEADER_BYTES),
        ordinal: Cell::new(0),
        last_read: Some(&run.last),
    };
    let result = if let Some(tuple) = tuple {
        run.budget.tuple(identity, tuple.address, tuple.key)?;
        let (physical, proof) = tuple_proof(&store, address, key)
            .map_err(|_| run.budget.error().unwrap_or(ConcreteWitnessError::Proof))?;
        Some(reconcile(identity, physical, proof)?.1)
    } else {
        account_proof(&store)
            .map_err(|_| run.budget.error().unwrap_or(ConcreteWitnessError::Proof))?;
        None
    };
    if store.offset.get() != bytes.len() || store.ordinal.get() != u64::from(header.records) {
        return Err(ConcreteWitnessError::UnusedRecords);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "witness_live_tests.rs"]
mod tests;
