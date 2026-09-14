//! Process-local lifecycle for the legacy slashing cleanup scheduler.
//!
//! Go keeps `nextCleanUpBlock` in the live slashing `Contract`; it is neither
//! part of the state trie nor recoverable from a committed root. This module
//! models that cache separately from consensus state. A staged native session
//! receives an immutable basis and returns a prepared successor. The successor
//! becomes live only after the exact external-state generation and FinalChain
//! publication to which it is bound have committed. Restart and verified
//! StateAPI discard/reopen reset the cache to zero, matching construction of a
//! new Go `StateTransition`.

use super::FinalChainBlockNumber;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REWARD_SCHEDULER_INSTANCE: AtomicU64 = AtomicU64::new(1);

fn scheduler_instance_successor(id: u64) -> Option<u64> {
    (id != 0).then(|| id.checked_add(1).unwrap_or(0))
}

fn next_scheduler_instance() -> u64 {
    NEXT_REWARD_SCHEDULER_INSTANCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            scheduler_instance_successor(id)
        })
        .unwrap_or_else(|_| panic!("reward scheduler instance identity exhausted"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinalChainRewardSchedulerEpochState {
    Unbound,
    JointStartup(u64),
    Live(u64),
}

/// Immutable process-local scheduler facts captured by one bound native session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalChainRewardSchedulerBasis {
    source_instance_id: u64,
    state_api_epoch: u64,
    expected_runtime_generation: u64,
    expected_parent: FinalChainBlockNumber,
    period: FinalChainBlockNumber,
    request_id: [u8; 32],
    next_cleanup_block: u64,
}

impl FinalChainRewardSchedulerBasis {
    /// Produces a private successor after exact EndBlock reconstruction.
    pub(crate) const fn successor(
        self,
        next_cleanup_block: u64,
    ) -> FinalChainPreparedRewardSchedulerSuccessor {
        FinalChainPreparedRewardSchedulerSuccessor {
            source_instance_id: self.source_instance_id,
            state_api_epoch: self.state_api_epoch,
            expected_runtime_generation: self.expected_runtime_generation,
            expected_parent: self.expected_parent,
            period: self.period,
            request_id: self.request_id,
            next_cleanup_block,
        }
    }

    /// Reproduces the Go cleanup scheduler decision from an authenticated
    /// committed jailed list and its per-validator expiry rows.
    pub(crate) fn plan_cleanup(
        self,
        jailed_validators: &[[u8; 20]],
        jail_blocks: &BTreeMap<[u8; 20], u64>,
    ) -> FinalChainRewardSchedulerCleanupPlan {
        let current = self.period.as_u64();
        if self.next_cleanup_block > current || jailed_validators.is_empty() {
            return FinalChainRewardSchedulerCleanupPlan {
                ran: false,
                retained_validators: jailed_validators.to_vec(),
                successor: self.successor(self.next_cleanup_block),
            };
        }

        let mut minimum = 0;
        let mut retained_validators = Vec::with_capacity(jailed_validators.len());
        for validator in jailed_validators {
            let jail_block = jail_blocks.get(validator).copied().unwrap_or_default();
            if jail_block > current {
                retained_validators.push(*validator);
            }
            if minimum == 0 || jail_block < minimum {
                minimum = jail_block;
            }
        }
        let next_cleanup_block = if minimum == 0 {
            self.next_cleanup_block
        } else {
            minimum
        };
        FinalChainRewardSchedulerCleanupPlan {
            ran: true,
            retained_validators,
            successor: self.successor(next_cleanup_block),
        }
    }
}

/// Exact Go cleanup decision and its unpublished scheduler successor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FinalChainRewardSchedulerCleanupPlan {
    pub(crate) ran: bool,
    pub(crate) retained_validators: Vec<[u8; 20]>,
    pub(crate) successor: FinalChainPreparedRewardSchedulerSuccessor,
}

/// Session-owned scheduler successor before concrete-publication binding.
///
/// Every field is private so an executor cannot inject a cleanup timer. The
/// value can only be derived from a basis issued by the owning FinalChain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalChainPreparedRewardSchedulerSuccessor {
    source_instance_id: u64,
    state_api_epoch: u64,
    expected_runtime_generation: u64,
    expected_parent: FinalChainBlockNumber,
    period: FinalChainBlockNumber,
    request_id: [u8; 32],
    next_cleanup_block: u64,
}

