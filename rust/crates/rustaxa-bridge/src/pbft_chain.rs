//! PBFT-chain bridge adapters backed by the Rust consensus and storage implementations.
//!
//! A storage-backed [`BridgePbftChain`] owns a cloned `Arc` to the native Rust storage. Its block-existence and canonical
//! RLP lookups therefore remain valid after the originating C++ `DbStorage` bridge handle is destroyed.

use crate::ffi::rustaxa_ffi::{
    PbftBlockStorageLookup as FfiPbftBlockStorageLookup, PbftBlockValidationResult,
    PbftChainFinalizationUpdateReport as FfiPbftChainFinalizationUpdateReport,
    PbftChainHeadPayload, PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
};
use crate::ffi::BridgePbftChain;
use crate::ffi::BridgeStorage;
use anyhow::anyhow;
use ethereum_types::H256;
use rustaxa_consensus::pbft_chain::{
    load_pbft_block_from_storage, pbft_block_exists_in_storage,
    restore_pbft_chain_from_storage as domain_restore_pbft_chain_from_storage,
    PbftBlockStorageLookup, PbftBlockValidation, PbftChain, PbftChainHead,
};
use rustaxa_storage::Storage;

const PBFT_VALIDATION_VALID: u8 = 0;
const PBFT_VALIDATION_PERIOD_MISMATCH: u8 = 1;
const PBFT_VALIDATION_PREVIOUS_HASH_MISMATCH: u8 = 2;
/// Creates a Rust PBFT chain state model from a C++-parsed head payload.
///
/// The bridge intentionally receives structured state instead of raw JSON so C++ can preserve the legacy JsonCpp
/// formatting used for persisted `pbft_head` records.
#[cfg(test)]
pub fn create_pbft_chain(
    head: PbftChainHeadPayload,
) -> Result<Box<BridgePbftChain>, anyhow::Error> {
    Ok(Box::new(BridgePbftChain {
        state: PbftChain::new(head.into())?,
        storage: None,
        initialized_default: false,
    }))
}

/// Creates a Rust PBFT chain state model directly from native Rust storage.
///
/// The bridge is only a DTO adapter: storage recovery, legacy head parsing,
/// default-head initialization, and last-anchor recovery are owned by
/// `rustaxa-consensus`. The returned [`BridgePbftChain`] clones and owns the
/// storage `Arc`, so its runtime lookups do not depend on the lifetime of the
/// supplied bridge handle or the originating C++ `DbStorage` object.
pub fn create_pbft_chain_from_storage(
    storage: &BridgeStorage,
) -> Result<Box<BridgePbftChain>, anyhow::Error> {
    let restored = domain_restore_pbft_chain_from_storage(storage.0.as_ref())?;
    Ok(Box::new(BridgePbftChain {
        state: PbftChain::new(restored.head)?,
        storage: Some(storage.0.clone()),
        initialized_default: restored.initialized_default,
    }))
}

impl BridgePbftChain {
    /// Returns whether storage recovery initialized the default PBFT chain head.
    pub fn pbft_chain_initialized_default(&self) -> bool {
        self.initialized_default
    }

    /// Returns the current PBFT chain head payload for C++ JSON formatting and public accessors.
    pub fn pbft_chain_head(&self) -> PbftChainHeadPayload {
        self.state.head().into()
    }

