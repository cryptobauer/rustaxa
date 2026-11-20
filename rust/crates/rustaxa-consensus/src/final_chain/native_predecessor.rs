//! Native source inputs from one application database sequence.
//! A capture retains its owner lifetime without reserving mutable state.

use super::native_session::{
    FinalChainNativeExecutionProbe, FinalChainNativeSessionError, FinalChainPreparedNativeSession,
};
use super::{DposSnapshot, FinalChain, ValidatorRowAuthority, decode_dpos_snapshot_rlp};
use anyhow::{Context, Result, ensure};
use rlp::{Rlp, RlpStream};
use rustaxa_storage::{Column, StorageReadSnapshot};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::evm_correction::CorrectionStage;
use rustaxa_types::{FinalChainBlockNumber, StoredFinalChainBlockHeader};
use std::sync::Arc;

/// A raw-source retention limit. Point reads and decoded states can allocate
/// more memory than this limit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativePredecessorLimits {
    pub(crate) max_retained_source_bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NativePredecessorUnavailable {
    PendingGenesis,
    MissingHead,
    HeadMismatch {
        expected: FinalChainBlockNumber,
        actual: FinalChainBlockNumber,
    },
    PublicationPending,
    MissingPairing,
    MissingHeader(FinalChainBlockNumber),
    MissingNativeSnapshot(FinalChainBlockNumber),
    UnsupportedNativeLayout(FinalChainBlockNumber),
    RetainedSourceLimit,
}

pub(crate) enum NativePredecessorCapture<'a> {
    Ready(PreparedNativePredecessor<'a>),
    Unavailable(NativePredecessorUnavailable),
}

pub(crate) struct PreparedNativePredecessor<'a> {
    owner: &'a FinalChain,
    pending_period: FinalChainBlockNumber,
    correction_stage: CorrectionStage,
    head_raw: [u8; 8],
    pairing_raw: Vec<u8>,
    pairing: RecordedPairing,
    current: Arc<NativeSource>,
    delayed: Arc<NativeSource>,
    retained_source_bytes: usize,
}

impl<'a> PreparedNativePredecessor<'a> {
    pub(crate) fn committed_identity(
        &self,
    ) -> rustaxa_types::concrete_state::ConcreteStateIdentity {
        rustaxa_types::concrete_state::ConcreteStateIdentity {
            period: self.current.period,
            state_root: self.current.header.state_root.0,
        }
    }

    pub(crate) fn delayed_identity(&self) -> rustaxa_types::concrete_state::ConcreteStateIdentity {
        rustaxa_types::concrete_state::ConcreteStateIdentity {
            period: self.delayed.period,
            state_root: self.delayed.header.state_root.0,
        }
    }

    pub(crate) fn pairing_identity(
        &self,
    ) -> rustaxa_types::concrete_lifecycle::FinalChainConcreteIdentity {
        rustaxa_types::concrete_lifecycle::FinalChainConcreteIdentity {
            policy_version: u64::from(self.pairing.policy_version),
            database_id: self.pairing.database_id,
            chain_id: self.pairing.chain_id,
        }
    }

    pub(crate) fn pairing_storage_genesis(&self) -> [u8; 32] {
        self.pairing.storage_genesis
    }

    pub(crate) fn matches_owner(&self, owner: &FinalChain) -> bool {
        std::ptr::eq(self.owner, owner)
    }

    pub(crate) fn retained_source_bytes(&self) -> usize {
        self.retained_source_bytes
    }

    pub(crate) fn same_sources(&self, other: &Self) -> bool {
        std::ptr::eq(self.owner, other.owner)
            && self.pending_period == other.pending_period
            && self.correction_stage == other.correction_stage
            && self.head_raw == other.head_raw
            && self.pairing_raw == other.pairing_raw
            && self.current.period == other.current.period
            && self.current.header_raw == other.current.header_raw
            && self.current.native_raw == other.current.native_raw
            && self.delayed.period == other.delayed.period
            && self.delayed.header_raw == other.delayed.header_raw
            && self.delayed.native_raw == other.delayed.native_raw
    }

    pub(crate) fn source_digest(&self) -> [u8; 32] {
        use tiny_keccak::{Hasher, Keccak};
        let mut hash = Keccak::v256();
        hash.update(b"rustaxa-captured-native-predecessor-v1");
        hash.update(&self.pending_period.as_u64().to_be_bytes());
        let stage = format!("{:?}", self.correction_stage);
        for value in [
            stage.as_bytes(),
            self.head_raw.as_slice(),
            self.pairing_raw.as_slice(),
            self.current.header_raw.as_slice(),
            self.current.native_raw.as_slice(),
            self.delayed.header_raw.as_slice(),
            self.delayed.native_raw.as_slice(),
        ] {
            hash.update(&(value.len() as u64).to_be_bytes());
            hash.update(value);
        }
        let mut digest = [0; 32];
        hash.finalize(&mut digest);
        digest
    }

    pub(crate) fn into_owned_transition(
        self,
        owner: &Arc<FinalChain>,
        request_id: [u8; 32],
        state_api_epoch: u64,
        generation: u64,
        reader: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
    ) -> std::result::Result<FinalChainPreparedNativeSession, FinalChainNativeSessionError> {
        if !self.matches_owner(owner.as_ref()) {
            return Err(FinalChainNativeSessionError::Domain(
                "native session owner mismatch".into(),
            ));
        }
        let committed = super::FinalChainExternalEvmCommittedStateDescriptor {
            period: self.current.period,
            state_root: self.current.header.state_root.0,
        };
        let selected = self.delayed_identity();
        let session = self.owner.native_session_from_captured_states(
            request_id,
            self.pending_period,
            self.current.native.clone(),
            self.delayed.period,
            self.delayed.native.clone(),
        )?;
        let parent = Arc::new(self.current.native.clone());
        let source = self.into_owned_source(owner)?;
        FinalChainPreparedNativeSession::from_captured_session(
            session,
            owner,
            source,
            parent,
            committed,
            selected,
            state_api_epoch,
            generation,
            reader,
        )
    }

    fn into_owned_source(
        self,
        owner: &Arc<FinalChain>,
    ) -> std::result::Result<CapturedNativePredecessor, FinalChainNativeSessionError> {
        if !self.matches_owner(owner) {
            return Err(FinalChainNativeSessionError::Domain(
                "native session owner mismatch".into(),
            ));
        }
        Ok(CapturedNativePredecessor {
            owner: Arc::clone(owner),
            pending_period: self.pending_period,
            correction_stage: self.correction_stage,
            head_raw: self.head_raw,
            pairing_raw: self.pairing_raw,
            current: self.current,
            delayed: self.delayed,
            retained_source_bytes: self.retained_source_bytes,
        })
    }

