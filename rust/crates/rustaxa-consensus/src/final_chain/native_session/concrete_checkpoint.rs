//! Immutable concrete-checkpoint fallback for FinalChain native state reads.
//!
//! This adapter selects one exact identity from a read-only
//! [`ConcreteCheckpointReaders`] handle. Account reads authenticate the selected
//! state-root path and preserve arbitrary-width nonce and balance values. Raw
//! reads preserve the concrete database's physical version selection, including
//! tombstones, orphan rows outside the live storage trie, and unavailable
//! historical rows.
//!
//! The adapter is only a checkpoint fallback. It does not observe earlier
//! same-period journal mutations, prove a complete native-key catalog, publish
//! state, or authorize database adoption. Direct reads are valid only while the
//! current execution state equals the checkpoint. Whenever an earlier execution
//! effect exists, including a frame-entry value transfer before the first native
//! invocation, the caller must layer its current account/raw journal over this
//! immutable source. The existing staged DPoS account port supplies the
//! invocation-local touched-account overlay and ordered full-width account
//! effects used by the native session.

use super::{FinalChainNativeAccount, FinalChainNativeStateRead, FinalChainNativeStateReadError};
use num_bigint::{BigInt, Sign};
use rustaxa_storage::ConcreteCheckpointReaders;
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStorageKey,
};

/// Exact read operations required from a concrete checkpoint source.
///
/// The private port keeps tests independent from RocksDB while the public
/// constructor accepts only the storage-owned authenticated reader.
trait CheckpointRead {
    fn retains(&self, identity: ConcreteStateIdentity) -> bool;

    fn account_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError>;

    fn storage_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError>;
}

impl CheckpointRead for ConcreteCheckpointReaders {
    fn retains(&self, identity: ConcreteStateIdentity) -> bool {
        self.retained_identities().contains(&identity)
    }

    fn account_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        ConcreteCheckpointReaders::account_at(self, identity, address)
    }

    fn storage_at(
        &self,
        identity: ConcreteStateIdentity,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        ConcreteCheckpointReaders::storage_at(self, identity, address, key)
    }
}

/// Native state reader pinned to one authenticated concrete checkpoint.
///
/// Construction accepts only an identity listed by the supplied checkpoint
/// reader. Historical identity provenance still belongs to the caller that
/// opened [`ConcreteCheckpointReaders`]; this type does not link a root to a
/// finalized header. Reads are immutable and have no persistence or publication
/// authority.
pub struct ConcreteCheckpointNativeStateRead<'a> {
    source: &'a dyn CheckpointRead,
    identity: ConcreteStateIdentity,
}

impl std::fmt::Debug for ConcreteCheckpointNativeStateRead<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConcreteCheckpointNativeStateRead")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl<'a> ConcreteCheckpointNativeStateRead<'a> {
    /// Selects one exact identity already accepted by `readers`.
    ///
    /// An unlisted period/root pair is `HistoryUnavailable`; roots with the
    /// same period are not interchangeable. The constructor does no database
    /// writes and does not turn the supplied retained set into a completeness
    /// claim.
    pub fn new(
        readers: &'a ConcreteCheckpointReaders,
        identity: ConcreteStateIdentity,
    ) -> Result<Self, FinalChainNativeStateReadError> {
        Self::from_source(readers, identity)
    }

    /// Returns the exact period/root selected by this adapter.
    pub fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn from_source(
        source: &'a dyn CheckpointRead,
        identity: ConcreteStateIdentity,
    ) -> Result<Self, FinalChainNativeStateReadError> {
        if !source.retains(identity) {
            return Err(ConcreteReadError::HistoryUnavailable(identity).into());
        }
        Ok(Self { source, identity })
    }
}

