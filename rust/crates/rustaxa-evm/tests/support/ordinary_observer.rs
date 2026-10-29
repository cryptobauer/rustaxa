//! Test-owned ordinary-slot observations. Not a durable preimage catalog.
//! PARTIAL: seed evidence and the physical-history comparator need reviewed wiring.
use anyhow::{Result, ensure};
use rustaxa_types::concrete_lifecycle::{ConcreteStorageSlot, FinalChainConcreteIdentity};
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStateIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

type SlotValues = BTreeMap<ConcreteStorageSlot, ConcreteRead<Vec<u8>>>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct OrdinarySnapshot {
    identity: ConcreteStateIdentity,
    values: SlotValues,
}

struct OrdinaryStage {
    prior: ConcreteStateIdentity,
    period: u64,
    slots: BTreeSet<ConcreteStorageSlot>,
    prepared: Option<OrdinarySnapshot>,
    marker: Option<Vec<u8>>,
    prior_generation: u64,
    concrete_committed: bool,
}

/// This value must be produced by the independently reviewed fresh-bootstrap
/// observation path, never by a Go expected catalog. That constructor is not
/// implemented in this artifact while seed evidence is under review.
pub(super) struct VerifiedFreshOrdinarySeed {
    owner: FinalChainConcreteIdentity,
    root_path: PathBuf,
    snapshot: OrdinarySnapshot,
}

pub(super) struct KnownOrdinaryStorageIdentities {
    owner: FinalChainConcreteIdentity,
    root_path: PathBuf,
    committed: OrdinarySnapshot,
    generation: u64,
    staged: Option<OrdinaryStage>,
    history: BTreeMap<u64, OrdinarySnapshot>,
}

pub(super) fn is_native_storage_domain(address: [u8; 20]) -> bool {
    // The exact two consensus domains are fixed, regardless of activation.
    address[..19] == [0; 19] && (address[19] == 0xfe || address[19] == 0xee)
}

impl KnownOrdinaryStorageIdentities {
    /// Caller must have just completed the asserted exclusive fresh bootstrap.
    pub(super) fn from_asserted_fresh_bootstrap(
        path: &Path,
        owner: FinalChainConcreteIdentity,
        identity: ConcreteStateIdentity,
        generation: u64,
    ) -> Result<Self> {
        ensure!(
            path.is_absolute() && std::fs::canonicalize(path)? == path,
            "ordinary seed path is not canonical"
        );
        let mut result = Self::from_verified_fresh_seed(VerifiedFreshOrdinarySeed {
            owner,
            root_path: path.to_path_buf(),
            snapshot: OrdinarySnapshot {
                identity,
                values: BTreeMap::new(),
            },
        })?;
        result.generation = generation;
        Ok(result)
    }

    pub(super) fn from_verified_fresh_seed(seed: VerifiedFreshOrdinarySeed) -> Result<Self> {
        ensure!(
            seed.root_path.is_absolute(),
            "ordinary observer path must be absolute"
        );
        ensure!(
            seed.snapshot.identity.state_root != [0; 32],
            "ordinary seed root missing"
        );
        ensure!(
            seed.snapshot
                .values
                .keys()
                .all(|s| !is_native_storage_domain(s.address)),
            "ordinary seed contains native-domain slot"
        );
        let mut history = BTreeMap::new();
        history.insert(
            seed.snapshot.identity.period.as_u64(),
            seed.snapshot.clone(),
        );
        Ok(Self {
            owner: seed.owner,
            root_path: seed.root_path,
            committed: seed.snapshot,
            generation: 0,
            staged: None,
            history,
        })
    }

    fn check_owner(&self, path: &Path, owner: FinalChainConcreteIdentity) -> Result<()> {
        ensure!(
            self.root_path == path && std::fs::canonicalize(path)? == path && self.owner == owner,
            "ordinary observer owner mismatch"
        );
        Ok(())
    }

    pub(super) fn begin(
        &mut self,
        path: &Path,
        owner: FinalChainConcreteIdentity,
        prior: ConcreteStateIdentity,
        period: u64,
        generation: u64,
    ) -> Result<()> {
        self.check_owner(path, owner)?;
        ensure!(self.staged.is_none(), "ordinary observer already staged");
        ensure!(
            self.generation == generation,
            "ordinary observer generation mismatch"
        );
        ensure!(
            self.committed.identity == prior,
            "ordinary observer prior mismatch"
        );
        ensure!(
            prior.period.as_u64().checked_add(1) == Some(period),
            "ordinary period mismatch"
        );
        self.staged = Some(OrdinaryStage {
            prior,
            period,
            slots: self.committed.values.keys().copied().collect(),
            prepared: None,
            marker: None,
            prior_generation: generation,
            concrete_committed: false,
        });
        Ok(())
    }

