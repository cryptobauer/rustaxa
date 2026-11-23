use super::*;
use crate::consensus_application_runtime::{
    DagGasEstimateRequest, OwnedDagGasOrigin, OwnedDagGasReport, OwnedDagGasRequest,
};
use crate::final_chain::{DPOS_CONTRACT_ADDRESS, SLASHING_CONTRACT_ADDRESS};
use crate::native_session::{
    ConcreteCheckpointNativeStateRead, FinalChainNativeSessionError, FinalChainNativeSimulation,
};
use crate::pbft_execution_validation::PbftInstructionPhase;
use rustaxa_storage::ConcreteCheckpointReaders;
use rustaxa_types::{FinalChainCallOutcome, FinalChainCallRequest, concrete_state::ConcreteRead};

enum DagGasScope<'a> {
    Read(&'a BoundApplicationAction<'a>),
    Mutation(&'a ApplicationMutationScope<'a>),
}

/// Read-only execution facts for an exact finalized checkpoint.
pub struct DagGasState<'a> {
    application: &'a BoundConsensusApplication,
    scope: DagGasScope<'a>,
    proposal_period: FinalChainBlockNumber,
    committed_period: FinalChainBlockNumber,
}

impl BoundConsensusApplication {
    #[doc(hidden)]
    pub fn with_finalized_execution_state<R>(
        &self,
        period: FinalChainBlockNumber,
        read: impl FnOnce(&DagGasState<'_>) -> Result<R>,
    ) -> Result<R> {
        let action = self.source.read()?;
        action.validate(self)?;
        let head = read_committed_identity(&self.core.storage)?.period;
        ensure!(
            self.core.final_chain.last_block_number_typed()? == head,
            "PRODUCTION_FINALIZED_EXECUTION_HEAD_MISMATCH"
        );
        read(&DagGasState {
            application: self,
            scope: DagGasScope::Read(&action),
            proposal_period: period,
            committed_period: head,
        })
    }

    #[doc(hidden)]
    pub fn with_owned_dag_gas_state(
        &self,
        authority: &ProductionOwnerPublicationAuthority,
        request: OwnedDagGasRequest<'_>,
        estimate: impl FnOnce(&DagGasState<'_>, &DagGasEstimateRequest) -> Result<OwnedDagGasReport>,
    ) -> Result<Option<OwnedDagGasReport>> {
        ensure!(
            std::ptr::eq(request.source, self.source.as_ref()),
            "PRODUCTION_DAG_GAS_FOREIGN_OWNER"
        );
        match request.origin {
            OwnedDagGasOrigin::Pack(id) => {
                let action = self.source.read()?;
                if action.generation != request.generation {
                    return Ok(None);
                }
                action.validate(self)?;
                if self.core.dag_transaction.active_proposer_pack_period(id)?
                    != Some(request.request.proposal_period)
                {
                    return Ok(None);
                }
                self.estimate_dag_gas(
                    authority,
                    DagGasScope::Read(&action),
                    request.committed_period,
                    request.request,
                    estimate,
                )
                .map(Some)
            }
            OwnedDagGasOrigin::Verify {
                scope,
                cursor_id,
                block_hash,
            } => {
                let ApplicationOperationScope::Mutation(source, mutation) = scope else {
                    bail!("PRODUCTION_DAG_GAS_VERIFICATION_OWNER_MISSING");
                };
                ensure!(
                    std::ptr::eq(*source, self.source.as_ref())
                        && mutation.generation() == request.generation,
                    "PRODUCTION_DAG_GAS_VERIFICATION_OWNER_MISMATCH"
                );
                ensure!(
                    self.core.dag_transaction.matches_verify_gas_cursor(
                        cursor_id,
                        block_hash,
                        request.request.proposal_period,
                    )?,
                    "PRODUCTION_DAG_GAS_VERIFICATION_CURSOR_MISMATCH"
                );
                self.estimate_dag_gas(
                    authority,
                    DagGasScope::Mutation(mutation),
                    request.committed_period,
                    request.request,
                    estimate,
                )
                .map(Some)
            }
        }
    }

    fn estimate_dag_gas(
        &self,
        authority: &ProductionOwnerPublicationAuthority,
        scope: DagGasScope<'_>,
        committed_period: u64,
        request: &DagGasEstimateRequest,
        estimate: impl FnOnce(&DagGasState<'_>, &DagGasEstimateRequest) -> Result<OwnedDagGasReport>,
    ) -> Result<OwnedDagGasReport> {
        authority.validate(self)?;
        let committed = read_committed_identity(&self.core.storage)?;
        ensure!(
            committed.period.as_u64() == committed_period
                && self.core.final_chain.last_block_number_typed()? == committed.period,
            "PRODUCTION_DAG_GAS_HEAD_MISMATCH"
        );
        let proposal_period = request.proposal_period.into();
        ensure!(
            self.source.schedule.correction_stage(proposal_period) == CorrectionStage::Historical,
            "PRODUCTION_DAG_GAS_RULES_UNAVAILABLE"
        );
        let state = DagGasState {
            application: self,
            scope,
            proposal_period,
            committed_period: committed.period,
        };
        estimate(&state, request)
    }
}

impl DagGasState<'_> {
    fn with_checkpoint<R>(
        &self,
        period: Option<u64>,
        read: impl FnOnce(&ConcreteCheckpointReaders, ConcreteStateIdentity) -> Result<R>,
    ) -> Result<Option<R>> {
        match self.scope {
            DagGasScope::Read(action) => self
                .application
                .with_finalized_state_in_action(action, period, read),
            DagGasScope::Mutation(scope) => self
                .application
                .source
                .with_finalized_state_in_mutation(scope, period, read),
        }
    }

    pub fn proposal_period(&self) -> FinalChainBlockNumber {
        self.proposal_period
    }

    pub fn is_direct_dpos(&self, recipient: Option<[u8; 20]>) -> bool {
        recipient == Some(DPOS_CONTRACT_ADDRESS)
    }

    pub fn dpos_call(&self, request: FinalChainCallRequest) -> Result<FinalChainCallOutcome> {
        ensure!(
            request.block_number == self.proposal_period
                && request.receiver == Some(DPOS_CONTRACT_ADDRESS),
            "PRODUCTION_DAG_DPOS_CALL_MISMATCH"
        );
        let requested = FinalChain::dpos_call_uses_historical_accounts(&request)
            .then_some(self.proposal_period.as_u64());
        self.with_checkpoint(requested, |readers, identity| {
            let reader = ConcreteCheckpointNativeStateRead::new(readers, identity)?;
            self.application
                .core
                .final_chain
                .call_with_dpos_checkpoint(request, &reader)
        })?
        .context("PRODUCTION_DAG_DPOS_CHECKPOINT_UNAVAILABLE")
    }

    pub fn with_evm_checkpoint<R>(
        &self,
        read: impl FnOnce(&ConcreteCheckpointReaders, ConcreteStateIdentity) -> Result<R>,
    ) -> Result<R> {
        self.with_checkpoint(Some(self.period().as_u64()), read)?
            .context("PRODUCTION_DAG_EVM_CHECKPOINT_UNAVAILABLE")
    }

    pub fn period(&self) -> FinalChainBlockNumber {
        self.proposal_period.min(self.committed_period)
    }

    pub fn chain_id(&self) -> u64 {
        self.application.source.schedule.chain_id()
    }

    pub fn correction_stage(&self) -> CorrectionStage {
        self.application
            .source
            .schedule
            .correction_stage(self.period())
    }

    pub fn bridge_address(&self) -> [u8; 20] {
        self.application.transition_policy.bridge
    }

    pub fn block_header(&self) -> Result<Vec<u8>> {
        self.published_block_header()?
            .context("PRODUCTION_DAG_EVM_HEADER_MISSING")
    }

    /// Returns the published header at the selected period, capped at the committed head.
    pub fn published_block_header(&self) -> Result<Option<Vec<u8>>> {
        self.application
            .core
            .final_chain
            .block_header(self.period())
    }

    pub fn block_hash(&self, number: FinalChainBlockNumber) -> Result<Option<Vec<u8>>> {
        self.application.core.final_chain.block_hash(number)
    }

    pub fn instruction_phase(&self) -> PbftInstructionPhase {
        self.application
            .core
            .final_chain
            .pbft_execution_instruction_phase(self.period())
    }

    pub fn cornus(&self) -> bool {
        self.application
            .core
            .final_chain
            .pbft_execution_cornus(self.period())
    }

    pub fn is_native(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.application
            .core
            .final_chain
            .native_contract_kind(period, address)
            .is_some()
    }

    pub fn with_trace_checkpoint<R>(
        &self,
        read: impl FnOnce(
            &ConcreteCheckpointReaders,
            ConcreteStateIdentity,
            FinalChainNativeSimulation<'_>,
        ) -> Result<R>,
    ) -> Result<R> {
        let period = self.proposal_period();
        ensure!(period == self.period(), "PRODUCTION_TRACE_ABOVE_HEAD");
        let prior = FinalChainBlockNumber::new(period.as_u64().saturating_sub(1));
        let header = self
            .application
            .core
            .final_chain
            .block_header(prior)?
            .context("PRODUCTION_TRACE_PARENT_HEADER_MISSING")?;
        let root: ethereum_types::H256 = rlp::Rlp::new(&header).val_at(3)?;
        let native = self
            .application
            .core
            .final_chain
            .begin_native_trace_with_slashing_accounts(prior, period, |source| {
                self.slashing_account_present(source)
            })?;
        self.with_checkpoint(Some(prior.as_u64()), |readers, identity| {
            ensure!(
                identity.period == prior && identity.state_root == root.0,
                "PRODUCTION_TRACE_PARENT_MISMATCH"
            );
            read(readers, identity, native)
        })?
        .context("PRODUCTION_TRACE_CHECKPOINT_UNAVAILABLE")
    }

    fn slashing_account_present(
        &self,
        period: FinalChainBlockNumber,
    ) -> std::result::Result<bool, FinalChainNativeSessionError> {
        self.with_checkpoint(Some(period.as_u64()), |readers, identity| {
            Ok(matches!(
                readers.account_at(identity, SLASHING_CONTRACT_ADDRESS)?,
                ConcreteRead::Present(_)
            ))
        })
        .and_then(|present| present.context("PRODUCTION_DAG_SLASHING_CHECKPOINT_UNAVAILABLE"))
        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))
    }

    pub fn begin_native_simulation(
        &self,
    ) -> std::result::Result<FinalChainNativeSimulation<'_>, FinalChainNativeSessionError> {
        self.application
            .core
            .final_chain
            .begin_native_simulation_with_slashing_accounts(self.period(), |period| {
                self.slashing_account_present(period)
            })
    }
}
