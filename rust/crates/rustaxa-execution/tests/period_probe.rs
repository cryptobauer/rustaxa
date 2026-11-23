//! Synthetic signed candidate tests over observed genesis rows; no reference output is generated.
use anyhow::{Context, Result, ensure};
use ethereum_types::{H256, U256};
use k256::ecdsa::SigningKey;
use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_consensus::{pbft_execution_validation::*, *};
use rustaxa_execution::period_probe::*;
use rustaxa_storage::{ConcreteCheckpointReaders, ConcreteExecutionProbe, ConcreteStateLifecycle};
use rustaxa_types::{
    LegacyTransactionEnvelope, concrete_state::ConcreteStateIdentity, dag::DagBlock,
    evm_correction::CorrectionStage,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[allow(dead_code)]
#[path = "../../rustaxa-evm/tests/support/mixed_genesis.rs"]
mod mixed_genesis;

#[path = "../../rustaxa-storage/src/test_paths.rs"]
mod test_paths;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Observed {
    entered: Vec<u32>,
    completed: Vec<(
        u32,
        rustaxa_evm::contracts::TransactionExecutionResult,
        Vec<rustaxa_evm::contracts::ConsensusNativeObservation>,
        usize,
        ConcreteStateIdentity,
    )>,
}
impl PbftExecutionObserver for Observed {
    fn transaction_started(&mut self, tx: &rustaxa_evm::contracts::ExecutionTransaction) {
        self.entered.push(tx.position.as_u32());
    }
    fn transaction_completed(
        &mut self,
        tx: &rustaxa_evm::contracts::ExecutionTransaction,
        result: &rustaxa_evm::contracts::TransactionExecutionResult,
        native: &[rustaxa_evm::contracts::ConsensusNativeObservation],
        raw_writes: usize,
        state: ConcreteStateIdentity,
    ) {
        self.completed.push((
            tx.position.as_u32(),
            result.clone(),
            native.to_vec(),
            raw_writes,
            state,
        ));
    }
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    keccak256(bytes).0
}
fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap().trim_start_matches("0x")).unwrap()
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/mixed_workload_local_observer.json")).unwrap()
}
fn scratch() -> PathBuf {
    crate::test_paths::new_path("ordered-probe-test")
}
struct StartupLeaf<'a>(&'a ConcreteStateLifecycle);
impl ConsensusExecutionPort for StartupLeaf<'_> {
    fn load_final_chain_committed_state(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        let observed = self.0.observation()?;
        ensure!(observed.identity.chain_id == request.concrete_chain_identity);
        Ok(FinalChainExternalEvmPreflightReport {
            request_id: request.request_id,
            state_api_epoch: 1,
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
    fn load_pillar_anchor_state(
        &self,
        _: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        anyhow::bail!("unexpected pillar read")
    }
}
/// Synthetic test checks only; this is not production owner authority.
struct SyntheticKernelBinding<'a> {
    chain: &'a FinalChain,
    path: &'a Path,
    expected: PbftExecutionContextFacts,
}
impl PbftProbeOwnerBinding for SyntheticKernelBinding<'_> {
    fn validate(&self, context: &PbftExecutionContext) -> Result<Option<PbftExecutionUnavailable>> {
        let observed = ConcreteStateLifecycle::inspect_existing(
            self.path,
            self.chain.concrete_chain_identity()?,
        )?;
        Ok((context.facts() != &self.expected
            || observed.committed != self.expected.prior
            || observed.generation != self.expected.concrete_generation
            || hash(&observed.provenance_rlp) != self.expected.concrete_provenance_digest
            || self.chain.last_block_number_typed()? != self.expected.prior.period)
            .then_some(PbftExecutionUnavailable::StateChanged))
    }
}

