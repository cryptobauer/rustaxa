use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeConsensusApplication;
use crate::ffi::BridgeDagStorageQueries;
use crate::ffi::BridgeFinalChainStorageQueries;
use crate::ffi::BridgePbftStorageQueries;
use crate::ffi::BridgePbftVoteStorageQueries;
use crate::ffi::BridgePeriodStorageQueries;
use crate::ffi::BridgeStorageBatch;
use crate::ffi::BridgeTransactionStorageQueries;
#[cfg(test)]
use anyhow::Context;
use anyhow::Result;
use ethereum_types::H256;
use rustaxa_consensus::proposed_blocks::{
    restore_proposed_blocks_from_storage, save_proposed_block_storage,
};
use rustaxa_consensus::{
    load_current_pillar_block_data_storage as consensus_load_current_pillar_block_data_storage,
    load_latest_pillar_block_storage as consensus_load_latest_pillar_block_storage,
    load_own_pillar_block_vote_storage as consensus_load_own_pillar_block_vote_storage,
    save_current_pillar_block_data_storage, save_finalized_pillar_block_storage,
    save_own_pillar_block_vote_storage,
};
use rustaxa_storage::Storage;
use std::sync::Arc;

fn runtime_storage(runtime: &BridgeConsensusApplication) -> Arc<Storage> {
    runtime.0.storage_for_bridge().clone()
}

/// Creates a typed PBFT vote-list query handle from the shared Rust storage owner.
///
/// Inputs:
/// - `storage`: generic bridge storage owner used only as a construction-time
///   lifetime seed.
///
/// Outputs:
/// - a read-only PBFT vote query handle that owns a cloned Rust storage handle.
///
/// Invariants and edge behavior:
/// - callers can materialize legacy C++ `PbftVote` objects without retaining a
///   broad `BridgeStorage` query surface for vote-list reads
/// - the handle does not mutate storage or decode votes.
pub fn create_pbft_vote_storage_queries(
    runtime: &BridgeConsensusApplication,
) -> Box<BridgePbftVoteStorageQueries> {
    Box::new(BridgePbftVoteStorageQueries {
        storage: runtime_storage(runtime),
    })
}

/// Creates a typed PBFT scalar/head query handle from the shared Rust storage owner.
///
/// Inputs:
/// - `storage`: generic bridge storage owner used only as a construction-time
///   lifetime seed.
///
/// Outputs:
/// - a read-only PBFT query handle that owns a cloned Rust storage handle.
///
/// Invariants and edge behavior:
/// - callers can inspect PBFT scalar/head compatibility rows without retaining
///   broad `BridgeStorage` read methods
/// - the handle does not mutate storage or decode PBFT block objects.
pub fn create_pbft_storage_queries(
    runtime: &BridgeConsensusApplication,
) -> Box<BridgePbftStorageQueries> {
    Box::new(BridgePbftStorageQueries {
        storage: runtime_storage(runtime),
    })
}

/// Creates a typed DAG query handle from the shared Rust storage owner.
///
/// Inputs:
/// - `storage`: generic bridge storage owner used only as a construction-time
///   lifetime seed.
///
/// Outputs:
/// - a read-only DAG query handle that owns a cloned Rust storage handle.
///
/// Invariants and edge behavior:
/// - callers can materialize legacy DAG objects and indexes at public/query
///   boundaries without retaining broad `BridgeStorage` DAG reads
/// - the handle does not mutate storage or decode DAG block payloads.
pub fn create_dag_storage_queries(
    runtime: &BridgeConsensusApplication,
) -> Box<BridgeDagStorageQueries> {
    Box::new(BridgeDagStorageQueries {
        storage: runtime_storage(runtime),
    })
}

/// Creates a typed transaction query handle from the shared Rust storage owner.
///
/// Inputs:
/// - `storage`: generic bridge storage owner used only as a construction-time
///   lifetime seed.
///
/// Outputs:
/// - a read-only transaction query handle that owns a cloned Rust storage handle.
///
/// Invariants and edge behavior:
/// - C++ callers can keep materializing legacy transaction objects at public
///   API boundaries without retaining broad `BridgeStorage` transaction reads
/// - the handle does not mutate storage or decode transaction payloads.
pub fn create_transaction_storage_queries(
    runtime: &BridgeConsensusApplication,
) -> Box<BridgeTransactionStorageQueries> {
    Box::new(BridgeTransactionStorageQueries {
        storage: runtime_storage(runtime),
    })
}

/// Creates a typed FinalChain lookup query handle from the shared Rust storage
/// owner.
///
/// Inputs:
/// - `storage`: generic bridge storage owner used only as a construction-time
///   lifetime seed.
///
/// Outputs:
/// - a read-only FinalChain lookup handle that owns a cloned Rust storage
///   handle.
///
/// Invariants and edge behavior:
/// - callers can resolve FinalChain compatibility rows without retaining broad
///   `BridgeStorage` lookup methods.
/// - the handle does not mutate storage.
pub fn create_final_chain_storage_queries(
    runtime: &BridgeConsensusApplication,
) -> Box<BridgeFinalChainStorageQueries> {
    Box::new(BridgeFinalChainStorageQueries {
        storage: runtime_storage(runtime),
    })
}

/// Creates a typed period lookup query handle from the shared Rust storage owner.
///
/// Inputs:
/// - `storage`: generic bridge storage owner used only as a construction-time
///   lifetime seed.
///
/// Outputs:
/// - a read-only period lookup query handle that owns a cloned Rust storage
///   handle.
///
/// Invariants and edge behavior:
/// - callers can resolve period rows for compatibility materialization without
///   retaining broad `BridgeStorage` period read methods.
/// - the handle does not mutate storage.
pub fn create_period_storage_queries(
    runtime: &BridgeConsensusApplication,
) -> Box<BridgePeriodStorageQueries> {
    Box::new(BridgePeriodStorageQueries {
        storage: runtime_storage(runtime),
    })
}

