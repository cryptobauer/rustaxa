//! Bounded synthetic witness diagnostics. Expected roots require external
//! authority; this module establishes no history completeness or live routing.
//! Limits cover application payload copies and reconstruction charges, not
//! database-engine memory, allocator overhead, operating-system I/O or RSS.
use super::{
    codec,
    physical_node::{PathProof, PhysicalTrieStore, SelectedVersion, TrieSchema, verify_path},
};
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteReadError, ConcreteStateIdentity, ConcreteStorageKey},
};
use std::cell::{Cell, RefCell};
#[path = "witness_binary.rs"]
mod binary;

/// Independent accounting categories. Key/row/constructor widths are maxima;
/// all other counters are cumulative and cannot be refunded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum ConcreteWitnessResource {
    KeyBytes,
    RowBytes,
    RawCopyBytes,
    CanonicalBytes,
    EncodedBytes,
    Operations,
    Visits,
    Records,
    FrameInspections,
    ConstructorIdentities,
    Registrations,
    DistinctIdentities,
    PayloadTuples,
}
use ConcreteWitnessResource as R;
const CEILINGS: [u64; 13] = [
    64,
    1_048_576,
    67_108_864,
    67_108_864,
    134_217_728,
    100_000,
    100_000,
    100_000,
    300_000,
    49,
    51,
    50,
    96,
];
/// Checked owner ceilings. Explicitly lowering a resource to zero is deny-all
/// for that resource; no operation silently restores the default.
#[derive(Clone, Copy, Debug)]
pub struct ConcreteWitnessLimits {
    limits: [u64; 13],
}
impl Default for ConcreteWitnessLimits {
    fn default() -> Self {
        Self { limits: CEILINGS }
    }
}
impl ConcreteWitnessLimits {
    pub fn limit(&self, resource: R) -> u64 {
        self.limits[resource as usize]
    }
    /// A stricter finite policy; raising any current limit returns an error.
    pub fn lowered(mut self, resource: R, limit: u64) -> Result<Self, ConcreteWitnessError> {
        if limit > self.limit(resource) {
            return Err(ConcreteWitnessError::LimitExceeded {
                resource,
                limit: self.limit(resource),
                required: limit,
            });
        }
        self.limits[resource as usize] = limit;
        Ok(self)
    }
}
/// Bounded errors contain no decoded strings or accepted partial values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConcreteWitnessError {
    Schema,
    Identity,
    Request,
    Conflict,
    UnusedRecords,
    Reconciliation,
    Proof,
    Read(u8),
    Allocation,
    Overflow,
    LimitExceeded {
        resource: R,
        limit: u64,
        required: u64,
    },
}
impl std::fmt::Display for ConcreteWitnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ConcreteWitnessError {}
/// Actual accounting, separate from encoded input admission. It is neither
/// allocator peak memory nor database read-byte measurement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConcreteWitnessUsage {
    counters: [u64; 13],
    pub admitted_input_bytes: u64,
}
impl ConcreteWitnessUsage {
    pub fn used(&self, resource: R) -> u64 {
        self.counters[resource as usize]
    }
}
struct State {
    usage: ConcreteWitnessUsage,
    stopped: Option<ConcreteWitnessError>,
    identities: [Option<ConcreteStateIdentity>; 50],
    tuples: [Option<(ConcreteStateIdentity, [u8; 20], ConcreteStorageKey)>; 96],
    constructors: u8,
}
/// One non-cloneable monotonic budget. Internal charging stops permanently on
/// failure before the next copy, visit or operation. Metadata has fixed capacity.
pub struct ConcreteWitnessBudget {
    limits: ConcreteWitnessLimits,
    state: RefCell<State>,
}
impl ConcreteWitnessBudget {
    pub fn new(limits: ConcreteWitnessLimits) -> Self {
        Self {
            limits,
            state: RefCell::new(State {
                usage: ConcreteWitnessUsage::default(),
                stopped: None,
                identities: [None; 50],
                tuples: [None; 96],
                constructors: 0,
            }),
        }
    }
    pub fn usage(&self) -> ConcreteWitnessUsage {
        self.state.borrow().usage
    }
    fn stop(&self, error: ConcreteWitnessError) -> ConcreteWitnessError {
        let mut state = self.state.borrow_mut();
        *state.stopped.get_or_insert(error)
    }
    fn charge(&self, resource: R, amount: u64) -> Result<(), ConcreteWitnessError> {
        let mut s = self.state.borrow_mut();
        if let Some(e) = s.stopped {
            return Err(e);
        }
        let previous = s.usage.counters[resource as usize];
        let required = if matches!(
            resource,
            R::KeyBytes | R::RowBytes | R::ConstructorIdentities
        ) {
            Some(previous.max(amount))
        } else {
            previous.checked_add(amount)
        };
        let Some(required) = required else {
            s.stopped = Some(ConcreteWitnessError::Overflow);
            return Err(ConcreteWitnessError::Overflow);
        };
        let limit = self.limits.limit(resource);
        if required > limit {
            let e = ConcreteWitnessError::LimitExceeded {
                resource,
                limit,
                required,
            };
            s.stopped = Some(e);
            return Err(e);
        }
        s.usage.counters[resource as usize] = required;
        Ok(())
    }
    fn copy(&self, bytes: &[u8]) -> Result<Vec<u8>, ConcreteWitnessError> {
        self.charge(R::RowBytes, bytes.len() as u64)?;
        self.charge(R::RawCopyBytes, bytes.len() as u64)?;
        let mut out = Vec::new();
        out.try_reserve_exact(bytes.len())
            .map_err(|_| self.stop(ConcreteWitnessError::Allocation))?;
        out.extend_from_slice(bytes);
        Ok(out)
    }
    fn error(&self) -> Option<ConcreteWitnessError> {
        self.state.borrow().stopped
    }
    fn tuple(
        &self,
        id: ConcreteStateIdentity,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<(), ConcreteWitnessError> {
        let tuple = (id, address, key);
        if self
            .state
            .borrow()
            .tuples
            .iter()
            .flatten()
            .any(|t| *t == tuple)
        {
            return Err(self.stop(ConcreteWitnessError::Conflict));
        }
        self.charge(R::PayloadTuples, 1)?;
        let index = self.usage().used(R::PayloadTuples) as usize - 1;
        self.state.borrow_mut().tuples[index] = Some(tuple);
        Ok(())
    }
    /// Synthetic registration accounting only. The later live route must also
    /// authenticate every constructor path and pair/header input.
    pub fn register_constructor(
        &self,
        identities: &[ConcreteStateIdentity],
    ) -> Result<(), ConcreteWitnessError> {
        self.charge(R::ConstructorIdentities, identities.len() as u64)?;
        let phase = self.state.borrow().constructors;
        if phase >= 2 || identities.len() != if phase == 0 { 2 } else { 49 } {
            return Err(self.stop(ConcreteWitnessError::Request));
        }
        for (i, id) in identities.iter().enumerate() {
            if identities[..i].contains(id) {
                return Err(self.stop(ConcreteWitnessError::Conflict));
            }
            self.charge(R::Registrations, 1)?;
            if !self
                .state
                .borrow()
                .identities
                .iter()
                .flatten()
                .any(|v| v == id)
            {
                self.charge(R::DistinctIdentities, 1)?;
                let index = self.usage().used(R::DistinctIdentities) as usize - 1;
                self.state.borrow_mut().identities[index] = Some(*id);
            }
        }
        self.state.borrow_mut().constructors += 1;
        Ok(())
    }
}
/// Schema-admitted borrowed bytes. This is not an authenticated historical token.
pub struct ConcreteStorageWitness<'a> {
    bytes: &'a [u8],
    header: binary::Header,
}
impl<'a> ConcreteStorageWitness<'a> {
    pub fn admit(
        bytes: &'a [u8],
        limits: ConcreteWitnessLimits,
    ) -> Result<Self, ConcreteWitnessError> {
        Self::admit_budget(bytes, &ConcreteWitnessBudget::new(limits), None)
    }
    fn admit_budget(
        bytes: &'a [u8],
        budget: &ConcreteWitnessBudget,
        expected: Option<(ConcreteStateIdentity, [u8; 20], ConcreteStorageKey)>,
    ) -> Result<Self, ConcreteWitnessError> {
        let header = binary::admit(bytes, budget, expected)?;
        Ok(Self { bytes, header })
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}
/// Physical history is observed provenance, never proof of iterator completeness.
#[derive(Debug, Eq, PartialEq)]
pub enum ConcreteWitnessPhysical {
    Present(Vec<u8>),
    Tombstone,
    HistoryUnavailable(ConcreteStateIdentity),
}
/// Reconstructed logical commitment against the independently expected root.
#[derive(Debug, Eq, PartialEq)]
pub enum ConcreteWitnessLogical {
    Member(Vec<u8>),
    NonMember,
}
#[derive(Debug)]
pub struct ConcreteWitnessReplay {
    pub identity: ConcreteStateIdentity,
    pub address: [u8; 20],
    pub key: ConcreteStorageKey,
    pub physical: ConcreteWitnessPhysical,
    pub logical: ConcreteWitnessLogical,
    pub usage: ConcreteWitnessUsage,
}
struct ReplayStore<'a, 'b> {
    witness: &'b ConcreteStorageWitness<'a>,
    budget: &'b ConcreteWitnessBudget,
    offset: Cell<usize>,
    ordinal: Cell<u64>,
}
impl ReplayStore<'_, '_> {
    fn failure(&self, e: ConcreteWitnessError) -> ConcreteReadError {
        self.budget.stop(e);
        codec::corrupt("bounded witness stopped")
    }
    fn next(
        &self,
        operation: u8,
        column: &str,
        request: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<binary::Outcome<'_>, ConcreteReadError> {
        self.budget
            .charge(R::Operations, 1)
            .map_err(|e| self.failure(e))?;
        let (frame, next) = binary::frame_at(self.witness.bytes, self.offset.get(), self.budget)
            .map_err(|e| self.failure(e))?;
        let column = match column {
            "2" => 2,
            "3" => 3,
            "4" => 4,
            "5" => 5,
            _ => return Err(self.failure(ConcreteWitnessError::Request)),
        };
        if frame.ordinal != self.ordinal.get()
            || frame.identity != self.identity()
            || frame.operation != operation
            || frame.column != column
            || frame.request != request
            || frame.period != period.as_u64()
        {
            return Err(self.failure(ConcreteWitnessError::Request));
        }
        self.offset.set(next);
        self.ordinal.set(self.ordinal.get() + 1);
        Ok(frame.outcome)
    }
}
impl PhysicalTrieStore for ReplayStore<'_, '_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.witness.header.identity
    }
    fn allocation_error(&self) -> ConcreteReadError {
        self.failure(ConcreteWitnessError::Allocation)
    }
    fn visit_budget(&self, raw_bytes: usize) -> Result<(), ConcreteReadError> {
        self.budget
            .charge(R::RowBytes, raw_bytes as u64)
            .and_then(|()| self.budget.charge(R::Visits, 1))
            .map_err(|e| self.failure(e))
    }
    fn canonical_budget(&self, bytes: usize) -> Result<(), ConcreteReadError> {
        self.budget
            .charge(R::CanonicalBytes, bytes as u64)
            .map_err(|e| self.failure(e))
    }
    fn account_commitment(&self, bytes: &[u8]) -> Result<Vec<u8>, ConcreteReadError> {
        codec::account_view_commitment(
            bytes,
            |n| self.canonical_budget(n),
            || self.allocation_error(),
        )
    }
    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
        match self.next(0, column, hash, self.identity().period)? {
            binary::Outcome::End => Ok(None),
            binary::Outcome::Node(bytes) => self
                .budget
                .copy(bytes)
                .map(Some)
                .map_err(|e| self.failure(e)),
            binary::Outcome::Error(3) => {
                Err(ConcreteReadError::HistoryUnavailable(self.identity()))
            }
            binary::Outcome::Error(code) => Err(self.failure(ConcreteWitnessError::Read(code))),
            _ => Err(self.failure(ConcreteWitnessError::Schema)),
        }
    }
    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
        match self.next(1, column, prefix, period)? {
            binary::Outcome::End => Ok(None),
            binary::Outcome::Row(key, value) => {
                let seek = codec::versioned_key(prefix, period);
                if key > seek.as_slice() {
                    return Err(self.failure(ConcreteWitnessError::Request));
                }
                if key.len() != 40 {
                    if key.starts_with(&prefix) {
                        return Err(self.failure(ConcreteWitnessError::Schema));
                    }
                    return Ok(None);
                }
                if !key.starts_with(&prefix) {
                    return Ok(None);
                }
                self.budget
                    .copy(value)
                    .map(|value| Some(SelectedVersion { value }))
                    .map_err(|e| self.failure(e))
            }
            binary::Outcome::Error(3) => {
                Err(ConcreteReadError::HistoryUnavailable(self.identity()))
            }
            binary::Outcome::Error(code) => Err(self.failure(ConcreteWitnessError::Read(code))),
            _ => Err(self.failure(ConcreteWitnessError::Schema)),
        }
    }
}
fn tuple_proof<S: PhysicalTrieStore>(
    store: &S,
    address: [u8; 20],
    key: ConcreteStorageKey,
) -> Result<(ConcreteWitnessPhysical, PathProof), ConcreteReadError> {
    let id = store.identity();
    let selected = match store.value("5", codec::storage_version_prefix(address, key), id.period) {
        Ok(value) => value,
        Err(ConcreteReadError::HistoryUnavailable(observed)) if observed == id => None,
        Err(error) => return Err(error),
    };
    let physical = match selected {
        None => ConcreteWitnessPhysical::HistoryUnavailable(id),
        Some(v) if v.value.is_empty() => ConcreteWitnessPhysical::Tombstone,
        Some(v) => ConcreteWitnessPhysical::Present(v.value),
    };
    let account_prefix = codec::account_version_prefix(address);
    let selected = store.value("3", account_prefix, id.period)?;
    let proof = verify_path(
        store,
        id.state_root,
        account_prefix,
        "2",
        "3",
        |p| p,
        TrieSchema::Account,
    )?;
    let storage_root = match (proof, selected) {
        (PathProof::Member(proved), Some(selected))
            if !proved.is_empty() && proved == selected.value =>
        {
            codec::account_view(&proved)?.storage_root
        }
        (PathProof::NonMember, None) => None,
        (PathProof::NonMember, Some(selected)) if selected.value.is_empty() => None,
        _ => return Err(codec::corrupt("witness account reconciliation failed")),
    };
    let proof = if let Some(root) = storage_root {
        verify_path(
            store,
            root,
            codec::storage_trie_path(key),
            "4",
            "5",
            |p| codec::storage_prefix_for_path(address, p),
            TrieSchema::Storage,
        )?
    } else {
        PathProof::NonMember
    };
    Ok((physical, proof))
}
fn reconcile(
    expected: ConcreteStateIdentity,
    physical: ConcreteWitnessPhysical,
    proof: PathProof,
) -> Result<(ConcreteWitnessPhysical, ConcreteWitnessLogical), ConcreteWitnessError> {
    match (&physical, proof) {
        (ConcreteWitnessPhysical::Present(raw), PathProof::Member(proved)) if *raw == proved => {
            Ok((physical, ConcreteWitnessLogical::Member(proved)))
        }
        (ConcreteWitnessPhysical::Tombstone, PathProof::NonMember) => {
            Ok((physical, ConcreteWitnessLogical::NonMember))
        }
        (ConcreteWitnessPhysical::HistoryUnavailable(observed), PathProof::NonMember)
            if *observed == expected =>
        {
            Ok((physical, ConcreteWitnessLogical::NonMember))
        }
        _ => Err(ConcreteWitnessError::Reconciliation),
    }
}
/// Fresh bounded offline verification. The expected tuple/root is external test
/// authority, not a witness verdict or sealed historical owner. All consumed
/// account and storage dependencies must reconstruct the exact expected root.
pub fn replay_storage_witness(
    bytes: &[u8],
    expected_identity: ConcreteStateIdentity,
    address: [u8; 20],
    key: ConcreteStorageKey,
    limits: ConcreteWitnessLimits,
) -> Result<ConcreteWitnessReplay, ConcreteWitnessError> {
    let budget = ConcreteWitnessBudget::new(limits);
    let witness = ConcreteStorageWitness::admit_budget(
        bytes,
        &budget,
        Some((expected_identity, address, key)),
    )?;
    if witness.header.identity != expected_identity
        || witness.header.address != address
        || witness.header.key != key
    {
        return Err(budget.stop(ConcreteWitnessError::Identity));
    }
    budget.tuple(expected_identity, address, key)?;
    // Existing verifier errors use fixed labels and fixed-width hashes. Reserve
    // 1024 bytes before that path; no decoded variable error text enters it.
    budget.charge(R::CanonicalBytes, 1024)?;
    let store = ReplayStore {
        witness: &witness,
        budget: &budget,
        offset: Cell::new(binary::HEADER_BYTES),
        ordinal: Cell::new(0),
    };
    let (physical, proof) = tuple_proof(&store, address, key).map_err(|_| {
        budget
            .error()
            .unwrap_or_else(|| budget.stop(ConcreteWitnessError::Proof))
    })?;
    let (physical, logical) =
        reconcile(expected_identity, physical, proof).map_err(|e| budget.stop(e))?;
    if store.offset.get() != bytes.len() || store.ordinal.get() != u64::from(witness.header.records)
    {
        return Err(budget.stop(ConcreteWitnessError::UnusedRecords));
    }
    Ok(ConcreteWitnessReplay {
        identity: expected_identity,
        address,
        key,
        physical,
        logical,
        usage: budget.usage(),
    })
}

#[cfg(test)]
#[path = "witness_tests.rs"]
mod tests;
