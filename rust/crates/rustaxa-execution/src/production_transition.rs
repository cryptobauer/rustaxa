//! Real owner computation. Prepared results expose no publication capability.
use crate::period_probe::gas_used;
use crate::{native::PreparedNativeExecutionPort, period_kernel::*};
use anyhow::{Result, ensure};
use rustaxa_consensus::ApplicationTransitionContext;
use rustaxa_consensus::concrete_state_projection::*;
use rustaxa_consensus::final_chain_execution::{
    FinalChainEvmLog, FinalChainEvmLogTopic, FinalChainEvmTransactionResult,
    FinalChainSystemTransactionPlanFact, encode_external_evm_receipt,
    plan_external_evm_system_transactions,
};
use rustaxa_consensus::native_projection_context::{
    FinalChainLifecycleAuthority, FinalChainLifecycleEvent, FinalChainLifecycleReceipt,
    FinalChainNativeProjectionContext,
};
use rustaxa_consensus::native_session::{
    FinalChainFinishedNativeTransition, FinalChainPreparedNativeSession,
};
use rustaxa_consensus::rewards_stats::RewardTransactionFact;
use rustaxa_evm::{
    contracts::*,
    driver::NativeAddressClassifier,
    input::LegacyInputKind,
    journal::ExecutionJournal,
    profile::{TaraxaPhase, TaraxaProfile},
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteExecutionProbeView, ConcreteObserverAccountChange,
    ConcreteObserverPhaseDelta, ConcreteTransitionBuilder, PreparedConcreteTransition,
};
use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
use rustaxa_types::concrete_state::ConcreteStorageKey;
use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{ConcreteRead, ConcreteStateIdentity},
};
use std::collections::BTreeSet;

/// Unpublished output from a real application owner. Drop discards all effects.
pub struct PreparedProductionTransition {
    concrete: PreparedConcreteTransition,
    native: FinalChainFinishedNativeTransition,
    transactions: Vec<PeriodTransactionOutput>,
    accounts: Vec<ConcreteObserverAccountChange>,
    request_digest: [u8; 32],
    verified_certificate: Option<rustaxa_consensus::VerifiedCertificateEvidence>,
    projection: FinalChainConcreteStateProjection,
    system_transactions: Vec<Vec<u8>>,
    native_replay: FinalChainNativeProjectionContext,
    lifecycle: FinalChainLifecycleAuthority,
    marker: Vec<u8>,
}
impl PreparedProductionTransition {
    pub(crate) fn into_verified(mut self) -> Result<PreparedVerifiedProductionTransition> {
        let evidence = self
            .verified_certificate
            .take()
            .ok_or_else(|| anyhow::anyhow!("PRODUCTION_VERIFIED_CERTIFICATE_EVIDENCE_MISSING"))?;
        ensure!(
            evidence.request_digest() == self.request_digest,
            "PRODUCTION_VERIFIED_CERTIFICATE_REQUEST_MISMATCH"
        );
        Ok(PreparedVerifiedProductionTransition {
            transition: self,
            evidence,
        })
    }

    pub fn next_identity(&self) -> ConcreteStateIdentity {
        self.concrete.next_identity()
    }
    pub fn changed_accounts(&self) -> &[ConcreteObserverAccountChange] {
        &self.accounts
    }
    pub fn results(
        &self,
    ) -> impl Iterator<Item = (&ExecutionTransaction, &TransactionExecutionResult)> {
        self.transactions
            .iter()
            .map(|t| (&t.transaction, &t.result))
    }
    pub fn request_digest(&self) -> [u8; 32] {
        self.request_digest
    }
    pub fn total_reward(&self) -> &rustaxa_types::DposTokenAmount {
        &self.native.rewards().total_reward
    }
}

