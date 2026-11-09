//! Deterministic mixed-state publication interruption and rejection checks.
//!
//! Mounted by the mixed-period integration test, this module reuses its actual
//! execution adapter and adds faults only at application/commit report boundaries.
//! Every database is exclusively created by this finite synthetic fixture.

use super::mainnet_era::duration::DurationPhaseCapture;
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Fault {
    None,
    BeforeCommit,
    AfterCommit,
    StaleNativeValue,
    MissingCatalog,
    InvocationOrder,
    FailureDisposition,
    UnchangedAuthorityProjection,
    MissingAuthority,
    PriorIdentity,
    PreparedIntent,
    RewardAccount,
}

pub(super) struct FaultAdapter<'a> {
    pub(super) inner: Adapter<'a>,
    pub(super) fault: Fault,
    pub(super) executions: Cell<usize>,
    pub(super) rewards: Cell<usize>,
    pub(super) commits: Cell<usize>,
    pub(super) original_projection_hash: Cell<Option<[u8; 32]>>,
    pub(super) recovery_capture:
        Option<std::rc::Rc<RefCell<super::mainnet_era::duration::CapturedPhases>>>,
    pub(super) recovery_journal_cap: usize,
}
type LifecycleAuthority =
    rustaxa_consensus::native_projection_context::FinalChainLifecycleAuthority;

fn synthetic_projection_fault(fault: Fault) -> bool {
    matches!(
        fault,
        Fault::StaleNativeValue
            | Fault::MissingCatalog
            | Fault::InvocationOrder
            | Fault::FailureDisposition
    )
}

/// Rebinds only the digest for four deliberate synthetic projection faults.
fn test_only_authority_for_mutated_projection(
    fault: Fault,
    mut authority: LifecycleAuthority,
    projection: &FinalChainConcreteStateProjection,
    requested_hash: [u8; 32],
) -> Result<LifecycleAuthority> {
    ensure!(
        synthetic_projection_fault(fault),
        "unsupported synthetic authority transformation"
    );
    let staged_hash = concrete_state_bytes_digest(&encode_concrete_state_projection(projection));
    ensure!(
        staged_hash == requested_hash,
        "synthetic staged projection hash mismatch"
    );
    authority.projection_hash = staged_hash;
    Ok(authority)
}

fn forward_lifecycle_authority<P: ConsensusExecutionPort>(
    inner: &P,
    fault: Fault,
    original_hash: Option<[u8; 32]>,
    projection: Option<&FinalChainConcreteStateProjection>,
    request_id: [u8; 32],
    requested_hash: [u8; 32],
) -> Result<Option<LifecycleAuthority>> {
    let original = original_hash.unwrap_or(requested_hash);
    let authority = inner.final_chain_native_lifecycle_authority(request_id, original)?;
    if original_hash.is_none() {
        ensure!(
            !synthetic_projection_fault(fault),
            "synthetic projection identity missing"
        );
        return Ok(authority);
    }
    if fault == Fault::UnchangedAuthorityProjection {
        return Ok(authority);
    }
    let authority = authority.context("synthetic original authority unavailable")?;
    let projection = projection.context("synthetic staged projection unavailable")?;
    Ok(Some(test_only_authority_for_mutated_projection(
        fault,
        authority,
        projection,
        requested_hash,
    )?))
}

