//! Native PBFT-chain state and application runtime.
//!
//! This module models the PBFT head fields and validation/update rules from the
//! legacy `PbftChain` class and owns Rust-mode PBFT-chain storage recovery over
//! `rustaxa-storage`.

use anyhow::{Context, Result, anyhow};
use ethereum_types::H256;
use rustaxa_storage::Storage;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard};

const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;

/// PBFT head state mirrored from legacy `pbft_head` JSON payload.
///
/// Inputs/outputs:
/// - `head_hash`: key used for persisted PBFT head records.
/// - `size`: PBFT chain size including null-anchor blocks.
/// - `non_empty_size`: PBFT chain size excluding null-anchor blocks.
/// - `last_pbft_block_hash`: last appended PBFT block hash.
/// - `last_non_null_pbft_dag_anchor_hash`: latest non-null pivot DAG anchor.
///
/// Invariants:
/// - `non_empty_size <= size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbftChainHead {
    pub head_hash: H256,
    pub size: u64,
    pub non_empty_size: u64,
    pub last_pbft_block_hash: H256,
    pub last_non_null_pbft_dag_anchor_hash: H256,
}

/// Result of loading and normalizing PBFT-chain head state from storage.
///
/// Inputs/outputs:
/// - `head`: structured PBFT-chain head used to construct the in-memory runtime.
/// - `initialized_default`: true when storage had no legacy zero-head entry and
///   Rust persisted the default head record before returning.
///
/// Invariants and edge behavior:
/// - Missing head storage initializes the same zero-head record expected by the
///   legacy C++ path.
/// - Existing JSON is parsed for the public fields and the hidden
///   last-non-null anchor is recovered by walking canonical stored PBFT blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbftChainStorageRestore {
    pub head: PbftChainHead,
    pub initialized_default: bool,
}

/// Persisted PBFT-head identity used by fail-closed storage migrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbftChainPersistedHeadIdentity {
    /// Number of finalized PBFT blocks represented by the persisted head.
    pub size: u64,
    /// Hash of the latest finalized PBFT block.
    pub last_pbft_block_hash: H256,
}

/// Loads only the persisted PBFT-head identity without initializing storage.
///
/// This read-only helper parses the existing zero-key legacy JSON row and
/// returns `None` when it is absent. It deliberately does not recover anchors
/// or create the default head, making it suitable for fail-closed migrations
/// that need finalized size/hash authority without startup side effects.
pub fn load_persisted_pbft_chain_head_identity(
    storage: &Storage,
) -> Result<Option<PbftChainPersistedHeadIdentity>> {
    let Some(bytes) = storage.pbft().head(H256::zero())? else {
        return Ok(None);
    };
    let head = parse_legacy_head_json(&bytes).context("PBFT_CHAIN_PARSE_HEAD_IDENTITY")?;
    validate_head(head).context("PBFT_CHAIN_VALIDATE_HEAD_IDENTITY")?;
    Ok(Some(PbftChainPersistedHeadIdentity {
        size: head.size,
        last_pbft_block_hash: head.last_pbft_block_hash,
    }))
}

/// Result of loading a PBFT block payload by hash from Rust storage.
///
/// `found = false` represents the legacy optional return when the hash has no
/// finalized period mapping. Corrupt mappings, missing period data, malformed
/// payloads, or hash mismatches are returned as errors because callers cannot
/// safely materialize a C++ `PbftBlock` sidecar from those rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftBlockStorageLookup {
    pub found: bool,
    pub block_rlp: Vec<u8>,
}

/// Candidate PBFT block validation result.
///
/// `Valid` means the candidate extends the current PBFT head. Mismatch variants
/// carry expected and actual values for explicit caller-side diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbftBlockValidation {
    Valid,
    PeriodMismatch { expected: u64, actual: u64 },
    PreviousHashMismatch { expected: H256, actual: H256 },
}

/// In-memory PBFT chain state and transition rules.
///
/// This type owns only runtime state transitions:
/// - project and apply head updates for accepted PBFT blocks
/// - validate next-block period and previous-hash linkage
///
/// Storage/database lookup is composed by [`PbftChainService`]. Legacy JSON
/// formatting remains a public C++ compatibility concern.
#[derive(Debug, Clone)]
pub struct PbftChain {
    head: PbftChainHead,
}