/// Unpublished complete output with checked certificate evidence.
pub struct PreparedVerifiedProductionTransition {
    transition: PreparedProductionTransition,
    evidence: rustaxa_consensus::VerifiedCertificateEvidence,
}
impl PreparedVerifiedProductionTransition {
    pub(crate) fn publish(
        self,
        application: &rustaxa_consensus::BoundConsensusApplication,
        authority: &rustaxa_consensus::ProductionOwnerPublicationAuthority,
        stop: u8,
    ) -> Result<ConcreteStateIdentity> {
        let Self {
            transition,
            evidence,
        } = self;
        let mut cumulative = 0_u64;
        let mut results = Vec::new();
        for tx in &transition.transactions {
            cumulative = cumulative
                .checked_add(gas_used(&tx.result))
                .ok_or_else(|| anyhow::anyhow!("PRODUCTION_GAS_SUM_OVERFLOW"))?;
            results.push(publication_result(tx, cumulative));
        }
        let pillar = application
            .scheduled_pillar_checkpoint(&evidence)?
            .map(|period| {
                application.with_finalized_execution_state(period.into(), |state| {
                    ensure!(
                        state.period().as_u64() == period,
                        "PRODUCTION_PILLAR_ANCHOR_ABOVE_HEAD"
                    );
                    let identity = state.with_evm_checkpoint(|_, identity| Ok(identity))?;
                    let (bridge_root, bridge_epoch) =
                        crate::production_dag_gas::pillar_bridge_values(state)?;
                    Ok((identity, bridge_root, bridge_epoch))
                })
            })
            .transpose()?;
        application.publish_verified_transition(
            authority,
            evidence,
            transition.concrete,
            transition.native,
            transition.projection,
            &transition.system_transactions,
            pillar,
            (&transition.native_replay, &transition.lifecycle),
            transition.marker,
            results,
            stop,
        )
    }
    pub fn next_identity(&self) -> ConcreteStateIdentity {
        self.transition.next_identity()
    }
    pub fn changed_accounts(&self) -> &[ConcreteObserverAccountChange] {
        self.transition.changed_accounts()
    }
    pub fn results(
        &self,
    ) -> impl Iterator<Item = (&ExecutionTransaction, &TransactionExecutionResult)> {
        self.transition.results()
    }
    pub fn request_digest(&self) -> [u8; 32] {
        self.transition.request_digest()
    }
    pub fn certificate_digest(&self) -> [u8; 32] {
        self.evidence.certificate_digest()
    }
    pub fn total_reward(&self) -> &rustaxa_types::DposTokenAmount {
        self.transition.total_reward()
    }
}