/// Creates a Rust-owned storage batch for the C++ `DbStorage` shim.
///
/// The returned object owns a native `rustaxa-storage` write batch and the shared
/// storage handle needed to append and commit it. This replaces the previous
/// bridge-global integer batch registry while the public C++ `Batch&` surface is
/// still being retired.
pub fn create_storage_shim_batch(runtime: &BridgeConsensusApplication) -> Box<BridgeStorageBatch> {
    let storage = runtime_storage(runtime);
    Box::new(BridgeStorageBatch {
        storage: storage.clone(),
        batch: Some(storage.create_write_batch()),
    })
}

impl BridgeConsensusApplication {
    /// Loads the canonical genesis-hash metadata bytes for `DbStorage` compatibility.
    ///
    /// Returns empty bytes when the row is missing and propagates native storage errors.
    /// The method is read-only and preserves the stored bytes without decoding them.
    pub fn get_genesis_hash(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .storage_for_bridge()
            .metadata()
            .genesis_hash()?
            .unwrap_or_default())
    }

    /// Loads at most `count` latest sortition-parameter change RLP payloads.
    ///
    /// Results preserve native storage ordering and canonical bytes. A CXX `u64`
    /// count that cannot fit `usize` saturates to `usize::MAX`; storage errors propagate.
    pub fn get_last_sortition_params(
        &self,
        count: u64,
    ) -> Result<Vec<rustaxa_ffi::BlockRlp>, anyhow::Error> {
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        let changes = self
            .0
            .storage_for_bridge()
            .metadata()
            .last_sortition_params_changes_rlp(count)?;
        Ok(changes
            .into_iter()
            .map(|data| rustaxa_ffi::BlockRlp { data })
            .collect())
    }

    /// Loads the canonical sortition-parameter change RLP for `period`.
    ///
    /// Returns empty bytes when no change exists, performs no decoding or mutation,
    /// and propagates native storage errors unchanged.
    pub fn get_params_change_for_period(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .storage_for_bridge()
            .metadata()
            .params_change_for_period_rlp(period)?
            .unwrap_or_default())
    }

    /// Loads one numeric compatibility status field selected by its stable field code.
    ///
    /// Unknown fields and storage failures are returned as native errors; this method
    /// neither supplies defaults nor mutates metadata.
    pub fn get_status_field(&self, field: u8) -> Result<u64, anyhow::Error> {
        self.0.storage_for_bridge().metadata().status_field(field)
    }

    /// Loads the dynamic lambda for `period`, optionally selecting the closest row.
    ///
    /// The result distinguishes absence with `found = false` and a zero placeholder.
    /// Closest-period semantics and storage errors are owned by native storage.
    pub fn get_period_lambda(
        &self,
        period: u64,
        find_closest: bool,
    ) -> Result<rustaxa_ffi::PeriodLambda, anyhow::Error> {
        let value = self
            .0
            .storage_for_bridge()
            .metadata()
            .period_lambda(period, find_closest)?;
        Ok(match value {
            Some(value) => rustaxa_ffi::PeriodLambda { found: true, value },
            None => rustaxa_ffi::PeriodLambda {
                found: false,
                value: 0,
            },
        })
    }

    /// Loads the persisted dynamic-lambda rounds counter.
    ///
    /// The scalar is returned exactly as stored; missing/malformed rows and storage
    /// failures retain the native metadata error behavior.
    pub fn get_rounds_count_dynamic_lambda(&self) -> Result<u32, anyhow::Error> {
        self.0
            .storage_for_bridge()
            .metadata()
            .rounds_count_dynamic_lambda()
    }

    /// Loads every persisted block-rewards statistics row as `(period, RLP)` payloads.
    ///
    /// Native storage determines ordering. Canonical RLP bytes are not decoded or
    /// rewritten, and any iteration or storage failure is propagated.
    pub fn get_blocks_rewards_stats(&self) -> Result<Vec<rustaxa_ffi::PeriodRlp>, anyhow::Error> {
        Ok(self
            .0
            .storage_for_bridge()
            .metadata()
            .block_rewards_stats_rlp()?
            .into_iter()
            .map(|(period, data)| rustaxa_ffi::PeriodRlp { period, data })
            .collect())
    }

    /// Persists current pillar-block sidecar data through consensus-owned storage.
    pub fn pillar_chain_storage_apply_current_block_data(&self, data_rlp: Vec<u8>) -> Result<()> {
        save_current_pillar_block_data_storage(self.0.storage_for_bridge().as_ref(), &data_rlp)
    }

    /// Persists this node's own pillar-block vote through consensus-owned storage.
    pub fn pillar_chain_storage_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        save_own_pillar_block_vote_storage(self.0.storage_for_bridge().as_ref(), &vote_rlp)
    }

    /// Persists a finalized pillar block through consensus-owned storage.
    pub fn pillar_chain_storage_apply_finalized_block(
        &self,
        period: u64,
        pillar_block_rlp: Vec<u8>,
    ) -> Result<()> {
        save_finalized_pillar_block_storage(
            self.0.storage_for_bridge().as_ref(),
            period,
            &pillar_block_rlp,
        )
    }

    /// Loads this node's own pillar-block vote bytes, returning empty bytes when
    /// no vote is stored.
    pub fn pillar_chain_storage_load_own_vote(&self) -> Result<Vec<u8>> {
        consensus_load_own_pillar_block_vote_storage(self.0.storage_for_bridge().as_ref())
    }

    /// Loads current pillar-block sidecar bytes, returning empty bytes when
    /// missing.
    pub fn pillar_chain_storage_load_current_block_data(&self) -> Result<Vec<u8>> {
        consensus_load_current_pillar_block_data_storage(self.0.storage_for_bridge().as_ref())
    }

    /// Loads the latest finalized pillar block bytes, returning empty bytes when
    /// no finalized pillar block is stored.
    pub fn pillar_chain_storage_load_latest_block(&self) -> Result<Vec<u8>> {
        consensus_load_latest_pillar_block_storage(self.0.storage_for_bridge().as_ref())
    }

    /// Loads a finalized pillar block by period, returning empty bytes when no
    /// block is stored for that period.
    pub fn pillar_chain_storage_load_block(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self
            .0
            .storage_for_bridge()
            .pillar()
            .rlp(period)?
            .unwrap_or_default())
    }
}

