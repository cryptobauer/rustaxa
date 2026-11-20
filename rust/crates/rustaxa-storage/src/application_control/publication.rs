//! Exact logical primary writes for one certified, paired publication.
use super::*;
use crate::{
    CertifiedConcreteCommit, ConcreteLifecycleObservation, ConcreteStateLifecycle,
    PreparedConcreteTransition,
};
use crate::{Column, StorageWriteBatch, StoredFinalizedRewardVoteCursor};
use anyhow::{Result, ensure};
use rlp::{Rlp, RlpStream};
use rocksdb::WriteBatchIteratorCf;
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::codec::rlp::concrete_lifecycle::{
    concrete_storage_slot_catalog_hash, decode_concrete_execution_marker,
    decode_concrete_state_provenance, decode_concrete_storage_catalog,
    encode_concrete_execution_marker, encode_concrete_state_provenance,
    encode_concrete_storage_catalog,
};
use rustaxa_types::concrete_state::ConcreteStateIdentity;
use std::collections::{BTreeMap, HashMap};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ROWS: usize = 262_144;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Row {
    column: Column,
    key: Vec<u8>,
    prior: Option<Vec<u8>>,
    next: Option<Vec<u8>>,
}

/// A bounded batch with actual predecessor values. It grants no consensus authority.
pub struct CertifiedPrimaryPlan {
    rows: Vec<Row>,
    reward_reset: Option<(Arc<()>, u64)>,
}

struct Collect {
    names: HashMap<u32, Column>,
    rows: BTreeMap<(String, Vec<u8>), (Column, Option<Vec<u8>>)>,
    count: usize,
    bytes: usize,
    failed: bool,
}
impl Collect {
    fn put(&mut self, id: u32, key: &[u8], value: Option<&[u8]>) {
        self.count = self.count.saturating_add(1);
        self.bytes = self
            .bytes
            .saturating_add(key.len())
            .saturating_add(value.map_or(0, <[u8]>::len));
        let Some(column) = self.names.get(&id).copied() else {
            self.failed = true;
            return;
        };
        if self.failed
            || self.count > MAX_ROWS
            || self.bytes > MAX_BYTES
            || (column == Column::Status && key == PBFT_CERTIFIED_INTENT_KEY)
        {
            self.failed = true;
            return;
        }
        self.rows.insert(
            (column.name().into(), key.to_vec()),
            (column, value.map(Vec::from)),
        );
    }
}
impl WriteBatchIteratorCf for Collect {
    fn put_cf(&mut self, id: u32, key: &[u8], value: &[u8]) {
        self.put(id, key, Some(value));
    }
    fn delete_cf(&mut self, id: u32, key: &[u8]) {
        self.put(id, key, None);
    }
    fn merge_cf(&mut self, _: u32, _: &[u8], _: &[u8]) {
        self.failed = true;
    }
}

fn append_optional(out: &mut RlpStream, value: &Option<Vec<u8>>) {
    out.begin_list(usize::from(value.is_some()));
    if let Some(value) = value {
        out.append(value);
    }
}
fn optional(raw: Rlp<'_>) -> Result<Option<Vec<u8>>> {
    ensure!(
        raw.is_list() && raw.item_count()? <= 1,
        "CERTIFIED_PRIMARY_OPTION"
    );
    Ok(if raw.item_count()? == 0 {
        None
    } else {
        Some(raw.val_at(0)?)
    })
}

