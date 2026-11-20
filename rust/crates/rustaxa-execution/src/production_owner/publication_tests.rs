use super::*;
use anyhow::Context;
use rustaxa_consensus::pbft_vote_generation::{
    PbftVoteGenerationInput, PbftVoteWeightFacts, generate_pbft_vote_with_weight,
};
use rustaxa_storage::integration_inspection::DatabaseImage;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;

const SECOND_VRF: [u8; 64] = [
    0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4, 0xe0,
    0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c, 0x9a, 0x0d,
    0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28, 0x7e, 0xab, 0xba,
    0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97, 0xad, 0xe4, 0x00, 0x81,
];

fn open(
    selected: &Arc<rustaxa_native_config::SelectedProductionConfig>,
) -> Result<ProductionApplication> {
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    ProductionApplication::open_existing(ProductionBootstrapConfig {
        selected: Arc::clone(selected),
        concrete_path: config.storage_path.join("state_db"),
    })
}

fn images(application: &ProductionApplication) -> Result<(DatabaseImage, DatabaseImage)> {
    application
        .inner
        .application
        .integration_transition_database_images()
}

pub(super) fn save_image(image: &DatabaseImage, path: &std::path::Path) -> Result<()> {
    let bytes = image
        .columns
        .iter()
        .try_fold(0_usize, |sum, (name, rows)| {
            rows.iter().try_fold(sum + name.len() + 128, |sum, (k, v)| {
                sum.checked_add(k.len() + v.len() + 64)
            })
        })
        .context("PUBLICATION_TEST_IMAGE_SIZE_OVERFLOW")?;
    let bytes = image
        .wal_files
        .iter()
        .try_fold(bytes, |sum, (name, value)| {
            sum.checked_add(name.len() + value.len() + 64)
        })
        .context("PUBLICATION_TEST_IMAGE_SIZE_OVERFLOW")?;
    ensure!(
        bytes <= 8 * 1024 * 1024,
        "PUBLICATION_TEST_IMAGE_RETENTION_BOUND"
    );
    image.save(path)
}

fn raw_images(
    config: &rustaxa_consensus::ConsensusApplicationBootstrap,
) -> Result<(DatabaseImage, DatabaseImage)> {
    let storage = Storage::open_existing(Config::new(config.storage_path.clone()))?;
    let mut hash = Keccak::v256();
    let mut chain = [0; 32];
    hash.update(b"rustaxa-final-chain-concrete-chain-identity-v1");
    hash.update(config.storage_genesis_hash.as_bytes());
    hash.finalize(&mut chain);
    let path = config.storage_path.join("state_db");
    let observed = ConcreteStateLifecycle::inspect_existing(&path, chain)?;
    let concrete = ConcreteStateLifecycle::open(&path, chain, observed.committed)?;
    Ok((
        storage.integration_database_image()?,
        concrete.integration_database_image()?,
    ))
}

fn corrupt_recovery_record(
    config: &rustaxa_consensus::ConsensusApplicationBootstrap,
    kind: u8,
    evidence: &std::path::Path,
) -> Result<()> {
    let storage = Storage::open_existing(Config::new(config.storage_path.clone()))?;
    let original = storage
        .get_raw(
            rustaxa_storage::Column::Status,
            rustaxa_storage::application_control::PBFT_CERTIFIED_INTENT_KEY,
        )?
        .context("PUBLICATION_TEST_PENDING_INTENT_MISSING")?;
    std::fs::write(evidence.join("original-intent.rlp"), &original)?;
    match kind {
        5 => {
            let intent = rlp::Rlp::new(&original);
            let binding: Vec<u8> = intent.val_at(1)?;
            let binding = rlp::Rlp::new(&binding);
            let mut changed = rlp::RlpStream::new_list(8);
            for index in 0..8 {
                if index == 7 {
                    changed.append(&vec![0x91_u8; 32]);
                } else {
                    changed.append_raw(binding.at(index)?.as_raw(), 1);
                }
            }
            let changed = changed.out();
            let mut out = rlp::RlpStream::new_list(8);
            for index in 0..8 {
                if index == 1 {
                    out.append(&changed.as_ref());
                } else {
                    out.append_raw(intent.at(index)?.as_raw(), 1);
                }
            }
            let mut batch = storage.create_write_batch();
            storage.batch_put_raw(
                &mut batch,
                rustaxa_storage::Column::Status,
                rustaxa_storage::application_control::PBFT_CERTIFIED_INTENT_KEY,
                &out.out(),
            )?;
            storage.commit_write_batch_with_sync(batch, true)?;
        }
        6 => storage
            .final_chain()
            .write_external_evm_pending_publication(
                rustaxa_storage::FinalChainExternalEvmPendingPublication {
                    payload: b"test-local-legacy-conflict",
                },
            )?,
        7 => storage
            .metadata()
            .write_status_field(rustaxa_storage::StatusField::FinalChainRootPolicy as u8, 2)?,
        _ => bail!("PUBLICATION_TEST_CORRUPTION_KIND"),
    }
    Ok(())
}

