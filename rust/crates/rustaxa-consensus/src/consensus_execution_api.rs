use crate::{
    FinalChain, FinalChainEvmExecutionReport, FinalChainEvmRewardsReport,
    FinalChainExecutionSession, FinalChainExecutionStep, FinalChainExternalEvmCommitDecision,
    FinalChainExternalEvmCommitPlan, FinalChainExternalEvmCommittedStateDescriptor,
    FinalChainExternalEvmPublicationAuditReport, FinalChainExternalEvmPublicationPlan,
    FinalChainExternalEvmPublicationReport, FinalChainExternalEvmRewardsStatsUpdate,
    FinalChainExternalEvmStateCommitIntent, FinalChainExternalEvmStateCommitRequest,
    FinalChainExternalEvmStateCommitResult, FinalChainProposalPeriodDagLevelUpdate,
    FinalChainSystemTransactionReport,
    final_chain_execution_session_attach_external_evm_proposal_period_dag_level,
    final_chain_execution_session_attach_external_evm_rewards_stats,
    final_chain_execution_session_next,
    final_chain_execution_session_persist_external_evm_pending_publication,
    final_chain_execution_session_plan_external_evm_commit,
    final_chain_execution_session_plan_external_evm_publication,
    final_chain_execution_session_prepare_external_evm_state_commit,
    final_chain_execution_session_publish_external_evm_publication,
    final_chain_execution_session_report_evm,
    final_chain_execution_session_report_external_evm_state_commit_result,
    final_chain_execution_session_report_system_transactions,
    final_chain_execution_session_request_external_evm_state_commit,
};

/// External EVM and StateAPI facade for Rust-owned FinalChain execution.
///
/// This facade is the narrow consensus-facing API for the external execution
/// boundary. Rust owns request identity, report validation, publication
/// planning, pending-publication markers, storage publication, and audit
/// decisions. The caller owns arbitrary EVM execution, `StateAPI` state
/// mutation, rewards execution, and the concrete state DB lifecycle. Methods
/// operate on an explicit FinalChain execution session so ownership and
/// sequencing remain visible while C++ still hosts the executor.
#[derive(Debug, Default)]
pub struct ConsensusExecutionApi;

impl ConsensusExecutionApi {
    /// Creates a stateless external execution facade.
    ///
    /// The facade intentionally stores no `StateAPI`, EVM executor, storage
    /// batch, or FinalChain session. Callers pass the live Rust handles for each
    /// operation, which keeps temporary C++ executor ownership explicit.
    pub fn new() -> Self {
        Self
    }

    /// Returns the next external-execution request or action for `session`.
    ///
    /// Output is the existing session step DTO. `EXECUTE_EXTERNAL_EVM`,
    /// `DISTRIBUTE_EXTERNAL_EVM_REWARDS`, `REQUEST_EXTERNAL_EVM_STATE_COMMIT`,
    /// and `PUBLISH_EXTERNAL_EVM_STORAGE` are external boundary actions; native
    /// commit and rejection remain explicit session outcomes.
    pub fn next_execution_request(
        &self,
        session: &mut FinalChainExecutionSession,
    ) -> FinalChainExecutionStep {
        final_chain_execution_session_next(session)
    }

    /// Reports an external EVM transaction execution result.
    ///
    /// Rust validates the report against the exact request emitted by the
    /// session and returns the next action. The method does not execute EVM,
    /// mutate state DB, publish FinalChain storage, or inspect `StateAPI`.
    pub fn report_execution_result(
        &self,
        session: &mut FinalChainExecutionSession,
        report: FinalChainEvmExecutionReport,
    ) -> FinalChainExecutionStep {
        final_chain_execution_session_report_evm(session, report)
    }

    /// Reports Rust-planned system transaction bytes materialized by the executor boundary.
    ///
    /// C++ may still collect bridge-contract facts through `StateAPI`, but the
    /// returned transaction bytes are validated by Rust before arbitrary EVM
    /// execution is requested.
    pub fn report_system_transactions(
        &self,
        session: &mut FinalChainExecutionSession,
        report: FinalChainSystemTransactionReport,
    ) -> FinalChainExecutionStep {
        final_chain_execution_session_report_system_transactions(session, report)
    }

    /// Reports external reward execution and derives the Rust commit plan.
    ///
    /// The executor boundary supplies the post-rewards root and total reward.
    /// Rust validates those facts against the prior EVM execution request and
    /// produces deterministic publication inputs without mutating storage.
    pub fn report_rewards_result(
        &self,
        session: &mut FinalChainExecutionSession,
        report: FinalChainEvmRewardsReport,
    ) -> FinalChainExternalEvmCommitPlan {
        final_chain_execution_session_plan_external_evm_commit(session, report)
    }

    /// Plans Rust-owned FinalChain publication facts for an external EVM block.
    ///
    /// The plan is read-only with respect to `StateAPI` and state DB. It
    /// materializes the deterministic block/publication facts that must be
    /// matched by the later state-commit and storage-publication steps.
    pub fn plan_publication(
        &self,
        final_chain: &FinalChain,
        session: &mut FinalChainExecutionSession,
    ) -> FinalChainExternalEvmPublicationPlan {
        final_chain_execution_session_plan_external_evm_publication(final_chain, session)
    }