impl CertifiedPrimaryPlan {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = RlpStream::new_list(self.rows.len());
        for row in &self.rows {
            out.begin_list(4)
                .append(&row.column.name())
                .append(&row.key);
            append_optional(&mut out, &row.prior);
            append_optional(&mut out, &row.next);
        }
        out.out().to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "CERTIFIED_PRIMARY_BYTES");
        let raw = Rlp::new(bytes);
        ensure!(
            raw.is_list() && raw.item_count()? <= MAX_ROWS,
            "CERTIFIED_PRIMARY_ROWS"
        );
        let mut rows = Vec::new();
        let mut previous = None;
        for raw in raw.iter() {
            ensure!(raw.item_count()? == 4, "CERTIFIED_PRIMARY_ROW");
            let name: String = raw.val_at(0)?;
            let column = Column::all()
                .iter()
                .copied()
                .find(|c| c.name() == name)
                .ok_or_else(|| anyhow::anyhow!("CERTIFIED_PRIMARY_COLUMN"))?;
            let key: Vec<u8> = raw.val_at(1)?;
            ensure!(
                column != Column::Status || key != PBFT_CERTIFIED_INTENT_KEY,
                "CERTIFIED_PRIMARY_RESERVED_KEY"
            );
            let order = (name, key.clone());
            ensure!(
                previous.as_ref().is_none_or(|p| p < &order),
                "CERTIFIED_PRIMARY_ORDER"
            );
            previous = Some(order);
            rows.push(Row {
                column,
                key,
                prior: optional(raw.at(2)?)?,
                next: optional(raw.at(3)?)?,
            });
        }
        let plan = Self {
            rows,
            reward_reset: None,
        };
        ensure!(plan.encode() == bytes, "CERTIFIED_PRIMARY_NONCANONICAL");
        Ok(plan)
    }

    pub(crate) fn batch(&self, storage: &Storage) -> Result<StorageWriteBatch> {
        let mut batch = storage.create_write_batch();
        for row in &self.rows {
            let prior = crate::db::DbReader::get(storage, row.column, &row.key)?;
            ensure!(
                prior.as_ref().map(|p| p.as_ref()) == row.prior.as_deref(),
                "CERTIFIED_PRIMARY_PREDECESSOR_CHANGED"
            );
            match &row.next {
                Some(value) => storage.batch_put_raw(&mut batch, row.column, &row.key, value)?,
                None => storage.batch_delete_raw(&mut batch, row.column, &row.key)?,
            }
        }
        Ok(batch)
    }
}

impl Storage {
    /// Reads a bounded durable record before an application can recover it.
    pub fn pending_certified_intent(&self) -> Result<Option<CertifiedPublicationIntent>> {
        let raw = crate::db::DbReader::get(self, Column::Status, PBFT_CERTIFIED_INTENT_KEY)?;
        raw.map(|p| CertifiedPublicationIntent::decode(p.as_ref()))
            .transpose()
    }

    /// Captures last-write semantics and exact initial values without a write.
    pub fn prepare_certified_primary(
        &self,
        batch: &StorageWriteBatch,
    ) -> Result<CertifiedPrimaryPlan> {
        ensure!(
            batch.len() <= MAX_ROWS && batch.data().len() <= MAX_BYTES,
            "CERTIFIED_PRIMARY_BATCH_LIMIT"
        );
        let mut collector = Collect {
            names: HashMap::new(),
            rows: BTreeMap::new(),
            count: 0,
            bytes: 0,
            failed: false,
        };
        struct Id(Option<u32>);
        impl WriteBatchIteratorCf for Id {
            fn put_cf(&mut self, id: u32, _: &[u8], _: &[u8]) {
                self.0 = Some(id);
            }
            fn delete_cf(&mut self, _: u32, _: &[u8]) {}
            fn merge_cf(&mut self, _: u32, _: &[u8], _: &[u8]) {}
        }
        for column in Column::all() {
            let mut probe = self.create_write_batch();
            self.batch_put_raw(&mut probe, *column, b"", b"")?;
            let mut id = Id(None);
            probe.iterate_cf(&mut id);
            ensure!(
                collector
                    .names
                    .insert(
                        id.0.ok_or_else(|| anyhow::anyhow!("CERTIFIED_PRIMARY_CF_ID"))?,
                        *column
                    )
                    .is_none(),
                "CERTIFIED_PRIMARY_DUPLICATE_CF"
            );
        }
        batch.iterate_cf(&mut collector);
        // Range deletions and unsupported records do not have iterator callbacks.
        ensure!(
            !collector.failed && collector.count == batch.len(),
            "CERTIFIED_PRIMARY_UNSUPPORTED_OPERATION"
        );
        let mut encoded_bound = 16_usize;
        let mut rows = Vec::new();
        for ((name, key), (column, next)) in collector.rows {
            let prior = crate::db::DbReader::get(self, column, &key)?;
            encoded_bound = encoded_bound
                .checked_add(
                    128 + name.len()
                        + key.len()
                        + prior.as_ref().map_or(0, |p| p.as_ref().len())
                        + next.as_ref().map_or(0, Vec::len),
                )
                .ok_or_else(|| anyhow::anyhow!("CERTIFIED_PRIMARY_SIZE_OVERFLOW"))?;
            ensure!(
                encoded_bound <= MAX_BYTES,
                "CERTIFIED_PRIMARY_PRIOR_VALUE_LIMIT"
            );
            rows.push(Row {
                prior: prior.map(|p| p.as_ref().to_vec()),
                column,
                key,
                next,
            });
        }
        let plan = CertifiedPrimaryPlan {
            rows,
            reward_reset: None,
        };
        ensure!(
            plan.encode().len() <= MAX_BYTES,
            "CERTIFIED_PRIMARY_ENCODED_LIMIT"
        );
        Ok(plan)
    }
}