impl PbftChain {
    /// Creates a PBFT runtime state object from an externally loaded head.
    ///
    /// Returns an error when invariants are invalid.
    pub fn new(head: PbftChainHead) -> Result<Self> {
        validate_head(head)?;
        Ok(Self { head })
    }

    /// Returns the current PBFT head snapshot.
    pub fn head(&self) -> PbftChainHead {
        self.head
    }

    /// Returns the projected PBFT head after appending a candidate block.
    ///
    /// Inputs:
    /// - `block_hash`: appended PBFT block hash.
    /// - `anchor_hash`: appended PBFT block pivot DAG anchor hash.
    ///
    /// Outputs:
    /// - Updated head snapshot without mutating internal state.
    ///
    /// Error behavior:
    /// - Returns an error if chain size arithmetic overflows.
    pub fn project_update(&self, block_hash: H256, anchor_hash: H256) -> Result<PbftChainHead> {
        let size = self
            .head
            .size
            .checked_add(1)
            .ok_or_else(|| anyhow!("pbft chain size overflow"))?;

        let (non_empty_size, last_non_null_pbft_dag_anchor_hash) = if anchor_hash == H256::zero() {
            (
                self.head.non_empty_size,
                self.head.last_non_null_pbft_dag_anchor_hash,
            )
        } else {
            (
                self.head
                    .non_empty_size
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("pbft non-empty chain size overflow"))?,
                anchor_hash,
            )
        };

        Ok(PbftChainHead {
            head_hash: self.head.head_hash,
            size,
            non_empty_size,
            last_pbft_block_hash: block_hash,
            last_non_null_pbft_dag_anchor_hash,
        })
    }

    /// Returns the projected legacy persisted-head JSON fields for a candidate block.
    ///
    /// Legacy callers compute persisted `pbft_head` JSON before the full pivot DAG
    /// anchor is passed into `update`. This projection therefore accepts only the
    /// null/non-null anchor classification and preserves the current hidden
    /// `last_non_null_pbft_dag_anchor_hash` field.
    pub fn project_legacy_json_head(
        &self,
        block_hash: H256,
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHead> {
        let size = self
            .head
            .size
            .checked_add(1)
            .ok_or_else(|| anyhow!("pbft chain size overflow"))?;
        let non_empty_size = if increments_non_empty_size {
            self.head
                .non_empty_size
                .checked_add(1)
                .ok_or_else(|| anyhow!("pbft non-empty chain size overflow"))?
        } else {
            self.head.non_empty_size
        };

        Ok(PbftChainHead {
            head_hash: self.head.head_hash,
            size,
            non_empty_size,
            last_pbft_block_hash: block_hash,
            last_non_null_pbft_dag_anchor_hash: self.head.last_non_null_pbft_dag_anchor_hash,
        })
    }

    /// Applies a PBFT head update in place and returns the new head snapshot.
    pub fn update(&mut self, block_hash: H256, anchor_hash: H256) -> Result<PbftChainHead> {
        let next = self.project_update(block_hash, anchor_hash)?;
        self.head = next;
        Ok(next)
    }

    /// Validates whether a candidate PBFT block extends the current head.
    ///
    /// Rules:
    /// - candidate period must equal `head.size + 1`
    /// - candidate prev hash must equal `head.last_pbft_block_hash`
    pub fn validate_next_block(
        &self,
        candidate_period: u64,
        candidate_prev_hash: H256,
    ) -> PbftBlockValidation {
        let Some(expected_period) = self.head.size.checked_add(1) else {
            return PbftBlockValidation::PeriodMismatch {
                expected: u64::MAX,
                actual: candidate_period,
            };
        };

        if expected_period != candidate_period {
            return PbftBlockValidation::PeriodMismatch {
                expected: expected_period,
                actual: candidate_period,
            };
        }

        if self.head.last_pbft_block_hash != candidate_prev_hash {
            return PbftBlockValidation::PreviousHashMismatch {
                expected: self.head.last_pbft_block_hash,
                actual: candidate_prev_hash,
            };
        }

        PbftBlockValidation::Valid
    }
}

/// Lock-protected native PBFT-chain state.
///
/// `initialized_default` records whether restoration created the legacy
/// default head row. The state never contains bridge or CXX carriers.
pub struct PbftChainState {
    /// Deterministic in-memory head state and transition rules.
    pub state: PbftChain,
    /// Whether startup restoration persisted the missing legacy default head.
    pub initialized_default: bool,
}