/// Concrete generation and publication identity attached before marker write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalChainRewardSchedulerPublicationBinding {
    pub(crate) request_id: [u8; 32],
    pub(crate) expected_parent: FinalChainBlockNumber,
    pub(crate) period: FinalChainBlockNumber,
    pub(crate) state_api_epoch: u64,
    pub(crate) concrete_database_id: [u8; 32],
    pub(crate) concrete_generation: u64,
    pub(crate) concrete_projection_hash: [u8; 32],
    pub(crate) publication_plan_id: [u8; 32],
}

/// Exact successor retained across same-process ambiguous marker/publication calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalChainArmedRewardSchedulerSuccessor {
    prepared: FinalChainPreparedRewardSchedulerSuccessor,
    binding: FinalChainRewardSchedulerPublicationBinding,
}

/// Result of installing a publication-bound successor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FinalChainRewardSchedulerInstall {
    /// This call advanced the live process-local scheduler.
    Installed,
    /// The same exact publication installed this successor earlier.
    AlreadyInstalled,
}

/// FinalChain-owned process-local cleanup scheduler.
///
/// `published_head` is an admission fence, not persisted scheduler state. A
/// head mismatch rejects new sessions after storage publication but before RAM
/// installation. `generation` is local to this scheduler and deliberately does
/// not share the rewards-stat runtime generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FinalChainRewardSchedulerRuntime {
    source_instance_id: u64,
    published_head: FinalChainBlockNumber,
    generation: u64,
    next_cleanup_block: u64,
    epoch: FinalChainRewardSchedulerEpochState,
    admitted: Option<FinalChainRewardSchedulerBasis>,
    pending: Option<FinalChainArmedRewardSchedulerSuccessor>,
    last_installed: Option<FinalChainArmedRewardSchedulerSuccessor>,
    observed_discard: Option<(u64, u64, FinalChainBlockNumber)>,
    verified_reopen: Option<(u64, u64, FinalChainBlockNumber)>,
}

impl FinalChainRewardSchedulerRuntime {
    /// Creates the scheduler paired with a newly constructed FinalChain.
    ///
    /// Timer zero matches a newly constructed Go Contract. The sole application
    /// factory must explicitly authorize the jointly constructed StateAPI epoch;
    /// an ordinary observation cannot bind this runtime.
    pub(crate) fn new(published_head: FinalChainBlockNumber) -> Self {
        Self {
            source_instance_id: next_scheduler_instance(),
            published_head,
            generation: 0,
            next_cleanup_block: 0,
            epoch: FinalChainRewardSchedulerEpochState::Unbound,
            admitted: None,
            pending: None,
            last_installed: None,
            observed_discard: None,
            verified_reopen: None,
        }
    }

