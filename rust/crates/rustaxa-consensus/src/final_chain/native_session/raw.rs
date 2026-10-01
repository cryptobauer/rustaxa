//! Ordered exact raw-storage effects emitted by staged native kernels.
//!
//! Native storage writes bypass the ordinary EVM checkpoint lane in the pinned
//! implementation. These types preserve their operation order and the
//! absent/tombstone/value observation that preceded each write. Deletion is an
//! explicit operation because empty bytes mean deletion at this boundary.

use super::*;
use std::collections::BTreeMap;

/// Exact nonempty bytes for a staged native raw-storage put.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRawValue(Vec<u8>);

impl FinalChainNativeRawValue {
    /// Constructs a raw put value.
    ///
    /// Empty bytes are rejected because the reference interprets them as a
    /// deletion rather than as stored bytes.
    pub fn new(value: Vec<u8>) -> Result<Self, FinalChainNativeRawValueError> {
        if value.is_empty() {
            Err(FinalChainNativeRawValueError)
        } else {
            Ok(Self(value))
        }
    }

    /// Borrows the exact bytes written by the native serializer.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the value and returns its exact bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Empty bytes cannot be represented as a staged native raw put.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRawValueError;

impl std::fmt::Display for FinalChainNativeRawValueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("empty FinalChain native raw value denotes deletion")
    }
}

impl std::error::Error for FinalChainNativeRawValueError {}

/// One exact native raw-storage operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeRawOperation {
    /// Store the supplied nonempty exact bytes.
    Put(FinalChainNativeRawValue),
    /// Delete the logical row by writing empty bytes to the current raw lane.
    /// The later storage writer decides whether this requires a physical
    /// tombstone or is a no-op for a row that was already absent.
    Delete,
}

/// One ordered raw mutation emitted by a staged native business transition.
///
/// `expected` binds this operation to the current journal overlay immediately
/// before the operation. Repeated operations for the same key therefore carry
/// the preceding operation's result rather than a collapsed final-map diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRawMutation {
    /// Native contract whose storage trie owns the row.
    pub address: [u8; 20],
    /// Logical unhashed storage key.
    pub key: ConcreteStorageKey,
    /// Exact classified raw value observed before this operation.
    pub expected: ConcreteRead<Vec<u8>>,
    /// Exact put or delete operation to apply in vector order.
    pub operation: FinalChainNativeRawOperation,
}

/// Invocation-local ordered raw overlay used by operation-owned serializers.
///
/// The first access to a key reads the caller's current journal view. Later
/// accesses observe preceding writes in this builder, allowing repeated writes
/// to one key to retain exact intermediate expectations.
pub(super) struct FinalChainNativeRawTrace<'a> {
    state: &'a dyn FinalChainNativeStateRead,
    current: BTreeMap<([u8; 20], ConcreteStorageKey), ConcreteRead<Vec<u8>>>,
    mutations: Vec<FinalChainNativeRawMutation>,
}

impl<'a> FinalChainNativeRawTrace<'a> {
    /// Starts an empty trace against the authoritative current journal view.
    pub(super) fn new(state: &'a dyn FinalChainNativeStateRead) -> Self {
        Self {
            state,
            current: BTreeMap::new(),
            mutations: Vec::new(),
        }
    }