/// Application-owned PBFT-chain runtime.
///
/// This CXX-free owner binds storage lifetime, startup restoration, the sibling
/// chain lock, head transitions, validation, and storage-backed block lookup.
/// Storage is retained by `Arc`, so public compatibility readers remain valid
/// after the originating C++ database wrapper is destroyed.
#[derive(Clone)]
pub struct PbftChainService {
    storage: Option<Arc<Storage>>,
    state: Arc<RwLock<PbftChainState>>,
}

impl PbftChainService {
    /// Restores the canonical PBFT head before publishing the runtime.
    pub fn restore(storage: Arc<Storage>) -> Result<Self> {
        let restored = restore_pbft_chain_from_storage(storage.as_ref())?;
        Ok(Self {
            storage: Some(storage),
            state: Arc::new(RwLock::new(PbftChainState {
                state: PbftChain::new(restored.head)?,
                initialized_default: restored.initialized_default,
            })),
        })
    }

    /// Creates a native runtime from explicit compatibility parts.
    ///
    /// This temporary constructor supports bridge fixtures until the complete
    /// PBFT application owner moves native. Storage-backed lookup methods fail
    /// with `PBFT_CHAIN_STORAGE_HANDLE_MISSING` when `storage` is absent.
    pub fn from_parts(
        storage: Option<Arc<Storage>>,
        head: PbftChainHead,
        initialized_default: bool,
    ) -> Result<Self> {
        Ok(Self {
            storage,
            state: Arc::new(RwLock::new(PbftChainState {
                state: PbftChain::new(head)?,
                initialized_default,
            })),
        })
    }

    /// Borrows chain state for a cross-domain PBFT operation.
    ///
    /// This temporary escape hatch supports finalization and leader selection
    /// until their application owner moves out of the bridge.
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, PbftChainState>> {
        self.state.read()
    }

    /// Mutably borrows chain state for a cross-domain PBFT operation.
    pub fn write(&self) -> LockResult<RwLockWriteGuard<'_, PbftChainState>> {
        self.state.write()
    }

    /// Reports whether restore initialized the legacy default head row.
    pub fn initialized_default(&self) -> bool {
        self.state
            .read()
            .expect("PBFT chain lock poisoned")
            .initialized_default
    }

    /// Returns the current head snapshot.
    pub fn head(&self) -> PbftChainHead {
        self.state
            .read()
            .expect("PBFT chain lock poisoned")
            .state
            .head()
    }

    /// Returns the current head snapshot without panicking on lock poison.
    ///
    /// Native application pipelines use this fallible form so a poisoned
    /// sibling aborts the operation before any external effects are queued.
    pub fn try_head(&self) -> Result<PbftChainHead> {
        Ok(self
            .state
            .read()
            .map_err(|_| anyhow!("PBFT_CHAIN_SERVICE_LOCK_POISONED"))?
            .state
            .head())
    }

    /// Projects legacy persisted-head fields without mutation.
    pub fn project_legacy_json_head(
        &self,
        block_hash: H256,
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHead> {
        self.try_project_legacy_json_head(block_hash, increments_non_empty_size)
    }

    /// Projects legacy persisted-head fields without mutation.
    ///
    /// `lock` failures are surfaced as `PBFT_CHAIN_SERVICE_LOCK_POISONED` so
    /// callers can fail closed without panicking across FFI boundaries.
    pub fn try_project_legacy_json_head(
        &self,
        block_hash: H256,
        increments_non_empty_size: bool,
    ) -> Result<PbftChainHead> {
        self.state
            .read()
            .map_err(|_| anyhow!("PBFT_CHAIN_SERVICE_LOCK_POISONED"))?
            .state
            .project_legacy_json_head(block_hash, increments_non_empty_size)
    }

    /// Projects legacy persisted-head JSON bytes without mutation.
    ///
    /// This helper centralizes the legacy-compatible serialization path and
    /// returns lock poison as `PBFT_CHAIN_SERVICE_LOCK_POISONED`.
    pub fn project_legacy_json_head_payload(
        &self,
        block_hash: H256,
        increments_non_empty_size: bool,
    ) -> Result<Vec<u8>> {
        let projected = self.try_project_legacy_json_head(block_hash, increments_non_empty_size)?;
        Ok(legacy_head_json(projected).into_bytes())
    }

    /// Applies one in-memory accepted-block head transition.
    pub fn update(&self, block_hash: H256, anchor_hash: H256) -> Result<PbftChainHead> {
        self.state
            .write()
            .expect("PBFT chain lock poisoned")
            .state
            .update(block_hash, anchor_hash)
    }

    /// Checks whether the runtime-owned storage contains a finalized block.
    pub fn block_exists(&self, block_hash: H256) -> Result<bool> {
        pbft_block_exists_in_storage(self.storage()?, block_hash)
    }

    /// Loads one canonical finalized PBFT block payload from runtime storage.
    pub fn block_rlp(&self, block_hash: H256) -> Result<PbftBlockStorageLookup> {
        load_pbft_block_from_storage(self.storage()?, block_hash)
    }

    /// Validates whether a candidate period/previous hash extends the head.
    pub fn validate_block(&self, period: u64, prev_hash: H256) -> PbftBlockValidation {
        self.state
            .read()
            .expect("PBFT chain lock poisoned")
            .state
            .validate_next_block(period, prev_hash)
    }

    /// Fallibly validates whether a candidate extends the current PBFT head.
    ///
    /// The result matches [`Self::validate_block`], but lock poison is returned
    /// as `PBFT_CHAIN_SERVICE_LOCK_POISONED` so native application pipelines can
    /// abort without unwinding across an FFI boundary. Validation holds only the
    /// sibling read lock and performs no mutation.
    pub fn try_validate_block(&self, period: u64, prev_hash: H256) -> Result<PbftBlockValidation> {
        Ok(self
            .state
            .read()
            .map_err(|_| anyhow!("PBFT_CHAIN_SERVICE_LOCK_POISONED"))?
            .state
            .validate_next_block(period, prev_hash))
    }

    fn storage(&self) -> Result<&Storage> {
        self.storage
            .as_deref()
            .ok_or_else(|| anyhow!("PBFT_CHAIN_STORAGE_HANDLE_MISSING"))
    }
}