    /// Grants the one joint-startup recovery privilege to an authenticated
    /// StateAPI epoch constructed by the same application factory.
    ///
    /// Merely constructing FinalChain, observing an epoch, or finding no
    /// pending publication does not grant this privilege. The sole application
    /// startup composition must call this after its first validated preflight.
    pub(crate) fn authorize_joint_startup(
        &mut self,
        state_api_epoch: u64,
        committed_head: FinalChainBlockNumber,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            state_api_epoch != 0,
            "FINAL_CHAIN_REWARD_SCHEDULER_EPOCH_ZERO"
        );
        anyhow::ensure!(
            self.epoch == FinalChainRewardSchedulerEpochState::Unbound,
            "FINAL_CHAIN_REWARD_SCHEDULER_JOINT_STARTUP_ALREADY_BOUND"
        );
        anyhow::ensure!(
            self.published_head == committed_head,
            "FINAL_CHAIN_REWARD_SCHEDULER_JOINT_STARTUP_HEAD_MISMATCH"
        );
        self.epoch = FinalChainRewardSchedulerEpochState::JointStartup(state_api_epoch);
        Ok(())
    }

    /// Closes the joint-startup privilege after successful startup recovery.
    pub(crate) fn complete_recovery(&mut self, state_api_epoch: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.observed_discard.is_none(),
            "FINAL_CHAIN_REWARD_SCHEDULER_REOPEN_OBSERVATION_INCOMPLETE"
        );
        match self.epoch {
            FinalChainRewardSchedulerEpochState::JointStartup(epoch)
                if epoch == state_api_epoch =>
            {
                self.epoch = FinalChainRewardSchedulerEpochState::Live(epoch);
                self.verified_reopen = None;
                Ok(())
            }
            FinalChainRewardSchedulerEpochState::Live(epoch) if epoch == state_api_epoch => {
                self.verified_reopen = None;
                Ok(())
            }
            _ => anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_JOINT_STARTUP_EPOCH_MISMATCH"),
        }
    }

    /// Validates an epoch for retrying the serialized startup recovery call.
    /// Joint-startup authority remains pending until a successful classification
    /// explicitly completes it; ordinary execution cannot use this state.
    pub(crate) fn validate_bound_recovery_epoch(&self, state_api_epoch: u64) -> anyhow::Result<()> {
        let matches_observed_discard = self
            .observed_discard
            .is_some_and(|(_, new_epoch, _)| new_epoch == state_api_epoch);
        anyhow::ensure!(
            state_api_epoch != 0
                && (matches!(
                    self.epoch,
                    FinalChainRewardSchedulerEpochState::JointStartup(epoch)
                        | FinalChainRewardSchedulerEpochState::Live(epoch)
                        if epoch == state_api_epoch
                ) || matches_observed_discard),
            "FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_EPOCH_MISMATCH"
        );
        Ok(())
    }

    /// Validates deletion of an uncommitted durable publication marker after
    /// StateAPI reports the exact prior descriptor and no staged marker.
    ///
    /// A same-epoch live StateAPI cannot prove what happened to Go's mutable
    /// cleanup timer and is rejected. Safe deletion requires either the
    /// explicitly authorized joint-startup timer-zero fact or a previously
    /// verified discard/reopen that replaced the marker's old epoch.
    pub(crate) fn validate_uncommitted_clear(
        &self,
        marker_state_api_epoch: u64,
        observed_state_api_epoch: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            observed_state_api_epoch != 0,
            "FINAL_CHAIN_REWARD_SCHEDULER_UNCOMMITTED_CLEAR_EPOCH_ZERO"
        );
        let authorized = match self.epoch {
            FinalChainRewardSchedulerEpochState::JointStartup(epoch) => {
                epoch == observed_state_api_epoch
            }
            FinalChainRewardSchedulerEpochState::Live(epoch) => {
                epoch == observed_state_api_epoch
                    && self.verified_reopen.is_some_and(|(old, new, head)| {
                        new == observed_state_api_epoch
                            && head == self.published_head
                            && (marker_state_api_epoch == 0 || marker_state_api_epoch == old)
                    })
            }
            FinalChainRewardSchedulerEpochState::Unbound => false,
        };
        anyhow::ensure!(
            authorized,
            "FINAL_CHAIN_REWARD_SCHEDULER_UNCOMMITTED_CLEAR_UNPROVEN_RESET"
        );
        Ok(())
    }

    /// Validates the already-live owner epoch for ordinary execution.
    ///
    /// An unbound runtime can become live only through the explicitly named
    /// joint-startup operation. Ordinary preflight observations never prove
    /// that FinalChain and StateAPI were freshly constructed together.
    pub(crate) fn observe_live_epoch(&mut self, state_api_epoch: u64) -> anyhow::Result<()> {
        match self.epoch {
            FinalChainRewardSchedulerEpochState::Live(epoch) => {
                anyhow::ensure!(
                    state_api_epoch != 0 && epoch == state_api_epoch,
                    "FINAL_CHAIN_REWARD_SCHEDULER_EPOCH_MISMATCH"
                );
                Ok(())
            }
            FinalChainRewardSchedulerEpochState::JointStartup(_) => {
                anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_JOINT_STARTUP_INCOMPLETE")
            }
            FinalChainRewardSchedulerEpochState::Unbound => {
                anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_STARTUP_UNBOUND")
            }
        }
    }

    /// Validates the live StateAPI instance and captures one session basis.
    pub(crate) fn basis(
        &mut self,
        state_api_epoch: u64,
        expected_parent: FinalChainBlockNumber,
        period: FinalChainBlockNumber,
        request_id: [u8; 32],
    ) -> anyhow::Result<FinalChainRewardSchedulerBasis> {
        anyhow::ensure!(
            self.epoch == FinalChainRewardSchedulerEpochState::Live(state_api_epoch),
            "FINAL_CHAIN_REWARD_SCHEDULER_EPOCH_MISMATCH"
        );
        anyhow::ensure!(
            self.published_head == expected_parent,
            "FINAL_CHAIN_REWARD_SCHEDULER_HEAD_MISMATCH"
        );
        anyhow::ensure!(
            expected_parent.checked_next() == Some(period),
            "FINAL_CHAIN_REWARD_SCHEDULER_PERIOD_MISMATCH"
        );
        anyhow::ensure!(
            self.observed_discard.is_none() && self.verified_reopen.is_none(),
            "FINAL_CHAIN_REWARD_SCHEDULER_REOPEN_RECOVERY_INCOMPLETE"
        );
        let basis = FinalChainRewardSchedulerBasis {
            source_instance_id: self.source_instance_id,
            state_api_epoch,
            expected_runtime_generation: self.generation,
            expected_parent,
            period,
            request_id,
            next_cleanup_block: self.next_cleanup_block,
        };
        anyhow::ensure!(
            self.pending.is_none(),
            "FINAL_CHAIN_REWARD_SCHEDULER_PENDING_CONFLICT"
        );
        match self.admitted {
            Some(existing) if existing == basis => {}
            Some(_) => anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_ADMISSION_CONFLICT"),
            None => self.admitted = Some(basis),
        }
        Ok(basis)
    }

    /// Arms a prepared successor before the potentially ambiguous marker write.
    ///
    /// The caller must retain the pending value until durable facts classify an
    /// error. Installing it is a separate post-publication operation.
    pub(crate) fn arm(
        &mut self,
        prepared: FinalChainPreparedRewardSchedulerSuccessor,
        binding: FinalChainRewardSchedulerPublicationBinding,
    ) -> anyhow::Result<FinalChainArmedRewardSchedulerSuccessor> {
        self.validate_prepared(prepared)?;
        anyhow::ensure!(
            prepared.request_id == binding.request_id
                && prepared.expected_parent == binding.expected_parent
                && prepared.period == binding.period
                && prepared.state_api_epoch == binding.state_api_epoch,
            "FINAL_CHAIN_REWARD_SCHEDULER_PUBLICATION_BINDING_MISMATCH"
        );
        let armed = FinalChainArmedRewardSchedulerSuccessor { prepared, binding };
        match self.pending {
            Some(existing) if existing == armed => return Ok(existing),
            Some(_) => anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_PENDING_CONFLICT"),
            None => {}
        }
        anyhow::ensure!(
            self.admitted
                == Some(FinalChainRewardSchedulerBasis {
                    source_instance_id: prepared.source_instance_id,
                    state_api_epoch: prepared.state_api_epoch,
                    expected_runtime_generation: prepared.expected_runtime_generation,
                    expected_parent: prepared.expected_parent,
                    period: prepared.period,
                    request_id: prepared.request_id,
                    next_cleanup_block: self.next_cleanup_block,
                }),
            "FINAL_CHAIN_REWARD_SCHEDULER_ADMISSION_MISMATCH"
        );
        self.admitted = None;
        self.pending = Some(armed);
        Ok(armed)
    }

    /// Installs the exact armed successor after durable publication.
    pub(crate) fn install_applied(
        &mut self,
        binding: FinalChainRewardSchedulerPublicationBinding,
    ) -> anyhow::Result<FinalChainRewardSchedulerInstall> {
        if self.pending.is_none()
            && self
                .last_installed
                .is_some_and(|installed| installed.binding == binding)
        {
            return Ok(FinalChainRewardSchedulerInstall::AlreadyInstalled);
        }
        let armed = self
            .pending
            .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_REWARD_SCHEDULER_PENDING_MISSING"))?;
        anyhow::ensure!(
            armed.binding == binding,
            "FINAL_CHAIN_REWARD_SCHEDULER_PENDING_BINDING_MISMATCH"
        );
        self.validate_prepared(armed.prepared)?;
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_REWARD_SCHEDULER_GENERATION_OVERFLOW"))?;
        self.published_head = armed.prepared.period;
        self.next_cleanup_block = armed.prepared.next_cleanup_block;
        self.generation = next_generation;
        self.pending = None;
        self.last_installed = Some(armed);
        Ok(FinalChainRewardSchedulerInstall::Installed)
    }

    /// Validates that recovery has either a one-shot fresh-startup token or the
    /// exact armed candidate retained by this live process.
    pub(crate) fn validate_recovery_publication(
        &self,
        state_api_epoch: u64,
        binding: FinalChainRewardSchedulerPublicationBinding,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            state_api_epoch != 0 && binding.state_api_epoch == state_api_epoch,
            "FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_EPOCH_MISMATCH"
        );
        match self.epoch {
            FinalChainRewardSchedulerEpochState::JointStartup(epoch) => {
                anyhow::ensure!(
                    epoch == state_api_epoch,
                    "FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_EPOCH_MISMATCH"
                );
                anyhow::ensure!(
                    (self.published_head == binding.expected_parent
                        && binding.expected_parent.checked_next() == Some(binding.period))
                        || self.published_head == binding.period,
                    "FINAL_CHAIN_REWARD_SCHEDULER_JOINT_STARTUP_HEAD_MISMATCH"
                );
            }
            FinalChainRewardSchedulerEpochState::Live(epoch) => {
                anyhow::ensure!(
                    epoch == state_api_epoch,
                    "FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_EPOCH_MISMATCH"
                );
                let matches_pending = self
                    .pending
                    .is_some_and(|pending| pending.binding == binding);
                let matches_installed = self
                    .last_installed
                    .is_some_and(|installed| installed.binding == binding);
                anyhow::ensure!(
                    matches_pending || matches_installed,
                    "FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_PENDING_MISSING"
                );
            }
            FinalChainRewardSchedulerEpochState::Unbound => {
                anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_STARTUP_UNAUTHORIZED")
            }
        }
        Ok(())
    }

    /// Validates the exact armed runtime candidate before restoring the epoch
    /// omitted by the durable marker codec.
    pub(crate) fn validate_pending_publication(
        &self,
        binding: FinalChainRewardSchedulerPublicationBinding,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.epoch == FinalChainRewardSchedulerEpochState::Live(binding.state_api_epoch),
            "FINAL_CHAIN_REWARD_SCHEDULER_PENDING_EPOCH_MISMATCH"
        );
        anyhow::ensure!(
            self.pending
                .is_some_and(|pending| pending.binding == binding),
            "FINAL_CHAIN_REWARD_SCHEDULER_PENDING_BINDING_MISMATCH"
        );
        Ok(())
    }

    /// Installs a publication previously validated for startup or live recovery.
    pub(crate) fn install_recovered_publication(
        &mut self,
        state_api_epoch: u64,
        binding: FinalChainRewardSchedulerPublicationBinding,
    ) -> anyhow::Result<FinalChainRewardSchedulerInstall> {
        self.validate_recovery_publication(state_api_epoch, binding)?;
        if self.epoch == FinalChainRewardSchedulerEpochState::JointStartup(state_api_epoch) {
            let next_generation = self.generation.checked_add(1).ok_or_else(|| {
                anyhow::anyhow!("FINAL_CHAIN_REWARD_SCHEDULER_GENERATION_OVERFLOW")
            })?;
            let prepared = FinalChainPreparedRewardSchedulerSuccessor {
                source_instance_id: self.source_instance_id,
                state_api_epoch,
                expected_runtime_generation: self.generation,
                expected_parent: binding.expected_parent,
                period: binding.period,
                request_id: binding.request_id,
                next_cleanup_block: 0,
            };
            let installed = FinalChainArmedRewardSchedulerSuccessor { prepared, binding };
            self.epoch = FinalChainRewardSchedulerEpochState::Live(state_api_epoch);
            self.published_head = binding.period;
            self.next_cleanup_block = 0;
            self.generation = next_generation;
            self.last_installed = Some(installed);
            return Ok(FinalChainRewardSchedulerInstall::Installed);
        }
        self.install_applied(binding)
    }

    /// Retains a verified StateAPI discard transition before fallible reopen
    /// validation.
    ///
    /// The actual timer has already reset, so ordinary admission remains closed.
    /// A later recovery may accept only the exact replacement epoch and must
    /// validate its descriptor and provenance before applying the transition.
    pub(crate) fn record_verified_discard(
        &mut self,
        expected_old_epoch: u64,
        new_epoch: u64,
        prior_head: FinalChainBlockNumber,
    ) -> anyhow::Result<()> {
        if self.observed_discard == Some((expected_old_epoch, new_epoch, prior_head)) {
            return Ok(());
        }
        anyhow::ensure!(
            self.observed_discard.is_none(),
            "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_TRANSITION_CONFLICT"
        );
        match self.epoch {
            FinalChainRewardSchedulerEpochState::Unbound => {
                anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_STARTUP_UNBOUND")
            }
            FinalChainRewardSchedulerEpochState::JointStartup(epoch)
            | FinalChainRewardSchedulerEpochState::Live(epoch) => anyhow::ensure!(
                epoch == expected_old_epoch,
                "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_EPOCH_MISMATCH"
            ),
        }
        anyhow::ensure!(
            new_epoch != 0 && new_epoch != expected_old_epoch,
            "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_EPOCH_NOT_REPLACED"
        );
        anyhow::ensure!(
            self.published_head == prior_head,
            "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_HEAD_MISMATCH"
        );
        self.observed_discard = Some((expected_old_epoch, new_epoch, prior_head));
        Ok(())
    }

    /// Applies a retained discard only after the replacement StateAPI's exact
    /// committed head and provenance have been validated by the owner.
    pub(crate) fn complete_verified_reopen(
        &mut self,
        state_api_epoch: u64,
        prior_head: FinalChainBlockNumber,
    ) -> anyhow::Result<bool> {
        let Some((expected_old_epoch, new_epoch, expected_head)) = self.observed_discard else {
            return Ok(false);
        };
        anyhow::ensure!(
            state_api_epoch == new_epoch && prior_head == expected_head,
            "FINAL_CHAIN_REWARD_SCHEDULER_REOPEN_OBSERVATION_MISMATCH"
        );
        self.apply_verified_reopen(expected_old_epoch, new_epoch, prior_head)?;
        self.observed_discard = None;
        Ok(true)
    }

    pub(crate) fn has_observed_discard_for(&self, state_api_epoch: u64) -> bool {
        self.observed_discard
            .is_some_and(|(_, new_epoch, _)| new_epoch == state_api_epoch)
    }

    fn apply_verified_reopen(
        &mut self,
        expected_old_epoch: u64,
        new_epoch: u64,
        prior_head: FinalChainBlockNumber,
    ) -> anyhow::Result<()> {
        match self.epoch {
            FinalChainRewardSchedulerEpochState::Unbound => {
                anyhow::bail!("FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_STARTUP_UNBOUND")
            }
            FinalChainRewardSchedulerEpochState::JointStartup(epoch) => anyhow::ensure!(
                epoch == expected_old_epoch,
                "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_EPOCH_MISMATCH"
            ),
            FinalChainRewardSchedulerEpochState::Live(epoch) => anyhow::ensure!(
                epoch == expected_old_epoch,
                "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_EPOCH_MISMATCH"
            ),
        }
        anyhow::ensure!(
            new_epoch != 0 && new_epoch != expected_old_epoch,
            "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_EPOCH_NOT_REPLACED"
        );
        anyhow::ensure!(
            self.published_head == prior_head,
            "FINAL_CHAIN_REWARD_SCHEDULER_DISCARD_HEAD_MISMATCH"
        );
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_REWARD_SCHEDULER_GENERATION_OVERFLOW"))?;
        self.epoch = FinalChainRewardSchedulerEpochState::Live(new_epoch);
        self.next_cleanup_block = 0;
        self.admitted = None;
        self.pending = None;
        self.last_installed = None;
        self.verified_reopen = Some((expected_old_epoch, new_epoch, prior_head));
        self.generation = next_generation;
        Ok(())
    }

    /// Completes only a marker-free recovery proven by the retained verified
    /// discard transition.
    pub(crate) fn complete_verified_discard_recovery(
        &mut self,
        state_api_epoch: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.verified_reopen
                .is_some_and(|(_, new_epoch, head)| new_epoch == state_api_epoch
                    && head == self.published_head),
            "FINAL_CHAIN_REWARD_SCHEDULER_MARKER_FREE_DISCARD_UNPROVEN"
        );
        self.complete_recovery(state_api_epoch)
    }

    fn validate_prepared(
        &self,
        prepared: FinalChainPreparedRewardSchedulerSuccessor,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            prepared.source_instance_id == self.source_instance_id,
            "FINAL_CHAIN_REWARD_SCHEDULER_SOURCE_INSTANCE_MISMATCH"
        );
        anyhow::ensure!(
            self.epoch == FinalChainRewardSchedulerEpochState::Live(prepared.state_api_epoch),
            "FINAL_CHAIN_REWARD_SCHEDULER_EPOCH_MISMATCH"
        );
        anyhow::ensure!(
            self.published_head == prepared.expected_parent
                && prepared.expected_parent.checked_next() == Some(prepared.period),
            "FINAL_CHAIN_REWARD_SCHEDULER_HEAD_MISMATCH"
        );
        anyhow::ensure!(
            self.generation == prepared.expected_runtime_generation,
            "FINAL_CHAIN_REWARD_SCHEDULER_GENERATION_MISMATCH"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        basis: FinalChainRewardSchedulerBasis,
    ) -> FinalChainRewardSchedulerPublicationBinding {
        FinalChainRewardSchedulerPublicationBinding {
            request_id: basis.request_id,
            expected_parent: basis.expected_parent,
            period: basis.period,
            state_api_epoch: basis.state_api_epoch,
            concrete_database_id: [3; 32],
            concrete_generation: 9,
            concrete_projection_hash: [4; 32],
            publication_plan_id: [5; 32],
        }
    }

    fn make_live(runtime: &mut FinalChainRewardSchedulerRuntime, epoch: u64) {
        runtime
            .authorize_joint_startup(epoch, runtime.published_head)
            .unwrap();
        runtime.complete_recovery(epoch).unwrap();
    }

    fn apply_verified_reopen(
        runtime: &mut FinalChainRewardSchedulerRuntime,
        old_epoch: u64,
        new_epoch: u64,
        head: FinalChainBlockNumber,
    ) -> anyhow::Result<()> {
        runtime.record_verified_discard(old_epoch, new_epoch, head)?;
        anyhow::ensure!(runtime.complete_verified_reopen(new_epoch, head)?);
        Ok(())
    }

    #[test]
    fn instance_allocator_enters_permanent_zero_exhaustion_sentinel() {
        assert_eq!(scheduler_instance_successor(1), Some(2));
        assert_eq!(scheduler_instance_successor(u64::MAX), Some(0));
        assert_eq!(scheduler_instance_successor(0), None);
    }

    #[test]
    fn joint_startup_is_consumed_once_per_runtime_instance() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        runtime.authorize_joint_startup(11, 7.into()).unwrap();
        runtime.complete_recovery(11).unwrap();
        assert!(runtime.authorize_joint_startup(11, 7.into()).is_err());
        assert!(runtime.authorize_joint_startup(12, 7.into()).is_err());
    }

    #[test]
    fn unbound_runtime_rejects_ordinary_execution_recovery_and_discard() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        assert!(runtime.observe_live_epoch(11).is_err());
        assert!(runtime.validate_bound_recovery_epoch(11).is_err());
        assert!(apply_verified_reopen(&mut runtime, 11, 12, 7.into()).is_err());
    }

    #[test]
    fn uncommitted_clear_requires_joint_startup_or_verified_epoch_replacement() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        runtime.authorize_joint_startup(11, 7.into()).unwrap();
        runtime.validate_uncommitted_clear(0, 11).unwrap();
        runtime.complete_recovery(11).unwrap();
        assert!(runtime.validate_uncommitted_clear(0, 11).is_err());
        apply_verified_reopen(&mut runtime, 11, 12, 7.into()).unwrap();
        runtime.validate_uncommitted_clear(0, 12).unwrap();
        runtime.complete_recovery(12).unwrap();
        assert!(runtime.validate_uncommitted_clear(0, 12).is_err());
    }

    #[test]
    fn exact_successor_installs_once_after_arming() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        make_live(&mut runtime, 11);
        let basis = runtime.basis(11, 7.into(), 8.into(), [1; 32]).unwrap();
        let prepared = basis.successor(15);
        let binding = binding(basis);
        runtime.arm(prepared, binding).unwrap();
        runtime.arm(prepared, binding).unwrap();
        assert_eq!(
            runtime.install_applied(binding).unwrap(),
            FinalChainRewardSchedulerInstall::Installed
        );
        assert_eq!(runtime.published_head, FinalChainBlockNumber::new(8));
        assert_eq!(runtime.next_cleanup_block, 15);
        assert_eq!(
            runtime.install_applied(binding).unwrap(),
            FinalChainRewardSchedulerInstall::AlreadyInstalled
        );
    }

    #[test]
    fn foreign_final_chain_instance_cannot_arm_coincident_generation() {
        let mut first = FinalChainRewardSchedulerRuntime::new(7.into());
        let mut second = FinalChainRewardSchedulerRuntime::new(7.into());
        make_live(&mut first, 11);
        make_live(&mut second, 11);
        let basis = first.basis(11, 7.into(), 8.into(), [1; 32]).unwrap();
        assert!(second.arm(basis.successor(15), binding(basis)).is_err());
    }

    #[test]
    fn verified_reopen_replaces_epoch_resets_timer_and_invalidates_pending() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        make_live(&mut runtime, 11);
        let basis = runtime.basis(11, 7.into(), 8.into(), [1; 32]).unwrap();
        let publication = binding(basis);
        runtime.arm(basis.successor(15), publication).unwrap();
        apply_verified_reopen(&mut runtime, 11, 12, 7.into()).unwrap();
        assert_eq!(runtime.next_cleanup_block, 0);
        assert!(runtime.install_applied(publication).is_err());
        assert!(runtime.basis(11, 7.into(), 8.into(), [2; 32]).is_err());
        runtime.complete_recovery(12).unwrap();
        assert!(runtime.basis(12, 7.into(), 8.into(), [2; 32]).is_ok());
    }

    #[test]
    fn verified_discard_is_retained_across_reopen_load_failure() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        make_live(&mut runtime, 11);
        runtime.record_verified_discard(11, 12, 7.into()).unwrap();
        assert!(runtime.basis(11, 7.into(), 8.into(), [1; 32]).is_err());
        runtime.validate_bound_recovery_epoch(12).unwrap();
        assert!(runtime.validate_bound_recovery_epoch(13).is_err());
        assert!(runtime.complete_verified_reopen(12, 8.into()).is_err());
        assert!(runtime.complete_verified_reopen(12, 7.into()).unwrap());
        runtime.complete_verified_discard_recovery(12).unwrap();
        assert!(runtime.basis(12, 7.into(), 8.into(), [2; 32]).is_ok());
    }

    #[test]
    fn joint_startup_recovery_advances_with_zero_timer_and_retries_exact_binding() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        runtime.authorize_joint_startup(11, 7.into()).unwrap();
        let publication = FinalChainRewardSchedulerPublicationBinding {
            request_id: [1; 32],
            expected_parent: 7.into(),
            period: 8.into(),
            state_api_epoch: 11,
            concrete_database_id: [3; 32],
            concrete_generation: 10,
            concrete_projection_hash: [4; 32],
            publication_plan_id: [5; 32],
        };
        runtime
            .install_recovered_publication(11, publication)
            .unwrap();
        assert_eq!(runtime.next_cleanup_block, 0);
        assert_eq!(
            runtime
                .install_recovered_publication(11, publication)
                .unwrap(),
            FinalChainRewardSchedulerInstall::AlreadyInstalled
        );
        assert!(
            runtime
                .install_applied(FinalChainRewardSchedulerPublicationBinding {
                    request_id: [1; 32],
                    expected_parent: 8.into(),
                    period: 9.into(),
                    state_api_epoch: 11,
                    concrete_database_id: [3; 32],
                    concrete_generation: 10,
                    concrete_projection_hash: [4; 32],
                    publication_plan_id: [5; 32],
                })
                .is_err()
        );
    }

    #[test]
    fn continuous_runtime_skips_until_cached_minimum_then_cleans() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(1.into());
        make_live(&mut runtime, 11);
        let jailed = [[7; 20]];
        let jail_blocks = BTreeMap::from([([7; 20], 5)]);

        for period in 2..=5 {
            let parent = FinalChainBlockNumber::new(period - 1);
            let basis = runtime
                .basis(11, parent, period.into(), [period as u8; 32])
                .unwrap();
            let plan = basis.plan_cleanup(&jailed, &jail_blocks);
            assert_eq!(plan.ran, period == 2 || period == 5);
            assert_eq!(plan.retained_validators.is_empty(), period == 5);
            let publication = binding(basis);
            runtime.arm(plan.successor, publication).unwrap();
            runtime.install_applied(publication).unwrap();
        }
        assert_eq!(runtime.published_head, FinalChainBlockNumber::new(5));
        assert_eq!(runtime.next_cleanup_block, 5);
    }

    #[test]
    fn verified_reopen_resets_cached_skip_without_persisting_timer() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(3.into());
        make_live(&mut runtime, 11);
        runtime.next_cleanup_block = 101;
        let basis = runtime.basis(11, 3.into(), 4.into(), [4; 32]).unwrap();
        let jail_blocks = BTreeMap::from([([7; 20], 4)]);
        assert!(!basis.plan_cleanup(&[[7; 20]], &jail_blocks).ran);

        apply_verified_reopen(&mut runtime, 11, 12, 3.into()).unwrap();
        runtime.complete_recovery(12).unwrap();
        let reopened = runtime.basis(12, 3.into(), 4.into(), [5; 32]).unwrap();
        let plan = reopened.plan_cleanup(&[[7; 20]], &jail_blocks);
        assert!(plan.ran);
        assert!(plan.retained_validators.is_empty());
    }

    #[test]
    fn live_recovery_requires_retained_exact_candidate() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        make_live(&mut runtime, 11);
        let basis = runtime.basis(11, 7.into(), 8.into(), [1; 32]).unwrap();
        let publication = binding(basis);
        assert!(
            runtime
                .validate_recovery_publication(11, publication)
                .is_err()
        );
        runtime.arm(basis.successor(15), publication).unwrap();
        runtime
            .validate_recovery_publication(11, publication)
            .unwrap();
    }

    #[test]
    fn commit_epoch_restoration_requires_exact_pending_candidate() {
        let mut runtime = FinalChainRewardSchedulerRuntime::new(7.into());
        make_live(&mut runtime, 11);
        let basis = runtime.basis(11, 7.into(), 8.into(), [1; 32]).unwrap();
        let publication = binding(basis);
        assert!(runtime.validate_pending_publication(publication).is_err());
        runtime.arm(basis.successor(15), publication).unwrap();
        runtime.validate_pending_publication(publication).unwrap();
        assert!(
            runtime
                .validate_pending_publication(FinalChainRewardSchedulerPublicationBinding {
                    state_api_epoch: 12,
                    ..publication
                })
                .is_err()
        );
    }
}