fn admit_certificate(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    candidate: &[u8],
    vrf: [u8; 64],
) -> Result<()> {
    let votes = fixture_round_votes(application, selected, candidate, vrf, PbftVoteType::Cert, 3)?;
    application
        .inner
        .application
        .integration_admit_current_certificate(candidate, votes)
}

fn fixture_round_votes(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    candidate: &[u8],
    vrf: [u8; 64],
    vote_type: PbftVoteType,
    step: u64,
) -> Result<Vec<Vec<u8>>> {
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let link = rustaxa_types::pbft::PbftBlockLink::try_from(SignedPbftBlockRlp::new(candidate))?;
    let total = application
        .query_client()
        .final_chain_dpos_eligible_total_vote_count(link.period - 1)?;
    let validators = &selected.bootstrap().final_chain_genesis_validators;
    ensure!(validators.len() == 2, "PUBLICATION_TEST_TWO_VALIDATORS");
    let mut votes = Vec::new();
    for (index, secret) in [[0x66; 32], [0x77; 32]].into_iter().enumerate() {
        let count = application
            .query_client()
            .final_chain_dpos_eligible_vote_count(link.period - 1, validators[index].address)?;
        let vote = generate_pbft_vote_with_weight(
            PbftVoteGenerationInput {
                block_hash: link.block_hash,
                vote_type,
                period: link.period,
                round: 1,
                step,
                node_secret: secret,
                vrf_secret: if index == 0 { vrf } else { SECOND_VRF },
                expected_voter: ethereum_types::H160(validators[index].address),
                expected_vrf_public_key: validators[index].vrf_key,
            },
            PbftVoteWeightFacts {
                voter_dpos_vote_count: count,
                total_dpos_vote_count: total,
                committee_size: config.consensus.pbft.committee_size,
                number_of_proposers: config.consensus.pbft.number_of_proposers,
            },
        )?;
        ensure!(
            vote.accepted && vote.has_weight && vote.weight > 0,
            "PUBLICATION_TEST_CERT_GENERATION:{}",
            vote.error_code
        );
        votes.push(vote.vote_rlp);
    }
    Ok(votes)
}

fn candidate(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    first_nonce: u64,
    vrf: [u8; 64],
    deploy_bridge: bool,
) -> Result<(Vec<u8>, Vec<PublicTransactionSubmissionRequest>, Vec<u8>)> {
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let mut transactions = Vec::new();
    for nonce in first_nonce..first_nonce + 3 {
        let mut request = if deploy_bridge && nonce == 0 {
            rustaxa_consensus::production_bootstrap::signed_fixture_bridge_creation(
                selected, secret,
            )?
        } else {
            rustaxa_consensus::production_bootstrap::signed_fixture_transaction(
                selected, secret, nonce,
            )?
        };
        request.last_block_number = application.query_client().final_chain_last_block_number()?;
        let report = application
            .network_client()
            .submit_transaction_from_native_state(request.clone())?;
        ensure!(
            report.accepted && report.transaction_observed,
            "PUBLICATION_TEST_NORMAL_INGRESS:{}",
            report.message
        );
        transactions.push(request);
    }
    let inputs = transactions
        .iter()
        .map(|t| t.transaction_rlp.clone())
        .collect::<Vec<_>>();
    let period = application
        .query_client()
        .final_chain_last_block_number()?
        .checked_add(1)
        .context("test period")?;
    let (block, _, bad) = application
        .inner
        .application
        .integration_transition_candidates(
            &inputs,
            [0x66; 32],
            config
                .final_chain
                .genesis_timestamp
                .checked_add(period)
                .context("test timestamp")?,
        )?;
    admit_certificate(application, selected, &block, vrf)?;
    Ok((block, transactions, bad))
}