fn storage_shim_batch_mut(
    batch: &mut BridgeStorageBatch,
) -> Result<&mut rustaxa_storage::StorageWriteBatch, anyhow::Error> {
    batch
        .batch
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("storage shim batch already committed"))
}

fn vote_rlps_to_bridge(votes: Vec<Vec<u8>>) -> Vec<rustaxa_ffi::VoteRlp> {
    votes
        .into_iter()
        .map(|data| rustaxa_ffi::VoteRlp { data })
        .collect()
}

impl BridgePbftVoteStorageQueries {
    /// Returns locally stored verified vote RLPs from Rust PBFT storage.
    ///
    /// Inputs: none beyond the storage handle cloned into this typed query object.
    /// Outputs: canonical vote RLP bytes for C++ compatibility materialization.
    /// Edge behavior: missing storage rows return an empty vector.
    pub fn get_own_verified_votes(&self) -> Result<Vec<rustaxa_ffi::VoteRlp>, anyhow::Error> {
        Ok(vote_rlps_to_bridge(
            self.storage.pbft().own_verified_votes_rlp()?,
        ))
    }

    /// Returns flattened 2t+1 vote bundle RLPs in repository-defined order.
    ///
    /// Inputs: none beyond the cloned storage handle. Outputs are flattened
    /// vote RLPs, preserving the Rust repository's deterministic vote-type
    /// iteration order. Malformed stored bundle RLP returns an error.
    pub fn get_all_two_t_plus_one_votes(&self) -> Result<Vec<rustaxa_ffi::VoteRlp>, anyhow::Error> {
        Ok(vote_rlps_to_bridge(
            self.storage.pbft().all_two_t_plus_one_votes_rlp()?,
        ))
    }

    /// Returns extra reward vote RLPs from Rust PBFT storage.
    ///
    /// Inputs: none beyond the cloned storage handle. Outputs are canonical
    /// vote RLP bytes for C++ compatibility materialization. Missing rows
    /// return an empty vector.
    pub fn get_reward_votes(&self) -> Result<Vec<rustaxa_ffi::VoteRlp>, anyhow::Error> {
        Ok(vote_rlps_to_bridge(self.storage.pbft().reward_votes_rlp()?))
    }
}

impl BridgePbftStorageQueries {
    /// Returns whether a PBFT block hash resolves to a persisted finalized period.
    ///
    /// Inputs: canonical PBFT block hash bytes. Output is `true` only when the
    /// Rust PBFT repository can resolve the hash through the PBFT period index.
    /// Storage errors are returned to the caller.
    pub fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.storage.pbft().exists(H256::from(*hash))
    }

    /// Returns a PBFT manager numeric field or the legacy default.
    ///
    /// Inputs: legacy `PbftMgrField` numeric discriminant. Output preserves the
    /// legacy default of `1` when the row is absent.
    pub fn get_pbft_mgr_field(&self, field: u8) -> Result<u32, anyhow::Error> {
        Ok(self.storage.pbft().manager_field(field)?.unwrap_or(1))
    }

    /// Returns a PBFT manager boolean status or the legacy default.
    ///
    /// Inputs: legacy `PbftMgrStatus` numeric discriminant. Output preserves the
    /// legacy default of `false` when the row is absent.
    pub fn get_pbft_mgr_status(&self, field: u8) -> Result<bool, anyhow::Error> {
        Ok(self.storage.pbft().manager_status(field)?.unwrap_or(false))
    }

    /// Returns the persisted PBFT head payload for a hash.
    ///
    /// Inputs: canonical PBFT head hash bytes. Output is an empty vector when no
    /// row exists, matching the legacy C++ storage API.
    pub fn get_pbft_head(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .pbft()
            .head(H256::from(*hash))?
            .unwrap_or_default())
    }

    /// Returns the persisted cert-voted PBFT block payload in the legacy compact
    /// `[round, rlp]` encoding.
    ///
    /// Input is implicit via `self`; output is an empty vector when no payload is
    /// present, matching the previous `BridgeStorage` compatibility behavior.
    pub fn get_cert_voted_block_in_round(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .pbft()
            .cert_voted_block_in_round_rlp()?
            .unwrap_or_default())
    }

    /// Validates and persists one canonical proposed PBFT block for the legacy
    /// `DbStorage` compatibility client.
    ///
    /// The supplied period, block hash, and pivot hash must match the decoded
    /// signed block bytes. Success returns `true`; malformed bytes, identity
    /// mismatches, and storage failures are returned without publishing a
    /// process-local proposal state.
    pub fn save_proposed_pbft_block(
        &self,
        expected_period: u64,
        expected_hash: &[u8; 32],
        expected_pivot_hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<bool, anyhow::Error> {
        save_proposed_block_storage(
            self.storage.as_ref(),
            expected_period,
            H256::from(*expected_hash),
            H256::from(*expected_pivot_hash),
            block_rlp.as_slice(),
        )?;
        Ok(true)
    }

    /// Loads canonical proposed PBFT block bytes for legacy storage reads.
    ///
    /// Results preserve storage iteration order and contain only validated RLP
    /// payloads. Decode, key-identity, iterator, and storage failures are
    /// returned to C++; live proposal validation flags are intentionally not
    /// materialized by this compatibility query.
    pub fn get_proposed_pbft_blocks(&self) -> Result<Vec<rustaxa_ffi::BlockRlp>, anyhow::Error> {
        Ok(restore_proposed_blocks_from_storage(self.storage.as_ref())?
            .into_iter()
            .map(|entry| rustaxa_ffi::BlockRlp {
                data: entry.block_rlp,
            })
            .collect())
    }
}