fn validate_head(head: PbftChainHead) -> Result<()> {
    if head.non_empty_size > head.size {
        return Err(anyhow!(
            "invalid pbft head: non_empty_size ({}) exceeds size ({})",
            head.non_empty_size,
            head.size
        ));
    }
    Ok(())
}

/// Restores PBFT-chain head state directly from `rustaxa-storage`.
///
/// Inputs:
/// - `storage`: native Rust storage handle for PBFT-chain rows.
///
/// Outputs:
/// - Structured head state and whether a default head row was initialized.
///
/// Invariants and edge behavior:
/// - The legacy head key is the zero hash, matching the existing
///   `PbftChain::default_head_payload` path.
/// - Missing head storage writes a default JSON payload through
///   `rustaxa-storage` before returning.
/// - Existing head payloads must be valid legacy JSON with string hash fields
///   and unsigned numeric size fields.
/// - Last non-null DAG anchor recovery walks stored PBFT block links by previous
///   hash and errors if a referenced block cannot be loaded.
pub fn restore_pbft_chain_from_storage(storage: &Storage) -> Result<PbftChainStorageRestore> {
    let default = default_head();
    let Some(head_bytes) = storage.pbft().head(default.head_hash)? else {
        storage
            .pbft()
            .write_head(default.head_hash, legacy_head_json(default).as_bytes())
            .context("PBFT_CHAIN_WRITE_DEFAULT_HEAD")?;
        return Ok(PbftChainStorageRestore {
            head: default,
            initialized_default: true,
        });
    };

    let mut head = parse_legacy_head_json(&head_bytes).context("PBFT_CHAIN_PARSE_HEAD")?;
    head.last_non_null_pbft_dag_anchor_hash =
        recover_last_non_null_anchor(storage, head.last_pbft_block_hash)
            .context("PBFT_CHAIN_RECOVER_LAST_ANCHOR")?;
    validate_head(head)?;

    Ok(PbftChainStorageRestore {
        head,
        initialized_default: false,
    })
}

/// Returns whether Rust storage contains a PBFT block-period index entry.
///
/// This is the Rust-owned equivalent of `DbStorage::pbftBlockInDb` for
/// Rust-mode consensus shims.
pub fn pbft_block_exists_in_storage(storage: &Storage, pbft_block_hash: H256) -> Result<bool> {
    storage.pbft().exists(pbft_block_hash)
}