impl FinalChainNativeStateRead for ConcreteCheckpointNativeStateRead<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        let read = self.source.account_at(self.identity, address)?;
        Ok(match read {
            ConcreteRead::Present(record) => FinalChainNativeAccount {
                exists: true,
                nonce: record.account.nonce,
                balance: BigInt::from_bytes_be(
                    Sign::Plus,
                    &record.account.balance.value().to_bytes_be(),
                ),
            },
            ConcreteRead::Absent | ConcreteRead::Tombstone => FinalChainNativeAccount {
                exists: false,
                nonce: Default::default(),
                balance: Default::default(),
            },
        })
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        self.source
            .storage_at(self.identity, address, *key)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::final_chain::native_session::account::{DposAccountPort, StagedDposAccountPort};
    use num_bigint::BigUint;
    use rustaxa_types::concrete_state::{ConcreteAccount, ConcreteAccountBalance};
    use rustaxa_types::{FinalChainBlockNumber, FinalChainNonce};
    use std::cell::Cell;

    const ADDRESS: [u8; 20] = [0x44; 20];
    const SLOT: ConcreteStorageKey = ConcreteStorageKey([0x55; 32]);

    struct FixtureRead {
        retained: Vec<ConcreteStateIdentity>,
        account: Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError>,
        storage: Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
        account_reads: Cell<u64>,
        storage_reads: Cell<u64>,
    }

    impl FixtureRead {
        fn new(identity: ConcreteStateIdentity) -> Self {
            Self {
                retained: vec![identity],
                account: Ok(ConcreteRead::Absent),
                storage: Err(ConcreteReadError::HistoryUnavailable(identity)),
                account_reads: Cell::new(0),
                storage_reads: Cell::new(0),
            }
        }
    }

    impl CheckpointRead for FixtureRead {
        fn retains(&self, identity: ConcreteStateIdentity) -> bool {
            self.retained.contains(&identity)
        }

        fn account_at(
            &self,
            identity: ConcreteStateIdentity,
            address: [u8; 20],
        ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
            assert!(self.retained.contains(&identity));
            assert_eq!(address, ADDRESS);
            self.account_reads.set(self.account_reads.get() + 1);
            self.account.clone()
        }

        fn storage_at(
            &self,
            identity: ConcreteStateIdentity,
            address: [u8; 20],
            key: ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
            assert!(self.retained.contains(&identity));
            assert_eq!(address, ADDRESS);
            assert_eq!(key, SLOT);
            self.storage_reads.set(self.storage_reads.get() + 1);
            self.storage.clone()
        }
    }

    fn identity(period: u64, root_byte: u8) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(period),
            state_root: [root_byte; 32],
        }
    }

    fn record(nonce: FinalChainNonce, balance: BigUint) -> ConcreteAccountRecord {
        ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce,
                balance: ConcreteAccountBalance::new(balance),
                storage_root: Some([7; 32]),
                code_hash: Some([8; 32]),
                code_size: 9,
            },
            physical_rlp: vec![0xca, 0xfe],
        }
    }

    #[test]
    fn exact_identity_is_required_before_any_read() {
        let retained = identity(8, 1);
        let fixture = FixtureRead::new(retained);
        let mismatched_root = identity(8, 2);

        assert_eq!(
            ConcreteCheckpointNativeStateRead::from_source(&fixture, mismatched_root).unwrap_err(),
            FinalChainNativeStateReadError::State(ConcreteReadError::HistoryUnavailable(
                mismatched_root
            ))
        );
        assert_eq!(fixture.account_reads.get(), 0);
        assert_eq!(fixture.storage_reads.get(), 0);
    }

    #[test]
    fn account_read_and_staged_overlay_preserve_full_width_values() {
        let selected = identity(9, 3);
        let nonce = FinalChainNonce::from_bytes(&[1; 40]).unwrap();
        let balance = (BigUint::from(1_u8) << 400_usize) + BigUint::from(17_u8);
        let mut fixture = FixtureRead::new(selected);
        fixture.account = Ok(ConcreteRead::Present(record(
            nonce.clone(),
            balance.clone(),
        )));
        let read = ConcreteCheckpointNativeStateRead::from_source(&fixture, selected).unwrap();
        let mut working = StagedDposAccountPort::from_state(&read);

        assert_eq!(
            working.account(ADDRESS).unwrap(),
            FinalChainNativeAccount {
                exists: true,
                nonce,
                balance: BigInt::from(balance.clone()),
            }
        );
        working
            .subtract_balance(ADDRESS, &BigUint::from(7_u8))
            .unwrap();
        assert_eq!(
            working.account(ADDRESS).unwrap().balance,
            BigInt::from(balance) - BigInt::from(7_u8)
        );
        assert_eq!(fixture.account_reads.get(), 1);
        assert_eq!(working.into_mutations().len(), 1);
    }

    #[test]
    fn absent_and_tombstoned_accounts_are_canonical_missing_facts() {
        let selected = identity(10, 4);
        for classified in [ConcreteRead::Absent, ConcreteRead::Tombstone] {
            let mut fixture = FixtureRead::new(selected);
            fixture.account = Ok(classified);
            let read = ConcreteCheckpointNativeStateRead::from_source(&fixture, selected).unwrap();

            assert_eq!(
                read.account(ADDRESS).unwrap(),
                FinalChainNativeAccount {
                    exists: false,
                    nonce: FinalChainNonce::zero(),
                    balance: BigInt::default(),
                }
            );
        }
    }

    #[test]
    fn account_read_errors_are_not_converted_to_absence() {
        let selected = identity(11, 5);
        let error = ConcreteReadError::Corrupt("missing account trie child".into());
        let mut fixture = FixtureRead::new(selected);
        fixture.account = Err(error.clone());
        let read = ConcreteCheckpointNativeStateRead::from_source(&fixture, selected).unwrap();

        assert_eq!(
            read.account(ADDRESS).unwrap_err(),
            FinalChainNativeStateReadError::State(error)
        );
    }

    #[test]
    fn raw_reads_preserve_physical_classification_and_unavailability() {
        let selected = identity(12, 6);
        let cases = [
            Ok(ConcreteRead::Present(vec![1, 2, 3])),
            Ok(ConcreteRead::Absent),
            Ok(ConcreteRead::Tombstone),
            Err(ConcreteReadError::HistoryUnavailable(selected)),
        ];

        for expected in cases {
            let mut fixture = FixtureRead::new(selected);
            fixture.storage = expected.clone();
            let read = ConcreteCheckpointNativeStateRead::from_source(&fixture, selected).unwrap();
            let observed = read.raw_storage(ADDRESS, &SLOT);
            let expected = expected.map_err(FinalChainNativeStateReadError::State);

            assert_eq!(observed, expected);
            assert_eq!(fixture.storage_reads.get(), 1);
        }
    }
}