    /// Returns a non-mutating preview for the legacy persisted-head JSON path.
    pub fn pbft_chain_project_legacy_json_head(
        &self,
        block_hash: &[u8; 32],
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHeadPayload, anyhow::Error> {
        Ok(self
            .state
            .project_legacy_json_head(H256::from(*block_hash), increments_non_empty_size)?
            .into())
    }

    /// Applies an in-memory PBFT chain update without writing storage.
    pub fn pbft_chain_update(
        &mut self,
        block_hash: &[u8; 32],
        anchor_hash: &[u8; 32],
    ) -> Result<PbftChainHeadPayload, anyhow::Error> {
        Ok(self
            .state
            .update(H256::from(*block_hash), H256::from(*anchor_hash))?
            .into())
    }

    /// Applies the PBFT-chain finalization mutation described by a Rust-planned
    /// storage intent and returns PBFT-chain-owned head facts.
    ///
    /// Inputs:
    /// - `write_intent`: accepted finalization write plan from the Rust planner.
    ///
    /// Outputs:
    /// - Post-mutation PBFT-chain head facts for manager-runtime validation.
    ///
    /// Invariants and edge behavior:
    /// - Block hash, anchor hash, and period are derived from the accepted Rust
    ///   finalization intent, not from C++ sidecar state.
    /// - Persistence remains outside this method; the caller must have already
    ///   applied the Rust-owned finalized-period storage write stages.
    /// - Errors from the underlying PBFT-chain update are returned before any
    ///   report is emitted.
    pub fn pbft_chain_update_for_finalization(
        &mut self,
        write_intent: &FfiPbftFinalizationStorageWritePlan,
    ) -> Result<FfiPbftChainFinalizationUpdateReport, anyhow::Error> {
        let head = self.state.update(
            H256::from(write_intent.pbft_block_hash),
            H256::from(write_intent.anchor_hash),
        )?;
        Ok(FfiPbftChainFinalizationUpdateReport {
            size: head.size,
            last_pbft_block_hash: head.last_pbft_block_hash.into(),
            last_non_null_anchor_hash: head.last_non_null_pbft_dag_anchor_hash.into(),
        })
    }

    /// Returns whether this storage-backed PBFT chain runtime has a finalized
    /// PBFT block hash in Rust storage.
    pub fn pbft_chain_block_exists(&self, block_hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        pbft_block_exists_in_storage(self.storage_handle()?, H256::from(*block_hash))
    }

    /// Loads canonical signed PBFT block RLP from this runtime's owned Rust
    /// storage handle.
    pub fn pbft_chain_block_rlp(
        &self,
        block_hash: &[u8; 32],
    ) -> Result<FfiPbftBlockStorageLookup, anyhow::Error> {
        Ok(load_pbft_block_from_storage(self.storage_handle()?, H256::from(*block_hash))?.into())
    }

    /// Checks whether the supplied candidate block extends the current PBFT head.
    pub fn pbft_chain_validate_block(
        &self,
        period: u64,
        prev_hash: &[u8; 32],
    ) -> PbftBlockValidationResult {
        match self
            .state
            .validate_next_block(period, H256::from(*prev_hash))
        {
            PbftBlockValidation::Valid => PbftBlockValidationResult {
                ok: true,
                code: PBFT_VALIDATION_VALID,
                expected_period: 0,
                actual_period: period,
                expected_prev_hash: [0; 32],
                actual_prev_hash: *prev_hash,
            },
            PbftBlockValidation::PeriodMismatch { expected, actual } => PbftBlockValidationResult {
                ok: false,
                code: PBFT_VALIDATION_PERIOD_MISMATCH,
                expected_period: expected,
                actual_period: actual,
                expected_prev_hash: [0; 32],
                actual_prev_hash: *prev_hash,
            },
            PbftBlockValidation::PreviousHashMismatch { expected, actual } => {
                PbftBlockValidationResult {
                    ok: false,
                    code: PBFT_VALIDATION_PREVIOUS_HASH_MISMATCH,
                    expected_period: 0,
                    actual_period: period,
                    expected_prev_hash: expected.into(),
                    actual_prev_hash: actual.into(),
                }
            }
        }
    }

    fn storage_handle(&self) -> Result<&Storage, anyhow::Error> {
        self.storage
            .as_deref()
            .ok_or_else(|| anyhow!("PBFT_CHAIN_STORAGE_HANDLE_MISSING"))
    }
}

impl From<PbftChainHeadPayload> for PbftChainHead {
    fn from(value: PbftChainHeadPayload) -> Self {
        Self {
            head_hash: H256::from(value.head_hash),
            size: value.size,
            non_empty_size: value.non_empty_size,
            last_pbft_block_hash: H256::from(value.last_pbft_block_hash),
            last_non_null_pbft_dag_anchor_hash: H256::from(value.last_non_null_anchor_hash),
        }
    }
}

impl From<PbftChainHead> for PbftChainHeadPayload {
    fn from(value: PbftChainHead) -> Self {
        Self {
            head_hash: value.head_hash.into(),
            size: value.size,
            non_empty_size: value.non_empty_size,
            last_pbft_block_hash: value.last_pbft_block_hash.into(),
            last_non_null_anchor_hash: value.last_non_null_pbft_dag_anchor_hash.into(),
        }
    }
}

impl From<PbftBlockStorageLookup> for FfiPbftBlockStorageLookup {
    fn from(value: PbftBlockStorageLookup) -> Self {
        Self {
            found: value.found,
            block_rlp: value.block_rlp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn unique_storage_path(name: &str) -> String {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{name}_{}_{}", std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        path.to_str().expect("utf-8 temp path").to_string()
    }

    fn pbft_block_rlp(prev: H256, pivot: H256, period: u64) -> Vec<u8> {
        let mut stream = RlpStream::new_list(8);
        stream.append(&prev);
        stream.append(&pivot);
        stream.begin_list(0);
        stream.begin_list(0);
        stream.append(&period);
        stream.append(&0u64);
        stream.append(&0u64);
        stream.append(&Vec::<u8>::new());
        stream.out().to_vec()
    }

    fn period_data_rlp(block_rlp: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(4);
        stream.append_raw(block_rlp, 1);
        stream.begin_list(0);
        stream.begin_list(0);
        stream.begin_list(0);
        stream.out().to_vec()
    }

    #[test]
    fn bridge_creates_pbft_chain_from_storage_and_recovers_anchor() {
        let storage = crate::storage::create_storage(&unique_storage_path(
            "rustaxa_bridge_pbft_chain_from_storage",
        ))
        .unwrap();
        let first = pbft_block_rlp(H256::zero(), hash(100), 1);
        let first_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&first))
            .unwrap()
            .block_hash;
        let second = pbft_block_rlp(first_hash, H256::zero(), 2);
        let second_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&second))
            .unwrap()
            .block_hash;
        storage
            .0
            .period()
            .write(1, &period_data_rlp(&first))
            .unwrap();
        storage.0.period().write_pbft_period(first_hash, 1).unwrap();
        storage
            .0
            .period()
            .write(2, &period_data_rlp(&second))
            .unwrap();
        storage
            .0
            .period()
            .write_pbft_period(second_hash, 2)
            .unwrap();
        let legacy_head = format!(
            r#"{{"head_hash":"0x{:064x}","size":2,"non_empty_size":1,"last_pbft_block_hash":"0x{:064x}"}}"#,
            0, second_hash
        );
        storage
            .0
            .pbft()
            .write_head(H256::zero(), legacy_head.as_bytes())
            .unwrap();