struct ProductionState<'a> {
    builder: ConcreteTransitionBuilder<'a>,
    touched: BTreeSet<[u8; 20]>,
    catalog: BTreeSet<ConcreteStorageSlot>,
    effects: Vec<FinalChainConcreteTransactionEffect>,
}
impl PeriodState for ProductionState<'_> {
    type View<'a>
        = ConcreteExecutionProbeView<'a>
    where
        Self: 'a;
    fn view(&self) -> Result<Self::View<'_>> {
        Ok(self.builder.current_view()?)
    }
    fn apply_phase(&mut self, delta: ConcreteObserverPhaseDelta) -> Result<()> {
        let addresses: Vec<_> = delta
            .accounts
            .iter()
            .map(|mutation| match mutation {
                ConcreteAccountMutation::Upsert { address, .. }
                | ConcreteAccountMutation::Delete { address } => *address,
            })
            .chain(delta.storage.iter().map(|mutation| mutation.address))
            .collect();
        self.catalog.extend(
            delta
                .storage
                .iter()
                .filter(|s| s.address[..19] == [0; 19] && matches!(s.address[19], 0xfe | 0xee))
                .map(|s| ConcreteStorageSlot {
                    address: s.address,
                    key: s.key.0,
                }),
        );
        self.builder.apply_phase(delta)?;
        self.touched.extend(addresses);
        Ok(())
    }
    fn identity(&self) -> Result<ConcreteStateIdentity> {
        Ok(self.view()?.identity())
    }
    fn records_lifecycle(&self) -> bool {
        true
    }
    fn transaction_completed(&mut self, output: &PeriodTransactionOutput) -> Result<()> {
        let result = publication_result(output, 0);
        self.effects.push(FinalChainConcreteTransactionEffect {
            index: self.effects.len() as u64,
            transaction_rlp: projection_transaction(&output.transaction)?,
            execution_result_rlp: encode_concrete_execution_result(&result),
            intermediate_state: FinalChainConcreteState {
                period: output.state.period.as_u64(),
                root: output.state.state_root,
            },
            accounts: self.projection_accounts()?,
            storage: self.projection_storage(output.raw_slots.iter())?,
            invocations: output
                .native
                .iter()
                .map(project_native_invocation)
                .collect(),
        });
        Ok(())
    }
}
impl ProductionState<'_> {
    fn projection_accounts(&self) -> Result<Vec<FinalChainConcreteAccountProjection>> {
        self.final_accounts()?
            .into_iter()
            .map(|a| {
                Ok(FinalChainConcreteAccountProjection {
                    address: a.address,
                    raw_account_rlp: match a.account {
                        ConcreteRead::Present(row) => row.physical_rlp,
                        _ => Vec::new(),
                    },
                })
            })
            .collect()
    }
    fn projection_storage<'a>(
        &self,
        slots: impl Iterator<Item = &'a ConcreteStorageSlot>,
    ) -> Result<Vec<FinalChainConcreteStorageProjection>> {
        let view = self.view()?;
        slots
            .map(|s| {
                Ok(FinalChainConcreteStorageProjection {
                    contract: s.address,
                    key: s.key,
                    value: match view.storage(s.address, ConcreteStorageKey(s.key))? {
                        ConcreteRead::Present(v) => v,
                        _ => Vec::new(),
                    },
                })
            })
            .collect()
    }
    fn final_accounts(&self) -> Result<Vec<ConcreteObserverAccountChange>> {
        let view = self.view()?;
        self.touched
            .iter()
            .map(|address| {
                Ok(ConcreteObserverAccountChange {
                    address: *address,
                    account: view.account(*address)?,
                })
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct ProductionTransitionHardError {
    pub position: u64,
    pub native_sequence: Option<u64>,
    pub message: String,
}
impl std::fmt::Display for ProductionTransitionHardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PRODUCTION_TRANSITION_MALFORMED_PROOF at {}: {}",
            self.position, self.message
        )
    }
}
impl std::error::Error for ProductionTransitionHardError {}