impl ApplicationOwner {
    /// Binds the process-local reward reset to the exact certified primary batch.
    /// Recovery decodes the durable rows and restores the cursor from storage.
    #[doc(hidden)]
    pub fn prepare_certified_primary_with_reward_reset(
        &self,
        scope: &ApplicationReadScope<'_>,
        batch: &StorageWriteBatch,
        cursor: StoredFinalizedRewardVoteCursor,
    ) -> Result<CertifiedPrimaryPlan> {
        ensure!(
            scope.matches_owner(&self.key),
            "CERTIFIED_REWARD_RESET_SCOPE"
        );
        let storage = self.storage.as_ref();
        let mut plan = storage.prepare_certified_primary(batch)?;
        let mut reset = storage.create_write_batch();
        storage
            .pbft()
            .replace_cert_votes_in_batch(&mut reset, &cursor.votes_bundle_rlp)?;
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(&mut reset, cursor)?;
        for hash in storage.pbft().extra_reward_vote_hashes()? {
            storage.batch_delete_raw(&mut reset, Column::ExtraRewardVotes, hash.as_bytes())?;
        }
        let expected = storage.prepare_certified_primary(&reset)?;
        let actual = plan.rows.iter().filter(|row| {
            matches!(
                row.column,
                Column::ExtraRewardVotes | Column::FinalizedRewardVoteCursor
            ) || (row.column == Column::LatestRoundTwoTPlusOneVotes
                && expected
                    .rows
                    .iter()
                    .any(|other| other.column == row.column && other.key == row.key))
        });
        ensure!(
            actual.eq(expected.rows.iter()),
            "CERTIFIED_REWARD_RESET_BATCH_MISMATCH"
        );
        plan.reward_reset = Some((Arc::clone(&self.key.issuer), scope.generation()));
        Ok(plan)
    }
}

/// Durable work written only after the application checks the full transition.
/// This record is not independent certificate or execution authority.
pub struct CertifiedPublicationIntent {
    binding: Vec<u8>,
    prior: ConcreteLifecycleObservation,
    marker: Vec<u8>,
    provenance: Vec<u8>,
    catalog: Vec<u8>,
    primary: CertifiedPrimaryPlan,
}

