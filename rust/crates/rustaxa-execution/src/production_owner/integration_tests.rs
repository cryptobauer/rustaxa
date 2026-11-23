use super::*;
use anyhow::Context;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
enum Behavior {
    Ingress,
    Stop,
    Error,
    Panic,
    WrongReport,
    Replace,
}

struct TestProcess<'a> {
    application: &'a ProductionApplication,
    transactions: &'a [PublicTransactionSubmissionRequest],
    behavior: Behavior,
    waits: AtomicU64,
    generation: AtomicU64,
    last_tick: AtomicU64,
    durable_transaction_count: u64,
}

impl ConsensusProcessPort for TestProcess<'_> {
    fn now_millis(&self) -> u64 {
        self.waits.load(Ordering::Relaxed) * 100
    }
    fn unix_time_seconds(&self) -> u64 {
        1
    }
    fn stop_requested(&self, _: u64) -> bool {
        false
    }
    fn wait(&self, request: &ConsensusWaitRequest) -> Result<ConsensusWaitReport> {
        let index = self.waits.fetch_add(1, Ordering::Relaxed);
        let generation = self.generation.load(Ordering::Relaxed);
        if generation == 0 {
            self.generation
                .store(request.effect_id.generation, Ordering::Relaxed);
        } else {
            ensure!(
                generation == request.effect_id.generation,
                "PRODUCTION_TEST_RUNTIME_RESTARTED"
            );
        }
        let tick = self
            .application
            .inner
            .application
            .integration_runtime_tick()?;
        ensure!(
            tick > self.last_tick.swap(tick, Ordering::Relaxed),
            "PRODUCTION_TEST_STALE_TICK_REUSED"
        );
        ensure!(
            self.application.network_client().cancel_egress(17).is_err(),
            "PRODUCTION_TEST_WAIT_REENTRY_ACCEPTED"
        );
        if !matches!(self.behavior, Behavior::Ingress | Behavior::Replace) {
            let application = self.application.clone();
            let transaction = self.transactions[0].clone();
            std::thread::spawn(move || -> Result<()> {
                let report = application
                    .network_client()
                    .submit_transaction_from_native_state(transaction)?;
                ensure!(
                    !report.transaction_observed,
                    "PRODUCTION_TEST_DUPLICATE_EFFECT"
                );
                Ok(())
            })
            .join()
            .map_err(|_| anyhow::anyhow!("PRODUCTION_TEST_DUPLICATE_INGRESS_UNWOUND"))??;
        }
        match self.behavior {
            Behavior::Ingress if (index as usize) < self.transactions.len() => {
                let application = self.application.clone();
                let transaction = self.transactions[index as usize].clone();
                std::thread::spawn(move || -> Result<()> {
                    let client = application.network_client();
                    let report = client.submit_transaction_from_native_state(transaction)?;
                    ensure!(
                        report.accepted && report.transaction_observed,
                        "PRODUCTION_TEST_TRANSACTION_REFUSED: {}",
                        report.message
                    );
                    ensure!(
                        application
                            .query_client()
                            .live_transaction_is_known(report.transaction_hash.0)?,
                        "PRODUCTION_TEST_TRANSACTION_NOT_OBSERVED"
                    );
                    Ok(())
                })
                .join()
                .map_err(|_| anyhow::anyhow!("PRODUCTION_TEST_INGRESS_UNWOUND"))??;
                let pool = self.application.query_client().transaction_pool_status()?;
                ensure!(
                    pool.queue_size == index + 1
                        && pool.transaction_count == self.durable_transaction_count,
                    "PRODUCTION_TEST_TRANSACTION_COUNTS: {pool:?}"
                );
            }
            Behavior::Ingress | Behavior::Stop => {
                return Ok(ConsensusWaitReport {
                    effect_id: request.effect_id,
                    outcome: ConsensusWaitOutcome::Stopped,
                });
            }
            Behavior::Error => bail!("PRODUCTION_TEST_CALLBACK_ERROR"),
            Behavior::Panic => panic!("PRODUCTION_TEST_CALLBACK_UNWIND"),
            Behavior::WrongReport => {
                return Ok(ConsensusWaitReport {
                    effect_id: ConsensusEffectId {
                        generation: request.effect_id.generation,
                        sequence: request.effect_id.sequence + 1,
                    },
                    outcome: ConsensusWaitOutcome::Elapsed,
                });
            }
            Behavior::Replace => {
                let application = self.application.clone();
                std::thread::spawn(move || {
                    application
                        .inner
                        .application
                        .integration_replace_runtime_tick()
                })
                .join()
                .map_err(|_| anyhow::anyhow!("PRODUCTION_TEST_REPLACEMENT_UNWOUND"))??;
            }
        }
        Ok(ConsensusWaitReport {
            effect_id: request.effect_id,
            outcome: ConsensusWaitOutcome::Elapsed,
        })
    }
}