fn bridge_system_transactions(
    context: &mut ApplicationTransitionContext<'_>,
) -> Result<Vec<Vec<u8>>> {
    use rustaxa_evm::{envelope::EnvelopeRules, simulation::SimulationBinding};
    use rustaxa_types::TARAXA_SYSTEM_ACCOUNT;
    use tiny_keccak::{Hasher, Keccak};
    let Some(simulation) = context.take_bridge_simulation() else {
        return Ok(Vec::new());
    };
    let (bridge, readers, phase, cornus) = context.bridge_simulation_inputs();
    let request = context.request();
    let facts = request.context().facts();
    let prior = facts.prior;
    let header = context.bridge_simulation_header()?;
    let header = rlp::Rlp::new(&header);
    ensure!(
        header.val_at::<u64>(7)? == prior.period.as_u64()
            && header.val_at::<ethereum_types::H256>(3)?.0 == prior.state_root,
        "PRODUCTION_BRIDGE_CHECKPOINT_HEADER_MISMATCH"
    );
    let mut hash = Keccak::v256();
    hash.update(b"shouldFinalizeEpoch()");
    let mut selector = [0; 32];
    hash.finalize(&mut selector);
    let transaction = ExecutionTransaction {
        position: 0_u32.into(),
        hash: [0; 32],
        sender: [0; 20],
        receiver: Some(bridge),
        nonce: Default::default(),
        gas_price: ExecutionGasPrice::new(1_u8.into()),
        gas_limit: 10_000_000.into(),
        value: ExecutionValue::new(Default::default()),
        input: selector[..4].to_vec(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    };
    let execution = crate::production_dag_gas::simulate_at_checkpoint(
        readers,
        SimulationBinding {
            state: prior,
            block: ExecutionBlockContext {
                period: prior.period,
                timestamp: header.val_at(10)?,
                author: header.val_at::<ethereum_types::H160>(2)?.0,
                gas_limit: header.val_at::<u64>(8)?.into(),
                chain_id: facts.schedule.chain_id(),
                difficulty: Default::default(),
            },
            profile: historical_profile(phase)
                .with_correction_stage(facts.schedule.correction_stage(prior.period)),
            envelope_rules: EnvelopeRules { cornus },
        },
        simulation,
        |number| context.block_hash(number),
        |period, address| context.is_native(period, address),
        &transaction,
    )
    .map_err(|error| anyhow::anyhow!("PRODUCTION_BRIDGE_SYSTEM_READ: {error}"))?;
    let data = match execution {
        TransactionExecutionResult::Executed(result) => result.output,
        TransactionExecutionResult::ConsensusFailure(result) => result.output,
    };
    ensure!(data.len() <= 32, "PRODUCTION_BRIDGE_SYSTEM_BOOL_WIDTH");
    if data.iter().all(|byte| *byte == 0) {
        return Ok(Vec::new());
    }
    let nonce = match readers.account_at(prior, TARAXA_SYSTEM_ACCOUNT)? {
        ConcreteRead::Present(row) => row.account.nonce,
        ConcreteRead::Absent | ConcreteRead::Tombstone => Default::default(),
    };
    Ok(
        plan_external_evm_system_transactions(FinalChainSystemTransactionPlanFact {
            request_id: request.digest(),
            period: facts.period,
            is_pillar_block_period: true,
            bridge_contract_address: bridge,
            bridge_contract_found: true,
            bridge_contract_has_code: true,
            should_finalize_epoch: true,
            system_account_nonce: nonce,
            block_gas_limit: facts.gas_limit.into(),
        })?
        .transactions,
    )
}

struct TransactionPhase<'store> {
    concrete: ProductionState<'store>,
    native: PreparedNativeExecutionPort,
    transactions: Vec<PeriodTransactionOutput>,
    system_transactions: Vec<Vec<u8>>,
    post_transactions: ConcreteStateIdentity,
    profile: TaraxaProfile,
}

fn run_transaction_phase<'store>(
    context: &mut ApplicationTransitionContext<'_>,
    concrete: ConcreteTransitionBuilder<'store>,
    session: FinalChainPreparedNativeSession,
) -> Result<TransactionPhase<'store>> {
    let system_transactions = bridge_system_transactions(context)?;
    let observation = context.concrete_observation();
    let catalog = rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog(
        &observation.catalog_rlp,
    )?;
    let mut concrete = ProductionState {
        builder: concrete,
        touched: BTreeSet::new(),
        catalog: catalog.into_iter().collect(),
        effects: Vec::new(),
    };
    let request = context.request();
    let facts = request.context().facts();
    let profile = historical_profile(facts.instruction_phase)
        .with_correction_stage(request.context().correction_stage());
    ensure!(
        session.correction_stage() == profile.correction_stage(),
        "PRODUCTION_NATIVE_RULE_MISMATCH"
    );
    let mut dpos = [0; 20];
    dpos[19] = 0xfe;
    let empty = !matches!(concrete.view()?.account(dpos)?, ConcreteRead::Present(ref row) if row.account.code_size != 0);
    prepare_native_code(
        &mut concrete,
        context
            .code_updates(empty)
            .into_iter()
            .map(|u| NativeCodeUpdate {
                address: u.address,
                code: u.code.to_vec(),
            }),
    )?;
    let block = ExecutionBlockContext {
        period: facts.period,
        author: facts.author,
        timestamp: facts.timestamp,
        gas_limit: facts.gas_limit.into(),
        chain_id: facts.schedule.chain_id(),
        difficulty: Default::default(),
    };
    let mut native = PreparedNativeExecutionPort::new(session);
    let transactions = match execute_transaction_stream(
        &mut concrete,
        &mut native,
        &block,
        profile,
        facts.cornus,
        context.credit_fees(),
        request
            .snapshot()
            .transactions()
            .iter()
            .map(|t| (t.rlp.as_slice(), LegacyInputKind::Signed))
            .chain(
                system_transactions
                    .iter()
                    .map(|t| (t.as_slice(), LegacyInputKind::System)),
            ),
        &Hashes(|number| context.block_hash(number)),
        &Natives(
            |period, address| context.is_native(period, address),
            profile,
            true,
        ),
        &Natives(
            |period, address| context.is_native(period, address),
            profile,
            false,
        ),
        &mut (),
    )? {
        PeriodKernelOutcome::Completed(results) => results,
        PeriodKernelOutcome::MalformedProof {
            transaction_position,
            native_sequence,
            message,
        } => {
            return Err(ProductionTransitionHardError {
                position: transaction_position,
                native_sequence,
                message,
            }
            .into());
        }
    };
    let post_transactions = concrete.identity()?;
    native.validate_context()?;
    Ok(TransactionPhase {
        concrete,
        native,
        transactions,
        system_transactions,
        post_transactions,
        profile,
    })
}