impl BridgeDagStorageQueries {
    pub fn dag_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.storage
            .dag()
            .exists(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_dag_block(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .dag()
            .by_hash_rlp_optional(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?
            .unwrap_or_default())
    }

    pub fn get_dag_block_period_lookup(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::BlockPeriodLookup, anyhow::Error> {
        let lookup = self
            .storage
            .dag()
            .period_optional(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(match lookup {
            Some((period, position)) => rustaxa_ffi::BlockPeriodLookup {
                found: true,
                period,
                position,
            },
            None => rustaxa_ffi::BlockPeriodLookup {
                found: false,
                period: 0,
                position: 0,
            },
        })
    }

    pub fn get_last_blocks_level(&self) -> Result<u64, anyhow::Error> {
        self.storage
            .dag()
            .last_level()
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_blocks_by_level(&self, level: u64) -> Result<Vec<u8>, anyhow::Error> {
        let hashes = self
            .storage
            .dag()
            .hashes_at_level(level)
            .map_err(|e| anyhow::anyhow!(e))?;
        let mut bytes = Vec::with_capacity(hashes.len() * 32);
        for h in hashes {
            bytes.extend_from_slice(h.as_bytes());
        }
        Ok(bytes)
    }

    pub fn get_dag_blocks_at_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<rustaxa_ffi::BlockRlp>, anyhow::Error> {
        let rlps = self
            .storage
            .dag()
            .at_level_range(level, number_of_levels)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(rlps
            .into_iter()
            .map(|data| rustaxa_ffi::BlockRlp { data })
            .collect())
    }

    pub fn get_nonfinalized_dag_blocks(
        &self,
    ) -> Result<Vec<rustaxa_ffi::LevelBlocks>, anyhow::Error> {
        let map = self
            .storage
            .dag()
            .non_finalized()
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(map
            .into_iter()
            .map(|(level, blocks)| rustaxa_ffi::LevelBlocks {
                level,
                blocks: blocks
                    .into_iter()
                    .map(|data| rustaxa_ffi::BlockRlp { data })
                    .collect(),
            })
            .collect())
    }

    pub fn get_proposal_period_for_dag_level(
        &self,
        level: u64,
    ) -> Result<rustaxa_ffi::PeriodLookup, anyhow::Error> {
        let period = self
            .storage
            .dag()
            .proposal_period_at_level(level)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(match period {
            Some(period) => rustaxa_ffi::PeriodLookup {
                found: true,
                period,
            },
            None => rustaxa_ffi::PeriodLookup {
                found: false,
                period: 0,
            },
        })
    }
}

impl BridgeTransactionStorageQueries {
    /// Returns whether a transaction hash exists in pending or finalized storage.
    ///
    /// Inputs: canonical transaction hash bytes. Output follows
    /// `rustaxa-storage` transaction existence semantics and propagates storage
    /// errors to the caller.
    pub fn transaction_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.storage.transaction().exists(H256::from(*hash))
    }

    /// Returns whether a transaction hash has a finalized location index.
    ///
    /// Inputs: canonical transaction hash bytes. Output is `true` only when the
    /// finalized transaction-location row exists and is marked finalized.
    pub fn transaction_finalized(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.storage.transaction().finalized(H256::from(*hash))
    }

    /// Returns the serialized transaction-location payload for a hash.
    ///
    /// Inputs: canonical transaction hash bytes. Output is an empty vector when
    /// the row is absent, matching the legacy C++ storage API.
    pub fn get_transaction_location(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .transaction()
            .location_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    /// Returns a pending transaction RLP payload by hash.
    ///
    /// Inputs: canonical transaction hash bytes. Output is an empty vector when
    /// no pending transaction payload exists.
    pub fn get_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .transaction()
            .rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    /// Returns a finalized transaction RLP payload by period and position.
    ///
    /// Inputs: finalized PBFT period and transaction position within period
    /// data. Output is an empty vector when the period/position is absent.
    pub fn get_transaction_by_period_position(
        &self,
        period: u64,
        position: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .transaction()
            .by_period_position_rlp(period, position)?
            .unwrap_or_default())
    }

    /// Returns the persisted transaction count for a finalized period.
    pub fn get_transaction_count(&self, period: u64) -> Result<u64, anyhow::Error> {
        self.storage.transaction().count(period)
    }

    /// Returns a system transaction RLP payload by hash.
    ///
    /// Inputs: canonical system transaction hash bytes. Output is an empty
    /// vector when the payload is absent.
    pub fn get_system_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .transaction()
            .system_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    /// Returns all nonfinalized transaction RLP payloads.
    ///
    /// Outputs are canonical RLP bytes for C++ compatibility materialization.
    pub fn get_all_nonfinalized_transactions(
        &self,
    ) -> Result<Vec<rustaxa_ffi::TxRlp>, anyhow::Error> {
        let trxs = self.storage.transaction().all_nonfinalized_rlp()?;
        Ok(trxs
            .into_iter()
            .map(|data| rustaxa_ffi::TxRlp {
                data,
                is_system: false,
            })
            .collect())
    }

    /// Returns serialized system transaction hashes for a finalized period.
    ///
    /// Inputs: PBFT period number.
    /// Outputs: legacy-RLP encoded system transaction hash list bytes.
    pub fn get_period_system_transactions_hashes(
        &self,
        period: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        self.storage
            .transaction()
            .period_system_hashes_rlp(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// Returns transaction hash-to-period mappings from Rust storage.
    ///
    /// Outputs preserve the compatibility payload shape expected by C++ tests
    /// and public materializers.
    pub fn get_all_transaction_period(
        &self,
    ) -> Result<Vec<rustaxa_ffi::HashPeriod>, anyhow::Error> {
        let values = self.storage.transaction().all_with_period()?;
        Ok(values
            .into_iter()
            .map(|(hash, period)| {
                let mut h = [0u8; 32];
                h.copy_from_slice(hash.as_bytes());
                rustaxa_ffi::HashPeriod { hash: h, period }
            })
            .collect())
    }
}

impl BridgePeriodStorageQueries {
    /// Returns raw `PeriodData` bytes for a finalized period.
    pub fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.storage
            .period()
            .data_raw(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// Maps a PBFT block hash to its period index.
    pub fn get_period_from_pbft_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::PeriodLookup, anyhow::Error> {
        let lookup = self
            .storage
            .period()
            .by_pbft_hash(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;

        match lookup {
            Some(period) => Ok(rustaxa_ffi::PeriodLookup {
                found: true,
                period,
            }),
            None => Ok(rustaxa_ffi::PeriodLookup {
                found: false,
                period: 0,
            }),
        }
    }

    /// Returns raw finalized period receipts bytes.
    pub fn get_block_receipt(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.storage
            .period()
            .receipt(period)
            .map_err(|e| anyhow::anyhow!(e))
    }
}

impl BridgeFinalChainStorageQueries {
    pub fn get_final_chain_meta_value(&self, key: u32) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .final_chain()
            .meta_value(key)?
            .unwrap_or_default())
    }

    pub fn get_final_chain_block_header(
        &self,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .final_chain()
            .block_header_raw(block_number)?
            .unwrap_or_default())
    }

    pub fn get_final_chain_block_hash_by_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .final_chain()
            .block_hash_by_number(block_number)?
            .unwrap_or_default())
    }

    pub fn get_final_chain_block_number_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .final_chain()
            .block_number_by_hash(H256::from(*hash))?
            .unwrap_or_default())
    }