/// Loads a canonical signed PBFT block RLP payload by finalized PBFT hash.
///
/// The lookup mirrors legacy `DbStorage::getPbftBlock(hash)` while keeping the
/// deterministic storage reads in Rust:
/// - resolve `pbft_block_period`
/// - read period data
/// - extract item 0 as the signed PBFT block
/// - verify the extracted block hash matches the requested hash
pub fn load_pbft_block_from_storage(
    storage: &Storage,
    pbft_block_hash: H256,
) -> Result<PbftBlockStorageLookup> {
    let Some((_, block_rlp)) = load_pbft_block_link_and_rlp(storage, pbft_block_hash)? else {
        return Ok(PbftBlockStorageLookup {
            found: false,
            block_rlp: Vec::new(),
        });
    };
    Ok(PbftBlockStorageLookup {
        found: true,
        block_rlp,
    })
}

fn default_head() -> PbftChainHead {
    PbftChainHead {
        head_hash: H256::zero(),
        size: 0,
        non_empty_size: 0,
        last_pbft_block_hash: H256::zero(),
        last_non_null_pbft_dag_anchor_hash: H256::zero(),
    }
}

fn recover_last_non_null_anchor(storage: &Storage, mut last_pbft_block_hash: H256) -> Result<H256> {
    while last_pbft_block_hash != H256::zero() {
        let Some((link, _)) = load_pbft_block_link_and_rlp(storage, last_pbft_block_hash)? else {
            return Err(anyhow!(
                "missing PBFT block {} while recovering PBFT chain head",
                format_hash(last_pbft_block_hash)
            ));
        };
        if link.pivot_dag_block_hash != H256::zero() {
            return Ok(link.pivot_dag_block_hash);
        }
        last_pbft_block_hash = link.prev_block_hash;
    }
    Ok(H256::zero())
}

fn load_pbft_block_link_and_rlp(
    storage: &Storage,
    pbft_block_hash: H256,
) -> Result<Option<(PbftBlockLink, Vec<u8>)>> {
    let Some(period) = storage.period().by_pbft_hash(pbft_block_hash)? else {
        return Ok(None);
    };
    let period_data = storage.period().data_raw(period)?;
    if period_data.is_empty() {
        return Err(anyhow!(
            "missing period data for PBFT block {} at period {}",
            format_hash(pbft_block_hash),
            period
        ));
    }

    let period_rlp = rlp::Rlp::new(&period_data);
    let block_rlp = period_rlp
        .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)
        .context("PBFT_CHAIN_PERIOD_DATA_PBFT_BLOCK")?;
    let block_bytes = block_rlp.as_raw().to_vec();
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_bytes))
        .context("PBFT_CHAIN_DECODE_PBFT_BLOCK_LINK")?;
    if link.block_hash != pbft_block_hash {
        return Err(anyhow!(
            "PBFT block hash mismatch: requested {}, loaded {}",
            format_hash(pbft_block_hash),
            format_hash(link.block_hash)
        ));
    }

    Ok(Some((link, block_bytes)))
}

fn parse_legacy_head_json(head_bytes: &[u8]) -> Result<PbftChainHead> {
    let value: serde_json::Value = serde_json::from_slice(head_bytes)?;
    Ok(PbftChainHead {
        head_hash: parse_head_hash(&value, "head_hash")?,
        size: parse_head_u64(&value, "size")?,
        non_empty_size: parse_head_u64(&value, "non_empty_size")?,
        last_pbft_block_hash: parse_head_hash(&value, "last_pbft_block_hash")?,
        last_non_null_pbft_dag_anchor_hash: H256::zero(),
    })
}

fn parse_head_hash(value: &serde_json::Value, field: &str) -> Result<H256> {
    let raw = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("missing PBFT head hash field {field}"))?;
    let hex = raw.strip_prefix("0x").unwrap_or(raw);
    if hex.len() != 64 {
        return Err(anyhow!(
            "invalid PBFT head hash field {field}: expected 32-byte hex"
        ));
    }
    Ok(H256::from_slice(&hex_bytes(hex)?))
}

fn parse_head_u64(value: &serde_json::Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("missing PBFT head numeric field {field}"))
}

