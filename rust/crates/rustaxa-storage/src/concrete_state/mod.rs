//! Compatible access to Taraxa's concrete state database.
//!
//! The reader preserves the existing separate RocksDB layout and pins the
//! descriptor supplied by FinalChain. Physical codecs keep arbitrary-width
//! account values and exact persisted RLP. Account reads authenticate their
//! trie path. Raw storage reads preserve selected physical history, while a
//! separate diagnostic authenticates logical slot membership. This module does
//! not migrate, repair, publish, or route a concrete database. The writer can
//! stage compatible content rows, but durable descriptor/lifecycle publication
//! remains an explicit FinalChain integration boundary.

mod codec;
mod lifecycle;
mod physical_node;
mod reader;
mod snapshot;
mod trie_writer;
mod witness_rlp;
mod writer;

pub use codec::{
    account_commitment_rlp, account_version_prefix, decode_physical_account,
    storage_version_prefix, versioned_key,
};
pub use lifecycle::{
    CertifiedConcreteCommit, ConcreteCommitApproval, ConcreteLifecycleObservation,
    ConcreteObserverAccountChange, ConcreteObserverPhaseOutput, ConcreteStateLifecycle,
    ConcreteTransitionBuilder, PreparedConcreteTransition,
};
#[cfg(feature = "synthetic-fixtures")]
pub use lifecycle::{ConcreteCommittedPut, ConcreteCommittedReceipt};
pub use reader::{
    ConcreteCheckpointReaders, ConcreteStateReader, ConcreteStorageInventory,
    ConcreteStorageInventoryEntry, ConcreteStorageInventoryError, ConcreteStorageInventoryLimits,
    ConcreteStorageInventoryResource, ConcreteStoragePath,
};
pub use writer::{
    BorrowedConcreteExecutionProbe, ConcreteAccountMutation, ConcreteCodeInsertion,
    ConcreteExecutionProbe, ConcreteExecutionProbeView, ConcreteObserverPhaseDelta,
    ConcreteStateMutationBatch, ConcreteStateWriter, ConcreteStorageMutation,
    HistoricalConcreteAccountChange, HistoricalConcretePreparation,
    HistoricalConcretePreparationSummary, PreparedConcreteState, PreparedConcreteView,
};