    pub fn get_final_chain_log_blooms_chunk(
        &self,
        chunk_id: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .final_chain()
            .log_blooms_chunk_raw(H256::from(*chunk_id))?
            .unwrap_or_default())
    }

    pub fn get_final_chain_receipt_by_trx_hash(
        &self,
        trx_hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .storage
            .final_chain()
            .receipt_by_trx_hash(H256::from(*trx_hash))?
            .unwrap_or_default())
    }
}

/// Appends a typed status-field write to a Rust-owned storage shim batch.
///
/// This keeps the legacy C++ batch commit/drop boundary while moving the
/// status-column key/value encoding into `rustaxa-storage`.
pub fn storage_shim_save_status_field(
    batch: &mut BridgeStorageBatch,
    field: u8,
    value: u64,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .metadata()
        .write_status_field_in_batch(storage_shim_batch_mut(batch)?, field, value)
}

/// Appends a typed sortition-params change write to a Rust-owned storage shim batch.
///
/// The payload must already be the legacy RLP bytes. The Rust metadata
/// repository owns the target column and period-key encoding.
pub fn storage_shim_save_sortition_params_change(
    batch: &mut BridgeStorageBatch,
    period: u64,
    params_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.metadata().write_sortition_params_change_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        &params_rlp,
    )
}

/// Appends a typed period-lambda write to a Rust-owned storage shim batch.
pub fn storage_shim_save_period_lambda(
    batch: &mut BridgeStorageBatch,
    period: u64,
    period_lambda: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.metadata().write_period_lambda_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        period_lambda,
    )
}

/// Appends a typed dynamic-lambda rounds-count write to a Rust-owned storage shim batch.
pub fn storage_shim_save_rounds_count_dynamic_lambda(
    batch: &mut BridgeStorageBatch,
    rounds_count: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .metadata()
        .write_rounds_count_dynamic_lambda_in_batch(storage_shim_batch_mut(batch)?, rounds_count)
}

/// Appends typed block reward statistics bytes to a Rust-owned storage shim batch.
///
/// The caller supplies legacy-compatible encoded block-stats RLP; Rust owns the
/// period-key encoding and `block_rewards_stats` column selection.
pub fn storage_shim_save_block_rewards_stats(
    batch: &mut BridgeStorageBatch,
    period: u64,
    stats_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.metadata().write_block_rewards_stats_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        &stats_rlp,
    )
}

/// Clears block reward statistics through the storage shim boundary.
///
/// The Rust storage repository owns the aggregate delete and commits it as a
/// native storage batch. The C++ shim uses this only for the public
/// `DbStorage::deleteColumnData(block_rewards_stats)` compatibility route.
pub fn storage_shim_clear_block_rewards_stats(
    runtime: &BridgeConsensusApplication,
) -> Result<(), anyhow::Error> {
    runtime
        .0
        .storage_for_bridge()
        .metadata()
        .clear_block_rewards_stats()
}

/// Writes the genesis hash through the storage shim boundary.
///
/// The storage repository owns the legacy single-value key and write-once
/// behavior. The C++ shim supplies the already validated 32-byte hash from its
/// public `DbStorage` compatibility API.
pub fn storage_shim_set_genesis_hash(
    runtime: &BridgeConsensusApplication,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    runtime
        .0
        .storage_for_bridge()
        .metadata()
        .set_genesis_hash_if_empty(hash)
}

/// Seeds exact FinalChain lookup rows for storage conformance.
///
/// Inputs are legacy-compatible raw bytes supplied by the conformance runner.
/// The Rust storage repository owns the atomic write group for meta, header,
/// hash/number, receipt, bloom, and by-period receipt rows. This is a dedicated
/// fixture API so C++ tests do not regain a broad `BridgeStorage` mutator.
///
/// Errors preserve the native storage write failure from the underlying
/// FinalChain repository. The helper does not perform partial repair if a write
/// conflict or RocksDB error occurs.
#[allow(clippy::too_many_arguments)]
pub fn storage_shim_seed_final_chain_conformance_lookup_rows(
    runtime: &BridgeConsensusApplication,
    meta_key: u32,
    meta_value: Vec<u8>,
    block_number: u64,
    block_hash: &[u8; 32],
    block_header_rlp: Vec<u8>,
    receipt_hash: &[u8; 32],
    receipt_rlp: Vec<u8>,
    blooms_chunk: &[u8; 32],
    blooms_rlp: Vec<u8>,
    receipt_period: u64,
    receipts_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    runtime
        .0
        .storage_for_bridge()
        .final_chain()
        .write_conformance_lookup_rows(
            meta_key,
            &meta_value,
            block_number,
            H256::from(*block_hash),
            &block_header_rlp,
            H256::from(*receipt_hash),
            &receipt_rlp,
            H256::from(*blooms_chunk),
            &blooms_rlp,
            receipt_period,
            &receipts_rlp,
        )
}

/// Appends a typed PBFT manager numeric-field write to a Rust-owned storage shim batch.
///
/// The C++ shim supplies legacy enum discriminants and values; `rustaxa-storage`
/// owns the PBFT manager column and little-endian value encoding.
pub fn storage_shim_save_pbft_mgr_field(
    batch: &mut BridgeStorageBatch,
    field: u8,
    value: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .write_manager_field_in_batch(storage_shim_batch_mut(batch)?, field, value)
}