fn legacy_head_json(head: PbftChainHead) -> String {
    format!(
        "{{\n\t\"head_hash\" : \"{}\",\n\t\"last_pbft_block_hash\" : \"{}\",\n\t\"non_empty_size\" : {},\n\t\"size\" : {}\n}}\n",
        format_hash(head.head_hash),
        format_hash(head.last_pbft_block_hash),
        head.non_empty_size,
        head.size
    )
}

fn format_hash(hash: H256) -> String {
    format!("0x{}", hex_lower(hash.as_bytes()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_bytes(input: &str) -> Result<Vec<u8>> {
    if !input.len().is_multiple_of(2) {
        return Err(anyhow!("hex input has odd length"));
    }
    let mut out = Vec::with_capacity(input.len() / 2);
    for chunk in input.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(anyhow!("invalid hex digit")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, Storage};
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn head(size: u64, non_empty_size: u64, last: u64, last_non_null: u64) -> PbftChainHead {
        PbftChainHead {
            head_hash: H256::zero(),
            size,
            non_empty_size,
            last_pbft_block_hash: hash(last),
            last_non_null_pbft_dag_anchor_hash: hash(last_non_null),
        }
    }

    fn temp_storage(name: &str) -> Storage {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{name}_{}_{}", std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        Storage::new(Config::new(path)).unwrap()
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
    fn rejects_invalid_head_invariant() {
        let err = PbftChain::new(head(2, 3, 0, 0)).unwrap_err().to_string();
        assert!(err.contains("non_empty_size"));
    }

    #[test]
    fn projects_and_updates_non_null_anchor_block() {
        let mut chain = PbftChain::new(head(1, 0, 11, 0)).unwrap();

        let projected = chain.project_update(hash(12), hash(99)).unwrap();
        assert_eq!(projected.size, 2);
        assert_eq!(projected.non_empty_size, 1);
        assert_eq!(projected.last_pbft_block_hash, hash(12));
        assert_eq!(projected.last_non_null_pbft_dag_anchor_hash, hash(99));

        let updated = chain.update(hash(12), hash(99)).unwrap();
        assert_eq!(updated, projected);
        assert_eq!(chain.head(), projected);
    }

    #[test]
    fn updates_null_anchor_without_changing_non_empty_or_last_non_null_anchor() {
        let mut chain = PbftChain::new(head(4, 2, 44, 777)).unwrap();

        let updated = chain.update(hash(45), H256::zero()).unwrap();
        assert_eq!(updated.size, 5);
        assert_eq!(updated.non_empty_size, 2);
        assert_eq!(updated.last_pbft_block_hash, hash(45));
        assert_eq!(updated.last_non_null_pbft_dag_anchor_hash, hash(777));
    }

    #[test]
    fn projects_legacy_json_head_from_anchor_classification() {
        let chain = PbftChain::new(head(4, 2, 44, 777)).unwrap();

        let non_empty = chain.project_legacy_json_head(hash(45), true).unwrap();
        assert_eq!(non_empty.size, 5);
        assert_eq!(non_empty.non_empty_size, 3);
        assert_eq!(non_empty.last_pbft_block_hash, hash(45));
        assert_eq!(non_empty.last_non_null_pbft_dag_anchor_hash, hash(777));

        let empty = chain.project_legacy_json_head(hash(46), false).unwrap();
        assert_eq!(empty.size, 5);
        assert_eq!(empty.non_empty_size, 2);
        assert_eq!(empty.last_pbft_block_hash, hash(46));
        assert_eq!(empty.last_non_null_pbft_dag_anchor_hash, hash(777));
    }

    #[test]
    fn validates_next_block_period_and_previous_hash() {
        let chain = PbftChain::new(head(3, 2, 333, 222)).unwrap();

        assert_eq!(
            chain.validate_next_block(4, hash(333)),
            PbftBlockValidation::Valid
        );
        assert_eq!(
            chain.validate_next_block(5, hash(333)),
            PbftBlockValidation::PeriodMismatch {
                expected: 4,
                actual: 5
            }
        );
        assert_eq!(
            chain.validate_next_block(4, hash(999)),
            PbftBlockValidation::PreviousHashMismatch {
                expected: hash(333),
                actual: hash(999)
            }
        );
    }

    #[test]
    fn storage_restore_initializes_default_head_when_missing() {
        let storage = temp_storage("rustaxa_consensus_pbft_chain_restore_default");

        let restored = restore_pbft_chain_from_storage(&storage).unwrap();

        assert!(restored.initialized_default);
        assert_eq!(restored.head, default_head());
        assert!(storage.pbft().head(H256::zero()).unwrap().is_some());
    }

    #[test]
    fn storage_restore_parses_legacy_head_and_recovers_last_anchor() {
        let storage = temp_storage("rustaxa_consensus_pbft_chain_restore_anchor");
        let first = pbft_block_rlp(H256::zero(), hash(100), 1);
        let first_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&first))
            .unwrap()
            .block_hash;
        let second = pbft_block_rlp(first_hash, H256::zero(), 2);
        let second_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&second))
            .unwrap()
            .block_hash;
        storage.period().write(1, &period_data_rlp(&first)).unwrap();
        storage.period().write_pbft_period(first_hash, 1).unwrap();
        storage
            .period()
            .write(2, &period_data_rlp(&second))
            .unwrap();
        storage.period().write_pbft_period(second_hash, 2).unwrap();
        let legacy_head = format!(
            r#"{{"head_hash":"{}","size":2,"non_empty_size":1,"last_pbft_block_hash":"{}"}}"#,
            format_hash(H256::zero()),
            format_hash(second_hash)
        );
        storage
            .pbft()
            .write_head(H256::zero(), legacy_head.as_bytes())
            .unwrap();

        let restored = restore_pbft_chain_from_storage(&storage).unwrap();

        assert!(!restored.initialized_default);
        assert_eq!(restored.head.size, 2);
        assert_eq!(restored.head.non_empty_size, 1);
        assert_eq!(restored.head.last_pbft_block_hash, second_hash);
        assert_eq!(restored.head.last_non_null_pbft_dag_anchor_hash, hash(100));
    }

    #[test]
    fn storage_loads_pbft_block_by_hash_from_period_data() {
        let storage = temp_storage("rustaxa_consensus_pbft_chain_load_block");
        let block = pbft_block_rlp(H256::zero(), hash(9), 1);
        let block_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))
            .unwrap()
            .block_hash;
        storage.period().write(1, &period_data_rlp(&block)).unwrap();
        storage.period().write_pbft_period(block_hash, 1).unwrap();

        let loaded = load_pbft_block_from_storage(&storage, block_hash).unwrap();
        let missing = load_pbft_block_from_storage(&storage, hash(999)).unwrap();

        assert!(loaded.found);
        assert_eq!(loaded.block_rlp, block);
        assert!(!missing.found);
        assert!(missing.block_rlp.is_empty());
    }

    #[test]
    fn service_owns_restore_lock_transitions_and_storage_lifetime_without_cxx() {
        let storage = Arc::new(temp_storage("rustaxa_consensus_pbft_chain_service"));
        let block = pbft_block_rlp(H256::zero(), hash(9), 1);
        let block_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))
            .unwrap()
            .block_hash;
        storage.period().write(1, &period_data_rlp(&block)).unwrap();
        storage.period().write_pbft_period(block_hash, 1).unwrap();

        let service = PbftChainService::restore(storage.clone()).unwrap();
        drop(storage);

        assert!(service.initialized_default());
        assert_eq!(service.head().size, 0);
        assert!(service.block_exists(block_hash).unwrap());
        assert_eq!(service.block_rlp(block_hash).unwrap().block_rlp, block);
        assert_eq!(
            service.validate_block(1, H256::zero()),
            PbftBlockValidation::Valid
        );

        let projected = service.project_legacy_json_head(block_hash, true).unwrap();
        assert_eq!(projected.size, 1);
        assert_eq!(service.head().size, 0);
        let updated = service.update(block_hash, hash(9)).unwrap();
        assert_eq!(updated.size, 1);
        assert_eq!(updated.non_empty_size, 1);
        assert_eq!(service.head(), updated);
    }

    #[test]
    fn fallible_validation_reports_poisoned_chain_lock() {
        let service = PbftChainService::from_parts(None, default_head(), false).unwrap();
        let state = service.state.clone();
        let poison = std::thread::spawn(move || {
            let _guard = state.write().unwrap();
            panic!("poison PBFT chain lock");
        });
        assert!(poison.join().is_err());

        let error = service
            .try_validate_block(1, H256::zero())
            .expect_err("poison must be returned to native pipelines");
        assert_eq!(error.to_string(), "PBFT_CHAIN_SERVICE_LOCK_POISONED");
    }
}