    /// Builds the entire external-EVM publication preparation bundle.
    ///
    /// This performs publication plan derivation, reward-stat/proposal-period
    /// plan attachment, state-commit request authorization, and pending
    /// publication marker persistence. Callers use the returned intent after
    /// `StateAPI::transition_state_commit()`.
    pub fn prepare_external_evm_state_commit(
        &self,
        final_chain: &FinalChain,
        session: &mut FinalChainExecutionSession,
        rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
        proposal_period_update: FinalChainProposalPeriodDagLevelUpdate,
    ) -> Result<FinalChainExternalEvmStateCommitIntent, anyhow::Error> {
        final_chain_execution_session_prepare_external_evm_state_commit(
            final_chain,
            session,
            rewards_stats_update,
            proposal_period_update,
        )
    }

    /// Attaches rewards-stat storage facts to the pending publication plan.
    ///
    /// The facts are computed by the current C++ rewards executor boundary but
    /// Rust owns committing them atomically with FinalChain publication.
    pub fn attach_rewards_stats(
        &self,
        session: &mut FinalChainExecutionSession,
        update: FinalChainExternalEvmRewardsStatsUpdate,
    ) -> FinalChainExternalEvmPublicationPlan {
        final_chain_execution_session_attach_external_evm_rewards_stats(session, update)
    }

    /// Attaches the optional proposal-period DAG-level mapping to the publication plan.
    ///
    /// C++ currently materializes the anchor object, while Rust owns validating
    /// and publishing the resulting mapping with the FinalChain block batch.
    pub fn attach_proposal_period_dag_level(
        &self,
        session: &mut FinalChainExecutionSession,
        update: FinalChainProposalPeriodDagLevelUpdate,
    ) -> FinalChainExternalEvmPublicationPlan {
        final_chain_execution_session_attach_external_evm_proposal_period_dag_level(session, update)
    }

    /// Returns Rust's state-commit intent for the supplied publication facts.
    ///
    /// The caller supplies the immutable post-execution and post-rewards roots
    /// from the Rust commit plan plus the publication identity. A ready intent
    /// only allows the external executor to attempt its own staged-state commit;
    /// storage publication still requires a subsequent committed result report.
    pub fn next_state_commit_request(
        &self,
        session: &mut FinalChainExecutionSession,
        commit_plan: &FinalChainExternalEvmCommitPlan,
        publication_plan: &FinalChainExternalEvmPublicationPlan,
    ) -> FinalChainExternalEvmStateCommitIntent {
        final_chain_execution_session_request_external_evm_state_commit(
            session,
            FinalChainExternalEvmStateCommitRequest {
                request_id: publication_plan.request_id,
                plan_id: publication_plan.plan_id,
                period: publication_plan.period,
                post_execution_state_root: commit_plan.post_execution_state_root,
                post_rewards_state_root: commit_plan.state_root,
                publication_block_hash: publication_plan.block_hash,
            },
        )
    }

    /// Persists the pending-publication marker before external state commit.
    ///
    /// This must happen before the caller invokes `StateAPI` state commit. On
    /// restart, Rust can then recover or clear the publication without
    /// re-executing arbitrary EVM work.
    pub fn persist_pending_publication(
        &self,
        final_chain: &FinalChain,
        session: &mut FinalChainExecutionSession,
    ) -> Result<FinalChainExternalEvmPublicationReport, anyhow::Error> {
        final_chain_execution_session_persist_external_evm_pending_publication(final_chain, session)
    }

    /// Reports the external state DB commit result.
    ///
    /// Rust derives the full lifecycle report from the session-owned request,
    /// commit plan, and publication plan. A successful committed report returns
    /// a decision that permits Rust FinalChain storage publication.
    pub fn report_state_commit_result(
        &self,
        final_chain: &FinalChain,
        session: &mut FinalChainExecutionSession,
        result: FinalChainExternalEvmStateCommitResult,
    ) -> Result<FinalChainExternalEvmCommitDecision, anyhow::Error> {
        final_chain_execution_session_report_external_evm_state_commit_result(
            final_chain,
            session,
            result,
        )
    }

    /// Publishes the session-owned FinalChain storage batch after state commit.
    ///
    /// The method writes only Rust-owned FinalChain/storage rows described by
    /// the accepted publication plan. It does not call `StateAPI` or mutate
    /// `state_db/`.
    pub fn publish_state_commit(
        &self,
        final_chain: &FinalChain,
        session: &mut FinalChainExecutionSession,
    ) -> Result<FinalChainExternalEvmPublicationReport, anyhow::Error> {
        final_chain_execution_session_publish_external_evm_publication(final_chain, session)
    }

    /// Audits a publication plan against persisted Rust storage and external state.
    ///
    /// The audit is read-only and checks storage rows, indexes, receipts,
    /// blooms, transaction mappings, pending-marker state, and the committed
    /// external StateAPI descriptor for the requested publication. It is
    /// suitable for restart recovery validation and external boundary smoke
    /// tests.
    pub fn publication_audit(
        &self,
        final_chain: &FinalChain,
        publication_plan: FinalChainExternalEvmPublicationPlan,
        committed_state: FinalChainExternalEvmCommittedStateDescriptor,
    ) -> Result<FinalChainExternalEvmPublicationAuditReport, anyhow::Error> {
        final_chain
            .audit_external_evm_publication_with_committed_state(publication_plan, committed_state)
    }
}