/// Appends a typed PBFT manager status write to a Rust-owned storage shim batch.
///
/// The C++ shim supplies the legacy status discriminant while Rust owns the
/// status-column key and bool encoding.
pub fn storage_shim_save_pbft_mgr_status(
    batch: &mut BridgeStorageBatch,
    field: u8,
    value: bool,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .write_manager_status_in_batch(storage_shim_batch_mut(batch)?, field, value)
}

/// Appends a typed cert-voted block cleanup to a Rust-owned storage shim batch.
pub fn storage_shim_remove_cert_voted_block_in_round(
    batch: &mut BridgeStorageBatch,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_cert_voted_block_in_round_in_batch(storage_shim_batch_mut(batch)?)
}

/// Appends a typed cert-voted block write to a Rust-owned storage shim batch.
///
/// The block payload remains the canonical legacy PBFT block RLP supplied by
/// the C++ facade. Rust owns the single-value key and the `[round, block]`
/// storage wrapper used by the PBFT repository.
pub fn storage_shim_save_cert_voted_block_in_round(
    batch: &mut BridgeStorageBatch,
    round: u64,
    block_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.pbft().write_cert_voted_block_in_round_in_batch(
        storage_shim_batch_mut(batch)?,
        round,
        &block_rlp,
    )
}

/// Appends a typed PBFT head write to a Rust-owned storage shim batch.
///
/// The head payload remains opaque legacy bytes while Rust owns the PBFT head
/// column and hash-key layout.
pub fn storage_shim_save_pbft_head(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    head: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .write_head_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash), &head)
}

/// Appends a typed own verified vote cleanup to a Rust-owned storage shim batch.
///
/// This caller-owned compatibility batch does not retain the production
/// own-vote serialization guard through its later commit. Its caller must
/// serialize the complete batch lifetime with production own-vote operations.
pub fn storage_shim_remove_own_verified_vote(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_own_verified_vote_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Appends a typed own verified vote write to a Rust-owned storage shim batch.
///
/// The caller supplies the canonical weighted vote RLP bytes. Rust owns the
/// latest-round own-vote column and hash-key layout.
/// This caller-owned compatibility batch does not retain the production
/// own-vote serialization guard through its later commit. Its caller must
/// serialize the complete batch lifetime with production own-vote operations.
pub fn storage_shim_save_own_verified_vote(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    vote_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.pbft().write_own_verified_vote_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        &vote_rlp,
    )
}

/// Appends a typed 2t+1 vote bundle replacement to a Rust-owned storage shim batch.
///
/// Rust validates the legacy vote-type discriminant and owns the delete-then-put
/// ordering inside the caller-owned batch.
pub fn storage_shim_replace_two_t_plus_one_votes(
    batch: &mut BridgeStorageBatch,
    vote_type: u8,
    votes_bundle_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.pbft().replace_two_t_plus_one_votes_in_batch(
        storage_shim_batch_mut(batch)?,
        vote_type,
        &votes_bundle_rlp,
    )
}

/// Appends a typed extra reward vote cleanup to a Rust-owned storage shim batch.
///
/// This compatibility batch does not retain the production extra-reward lock
/// through its later commit; its caller must externally serialize the complete
/// batch lifetime with reward admission and finalization reset.
pub fn storage_shim_remove_extra_reward_vote(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_extra_reward_vote_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Appends a typed extra reward vote write to a Rust-owned storage shim batch.
///
/// The caller supplies the canonical weighted vote RLP bytes. Rust owns the
/// extra-reward-vote column and hash-key layout.
/// This compatibility batch does not retain the production extra-reward lock
/// through its later commit; its caller must externally serialize the complete
/// batch lifetime with reward admission and finalization reset.
pub fn storage_shim_save_extra_reward_vote(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    vote_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.pbft().write_extra_reward_vote_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        &vote_rlp,
    )
}

/// Appends a typed PBFT block hash-to-period index write to a Rust-owned storage shim batch.
pub fn storage_shim_save_pbft_block_period(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    period: u64,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.period().write_pbft_period_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        period,
    )
}

/// Appends a typed DAG block hash-to-period/position index write to a Rust-owned storage shim batch.
///
/// Rust owns the legacy `dag_block_period` RLP payload shape while the C++ shim
/// supplies the finalized period and position facts.
pub fn storage_shim_save_dag_block_period(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    period: u64,
    position: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.dag().write_period_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        period,
        position,
    )
}

/// Appends a typed non-finalized DAG block write to a Rust-owned storage shim batch.
///
/// Rust owns the `dag_blocks`, `dag_blocks_level`, and DAG status field
/// column/key/value encodings. The C++ shim still supplies canonical DAG block
/// RLP and final status sidecar values while those sidecars remain C++
/// materialized.
pub fn storage_shim_save_dag_block(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    level: u64,
    block_rlp: Vec<u8>,
    dag_blocks_count: u64,
    dag_edge_count: u64,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.dag().write_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        level,
        &block_rlp,
        dag_blocks_count,
        dag_edge_count,
    )
}

/// Appends typed DAG level-index and counter writes to a Rust-owned storage shim batch.
///
/// Updates are grouped by level in `rustaxa-storage`, so one caller-owned batch
/// can stage multiple blocks at the same DAG level without overwriting staged
/// level-index membership.
pub fn storage_shim_update_dag_block_counters(
    batch: &mut BridgeStorageBatch,
    updates: Vec<rustaxa_ffi::DagCounterUpdate>,
    dag_blocks_count: u64,
    dag_edge_count: u64,
) -> Result<(), anyhow::Error> {
    let updates: Vec<(H256, u64, u64)> = updates
        .into_iter()
        .map(|update| (H256::from(update.hash), update.level, update.tips_count))
        .collect();
    let storage = batch.storage.clone();
    storage.dag().update_counters_in_batch(
        storage_shim_batch_mut(batch)?,
        &updates,
        dag_blocks_count,
        dag_edge_count,
    )
}