/// Checks only transaction execution. The detached builder and native ports are
/// dropped before the report leaves its owner scope. This report cannot publish.
pub(crate) fn check_candidate_transactions(
    mut context: ApplicationTransitionContext<'_>,
    concrete: ConcreteTransitionBuilder<'_>,
    session: FinalChainPreparedNativeSession,
) -> Result<rustaxa_consensus::pbft_execution_validation::PbftExecutionValidation> {
    use rustaxa_consensus::pbft_execution_validation::{
        PbftExecutionHardError, PbftExecutionValidation,
    };
    let request_digest = context.request().digest();
    let phase = match run_transaction_phase(&mut context, concrete, session) {
        Ok(phase) => phase,
        Err(error) => {
            let bad = match error.downcast::<ProductionTransitionHardError>() {
                Ok(bad) => bad,
                Err(error) => return Err(error),
            };
            return Ok(PbftExecutionValidation::Invalid {
                request_digest,
                transaction_position: bad.position,
                native_sequence: bad.native_sequence,
                cause: PbftExecutionHardError::MalformedNestedSlashingProof(bad.message),
            });
        }
    };
    let mut results = rlp::RlpStream::new_list(4);
    results
        .append(&b"rustaxa-pbft-probe-results".as_slice())
        .append(&1_u8)
        .append(&request_digest.as_slice())
        .begin_list(phase.transactions.len());
    for transaction in &phase.transactions {
        crate::period_probe::append_result(
            &mut results,
            &transaction.transaction,
            &transaction.result,
            transaction.state,
        );
    }
    let executed_transactions = u64::try_from(phase.transactions.len())?;
    use tiny_keccak::{Hasher, Keccak};
    let mut hash = Keccak::v256();
    let mut results_digest = [0; 32];
    hash.update(&results.out());
    hash.finalize(&mut results_digest);
    Ok(PbftExecutionValidation::Valid {
        request_digest,
        executed_transactions,
        system_transactions: phase.system_transactions,
        results_digest,
    })
}

