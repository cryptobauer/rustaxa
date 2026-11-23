//! Captured physical sources. This module grants no execution or vote authority.
use super::ConsensusApplication;
use crate::execution_configuration::{ConfigurationUnavailable, ExecutionConfiguration};
use crate::final_chain::{
    NativePredecessorCapture, NativePredecessorLimits, NativePredecessorUnavailable,
    PreparedNativePredecessor,
};
use anyhow::Result;
use rustaxa_storage::{
    BorrowedConcreteExecutionProbe, ConcreteCheckpointReaders, ConcreteLifecycleObservation,
    ConcreteStateLifecycle,
};
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteReadError, ConcreteStateIdentity},
};
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

#[derive(Clone, Copy)]
pub(crate) struct ExecutionPredecessorLimits {
    pub(crate) maximum_native_source_bytes: usize,
    pub(crate) maximum_retained_bytes: usize,
}

#[derive(Debug)]
pub(crate) enum ExecutionPredecessorUnavailable {
    Configuration(ConfigurationUnavailable),
    GenesisChanged,
    Native(NativePredecessorUnavailable),
    Concrete(ConcreteReadError),
    PairingMismatch,
    StateMismatch,
    SourceChanged,
    LocalLimit,
}

pub(crate) enum ExecutionPredecessorCapture<T> {
    Captured(T),
    Unavailable(ExecutionPredecessorUnavailable),
}

pub(crate) struct CapturedExecutionPredecessor<'a, 'c> {
    application: &'a ConsensusApplication,
    configuration: ExecutionConfiguration,
    native: PreparedNativePredecessor<'a>,
    concrete_owner: &'c ConcreteStateLifecycle,
    concrete_probe: BorrowedConcreteExecutionProbe<'c>,
    checkpoint: Arc<ConcreteCheckpointReaders>,
    observation: ConcreteLifecycleObservation,
    pending: FinalChainBlockNumber,
    retained_bytes: usize,
    source_digest: [u8; 32],
}

impl ConsensusApplication {
    pub(crate) fn capture_execution_predecessor<'a, 'c>(
        &'a self,
        concrete: &'c ConcreteStateLifecycle,
        pending: FinalChainBlockNumber,
        limits: ExecutionPredecessorLimits,
    ) -> Result<ExecutionPredecessorCapture<CapturedExecutionPredecessor<'a, 'c>>> {
        use ExecutionPredecessorCapture::{Captured, Unavailable};
        use ExecutionPredecessorUnavailable as Missing;
        let configuration = match self.execution_configuration() {
            Ok(value) => value,
            Err(reason) => return Ok(Unavailable(Missing::Configuration(reason.clone()))),
        };
        if configuration.canonical_bytes().len() > limits.maximum_retained_bytes {
            return Ok(Unavailable(Missing::LocalLimit));
        }
        let genesis = self.storage.metadata().genesis_hash()?;
        if genesis.as_deref() != Some(configuration.full_genesis().as_bytes()) {
            return Ok(Unavailable(Missing::GenesisChanged));
        }
        let observation = match concrete.observation() {
            Ok(value) => value,
            Err(reason) => return Ok(Unavailable(Missing::Concrete(reason))),
        };
        let concrete_probe = match concrete.borrow_execution_probe() {
            Ok(value) => value,
            Err(reason) => return Ok(Unavailable(Missing::Concrete(reason))),
        };
        let native = match self.final_chain.capture_native_predecessor(
            pending,
            NativePredecessorLimits {
                max_retained_source_bytes: limits.maximum_native_source_bytes,
            },
        )? {
            NativePredecessorCapture::Ready(value) => value,
            NativePredecessorCapture::Unavailable(
                NativePredecessorUnavailable::RetainedSourceLimit,
            ) => {
                return Ok(Unavailable(Missing::LocalLimit));
            }
            NativePredecessorCapture::Unavailable(reason) => {
                return Ok(Unavailable(Missing::Native(reason)));
            }
        };
        if !native.matches_owner(self.final_chain.as_ref())
            || native.committed_identity().period.checked_next() != Some(pending)
            || native.committed_identity() != observation.committed
            || concrete_probe.committed_identity() != observation.committed
            || concrete_probe.historical_identity() != observation.committed
            || !observation.pending_marker_rlp.is_empty()
        {
            return Ok(Unavailable(Missing::StateMismatch));
        }
        if native.pairing_identity() != observation.identity
            || observation.identity.database_id == [0; 32]
            || observation.identity.chain_id != self.final_chain.concrete_chain_identity()?
            || native.pairing_storage_genesis() != configuration.full_genesis().0
        {
            return Ok(Unavailable(Missing::PairingMismatch));
        }
        let retained_bytes = [
            native.retained_source_bytes(),
            configuration.canonical_bytes().len(),
            observation.provenance_rlp.len(),
            observation.catalog_rlp.len(),
            observation.pending_marker_rlp.len(),
            32,
            std::mem::size_of::<ExecutionConfiguration>(),
            std::mem::size_of::<ConcreteLifecycleObservation>(),
        ]
        .into_iter()
        .try_fold(0_usize, usize::checked_add);
        let Some(retained_bytes) = retained_bytes.filter(|n| *n <= limits.maximum_retained_bytes)
        else {
            return Ok(Unavailable(Missing::LocalLimit));
        };
        let checkpoint = match concrete.open_retained_checkpoint(native.delayed_identity()) {
            Ok(value) => Arc::new(value),
            Err(reason) => return Ok(Unavailable(Missing::Concrete(reason))),
        };
        if !checkpoint_matches(
            &checkpoint,
            native.committed_identity(),
            native.delayed_identity(),
        ) {
            return Ok(Unavailable(Missing::StateMismatch));
        }
        let source_digest = digest_sources(configuration, &native, &observation);
        let captured = CapturedExecutionPredecessor {
            application: self,
            configuration: configuration.clone(),
            native,
            concrete_owner: concrete,
            concrete_probe,
            checkpoint,
            observation,
            pending,
            retained_bytes,
            source_digest,
        };
        if let Some(reason) = captured.recheck_sources(limits)? {
            return Ok(Unavailable(reason));
        }
        Ok(Captured(captured))
    }
}