impl CertifiedPublicationIntent {
    #[cfg(feature = "production_integration_test")]
    pub fn integration_expected_primary_image(
        &self,
        mut image: crate::integration_inspection::DatabaseImage,
    ) -> Result<crate::integration_inspection::DatabaseImage> {
        for row in &self.primary.rows {
            let (_, rows) = image
                .columns
                .iter_mut()
                .find(|(name, _)| name == row.column.name())
                .ok_or_else(|| anyhow::anyhow!("CERTIFIED_TEST_COLUMN_MISSING"))?;
            let mut values: BTreeMap<_, _> = std::mem::take(rows).into_iter().collect();
            ensure!(
                values.get(&row.key) == row.prior.as_ref(),
                "CERTIFIED_TEST_PRIOR_MISMATCH"
            );
            match &row.next {
                Some(value) => {
                    values.insert(row.key.clone(), value.clone());
                }
                None => {
                    values.remove(&row.key);
                }
            }
            *rows = values.into_iter().collect();
        }
        let (_, rows) = image
            .columns
            .iter_mut()
            .find(|(name, _)| name == Column::Status.name())
            .ok_or_else(|| anyhow::anyhow!("CERTIFIED_TEST_STATUS_MISSING"))?;
        rows.retain(|(key, _)| key != PBFT_CERTIFIED_INTENT_KEY);
        image.wal_files.clear();
        Ok(image)
    }

    pub fn binding(&self) -> &[u8] {
        &self.binding
    }
    pub fn prior(&self) -> &ConcreteLifecycleObservation {
        &self.prior
    }
    pub fn plan_hash(&self) -> Result<[u8; 32]> {
        Ok(decode_concrete_execution_marker(&self.marker)?.plan_hash)
    }
    pub fn next_identity(&self) -> Result<ConcreteStateIdentity> {
        let p = decode_concrete_state_provenance(&self.provenance)?;
        Ok(ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(p.committed_state.period),
            state_root: p.committed_state.root,
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = RlpStream::new_list(8);
        out.append(&1_u8)
            .append(&self.binding)
            .append(&self.prior.provenance_rlp)
            .append(&self.prior.catalog_rlp)
            .append(&self.marker)
            .append(&self.provenance)
            .append(&self.catalog)
            .append(&self.primary.encode());
        out.out().to_vec()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "CERTIFIED_INTENT_BYTES");
        let raw = Rlp::new(bytes);
        ensure!(
            raw.is_list() && raw.item_count()? == 8 && raw.val_at::<u8>(0)? == 1,
            "CERTIFIED_INTENT_VERSION"
        );
        let prior_raw: Vec<u8> = raw.val_at(2)?;
        let prior_catalog: Vec<u8> = raw.val_at(3)?;
        let p = decode_concrete_state_provenance(&prior_raw)?;
        let marker: Vec<u8> = raw.val_at(4)?;
        let m = decode_concrete_execution_marker(&marker)?;
        let provenance: Vec<u8> = raw.val_at(5)?;
        let next = decode_concrete_state_provenance(&provenance)?;
        let catalog: Vec<u8> = raw.val_at(6)?;
        let old_slots = decode_concrete_storage_catalog(&prior_catalog)?;
        let new_slots = decode_concrete_storage_catalog(&catalog)?;
        ensure!(
            encode_concrete_state_provenance(&p) == prior_raw
                && encode_concrete_execution_marker(&m) == marker
                && encode_concrete_state_provenance(&next) == provenance
                && p.catalog_hash == concrete_storage_slot_catalog_hash(old_slots.iter().copied())
                && next.catalog_hash
                    == concrete_storage_slot_catalog_hash(new_slots.iter().copied())
                && p.committed_state.root != [0; 32]
                && next.committed_state.root != [0; 32]
                && next.projection_hash != [0; 32],
            "CERTIFIED_INTENT_PROVENANCE"
        );
        ensure!(
            encode_concrete_storage_catalog(old_slots.iter().copied()) == prior_catalog
                && encode_concrete_storage_catalog(new_slots.iter().copied()) == catalog
                && old_slots.iter().all(|s| new_slots.binary_search(s).is_ok()),
            "CERTIFIED_INTENT_CATALOG"
        );
        ensure!(
            m.identity == p.identity
                && next.identity == p.identity
                && p.generation.checked_add(1) == Some(m.generation)
                && next.generation == m.generation
                && p.committed_state == m.prior_state
                && p.committed_state.period.checked_add(1) == Some(m.period)
                && next.committed_state.period == m.period
                && next.plan_hash == m.plan_hash
                && next.transactions_hash == m.transactions_hash
                && next.rewards_hash == m.rewards_hash,
            "CERTIFIED_INTENT_LINEAGE"
        );
        let intent = Self {
            binding: raw.val_at(1)?,
            prior: ConcreteLifecycleObservation {
                identity: p.identity,
                committed: ConcreteStateIdentity {
                    period: FinalChainBlockNumber::new(p.committed_state.period),
                    state_root: p.committed_state.root,
                },
                generation: p.generation,
                provenance_rlp: prior_raw,
                catalog_rlp: prior_catalog,
                pending_marker_rlp: Vec::new(),
            },
            marker,
            provenance,
            catalog,
            primary: CertifiedPrimaryPlan::decode(&raw.val_at::<Vec<u8>>(7)?)?,
        };
        ensure!(intent.encode() == bytes, "CERTIFIED_INTENT_NONCANONICAL");
        Ok(intent)
    }
}