fn check_committed(
    application: &ProductionApplication,
    transactions: &[PublicTransactionSubmissionRequest],
    period: u64,
) -> Result<()> {
    ensure!(
        application.query_client().final_chain_last_block_number()? == period,
        "PUBLICATION_TEST_HEAD"
    );
    ensure!(
        application
            .query_client()
            .transaction_count_by_block_number(period)?
            == 3,
        "PUBLICATION_TEST_TRANSACTION_COUNT"
    );
    for (index, transaction) in transactions.iter().enumerate() {
        let mut hash = Keccak::v256();
        let mut bytes = [0; 32];
        hash.update(&transaction.transaction_rlp);
        hash.finalize(&mut bytes);
        let receipt = application
            .query_client()
            .transaction_receipt_by_hash(bytes)?;
        ensure!(
            receipt.found
                && receipt.block_number == period
                && receipt.transaction_index == index as u32
                && !receipt.is_system
                && receipt.block_hash_found,
            "PUBLICATION_TEST_RECEIPT_INDEX"
        );
        let raw = rlp::Rlp::new(&receipt.receipt_rlp);
        ensure!(
            raw.val_at::<u8>(0)? == 1
                && raw.val_at::<u64>(1)? == 21000
                && raw.val_at::<u64>(2)? == (index as u64 + 1) * 21000,
            "PUBLICATION_TEST_RECEIPT_GAS"
        );
        let tx = application.query_client().transaction_by_hash(bytes)?;
        ensure!(
            tx.found
                && tx.location_found
                && tx.block_number == period
                && tx.transaction_rlp == transaction.transaction_rlp,
            "PUBLICATION_TEST_TRANSACTION_LOOKUP"
        );
    }
    ensure!(
        application
            .query_client()
            .final_chain_block_by_number(0)?
            .found,
        "PUBLICATION_TEST_GENESIS_HISTORY_LOST"
    );
    Ok(())
}

/// Creates a nonzero paired predecessor for the offline App test.
pub fn seed_app_predecessor(
    selected: rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    vrf: [u8; 64],
    deploy_bridge: bool,
) -> Result<()> {
    let selected = Arc::new(selected);
    let application = open(&selected)?;
    let (block, _, _) = candidate(&application, &selected, secret, 0, vrf, deploy_bridge)?;
    let prepared = application.prepare_verified_certified_transition(&block, 1)?;
    ensure!(
        application
            .publish_verified_transition(prepared)?
            .period
            .as_u64()
            == 1
    );
    Ok(())
}

/// Captures both closed App stores and compares logical rows after restart.
pub fn check_app_restart_images(
    selected: &rustaxa_native_config::SelectedProductionConfig,
    after_restart: bool,
) -> Result<()> {
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let evidence = config.storage_path.join("app-restart-evidence");
    if !after_restart {
        std::fs::create_dir(&evidence)?;
    }
    let (primary, concrete) = raw_images(&config)?;
    let status = primary
        .columns
        .iter()
        .find(|(name, _)| name == rustaxa_storage::Column::Status.name())
        .context("APP_TEST_STATUS_COLUMN_MISSING")?;
    ensure!(
        !status.1.iter().any(|(key, _)| key.as_slice()
            == rustaxa_storage::application_control::PBFT_CERTIFIED_INTENT_KEY),
        "APP_TEST_PUBLICATION_INTENT_REMAINS"
    );
    for (name, image) in [("primary", primary), ("concrete", concrete)] {
        let before = evidence.join(format!("{name}-before-restart.rlp"));
        if after_restart {
            let after = evidence.join(format!("{name}-after-restart.rlp"));
            save_image(&image, &after)?;
            let old = std::fs::read(&before)?;
            let new = std::fs::read(&after)?;
            ensure!(
                rlp::Rlp::new(&old).at(0)?.as_raw() == rlp::Rlp::new(&new).at(0)?.as_raw(),
                "APP_TEST_RESTART_LOGICAL_CONTENT_CHANGED: {name}"
            );
        } else {
            save_image(&image, &before)?;
        }
    }
    Ok(())
}