    /// Uses captured native states in a detached probe. The caller must still
    /// qualify the concrete reader and the ordinary account read source.
    pub(crate) fn into_execution_probe(
        self,
        request_id: [u8; 32],
        state_api_epoch: u64,
        generation: u64,
        reader: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
    ) -> std::result::Result<FinalChainNativeExecutionProbe<'a>, FinalChainNativeSessionError> {
        if self.owner.correction_stage_at(self.pending_period) != self.correction_stage {
            return Err(FinalChainNativeSessionError::Domain(
                "native probe correction stage unavailable".into(),
            ));
        }
        let committed = super::FinalChainExternalEvmCommittedStateDescriptor {
            period: self.current.period,
            state_root: self.current.header.state_root.0,
        };
        let delayed = rustaxa_types::concrete_state::ConcreteStateIdentity {
            period: self.delayed.period,
            state_root: self.delayed.header.state_root.0,
        };
        let session = self.owner.native_session_from_captured_states(
            request_id,
            self.pending_period,
            self.current.native.clone(),
            self.delayed.period,
            self.delayed.native.clone(),
        )?;
        FinalChainNativeExecutionProbe::from_captured_session(
            session,
            committed,
            delayed,
            state_api_epoch,
            generation,
            reader,
        )
    }
}

pub(super) struct CapturedNativePredecessor {
    owner: Arc<FinalChain>,
    pending_period: FinalChainBlockNumber,
    correction_stage: CorrectionStage,
    head_raw: [u8; 8],
    pairing_raw: Vec<u8>,
    current: Arc<NativeSource>,
    delayed: Arc<NativeSource>,
    retained_source_bytes: usize,
}

impl CapturedNativePredecessor {
    pub(in crate::final_chain) fn validate(
        &self,
        owner: &FinalChain,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        if !std::ptr::eq(self.owner.as_ref(), owner) {
            return Err(FinalChainNativeSessionError::Domain(
                "native captured owner mismatch".into(),
            ));
        }
        let fresh = capture(
            owner,
            self.pending_period,
            NativePredecessorLimits {
                max_retained_source_bytes: self.retained_source_bytes,
            },
        )
        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let NativePredecessorCapture::Ready(fresh) = fresh else {
            return Err(FinalChainNativeSessionError::Domain(
                "native captured source unavailable".into(),
            ));
        };
        if self.pending_period != fresh.pending_period
            || self.correction_stage != fresh.correction_stage
            || self.head_raw != fresh.head_raw
            || self.pairing_raw != fresh.pairing_raw
            || self.current.period != fresh.current.period
            || self.current.header_raw != fresh.current.header_raw
            || self.current.native_raw != fresh.current.native_raw
            || self.delayed.period != fresh.delayed.period
            || self.delayed.header_raw != fresh.delayed.header_raw
            || self.delayed.native_raw != fresh.delayed.native_raw
        {
            return Err(FinalChainNativeSessionError::Domain(
                "native captured source changed".into(),
            ));
        }
        Ok(())
    }
}

struct NativeSource {
    period: FinalChainBlockNumber,
    header_raw: Vec<u8>,
    header: StoredFinalChainBlockHeader,
    native_raw: Vec<u8>,
    native: DposSnapshot,
}

struct RawNativeSource {
    period: FinalChainBlockNumber,
    header_raw: Vec<u8>,
    header: StoredFinalChainBlockHeader,
    native_raw: Vec<u8>,
}

impl RawNativeSource {
    fn decode(self) -> Result<NativeSource> {
        let native = decode_dpos_snapshot_rlp(&self.native_raw)
            .with_context(|| format!("native predecessor snapshot {}", self.period.as_u64()))?;
        Ok(NativeSource {
            period: self.period,
            header_raw: self.header_raw,
            header: self.header,
            native_raw: self.native_raw,
            native,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RecordedPairing {
    policy_version: u32,
    storage_genesis: [u8; 32],
    database_id: [u8; 32],
    chain_id: [u8; 32],
}

struct RetainedSourceBudget {
    used: usize,
    limit: usize,
}

impl RetainedSourceBudget {
    fn add(&mut self, bytes: usize) -> bool {
        match self.used.checked_add(bytes) {
            Some(total) if total <= self.limit => {
                self.used = total;
                true
            }
            _ => false,
        }
    }
}

enum SourceRead {
    Present(RawNativeSource),
    Unavailable(NativePredecessorUnavailable),
}

struct NativeSourceReader<'a> {
    owner: &'a FinalChain,
    snapshot: StorageReadSnapshot<'a>,
    budget: RetainedSourceBudget,
}

impl<'a> NativeSourceReader<'a> {
    fn new(owner: &'a FinalChain, limits: NativePredecessorLimits) -> Self {
        Self {
            owner,
            snapshot: owner.storage.read_snapshot(),
            budget: RetainedSourceBudget {
                used: 0,
                limit: limits.max_retained_source_bytes,
            },
        }
    }

    fn source(&mut self, period: FinalChainBlockNumber) -> Result<SourceRead> {
        let Some(header_raw) = self.snapshot.get(
            Column::FinalChainBlkByNumber,
            &period.as_u64().to_le_bytes(),
        )?
        else {
            return Ok(SourceRead::Unavailable(
                NativePredecessorUnavailable::MissingHeader(period),
            ));
        };
        if !self.budget.add(header_raw.len()) {
            return Ok(SourceRead::Unavailable(
                NativePredecessorUnavailable::RetainedSourceLimit,
            ));
        }
        exact_list(&header_raw, 7).context("native predecessor stored header framing")?;
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header_raw))?;
        let Some(native_raw) = self.snapshot.dpos_snapshot_raw(period.as_u64())? else {
            return Ok(SourceRead::Unavailable(
                NativePredecessorUnavailable::MissingNativeSnapshot(period),
            ));
        };
        if !self.budget.add(native_raw.len()) {
            return Ok(SourceRead::Unavailable(
                NativePredecessorUnavailable::RetainedSourceLimit,
            ));
        }
        Ok(SourceRead::Present(RawNativeSource {
            period,
            header_raw,
            header,
            native_raw,
        }))
    }

    fn capture(
        mut self,
        pending_period: FinalChainBlockNumber,
    ) -> Result<NativePredecessorCapture<'a>> {
        use NativePredecessorCapture::{Ready, Unavailable};
        use NativePredecessorUnavailable as Missing;

        let Some(parent) = pending_period.checked_sub_distance(1) else {
            return Ok(Unavailable(Missing::PendingGenesis));
        };
        let delayed_period = parent.saturating_sub_distance(self.owner.dpos_delegation_delay);
        let Some(raw) = self.snapshot.get(
            Column::FinalChainMeta,
            &FinalChain::DB_META_LAST_NUMBER.to_le_bytes(),
        )?
        else {
            return Ok(Unavailable(Missing::MissingHead));
        };
        let head_raw: [u8; 8] = raw
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("native predecessor head must contain eight bytes"))?;
        let actual = FinalChainBlockNumber::new(u64::from_le_bytes(head_raw));
        if actual != parent {
            return Ok(Unavailable(Missing::HeadMismatch {
                expected: parent,
                actual,
            }));
        }
        if !self.budget.add(head_raw.len()) {
            return Ok(Unavailable(Missing::RetainedSourceLimit));
        }
        if self
            .snapshot
            .external_evm_pending_publication_raw()?
            .is_some()
        {
            return Ok(Unavailable(Missing::PublicationPending));
        }
        let Some(pairing_raw) = self.snapshot.concrete_state_pairing_raw()? else {
            return Ok(Unavailable(Missing::MissingPairing));
        };
        if !self.budget.add(pairing_raw.len()) {
            return Ok(Unavailable(Missing::RetainedSourceLimit));
        }
        let pairing = recorded_pairing(&pairing_raw)?;
        let current_raw = match self.source(parent)? {
            SourceRead::Present(source) => source,
            SourceRead::Unavailable(reason) => return Ok(Unavailable(reason)),
        };
        let delayed_raw = if delayed_period == parent {
            None
        } else {
            match self.source(delayed_period)? {
                SourceRead::Present(source) => Some(source),
                SourceRead::Unavailable(reason) => return Ok(Unavailable(reason)),
            }
        };
        let current = Arc::new(current_raw.decode()?);
        let delayed = match delayed_raw {
            Some(source) => Arc::new(source.decode()?),
            None => Arc::clone(&current),
        };
        if self
            .owner
            .pending_native_session_requires_complete_layout(pending_period)
        {
            for source in [&current, &delayed] {
                if matches!(
                    &source.native.validator_row_authority,
                    ValidatorRowAuthority::CompatibilityOnly
                ) {
                    return Ok(Unavailable(Missing::UnsupportedNativeLayout(source.period)));
                }
            }
        }
        Ok(Ready(PreparedNativePredecessor {
            owner: self.owner,
            pending_period,
            correction_stage: self.owner.correction_stage_at(pending_period),
            head_raw,
            pairing_raw,
            pairing,
            current,
            delayed,
            retained_source_bytes: self.budget.used,
        }))
    }
}