/// A storage-only preparation bound to one live owner scope.
pub struct PreparedCertifiedState {
    issuer: std::sync::Arc<()>,
    generation: u64,
    concrete: CertifiedConcreteCommit,
    intent: CertifiedPublicationIntent,
}

impl ApplicationConcreteState {
    /// Validates both store plans without creating a durable marker.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_certified_state(
        &self,
        scope: &ApplicationReadScope<'_>,
        prepared: PreparedConcreteTransition,
        projection_hash: [u8; 32],
        catalog: Vec<u8>,
        primary: CertifiedPrimaryPlan,
        binding: Vec<u8>,
    ) -> Result<PreparedCertifiedState> {
        ensure!(
            scope.matches_owner(&self.owner.key) && scope.is_current(),
            "CERTIFIED_STATE_OWNER"
        );
        ensure!(
            primary
                .reward_reset
                .as_ref()
                .is_none_or(
                    |(issuer, generation)| Arc::ptr_eq(issuer, &self.owner.key.issuer)
                        && *generation == scope.generation()
                ),
            "CERTIFIED_REWARD_RESET_STALE_PLAN"
        );
        let guard = self
            .lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?;
        let lifecycle = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?;
        let concrete = lifecycle.prepare_certified_commit(prepared, projection_hash, catalog)?;
        let approval = concrete.approval();
        let intent = CertifiedPublicationIntent {
            binding,
            prior: concrete.prior().clone(),
            marker: approval.marker_rlp.clone(),
            provenance: approval.provenance_rlp.clone(),
            catalog: approval.catalog_rlp.clone(),
            primary,
        };
        let encoded = intent.encode();
        CertifiedPublicationIntent::decode(&encoded)?;
        drop(intent.primary.batch(&self.owner.storage)?);
        ensure!(
            self.owner
                .storage
                .get_raw(Column::Status, PBFT_CERTIFIED_INTENT_KEY)?
                .is_none(),
            "CERTIFIED_INTENT_ALREADY_PENDING"
        );
        Ok(PreparedCertifiedState {
            issuer: std::sync::Arc::clone(&self.owner.key.issuer),
            generation: scope.generation(),
            concrete,
            intent,
        })
    }

    /// Commits exact prepared state under the same gate. The application must
    /// first consume its private certificate authority and retire old clients.
    pub fn publish_certified_state(
        &self,
        scope: ApplicationReadScope<'_>,
        prepared: PreparedCertifiedState,
    ) -> Result<ConcreteStateIdentity> {
        self.publish_certified_state_retaining_scope(scope, prepared, 0)
            .map(|(identity, _, _)| identity)
    }

    #[cfg(feature = "production_integration_test")]
    pub fn integration_publish_certified_state_at(
        &self,
        scope: ApplicationReadScope<'_>,
        prepared: PreparedCertifiedState,
        stop: u8,
    ) -> Result<ConcreteStateIdentity> {
        self.publish_certified_state_retaining_scope(scope, prepared, stop)
            .map(|(identity, _, _)| identity)
    }

    /// Holds the same coordinator gate across success-only native state installation.
    #[doc(hidden)]
    pub fn publish_certified_state_retaining_scope<'owner>(
        &self,
        scope: ApplicationReadScope<'owner>,
        prepared: PreparedCertifiedState,
        stop: u8,
    ) -> Result<(
        ConcreteStateIdentity,
        ApplicationReadScope<'owner>,
        Option<u64>,
    )> {
        #[cfg(feature = "production_integration_test")]
        ensure!(stop <= 4, "CERTIFIED_TEST_STOP_POINT");
        #[cfg(not(feature = "production_integration_test"))]
        ensure!(stop == 0, "CERTIFIED_TEST_STOP_POINT");
        ensure!(
            scope.matches_owner(&self.owner.key)
                && scope.generation() == prepared.generation
                && std::sync::Arc::ptr_eq(&prepared.issuer, &self.owner.key.issuer),
            "CERTIFIED_STATE_STALE_OWNER"
        );
        let mut lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| anyhow::anyhow!("APPLICATION_CONCRETE_LOCK_POISONED"))?;
        ensure!(
            lifecycle
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?
                .observation()?
                == prepared.intent.prior,
            "CERTIFIED_STATE_CONCRETE_CHANGED"
        );
        let raw = prepared.intent.encode();
        let mut final_batch = prepared.intent.primary.batch(&self.owner.storage)?;
        let reward_reset = prepared
            .intent
            .primary
            .reward_reset
            .is_some()
            .then(|| self.owner.storage.lock_extra_reward_votes())
            .transpose()?;
        ensure!(
            reward_reset.is_none()
                || self.owner.storage.extra_reward_votes_reset_generation() < u64::MAX,
            "CERTIFIED_REWARD_RESET_GENERATION_EXHAUSTED"
        );
        self.owner.storage.batch_delete_raw(
            &mut final_batch,
            Column::Status,
            PBFT_CERTIFIED_INTENT_KEY,
        )?;
        let mut intent_batch = self.owner.storage.create_write_batch();
        self.owner.storage.batch_put_raw(
            &mut intent_batch,
            Column::Status,
            PBFT_CERTIFIED_INTENT_KEY,
            &raw,
        )?;
        let mut effects = scope.begin_owned_effects()?;
        let result = (|| {
            effects.commit_batch(&self.owner.storage, intent_batch, true)?;
            ensure!(stop != 1, "CERTIFIED_TEST_AFTER_INTENT");
            let mut owned = lifecycle
                .take()
                .ok_or_else(|| anyhow::anyhow!("APPLICATION_CONCRETE_REOPEN_REQUIRED"))?;
            owned.stage_execution(&prepared.intent.marker)?;
            ensure!(stop != 2, "CERTIFIED_TEST_AFTER_CONCRETE_MARKER");
            let (successor, committed) = owned.commit_certified_retaining(prepared.concrete)?;
            ensure!(
                committed.committed == prepared.intent.next_identity()?
                    && committed.provenance_rlp == prepared.intent.provenance
                    && committed.catalog_rlp == prepared.intent.catalog,
                "CERTIFIED_CONCRETE_COMMIT_MISMATCH"
            );
            ensure!(stop != 3, "CERTIFIED_TEST_AFTER_CONCRETE_COMMIT");
            effects.commit_batch(&self.owner.storage, final_batch, true)?;
            ensure!(stop != 4, "CERTIFIED_TEST_AFTER_PRIMARY_COMMIT");
            Ok((successor, committed.committed))
        })();
        match result {
            Ok((successor, committed)) => {
                let (scope, _) = effects.finish()?;
                let reset_generation = reward_reset.map(|reset| reset.record_committed_reset());
                *lifecycle = Some(successor);
                Ok((committed, scope, reset_generation))
            }
            Err(error) => {
                effects.poison();
                Err(error)
            }
        }
    }
}