/// Provides persisted DAG fixture input and admits a signed certificate.
/// The App scheduler must qualify the retained proposal and publish it.
pub fn retain_app_certificate(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    transactions: &[Vec<u8>],
    vrf: [u8; 64],
    next_round: bool,
    expect_system: bool,
) -> Result<()> {
    let query = application.query_client();
    ensure!(query.final_chain_last_block_number()? == 1 && transactions.len() == 3);
    for transaction in transactions {
        let mut hash = Keccak::v256();
        let mut bytes = [0; 32];
        hash.update(transaction);
        hash.finalize(&mut bytes);
        ensure!(
            query.live_transaction_is_known(bytes)?,
            "APP_TEST_TRANSACTION_NOT_SUBMITTED"
        );
    }
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let block = application
        .inner
        .application
        .integration_scheduler_candidate(
            transactions,
            [0x66; 32],
            config
                .final_chain
                .genesis_timestamp
                .checked_add(2)
                .context("test timestamp")?,
        )?;
    application
        .inner
        .application
        .integration_retain_unqualified_proposal(&block)?;
    let validation = application.check_candidate_transactions(&block)?;
    ensure!(
        matches!(
            validation,
            rustaxa_consensus::pbft_execution_validation::PbftExecutionValidation::Valid {
                executed_transactions,
                ..
            } if executed_transactions == if expect_system {4} else {3}
        ),
        "APP_TEST_CANDIDATE_TRANSACTION_CHECK: {validation:?}"
    );
    if next_round {
        let votes = fixture_round_votes(application, selected, &block, vrf, PbftVoteType::Next, 4)?;
        return application
            .inner
            .application
            .integration_admit_next_votes(votes);
    }
    admit_certificate(application, selected, &block, vrf)?;
    let (period, round, owner_generation, votes) = application
        .inner
        .application
        .integration_scheduler_certificate(&block)?;
    let selection = rustaxa_consensus::consensus_application_runtime::ScheduledCertificate {
        period,
        round,
        block_rlp: &block,
        votes: &votes,
        owner_generation,
    };
    let prepared = application
        .inner
        .application
        .with_scheduled_certificate_preparation(&selection, |context, concrete, native| {
            crate::production_transition::execute(context, concrete, native)?.into_verified()
        })?
        .context("APP_TEST_SCHEDULER_SELECTION_NOT_CURRENT")?;
    let before = images(application)?;
    application.submit_public_transaction_from_native_state(
        PublicTransactionSubmissionRequest {
            transaction_rlp: transactions[0].clone(),
            expected_chain_id: config.consensus.pbft.network_identity.chain_id,
            maximum_gas_limit: selected.transaction_max_gas(),
            minimum_gas_price: ethereum_types::U256::from(selected.transaction_min_gas_price()),
            last_block_number: 1,
            cornus_active: 1 >= config.consensus.pbft.cornus_activation_period,
        },
    )?;
    let error = application
        .publish_verified_transition(prepared)
        .expect_err("stale scheduler preparation must not publish");
    ensure!(
        error.is::<ScheduledCertificateStale>(),
        "APP_TEST_EXPECTED_TYPED_STALE: {error}"
    );
    ensure!(
        images(application)? == before,
        "APP_TEST_STALE_ATTEMPT_CHANGED_STORAGE"
    );
    Ok(())
}