    pub(super) fn bind_marker(&mut self, marker: &[u8]) -> Result<()> {
        let stage = self
            .staged
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage missing"))?;
        ensure!(
            stage.marker.is_none() && !marker.is_empty(),
            "ordinary marker mismatch"
        );
        stage.marker = Some(marker.to_vec());
        Ok(())
    }

    pub(super) fn validate_pending_commit(
        &self,
        owner: FinalChainConcreteIdentity,
        prior: ConcreteStateIdentity,
        generation: u64,
        marker: &[u8],
        next: ConcreteStateIdentity,
    ) -> Result<()> {
        ensure!(
            self.owner == owner
                && self.committed.identity == prior
                && self.generation == generation,
            "ordinary pending owner mismatch"
        );
        let stage = self
            .staged
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage missing"))?;
        ensure!(
            !stage.concrete_committed
                && stage.prior == prior
                && stage.prior_generation == generation
                && stage.marker.as_deref() == Some(marker)
                && stage.prepared.as_ref().is_some_and(|s| s.identity == next),
            "ordinary pending commit mismatch"
        );
        Ok(())
    }

    pub(super) fn concrete_commit(
        &mut self,
        owner: FinalChainConcreteIdentity,
        marker: &[u8],
        actual: ConcreteStateIdentity,
        generation: u64,
    ) -> Result<()> {
        ensure!(self.owner == owner, "ordinary concrete owner mismatch");
        let stage = self
            .staged
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage missing"))?;
        ensure!(
            !stage.concrete_committed
                && stage.marker.as_deref() == Some(marker)
                && stage
                    .prepared
                    .as_ref()
                    .is_some_and(|s| s.identity == actual)
                && stage.prior_generation.checked_add(1) == Some(generation),
            "ordinary concrete commit mismatch"
        );
        stage.concrete_committed = true;
        Ok(())
    }

    /// Invoke only for actual surviving settled ordinary journal writes.
    pub(super) fn record_settled(&mut self, slot: ConcreteStorageSlot) -> Result<()> {
        ensure!(
            !is_native_storage_domain(slot.address),
            "ordinary write overlaps native domain"
        );
        let stage = self
            .staged
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage missing"))?;
        ensure!(stage.prepared.is_none(), "ordinary stage already prepared");
        stage.slots.insert(slot);
        Ok(())
    }

    /// `read` must read the actual final prepared root, including proved absence.
    pub(super) fn finish_prepared(
        &mut self,
        next: ConcreteStateIdentity,
        mut read: impl FnMut(ConcreteStorageSlot) -> Result<ConcreteRead<Vec<u8>>>,
    ) -> Result<()> {
        let stage = self
            .staged
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage missing"))?;
        ensure!(
            stage.prepared.is_none()
                && next.period.as_u64() == stage.period
                && next.state_root != [0; 32],
            "ordinary prepared identity mismatch"
        );
        let values = stage
            .slots
            .iter()
            .map(|slot| Ok((*slot, read(*slot)?)))
            .collect::<Result<SlotValues>>()?;
        stage.prepared = Some(OrdinarySnapshot {
            identity: next,
            values,
        });
        Ok(())
    }

    pub(super) fn staged_values(&self) -> Result<&SlotValues> {
        Ok(&self
            .staged
            .as_ref()
            .and_then(|s| s.prepared.as_ref())
            .ok_or_else(|| anyhow::anyhow!("ordinary prepared snapshot missing"))?
            .values)
    }

    pub(super) fn staged_slots(&self) -> Result<BTreeSet<ConcreteStorageSlot>> {
        let snapshot = self
            .staged
            .as_ref()
            .and_then(|s| s.prepared.as_ref())
            .ok_or_else(|| anyhow::anyhow!("ordinary prepared snapshot missing"))?;
        Ok(snapshot.values.keys().copied().collect())
    }

    /// Call only after actual concrete descriptor AND application publication
    /// have been classified as committed. Idempotent for an exact repeat.
    pub(super) fn promote(
        &mut self,
        path: &Path,
        owner: FinalChainConcreteIdentity,
        actual: ConcreteStateIdentity,
        generation: u64,
    ) -> Result<()> {
        self.check_owner(path, owner)?;
        if self.staged.is_none() {
            ensure!(
                self.committed.identity == actual && self.generation == generation,
                "ordinary repeated promotion mismatch"
            );
            return Ok(());
        }
        let stage = self.staged.as_ref().expect("checked staged");
        let next = stage
            .prepared
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage not prepared"))?;
        ensure!(
            stage.concrete_committed
                && stage.prior == self.committed.identity
                && next.identity == actual
                && stage.prior_generation.checked_add(1) == Some(generation),
            "ordinary committed descriptor mismatch"
        );
        ensure!(
            !self.history.contains_key(&actual.period.as_u64()),
            "ordinary history collision"
        );
        let next = next.clone();
        self.history.insert(actual.period.as_u64(), next.clone());
        self.committed = next;
        self.generation = stage.prior_generation + 1;
        self.staged = None;
        Ok(())
    }