/// Appends a typed non-finalized DAG block delete to a Rust-owned storage shim batch.
pub fn storage_shim_remove_dag_block(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .dag()
        .remove_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Appends typed finalized period data bytes to a Rust-owned storage shim batch.
///
/// The C++ shim still supplies the legacy `PeriodData` RLP payload, while Rust
/// owns the `period_data` column and period-key encoding.
pub fn storage_shim_save_period_data(
    batch: &mut BridgeStorageBatch,
    period: u64,
    period_data_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .period()
        .write_in_batch(storage_shim_batch_mut(batch)?, period, &period_data_rlp)
}

/// Appends typed proposed PBFT block cleanup to a Rust-owned storage shim batch.
pub fn storage_shim_remove_proposed_pbft_block(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_proposed_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Appends a typed proposal-period DAG level map write to a Rust-owned storage shim batch.
pub fn storage_shim_save_proposal_period_dag_level(
    batch: &mut BridgeStorageBatch,
    level: u64,
    period: u64,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.dag().write_proposal_period_at_level_in_batch(
        storage_shim_batch_mut(batch)?,
        level,
        period,
    )
}

/// Appends a typed finalized transaction-location write to a Rust-owned storage shim batch.
pub fn storage_shim_save_transaction_location(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    period: u64,
    position: u32,
    is_system: bool,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.transaction().write_location_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        period,
        position,
        is_system,
    )
}

/// Appends a typed pending transaction payload write to a Rust-owned storage shim batch.
pub fn storage_shim_save_transaction(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    trx_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.transaction().write_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        &trx_rlp,
    )
}

/// Appends a typed pending transaction payload removal to a Rust-owned storage shim batch.
pub fn storage_shim_remove_transaction(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .transaction()
        .remove_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Appends a typed system transaction payload write to a Rust-owned storage shim batch.
pub fn storage_shim_save_system_transaction(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    trx_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.transaction().write_system_in_batch(
        storage_shim_batch_mut(batch)?,
        H256::from(*hash),
        &trx_rlp,
    )
}

/// Appends typed period system-transaction hash-list bytes to a Rust-owned storage shim batch.
pub fn storage_shim_save_period_system_transactions_hashes(
    batch: &mut BridgeStorageBatch,
    period: u64,
    hashes_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.transaction().write_period_system_hashes_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        &hashes_rlp,
    )
}

/// Commits a Rust-owned storage shim batch and consumes it.
///
/// Dropping a `BridgeStorageBatch` without calling this method discards staged
/// writes, matching legacy dropped-batch behavior without a bridge-side batch
/// registry.
pub fn storage_shim_commit_batch(
    mut batch: Box<BridgeStorageBatch>,
    sync: bool,
) -> Result<(), anyhow::Error> {
    let storage_batch = batch
        .batch
        .take()
        .ok_or_else(|| anyhow::anyhow!("storage shim batch already committed"))?;
    batch
        .storage
        .commit_write_batch_with_sync(storage_batch, sync)
}