/// Synthetic native/configuration facts; concrete facts come from this owner.
struct BorrowedSyntheticKernelBinding<'a> {
    chain: &'a FinalChain,
    owner: &'a ConcreteStateLifecycle,
    expected: PbftExecutionContextFacts,
}
impl PbftProbeOwnerBinding for BorrowedSyntheticKernelBinding<'_> {
    fn validate(&self, context: &PbftExecutionContext) -> Result<Option<PbftExecutionUnavailable>> {
        let observed = self.owner.observation()?;
        Ok((context.facts() != &self.expected
            || observed.identity.chain_id != self.chain.concrete_chain_identity()?
            || observed.committed != self.expected.prior
            || observed.generation != self.expected.concrete_generation
            || hash(&observed.provenance_rlp) != self.expected.concrete_provenance_digest
            || !observed.pending_marker_rlp.is_empty()
            || self.chain.last_block_number_typed()? != self.expected.prior.period)
            .then_some(PbftExecutionUnavailable::StateChanged))
    }
}
fn sign(bytes: &[u8], key: &SigningKey) -> Vec<u8> {
    let (signature, recovery) = key.sign_prehash_recoverable(&hash(bytes)).unwrap();
    let mut result = signature.to_bytes().to_vec();
    result.push(recovery.to_byte());
    result
}
fn request(
    fixture: &Value,
    facts: PbftExecutionContextFacts,
    transactions: Vec<Vec<u8>>,
) -> PbftExecutionRequest {
    let context = PbftExecutionContext::new(facts).unwrap();
    let key = SigningKey::from_slice(&bytes(&fixture["inputs"]["pbft_private_key"])).unwrap();
    let envelopes: Vec<_> = transactions
        .into_iter()
        .map(|tx| LegacyTransactionEnvelope::decode(&tx).unwrap())
        .collect();
    let mut dag = DagBlock {
        pivot: H256::zero(),
        level: 1,
        timestamp: context.facts().timestamp,
        vdf: vec![],
        tips: vec![],
        transactions: envelopes.iter().map(|tx| tx.hash).collect(),
        signature: [0; 65],
        gas_estimation: 1_000_000,
    };
    let (signature, recovery) = key
        .sign_prehash_recoverable(dag.signing_hash().as_bytes())
        .unwrap();
    dag.signature[..64].copy_from_slice(&signature.to_bytes());
    dag.signature[64] = recovery.to_byte();
    let mut raw = RlpStream::new_list(8);
    raw.append(&dag.pivot)
        .append(&dag.level)
        .append(&dag.timestamp)
        .append(&dag.vdf)
        .append_list(&dag.tips)
        .append_list(&dag.transactions)
        .append(&dag.signature.as_slice())
        .append(&dag.gas_estimation);
    let dag_bytes = raw.out().to_vec();
    let dag_hash = H256(hash(&dag_bytes));
    let mut order = RlpStream::new_list(1);
    order.begin_list(1).append(&dag_hash.0.as_slice());
    let order_hash = H256(hash(&order.out()));
    let fields = |out: &mut RlpStream| {
        out.append(&H256(context.facts().previous_pbft_hash))
            .append(&dag_hash)
            .append(&order_hash)
            .append(&H256::zero())
            .append(&context.facts().period.as_u64())
            .append(&context.facts().timestamp)
            .begin_list(0);
    };
    let mut unsigned = RlpStream::new_list(7);
    fields(&mut unsigned);
    let signature = sign(&unsigned.out(), &key);
    let mut block = RlpStream::new_list(8);
    fields(&mut block);
    block.append(&signature);
    let snapshot = PbftExecutionSnapshot::new(
        block.out().to_vec(),
        vec![rustaxa_consensus::dag::DagSyncBlockRlp {
            hash: dag_hash,
            block_rlp: dag_bytes,
        }],
        envelopes
            .into_iter()
            .map(|tx| PbftExecutionTransactionLookup::Included {
                hash: tx.hash,
                rlp: tx.rlp,
            })
            .collect(),
        vec![],
        &context,
        PbftExecutionInputLimits {
            maximum_bytes: 2_000_000,
            maximum_transactions: 100,
            maximum_dag_blocks: 10,
        },
    )
    .unwrap();
    PbftExecutionRequest::new(Arc::new(snapshot), context).unwrap()
}
fn transaction(fixture: &Value, nonce: u64, receiver: [u8; 20], input: &[u8], gas: u64) -> Vec<u8> {
    let chain_id = fixture["configuration"]["chain_id"].as_u64().unwrap();
    let key = SigningKey::from_slice(&bytes(&fixture["inputs"]["sender_private_key"])).unwrap();
    let fields = |out: &mut RlpStream| {
        out.append(&nonce)
            .append(&1_u8)
            .append(&gas)
            .append(&receiver.as_slice())
            .append(&0_u8)
            .append(&input);
    };
    let mut unsigned = RlpStream::new_list(9);
    fields(&mut unsigned);
    unsigned.append(&chain_id).append(&0_u8).append(&0_u8);
    let (signature, recovery) = key
        .sign_prehash_recoverable(&hash(&unsigned.out()))
        .unwrap();
    let signature = signature.to_bytes();
    let mut signed = RlpStream::new_list(9);
    fields(&mut signed);
    signed
        .append(&(chain_id * 2 + 35 + u64::from(recovery.to_byte())))
        .append(&U256::from_big_endian(&signature[..32]))
        .append(&U256::from_big_endian(&signature[32..]));
    signed.out().to_vec()
}
fn malformed() -> Vec<u8> {
    let word = |n: u64| {
        let mut out = [0; 32];
        out[24..].copy_from_slice(&n.to_be_bytes());
        out
    };
    let mut out = vec![0xfa, 0xc7, 0xc9, 0x4a];
    out.extend(word(64));
    out.extend(word(128));
    for _ in 0..2 {
        out.extend(word(1));
        out.push(0xc0);
        out.extend([0; 31]);
    }
    out
}