/// Produces an interrupted period 2 publication on a new App fixture.
pub fn interrupt_app_publication(
    selected: rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    vrf: [u8; 64],
    stop: u8,
) -> Result<Vec<Vec<u8>>> {
    let selected = Arc::new(selected);
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(&selected)?;
    let evidence = config.storage_path.join("app-interruption-evidence");
    std::fs::create_dir(&evidence)?;
    let application = open(&selected)?;
    ensure!(application.query_client().final_chain_last_block_number()? == 1);
    let (block, transactions, _) = candidate(&application, &selected, secret, 3, vrf, false)?;
    let before = images(&application)?;
    let prepared = application.prepare_verified_certified_transition(&block, 1)?;
    ensure!(
        prepared.next_identity().period.as_u64() == 2,
        "APP_INTERRUPT_NONZERO_PREDECESSOR"
    );
    let expected = match stop {
        1 => "CERTIFIED_TEST_AFTER_INTENT",
        3 => "CERTIFIED_TEST_AFTER_CONCRETE_COMMIT",
        _ => bail!("APP_INTERRUPT_STOP"),
    };
    let error = application
        .integration_publish_at(prepared, stop)
        .expect_err("the injected stop must interrupt the producer");
    ensure!(
        error.to_string().contains(expected),
        "APP_INTERRUPT_WRONG_BOUNDARY: {error:#}"
    );
    drop(application);
    let interrupted = raw_images(&config)?;
    save_image(&interrupted.0, &evidence.join("interrupted-primary.rlp"))?;
    save_image(&interrupted.1, &evidence.join("interrupted-concrete.rlp"))?;
    let storage = Storage::open_existing(Config::new(config.storage_path.clone()))?;
    let intent = storage
        .pending_certified_intent()?
        .context("APP_INTERRUPT_INTENT_MISSING")?;
    let expected = if stop == 1 {
        before
    } else {
        (
            intent.integration_expected_primary_image(interrupted.0)?,
            interrupted.1,
        )
    };
    save_image(&expected.0, &evidence.join("expected-primary.rlp"))?;
    save_image(&expected.1, &evidence.join("expected-concrete.rlp"))?;
    Ok(transactions
        .into_iter()
        .map(|request| request.transaction_rlp)
        .collect())
}

/// Checks the recovered App's complete logical stores under its actual owner.
pub fn check_app_interrupted_recovery(
    application: &ProductionApplication,
    selected: &rustaxa_native_config::SelectedProductionConfig,
    second_open: bool,
) -> Result<()> {
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let evidence = config.storage_path.join("app-interruption-evidence");
    let (primary, concrete) = images(application)?;
    for (name, image) in [("primary", primary), ("concrete", concrete)] {
        let suffix = if second_open { "reopened" } else { "recovered" };
        let observed = evidence.join(format!("{suffix}-{name}.rlp"));
        save_image(&image, &observed)?;
        let expected = std::fs::read(evidence.join(format!("expected-{name}.rlp")))?;
        let observed = std::fs::read(observed)?;
        ensure!(
            rlp::Rlp::new(&expected).at(0)?.as_raw() == rlp::Rlp::new(&observed).at(0)?.as_raw(),
            "APP_INTERRUPT_RECOVERY_CONTENT: {name}"
        );
    }
    Ok(())
}