    /// Returns the current logical raw value, loading each key at most once.
    pub(super) fn current(
        &mut self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeSessionError> {
        if let Some(current) = self.current.get(&(address, key)) {
            return Ok(current.clone());
        }
        let current = self.state.raw_storage(address, &key)?;
        self.current.insert((address, key), current.clone());
        Ok(current)
    }

    /// Appends one reference `Put`, converting empty bytes to explicit delete.
    pub(super) fn put(
        &mut self,
        address: [u8; 20],
        key: ConcreteStorageKey,
        value: Vec<u8>,
    ) -> Result<(), FinalChainNativeSessionError> {
        let expected = self.current(address, key)?;
        let operation = if value.is_empty() {
            FinalChainNativeRawOperation::Delete
        } else {
            FinalChainNativeRawOperation::Put(
                FinalChainNativeRawValue::new(value.clone())
                    .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?,
            )
        };
        self.mutations.push(FinalChainNativeRawMutation {
            address,
            key,
            expected,
            operation,
        });
        self.current
            .insert((address, key), ConcreteRead::Present(value));
        Ok(())
    }

    /// Completes the trace without collapsing repeated-key operations.
    pub(super) fn finish(self) -> Vec<FinalChainNativeRawMutation> {
        self.mutations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_types::FinalChainBlockNumber;
    use rustaxa_types::concrete_state::ConcreteStateIdentity;
    use std::cell::RefCell;

    struct DeniedRawReads {
        error: FinalChainNativeStateReadError,
        attempts: RefCell<Vec<([u8; 20], ConcreteStorageKey)>>,
    }

    impl FinalChainNativeStateRead for DeniedRawReads {
        fn raw_storage(
            &self,
            address: [u8; 20],
            key: &ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            self.attempts.borrow_mut().push((address, *key));
            Err(self.error.clone())
        }
    }

    #[test]
    fn bounded_transition_raw_reads_and_writes_reject_unavailable_state_without_staging() {
        // This identity labels an unavailable synthetic view, not an authenticated root.
        let identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(7),
            state_root: [0x71; 32],
        };
        for error in [
            FinalChainNativeStateReadError::State(ConcreteReadError::HistoryUnavailable(identity)),
            FinalChainNativeStateReadError::Invariant(
                "bounded raw reads are unavailable".to_owned(),
            ),
        ] {
            let reader = DeniedRawReads {
                error: error.clone(),
                attempts: RefCell::new(Vec::new()),
            };
            let address = [3; 20];
            let key = ConcreteStorageKey([4; 32]);
            let mut trace = FinalChainNativeRawTrace::new(&reader);
            let initial_current = trace.current.clone();
            let initial_mutations = trace.mutations.clone();

            // Repeat every operation against the same key: an error must not
            // become cached absence, a value, or a successful mutation.
            for _ in 0..2 {
                assert_eq!(
                    trace.current(address, key),
                    Err(FinalChainNativeSessionError::StateRead(error.clone()))
                );
                assert_eq!(trace.current, initial_current);
                assert_eq!(trace.mutations, initial_mutations);
                for value in [vec![9], Vec::new()] {
                    assert_eq!(
                        trace.put(address, key, value),
                        Err(FinalChainNativeSessionError::StateRead(error.clone()))
                    );
                    assert_eq!(trace.current, initial_current);
                    assert_eq!(trace.mutations, initial_mutations);
                }
            }
            assert_eq!(*reader.attempts.borrow(), vec![(address, key); 6]);
            assert_eq!(trace.finish(), initial_mutations);
        }
    }

    #[test]
    fn bounded_transition_raw_classifications_remain_distinct_from_read_failure() {
        struct ClassifiedRow(ConcreteRead<Vec<u8>>);

        impl FinalChainNativeStateRead for ClassifiedRow {
            fn raw_storage(
                &self,
                _address: [u8; 20],
                _key: &ConcreteStorageKey,
            ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
                Ok(self.0.clone())
            }
        }

        // The reader owns the coverage proof for Absent. The trace must
        // preserve that classification, tombstones, and actual zero bytes.
        for classified in [
            ConcreteRead::Absent,
            ConcreteRead::Tombstone,
            ConcreteRead::Present(Vec::new()),
            ConcreteRead::Present(vec![0]),
        ] {
            let reader = ClassifiedRow(classified.clone());
            let address = [3; 20];
            let key = ConcreteStorageKey([4; 32]);
            let mut trace = FinalChainNativeRawTrace::new(&reader);
            assert_eq!(trace.current(address, key).unwrap(), classified);
            trace.put(address, key, vec![9]).unwrap();
            assert_eq!(trace.finish()[0].expected, classified);
        }
    }

    struct OneRow;

    impl FinalChainNativeStateRead for OneRow {
        fn raw_storage(
            &self,
            _address: [u8; 20],
            _key: &ConcreteStorageKey,
        ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            Ok(ConcreteRead::Present(vec![7]))
        }
    }

    #[test]
    fn trace_retains_delete_and_repeated_key_intermediate_expectations() {
        assert!(FinalChainNativeRawValue::new(Vec::new()).is_err());
        let address = [3; 20];
        let key = ConcreteStorageKey([4; 32]);
        let mut trace = FinalChainNativeRawTrace::new(&OneRow);

        trace.put(address, key, Vec::new()).unwrap();
        trace.put(address, key, vec![9]).unwrap();
        let mutations = trace.finish();

        assert_eq!(mutations[0].expected, ConcreteRead::Present(vec![7]));
        assert_eq!(mutations[0].operation, FinalChainNativeRawOperation::Delete);
        assert_eq!(mutations[1].expected, ConcreteRead::Present(Vec::new()));
        assert_eq!(
            mutations[1].operation,
            FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(vec![9]).unwrap())
        );
    }
}