thread_local! {
    static WRAPPER_AUTHORITY_AUDIT: RefCell<Option<(bool, bool)>> = const { RefCell::new(None) };
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
        if self.recovery_capture.is_none() {
            self.executions.set(self.executions.get() + 1);
            return self.inner.execute_final_chain_transactions(request);
        }
        self.executions.set(self.executions.get() + 1);
        duration_begin_arm()?;
        rustaxa_evm::journal::duration_journal_arm(self.recovery_journal_cap)
            .map_err(anyhow::Error::msg)?;
        let execution = self.inner.execute_final_chain_transactions(request);
        let journal = rustaxa_evm::journal::duration_journal_take().map_err(anyhow::Error::msg);
        let begin = duration_begin_take()?;
        let report = execution?;
        let capture = self.recovery_capture.as_ref().expect("recovery capture");
        capture.borrow_mut().journal(false, journal?)?;
        capture.borrow_mut().begin(begin)?;
        capture.borrow_mut().transactions(&report, &self.inner)?;
        Ok(report)
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
        if self.recovery_capture.is_some() {
            rustaxa_evm::journal::duration_journal_arm(self.recovery_journal_cap)
                .map_err(anyhow::Error::msg)?;
        }
        let execution = self
            .inner
            .distribute_final_chain_rewards_with_native_plan(request, plan);
        let journal = if self.recovery_capture.is_some() {
            Some(rustaxa_evm::journal::duration_journal_take().map_err(anyhow::Error::msg))
        } else {
            None
        };
        let mut report = execution?;
        if let Some(capture) = &self.recovery_capture {
            capture
                .borrow_mut()
                .journal(true, journal.context("recovery rewards journal")??)?;
            capture.borrow_mut().rewards(&report, &self.inner)?;
        }
        if matches!(
            self.fault,
            Fault::StaleNativeValue
                | Fault::MissingCatalog
                | Fault::InvocationOrder
                | Fault::FailureDisposition
                | Fault::UnchangedAuthorityProjection
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
                Fault::StaleNativeValue | Fault::UnchangedAuthorityProjection => {
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
    fn final_chain_native_lifecycle_authority(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<LifecycleAuthority>> {
        if self.fault == Fault::MissingAuthority {
            return Ok(None);
        }
        let original = self.original_projection_hash.get();
        let staged = self.inner.staged.borrow();
        let projection = staged.as_ref().map(|value| &value.projection);
        let authority = forward_lifecycle_authority(
            &self.inner,
            self.fault,
            original,
            projection,
            request_id,
            projection_hash,
        )?;
        if self.fault == Fault::None {
            WRAPPER_AUTHORITY_AUDIT.with(|audit| {
                if let (Some(flags), Some(value)) = (audit.borrow_mut().as_mut(), authority.as_ref()) {
                    for receipt in &value.receipts {
                        let any = receipt.events.iter().any(|event| matches!(event, rustaxa_consensus::native_projection_context::FinalChainLifecycleEvent::NativeStarted { .. }));
                        flags.0 |= any;
                        flags.1 |= !any && matches!(receipt.events.last(), Some(rustaxa_consensus::native_projection_context::FinalChainLifecycleEvent::TransactionEnd { observations: 0 }));
                    }
                }
            });
        }
        Ok(authority)
    }

    fn final_chain_native_delayed_view(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<
        Option<
            std::rc::Rc<rustaxa_consensus::native_session::frozen_eligibility::FrozenEligibility>,
        >,
    > {
        self.inner.final_chain_native_delayed_view(
            request_id,
            self.original_projection_hash
                .get()
                .unwrap_or(projection_hash),
        )
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
        if let Some(capture) = &self.recovery_capture {
            capture.borrow_mut().native_context(
                context
                    .as_ref()
                    .context("recovery native context missing")?,
            )?;
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
        if request.period.as_u64() == 5 {
            let pending = self
                .inner
                .application
                .final_chain()
                .external_evm_pending_publication_raw()?;
            eprintln!(
                "RECOVERY_INTENT {}",
                serde_json::json!({
                    "period": request.period.as_u64(), "fault": format!("{:?}", self.fault),
                    "epoch": request.state_api_epoch, "request_id": hex::encode(request.request_id),
                    "projection_hash": hex::encode(request.concrete_projection_hash),
                    "post_transaction_root": hex::encode(request.post_transaction_state_root),
                    "post_rewards_root": hex::encode(request.post_rewards_state_root),
                    "pending_keccak256": pending.as_ref().map(|raw| hex::encode(keccak256(raw))),
                    "receipt_publication": "NOT_OBSERVED_AT_COMMIT_CALLBACK",
                })
            );
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
        if let Some(capture) = &self.recovery_capture {
            capture.borrow_mut().committed(&result)?;
        }
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
pub(super) struct RecoveryLeaf<'a> {
    pub(super) path: &'a Path,
    pub(super) chain_id: [u8; 32],
    pub(super) discards: Cell<usize>,
    pub(super) state_api_epoch: StateApiEpoch,
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
pub(super) fn run_period(root: &Path, fixture: &Value, index: usize, fault: Fault) -> Result<()> {
    run_period_observing(root, fixture, index, fault, None)
}
pub(super) fn run_period_observing(
    root: &Path,
    fixture: &Value,
    index: usize,
    fault: Fault,
    ordinary: Option<OrdinaryObserver>,
) -> Result<()> {
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
    if fixture["profile"] != "synthetic_mainnet_era" {
        hydrate_genesis(&mut chain, &state_path, fixture)?;
    }
    let prior = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    if let Some(observer) = &ordinary {
        observer.borrow_mut().begin(
            &state_path,
            prior.identity,
            prior.committed,
            number,
            prior.generation,
        )?;
    }
    let concrete = ConcreteStateLifecycle::open(&state_path, identity, prior.committed)?;
    let physical_receipt = std::rc::Rc::new(RefCell::new(None));
    let adapter = FaultAdapter {
        inner: Adapter {
            chain: &chain,
            application: &application,
            concrete: RefCell::new(Some(concrete)),
            staged: RefCell::new(None),
            native: RefCell::new(None),
            native_context: RefCell::new(None),
            physical_receipt: Some(physical_receipt.clone()),
            state_api_epoch: StateApiEpoch::new(),
            ordinary_observer: ordinary.clone(),
            fixture: period,
            account_node_publication: if fixture["oracle_lifecycle"] == "public_batched" {
                AccountNodePublication::FinalPeriod {
                    limits: rustaxa_storage::ConcreteStorageInventoryLimits {
                        max_nodes: 1_000_000,
                        max_leaves: 100_000,
                        max_value_bytes: 32_000_000,
                    },
                }
            } else {
                AccountNodePublication::ObserverHistory
            },
        },
        fault,
        executions: Cell::new(0),
        rewards: Cell::new(0),
        commits: Cell::new(0),
        original_projection_hash: Cell::new(None),
        recovery_capture: None,
        recovery_journal_cap: 0,
    };
    initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &adapter.inner)?;
    super::mainnet_era::bind_legacy_scheduler_order(&chain, fixture, period)?;
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
        ensure!(
            application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none()
        );
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
    {
        let receipt = physical_receipt.borrow();
        if fault == Fault::BeforeCommit {
            ensure!(
                receipt.is_none(),
                "pre-commit interruption produced a receipt"
            );
        } else {
            let receipt = receipt
                .as_ref()
                .context("successful concrete commit lost its receipt")?;
            ensure!(
                receipt.identity() == prior.identity
                    && receipt.prior() == prior.committed
                    && receipt.next().period.as_u64() == number
                    && receipt.generation() == prior.generation + 1,
                "interrupted commit receipt identity"
            );
        }
    }
    drop(adapter);
    drop(chain);
    drop(application);
    let observed = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    if fault == Fault::None {
        ensure!(observed.pending_marker_rlp.is_empty());
        if let Some(observer) = &ordinary {
            observer.borrow_mut().promote(
                &state_path,
                observed.identity,
                observed.committed,
                observed.generation,
            )?;
        }
    }
    let committed = fault != Fault::BeforeCommit;
    ensure!(observed.generation == prior.generation + u64::from(committed));
    if committed {
        ensure!(observed.committed.period.as_u64() == number);
        ensure!(observed.committed.state_root == fixed(&period["final"]["root"]));
        ensure!(observed.pending_marker_rlp.is_empty());
        if fixture["profile"] == "synthetic_mainnet_era" {
            super::mainnet_era::verify_committed_phase(
                &state_path,
                observed.committed,
                fixture,
                &period["final"],
            )?;
        } else {
            verify_physical_rows(&state_path, &period["final"]["rows"])?;
        }
    } else {
        ensure!(
            observed.committed == prior.committed
                && observed.catalog_rlp == prior.catalog_rlp
                && observed.provenance_rlp == prior.provenance_rlp
        );
        ensure!(!observed.pending_marker_rlp.is_empty());
        let previous = &fixture["periods"][index - 1]["final"];
        if fixture["profile"] == "synthetic_mainnet_era" {
            super::mainnet_era::verify_committed_phase(
                &state_path,
                observed.committed,
                fixture,
                previous,
            )?;
        } else {
            verify_physical_rows(&state_path, &previous["rows"])?;
        }
    }
    Ok(())
}

/// Reconciliation may publish the existing durable result or discard its marker;
/// it has no execution/native/reward capability and is idempotent across repeats.
pub(super) fn recover_period(
    root: &Path,
    fixture: &Value,
    index: usize,
    committed: bool,
) -> Result<()> {
    recover_period_observing(root, fixture, index, committed, None, || Ok(()))
}

fn recover_period_observing(
    root: &Path,
    fixture: &Value,
    index: usize,
    committed: bool,
    ordinary: Option<OrdinaryObserver>,
    mut observe: impl FnMut() -> Result<()>,
) -> Result<()> {
    let period = &fixture["periods"][index];
    let period_number = number(&period["number"]);
    let state_path = root.join("state_db");
    let (application, chain) = open_chain(&root.join("application"), fixture)?;
    let identity = chain.concrete_chain_identity()?;
    let before = ConcreteStateLifecycle::inspect_existing(&state_path, identity)?;
    let pending = application
        .final_chain()
        .external_evm_pending_publication_raw()?;
    eprintln!(
        "RECOVERY_REQUEST {}",
        serde_json::json!({
            "phase":"first", "root":root.display().to_string(), "period":period_number,
            "committed":committed, "pending_keccak256":pending.as_ref().map(|raw| hex::encode(keccak256(raw))),
            "concrete_period":before.committed.period.as_u64(), "concrete_root":hex::encode(before.committed.state_root),
        })
    );
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
    if let Some(observer) = &ordinary {
        if committed {
            observer.borrow_mut().promote(
                &state_path,
                after.identity,
                after.committed,
                after.generation,
            )?;
        } else {
            observer.borrow_mut().discard(
                &state_path,
                after.identity,
                after.committed,
                after.generation,
            )?;
        }
    }
    observe()?;
    eprintln!(
        "RECOVERY_RESULT {}",
        serde_json::json!({"phase":"first", "period":period_number, "error_code":recovered.error_code, "discards":recovery.discards.get()})
    );
    let header = application.final_chain().block_header_raw(period_number)?;
    let counts = chain.execution_status()?;
    eprintln!(
        "RECOVERY_REQUEST {}",
        serde_json::json!({"phase":"repeat", "root":root.display().to_string(), "period":period_number})
    );
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
    eprintln!(
        "RECOVERY_RESULT {}",
        serde_json::json!({"phase":"repeat", "period":period_number, "status":format!("{:?}", repeated.status), "error_code":repeated.error_code, "discards":recovery.discards.get()})
    );
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
    if let Some(observer) = &ordinary {
        observer.borrow().verify_committed(
            &state_path,
            repeated_observation.identity,
            repeated_observation.committed,
            repeated_observation.generation,
        )?;
    }
    observe()?;
    Ok(())
}

fn validate_distribution_membership(scenario: &str, period: u64, rows: &[Value]) -> Result<()> {
    let members = rows
        .iter()
        .map(|row| row.get("period").and_then(Value::as_u64))
        .collect::<Option<BTreeSet<_>>>();
    if scenario == "magnolia" {
        ensure!(
            period == 5_730_000 && rows.len() == 1 && members == Some(BTreeSet::from([5_730_000])),
            "Magnolia first distribution must contain its single activation-period row"
        );
    } else {
        let first = period
            .checked_sub(99)
            .context("100-row distribution period")?;
        ensure!(
            rows.len() == 100 && members == Some((first..=period).collect()),
            "recovery must interrupt every row of the qualified 100-row distribution"
        );
    }
    Ok(())
}

#[test]
fn distribution_membership_distinguishes_magnolia_first_batch_from_full_batches() {
    let row = |period: u64| serde_json::json!({"period": period});
    let boundary = 5_730_000;
    assert!(validate_distribution_membership("magnolia", boundary, &[row(boundary)]).is_ok());
    assert!(validate_distribution_membership("magnolia", boundary, &[]).is_err());
    assert!(
        validate_distribution_membership("magnolia", boundary, &[row(boundary), row(boundary)])
            .is_err()
    );
    assert!(validate_distribution_membership("magnolia", boundary, &[row(boundary - 1)]).is_err());
    assert!(
        validate_distribution_membership("magnolia", boundary + 1, &[row(boundary + 1)]).is_err()
    );

    let end = 25_706_900;
    let full = (end - 99..=end).map(row).collect::<Vec<_>>();
    assert!(validate_distribution_membership("mainnet_106", end, &full).is_ok());
    assert!(validate_distribution_membership("mainnet_106", end, &full[..99]).is_err());
    let mut extra = full.clone();
    extra.push(row(end + 1));
    assert!(validate_distribution_membership("mainnet_106", end, &extra).is_err());
    let mut duplicate = full.clone();
    duplicate[0] = row(end);
    assert!(validate_distribution_membership("mainnet_106", end, &duplicate).is_err());
    let mut wrong = full.clone();
    wrong[0] = row(end - 100);
    assert!(validate_distribution_membership("mainnet_106", end, &wrong).is_err());
}

/// Repeats the full mainnet workload from an independently bootstrapped pair,
/// interrupting the first distribution at an existing commit callback.
///
/// The caller owns bootstrap and the completed uninterrupted baseline. This
/// runner accepts only the two existing commit faults and explicitly qualified
/// era windows. It executes the exact prefix before the first selected
/// distribution and retains that committed reader across interruption,
/// repeated recovery, retry (BeforeCommit only), and the remaining workload.
/// RecoveryLeaf cannot execute transactions or rewards; run_period checks that
/// the fault reached exactly one execution, rewards, and commit callback. The
/// inner Adapter uses mainnet_era::compare_distribution_writes for the terminal
/// native transcript, including the exact pre-Aspen order. The membership check
/// here permits that legacy order but no missing or duplicate source period.
/// Magnolia's first activation distribution contains one activation-period row;
/// other qualified distribution windows require all 100 source periods.
/// No expected root enters execution or publication.
///
/// Both inputs must be distinct existing synthetic pairs. The fresh pair must
/// still be at the declared retained seed coordinate, with no pending marker.
/// All headers, receipts, counters, final concrete identity/catalog/generation
/// must equal the uninterrupted result. Database IDs remain pair-local.
/// Artifacts are retained; this helper never creates or removes either pair.
pub(super) fn verify_mainnet_recovery(
    fixture: &Value,
    baseline_root: &Path,
    fresh_bootstrapped_root: &Path,
    fault: Fault,
) -> Result<()> {
    verify_mainnet_recovery_observing(fixture, baseline_root, fresh_bootstrapped_root, fault, None)
}
pub(super) fn verify_mainnet_recovery_observing(
    fixture: &Value,
    baseline_root: &Path,
    fresh_bootstrapped_root: &Path,
    fault: Fault,
    ordinary: Option<OrdinaryObserver>,
) -> Result<()> {
    ensure!(fixture["profile"] == "synthetic_mainnet_era");
    ensure!((is_second_distribution_scenario(fixture)) == ordinary.is_some());
    let (first_period, last_period, distribution_period) = match string(&fixture["scenario"]) {
        "mainnet_106" => (25_706_890u64, 25_707_000u64, 25_706_900u64),
        "mainnet_second_distribution_followthrough"
        | "mainnet_second_distribution_reopen"
        | "mainnet_second_distribution_followthrough_recovery" => {
            (25_706_890, 25_707_005, 25_707_000)
        }
        "magnolia" => (5729995, 5730001, 5730000),
        "magnolia_first_full_distribution" => (5729995, 5730101, 5730100),
        "aspen_one" => (8_117_995, 8_118_001, 8_118_000),
        "aspen_two" => (8_571_995, 8_572_001, 8_572_000),
        "soleirolia" => (17_379_995, 17_380_001, 17_380_000),
        "phalaenopsis" => (6_942_995, 6_943_005, 6_943_000),
        "claim_all_legacy_batch_correction" => (7_599_995, 7_600_005, 7_600_000),
        "cornus" => (15_609_995, 15_610_005, 15_610_000),
        "ficus" => (11_615_995, 11_616_005, 11_616_000),
        "cornus_v2_maturity" => (15_773_564, 15_773_574, 15_773_569),
        _ => bail!("unqualified recovery scenario"),
    };
    ensure!(matches!(fault, Fault::BeforeCommit | Fault::AfterCommit));
    ensure!(
        std::fs::canonicalize(baseline_root)? != std::fs::canonicalize(fresh_bootstrapped_root)?,
        "recovery pair must differ from the uninterrupted baseline"
    );
    let periods = fixture["periods"]
        .as_array()
        .context("complete mainnet recovery periods")?;
    ensure!(
        periods.len() as u64 == last_period - first_period + 1
            && number(&periods[0]["number"]) == first_period
            && number(&periods.last().unwrap()["number"]) == last_period,
        "recovery requires the complete explicitly qualified era window"
    );
    ensure!(
        periods.windows(2).all(|pair| {
            number(&pair[0]["number"]).checked_add(1) == Some(number(&pair[1]["number"]))
        }),
        "mainnet recovery periods must be consecutive"
    );
    let distribution_index = periods
        .iter()
        .position(|period| number(&period["number"]) == distribution_period)
        .context("first mainnet distribution")?;
    ensure!(distribution_index > 0);
    let distributions = periods[distribution_index]["distributions"]
        .as_array()
        .context("actual first distribution invocations")?;
    if fixture["scenario"] == "cornus_v2_maturity" {
        ensure!(
            distributions.is_empty(),
            "V2 maturity fault is an actual confirmation period, not a reward boundary"
        );
        ensure!(
            !periods[distribution_index]["transactions"]
                .as_array()
                .context("actual V2 confirmation")?
                .is_empty()
        );
    } else {
        validate_distribution_membership(
            string(&fixture["scenario"]),
            distribution_period,
            distributions,
        )?;
    }
    if fixture["scenario"] == "mainnet_106" {
        ensure!(
            distributions
                .iter()
                .enumerate()
                .all(|(index, row)| { number(&row["period"]) == 25_706_801 + index as u64 }),
            "current-era distribution retains its exact chronological order"
        );
    }
    if is_second_distribution_scenario(fixture) {
        ensure!(
            distributions
                .iter()
                .enumerate()
                .all(|(i, row)| number(&row["period"]) == 25_706_901 + i as u64),
            "second distribution exact chronology"
        );
    }
    let expected = final_facts(baseline_root, fixture)?;
    let application_path = fresh_bootstrapped_root.join("application");
    let state_path = fresh_bootstrapped_root.join("state_db");
    let owner_identity = {
        let (application, chain) = open_chain(&application_path, fixture)?;
        let seed = ConcreteStateLifecycle::inspect_existing(
            &state_path,
            chain.concrete_chain_identity()?,
        )?;
        ensure!(
            seed.committed.period.as_u64() == number(&fixture["seed"]["period"])
                && seed.committed.state_root == fixed(&fixture["seed"]["root"])
                && seed.pending_marker_rlp.is_empty()
                && chain.last_block_number_typed()? == seed.committed.period
                && application
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    .is_none(),
            "mainnet recovery requires a fresh, completely paired synthetic seed"
        );
        seed.identity
    };
    if fixture["scenario"] == "mainnet_second_distribution_followthrough_recovery" {
        let split = periods
            .iter()
            .position(|p| number(&p["number"]) == 25_706_906)
            .context("companion reopen after 25706905")?;
        ensure!(split < distribution_index);
        native_integration::run_segment_observing(
            fixture,
            &periods[..split],
            &application_path,
            &state_path,
            false,
            Fault::None,
            ordinary.clone(),
        )?;
        native_integration::run_segment_observing(
            fixture,
            &periods[split..distribution_index],
            &application_path,
            &state_path,
            false,
            Fault::None,
            ordinary.clone(),
        )?;
    } else {
        native_integration::run_segment_observing(
            fixture,
            &periods[..distribution_index],
            &application_path,
            &state_path,
            false,
            Fault::None,
            ordinary.clone(),
        )?;
    }
    let prior_period = &periods[distribution_index - 1];
    let prior_identity = ConcreteStateIdentity {
        period: number(&prior_period["number"]).into(),
        state_root: fixed(&prior_period["final"]["root"]),
    };
    let mut known = BTreeSet::new();
    for key in fixture["history_rows"][4]
        .as_object()
        .context("complete mainnet physical storage history inventory")?
        .keys()
    {
        let raw = hex::decode(key)?;
        ensure!(raw.len() == 40, "mainnet physical history key width");
        if u64::from_be_bytes(raw[32..].try_into()?) <= prior_identity.period.as_u64() {
            known.insert(raw[..32].try_into()?);
        }
    }
    let retained = native_integration::api::CompleteReader {
        inner: ConcreteStateReader::open_historical_read_only(
            &state_path,
            prior_identity,
            prior_identity,
        )?,
        known,
    };
    verify_complete_final_reader(&retained, &prior_period["final"])?;
    // Open five distinct historical handles BEFORE the faulted B2 execution.
    // Their actual immutable identities stay live through all reconciliation
    // and signed continuation. The existing prior reader remains independent.
    let mut followthrough_readers = Vec::new();
    let legacy_readers = fixture["scenario"] == "claim_all_legacy_batch_correction";
    let five_readers = is_second_distribution_scenario(fixture) || legacy_readers;
    if five_readers {
        let first_reader = if legacy_readers {
            7_599_995
        } else {
            25_706_995
        };
        for height in first_reader..=first_reader + 4 {
            let index = periods
                .iter()
                .position(|p| number(&p["number"]) == height)
                .context("pre-second retained period")?;
            let identity = ConcreteStateIdentity {
                period: height.into(),
                state_root: fixed(&periods[index]["final"]["root"]),
            };
            let mut known = BTreeSet::new();
            for key in fixture["history_rows"][4]
                .as_object()
                .context("complete followthrough inventory")?
                .keys()
            {
                let raw = hex::decode(key)?;
                ensure!(raw.len() == 40, "followthrough history key width");
                if u64::from_be_bytes(raw[32..].try_into()?) <= height {
                    known.insert(raw[..32].try_into()?);
                }
            }
            let reader = native_integration::api::CompleteReader {
                inner: ConcreteStateReader::open_historical_read_only(
                    &state_path,
                    prior_identity,
                    identity,
                )?,
                known,
            };
            verify_complete_final_reader(&reader, &periods[index]["final"])?;
            followthrough_readers.push((reader, index));
        }
        ensure!(
            followthrough_readers.len() == 5,
            "five independent pre-fault readers"
        );
    }
    let verify_five = || -> Result<()> {
        if five_readers {
            ensure!(
                followthrough_readers.len() == 5,
                "five live historical readers required"
            );
        }
        for (reader, index) in &followthrough_readers {
            let expected = ConcreteStateIdentity {
                period: number(&periods[*index]["number"]).into(),
                state_root: fixed(&periods[*index]["final"]["root"]),
            };
            ensure!(
                reader.identity() == expected,
                "retained historical identity changed"
            );
            verify_complete_final_reader(reader, &periods[*index]["final"])?;
            if let Some(observer) = &ordinary {
                let borrowed = observer.borrow();
                for (slot, value) in borrowed.historical(&state_path, owner_identity, expected)? {
                    ensure!(
                        reader.storage(slot.address, ConcreteStorageKey(slot.key))? == *value,
                        "historical ordinary value mismatch"
                    );
                }
            }
        }
        Ok(())
    };
    verify_five()?;
    run_period_observing(
        fresh_bootstrapped_root,
        fixture,
        distribution_index,
        fault,
        ordinary.clone(),
    )?;
    verify_complete_final_reader(&retained, &prior_period["final"])?;
    verify_five()?;
    // recover_period already repeats reconciliation with an execution-free
    // leaf and verifies unchanged descriptors, receipts and execution counters.
    if five_readers {
        recover_period_observing(
            fresh_bootstrapped_root,
            fixture,
            distribution_index,
            fault == Fault::AfterCommit,
            ordinary.clone(),
            &verify_five,
        )?;
    } else {
        recover_period(
            fresh_bootstrapped_root,
            fixture,
            distribution_index,
            fault == Fault::AfterCommit,
        )?;
    }
    verify_complete_final_reader(&retained, &prior_period["final"])?;
    verify_five()?;
    if fault == Fault::BeforeCommit {
        run_period_observing(
            fresh_bootstrapped_root,
            fixture,
            distribution_index,
            Fault::None,
            ordinary.clone(),
        )?;
        verify_five()?;
    }
    // Reopen and reconcile the now-published distribution again, including
    // the internally repeated no-op reconciliation. No second execution occurs.
    if five_readers {
        recover_period_observing(
            fresh_bootstrapped_root,
            fixture,
            distribution_index,
            true,
            ordinary.clone(),
            &verify_five,
        )?;
    } else {
        recover_period(fresh_bootstrapped_root, fixture, distribution_index, true)?;
    }
    verify_five()?;
    if five_readers {
        for index in distribution_index + 1..periods.len() {
            run_period_observing(
                fresh_bootstrapped_root,
                fixture,
                index,
                Fault::None,
                ordinary.clone(),
            )?;
            verify_five()?;
        }
        verify_five()?;
    } else {
        native_integration::run_segment(
            fixture,
            &periods[distribution_index + 1..],
            &application_path,
            &state_path,
            false,
        )?;
    }
    verify_complete_final_reader(&retained, &prior_period["final"])?;
    verify_five()?;
    if ordinary.is_some() {
        verify_physical_cf5(&state_path, &fixture["history_rows"][4])?;
    }
    ensure!(
        final_facts(fresh_bootstrapped_root, fixture)? == expected,
        "{fault:?}: recovered mainnet workload differs from uninterrupted output"
    );
    drop(followthrough_readers);
    drop(retained);
    if legacy_readers || fixture["scenario"] == "magnolia_first_full_distribution" {
        verify_physical_rows(&state_path, &fixture["history_rows"])?;
    }
    Ok(())
}

/// Consensus-visible bytes and deterministic concrete facts can be compared
/// across independently created pairs. Database IDs/provenance are compared
/// exactly within each recovery run, since exclusive creation assigns new IDs.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct FinalFacts {
    stored_headers: Vec<Vec<u8>>,
    public_headers: Vec<Vec<u8>>,
    receipts: Vec<Vec<u8>>,
    dag_count: u64,
    transaction_count: u64,
    committed: ConcreteStateIdentity,
    catalog: Vec<u8>,
    generation: u64,
}
pub(super) fn final_facts(root: &Path, fixture: &Value) -> Result<FinalFacts> {
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
    let mut scratch = era_retention::register(&root, fixture)?;
    initialize_pair(&root, fixture)?;
    era_retention::created(&mut scratch)?;
    for index in 0..4 {
        run_period(&root, fixture, index, Fault::None)?;
    }
    let facts = final_facts(&root, fixture)?;
    if scratch.is_some() {
        era_retention::finish(
            scratch,
            serde_json::json!({"completed":"four period baseline with actual final facts","actual_facts":format!("{facts:?}")}),
            false,
        )?;
    } else {
        std::fs::remove_dir_all(root)?;
    }
    Ok(facts)
}

#[test]
fn reopened_recovery_requires_joint_startup_before_repeat() -> Result<()> {
    let fixture = load_fixture()?;
    let root = std::env::temp_dir().join(format!(
        "rustaxa-reopened-recovery-startup-{}",
        std::process::id()
    ));
    let mut scratch = era_retention::register(&root, &fixture)?;
    initialize_pair(&root, &fixture)?;
    era_retention::created(&mut scratch)?;
    let (application, chain) = open_chain(&root.join("application"), &fixture)?;
    let state_path = root.join("state_db");
    let chain_id = chain.concrete_chain_identity()?;
    let recovery = RecoveryLeaf {
        path: &state_path,
        chain_id,
        discards: Cell::new(0),
        state_api_epoch: StateApiEpoch::new(),
    };
    let before = ConcreteStateLifecycle::inspect_existing(&state_path, chain_id)?;
    let counts = chain.execution_status()?;
    let pending = application
        .final_chain()
        .external_evm_pending_publication_raw()?;
    let head = chain.last_block_number_typed()?;
    let header = application.final_chain().block_header_raw(head.as_u64())?;
    let check_unchanged = || -> Result<()> {
        ensure!(
            ConcreteStateLifecycle::inspect_existing(&state_path, chain_id)? == before
                && chain.execution_status()? == counts
                && chain.last_block_number_typed()? == head
                && application.final_chain().block_header_raw(head.as_u64())? == header
                && application
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    == pending
        );
        Ok(())
    };
    let error = recover_final_chain_application_state(&chain, &recovery).unwrap_err();
    ensure!(
        error
            .to_string()
            .contains("FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_EPOCH_MISMATCH")
    );
    check_unchanged()?;
    let startup =
        initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &recovery)?;
    ensure!(
        startup.status == FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
            && startup.error_code.is_empty()
            && recovery.discards.get() == 0
    );
    let repeated = recover_final_chain_application_state(&chain, &recovery)?;
    ensure!(
        repeated.status == FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
            && repeated.error_code.is_empty()
            && recovery.discards.get() == 0
    );
    check_unchanged()?;
    let foreign = RecoveryLeaf {
        path: &state_path,
        chain_id,
        discards: Cell::new(0),
        state_api_epoch: StateApiEpoch::new(),
    };
    let foreign_error = recover_final_chain_application_state(&chain, &foreign).unwrap_err();
    ensure!(
        foreign_error
            .to_string()
            .contains("FINAL_CHAIN_REWARD_SCHEDULER_RECOVERY_EPOCH_MISMATCH")
    );
    check_unchanged()?;
    let repeated_again = recover_final_chain_application_state(&chain, &recovery)?;
    ensure!(
        repeated_again.status == FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
            && repeated_again.error_code.is_empty()
            && recovery.discards.get() == 0
    );
    check_unchanged()?;
    drop(chain);
    drop(application);
    if scratch.is_some() {
        era_retention::finish(
            scratch,
            serde_json::json!({"completed":"fresh startup and same-epoch repeat"}),
            false,
        )?;
    } else {
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn mixed_custody_commit_interruptions_recover_and_continue_exactly() -> Result<()> {
    let fixture = load_fixture()?;
    WRAPPER_AUTHORITY_AUDIT.with(|audit| *audit.borrow_mut() = Some((false, false)));
    let uninterrupted = uninterrupted_facts(&fixture, "recovery")?;
    let (native_calls, empty_receipt) =
        WRAPPER_AUTHORITY_AUDIT.with(|audit| audit.borrow_mut().take().expect("authority audit"));
    ensure!(
        native_calls && empty_receipt,
        "actual wrapper must forward native and empty-invocation receipts"
    );
    for fault in [Fault::BeforeCommit, Fault::AfterCommit] {
        let root = std::env::temp_dir().join(format!(
            "rustaxa-mixed-recovery-{}-{fault:?}",
            std::process::id()
        ));
        let mut scratch = era_retention::register(&root, &fixture)?;
        initialize_pair(&root, &fixture)?;
        era_retention::created(&mut scratch)?;
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
        if scratch.is_some() {
            era_retention::finish(
                scratch,
                serde_json::json!({"fault":format!("{fault:?}"),"completed":"actual rejection/recovery/retry and full final facts equal uninterrupted; all handles closed","actual_facts":format!("{:?}",final_facts(&root,&fixture)?)}),
                false,
            )?;
        } else {
            std::fs::remove_dir_all(root)?;
        }
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
        Fault::UnchangedAuthorityProjection,
        Fault::MissingAuthority,
        Fault::PriorIdentity,
        Fault::PreparedIntent,
        Fault::RewardAccount,
    ] {
        let root = std::env::temp_dir().join(format!(
            "rustaxa-mixed-rejection-{}-{fault:?}",
            std::process::id()
        ));
        let mut scratch = era_retention::register(&root, &fixture)?;
        initialize_pair(&root, &fixture)?;
        era_retention::created(&mut scratch)?;
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
                physical_receipt: None,
                state_api_epoch: StateApiEpoch::new(),
                ordinary_observer: None,
                fixture: period,
                account_node_publication: if fixture["oracle_lifecycle"] == "public_batched" {
                    AccountNodePublication::FinalPeriod {
                        limits: rustaxa_storage::ConcreteStorageInventoryLimits {
                            max_nodes: 1_000_000,
                            max_leaves: 100_000,
                            max_value_bytes: 32_000_000,
                        },
                    }
                } else {
                    AccountNodePublication::ObserverHistory
                },
            },
            fault,
            executions: Cell::new(0),
            rewards: Cell::new(0),
            commits: Cell::new(0),
            original_projection_hash: Cell::new(None),
            recovery_capture: None,
            recovery_journal_cap: 0,
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
            Fault::UnchangedAuthorityProjection => "FINAL_CHAIN_NATIVE_AUTHORITY_BINDING_MISMATCH",
            Fault::MissingAuthority => "FINAL_CHAIN_NATIVE_AUTHORITY_REQUIRED",
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
        if scratch.is_some() {
            era_retention::finish(
                scratch,
                serde_json::json!({"fault":format!("{fault:?}"),"completed":"actual rejection/recovery/retry and full final facts equal uninterrupted; all handles closed","actual_facts":format!("{:?}",final_facts(&root,&fixture)?)}),
                false,
            )?;
        } else {
            std::fs::remove_dir_all(root)?;
        }
    }
    Ok(())
}

#[test]
fn wrapper_authority_forwards_before_context_and_checks_request() -> Result<()> {
    use rustaxa_consensus::native_projection_context::{
        FinalChainLifecycleEvent, FinalChainLifecycleReceipt, FinalChainNativeRewardsContext,
    };
    struct Fake {
        authority: LifecycleAuthority,
        available: Cell<bool>,
    }
    impl ConsensusExecutionPort for Fake {
        fn load_pillar_anchor_state(
            &self,
            _request: &rustaxa_consensus::PillarAnchorStateRequest,
        ) -> Result<rustaxa_consensus::PillarAnchorStateReport> {
            bail!("unexpected fake pillar request")
        }
        fn final_chain_native_lifecycle_authority(
            &self,
            request: [u8; 32],
            hash: [u8; 32],
        ) -> Result<Option<LifecycleAuthority>> {
            ensure!(
                self.available.get()
                    && self.authority.request_id == request
                    && self.authority.projection_hash == hash,
                "fake authority binding"
            );
            Ok(Some(self.authority.clone()))
        }
        fn final_chain_native_projection_context(
            &self,
            request_id: [u8; 32],
            projection_hash: [u8; 32],
        ) -> Result<Option<FinalChainNativeProjectionContext>> {
            ensure!(
                self.available.replace(false),
                "fake context already consumed"
            );
            Ok(Some(FinalChainNativeProjectionContext {
                request_id,
                projection_hash,
                invocations: Vec::new(),
                rewards: FinalChainNativeRewardsContext::default(),
                lifecycle: Vec::new(),
            }))
        }
    }
    let authority = LifecycleAuthority {
        request_id: [1; 32],
        state_api_epoch: 17,
        projection_hash: [2; 32],
        receipts: vec![FinalChainLifecycleReceipt {
            transaction_position: 0,
            transaction_hash: [3; 32],
            token: 9,
            event_count: 1,
            events: vec![FinalChainLifecycleEvent::TransactionEnd { observations: 0 }],
        }],
    };
    for fault in [
        Fault::None,
        Fault::BeforeCommit,
        Fault::AfterCommit,
        Fault::PriorIdentity,
        Fault::PreparedIntent,
        Fault::RewardAccount,
    ] {
        let fake = Fake {
            authority: authority.clone(),
            available: Cell::new(true),
        };
        assert_eq!(
            forward_lifecycle_authority(&fake, fault, None, None, [1; 32], [2; 32])?,
            Some(authority.clone())
        );
        assert!(forward_lifecycle_authority(&fake, fault, None, None, [8; 32], [2; 32]).is_err());
        fake.final_chain_native_projection_context([1; 32], [2; 32])?;
        assert!(forward_lifecycle_authority(&fake, fault, None, None, [1; 32], [2; 32]).is_err());
    }
    Ok(())
}

#[test]
fn synthetic_authority_changes_only_the_checked_projection_digest() -> Result<()> {
    use rustaxa_consensus::native_projection_context::{
        FinalChainLifecycleEvent, FinalChainLifecycleReceipt,
    };
    let authority = LifecycleAuthority {
        request_id: [1; 32],
        state_api_epoch: 17,
        projection_hash: [2; 32],
        receipts: vec![FinalChainLifecycleReceipt {
            transaction_position: 0,
            transaction_hash: [3; 32],
            token: 9,
            event_count: 1,
            events: vec![FinalChainLifecycleEvent::TransactionEnd { observations: 0 }],
        }],
    };
    let projection = FinalChainConcreteStateProjection::default();
    let hash = concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
    for fault in [
        Fault::StaleNativeValue,
        Fault::MissingCatalog,
        Fault::InvocationOrder,
        Fault::FailureDisposition,
    ] {
        let updated = test_only_authority_for_mutated_projection(
            fault,
            authority.clone(),
            &projection,
            hash,
        )?;
        let mut expected = authority.clone();
        expected.projection_hash = hash;
        assert_eq!(updated, expected);
        assert!(
            test_only_authority_for_mutated_projection(
                fault,
                authority.clone(),
                &projection,
                [0xff; 32]
            )
            .is_err()
        );
    }
    assert!(
        test_only_authority_for_mutated_projection(
            Fault::None,
            authority.clone(),
            &projection,
            hash
        )
        .is_err()
    );
    assert_eq!(authority.projection_hash, [2; 32]);
    Ok(())
}