/// Runs only over a fresh task fixture made by the actual native loader.
pub fn run_publication_fixture_checks(
    selected: rustaxa_native_config::SelectedProductionConfig,
    foreign: Option<rustaxa_native_config::SelectedProductionConfig>,
    secret: [u8; 32],
    vrf: [u8; 64],
    stop: u8,
) -> Result<String> {
    ensure!(stop <= 7, "PUBLICATION_TEST_STOP");
    let selected = Arc::new(selected);
    let config =
        rustaxa_consensus::production_bootstrap::application_bootstrap_from_selected(&selected)?;
    let evidence = config.storage_path.join("publication-evidence");
    std::fs::create_dir(&evidence)?;
    let application = open(&selected)?;
    let (block, transactions, bad) = candidate(&application, &selected, secret, 0, vrf, false)?;
    let before = images(&application)?;
    save_image(&before.0, &evidence.join("before-primary.rlp"))?;
    save_image(&before.1, &evidence.join("before-concrete.rlp"))?;
    ensure!(
        application
            .prepare_verified_certified_transition(&bad, 1)
            .is_err(),
        "PUBLICATION_TEST_BAD_ORDER_ACCEPTED"
    );
    ensure!(
        images(&application)? == before,
        "PUBLICATION_TEST_REJECTED_ATTEMPT_WROTE"
    );
    let stale = application.prepare_verified_certified_transition(&block, 1)?;
    let _ = application
        .network_client()
        .submit_transaction_from_native_state(transactions[0].clone())?;
    ensure!(
        application.publish_verified_transition(stale).is_err(),
        "PUBLICATION_TEST_STALE_ACCEPTED"
    );
    ensure!(
        images(&application)? == before,
        "PUBLICATION_TEST_STALE_ATTEMPT_WROTE"
    );
    if let Some(foreign) = foreign {
        let foreign = Arc::new(foreign);
        let other = open(&foreign)?;
        let (other_block, _, _) = candidate(&other, &foreign, secret, 0, vrf, false)?;
        let other_before = images(&other)?;
        let misplaced = application.prepare_verified_certified_transition(&block, 1)?;
        ensure!(
            other.publish_verified_transition(misplaced).is_err(),
            "PUBLICATION_TEST_FOREIGN_ACCEPTED"
        );
        ensure!(
            images(&application)? == before && images(&other)? == other_before,
            "PUBLICATION_TEST_FOREIGN_ATTEMPT_WROTE"
        );
        let valid = other.prepare_verified_certified_transition(&other_block, 1)?;
        other.publish_verified_transition(valid)?;
        drop(other);
        let other = open(&foreign)?;
        ensure!(
            other.query_client().final_chain_last_block_number()? == 1,
            "PUBLICATION_TEST_FOREIGN_CAP_CONSUMED_ON_REFUSAL"
        );
        drop(other);
    }
    let prepared = application.prepare_verified_certified_transition(&block, 1)?;
    let next = prepared.next_identity();
    let retained = application.network_client();
    let result = application.integration_publish_at(prepared, if stop >= 5 { 3 } else { stop });
    if stop == 0 {
        ensure!(result? == next, "PUBLICATION_TEST_COMMIT_IDENTITY");
    } else {
        ensure!(result.is_err(), "PUBLICATION_TEST_FAULT_DID_NOT_STOP");
    }
    ensure!(
        retained
            .submit_transaction_from_native_state(transactions[0].clone())
            .is_err(),
        "PUBLICATION_TEST_RETIRED_CLIENT_WROTE"
    );
    ensure!(
        application
            .prepare_verified_certified_transition(&block, 1)
            .is_err(),
        "PUBLICATION_TEST_RETIRED_OWNER_EXECUTED"
    );
    drop(retained);
    drop(application);
    if stop >= 5 {
        corrupt_recovery_record(&config, stop, &evidence)?;
        let before = raw_images(&config)?;
        save_image(&before.0, &evidence.join("conflict-before-primary.rlp"))?;
        save_image(&before.1, &evidence.join("conflict-before-concrete.rlp"))?;
        let error = open(&selected)
            .err()
            .context("PUBLICATION_TEST_CONFLICT_ACCEPTED")?;
        let expected = match stop {
            5 => "PRODUCTION_RECOVERY_REQUEST_PLAN_MISMATCH",
            6 => "PRODUCTION_RECOVERY_LEGACY_PUBLICATION_CONFLICT",
            _ => "PRODUCTION_STATUS_MISMATCH",
        };
        ensure!(
            error.to_string().contains(expected),
            "PUBLICATION_TEST_WRONG_LOCAL_FAILURE:{error:#}"
        );
        ensure!(
            error
                .downcast_ref::<ProductionCertificateProofError>()
                .is_none(),
            "PUBLICATION_TEST_LOCAL_FAILURE_CLASSIFIED_AS_INVALID_CERTIFICATE"
        );
        let after = raw_images(&config)?;
        ensure!(
            before.0.columns == after.0.columns && before.1.columns == after.1.columns,
            "PUBLICATION_TEST_CONFLICT_RECOVERY_WROTE"
        );
        save_image(&after.0, &evidence.join("conflict-after-primary.rlp"))?;
        save_image(&after.1, &evidence.join("conflict-after-concrete.rlp"))?;
        return Ok(format!(
            "CERTIFIED_PUBLICATION_RECOVERY_PASSED: stop={stop} local_conflict_refused_before_writes=true both_intents_and_committed_history_preserved=true no_Go_execution=true"
        ));
    }
    let expected_rollforward = if stop == 3 {
        let (primary, concrete) = raw_images(&config)?;
        let storage = Storage::open_existing(Config::new(config.storage_path.clone()))?;
        let intent = storage
            .pending_certified_intent()?
            .context("PUBLICATION_TEST_INTENT_MISSING")?;
        let expected = intent.integration_expected_primary_image(primary)?;
        save_image(&expected, &evidence.join("expected-primary.rlp"))?;
        save_image(&concrete, &evidence.join("expected-concrete.rlp"))?;
        Some((expected, concrete))
    } else {
        None
    };
    let application = open(&selected)?;
    let recovered = images(&application)?;
    if let Some((primary, concrete)) = expected_rollforward {
        ensure!(
            recovered.0.columns == primary.columns && recovered.1.columns == concrete.columns,
            "PUBLICATION_TEST_ROLLFORWARD_NOT_COMPLETE_INTENDED_STATE"
        );
    }
    save_image(&recovered.0, &evidence.join("recovered-primary.rlp"))?;
    save_image(&recovered.1, &evidence.join("recovered-concrete.rlp"))?;
    let head = if matches!(stop, 1 | 2) { 0 } else { 1 };
    ensure!(
        application.query_client().final_chain_last_block_number()? == head,
        "PUBLICATION_TEST_RECOVERY_HEAD"
    );
    if head == 0 {
        ensure!(
            recovered.0.columns == before.0.columns && recovered.1.columns == before.1.columns,
            "PUBLICATION_TEST_ABORT_CHANGED_COMMITTED_ROWS"
        );
        admit_certificate(&application, &selected, &block, vrf)?;
        let fresh = application.prepare_verified_certified_transition(&block, 1)?;
        application.publish_verified_transition(fresh)?;
        drop(application);
    } else {
        check_committed(&application, &transactions, 1)?;
        drop(application);
    }
    let application = open(&selected)?;
    check_committed(&application, &transactions, 1)?;
    let stable = images(&application)?;
    drop(application);
    let application = open(&selected)?;
    let reopened = images(&application)?;
    ensure!(
        reopened.0.columns == stable.0.columns && reopened.1.columns == stable.1.columns,
        "PUBLICATION_TEST_REOPEN_CHANGED_LOGICAL_STATE"
    );
    let history = application.query_client().final_chain_block_by_number(1)?;
    let (second, next_transactions, _) = candidate(&application, &selected, secret, 3, vrf, false)?;
    let prepared = application.prepare_verified_certified_transition(&second, 1)?;
    ensure!(
        prepared.next_identity().period.as_u64() == 2,
        "PUBLICATION_TEST_NONZERO_PREDECESSOR"
    );
    application.publish_verified_transition(prepared)?;
    drop(application);
    let application = open(&selected)?;
    check_committed(&application, &next_transactions, 2)?;
    ensure!(
        application.query_client().final_chain_block_by_number(1)? == history,
        "PUBLICATION_TEST_FINALIZED_HISTORY_CHANGED"
    );
    let final_images = images(&application)?;
    save_image(&final_images.0, &evidence.join("final-primary.rlp"))?;
    save_image(&final_images.1, &evidence.join("final-concrete.rlp"))?;
    Ok(format!(
        "CERTIFIED_PUBLICATION_RECOVERY_PASSED: stop={stop} normal_ingress=true rejected_and_stale_no_writes=true retired_clients_refused=true nonzero_existing_continuation=true height=2 prior_history_preserved=true complete_primary_columns={} complete_concrete_columns={} no_Go_execution=true",
        final_images.0.columns.len(),
        final_images.1.columns.len()
    ))
}

pub fn check_app_reproposal(application: &ProductionApplication) -> Result<bool> {
    application.inner.application.integration_has_reproposal()
}

pub fn check_app_scheduled_pillar_vote(application: &ProductionApplication) -> Result<bool> {
    application
        .inner
        .application
        .integration_has_scheduled_pillar_vote()
}