pub(super) fn capture(
    owner: &FinalChain,
    pending_period: FinalChainBlockNumber,
    limits: NativePredecessorLimits,
) -> Result<NativePredecessorCapture<'_>> {
    if pending_period.is_genesis() {
        return Ok(NativePredecessorCapture::Unavailable(
            NativePredecessorUnavailable::PendingGenesis,
        ));
    }
    NativeSourceReader::new(owner, limits).capture(pending_period)
}

fn exact_list(raw: &[u8], expected: usize) -> Result<()> {
    let list = Rlp::new(raw);
    let outer = list.payload_info()?;
    ensure!(
        list.is_list() && outer.total() == raw.len(),
        "native source list framing"
    );
    let mut consumed = 0_usize;
    let mut count = 0_usize;
    for child in list.iter() {
        let bytes = child.as_raw();
        ensure!(
            !bytes.is_empty() && child.payload_info()?.total() == bytes.len(),
            "native source child framing"
        );
        consumed = consumed
            .checked_add(bytes.len())
            .ok_or_else(|| anyhow::anyhow!("native source child length overflow"))?;
        count = count
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("native source item count overflow"))?;
    }
    ensure!(
        consumed == outer.value_len && count == expected,
        "native source list shape"
    );
    Ok(())
}

fn recorded_pairing(raw: &[u8]) -> Result<RecordedPairing> {
    exact_list(raw, 4).context("native predecessor pairing framing")?;
    let list = Rlp::new(raw);
    let policy_version: u32 = list.val_at(0)?;
    ensure!(policy_version == 1, "native predecessor pairing policy");
    let field = |index| -> Result<[u8; 32]> {
        list.at(index)?
            .data()?
            .try_into()
            .map_err(|_| anyhow::anyhow!("native predecessor pairing field width"))
    };
    let storage_genesis = field(1)?;
    let database_id = field(2)?;
    let chain_id = field(3)?;
    ensure!(
        database_id != [0; 32],
        "native predecessor pairing database identity"
    );
    let mut encoded = RlpStream::new_list(4);
    encoded.append(&policy_version);
    encoded.append(&storage_genesis.as_slice());
    encoded.append(&database_id.as_slice());
    encoded.append(&chain_id.as_slice());
    ensure!(
        encoded.out().as_ref() == raw,
        "native predecessor pairing canonical bytes"
    );
    Ok(RecordedPairing {
        policy_version,
        storage_genesis,
        database_id,
        chain_id,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{StoredUndelegationCounter, ValidatorRowShape, encode_dpos_snapshot_rlp};
    use super::*;
    use ethereum_types::{H256, U256};
    use rustaxa_storage::{Config, Storage};
    use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlpOwned;
    use rustaxa_types::{
        DposTokenAmount, FinalChainGas, FinalChainLogBloom, FinalChainRewardsConfig,
        GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const VALIDATOR: [u8; 20] = [0x11; 20];
    const MAX_SOURCE: usize = 1024 * 1024;

    struct RetainedRoot(PathBuf);

    impl Drop for RetainedRoot {
        fn drop(&mut self) {
            crate::closeout_test_scratch::preserve(&self.0);
        }
    }

    struct Fixture {
        owner: Option<FinalChain>,
        storage: Option<Arc<Storage>>,
        config: Config,
        root: RetainedRoot,
    }

    impl Fixture {
        fn new(delay: u64, require_complete: bool) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = crate::closeout_test_scratch::new_path(
                std::env::temp_dir()
                    .join(format!("native_predecessor_{}_{nonce}", std::process::id())),
                true,
            );
            let root = RetainedRoot(path.clone());
            let mut config = Config::new(path);
            config.db_write_buffer_size = 2 * 1024 * 1024;
            config.max_total_wal_size = 32 * 1024 * 1024;
            config.max_open_files = 32;
            let storage = Arc::new(Storage::new(config.clone()).unwrap());
            let amount = |value| DposTokenAmount::from(U256::from(value));
            let dpos = GenesisDposConfig {
                eligibility_balance_threshold: amount(1_u64),
                vote_eligibility_balance_step: amount(1_u64),
                validator_maximum_stake: amount(1000_u64),
                minimum_deposit: amount(1_u64),
                delegation_delay: delay,
                ..Default::default()
            };
            let rewards = if require_complete {
                FinalChainRewardsConfig {
                    magnolia_period: FinalChainBlockNumber::MAX,
                    cornus_period: FinalChainBlockNumber::MAX,
                    ..Default::default()
                }
            } else {
                FinalChainRewardsConfig::default()
            };
            let validator = GenesisValidator {
                address: VALIDATOR,
                vrf_key: [0x22; 32],
                total_stake: vec![10],
                delegations: vec![(VALIDATOR, vec![10])],
                metadata: GenesisValidatorMetadata {
                    owner: VALIDATOR,
                    ..Default::default()
                },
            };
            let owner = FinalChain::new_with_rewards_config(
                Arc::clone(&storage),
                FinalChainGas::new(1_000_000),
                0,
                vec![],
                vec![validator],
                dpos,
                rewards,
            )
            .unwrap();
            Self {
                owner: Some(owner),
                storage: Some(storage),
                config,
                root,
            }
        }

        fn owner(&self) -> &FinalChain {
            self.owner.as_ref().unwrap()
        }

        fn storage(&self) -> &Storage {
            self.storage.as_ref().unwrap()
        }

        fn snapshot(&self, tag: u8) -> DposSnapshot {
            let mut state = self
                .owner()
                .dpos_snapshots
                .lock()
                .unwrap()
                .get(&FinalChainBlockNumber::GENESIS)
                .unwrap()
                .clone();
            state.slashing_jail_blocks.insert(VALIDATOR, u64::from(tag));
            state.slashing_jailed_validators = vec![VALIDATOR];
            state.slashing_double_voting_proofs.insert([tag; 32]);
            state
        }

        fn records(&self, pending: u64, current_tag: u8, delayed_tag: u8) -> Seed {
            let parent = pending.checked_sub(1).unwrap();
            let delayed = parent.saturating_sub(self.owner().dpos_delegation_delay);
            let current_native = encode_dpos_snapshot_rlp(&self.snapshot(current_tag)).unwrap();
            let current_header = header(current_tag);
            let (delayed_header, delayed_native) = if delayed == parent {
                (current_header.clone(), current_native.clone())
            } else {
                (
                    header(delayed_tag),
                    encode_dpos_snapshot_rlp(&self.snapshot(delayed_tag)).unwrap(),
                )
            };
            Seed {
                parent,
                delayed,
                current_header,
                current_native,
                delayed_header,
                delayed_native,
                pairing: pairing(current_tag),
            }
        }

        fn capture(&self, pending: u64, bytes: usize) -> Result<NativePredecessorCapture<'_>> {
            self.owner().capture_native_predecessor(
                FinalChainBlockNumber::new(pending),
                NativePredecessorLimits {
                    max_retained_source_bytes: bytes,
                },
            )
        }
    }

    #[derive(Clone)]
    struct Seed {
        parent: u64,
        delayed: u64,
        current_header: Vec<u8>,
        current_native: Vec<u8>,
        delayed_header: Vec<u8>,
        delayed_native: Vec<u8>,
        pairing: Vec<u8>,
    }

    impl Seed {
        fn bytes(&self) -> usize {
            8 + self.pairing.len()
                + self.current_header.len()
                + self.current_native.len()
                + if self.delayed == self.parent {
                    0
                } else {
                    self.delayed_header.len() + self.delayed_native.len()
                }
        }
    }

    fn dpos_key(period: u64) -> Vec<u8> {
        [b"rustaxa:dpos_snapshot:".as_slice(), &period.to_le_bytes()].concat()
    }

    fn write(store: &Storage, entries: Vec<(Column, Vec<u8>, Option<Vec<u8>>)>) {
        let mut batch = store.create_write_batch();
        for (column, key, value) in entries {
            match value {
                Some(value) => store
                    .batch_put_raw(&mut batch, column, &key, &value)
                    .unwrap(),
                None => store.batch_delete_raw(&mut batch, column, &key).unwrap(),
            }
        }
        store.commit_write_batch_with_sync(batch, false).unwrap();
    }

    fn install(store: &Storage, seed: &Seed) {
        let mut entries = vec![
            (
                Column::FinalChainMeta,
                FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                Some(seed.parent.to_le_bytes().to_vec()),
            ),
            (
                Column::FinalChainMeta,
                b"rustaxa:concrete_state_pairing:v1".to_vec(),
                Some(seed.pairing.clone()),
            ),
            (
                Column::FinalChainBlkByNumber,
                seed.parent.to_le_bytes().to_vec(),
                Some(seed.current_header.clone()),
            ),
            (
                Column::FinalChainMeta,
                dpos_key(seed.parent),
                Some(seed.current_native.clone()),
            ),
        ];
        if seed.delayed != seed.parent {
            entries.push((
                Column::FinalChainBlkByNumber,
                seed.delayed.to_le_bytes().to_vec(),
                Some(seed.delayed_header.clone()),
            ));
            entries.push((
                Column::FinalChainMeta,
                dpos_key(seed.delayed),
                Some(seed.delayed_native.clone()),
            ));
        }
        write(store, entries);
    }

    fn header(tag: u8) -> Vec<u8> {
        StoredBlockHeaderRlpOwned::from(&StoredFinalChainBlockHeader {
            parent_hash: H256::repeat_byte(tag),
            state_root: H256::repeat_byte(tag.wrapping_add(1)),
            transactions_root: H256::repeat_byte(tag.wrapping_add(2)),
            receipts_root: H256::repeat_byte(tag.wrapping_add(3)),
            log_bloom: FinalChainLogBloom::ZERO,
            gas_used: FinalChainGas::ZERO,
            total_reward: DposTokenAmount::zero(),
        })
        .into_vec()
    }

    fn pairing(tag: u8) -> Vec<u8> {
        let mut out = RlpStream::new_list(4);
        out.append(&1_u32)
            .append(&[tag; 32].as_slice())
            .append(&[0x33; 32].as_slice())
            .append(&[0x44; 32].as_slice());
        out.out().to_vec()
    }

    fn ready(result: Result<NativePredecessorCapture<'_>>) -> PreparedNativePredecessor<'_> {
        match result {
            Ok(NativePredecessorCapture::Ready(value)) => value,
            Ok(NativePredecessorCapture::Unavailable(reason)) => {
                panic!("capture unavailable: {reason:?}")
            }
            Err(error) => panic!("capture error: {error:#}"),
        }
    }

    fn unavailable(result: Result<NativePredecessorCapture<'_>>) -> NativePredecessorUnavailable {
        match result {
            Ok(NativePredecessorCapture::Unavailable(reason)) => reason,
            Ok(NativePredecessorCapture::Ready(_)) => panic!("unexpected ready capture"),
            Err(error) => panic!("unexpected local error: {error:#}"),
        }
    }

    fn rows(store: &Storage) -> Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)> {
        Column::all()
            .iter()
            .map(|column| {
                (
                    column.name().to_owned(),
                    store
                        .iter(*column)
                        .map(|entry| {
                            let (key, value) = entry.unwrap();
                            (key.to_vec(), value.to_vec())
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn replace_fields(raw: &[u8], count: usize, replacements: &[(usize, Vec<u8>)]) -> Vec<u8> {
        let list = Rlp::new(raw);
        let mut out = RlpStream::new_list(count);
        for index in 0..count {
            if let Some((_, bytes)) = replacements.iter().find(|(field, _)| *field == index) {
                out.append_raw(bytes, 1);
            } else {
                out.append_raw(list.at(index).unwrap().as_raw(), 1);
            }
        }
        out.out().to_vec()
    }

    fn legacy_five(raw: &[u8]) -> Vec<u8> {
        let list = Rlp::new(raw);
        let mut out = RlpStream::new_list(5);
        for index in [0, 1, 2, 4, 5] {
            out.append_raw(list.at(index).unwrap().as_raw(), 1);
        }
        out.out().to_vec()
    }

    fn assert_seed(value: &PreparedNativePredecessor<'_>, seed: &Seed) {
        assert_eq!(value.head_raw, seed.parent.to_le_bytes());
        assert_eq!(value.current.period.as_u64(), seed.parent);
        assert_eq!(value.delayed.period.as_u64(), seed.delayed);
        assert_eq!(value.current.header_raw, seed.current_header);
        assert_eq!(value.delayed.header_raw, seed.delayed_header);
        assert_eq!(value.current.native_raw, seed.current_native);
        assert_eq!(value.delayed.native_raw, seed.delayed_native);
        assert_eq!(
            value.current.native,
            decode_dpos_snapshot_rlp(&seed.current_native).unwrap()
        );
        assert_eq!(
            value.delayed.native,
            decode_dpos_snapshot_rlp(&seed.delayed_native).unwrap()
        );
        assert_eq!(value.pairing_raw, seed.pairing);
        assert_eq!(value.retained_source_bytes, seed.bytes());
    }

    #[test]
    fn native_predecessor_selects_current_delayed_and_period_rules() {
        for (pending, delay, boundary) in
            [(2, 0, 3), (3, 1, 3), (3, 99, 3), (u64::MAX, 1, u64::MAX)]
        {
            let mut fixture = Fixture::new(delay, false);
            fixture.owner = Some(
                fixture
                    .owner
                    .take()
                    .unwrap()
                    .with_correction_activation(Some(FinalChainBlockNumber::new(boundary))),
            );
            let seed = fixture.records(pending, 4, 5);
            install(fixture.storage(), &seed);
            let before = rows(fixture.storage());
            let value = ready(fixture.capture(pending, MAX_SOURCE));
            assert_seed(&value, &seed);
            assert!(std::ptr::eq(value.owner, fixture.owner()));
            assert_eq!(value.pending_period.as_u64(), pending);
            assert_eq!(
                value.correction_stage,
                if pending >= boundary {
                    CorrectionStage::SlashingHardError
                } else {
                    CorrectionStage::Historical
                }
            );
            assert_eq!(Arc::ptr_eq(&value.current, &value.delayed), delay == 0);
            assert_eq!(value.current.header.state_root, H256::repeat_byte(5));
            assert_eq!(rows(fixture.storage()), before);
        }
        let fixture = Fixture::new(1, false);
        write(
            fixture.storage(),
            vec![(
                Column::FinalChainMeta,
                FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                Some(vec![1]),
            )],
        );
        assert_eq!(
            unavailable(fixture.capture(0, MAX_SOURCE)),
            NativePredecessorUnavailable::PendingGenesis
        );
    }

    #[test]
    fn native_predecessor_snapshot_is_one_sequence() {
        let fixture = Fixture::new(1, false);
        let old = fixture.records(3, 7, 8);
        install(fixture.storage(), &old);
        let reader = NativeSourceReader::new(
            fixture.owner(),
            NativePredecessorLimits {
                max_retained_source_bytes: MAX_SOURCE,
            },
        );
        assert_eq!(
            reader
                .snapshot
                .get(
                    Column::FinalChainMeta,
                    &FinalChain::DB_META_LAST_NUMBER.to_le_bytes()
                )
                .unwrap(),
            Some(2_u64.to_le_bytes().to_vec())
        );
        let new = fixture.records(4, 9, 10);
        let storage = Arc::clone(fixture.storage.as_ref().unwrap());
        let writer_values = new.clone();
        std::thread::spawn(move || install(&storage, &writer_values))
            .join()
            .unwrap();
        let committed = rows(fixture.storage());
        let captured = ready(reader.capture(FinalChainBlockNumber::new(3)));
        assert_seed(&captured, &old);
        let fresh = ready(fixture.capture(4, MAX_SOURCE));
        assert_seed(&fresh, &new);
        assert_seed(&captured, &old);
        assert_eq!(rows(fixture.storage()), committed);
    }

    #[test]
    fn native_predecessor_missing_and_pending_are_unavailable() {
        for case in 0..9 {
            let fixture = Fixture::new(1, false);
            let seed = fixture.records(3, 1, 2);
            install(fixture.storage(), &seed);
            let (column, key, value, expected) = match case {
                0 => (
                    Column::FinalChainMeta,
                    FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                    None,
                    NativePredecessorUnavailable::MissingHead,
                ),
                1 => (
                    Column::FinalChainMeta,
                    FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                    Some(9_u64.to_le_bytes().to_vec()),
                    NativePredecessorUnavailable::HeadMismatch {
                        expected: 2.into(),
                        actual: 9.into(),
                    },
                ),
                2 => (
                    Column::FinalChainMeta,
                    b"rustaxa:concrete_state_pairing:v1".to_vec(),
                    None,
                    NativePredecessorUnavailable::MissingPairing,
                ),
                3 => (
                    Column::FinalChainBlkByNumber,
                    2_u64.to_le_bytes().to_vec(),
                    None,
                    NativePredecessorUnavailable::MissingHeader(2.into()),
                ),
                4 => (
                    Column::FinalChainMeta,
                    dpos_key(2),
                    None,
                    NativePredecessorUnavailable::MissingNativeSnapshot(2.into()),
                ),
                5 => (
                    Column::FinalChainBlkByNumber,
                    1_u64.to_le_bytes().to_vec(),
                    None,
                    NativePredecessorUnavailable::MissingHeader(1.into()),
                ),
                6 => (
                    Column::FinalChainMeta,
                    dpos_key(1),
                    None,
                    NativePredecessorUnavailable::MissingNativeSnapshot(1.into()),
                ),
                7 => (
                    Column::FinalChainMeta,
                    b"rustaxa:external_evm_pending_publication".to_vec(),
                    Some(vec![]),
                    NativePredecessorUnavailable::PublicationPending,
                ),
                _ => (
                    Column::FinalChainMeta,
                    b"rustaxa:external_evm_pending_publication".to_vec(),
                    Some(vec![1]),
                    NativePredecessorUnavailable::PublicationPending,
                ),
            };
            write(fixture.storage(), vec![(column, key, value)]);
            let before = rows(fixture.storage());
            assert_eq!(unavailable(fixture.capture(3, MAX_SOURCE)), expected);
            assert_eq!(rows(fixture.storage()), before);
        }
        let fixture = Fixture::new(99, false);
        let seed = fixture.records(3, 1, 2);
        install(fixture.storage(), &seed);
        write(
            fixture.storage(),
            vec![(Column::FinalChainMeta, dpos_key(0), None)],
        );
        assert_eq!(
            unavailable(fixture.capture(3, MAX_SOURCE)),
            NativePredecessorUnavailable::MissingNativeSnapshot(FinalChainBlockNumber::GENESIS)
        );
    }

    #[test]
    fn native_predecessor_malformed_sources_are_local_errors() {
        for case in 0..12 {
            let fixture = Fixture::new(1, false);
            let seed = fixture.records(3, 1, 2);
            install(fixture.storage(), &seed);
            let (column, key, bytes) = match case {
                0 => (
                    Column::FinalChainMeta,
                    FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                    vec![],
                ),
                1 => (
                    Column::FinalChainMeta,
                    FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                    vec![0; 7],
                ),
                2 => (
                    Column::FinalChainMeta,
                    FinalChain::DB_META_LAST_NUMBER.to_le_bytes().to_vec(),
                    vec![0; 9],
                ),
                3 => (
                    Column::FinalChainBlkByNumber,
                    2_u64.to_le_bytes().to_vec(),
                    vec![],
                ),
                4 => (
                    Column::FinalChainBlkByNumber,
                    1_u64.to_le_bytes().to_vec(),
                    vec![0xc0],
                ),
                5 => (
                    Column::FinalChainBlkByNumber,
                    2_u64.to_le_bytes().to_vec(),
                    [seed.current_header.as_slice(), &[0x80]].concat(),
                ),
                6 => {
                    let list = Rlp::new(&seed.current_header);
                    let mut out = RlpStream::new_list(8);
                    for child in list.iter() {
                        out.append_raw(child.as_raw(), 1);
                    }
                    out.append(&0_u8);
                    (
                        Column::FinalChainBlkByNumber,
                        2_u64.to_le_bytes().to_vec(),
                        out.out().to_vec(),
                    )
                }
                7 => (Column::FinalChainMeta, dpos_key(2), vec![]),
                8 => (Column::FinalChainMeta, dpos_key(1), vec![0xc0]),
                9 => (
                    Column::FinalChainMeta,
                    dpos_key(2),
                    [seed.current_native.as_slice(), &[0x80]].concat(),
                ),
                10 => (Column::FinalChainMeta, dpos_key(2), vec![0xc1, 0xb8]),
                _ => (
                    Column::FinalChainMeta,
                    b"rustaxa:concrete_state_pairing:v1".to_vec(),
                    vec![],
                ),
            };
            write(fixture.storage(), vec![(column, key, Some(bytes))]);
            let before = rows(fixture.storage());
            assert!(
                fixture.capture(3, MAX_SOURCE).is_err(),
                "malformed source case {case}"
            );
            assert_eq!(rows(fixture.storage()), before);
        }
    }

    #[test]
    fn native_predecessor_legacy_and_complete_layout() {
        for schema in [5, 6, 7, 9, 11, 14, 15, 17, 20, 21, 22, 23, 24, 25, 26] {
            let fixture = Fixture::new(0, false);
            let mut seed = fixture.records(3, 1, 1);
            let raw = if schema == 5 {
                legacy_five(&seed.current_native)
            } else if schema == 25 {
                let mut native = fixture.snapshot(1);
                native.validator_row_authority = ValidatorRowAuthority::CompatibilityOnly;
                native
                    .retained_undelegation_counts
                    .insert(VALIDATOR, StoredUndelegationCounter(1));
                encode_dpos_snapshot_rlp(&native).unwrap()
            } else {
                replace_fields(&seed.current_native, schema, &[])
            };
            seed.current_native = raw.clone();
            seed.delayed_native = raw.clone();
            install(fixture.storage(), &seed);
            let value = ready(fixture.capture(3, MAX_SOURCE));
            assert_eq!(value.current.native_raw, raw);
            assert_eq!(
                matches!(
                    value.current.native.validator_row_authority,
                    ValidatorRowAuthority::Complete(_)
                ),
                schema == 26
            );
            if schema <= 6 {
                assert!(!value.current.native.delegation_ledger_history_complete);
            }
            if schema < 22 {
                assert!(
                    !value
                        .current
                        .native
                        .redelegate_same_validator_history_complete
                );
            }
            if schema < 24 {
                assert_ne!(
                    encode_dpos_snapshot_rlp(&value.current.native).unwrap(),
                    raw
                );
            }
        }
        let fixture = Fixture::new(0, true);
        let mut seed = fixture.records(3, 1, 1);
        seed.current_native = legacy_five(&seed.current_native);
        seed.delayed_native = seed.current_native.clone();
        install(fixture.storage(), &seed);
        assert_eq!(
            unavailable(fixture.capture(3, MAX_SOURCE)),
            NativePredecessorUnavailable::UnsupportedNativeLayout(2.into())
        );

        for scalar in [vec![], vec![0; 32]] {
            let fixture = Fixture::new(0, false);
            let mut seed = fixture.records(3, 1, 1);
            let mut rewards = RlpStream::new_list(1);
            rewards
                .begin_list(2)
                .append(&VALIDATOR.as_slice())
                .append(&scalar);
            let raw = legacy_five(&replace_fields(
                &seed.current_native,
                26,
                &[(1, rewards.out().to_vec())],
            ));
            seed.current_native = raw.clone();
            seed.delayed_native = raw.clone();
            install(fixture.storage(), &seed);
            let value = ready(fixture.capture(3, MAX_SOURCE));
            assert_eq!(value.current.native_raw, raw);
            assert_eq!(
                value.current.native.commission_rewards[&VALIDATOR].to_snapshot_bytes(),
                scalar
            );
        }
    }

    #[test]
    fn native_predecessor_validation_errors_precede_layout_refusal() {
        for case in 0..5 {
            let fixture = Fixture::new(1, true);
            let mut seed = fixture.records(3, 1, 2);
            let mut authority = RlpStream::new_list(3);
            authority.append(&1_u8).append(&true);
            authority.begin_list(if case == 0 || case == 4 { 0 } else { 1 });
            if case != 0 && case != 4 {
                authority
                    .begin_list(2)
                    .append(&VALIDATOR.as_slice())
                    .append(&0_u8);
            }
            let mut counts = RlpStream::new_list(1);
            counts
                .begin_list(2)
                .append(&(if case == 2 { [0x99; 20] } else { VALIDATOR }).as_slice())
                .append(&(if case == 1 { 0_u16 } else { 1_u16 }));
            let replacements = if case == 0 || case == 4 {
                vec![(25, authority.out().to_vec())]
            } else if case == 3 {
                vec![(24, counts.out().to_vec()), (25, authority.out().to_vec())]
            } else {
                vec![(24, counts.out().to_vec())]
            };
            if case == 4 {
                seed.current_native = legacy_five(&seed.current_native);
                seed.delayed_native = replace_fields(&seed.delayed_native, 26, &replacements);
            } else {
                seed.current_native = replace_fields(&seed.current_native, 26, &replacements);
            }
            install(fixture.storage(), &seed);
            let before = rows(fixture.storage());
            assert!(
                fixture.capture(3, MAX_SOURCE).is_err(),
                "invalid authority case {case}"
            );
            assert_eq!(rows(fixture.storage()), before);
        }

        let fixture = Fixture::new(0, false);
        let mut seed = fixture.records(3, 1, 1);
        let marker = rlp::encode(&vec![0_u8; 32]).to_vec();
        seed.current_native = replace_fields(
            &seed.current_native,
            26,
            &[
                (8, rlp::encode(&Vec::<u8>::new()).to_vec()),
                (9, marker),
                (10, rlp::encode(&0_u64).to_vec()),
            ],
        );
        seed.delayed_native = seed.current_native.clone();
        install(fixture.storage(), &seed);
        let value = ready(fixture.capture(3, MAX_SOURCE));
        assert_seed(&value, &seed);
        match &value.current.native.aspen_supply_state {
            super::super::AspenSupplyState::Migrated {
                total_supply,
                current_yield,
            } => {
                assert!(total_supply.to_snapshot_bytes().is_empty());
                assert_eq!(current_yield.as_u64(), 0);
            }
            _ => panic!("explicit schema26 zero migration was lost"),
        }
        drop(value);
        seed.current_native = replace_fields(
            &seed.current_native,
            26,
            &[(8, rlp::encode(&vec![1_u8]).to_vec())],
        );
        install(fixture.storage(), &seed);
        assert!(fixture.capture(3, MAX_SOURCE).is_err());
    }

    #[test]
    fn native_predecessor_pairing_record_is_read_only_and_structural() {
        let fixture = Fixture::new(0, false);
        let seed = fixture.records(3, 9, 9);
        install(fixture.storage(), &seed);
        let before = rows(fixture.storage());
        let value = ready(fixture.capture(3, MAX_SOURCE));
        assert_eq!(
            value.pairing,
            RecordedPairing {
                policy_version: 1,
                storage_genesis: [9; 32],
                database_id: [0x33; 32],
                chain_id: [0x44; 32],
            }
        );
        assert_eq!(value.pairing_raw, seed.pairing);
        assert_eq!(rows(fixture.storage()), before);
        drop(value);

        for case in 0..8 {
            let fixture = Fixture::new(0, false);
            let mut seed = fixture.records(3, 1, 1);
            let count = match case {
                0 => 3,
                1 => 5,
                _ => 4,
            };
            let mut out = RlpStream::new_list(count);
            if case == 6 {
                out.append_raw(&[0x81, 0x01], 1);
            } else {
                out.append(&(if case == 2 { 2_u32 } else { 1_u32 }));
            }
            out.append(&vec![1_u8; if case == 3 { 31 } else { 32 }]);
            out.append(&vec![if case == 5 { 0_u8 } else { 3_u8 }; 32]);
            if count >= 4 {
                out.append(&vec![4_u8; if case == 4 { 33 } else { 32 }]);
            }
            if count == 5 {
                out.append(&0_u8);
            }
            seed.pairing = out.out().to_vec();
            if case == 7 {
                seed.pairing.push(0x80);
            }
            install(fixture.storage(), &seed);
            let before = rows(fixture.storage());
            assert!(
                fixture.capture(3, MAX_SOURCE).is_err(),
                "pairing case {case}"
            );
            assert_eq!(rows(fixture.storage()), before);
        }
    }

    #[test]
    fn native_predecessor_retained_source_limits_are_exact() {
        for delay in [0, 1] {
            let fixture = Fixture::new(delay, false);
            let seed = fixture.records(3, 1, 2);
            install(fixture.storage(), &seed);
            let before = rows(fixture.storage());
            let value = ready(fixture.capture(3, seed.bytes()));
            assert_seed(&value, &seed);
            assert_eq!(Arc::ptr_eq(&value.current, &value.delayed), delay == 0);
            drop(value);
            assert_eq!(
                unavailable(fixture.capture(3, seed.bytes() - 1)),
                NativePredecessorUnavailable::RetainedSourceLimit
            );
            assert_eq!(
                unavailable(fixture.capture(3, 0)),
                NativePredecessorUnavailable::RetainedSourceLimit
            );
            assert_eq!(rows(fixture.storage()), before);
        }
        let mut budget = RetainedSourceBudget {
            used: usize::MAX - 1,
            limit: usize::MAX,
        };
        assert!(!budget.add(2));
        assert_eq!(budget.used, usize::MAX - 1);
        assert!(budget.add(1));
        assert!(!budget.add(1));
        assert_eq!(budget.used, usize::MAX);
    }

    #[test]
    fn native_predecessor_capture_preserves_complete_database_and_reopen() {
        let mut fixture = Fixture::new(1, false);
        let seed = fixture.records(3, 1, 2);
        install(fixture.storage(), &seed);
        write(
            fixture.storage(),
            vec![(
                Column::DefaultColumn,
                b"unrelated".to_vec(),
                Some(vec![9, 8, 7]),
            )],
        );
        let before = rows(fixture.storage());
        assert_eq!(before.len(), Column::all().len());
        assert!(before.iter().any(|(_, values)| values.is_empty()));
        let value = ready(fixture.capture(3, MAX_SOURCE));
        assert_seed(&value, &seed);
        assert_eq!(rows(fixture.storage()), before);
        drop(value);
        drop(fixture.owner.take());
        drop(fixture.storage.take());
        let mut config = fixture.config.clone();
        config.create_if_missing = false;
        config.create_missing_column_families = false;
        let reopened = Storage::new(config).unwrap();
        assert_eq!(rows(&reopened), before);
        assert_eq!(
            reopened
                .read_snapshot()
                .dpos_snapshot_raw(seed.parent)
                .unwrap(),
            Some(seed.current_native)
        );
        assert!(fixture.root.0.exists());
        drop(reopened);
    }

    #[test]
    fn native_predecessor_cannot_use_cache_or_advance_rewards() {
        let fixture = Fixture::new(1, false);
        let seed = fixture.records(3, 1, 2);
        install(fixture.storage(), &seed);
        {
            let different = fixture.snapshot(99);
            let mut cache = fixture.owner().dpos_snapshots.lock().unwrap();
            cache.insert(2.into(), different);
            cache.remove(&FinalChainBlockNumber::new(1));
        }
        let cache = fixture.owner().dpos_snapshots.lock().unwrap().clone();
        let config = fixture
            .owner()
            .native_execution_configuration()
            .canonical_bytes()
            .to_vec();
        let before = rows(fixture.storage());
        let value = ready(fixture.capture(3, MAX_SOURCE));
        assert_seed(&value, &seed);
        assert_ne!(value.current.native, cache[&FinalChainBlockNumber::new(2)]);
        assert_eq!(
            fixture.owner().dpos_snapshots.lock().unwrap().clone(),
            cache
        );
        assert_eq!(
            fixture
                .owner()
                .native_execution_configuration()
                .canonical_bytes(),
            config
        );
        assert_eq!(rows(fixture.storage()), before);
    }

    #[test]
    fn native_transition_source_bind_and_drop_preserve_live_state() {
        let mut fixture = Fixture::new(1, false);
        let seed = fixture.records(3, 1, 2);
        install(fixture.storage(), &seed);
        let owner = Arc::new(fixture.owner.take().unwrap());
        let scheduler = owner.reward_scheduler_runtime.lock().unwrap().clone();
        let cache = owner.dpos_snapshots.lock().unwrap().clone();
        let before = rows(fixture.storage());
        let captured = ready(capture(
            &owner,
            3.into(),
            NativePredecessorLimits {
                max_retained_source_bytes: MAX_SOURCE,
            },
        ));
        let binding = captured.into_owned_source(&owner).unwrap();
        binding.validate(&owner).unwrap();
        drop(binding);
        assert_eq!(*owner.reward_scheduler_runtime.lock().unwrap(), scheduler);
        assert_eq!(*owner.dpos_snapshots.lock().unwrap(), cache);
        assert_eq!(rows(fixture.storage()), before);
    }

    #[test]
    fn native_transition_source_rejects_foreign_owner_with_equal_rows() {
        let mut fixture = Fixture::new(1, false);
        let seed = fixture.records(3, 1, 2);
        install(fixture.storage(), &seed);
        let owner = Arc::new(fixture.owner.take().unwrap());
        let mut foreign = Fixture::new(1, false);
        install(foreign.storage(), &seed);
        let foreign_owner = Arc::new(foreign.owner.take().unwrap());
        let before = rows(fixture.storage());
        let foreign_before = rows(foreign.storage());
        let captured = ready(capture(
            &owner,
            3.into(),
            NativePredecessorLimits {
                max_retained_source_bytes: MAX_SOURCE,
            },
        ));
        assert!(captured.into_owned_source(&foreign_owner).is_err());
        let captured = ready(capture(
            &owner,
            3.into(),
            NativePredecessorLimits {
                max_retained_source_bytes: MAX_SOURCE,
            },
        ));
        let binding = captured.into_owned_source(&owner).unwrap();
        assert!(binding.validate(&foreign_owner).is_err());
        assert_eq!(rows(fixture.storage()), before);
        assert_eq!(rows(foreign.storage()), foreign_before);
    }

    #[test]
    fn native_transition_source_rejects_changed_current_or_delayed_bytes() {
        for change_delayed in [false, true] {
            let mut fixture = Fixture::new(1, false);
            let mut seed = fixture.records(3, 1, 2);
            install(fixture.storage(), &seed);
            let changed = encode_dpos_snapshot_rlp(&fixture.snapshot(9)).unwrap();
            let owner = Arc::new(fixture.owner.take().unwrap());
            let captured = ready(capture(
                &owner,
                3.into(),
                NativePredecessorLimits {
                    max_retained_source_bytes: MAX_SOURCE,
                },
            ));
            let binding = captured.into_owned_source(&owner).unwrap();
            if change_delayed {
                seed.delayed_native = changed;
            } else {
                seed.current_native = changed;
            }
            install(fixture.storage(), &seed);
            let before = rows(fixture.storage());
            let scheduler = owner.reward_scheduler_runtime.lock().unwrap().clone();
            let cache = owner.dpos_snapshots.lock().unwrap().clone();
            assert!(binding.validate(&owner).is_err());
            assert_eq!(*owner.reward_scheduler_runtime.lock().unwrap(), scheduler);
            assert_eq!(*owner.dpos_snapshots.lock().unwrap(), cache);
            assert_eq!(rows(fixture.storage()), before);
        }
    }
}