impl ApplicationOwner {
    /// Resolves an exact durable intent before clients exist. The application
    /// must independently qualify its certificate, configuration and schedule.
    pub fn recover_certified_state(
        &self,
        read: ApplicationReadScope<'_>,
        lifecycle: &mut ConcreteStateLifecycle,
        intent: CertifiedPublicationIntent,
    ) -> Result<()> {
        ensure!(
            read.matches_owner(&self.key) && read.is_current(),
            "CERTIFIED_RECOVERY_OWNER_SCOPE"
        );
        let raw = intent.encode();
        let current = crate::db::DbReader::get(
            self.storage.as_ref(),
            Column::Status,
            PBFT_CERTIFIED_INTENT_KEY,
        )?;
        ensure!(
            current.as_ref().map(|p| p.as_ref()) == Some(raw.as_slice()),
            "CERTIFIED_RECOVERY_INTENT_CHANGED"
        );
        let observed = lifecycle.observation()?;
        let mut prior = observed.clone();
        prior.pending_marker_rlp.clear();
        let is_prior = prior == intent.prior;
        let is_next = observed.identity == intent.prior.identity
            && observed.committed == intent.next_identity()?
            && observed.provenance_rlp == intent.provenance
            && observed.catalog_rlp == intent.catalog
            && observed.pending_marker_rlp.is_empty();
        ensure!(is_prior || is_next, "CERTIFIED_RECOVERY_CONCRETE_CONFLICT");
        ensure!(
            observed.pending_marker_rlp.is_empty() || observed.pending_marker_rlp == intent.marker,
            "CERTIFIED_RECOVERY_MARKER_CONFLICT"
        );
        let mut batch = if is_next {
            intent.primary.batch(&self.storage)?
        } else {
            drop(intent.primary.batch(&self.storage)?);
            self.storage.create_write_batch()
        };
        self.storage
            .batch_delete_raw(&mut batch, Column::Status, PBFT_CERTIFIED_INTENT_KEY)?;
        let mut effects = read.begin_owned_effects()?;
        let result = (|| {
            if is_prior && !observed.pending_marker_rlp.is_empty() {
                effects.note_write_attempt();
                lifecycle.discard_execution(&intent.marker)?;
            }
            effects.commit_batch(&self.storage, batch, true)
        })();
        if result.is_err() {
            effects.poison();
        }
        if result.is_ok() {
            drop(effects.finish()?);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    fn store() -> Storage {
        let path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!(
                "certified-primary-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            true,
        );
        let mut config = Config::new(path);
        config.db_write_buffer_size = 2 * 1024 * 1024;
        config.max_total_wal_size = 64 * 1024 * 1024;
        Storage::new(config).unwrap()
    }
    type Rows = Vec<(Vec<u8>, Vec<u8>)>;
    fn all_rows(storage: &Storage) -> Vec<(&'static str, Rows)> {
        Column::all()
            .iter()
            .map(|c| {
                (
                    c.name(),
                    storage
                        .iter(*c)
                        .map(|r| r.map(|(k, v)| (k.into_vec(), v.into_vec())))
                        .collect::<Result<Vec<_>>>()
                        .unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn certified_reward_reset_requires_exact_cursor_bundle_and_deletions() {
        let storage = Arc::new(store());
        let owner = storage.claim_application_owner().unwrap();
        let hash = ethereum_types::H256::repeat_byte(7);
        storage
            .pbft()
            .write_extra_reward_vote(hash, b"vote")
            .unwrap();
        let before = all_rows(&storage);
        let scope = owner.try_read(std::time::Duration::from_secs(1)).unwrap();
        let cursor = StoredFinalizedRewardVoteCursor {
            period: 3,
            round: 1,
            step: 3,
            block_hash: ethereum_types::H256::repeat_byte(9),
            votes_bundle_rlp: vec![0xc1, 0x07],
        };
        let mut batch = storage.create_write_batch();
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(&mut batch, cursor.clone())
            .unwrap();
        storage
            .pbft()
            .replace_cert_votes_in_batch(&mut batch, &cursor.votes_bundle_rlp)
            .unwrap();
        assert!(
            owner
                .prepare_certified_primary_with_reward_reset(&scope, &batch, cursor.clone())
                .is_err()
        );
        storage
            .batch_delete_raw(&mut batch, Column::ExtraRewardVotes, hash.as_bytes())
            .unwrap();
        assert!(
            owner
                .prepare_certified_primary_with_reward_reset(&scope, &batch, cursor.clone())
                .unwrap()
                .reward_reset
                .is_some()
        );
        let mut wrong = cursor;
        wrong.round += 1;
        assert!(
            owner
                .prepare_certified_primary_with_reward_reset(&scope, &batch, wrong)
                .is_err()
        );
        assert_eq!(all_rows(&storage), before);
        assert_eq!(storage.extra_reward_votes_reset_generation(), 0);
    }

    #[test]
    fn primary_recovery_preserves_last_write_and_empty_values() {
        let storage = store();
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::Status, b"a", b"first")
            .unwrap();
        storage
            .batch_delete_raw(&mut batch, Column::Status, b"a")
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::Status, b"a", b"")
            .unwrap();
        storage
            .batch_put_raw(&mut batch, Column::Status, b"b", b"removed")
            .unwrap();
        storage
            .batch_delete_raw(&mut batch, Column::Status, b"b")
            .unwrap();
        let before = all_rows(&storage);
        let original = storage.prepare_certified_primary(&batch).unwrap();
        let recovered = CertifiedPrimaryPlan::decode(&original.encode()).unwrap();
        assert_eq!(all_rows(&storage), before);
        storage
            .commit_write_batch_with_sync(recovered.batch(&storage).unwrap(), true)
            .unwrap();
        assert_eq!(
            storage.get_raw(Column::Status, b"a").unwrap(),
            Some(Vec::new())
        );
        assert_eq!(storage.get_raw(Column::Status, b"b").unwrap(), None);
    }
    #[test]
    fn changed_primary_predecessor_refuses_without_writes() {
        let storage = store();
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(&mut batch, Column::Status, b"key", b"next")
            .unwrap();
        let plan = storage.prepare_certified_primary(&batch).unwrap();
        let mut changed = storage.create_write_batch();
        storage
            .batch_put_raw(&mut changed, Column::Status, b"key", b"other")
            .unwrap();
        storage.commit_write_batch_with_sync(changed, true).unwrap();
        let before = all_rows(&storage);
        assert!(plan.batch(&storage).is_err());
        assert_eq!(all_rows(&storage), before);
    }
    #[test]
    fn reserved_intent_is_not_an_ordinary_primary_row() {
        let storage = store();
        let before = all_rows(&storage);
        let mut batch = storage.create_write_batch();
        storage
            .batch_delete_raw(&mut batch, Column::Status, PBFT_CERTIFIED_INTENT_KEY)
            .unwrap();
        assert!(storage.prepare_certified_primary(&batch).is_err());
        assert_eq!(all_rows(&storage), before);
    }
    #[test]
    fn range_deletion_cannot_enter_the_recovery_codec() {
        let storage = store();
        let before = all_rows(&storage);
        let mut batch = storage.create_write_batch();
        storage
            .batch_delete_range_raw(&mut batch, Column::Transactions, b"a", b"z")
            .unwrap();
        assert!(storage.prepare_certified_primary(&batch).is_err());
        assert_eq!(all_rows(&storage), before);
    }
    #[test]
    fn oversized_prior_value_is_refused_before_retention() {
        let storage = store();
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::DefaultColumn,
                b"large",
                &vec![0x42; MAX_BYTES + 1],
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, true).unwrap();
        let mut next = storage.create_write_batch();
        storage
            .batch_delete_raw(&mut next, Column::DefaultColumn, b"large")
            .unwrap();
        let error = storage
            .prepare_certified_primary(&next)
            .err()
            .expect("oversized prior must fail");
        assert!(
            error
                .to_string()
                .contains("CERTIFIED_PRIMARY_PRIOR_VALUE_LIMIT")
        );
        let prior = crate::db::DbReader::get(&storage, Column::DefaultColumn, b"large")
            .unwrap()
            .unwrap();
        assert_eq!(prior.as_ref().len(), MAX_BYTES + 1);
    }
}