struct TestPorts;
impl ConsensusSigningPort for TestPorts {
    fn sign_digest(&self, _: &ConsensusSignRequest) -> Result<ConsensusSignReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_SIGNING")
    }
    fn prove_vrf(&self, _: &ConsensusVrfRequest) -> Result<ConsensusVrfReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_VRF")
    }
}
impl ConsensusTransportPort for TestPorts {
    fn transport_status(&self) -> ConsensusTransportStatus {
        ConsensusTransportStatus {
            available: false,
            packet_queue_over_limit: false,
        }
    }
    fn gossip_vote(&self, _: &GossipVoteRequest) -> Result<ConsensusTransportReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_GOSSIP")
    }
    fn gossip_vote_bundle(&self, _: &GossipVoteBundleRequest) -> Result<ConsensusTransportReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_GOSSIP")
    }
    fn gossip_pillar_vote(&self, _: &GossipPillarVoteRequest) -> Result<ConsensusTransportReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_GOSSIP")
    }
    fn report_malicious_peer(
        &self,
        _: &ReportMaliciousPeerRequest,
    ) -> Result<ConsensusTransportReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_PEER_REPORT")
    }
}
impl ConsensusObserverPort for TestPorts {
    fn observe(&self, _: &ConsensusObservationRequest) -> Result<ConsensusObservationReport> {
        bail!("PRODUCTION_TEST_UNEXPECTED_OBSERVATION")
    }
}