impl CapturedExecutionPredecessor<'_, '_> {
    /// Checks named sources only. This is not an atomic application reservation.
    pub(crate) fn recheck_sources(
        &self,
        limits: ExecutionPredecessorLimits,
    ) -> Result<Option<ExecutionPredecessorUnavailable>> {
        use ExecutionPredecessorUnavailable as Missing;
        if self.retained_bytes > limits.maximum_retained_bytes {
            return Ok(Some(Missing::LocalLimit));
        }
        let configuration = match self.application.execution_configuration() {
            Ok(value) => value,
            Err(reason) => return Ok(Some(Missing::Configuration(reason.clone()))),
        };
        if configuration != &self.configuration
            || self
                .application
                .storage
                .metadata()
                .genesis_hash()?
                .as_deref()
                != Some(self.configuration.full_genesis().as_bytes())
        {
            return Ok(Some(Missing::SourceChanged));
        }
        let current = match self.application.final_chain.capture_native_predecessor(
            self.pending,
            NativePredecessorLimits {
                max_retained_source_bytes: limits.maximum_native_source_bytes,
            },
        )? {
            NativePredecessorCapture::Ready(value) => value,
            NativePredecessorCapture::Unavailable(
                NativePredecessorUnavailable::RetainedSourceLimit,
            ) => {
                return Ok(Some(Missing::LocalLimit));
            }
            NativePredecessorCapture::Unavailable(reason) => {
                return Ok(Some(Missing::Native(reason)));
            }
        };
        if !self.native.same_sources(&current) {
            return Ok(Some(Missing::SourceChanged));
        }
        let observation = match self.concrete_owner.observation() {
            Ok(value) => value,
            Err(reason) => return Ok(Some(Missing::Concrete(reason))),
        };
        if let Err(reason) = self.concrete_owner.borrow_execution_probe() {
            return Ok(Some(Missing::Concrete(reason)));
        }
        if observation != self.observation
            || self.concrete_probe.committed_identity() != observation.committed
            || !checkpoint_matches(
                &self.checkpoint,
                self.native.committed_identity(),
                self.native.delayed_identity(),
            )
        {
            return Ok(Some(Missing::SourceChanged));
        }
        Ok(None)
    }
}

fn checkpoint_matches(
    checkpoint: &ConcreteCheckpointReaders,
    current: ConcreteStateIdentity,
    delayed: ConcreteStateIdentity,
) -> bool {
    checkpoint.committed_identity() == current
        && checkpoint.retained_identities().contains(&current)
        && checkpoint.retained_identities().contains(&delayed)
}

fn digest_sources(
    configuration: &ExecutionConfiguration,
    native: &PreparedNativePredecessor<'_>,
    observation: &ConcreteLifecycleObservation,
) -> [u8; 32] {
    let mut hash = Keccak::v256();
    hash.update(b"rustaxa-captured-execution-predecessor-v1");
    hash.update(&configuration.digest());
    hash.update(&native.source_digest());
    hash.update(&observation.generation.to_be_bytes());
    for bytes in [
        observation.provenance_rlp.as_slice(),
        observation.catalog_rlp.as_slice(),
        observation.pending_marker_rlp.as_slice(),
    ] {
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    let mut digest = [0; 32];
    hash.finalize(&mut digest);
    digest
}

#[cfg(test)]
mod tests;
