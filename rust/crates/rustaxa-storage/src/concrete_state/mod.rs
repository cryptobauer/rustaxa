//! Compatible read-only access to Taraxa's concrete state database.
//!
//! The reader preserves the existing separate RocksDB layout and pins the
//! descriptor supplied by FinalChain. Physical codecs keep arbitrary-width
//! account values and exact persisted RLP. Account reads authenticate their
//! trie path. Raw storage reads preserve selected physical history, while a
//! separate diagnostic authenticates logical slot membership. This module does
//! not create, migrate, repair, publish, or route a concrete database.

mod codec;
mod physical_node;
mod reader;

pub use codec::{
    account_commitment_rlp, account_version_prefix, decode_physical_account,
    storage_version_prefix, versioned_key,
};
pub use reader::{ConcreteStateReader, ConcreteStoragePath};