    /// Call only after actual recovery proves the previous root is retained.
    pub(super) fn discard_from_owner(
        &mut self,
        owner: FinalChainConcreteIdentity,
        actual: ConcreteStateIdentity,
        generation: u64,
    ) -> Result<()> {
        let path = self.root_path.clone();
        self.discard(&path, owner, actual, generation)
    }

    pub(super) fn discard(
        &mut self,
        path: &Path,
        owner: FinalChainConcreteIdentity,
        actual: ConcreteStateIdentity,
        generation: u64,
    ) -> Result<()> {
        self.check_owner(path, owner)?;
        ensure!(
            self.committed.identity == actual && self.generation == generation,
            "ordinary discard descriptor mismatch"
        );
        let stage = self
            .staged
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ordinary stage missing"))?;
        ensure!(
            stage.prior == actual && !stage.concrete_committed,
            "ordinary discard prior mismatch"
        );
        self.staged = None;
        Ok(())
    }

    pub(super) fn verify_committed(
        &self,
        path: &Path,
        owner: FinalChainConcreteIdentity,
        actual: ConcreteStateIdentity,
        generation: u64,
    ) -> Result<()> {
        self.check_owner(path, owner)?;
        ensure!(
            self.staged.is_none()
                && self.committed.identity == actual
                && self.generation == generation,
            "ordinary reconciled state mismatch"
        );
        Ok(())
    }

    pub(super) fn historical(
        &self,
        path: &Path,
        owner: FinalChainConcreteIdentity,
        selected: ConcreteStateIdentity,
    ) -> Result<&SlotValues> {
        self.check_owner(path, owner)?;
        let snapshot = self
            .history
            .get(&selected.period.as_u64())
            .ok_or_else(|| anyhow::anyhow!("ordinary historical snapshot missing"))?;
        ensure!(
            snapshot.identity == selected,
            "ordinary historical root mismatch"
        );
        Ok(&snapshot.values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic identities exercise the in-memory state machine only; they do
    // not assert that a database, trie root or history exists.
    #[test]
    fn ordinary_stage_discard_promote_repeat_and_wrong_identity() -> Result<()> {
        let base = std::env::current_dir()?.canonicalize()?;
        let path = base.as_path();
        let owner = FinalChainConcreteIdentity {
            policy_version: 1,
            database_id: [1; 32],
            chain_id: [2; 32],
        };
        let prior = ConcreteStateIdentity {
            period: 10_u64.into(),
            state_root: [3; 32],
        };
        let next = ConcreteStateIdentity {
            period: 11_u64.into(),
            state_root: [4; 32],
        };
        let slot = ConcreteStorageSlot {
            address: [5; 20],
            key: [6; 32],
        };
        let mut observer =
            KnownOrdinaryStorageIdentities::from_asserted_fresh_bootstrap(path, owner, prior, 7)?;
        observer.begin(path, owner, prior, 11, 7)?;
        observer.bind_marker(b"first")?;
        observer.record_settled(slot)?;
        observer.finish_prepared(next, |_| Ok(ConcreteRead::Tombstone))?;
        assert!(observer.promote(path, owner, next, 8).is_err());
        assert!(observer.discard(path, owner, next, 8).is_err());
        observer.discard(path, owner, prior, 7)?;
        observer.begin(path, owner, prior, 11, 7)?;
        observer.bind_marker(b"retry")?;
        observer.record_settled(slot)?;
        observer.finish_prepared(next, |_| Ok(ConcreteRead::Tombstone))?;
        assert!(observer.concrete_commit(owner, b"wrong", next, 8).is_err());
        observer.concrete_commit(owner, b"retry", next, 8)?;
        assert!(observer.discard(path, owner, prior, 7).is_err());
        observer.promote(path, owner, next, 8)?;
        observer.promote(path, owner, next, 8)?;
        observer.verify_committed(path, owner, next, 8)?;
        assert_eq!(
            observer.historical(path, owner, next)?.get(&slot),
            Some(&ConcreteRead::Tombstone)
        );
        assert!(
            observer
                .historical(
                    path,
                    owner,
                    ConcreteStateIdentity {
                        state_root: [9; 32],
                        ..next
                    }
                )
                .is_err()
        );
        assert!(
            observer
                .begin(Path::new("/wrong-owner"), owner, next, 12, 8)
                .is_err()
        );
        Ok(())
    }
}