/// Batch-loads transaction RLP payloads by hash using Rust storage semantics shared by consensus bridges.
///
/// Inputs are canonical transaction hashes in caller-requested order. Outputs preserve
/// that order, return the original hash, mark whether a payload was found, and identify
/// whether the payload came from finalized storage. Lookup checks pending/non-finalized
/// transactions first, then finalized transaction-location metadata, including system
/// transactions. Missing hashes are returned as `found = false` rather than errors;
/// storage/codec failures are propagated with stable context labels.
#[cfg(test)]
fn transaction_rlp_lookups(
    storage: &Storage,
    hashes: Vec<H256>,
) -> Result<Vec<rustaxa_ffi::DagTransactionRlpLookup>, anyhow::Error> {
    let transaction = storage.transaction();
    let mut out = Vec::with_capacity(hashes.len());

    for hash in hashes {
        let (tx_rlp, finalized) = if let Some(tx_rlp) = transaction
            .rlp(hash)
            .context("DAG_TRANSACTION_RLP_PENDING_LOOKUP")?
        {
            (Some(tx_rlp), false)
        } else if let Some(location_rlp) = transaction
            .location_rlp(hash)
            .context("DAG_TRANSACTION_RLP_LOCATION_LOOKUP")?
        {
            let location = rlp::Rlp::new(&location_rlp);
            let period = location
                .val_at::<u64>(0)
                .context("DAG_TRANSACTION_RLP_LOCATION_PERIOD")?;
            let position = location
                .val_at::<u32>(1)
                .context("DAG_TRANSACTION_RLP_LOCATION_POSITION")?;
            let is_system = location
                .item_count()
                .context("DAG_TRANSACTION_RLP_LOCATION_SHAPE")?
                == 3
                && location
                    .val_at::<bool>(2)
                    .context("DAG_TRANSACTION_RLP_LOCATION_SYSTEM_FLAG")?;
            let tx_rlp = if is_system {
                transaction
                    .system_rlp(hash)
                    .context("DAG_TRANSACTION_RLP_SYSTEM_LOOKUP")?
            } else {
                transaction
                    .by_period_position_rlp(period, position)
                    .context("DAG_TRANSACTION_RLP_FINALIZED_LOOKUP")?
            };
            (tx_rlp, true)
        } else {
            (None, false)
        };

        out.push(rustaxa_ffi::DagTransactionRlpLookup {
            hash: hash.0,
            found: tx_rlp.is_some(),
            finalized,
            tx_rlp: tx_rlp.unwrap_or_default(),
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag_transaction_service::create_test_consensus_application;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn storage_fixture(path: &str) -> (Box<BridgeConsensusApplication>, Arc<Storage>) {
        let application = create_test_consensus_application(path, Vec::new(), 0)
            .expect("consensus application should initialize");
        let storage = application.0.storage_for_bridge().clone();
        (application, storage)
    }

    fn transaction_queries(
        application: &BridgeConsensusApplication,
    ) -> Box<BridgeTransactionStorageQueries> {
        create_transaction_storage_queries(application)
    }

    fn period_data_rlp(transaction_rlps: &[Vec<u8>]) -> Vec<u8> {
        let mut transactions = rlp::RlpStream::new_list(transaction_rlps.len());
        for transaction_rlp in transaction_rlps {
            transactions.append_raw(transaction_rlp, 1);
        }

        let mut period_data = rlp::RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&transactions.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.out().to_vec()
    }

    #[test]
    fn storage_shim_genesis_hash_preserves_write_once_behavior() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_genesis_hash");
        {
            let (application, _storage) =
                storage_fixture(temp_dir.to_str().expect("temp path should be valid UTF-8"));

            assert_eq!(application.get_genesis_hash().unwrap(), vec![1; 32]);

            storage_shim_set_genesis_hash(&application, &[0xAB; 32])
                .expect("replacement genesis hash should be a no-op");
            assert_eq!(application.get_genesis_hash().unwrap(), vec![1; 32]);

            storage_shim_set_genesis_hash(&application, &[0xCD; 32])
                .expect("second genesis hash should be a no-op");
            assert_eq!(application.get_genesis_hash().unwrap(), vec![1; 32]);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn storage_shim_clear_block_rewards_stats_removes_all_periods() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_clear_rewards");
        {
            let (application, _storage) =
                storage_fixture(temp_dir.to_str().expect("temp path should be valid UTF-8"));

            let mut batch = create_storage_shim_batch(&application);
            storage_shim_save_block_rewards_stats(&mut batch, 3, vec![0xC1, 0xA3])
                .expect("first stats row should stage");
            storage_shim_save_block_rewards_stats(&mut batch, 7, vec![0xC1, 0xA7])
                .expect("second stats row should stage");
            storage_shim_commit_batch(batch, false).expect("stats rows should commit");
            assert_eq!(application.get_blocks_rewards_stats().unwrap().len(), 2);

            storage_shim_clear_block_rewards_stats(&application)
                .expect("storage shim clear should remove stats");
            assert!(application.get_blocks_rewards_stats().unwrap().is_empty());
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn transaction_rlp_batch_lookup_reads_pending_finalized_system_and_missing() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_transaction_rlps");
        {
            let (_application, storage) =
                storage_fixture(temp_dir.to_str().expect("temp path should be valid UTF-8"));
            let pending = vec![0xC1, 0xA1];
            let finalized = vec![0xC1, 0xA2];
            let system = vec![0xC1, 0xA3];

            storage
                .transaction()
                .write(H256::from([1u8; 32]), &pending)
                .expect("pending transaction should save");
            storage
                .period()
                .write(7, &period_data_rlp(std::slice::from_ref(&finalized)))
                .expect("period data should save");
            storage
                .transaction()
                .write_location(H256::from([2u8; 32]), 7, 0, false)
                .expect("regular finalized location should save");
            storage
                .transaction()
                .write_system(H256::from([3u8; 32]), &system)
                .expect("system transaction should save");
            storage
                .transaction()
                .write_location(H256::from([3u8; 32]), 8, 0, true)
                .expect("system finalized location should save");

            let lookup = transaction_rlp_lookups(
                &storage,
                vec![
                    H256::from([1u8; 32]),
                    H256::from([2u8; 32]),
                    H256::from([3u8; 32]),
                    H256::from([4u8; 32]),
                ],
            )
            .expect("batch lookup should succeed");

            assert_eq!(lookup.len(), 4);
            assert_eq!(lookup[0].hash, [1u8; 32]);
            assert!(lookup[0].found);
            assert!(!lookup[0].finalized);
            assert_eq!(lookup[0].tx_rlp, pending);
            assert_eq!(lookup[1].hash, [2u8; 32]);
            assert!(lookup[1].found);
            assert!(lookup[1].finalized);
            assert_eq!(lookup[1].tx_rlp, finalized);
            assert_eq!(lookup[2].hash, [3u8; 32]);
            assert!(lookup[2].found);
            assert!(lookup[2].finalized);
            assert_eq!(lookup[2].tx_rlp, system);
            assert_eq!(lookup[3].hash, [4u8; 32]);
            assert!(!lookup[3].found);
            assert!(!lookup[3].finalized);
            assert!(lookup[3].tx_rlp.is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn save_non_finalized_transactions_batch_updates_trx_count_status() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_save_non_finalized_transactions");
        {
            let (application, storage) =
                storage_fixture(temp_dir.to_str().expect("temp path should be valid UTF-8"));

            let existing_tx_count = 3u64;
            storage
                .metadata()
                .write_status_field(
                    rustaxa_storage::StatusField::TrxCount as u8,
                    existing_tx_count,
                )
                .expect("pre-seeded transaction count should persist");

            rustaxa_consensus::save_non_finalized_transactions(
                &storage,
                vec![
                    rustaxa_consensus::NonFinalizedTransactionStoragePayload {
                        hash: H256::from([10u8; 32]),
                        trx_rlp: vec![1],
                    },
                    rustaxa_consensus::NonFinalizedTransactionStoragePayload {
                        hash: H256::from([11u8; 32]),
                        trx_rlp: vec![2],
                    },
                ],
                existing_tx_count + 2,
            )
            .expect("batch write should persist accepted transactions");

            assert_eq!(
                application
                    .get_status_field(rustaxa_storage::StatusField::TrxCount as u8)
                    .expect("trx count status should load"),
                existing_tx_count + 2,
            );
            assert_eq!(
                transaction_queries(&application)
                    .get_transaction(&[10u8; 32])
                    .expect("tx 10 should be retrievable"),
                vec![1],
            );

            rustaxa_consensus::save_non_finalized_transactions(
                &storage,
                vec![rustaxa_consensus::NonFinalizedTransactionStoragePayload {
                    hash: H256::from([13u8; 32]),
                    trx_rlp: vec![5],
                }],
                existing_tx_count + 3,
            )
            .expect("second batch write should persist accepted tx");

            assert_eq!(
                application
                    .get_status_field(rustaxa_storage::StatusField::TrxCount as u8)
                    .expect("trx count status should load"),
                existing_tx_count + 3,
            );
            assert_eq!(
                transaction_queries(&application)
                    .get_transaction(&[13u8; 32])
                    .expect("tx 13 should be persisted"),
                vec![5],
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