pub(crate) fn execute(
    mut context: ApplicationTransitionContext<'_>,
    concrete: ConcreteTransitionBuilder<'_>,
    session: FinalChainPreparedNativeSession,
) -> Result<PreparedProductionTransition> {
    let TransactionPhase {
        mut concrete,
        native,
        transactions,
        system_transactions,
        post_transactions,
        profile,
    } = run_transaction_phase(&mut context, concrete, session)?;
    let observation = context.concrete_observation();
    let request = context.request();
    let facts = request.context().facts();
    let rewards = context.plan_rewards(
        transactions
            .iter()
            .filter(|t| t.transaction.kind != ExecutionTransactionKind::System)
            .map(|t| RewardTransactionFact {
                hash: ethereum_types::H256(t.transaction.hash),
                gas_price: ethereum_types::U256::from_big_endian(
                    &t.transaction.gas_price.value().to_bytes_be(),
                ),
                gas_used: gas_used(&t.result).into(),
            })
            .collect(),
    )?;
    let mut journal =
        ExecutionJournal::new_with_correction_stage(concrete.view()?, profile.correction_stage());
    let (native, invocations, rewards) = native.finish(rewards, &journal)?;
    journal.apply_native_account_mutations(&crate::native::ordinary_mutations(
        &native.rewards().account_mutations,
    ))?;
    for mutation in crate::native::raw_mutations(&native.rewards().raw_mutations)? {
        ensure!(
            journal.raw_storage(mutation.address, mutation.key)? == mutation.expected,
            "PRODUCTION_REWARD_RAW_SOURCE_MISMATCH"
        );
        journal.set_raw_storage(mutation.address, mutation.key, mutation.operation)?;
    }
    let settled = journal.settle_transaction()?;
    drop(journal);
    concrete.apply_phase(settlement_delta(settled.writes))?;
    let accounts = concrete.final_accounts()?;
    let post_rewards = concrete.identity()?;
    let projection = FinalChainConcreteStateProjection {
        identity: observation.identity,
        generation: observation
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("PRODUCTION_GENERATION_OVERFLOW"))?,
        plan_hash: request.digest(),
        prior_state: FinalChainConcreteState {
            period: observation.committed.period.as_u64(),
            root: observation.committed.state_root,
        },
        post_transaction_state: FinalChainConcreteState {
            period: facts.period.as_u64(),
            root: post_transactions.state_root,
        },
        post_rewards_state: FinalChainConcreteState {
            period: post_rewards.period.as_u64(),
            root: post_rewards.state_root,
        },
        transaction_effects: concrete.effects.clone(),
        accounts: concrete.projection_accounts()?,
        storage: concrete.projection_storage(concrete.catalog.iter())?,
        invocations: concrete
            .effects
            .iter()
            .flat_map(|effect| effect.invocations.iter().cloned())
            .collect(),
        rewards_input: native.rewards_input(),
        catalog_hash:
            rustaxa_types::codec::rlp::concrete_lifecycle::concrete_storage_slot_catalog_hash(
                concrete.catalog.iter().copied(),
            ),
    };
    let projection_hash =
        rustaxa_types::codec::rlp::concrete_lifecycle::concrete_state_bytes_digest(
            &rustaxa_consensus::concrete_state_projection::encode_concrete_state_projection(
                &projection,
            ),
        );
    let receipts = transactions
        .iter()
        .map(lifecycle_receipt)
        .collect::<Result<Vec<_>>>()?;
    let lifecycle = FinalChainLifecycleAuthority {
        request_id: request.digest(),
        state_api_epoch: facts.state_api_epoch,
        projection_hash,
        receipts,
    };
    let native_replay = FinalChainNativeProjectionContext {
        request_id: request.digest(),
        projection_hash,
        invocations,
        rewards,
        lifecycle: lifecycle.receipts.clone(),
    };
    let marker = context.execution_marker()?;
    let concrete = concrete.builder.seal(&marker)?;
    let request_digest = request.digest();
    let verified_certificate = context.into_verified_certificate();
    Ok(PreparedProductionTransition {
        concrete,
        native,
        transactions,
        accounts,
        request_digest,
        verified_certificate,
        projection,
        system_transactions,
        native_replay,
        lifecycle,
        marker,
    })
}