pub fn run_native_fixture_checks(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    vrf_secret: [u8; 64],
) -> Result<String> {
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let identity = config
        .consensus
        .signing_identities
        .first()
        .context("fixture signing identity")?;
    ensure!(
        application
            .query_client()
            .final_chain_dpos_eligible_vote_count(0, identity.address)?
            == 0,
        "PRODUCTION_TEST_LOCAL_KEY_MUST_BE_INELIGIBLE"
    );
    if selected.bootstrap().final_chain_genesis_validators.len() == 2 {
        return run_certified_fixture_checks(application, selected, secret, vrf_secret);
    }
    let initial_pool = application.query_client().transaction_pool_status()?;
    ensure!(
        initial_pool.queue_size == 0,
        "PRODUCTION_TEST_INITIAL_QUEUE_NOT_EMPTY"
    );
    let durable_transaction_count = initial_pool.transaction_count;
    let transactions = (0..3)
        .map(|nonce| {
            rustaxa_consensus::production_bootstrap::signed_fixture_transaction(
                selected, secret, nonce,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    for behavior in [
        Behavior::Ingress,
        Behavior::Stop,
        Behavior::Error,
        Behavior::WrongReport,
        Behavior::Panic,
        Behavior::Stop,
        Behavior::Replace,
    ] {
        let process = TestProcess {
            application,
            transactions: &transactions,
            behavior,
            waits: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            last_tick: AtomicU64::new(0),
            durable_transaction_count,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            application.run(&process, &TestPorts, &TestPorts, &TestPorts)
        }));
        match behavior {
            Behavior::Panic => ensure!(result.is_err(), "PRODUCTION_TEST_CALLBACK_DID_NOT_UNWIND"),
            Behavior::Error | Behavior::WrongReport | Behavior::Replace => {
                let result =
                    result.map_err(|_| anyhow::anyhow!("PRODUCTION_TEST_UNEXPECTED_UNWIND"))?;
                let error = match result {
                    Ok(_) => bail!("PRODUCTION_TEST_ERROR_BECAME_SUCCESS"),
                    Err(error) => error,
                };
                let expected = match behavior {
                    Behavior::Error => "PRODUCTION_TEST_CALLBACK_ERROR",
                    Behavior::WrongReport => "STALE_EFFECT_REPORT",
                    Behavior::Replace => "SESSION_REPLACED",
                    _ => unreachable!(),
                };
                ensure!(error.to_string().contains(expected), "{error}");
                if matches!(behavior, Behavior::Replace) {
                    ensure!(
                        application.inner.application.integration_runtime_tick()? > 0,
                        "PRODUCTION_TEST_REPLACEMENT_REMOVED"
                    );
                }
            }
            Behavior::Ingress | Behavior::Stop => {
                let exit =
                    result.map_err(|_| anyhow::anyhow!("PRODUCTION_TEST_UNEXPECTED_UNWIND"))??;
                ensure!(
                    exit.reason == ConsensusRunReason::Stopped,
                    "PRODUCTION_TEST_STOP_RESULT"
                );
                if matches!(behavior, Behavior::Ingress) {
                    ensure!(
                        process.waits.load(Ordering::Relaxed) == 4,
                        "PRODUCTION_TEST_WAIT_COUNT"
                    );
                }
            }
        }
        if !matches!(behavior, Behavior::Replace) {
            ensure!(
                application
                    .inner
                    .application
                    .integration_runtime_tick()
                    .is_err(),
                "PRODUCTION_TEST_SUSPENDED_CURSOR_RETAINED"
            );
        }
        let pool = application.query_client().transaction_pool_status()?;
        ensure!(
            pool.queue_size == 3 && pool.transaction_count == durable_transaction_count,
            "PRODUCTION_TEST_PRIOR_TRANSACTION_EFFECT_CHANGED: {pool:?}"
        );
    }
    let transition = run_transition_checks(application, selected, secret, &transactions)?;
    Ok(format!(
        "7 native runtime cases passed; 3 transactions admitted during waits in one run; fresh tick IDs; shutdown, callback errors, unwind, wrong report, and exact replacement preserved; {transition}"
    ))
}

fn run_transition_checks(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    transactions: &[PublicTransactionSubmissionRequest],
) -> Result<String> {
    use rustaxa_evm::contracts::{CodeExecutionStatus, TransactionExecutionResult};
    use rustaxa_types::concrete_state::ConcreteRead;
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let timestamp = config
        .final_chain
        .genesis_timestamp
        .checked_add(1)
        .context("fixture timestamp")?;
    let raw: Vec<_> = transactions
        .iter()
        .map(|t| t.transaction_rlp.clone())
        .collect();
    let (candidate, empty, bad_order) = application
        .inner
        .application
        .integration_transition_candidates(&raw, secret, timestamp)?;
    let before = application
        .inner
        .application
        .integration_transition_database_images()?;
    let path = config.storage_path.join("transition-evidence");
    std::fs::create_dir(&path)?;
    super::publication_tests::save_image(&before.0, &path.join("transition-before-primary.rlp"))?;
    super::publication_tests::save_image(&before.1, &path.join("transition-before-concrete.rlp"))?;
    let unchanged = |name: &str| -> Result<()> {
        let after = application
            .inner
            .application
            .integration_transition_database_images()?;
        if after != before {
            super::publication_tests::save_image(
                &after.0,
                &path.join(format!("transition-failed-{name}-primary.rlp")),
            )?;
            super::publication_tests::save_image(
                &after.1,
                &path.join(format!("transition-failed-{name}-concrete.rlp")),
            )?;
            bail!("PRODUCTION_TRANSITION_DATABASE_CHANGED: {name}");
        }
        Ok(())
    };
    let prepared = application.prepare_transition(&candidate)?;
    ensure!(
        prepared.results().count() == 3,
        "PRODUCTION_TRANSITION_RESULT_COUNT"
    );
    for ((transaction, result), request) in prepared.results().zip(transactions) {
        let mut hash = Keccak::v256();
        let mut expected = [0; 32];
        hash.update(&request.transaction_rlp);
        hash.finalize(&mut expected);
        ensure!(
            transaction.hash == expected,
            "PRODUCTION_TRANSITION_INPUT_ORDER"
        );
        ensure!(
            matches!(result,TransactionExecutionResult::Executed(r)if matches!(r.status,CodeExecutionStatus::Success)&&r.gas_used.as_u64()==21_000&&r.logs.is_empty()),
            "PRODUCTION_TRANSITION_TRANSFER_RESULT"
        );
    }
    let recipient = prepared
        .changed_accounts()
        .iter()
        .find(|r| r.address == [0x79; 20])
        .context("prepared recipient account")?;
    ensure!(
        matches!(&recipient.account,ConcreteRead::Present(r)if r.account.balance.value()==&num_bigint::BigUint::from(3_u8)),
        "PRODUCTION_TRANSITION_RECIPIENT_BALANCE"
    );
    let identity = prepared.next_identity();
    let digest = prepared.request_digest();
    ensure!(
        identity.period.as_u64() == 1,
        "PRODUCTION_TRANSITION_PERIOD"
    );
    unchanged("prepared")?;
    drop(prepared);
    unchanged("dropped")?;
    let error = application
        .prepare_transition(&bad_order)
        .err()
        .context("bad order accepted")?;
    ensure!(
        error
            .to_string()
            .contains("PBFT_INPUT_CANDIDATE_ORDER_MISMATCH"),
        "PRODUCTION_TRANSITION_BAD_ORDER_ERROR: {error}"
    );
    unchanged("rejected")?;
    let fresh = application.prepare_transition(&candidate)?;
    ensure!(
        fresh.next_identity() == identity
            && fresh.request_digest() == digest
            && fresh.results().count() == 3,
        "PRODUCTION_TRANSITION_FRESH_RESULT"
    );
    drop(fresh);
    unchanged("fresh")?;
    let empty = application.prepare_transition(&empty)?;
    ensure!(
        empty.results().count() == 0,
        "PRODUCTION_TRANSITION_EMPTY_RESULT"
    );
    drop(empty);
    unchanged("empty")?;
    let after = application
        .inner
        .application
        .integration_transition_database_images()?;
    super::publication_tests::save_image(&after.0, &path.join("transition-after-primary.rlp"))?;
    super::publication_tests::save_image(&after.1, &path.join("transition-after-concrete.rlp"))?;
    Ok(format!(
        "PRODUCTION_TRANSITION_CHECKS_PASSED: ordered=3 recipient_balance=3 native_rewards_finished=true prepared_drop_unchanged=true bad_order_rejected=true fresh_valid=true empty_valid=true primary_columns={} concrete_columns={} complete_WAL_images_equal=true",
        before.0.columns.len(),
        before.1.columns.len()
    ))
}

struct FirstWritePorts(AtomicU64);
impl FirstWritePorts {
    fn reject<T>(&self) -> Result<T> {
        self.0.fetch_add(1, Ordering::Relaxed);
        bail!("PRODUCTION_FIRST_WRITE_UNEXPECTED_CALLBACK")
    }
}
impl ConsensusSigningPort for FirstWritePorts {
    fn sign_digest(&self, _: &ConsensusSignRequest) -> Result<ConsensusSignReport> {
        self.reject()
    }
    fn prove_vrf(&self, _: &ConsensusVrfRequest) -> Result<ConsensusVrfReport> {
        self.reject()
    }
}
impl ConsensusTransportPort for FirstWritePorts {
    fn transport_status(&self) -> ConsensusTransportStatus {
        ConsensusTransportStatus {
            available: true,
            packet_queue_over_limit: false,
        }
    }
    fn gossip_vote(&self, _: &GossipVoteRequest) -> Result<ConsensusTransportReport> {
        self.reject()
    }
    fn gossip_vote_bundle(&self, _: &GossipVoteBundleRequest) -> Result<ConsensusTransportReport> {
        self.reject()
    }
    fn gossip_pillar_vote(&self, _: &GossipPillarVoteRequest) -> Result<ConsensusTransportReport> {
        self.reject()
    }
    fn gossip_dag_block(&self, _: &GossipDagBlockRequest) -> Result<ConsensusTransportReport> {
        self.reject()
    }
    fn report_malicious_peer(
        &self,
        _: &ReportMaliciousPeerRequest,
    ) -> Result<ConsensusTransportReport> {
        self.reject()
    }
}
impl ConsensusObserverPort for FirstWritePorts {
    fn observe(&self, request: &ConsensusObservationRequest) -> Result<ConsensusObservationReport> {
        TestPorts.observe(request)
    }
}

fn run_certified_fixture_checks(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    vrf_secret: [u8; 64],
) -> Result<String> {
    use rustaxa_consensus::pbft_vote_generation::{
        PbftVoteGenerationInput, PbftVoteWeightFacts, generate_pbft_vote_with_weight,
    };
    use rustaxa_evm::contracts::{CodeExecutionStatus, TransactionExecutionResult};
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let validators = &selected.bootstrap().final_chain_genesis_validators;
    ensure!(
        validators.len() == 2 && validators[0].vrf_key != validators[1].vrf_key,
        "PRODUCTION_TEST_DISTINCT_VALIDATORS"
    );
    let secrets = [[0x66; 32], [0x77; 32]];
    let vrf_secrets = [
        vrf_secret,
        [
            0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57,
            0xa4, 0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8,
            0xd5, 0x1c, 0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38,
            0xdb, 0x7e, 0x28, 0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4,
            0x02, 0xea, 0x69, 0x97, 0xad, 0xe4, 0x00, 0x81,
        ],
    ];
    let certificate_secret = secrets[0];
    let transactions = (0..3)
        .map(|nonce| {
            rustaxa_consensus::production_bootstrap::signed_fixture_transaction(
                selected, secret, nonce,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    for transaction in &transactions {
        let report = application
            .network_client()
            .submit_transaction_from_native_state(transaction.clone())?;
        ensure!(
            report.accepted && report.transaction_observed,
            "PRODUCTION_TEST_CERT_TRANSACTION_ADMISSION"
        );
    }
    let raw: Vec<_> = transactions
        .iter()
        .map(|t| t.transaction_rlp.clone())
        .collect();
    let (candidate, empty, _) = application
        .inner
        .application
        .integration_transition_candidates(
            &raw,
            certificate_secret,
            config
                .final_chain
                .genesis_timestamp
                .checked_add(1)
                .context("certified timestamp")?,
        )?;
    let block = rustaxa_types::pbft::PbftBlockLink::try_from(
        rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(&candidate),
    )?;
    let total = application
        .query_client()
        .final_chain_dpos_eligible_total_vote_count(0)?;
    let counts = validators
        .iter()
        .map(|validator| {
            application
                .query_client()
                .final_chain_dpos_eligible_vote_count(0, validator.address)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        counts[0] > 0
            && counts[0] == counts[1]
            && counts[0].checked_mul(2) == Some(total)
            && config.consensus.pbft.committee_size >= total,
        "PRODUCTION_TEST_EQUAL_NATIVE_VOTERS"
    );
    let generate = |index: usize, round: u64| {
        generate_pbft_vote_with_weight(
            PbftVoteGenerationInput {
                block_hash: block.block_hash,
                vote_type: PbftVoteType::Cert,
                period: block.period,
                round,
                step: 3,
                node_secret: secrets[index],
                vrf_secret: vrf_secrets[index],
                expected_voter: ethereum_types::H160(validators[index].address),
                expected_vrf_public_key: validators[index].vrf_key,
            },
            PbftVoteWeightFacts {
                voter_dpos_vote_count: counts[index],
                total_dpos_vote_count: total,
                committee_size: config.consensus.pbft.committee_size,
                number_of_proposers: config.consensus.pbft.number_of_proposers,
            },
        )
    };
    let mut first_votes = Vec::new();
    let mut first_weights = Vec::new();
    for round in [1, 2] {
        let mut inputs = Vec::new();
        for index in 0..2 {
            let vote = generate(index, round)?;
            ensure!(
                vote.accepted && vote.has_weight && vote.weight > 0,
                "PRODUCTION_TEST_CERT_GENERATION: {}",
                vote.error_code
            );
            if round == 1 {
                first_votes.push(vote.vote_rlp.clone());
                first_weights.push(vote.weight);
            }
            inputs.push(vote.vote_rlp);
        }
        application
            .inner
            .application
            .integration_admit_current_certificate(&candidate, inputs)?;
    }
    let threshold = total.checked_mul(2).context("fixture threshold overflow")? / 3 + 1;
    ensure!(
        first_weights[0] < threshold
            && first_weights[0]
                .checked_add(first_weights[1])
                .context("fixture sum overflow")?
                >= threshold,
        "PRODUCTION_TEST_REAL_QUORUM_CASE"
    );
    application
        .inner
        .application
        .integration_retain_certificate_test_input(
            &first_votes[0],
            first_weights[0],
            Some((3, secrets[0])),
        )?;
    let other = generate(1, 3)?;
    ensure!(
        other.accepted && other.has_weight,
        "PRODUCTION_TEST_OTHER_CERT"
    );
    application
        .inner
        .application
        .integration_retain_certificate_test_input(&other.vote_rlp, other.weight, None)?;
    for index in 0..2 {
        let vote = generate(index, 4)?;
        ensure!(
            vote.accepted && vote.has_weight,
            "PRODUCTION_TEST_WEIGHT_CERT"
        );
        let weight = if index == 0 {
            vote.weight
                .checked_add(1)
                .context("fixture weight overflow")?
        } else {
            vote.weight
        };
        application
            .inner
            .application
            .integration_retain_certificate_test_input(&vote.vote_rlp, weight, None)?;
    }
    let partial = generate(0, 5)?;
    ensure!(
        partial.accepted && partial.has_weight && partial.weight < threshold,
        "PRODUCTION_TEST_PARTIAL_CERT"
    );
    application
        .inner
        .application
        .integration_retain_certificate_test_input(&partial.vote_rlp, partial.weight, None)?;
    application
        .inner
        .application
        .integration_retain_finalization_candidate(&candidate)?;
    // Admission and proposed-cache setup precede the complete state baseline.
    let before = application
        .inner
        .application
        .integration_transition_database_images()?;
    let path = config.storage_path.join("certified-transition-evidence");
    std::fs::create_dir(&path)?;
    super::publication_tests::save_image(&before.0, &path.join("before-primary.rlp"))?;
    super::publication_tests::save_image(&before.1, &path.join("before-concrete.rlp"))?;
    let retained_inputs = application
        .inner
        .application
        .integration_certificate_input_snapshot()?;
    let unchanged = || -> Result<()> {
        ensure!(
            application
                .inner
                .application
                .integration_certificate_input_snapshot()?
                == retained_inputs,
            "PRODUCTION_CERTIFICATE_INPUT_CHANGED"
        );

        let after = application
            .inner
            .application
            .integration_transition_database_images()?;
        if after != before {
            super::publication_tests::save_image(&after.0, &path.join("failed-primary.rlp"))?;
            super::publication_tests::save_image(&after.1, &path.join("failed-concrete.rlp"))?;
            bail!("PRODUCTION_CERTIFIED_DATABASE_CHANGED");
        }
        Ok(())
    };
    let process = TestProcess {
        application,
        transactions: &transactions,
        behavior: Behavior::Stop,
        waits: AtomicU64::new(0),
        generation: AtomicU64::new(0),
        last_tick: AtomicU64::new(0),
        durable_transaction_count: application
            .query_client()
            .transaction_pool_status()?
            .transaction_count,
    };
    let ports = FirstWritePorts(AtomicU64::new(0));
    let attempt = application.inner.application.run(
        &process,
        &ports,
        &ports,
        &super::ProductionExecutionState::Network(application),
        &ports,
    );
    eprintln!("PRODUCTION_FIRST_WRITE_ATTEMPT: {attempt:?}");
    unchanged()?;
    ensure!(
        ports.0.load(Ordering::Relaxed) == 0,
        "PRODUCTION_FIRST_WRITE_CALLBACK"
    );
    ensure!(
        process.waits.load(Ordering::Relaxed) == 0,
        "PRODUCTION_FIRST_WRITE_WAIT"
    );
    let error = attempt
        .err()
        .context("production finalization did not refuse")?;
    ensure!(
        error.to_string()
            == "PRODUCTION_EXECUTION_PLAN_UNAVAILABLE: certified preparation before writes",
        "PRODUCTION_FIRST_WRITE_WRONG_REFUSAL: {error}"
    );
    let mut control_entered = false;
    let control: Result<()> = application
        .inner
        .application
        .with_verified_certified_transition_preparation(
            &candidate,
            1,
            |_context, _builder, _native| {
                control_entered = true;
                bail!("PRODUCTION_VERIFIED_CALLBACK_CONTROL")
            },
        );
    unchanged()?;
    ensure!(
        control_entered
            && control
                .err()
                .context("verified control accepted")?
                .to_string()
                == "PRODUCTION_VERIFIED_CALLBACK_CONTROL",
        "PRODUCTION_VERIFIED_CALLBACK_CONTROL_FAILED"
    );
    let admitted = application.prepare_certified_transition(&candidate, 1)?;
    ensure!(
        admitted.into_verified().is_err(),
        "PRODUCTION_ADMITTED_RESULT_QUALIFIED"
    );
    unchanged()?;
    for (round, reason) in [(3, "proof"), (4, "sidecar"), (5, "quorum")] {
        let mut entered = false;
        let boundary: Result<()> = application
            .inner
            .application
            .with_verified_certified_transition_preparation(
                &candidate,
                round,
                |_context, _builder, _native| {
                    entered = true;
                    bail!("PRODUCTION_UNQUALIFIED_KERNEL_ENTRY")
                },
            );
        ensure!(
            !entered && boundary.is_err(),
            "PRODUCTION_UNQUALIFIED_KERNEL_ENTERED"
        );
        unchanged()?;
        let error = application
            .prepare_verified_certified_transition(&candidate, round)
            .err()
            .context("unqualified retained input accepted")?;
        if reason == "proof" {
            ensure!(
                error
                    .downcast_ref::<ProductionCertificateProofError>()
                    .is_some()
                    && error.to_string().contains("INVALID_VRF_PROOF"),
                "PRODUCTION_CERTIFICATE_WRONG_PROOF_ERROR: {error}"
            );
        } else {
            ensure!(
                error
                    .downcast_ref::<ProductionPreparationUnavailable>()
                    .is_some()
                    && error.to_string().contains(reason),
                "PRODUCTION_CERTIFICATE_WRONG_SOURCE_ERROR: {error}"
            );
        }
        unchanged()?;
    }
    let first = application.prepare_verified_certified_transition(&candidate, 1)?;
    ensure!(
        first.results().count() == 3,
        "PRODUCTION_CERTIFIED_RESULT_COUNT"
    );
    for ((transaction, result), input) in first.results().zip(&raw) {
        let mut h = Keccak::v256();
        let mut bytes = [0; 32];
        h.update(input);
        h.finalize(&mut bytes);
        ensure!(
            bytes == transaction.hash,
            "PRODUCTION_CERTIFIED_INPUT_ORDER"
        );
        ensure!(
            matches!(result,TransactionExecutionResult::Executed(r)if matches!(r.status,CodeExecutionStatus::Success)&&r.gas_used.as_u64()==21000&&r.logs.is_empty()),
            "PRODUCTION_CERTIFIED_RESULT"
        );
    }
    let recipient = first
        .changed_accounts()
        .iter()
        .find(|row| row.address == [0x79; 20])
        .context("certified recipient")?;
    ensure!(
        matches!(&recipient.account,rustaxa_types::concrete_state::ConcreteRead::Present(row)if row.account.balance.value()==&num_bigint::BigUint::from(3_u8)),
        "PRODUCTION_CERTIFIED_RECIPIENT_BALANCE"
    );
    let root = first.next_identity();
    let digest = first.request_digest();
    let certificate_digest = first.certificate_digest();
    ensure!(
        certificate_digest != [0; 32],
        "PRODUCTION_CERTIFICATE_EVIDENCE_EMPTY"
    );
    unchanged()?;
    drop(first);
    unchanged()?;
    let second = application.prepare_verified_certified_transition(&candidate, 2)?;
    ensure!(
        second.results().count() == 3
            && second.request_digest() != digest
            && second.certificate_digest() != certificate_digest,
        "PRODUCTION_CERTIFICATE_ROUND_BINDING"
    );
    unchanged()?;
    drop(second);
    unchanged()?;
    let fresh = application.prepare_verified_certified_transition(&candidate, 1)?;
    ensure!(
        fresh.next_identity() == root
            && fresh.request_digest() == digest
            && fresh.results().count() == 3
            && fresh.certificate_digest() == certificate_digest,
        "PRODUCTION_CERTIFIED_FRESH_ATTEMPT"
    );
    drop(fresh);
    unchanged()?;
    for (block, round) in [(&candidate, 99), (&empty, 1)] {
        let error = application
            .prepare_verified_certified_transition(block, round)
            .err()
            .context("missing certificate accepted")?;
        ensure!(
            error
                .downcast_ref::<ProductionPreparationUnavailable>()
                .is_some(),
            "PRODUCTION_CERTIFIED_REFUSAL_CLASS: {error}"
        );
        unchanged()?;
    }
    let after = application
        .inner
        .application
        .integration_transition_database_images()?;
    super::publication_tests::save_image(&after.0, &path.join("after-primary.rlp"))?;
    super::publication_tests::save_image(&after.1, &path.join("after-concrete.rlp"))?;
    Ok(format!(
        "CERTIFIED_TRANSITION_CHECKS_PASSED: first_write_guard=true signer_transport_effects=0 normal_runtime=true fully_checked=true rejected_before_kernel=true callback_abort_clean=true validators=2 proof_sidecar_quorum_refused=true weak_conversion_refused=true retained_inputs_unchanged=true rounds=2 ordered=3 native_rewards_finished=true fresh=true missing_and_mismatched_refused=true primary_columns={} concrete_columns={} complete_WAL_images_equal=true",
        before.0.columns.len(),
        before.1.columns.len()
    ))
}
