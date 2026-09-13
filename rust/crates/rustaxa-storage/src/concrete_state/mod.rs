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
mod trie_writer;
mod writer;

pub use codec::{
    account_commitment_rlp, account_version_prefix, decode_physical_account,
    storage_version_prefix, versioned_key,
};
pub use lifecycle::{
    ConcreteCommitApproval, ConcreteLifecycleObservation, ConcreteObserverAccountChange,
    ConcreteObserverPhaseOutput, ConcreteStateLifecycle,
};
pub use reader::{ConcreteStateReader, ConcreteStoragePath};
pub use writer::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteObserverPhaseDelta,
    ConcreteStateMutationBatch, ConcreteStateWriter, ConcreteStorageMutation,
    PreparedConcreteState, PreparedConcreteView,
};