#[derive(Clone, Copy)]
enum TestCorrectionPolicy {
    Fixed(CorrectionStage),
    Activation(u64),
}

impl TestCorrectionPolicy {
    fn apply(self, chain: FinalChain) -> FinalChain {
        match self {
            Self::Fixed(stage) => chain.with_correction_stage(stage),
            Self::Activation(period) => chain.with_correction_activation(Some(period.into())),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ProbeReport {
    facts: PbftExecutionContextFacts,
    outcomes: Vec<(PbftExecutionValidation, Observed)>,
}

#[test]
fn ordered_real_genesis_prefix_native_hard_error_and_reopen() -> Result<()> {
    let directory = scratch();
    ordered_probe_scenario(
        &directory,
        true,
        TestCorrectionPolicy::Fixed(CorrectionStage::SlashingHardError),
        false,
        false,
    )?;
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
fn period_probe_accepts_constructor_period_selection() -> Result<()> {
    compare_policy_probes(
        TestCorrectionPolicy::Activation(1),
        CorrectionStage::SlashingHardError,
        false,
        false,
    )
}

#[test]
fn period_probe_rejects_schedule_stage_mismatch() -> Result<()> {
    compare_policy_probes(
        TestCorrectionPolicy::Activation(2),
        CorrectionStage::Historical,
        true,
        true,
    )
}

fn compare_policy_probes(
    scheduled: TestCorrectionPolicy,
    fixed: CorrectionStage,
    historical: bool,
    check_mismatch: bool,
) -> Result<()> {
    let directory = scratch();
    let scheduled =
        ordered_probe_scenario(&directory, true, scheduled, historical, check_mismatch)?;
    let before_application = logical_rows(&directory.join("application"), true)?;
    let before_concrete = logical_rows(&directory.join("concrete"), false)?;
    let fixed = ordered_probe_scenario(
        &directory,
        false,
        TestCorrectionPolicy::Fixed(fixed),
        historical,
        false,
    )?;
    assert_eq!(scheduled.facts, fixed.facts);
    assert_eq!(scheduled, fixed);
    assert_eq!(
        logical_rows(&directory.join("application"), true)?,
        before_application
    );
    assert_eq!(
        logical_rows(&directory.join("concrete"), false)?,
        before_concrete
    );
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

fn ordered_probe_scenario(
    directory: &Path,
    initialize: bool,
    correction: TestCorrectionPolicy,
    historical: bool,
    check_mismatch: bool,
) -> Result<ProbeReport> {
    let fixture = fixture();
    let mut report = Vec::new();
    std::fs::create_dir_all(directory)?;
    let application_path = directory.join("application");
    let concrete_path = directory.join("concrete");
    let (application, chain) = mixed_genesis::open_chain(&application_path, &fixture)?;
    let mut chain = correction.apply(chain);
    let concrete = if initialize {
        mixed_genesis::fresh_concrete(&concrete_path, &chain, &fixture)?
    } else {
        let observed = ConcreteStateLifecycle::inspect_existing(
            &concrete_path,
            chain.concrete_chain_identity()?,
        )?;
        ConcreteStateLifecycle::open(
            &concrete_path,
            chain.concrete_chain_identity()?,
            observed.committed,
        )?
    };
    chain.hydrate_concrete_genesis_accounts(concrete.prior_reader())?;
    initialize_and_recover_final_chain_application_state_at_joint_startup(
        &chain,
        &StartupLeaf(&concrete),
    )?;
    let observed = concrete.observation()?;
    let policy = PbftSystemExecutionPolicy::new(
        bytes(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"])
            .try_into()
            .unwrap(),
        0,
        1000,
        fixture["configuration"]["dpos"]["delegation_delay"]
            .as_u64()
            .unwrap(),
    )?;
    let key = SigningKey::from_slice(&bytes(&fixture["inputs"]["pbft_private_key"])).unwrap();
    let author = hash(&key.verifying_key().to_encoded_point(false).as_bytes()[1..])[12..]
        .try_into()
        .unwrap();
    // This synthetic kernel context has no qualified native digest or enrollment.
    let facts = PbftExecutionContextFacts {
        schedule: PbftExecutionSchedule::new(
            fixture["configuration"]["chain_id"].as_u64().unwrap(),
            application
                .metadata()
                .genesis_hash()?
                .unwrap()
                .try_into()
                .unwrap(),
            hash(&serde_json::to_vec(&fixture["configuration"])?),
            Some(if historical { 2.into() } else { 1.into() }),
        ),
        period: 1.into(),
        previous_pbft_hash: [0; 32],
        prior: observed.committed,
        native_prior_digest: [0; 32],
        delayed: chain.pending_delayed_identity(1.into(), 0.into())?,
        concrete_provenance_digest: hash(&observed.provenance_rlp),
        concrete_generation: observed.generation,
        owner_generation: 1,
        state_api_epoch: 1,
        instruction_phase: chain.pbft_execution_instruction_phase(1.into()),
        cornus: chain.pbft_execution_cornus(1.into()),
        author,
        timestamp: 1,
        gas_limit: chain.pbft_execution_block_gas_limit().as_u64(),
        system_policy_digest: policy.digest(),
    };
    drop(concrete);
    drop(chain);
    drop(application);
    let before_application = logical_rows(&application_path, true)?;
    let before_concrete = logical_rows(&concrete_path, false)?;
    let (application, chain) = mixed_genesis::open_chain(&application_path, &fixture)?;
    let mut chain = correction.apply(chain);
    let reader = rustaxa_storage::ConcreteStateReader::open_historical_read_only(
        &concrete_path,
        observed.committed,
        observed.committed,
    )?;
    chain.hydrate_concrete_genesis_accounts(&reader)?;
    drop(reader);
    let owner = ConcreteStateLifecycle::open(
        &concrete_path,
        chain.concrete_chain_identity()?,
        observed.committed,
    )?;
    initialize_and_recover_final_chain_application_state_at_joint_startup(
        &chain,
        &StartupLeaf(&owner),
    )?;
    drop(owner);
    let binding = SyntheticKernelBinding {
        chain: &chain,
        path: &concrete_path,
        expected: facts.clone(),
    };
    let prefix: Vec<_> = fixture["periods"][0]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tx| bytes(&tx["signed_rlp"]))
        .chain(std::iter::once(bytes(
            &fixture["periods"][1]["transactions"][0]["signed_rlp"],
        )))
        .collect();
    let good = request(&fixture, facts.clone(), prefix.clone());
    if check_mismatch {
        let owner = ConcreteStateLifecycle::open(
            &concrete_path,
            chain.concrete_chain_identity()?,
            observed.committed,
        )?;
        let before = owner.observation()?;
        let mut corrected = facts.clone();
        corrected.schedule = PbftExecutionSchedule::new(
            corrected.schedule.chain_id(),
            application
                .metadata()
                .genesis_hash()?
                .unwrap()
                .try_into()
                .unwrap(),
            hash(&serde_json::to_vec(&fixture["configuration"])?),
            Some(1.into()),
        );
        let mismatch = request(&fixture, corrected.clone(), prefix.clone());
        let mismatch_binding = BorrowedSyntheticKernelBinding {
            chain: &chain,
            owner: &owner,
            expected: corrected,
        };
        let mut events = Observed::default();
        let result = probe_period_with_observer(
            &chain,
            owner.borrow_execution_probe()?,
            Arc::new(owner.open_retained_checkpoint(facts.delayed)?),
            &mismatch,
            &policy,
            &mismatch_binding,
            &mut events,
        )?;
        assert_eq!(
            result,
            PbftExecutionValidation::Unavailable {
                request_digest: mismatch.digest(),
                reason: PbftExecutionUnavailable::UnsupportedRules,
            }
        );
        assert!(events.entered.is_empty());
        assert!(events.completed.is_empty());
        assert_eq!(owner.observation()?, before);
    }
    let open = || -> Result<(ConcreteExecutionProbe, Arc<ConcreteCheckpointReaders>)> {
        let retained = if observed.committed == facts.delayed {
            vec![observed.committed]
        } else {
            vec![observed.committed, facts.delayed]
        };
        Ok((
            ConcreteExecutionProbe::open_read_only(
                &concrete_path,
                observed.committed,
                observed.committed,
            )?,
            Arc::new(ConcreteCheckpointReaders::open_read_only(
                &concrete_path,
                observed.committed,
                retained,
            )?),
        ))
    };
    let (probe, delayed) = open()?;
    let mut positive = Observed::default();
    let positive_result = probe_period_with_observer(
        &chain,
        probe,
        delayed,
        &good,
        &policy,
        &binding,
        &mut positive,
    )?;
    assert!(matches!(
        positive_result,
        PbftExecutionValidation::Valid {
            executed_transactions: 12,
            ..
        }
    ));
    assert_eq!(positive.entered, (0..12).collect::<Vec<_>>());
    assert_eq!(positive.completed.len(), 12);
    assert_eq!(
        positive
            .completed
            .iter()
            .map(|event| event.0)
            .collect::<Vec<_>>(),
        (0..12).collect::<Vec<_>>()
    );
    let (_, result, native, raw_writes, _) = &positive.completed[11];
    assert!(matches!(
        result,
        rustaxa_evm::contracts::TransactionExecutionResult::Executed(
            rustaxa_evm::contracts::ExecutedTransactionResult {
                status: rustaxa_evm::contracts::CodeExecutionStatus::Success,
                ..
            }
        )
    ));
    assert_eq!(native.len(), 1);
    assert_eq!(
        native[0].status,
        rustaxa_evm::contracts::CodeExecutionStatus::Success
    );
    assert_eq!(native[0].invocation.id.sequence, 0);
    assert_eq!(native[0].invocation.id.transaction.as_u32(), 11);
    assert_eq!(
        native[0].disposition,
        rustaxa_evm::contracts::ConsensusNativeDisposition::Normal
    );
    report.push((positive_result.clone(), positive.clone()));
    let mut dpos = [0; 20];
    dpos[19] = 0xfe;
    assert_eq!(native[0].invocation.contract, dpos);
    assert_eq!(native[0].invocation.state_address, dpos);
    assert_eq!(
        native[0].invocation.caller.as_slice(),
        bytes(&fixture["inputs"]["validator_owner"]).as_slice()
    );
    let mut commission_input = vec![0xf0, 0x00, 0x32, 0x2c];
    commission_input.extend([0; 12]);
    commission_input.extend(bytes(&fixture["inputs"]["validator"]));
    commission_input.extend([0; 31]);
    commission_input.push(100);
    assert_eq!(native[0].invocation.input, commission_input);
    assert_eq!(native[0].gas_used, native[0].required_gas);
    assert!(*raw_writes > 0);
    let mut slash = [0; 20];
    slash[19] = 0xee;
    let mut ordinary_controls = Vec::new();
    for (receiver, gas, status) in [
        (
            [0x44; 20],
            100_000,
            rustaxa_evm::contracts::CodeExecutionStatus::Success,
        ),
        (
            slash,
            30_000,
            rustaxa_evm::contracts::CodeExecutionStatus::Failure(
                rustaxa_evm::contracts::CodeExecutionError::OutOfGas,
            ),
        ),
    ] {
        let mut ordinary = prefix.clone();
        ordinary.push(transaction(&fixture, 12, receiver, &malformed(), gas));
        ordinary.push(transaction(&fixture, 13, [0x44; 20], &[], 100_000));
        let ordinary = request(&fixture, facts.clone(), ordinary);
        let (probe, delayed) = open()?;
        let mut events = Observed::default();
        let ordinary_result = probe_period_with_observer(
            &chain,
            probe,
            delayed,
            &ordinary,
            &policy,
            &binding,
            &mut events,
        )?;
        assert!(matches!(
            ordinary_result,
            PbftExecutionValidation::Valid {
                executed_transactions: 14,
                ..
            }
        ));
        assert_eq!(events.entered, (0..14).collect::<Vec<_>>());
        assert_eq!(events.completed.len(), 14);
        assert_eq!(events.completed[..12], positive.completed);
        let rustaxa_evm::contracts::TransactionExecutionResult::Executed(result) =
            &events.completed[12].1
        else {
            panic!("ordinary control must pass transaction admission");
        };
        assert_eq!(result.status, status);
        assert!(result.logs.is_empty());
        assert!(matches!(
            &events.completed[13].1,
            rustaxa_evm::contracts::TransactionExecutionResult::Executed(
                rustaxa_evm::contracts::ExecutedTransactionResult {
                    status: rustaxa_evm::contracts::CodeExecutionStatus::Success,
                    ..
                }
            )
        ));
        report.push((ordinary_result.clone(), events.clone()));
        ordinary_controls.push((ordinary, events, ordinary_result));
    }
    let mut bad = prefix;
    bad.push(transaction(&fixture, 12, slash, &malformed(), 1_000_000));
    bad.push(transaction(&fixture, 13, [0x44; 20], &[], 100_000));
    let bad = request(&fixture, facts.clone(), bad);
    let mut negative_results = Vec::new();
    for _ in 0..2 {
        let (probe, delayed) = open()?;
        let mut attempt_events = Observed::default();
        let invalid_result = probe_period_with_observer(
            &chain,
            probe,
            delayed,
            &bad,
            &policy,
            &binding,
            &mut attempt_events,
        )?;
        if historical {
            assert_historical_malformed(&invalid_result, &attempt_events, &positive);
        } else {
            assert!(matches!(
                invalid_result,
                PbftExecutionValidation::Invalid {
                    transaction_position: 12,
                    cause: PbftExecutionHardError::MalformedNestedSlashingProof(_),
                    ..
                }
            ));
            assert_eq!(attempt_events.entered, (0..13).collect::<Vec<_>>());
            assert_eq!(attempt_events.completed, positive.completed);
        }
        report.push((invalid_result.clone(), attempt_events.clone()));
        let after = ConcreteStateLifecycle::inspect_existing(
            &concrete_path,
            chain.concrete_chain_identity()?,
        )?;
        assert_eq!(after.committed, observed.committed);
        assert_eq!(after.generation, observed.generation);
        assert_eq!(after.pending_marker_rlp, observed.pending_marker_rlp);
        assert_eq!(after.provenance_rlp, observed.provenance_rlp);
        negative_results.push((invalid_result, attempt_events));
    }
    // A hard failure does not poison a fresh candidate's sequence or state.
    let (probe, delayed) = open()?;
    let mut fresh = Observed::default();
    let fresh_result =
        probe_period_with_observer(&chain, probe, delayed, &good, &policy, &binding, &mut fresh)?;
    assert!(matches!(
        fresh_result,
        PbftExecutionValidation::Valid {
            executed_transactions: 12,
            ..
        }
    ));
    assert_eq!(fresh.entered, positive.entered);
    assert_eq!(fresh.completed, positive.completed);
    report.push((fresh_result, fresh.clone()));
    let (probe, delayed) = open()?;
    assert_eq!(
        probe_period(&chain, probe, delayed, &good, &policy, &binding)?,
        positive_result
    );
    {
        let owner = ConcreteStateLifecycle::open(
            &concrete_path,
            chain.concrete_chain_identity()?,
            observed.committed,
        )?;
        let owner_before = owner.observation()?;
        let borrowed_binding = BorrowedSyntheticKernelBinding {
            chain: &chain,
            owner: &owner,
            expected: facts.clone(),
        };
        let delayed = || -> Result<Arc<ConcreteCheckpointReaders>> {
            let selected = chain.pending_delayed_identity(facts.period, facts.prior.period)?;
            ensure!(selected == facts.delayed);
            Ok(Arc::new(owner.open_retained_checkpoint(selected)?))
        };
        let run = |request: &PbftExecutionRequest, events: &mut Observed| {
            probe_period_with_observer(
                &chain,
                owner.borrow_execution_probe()?,
                delayed()?,
                request,
                &policy,
                &borrowed_binding,
                events,
            )
        };
        let mut borrowed_positive = Observed::default();
        assert_eq!(run(&good, &mut borrowed_positive)?, positive_result);
        assert_eq!(borrowed_positive.entered, positive.entered);
        assert_eq!(borrowed_positive.completed, positive.completed);
        assert_eq!(owner.observation()?, owner_before);
        for (ordinary, expected_events, expected_result) in ordinary_controls {
            let mut events = Observed::default();
            assert_eq!(run(&ordinary, &mut events)?, expected_result);
            assert_eq!(events.entered, expected_events.entered);
            assert_eq!(events.completed, expected_events.completed);
            assert_eq!(owner.observation()?, owner_before);
        }
        for (expected, expected_events) in negative_results {
            let mut events = Observed::default();
            assert_eq!(run(&bad, &mut events)?, expected);
            if historical {
                assert_historical_malformed(&expected, &events, &positive);
            } else {
                assert_eq!(events.entered, (0..13).collect::<Vec<_>>());
                assert_eq!(events.completed, positive.completed);
            }
            assert_eq!(events, expected_events);
            assert_eq!(owner.observation()?, owner_before);
        }
        let mut fresh = Observed::default();
        assert_eq!(run(&good, &mut fresh)?, positive_result);
        assert_eq!(fresh.entered, positive.entered);
        assert_eq!(fresh.completed, positive.completed);
        assert_eq!(
            probe_period(
                &chain,
                owner.borrow_execution_probe()?,
                delayed()?,
                &good,
                &policy,
                &borrowed_binding,
            )?,
            positive_result
        );
        assert_eq!(owner.observation()?, owner_before);
    }
    drop(binding);
    drop(chain);
    drop(application);
    assert_eq!(logical_rows(&application_path, true)?, before_application);
    assert_eq!(logical_rows(&concrete_path, false)?, before_concrete);
    Ok(ProbeReport {
        facts,
        outcomes: report,
    })
}

fn assert_historical_malformed(
    result: &PbftExecutionValidation,
    events: &Observed,
    positive: &Observed,
) {
    use rustaxa_evm::contracts::{
        CodeExecutionError, CodeExecutionStatus, ExecutedTransactionResult,
        TransactionExecutionResult,
    };
    assert!(matches!(
        result,
        PbftExecutionValidation::Valid {
            executed_transactions: 14,
            ..
        }
    ));
    assert_eq!(events.entered, (0..14).collect::<Vec<_>>());
    assert_eq!(events.completed.len(), 14);
    assert_eq!(events.completed[..12], positive.completed);
    assert!(matches!(
        &events.completed[12].1,
        TransactionExecutionResult::Executed(ExecutedTransactionResult {
            status: CodeExecutionStatus::Failure(CodeExecutionError::Native(_)),
            ..
        })
    ));
    assert!(matches!(
        &events.completed[13].1,
        TransactionExecutionResult::Executed(ExecutedTransactionResult {
            status: CodeExecutionStatus::Success,
            ..
        })
    ));
}

#[test]
fn policy_digest_binds_every_configured_field() -> Result<()> {
    let policy = PbftSystemExecutionPolicy::new([1; 20], 10, 100, 5)?;
    for other in [
        PbftSystemExecutionPolicy::new([2; 20], 10, 100, 5)?,
        PbftSystemExecutionPolicy::new([1; 20], 11, 100, 5)?,
        PbftSystemExecutionPolicy::new([1; 20], 10, 101, 5)?,
        PbftSystemExecutionPolicy::new([1; 20], 10, 100, 6)?,
    ] {
        assert_ne!(policy.digest(), other.digest());
    }
    assert!(PbftSystemExecutionPolicy::new([1; 20], 0, 0, 0).is_err());
    Ok(())
}

type LogicalRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;

/// All writable owners and retained views must be closed before this read.
fn logical_rows(path: &Path, application: bool) -> Result<LogicalRows> {
    let directory = if application {
        path.join("db")
    } else {
        path.to_path_buf()
    };
    let path = directory.as_path();
    let options = rocksdb::Options::default();
    let mut columns = rocksdb::DB::list_cf(&options, path)?;
    columns.sort();
    let descriptors = columns
        .iter()
        .map(|name| {
            if application && name != "default" {
                Ok(rustaxa_storage::Column::from_name(name)?.descriptor(&options))
            } else {
                if !application {
                    ensure!(
                        matches!(
                            name.as_str(),
                            "default" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8"
                        ),
                        "unknown concrete column"
                    );
                }
                Ok(rocksdb::ColumnFamilyDescriptor::new(name, options.clone()))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
    columns
        .into_iter()
        .map(|column| {
            let handle = db.cf_handle(&column).context("logical column")?;
            let values = db
                .iterator_cf(&handle, rocksdb::IteratorMode::Start)
                .map(|row| row.map(|(key, value)| (key.to_vec(), value.to_vec())))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((column, values))
        })
        .collect()
}