fn lifecycle_receipt(output: &PeriodTransactionOutput) -> Result<FinalChainLifecycleReceipt> {
    use FinalChainLifecycleEvent as Domain;
    use rustaxa_evm::journal::JournalLifecycleEvent as Event;
    let receipt = output
        .lifecycle_receipt
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PRODUCTION_LIFECYCLE_RECEIPT_MISSING"))?;
    let events: Vec<_> = receipt
        .events()
        .iter()
        .map(|event| match *event {
            Event::FrameOpened { parent } => Domain::FrameOpened { parent },
            Event::AccountCreated {
                address,
                frame,
                saved_absent,
            } => Domain::AccountCreated {
                address,
                frame,
                saved_absent,
            },
            Event::NativeStarted { id } => Domain::NativeStarted {
                transaction: id.transaction.as_u32(),
                sequence: id.sequence,
            },
            Event::NativeObserved { id } => Domain::NativeObserved {
                transaction: id.transaction.as_u32(),
                sequence: id.sequence,
            },
            Event::TransactionEnd { observations } => Domain::TransactionEnd { observations },
            Event::AccountReset {
                address,
                creation,
                rollback_frame,
            } => Domain::AccountReset {
                address,
                creation,
                rollback_frame,
            },
        })
        .collect();
    Ok(FinalChainLifecycleReceipt {
        transaction_position: output.transaction.position.as_u32(),
        transaction_hash: output.transaction.hash,
        token: receipt.token_value(),
        event_count: events.len(),
        events,
    })
}

fn project_native_invocation(
    observation: &ConsensusNativeObservation,
) -> FinalChainConcreteInvocation {
    let invocation = &observation.invocation;
    let call_type = match invocation.kind {
        NativeCallKind::Call => 0,
        NativeCallKind::CallCode => 1,
        NativeCallKind::DelegateCall => 2,
        NativeCallKind::StaticCall => 3,
    };
    let disposition = match observation.disposition {
        ConsensusNativeDisposition::Normal => FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL,
        ConsensusNativeDisposition::OwnFrameReverted => {
            FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED
        }
        ConsensusNativeDisposition::OuterFrameReverted => {
            FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED
        }
    };
    let error = match &observation.status {
        CodeExecutionStatus::Success => String::new(),
        CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) => error.error.clone(),
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas".into(),
        CodeExecutionStatus::Failure(CodeExecutionError::Revert) => "execution reverted".into(),
        CodeExecutionStatus::Failure(CodeExecutionError::InvalidOpcode(opcode)) => {
            format!("invalid opcode 0x{opcode:02x}")
        }
        CodeExecutionStatus::Failure(other) => format!("{other:?}"),
    };
    FinalChainConcreteInvocation {
        transaction_index: u64::from(invocation.id.transaction.as_u32()),
        sequence: invocation.id.sequence,
        depth: invocation.depth,
        call_type,
        caller: invocation.caller,
        contract: invocation.contract,
        value: invocation.value.value().to_bytes_be(),
        input: invocation.input.clone(),
        output: observation.output.clone(),
        supplied_gas: invocation.supplied_gas.as_u64(),
        required_gas: observation.required_gas.as_u64(),
        gas_used: observation.gas_used.as_u64(),
        logs: observation
            .logs
            .iter()
            .map(|log| FinalChainEvmLog {
                address: log.address,
                topics: log
                    .topics
                    .iter()
                    .map(|topic| FinalChainEvmLogTopic { topic: *topic })
                    .collect(),
                data: log.data.clone(),
            })
            .collect(),
        disposition,
        error,
    }
}

