//! Deterministic mixed-state publication interruption and rejection checks.
//!
//! Mounted by the mixed-period integration test, this module reuses its actual
//! execution adapter and adds faults only at application/commit report boundaries.
//! Every database is exclusively created by this finite synthetic fixture.

use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    None,
    BeforeCommit,
    AfterCommit,
    StaleNativeValue,
    MissingCatalog,
    InvocationOrder,
    FailureDisposition,
    PriorIdentity,
    PreparedIntent,
    RewardAccount,
}

struct FaultAdapter<'a> {
    inner: Adapter<'a>,
    fault: Fault,
    executions: Cell<usize>,
    rewards: Cell<usize>,
    commits: Cell<usize>,
    original_projection_hash: Cell<Option<[u8; 32]>>,
}
impl ConsensusExecutionPort for FaultAdapter<'_> {
    fn load_final_chain_committed_state(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        let mut report = self.inner.load_final_chain_committed_state(request)?;
        if self.fault == Fault::PriorIdentity {
            report.committed.state_root[0] ^= 1;
        }
        Ok(report)
    }
    fn load_system_transaction_facts(
        &self,
        request: &FinalChainSystemTransactionFactsRequest,
    ) -> Result<FinalChainSystemTransactionPlanFact> {
        ConsensusExecutionPort::load_system_transaction_facts(&self.inner, request)
    }
    fn execute_final_chain_transactions(
        &self,
        request: &FinalChainEvmExecutionRequest,
    ) -> Result<FinalChainEvmExecutionReport> {
        self.executions.set(self.executions.get() + 1);
        self.inner.execute_final_chain_transactions(request)
    }
    fn distribute_final_chain_rewards(
        &self,
        request: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport> {
        self.inner.distribute_final_chain_rewards(request)
    }
    fn distribute_final_chain_rewards_with_native_plan(
        &self,
        request: &FinalChainEvmRewardsRequest,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
    ) -> Result<FinalChainEvmRewardsReport> {
        self.rewards.set(self.rewards.get() + 1);
        let mut report = self
            .inner
            .distribute_final_chain_rewards_with_native_plan(request, plan)?;
        if matches!(
            self.fault,
            Fault::StaleNativeValue
                | Fault::MissingCatalog
                | Fault::InvocationOrder
                | Fault::FailureDisposition
        ) {
            let mut staged = self.inner.staged.borrow_mut();
            let staged = staged
                .as_mut()
                .context("negative projection must be prepared")?;
            self.original_projection_hash
                .set(Some(report.concrete_projection_hash));
            let parts = self.inner.native_context.borrow();
            let parts = parts
                .as_ref()
                .context("negative native contexts must exist")?;
            let projection = &mut staged.projection;
            match self.fault {
                Fault::StaleNativeValue => {
                    let (key, stale) = parts
                        .rewards
                        .raw_mutations
                        .iter()
                        .find_map(|mutation| {
                            let ConcreteRead::Present(value) = &mutation.expected else {
                                return None;
                            };
                            let row = projection.storage.iter().find(|row| {
                                row.contract == mutation.address && row.key == mutation.key.0
                            })?;
                            (!value.is_empty() && value != &row.value)
                                .then(|| ((mutation.address, mutation.key.0), value.clone()))
                        })
                        .context("changed native reward row with an earlier valid encoding")?;
                    projection
                        .storage
                        .iter_mut()
                        .find(|row| (row.contract, row.key) == key)
                        .unwrap()
                        .value = stale;
                }
                Fault::MissingCatalog => {
                    let touched = parts
                        .invocations
                        .iter()
                        .flat_map(|call| call.raw_mutations.iter())
                        .chain(parts.rewards.raw_mutations.iter())
                        .map(|mutation| (mutation.address, mutation.key.0))
                        .collect::<BTreeSet<_>>();
                    let index = projection
                        .storage
                        .iter()
                        .position(|row| {
                            !row.value.is_empty() && !touched.contains(&(row.contract, row.key))
                        })
                        .context("untouched live catalog row")?;
                    projection.storage.remove(index);
                    projection.catalog_hash = concrete_storage_catalog_hash(&projection.storage);
                    staged.catalog_rlp =
                        encode_concrete_storage_catalog(projection.storage.iter().map(|row| {
                            ConcreteStorageSlot {
                                address: row.contract,
                                key: row.key,
                            }
                        }));
                }
                Fault::InvocationOrder => {
                    let effect = projection
                        .transaction_effects
                        .iter_mut()
                        .find(|effect| effect.invocations.len() > 1)
                        .context("multiple actual native calls")?;
                    effect.invocations.swap(0, 1);
                    projection.invocations = projection
                        .transaction_effects
                        .iter()
                        .flat_map(|effect| effect.invocations.iter().cloned())
                        .collect();
                }
                Fault::FailureDisposition => {
                    let invocation = projection
                        .transaction_effects
                        .iter_mut()
                        .flat_map(|effect| effect.invocations.iter_mut())
                        .find(|call| !call.error.is_empty())
                        .context("actual native business failure")?;
                    invocation.disposition = FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED;
                    projection.invocations = projection
                        .transaction_effects
                        .iter()
                        .flat_map(|effect| effect.invocations.iter().cloned())
                        .collect();
                }
                _ => unreachable!(),
            }
            // Preserve all unrelated binding checks so each negative reaches
            // the semantic/order/catalog condition it is intended to exercise.
            report.concrete_projection_rlp = encode_concrete_state_projection(projection);
            report.concrete_projection_hash =
                concrete_state_bytes_digest(&report.concrete_projection_rlp);
            let mut provenance = decode_concrete_state_provenance(&report.concrete_provenance_rlp)?;
            provenance.projection_hash = report.concrete_projection_hash;
            provenance.catalog_hash = projection.catalog_hash;
            report.concrete_provenance_rlp = encode_concrete_state_provenance(&provenance);
            staged.provenance = report.concrete_provenance_rlp.clone();
        }
        Ok(report)
    }
    fn final_chain_native_projection_context(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<FinalChainNativeProjectionContext>> {
        let original = self.original_projection_hash.take();
        let mut context = self.inner.final_chain_native_projection_context(
            request_id,
            original.unwrap_or(projection_hash),
        )?;
        if original.is_some() {
            context
                .as_mut()
                .context("negative opt-in context")?
                .projection_hash = projection_hash;
        }
        if self.fault == Fault::RewardAccount {
            context
                .as_mut()
                .context("opt-in native context")?
                .rewards
                .accounts
                .first_mut()
                .context("actual reward custody read")?
                .1
                .balance += 1;
        }
        Ok(context)
    }
    fn commit_final_chain_state(
        &self,
        request: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult> {
        self.inner
            .state_api_epoch
            .validate(request.state_api_epoch)?;
        self.commits.set(self.commits.get() + 1);
        self.inner
            .chain
            .validate_pending_external_evm_commit(request)?;
        if self.fault == Fault::PreparedIntent {
            let mut foreign = request.clone();
            foreign.concrete_projection_hash[0] ^= 1;
            return Err(self
                .inner
                .chain
                .validate_pending_external_evm_commit(&foreign)
                .expect_err("altered intent accepted"));
        }
        ensure!(
            self.inner
                .application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_some()
        );
        if self.fault == Fault::BeforeCommit {
            self.inner.staged.borrow_mut().take();
            self.inner.concrete.borrow_mut().take();
            bail!("mixed fixture interrupted before concrete commit");
        }
        let result = self.inner.commit_final_chain_state(request)?;
        if self.fault == Fault::AfterCommit {
            bail!("mixed fixture interrupted after concrete commit");
        }
        Ok(result)
    }
    fn discard_final_chain_state(
        &self,
        request: &FinalChainExternalEvmDiscardRequest,
    ) -> Result<FinalChainExternalEvmDiscardReport> {
        self.inner.discard_final_chain_state(request)
    }
    fn load_pillar_anchor_state(
        &self,
        request: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        self.inner.load_pillar_anchor_state(request)
    }
}

/// Recovery exposes durable observations and exact owner-authorized discard
/// only. In particular it cannot reexecute a transaction or manufacture a new
/// publication intent while reconciling the two databases.
struct RecoveryLeaf<'a> {
    path: &'a Path,
    chain_id: [u8; 32],
    discards: Cell<usize>,
    state_api_epoch: StateApiEpoch,
}
impl FinalChainExecutionLeaf for RecoveryLeaf<'_> {
    fn load_committed_state_descriptor(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        ensure!(
            request.concrete_chain_identity == self.chain_id,
            "recovery chain"
        );
        let observed = ConcreteStateLifecycle::inspect_existing(self.path, self.chain_id)?;
        Ok(FinalChainExternalEvmPreflightReport {
            request_id: request.request_id,
            state_api_epoch: self.state_api_epoch.current(),
            committed: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
            concrete_provenance_rlp: observed.provenance_rlp,
            pending_concrete_marker_rlp: observed.pending_marker_rlp,
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn discard_staged_state(
        &self,
        request: &FinalChainExternalEvmDiscardRequest,
    ) -> Result<FinalChainExternalEvmDiscardReport> {
        self.state_api_epoch
            .validate(request.expected_state_api_epoch)?;
        let mut concrete = ConcreteStateLifecycle::open(
            self.path,
            self.chain_id,
            ConcreteStateIdentity {
                period: request.prior_state.period,
                state_root: request.prior_state.state_root,
            },
        )?;
        ensure!(
            concrete_state_bytes_digest(&request.concrete_marker_rlp) == request.marker_hash,
            "recovery discard marker hash"
        );
        concrete.discard_execution(&request.concrete_marker_rlp)?;
        let observed = concrete.observation()?;
        ensure!(
            observed.pending_marker_rlp.is_empty(),
            "recovery discard pending marker"
        );
        ensure!(
            observed.committed.period == request.prior_state.period
                && observed.committed.state_root == request.prior_state.state_root,
            "recovery discard committed descriptor"
        );
        let (previous_state_api_epoch, state_api_epoch) = self
            .state_api_epoch
            .replace_after_discard(request.expected_state_api_epoch)?;
        self.discards.set(self.discards.get() + 1);
        Ok(FinalChainExternalEvmDiscardReport {
            request_id: request.request_id,
            previous_state_api_epoch,
            state_api_epoch,
            period: request.period,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            marker_hash: request.marker_hash,
            prior_state: request.prior_state,
            committed_state: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn load_system_transaction_facts(
        &self,
        _: &FinalChainSystemTransactionFactsRequest,
    ) -> Result<FinalChainSystemTransactionPlanFact> {
        bail!("recovery cannot select system transactions")
    }
    fn execute_transactions(
        &self,
        _: &FinalChainEvmExecutionRequest,
    ) -> Result<FinalChainEvmExecutionReport> {
        bail!("recovery cannot reexecute transactions")
    }
    fn distribute_rewards(
        &self,
        _: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport> {
        bail!("recovery cannot reexecute rewards")
    }
    fn commit_staged_state(
        &self,
        _: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult> {
        bail!("recovery cannot commit a new concrete generation")
    }
}

/// Creates the owned pair once; subsequent attempts always reopen both owners.
fn initialize_pair(root: &Path, fixture: &Value) -> Result<()> {
    std::fs::create_dir(root)?;
    let (application, chain) = open_chain(&root.join("application"), fixture)?;
    let concrete = fresh_concrete(&root.join("state_db"), &chain, fixture)?;
    drop(concrete);
    drop(chain);
    drop(application);
    Ok(())
}

fn hydrate_genesis(chain: &mut FinalChain, state_path: &Path, fixture: &Value) -> Result<()> {
    let current =
        ConcreteStateLifecycle::inspect_existing(state_path, chain.concrete_chain_identity()?)?;
    let reader = ConcreteStateReader::open_historical_read_only(
        state_path,
        current.committed,
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: fixed(&fixture["genesis"]["root"]),
        },
    )?;
    chain.hydrate_concrete_genesis_accounts(&reader)?;
    Ok(())
}

/// Executes a genuine mixed period, optionally losing control at the named
/// callback boundary. No expected root is passed into the execution result.
fn run_period(root: &Path, fixture: &Value, index: usize, fault: Fault) -> Result<()> {
    let period = &fixture["periods"][index];
    let number = number(&period["number"]);
    let state_path = root.join("state_db");
    let (application, mut chain) = open_chain(&root.join("application"), fixture)?;
    let identity = chain.concrete_chain_identity()?;
    ensure!(
        application
            .final_chain()
            .external_evm_pending_publication_raw()?
            .is_none()
    );
    hydrate_genesis(&mut chain, &state_path, fixture)?;
    let prior = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    let concrete = ConcreteStateLifecycle::open(&state_path, identity, prior.committed)?;
    let adapter = FaultAdapter {
        inner: Adapter {
            chain: &chain,
            application: &application,
            concrete: RefCell::new(Some(concrete)),
            staged: RefCell::new(None),
            native: RefCell::new(None),
            native_context: RefCell::new(None),
            state_api_epoch: StateApiEpoch::new(),
            fixture: period,
            state_api_epoch: StateApiEpoch::new(),
        },
        fault,
        executions: Cell::new(0),
        rewards: Cell::new(0),
        commits: Cell::new(0),
        original_projection_hash: Cell::new(None),
    };
    initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &adapter.inner)?;
    let execution_request = request(fixture, period)?;
    chain.ensure_period_data(number.into(), &period_data(&execution_request))?;
    let outcome = execute_final_chain_application_task(
        &chain,
        execution_request,
        FinalChainProposalPeriodDagLevelUpdate::default(),
        false,
        fixed(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"]),
        &adapter,
    );
    ensure!(
        adapter.executions.get() == 1 && adapter.rewards.get() == 1 && adapter.commits.get() == 1,
        "period {number}: execution/reward/commit counts {}/{}/{}; result {:?}",
        adapter.executions.get(),
        adapter.rewards.get(),
        adapter.commits.get(),
        outcome.as_ref().err()
    );
    if fault == Fault::None {
        let report = outcome?;
        ensure!(report.period.as_u64() == number);
        ensure!(chain.last_block_number_typed()?.as_u64() == number);
        verify_receipts(&application, period)?;
    } else {
        let error = outcome.expect_err("injected interruption must require recovery");
        ensure!(
            format!("{error:#}").contains("mixed fixture interrupted"),
            "unexpected interruption: {error:#}"
        );
        ensure!(chain.last_block_number_typed()?.as_u64() == number - 1);
        ensure!(
            application
                .final_chain()
                .block_header_raw(number)?
                .is_none()
        );
        for tx in period["transactions"].as_array().unwrap() {
            ensure!(
                application
                    .final_chain()
                    .receipt_by_trx_hash(H256(fixed(&tx["hash"])))?
                    .is_none()
            );
        }
        ensure!(
            application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_some()
        );
    }
    drop(adapter);
    drop(chain);
    drop(application);
    let observed = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    let committed = fault != Fault::BeforeCommit;
    ensure!(observed.generation == prior.generation + u64::from(committed));
    if committed {
        ensure!(observed.committed.period.as_u64() == number);
        ensure!(observed.committed.state_root == fixed(&period["final"]["root"]));
        ensure!(observed.pending_marker_rlp.is_empty());
        verify_physical_rows(&state_path, &period["final"]["rows"])?;
    } else {
        ensure!(
            observed.committed == prior.committed
                && observed.catalog_rlp == prior.catalog_rlp
                && observed.provenance_rlp == prior.provenance_rlp
        );
        ensure!(!observed.pending_marker_rlp.is_empty());
        verify_physical_rows(&state_path, &fixture["periods"][index - 1]["final"]["rows"])?;
    }
    Ok(())
}

/// Reconciliation may publish the existing durable result or discard its marker;
/// it has no execution/native/reward capability and is idempotent across repeats.
fn recover_period(root: &Path, fixture: &Value, index: usize, committed: bool) -> Result<()> {
    let period = &fixture["periods"][index];
    let period_number = number(&period["number"]);
    let state_path = root.join("state_db");
    let (application, chain) = open_chain(&root.join("application"), fixture)?;
    let identity = chain.concrete_chain_identity()?;
    let before = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    let recovery = RecoveryLeaf {
        path: &state_path,
        chain_id: identity,
        discards: Cell::new(0),
        state_api_epoch: StateApiEpoch::new(),
    };
    let recovered =
        initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &recovery)?;
    ensure!(recovered.error_code.is_empty());
    ensure!(chain.last_block_number_typed()?.as_u64() == period_number - u64::from(!committed));
    ensure!(recovery.discards.get() == usize::from(!committed));
    ensure!(
        application
            .final_chain()
            .external_evm_pending_publication_raw()?
            .is_none()
    );
    let after = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    ensure!(after.identity == before.identity);
    ensure!(
        after.pending_marker_rlp.is_empty()
            && after.committed == before.committed
            && after.generation == before.generation
            && after.catalog_rlp == before.catalog_rlp
            && after.provenance_rlp == before.provenance_rlp
    );
    if committed {
        verify_receipts(&application, period)?;
    }
    let header = application.final_chain().block_header_raw(period_number)?;
    let counts = chain.execution_status()?;
    let repeated = recover_final_chain_application_state(&chain, &recovery)?;
    ensure!(
        repeated.status == FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
            && repeated.error_code.is_empty()
    );
    ensure!(
        application.final_chain().block_header_raw(period_number)? == header
            && chain.execution_status()? == counts
    );
    ensure!(recovery.discards.get() == usize::from(!committed));
    let repeated_observation = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    ensure!(repeated_observation == after);
    ensure!(
        application
            .final_chain()
            .external_evm_pending_publication_raw()?
            .is_none()
    );
    if committed {
        verify_receipts(&application, period)?;
    }
    Ok(())
}

/// Consensus-visible bytes and deterministic concrete facts can be compared
/// across independently created pairs. Database IDs/provenance are compared
/// exactly within each recovery run, since exclusive creation assigns new IDs.
#[derive(Debug, Eq, PartialEq)]
struct FinalFacts {
    stored_headers: Vec<Vec<u8>>,
    public_headers: Vec<Vec<u8>>,
    receipts: Vec<Vec<u8>>,
    dag_count: u64,
    transaction_count: u64,
    committed: ConcreteStateIdentity,
    catalog: Vec<u8>,
    generation: u64,
}
fn final_facts(root: &Path, fixture: &Value) -> Result<FinalFacts> {
    let (application, chain) = open_chain(&root.join("application"), fixture)?;
    let observed = ConcreteStateLifecycle::inspect_existing(
        root.join("state_db"),
        chain.concrete_chain_identity()?,
    )?;
    ensure!(
        observed.pending_marker_rlp.is_empty()
            && application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none()
    );
    let mut stored_headers = Vec::new();
    let mut public_headers = Vec::new();
    let mut receipts = Vec::new();
    for period in fixture["periods"].as_array().unwrap() {
        let height = number(&period["number"]);
        stored_headers.push(
            application
                .final_chain()
                .block_header_raw(height)?
                .context("missing final header")?,
        );
        public_headers.push(
            chain
                .block_header(height.into())?
                .context("missing materialized header")?,
        );
        for tx in period["transactions"].as_array().unwrap() {
            receipts.push(
                application
                    .final_chain()
                    .receipt_by_trx_hash(H256(fixed(&tx["hash"])))?
                    .context("missing final receipt")?,
            );
        }
    }
    let counters = chain.execution_status()?;
    Ok(FinalFacts {
        stored_headers,
        public_headers,
        receipts,
        dag_count: counters.executed_dag_block_count,
        transaction_count: counters.executed_transaction_count,
        committed: observed.committed,
        catalog: observed.catalog_rlp,
        generation: observed.generation,
    })
}
fn uninterrupted_facts(fixture: &Value, label: &str) -> Result<FinalFacts> {
    let root = std::env::temp_dir().join(format!(
        "rustaxa-mixed-{label}-baseline-{}",
        std::process::id()
    ));
    initialize_pair(&root, fixture)?;
    for index in 0..4 {
        run_period(&root, fixture, index, Fault::None)?;
    }
    let facts = final_facts(&root, fixture)?;
    std::fs::remove_dir_all(root)?;
    Ok(facts)
}

#[test]
fn mixed_custody_commit_interruptions_recover_and_continue_exactly() -> Result<()> {
    let fixture = load_fixture()?;
    let uninterrupted = uninterrupted_facts(&fixture, "recovery")?;
    for fault in [Fault::BeforeCommit, Fault::AfterCommit] {
        let root = std::env::temp_dir().join(format!(
            "rustaxa-mixed-recovery-{}-{fault:?}",
            std::process::id()
        ));
        initialize_pair(&root, &fixture)?;
        run_period(&root, &fixture, 0, Fault::None)?;
        run_period(&root, &fixture, 1, fault)?;
        recover_period(&root, &fixture, 1, fault == Fault::AfterCommit)?;
        if fault == Fault::BeforeCommit {
            run_period(&root, &fixture, 1, Fault::None)?;
        }
        run_period(&root, &fixture, 2, Fault::None)?;
        run_period(&root, &fixture, 3, Fault::None)?;
        ensure!(
            final_facts(&root, &fixture)? == uninterrupted,
            "recovered/retried mixed output differs from uninterrupted run"
        );
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

/// A forged executor report is rejected before either committed descriptor
/// advances. Each case then reopens and executes the real period to completion.
#[test]
fn mixed_invalid_reports_cannot_contaminate_retry_or_publication() -> Result<()> {
    let fixture = load_fixture()?;
    let uninterrupted = uninterrupted_facts(&fixture, "rejection")?;
    for fault in [
        Fault::StaleNativeValue,
        Fault::MissingCatalog,
        Fault::InvocationOrder,
        Fault::FailureDisposition,
        Fault::PriorIdentity,
        Fault::PreparedIntent,
        Fault::RewardAccount,
    ] {
        let root = std::env::temp_dir().join(format!(
            "rustaxa-mixed-rejection-{}-{fault:?}",
            std::process::id()
        ));
        initialize_pair(&root, &fixture)?;
        run_period(&root, &fixture, 0, Fault::None)?;
        let state_path = root.join("state_db");
        let (application, mut chain) = open_chain(&root.join("application"), &fixture)?;
        hydrate_genesis(&mut chain, &state_path, &fixture)?;
        let identity = chain.concrete_chain_identity()?;
        let prior = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
        let counters = chain.execution_status()?;
        let old_header = application.final_chain().block_header_raw(1)?;
        let concrete = ConcreteStateLifecycle::open(&state_path, identity, prior.committed)?;
        let period = &fixture["periods"][1];
        let adapter = FaultAdapter {
            inner: Adapter {
                chain: &chain,
                application: &application,
                concrete: RefCell::new(Some(concrete)),
                staged: RefCell::new(None),
                native: RefCell::new(None),
                native_context: RefCell::new(None),
                state_api_epoch: StateApiEpoch::new(),
                fixture: period,
                state_api_epoch: StateApiEpoch::new(),
            },
            fault,
            executions: Cell::new(0),
            rewards: Cell::new(0),
            commits: Cell::new(0),
            original_projection_hash: Cell::new(None),
        };
        initialize_and_recover_final_chain_application_state_at_joint_startup(
            &chain,
            &adapter.inner,
        )?;
        let execution_request = request(&fixture, period)?;
        chain.ensure_period_data(2_u64.into(), &period_data(&execution_request))?;
        let error = execute_final_chain_application_task(
            &chain,
            execution_request,
            FinalChainProposalPeriodDagLevelUpdate::default(),
            false,
            fixed(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"]),
            &adapter,
        )
        .expect_err("forged mixed report must be rejected");
        let message = format!("{error:#}");
        // Exact rejection boundaries ensure each forgery reaches its intended
        // semantic or lifecycle check after unrelated bindings are updated.
        let expected = match fault {
            Fault::StaleNativeValue => "FINAL_CHAIN_NATIVE_CONTEXT_FINAL_STORAGE_MISMATCH",
            Fault::MissingCatalog => "concrete storage catalog is not monotonic",
            Fault::InvocationOrder => "concrete invocations are unordered or duplicated",
            Fault::FailureDisposition => "FINAL_CHAIN_NATIVE_CONTEXT_DISPOSITION_MISMATCH",
            Fault::PriorIdentity => "FINAL_CHAIN_EXTERNAL_EVM_PRIOR_DESCRIPTOR_MISMATCH",
            Fault::PreparedIntent => "FINAL_CHAIN_CONCRETE_PENDING_INTENT_MISMATCH",
            Fault::RewardAccount => "FINAL_CHAIN_NATIVE_REWARDS_CONTEXT_ACCOUNT_MISMATCH",
            _ => unreachable!(),
        };
        ensure!(
            message.contains(expected),
            "{fault:?} reached unexpected rejection: {message}"
        );
        ensure!(adapter.executions.get() == usize::from(fault != Fault::PriorIdentity));
        ensure!(adapter.rewards.get() == usize::from(fault != Fault::PriorIdentity));
        ensure!(
            adapter.commits.get()
                == usize::from(matches!(
                    fault,
                    Fault::PreparedIntent | Fault::MissingCatalog
                ))
        );
        ensure!(
            chain.last_block_number_typed()?.as_u64() == 1 && chain.execution_status()? == counters
        );
        ensure!(
            application.final_chain().block_header_raw(1)? == old_header
                && application.final_chain().block_header_raw(2)?.is_none()
        );
        for tx in period["transactions"].as_array().unwrap() {
            ensure!(
                application
                    .final_chain()
                    .receipt_by_trx_hash(H256(fixed(&tx["hash"])))?
                    .is_none()
            );
        }
        ensure!(
            application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_some()
                == (fault == Fault::MissingCatalog)
        );
        drop(adapter);
        drop(chain);
        drop(application);
        if fault == Fault::MissingCatalog {
            recover_period(&root, &fixture, 1, false)?;
        }
        let after = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
        ensure!(
            after.committed == prior.committed
                && after.generation == prior.generation
                && after.provenance_rlp == prior.provenance_rlp
                && after.catalog_rlp == prior.catalog_rlp
                && after.pending_marker_rlp.is_empty()
        );
        verify_physical_rows(&state_path, &fixture["periods"][0]["final"]["rows"])?;
        run_period(&root, &fixture, 1, Fault::None)?;
        run_period(&root, &fixture, 2, Fault::None)?;
        run_period(&root, &fixture, 3, Fault::None)?;
        ensure!(
            final_facts(&root, &fixture)? == uninterrupted,
            "recovered/retried mixed output differs from uninterrupted run"
        );
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}