        let chain = create_pbft_chain_from_storage(&storage).unwrap();
        let head = chain.pbft_chain_head();

        assert!(!chain.pbft_chain_initialized_default());
        assert_eq!(head.size, 2);
        assert_eq!(head.non_empty_size, 1);
        assert_eq!(H256::from(head.last_pbft_block_hash), second_hash);
        assert_eq!(H256::from(head.last_non_null_anchor_hash), hash(100));
    }

    #[test]
    fn bridge_create_pbft_chain_from_storage_reports_default_initialization() {
        let storage = crate::storage::create_storage(&unique_storage_path(
            "rustaxa_bridge_pbft_chain_default_init",
        ))
        .unwrap();

        let chain = create_pbft_chain_from_storage(&storage).unwrap();

        assert!(chain.pbft_chain_initialized_default());
        assert_eq!(chain.pbft_chain_head().size, 0);
    }

    #[test]
    fn bridge_pbft_chain_block_lookup_uses_runtime_owned_storage() {
        let storage = crate::storage::create_storage(&unique_storage_path(
            "rustaxa_bridge_pbft_chain_block_lookup",
        ))
        .unwrap();
        let block = pbft_block_rlp(H256::zero(), hash(9), 1);
        let block_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))
            .unwrap()
            .block_hash;
        storage
            .0
            .period()
            .write(1, &period_data_rlp(&block))
            .unwrap();
        storage.0.period().write_pbft_period(block_hash, 1).unwrap();

        let chain = create_pbft_chain_from_storage(&storage).unwrap();
        drop(storage);

        let exists = chain.pbft_chain_block_exists(&block_hash.into()).unwrap();
        let loaded = chain.pbft_chain_block_rlp(&block_hash.into()).unwrap();
        let missing = chain.pbft_chain_block_rlp(&hash(999).into()).unwrap();

        assert!(exists);
        assert!(loaded.found);
        assert_eq!(loaded.block_rlp, block);
        assert!(!missing.found);
    }

    #[test]
    fn bridge_pbft_chain_finalization_update_derives_report_from_write_intent() {
        let mut chain = create_pbft_chain(
            PbftChainHead {
                head_hash: H256::zero(),
                size: 9,
                non_empty_size: 4,
                last_pbft_block_hash: hash(8),
                last_non_null_pbft_dag_anchor_hash: hash(77),
            }
            .into(),
        )
        .unwrap();
        let mut write_intent = FfiPbftFinalizationStorageWritePlan {
            persist_pbft_head: true,
            persist_period_data: true,
            reset_reward_votes: false,
            update_sortition_params: false,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: false,
            process_pillar_block: false,
            pbft_block_hash: hash(99).into(),
            pbft_head_hash: H256::zero().into(),
            block_period: 10,
            null_anchor: false,
            anchor_hash: hash(123).into(),
            reward_vote_period: 0,
            reward_vote_round: 0,
            reward_vote_step: 0,
            reward_vote_block_hash: H256::zero().into(),
            period_lambda: 0,
            blocks_per_year: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_status: false,
            pbft_head_payload: Vec::new(),
            period_data_rlp: Vec::new(),
            dag_block_period_writes: Vec::new(),
            transaction_location_writes: Vec::new(),
        };

        let report = chain
            .pbft_chain_update_for_finalization(&write_intent)
            .unwrap();

        assert_eq!(report.size, 10);
        assert_eq!(H256::from(report.last_pbft_block_hash), hash(99));
        assert_eq!(H256::from(report.last_non_null_anchor_hash), hash(123));

        write_intent.pbft_block_hash = hash(100).into();
        write_intent.anchor_hash = H256::zero().into();
        write_intent.block_period = 11;
        let null_anchor_report = chain
            .pbft_chain_update_for_finalization(&write_intent)
            .unwrap();

        assert_eq!(null_anchor_report.size, 11);
        assert_eq!(
            H256::from(null_anchor_report.last_pbft_block_hash),
            hash(100)
        );
        assert_eq!(
            H256::from(null_anchor_report.last_non_null_anchor_hash),
            hash(123)
        );
    }
}