fn publication_result(
    output: &PeriodTransactionOutput,
    cumulative: u64,
) -> FinalChainEvmTransactionResult {
    let (success, data, logs, address, code_error, consensus_error) = match &output.result {
        TransactionExecutionResult::Executed(r) => (
            matches!(r.status, CodeExecutionStatus::Success),
            r.output.clone(),
            r.logs.clone(),
            r.attempted_contract_address,
            match &r.status {
                CodeExecutionStatus::Success => String::new(),
                CodeExecutionStatus::Failure(e) => format!("{e:?}"),
            },
            String::new(),
        ),
        TransactionExecutionResult::ConsensusFailure(r) => (
            false,
            r.output.clone(),
            Vec::new(),
            r.attempted_contract_address,
            String::new(),
            format!("{:?}", r.error),
        ),
    };
    let mut result = FinalChainEvmTransactionResult {
        position: output.transaction.position,
        hash: output.transaction.hash,
        status: u8::from(success),
        gas_used: gas_used(&output.result).into(),
        cumulative_gas_used: cumulative.into(),
        receipt_rlp: Vec::new(),
        logs: logs
            .into_iter()
            .map(|l| FinalChainEvmLog {
                address: l.address,
                topics: l
                    .topics
                    .into_iter()
                    .map(|topic| FinalChainEvmLogTopic { topic })
                    .collect(),
                data: l.data,
            })
            .collect(),
        new_contract_address: address,
        output: data,
        code_error,
        consensus_error,
    };
    result.receipt_rlp = encode_external_evm_receipt(&result);
    result
}

pub(crate) fn historical_profile(
    phase: rustaxa_consensus::pbft_execution_validation::PbftInstructionPhase,
) -> TaraxaProfile {
    use rustaxa_consensus::pbft_execution_validation::PbftInstructionPhase;
    TaraxaProfile::for_phase(match phase {
        PbftInstructionPhase::Californicum => TaraxaPhase::Californicum,
        PbftInstructionPhase::Ficus => TaraxaPhase::Ficus,
        PbftInstructionPhase::Cacti => TaraxaPhase::Cacti,
    })
}

pub(crate) struct Hashes<F>(pub(crate) F);
impl<F: Fn(FinalChainBlockNumber) -> Result<Option<Vec<u8>>>> BlockHashRead for Hashes<F> {
    fn block_hash(&self, n: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        (self.0)(n)
            .map_err(|e| BlockHashReadError::Io(e.to_string()))?
            .ok_or(BlockHashReadError::HistoryUnavailable(n))?
            .try_into()
            .map_err(|_| BlockHashReadError::Corrupt("block hash width".into()))
    }
}
pub(crate) struct Natives<F>(pub(crate) F, pub(crate) TaraxaProfile, pub(crate) bool);
impl<F: Fn(FinalChainBlockNumber, [u8; 20]) -> bool> NativeAddressClassifier for Natives<F> {
    fn is_native_address(&self, p: FinalChainBlockNumber, a: [u8; 20]) -> bool {
        use rustaxa_evm::bls::{BlsPrecompile, BlsRegistry};
        (self.0)(p, a)
            || self.2
                && ((a[..19] == [0; 19]
                    && (matches!(a[19], 1..=8) || self.1.ficus() && a[19] == 9))
                    || self.1.ficus()
                        && BlsPrecompile::at_address(
                            if self.1.cacti() {
                                BlsRegistry::Cacti
                            } else {
                                BlsRegistry::Ficus
                            },
                            a,
                        )
                        .is_some()
                    || self.1.cacti()
                        && matches!(
                            a,
                            rustaxa_evm::p256::P256_VERIFY_ADDRESS
                                | rustaxa_evm::falcon::FALCON_VERIFY_ADDRESS
                        ))
    }
}

fn projection_transaction(transaction: &ExecutionTransaction) -> Result<Vec<u8>> {
    let price = transaction.gas_price.value().to_bytes_be();
    let value = transaction.value.value().to_bytes_be();
    ensure!(
        price.len() <= 32 && value.len() <= 32,
        "PRODUCTION_PUBLICATION_TRANSACTION_WIDTH_UNAVAILABLE"
    );
    let mut out = rlp::RlpStream::new_list(7);
    out.append(&transaction.sender.as_slice())
        .append(&ethereum_types::U256::from_big_endian(&price));
    match transaction.receiver {
        Some(receiver) => {
            out.append(&receiver.as_slice());
        }
        None => {
            out.append_empty_data();
        }
    }
    out.append(&transaction.nonce.to_bytes())
        .append(&ethereum_types::U256::from_big_endian(&value))
        .append(&transaction.gas_limit.as_u64())
        .append(&transaction.input);
    Ok(out.out().to_vec())
}
