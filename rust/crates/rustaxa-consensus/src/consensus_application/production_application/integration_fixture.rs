use super::*;
use k256::ecdsa::SigningKey;
use rustaxa_storage::ConcreteStateLifecycle;
use tiny_keccak::{Hasher, Keccak};

impl BoundConsensusApplication {
    /// Retains a signed scheduler fixture without marking it valid.
    pub fn integration_retain_unqualified_proposal(&self, block_rlp: &[u8]) -> Result<()> {
        let _action = self.source.mutation()?;
        ensure!(
            self.core
                .pbft
                .publish_proposed_block_effect(block_rlp.to_vec())?,
            "PRODUCTION_TEST_PROPOSAL_NOT_NEW"
        );
        Ok(())
    }

    /// Reads the current real vote bundle and its owner generation for a race test.
    pub fn integration_scheduler_certificate(
        &self,
        block_rlp: &[u8],
    ) -> Result<(
        u64,
        u64,
        u64,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    )> {
        let action = self.source.read()?;
        let live = self.core.pbft.manager_snapshot();
        let candidate = rustaxa_types::pbft::PbftBlockLink::try_from(
            rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(block_rlp),
        )?;
        let certificate = self
            .core
            .pbft
            .verified_votes_get_two_t_plus_one_voted_block_payloads(
                live.period,
                live.round,
                crate::verified_votes::TwoTPlusOneVotedBlockType::CertVotedBlock,
            )?
            .context("PRODUCTION_TEST_SCHEDULER_CERTIFICATE_MISSING")?;
        ensure!(
            candidate.period == live.period && certificate.block_hash == candidate.block_hash,
            "PRODUCTION_TEST_SCHEDULER_CERTIFICATE_MISMATCH"
        );
        Ok((
            live.period,
            live.round,
            action.generation,
            certificate.votes,
        ))
    }

    /// Admits remote test inputs through the real native certificate service.
    pub fn integration_admit_current_certificate(
        &self,
        block_rlp: &[u8],
        votes: Vec<Vec<u8>>,
    ) -> Result<()> {
        let _action = self.source.mutation()?;
        let link = rustaxa_types::pbft::PbftBlockLink::try_from(
            rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(block_rlp),
        )?;
        let step = self.core.pbft.begin_pbft_sync_cert_bundle(
            &self.core.final_chain,
            link.period,
            link.block_hash,
            votes,
            || Ok(Vec::new()),
        )?;
        ensure!(
            step.action == crate::pbft_service::PbftSyncCertBundleAction::Accepted
                && step.error_code.is_empty()
                && !step.weighted_vote_rlps.is_empty()
                && step.total_weight >= step.two_t_plus_one
                && step.two_t_plus_one > 0,
            "PRODUCTION_TEST_CERTIFICATE_ADMISSION: {step:?}"
        );
        Ok(())
    }

    /// Admits signed prior-round tests through the live vote service.
    pub fn integration_admit_next_votes(&self, votes: Vec<Vec<u8>>) -> Result<()> {
        let _action = self.source.mutation()?;
        for vote in votes {
            let report =
                self.core
                    .pbft
                    .admit_network_verified_vote(&self.core.final_chain, &vote, &[])?;
            ensure!(
                report.validation.accepted && report.transaction.transition_published,
                "PRODUCTION_TEST_NEXT_VOTE_ADMISSION"
            );
        }
        Ok(())
    }

    /// Checks the exact candidate hash of the new round's durable propose vote.
    pub fn integration_has_reproposal(&self) -> Result<bool> {
        use crate::verified_votes::TwoTPlusOneVotedBlockType;
        let _action = self.source.read()?;
        let period = self.core.pbft.manager_snapshot().period;
        let Some(previous) = self
            .core
            .pbft
            .verified_votes_get_two_t_plus_one_voted_block(
                period,
                1,
                TwoTPlusOneVotedBlockType::NextVotedBlock,
            )?
        else {
            return Ok(false);
        };
        for record in self.core.pbft.verified_votes_own_vote_records()? {
            let vote = crate::pbft_vote_validation::inspect_canonical_pbft_vote(&record.vote_rlp)?;
            if vote.period == period && vote.round == 2 && vote.step == 1 {
                ensure!(
                    vote.signature_valid
                        && vote.vote_hash == record.hash
                        && vote.block_hash == previous.block_hash,
                    "PRODUCTION_TEST_REPROPOSAL_IDENTITY"
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Checks the paired pillar anchor and the vote stored by the live scheduler.
    pub fn integration_has_scheduled_pillar_vote(&self) -> Result<bool> {
        use rustaxa_types::{PillarBlock, PillarVote};
        let _action = self.source.read()?;
        let bootstrap = self.core.pbft.load_pillar_startup_bootstrap()?;
        if bootstrap.own_vote_rlp.is_empty() {
            return Ok(false);
        }
        let block = PillarBlock::decode_rlp(&self.core.pbft.current_pillar_block_rlp()?)?;
        let vote = PillarVote::decode_rlp(&bootstrap.own_vote_rlp)?;
        let retained = self.core.pbft.pillar_verified_vote_payloads(
            vote.period,
            &block.hash().into(),
            false,
        )?;
        ensure!(
            retained
                .votes
                .iter()
                .any(|record| record.weight > 0 && record.vote_rlp == bootstrap.own_vote_rlp),
            "PRODUCTION_TEST_SCHEDULED_PILLAR_VOTE_NOT_ADMITTED"
        );
        let header = self
            .core
            .final_chain
            .block_header(1_u64.into())?
            .context("PRODUCTION_TEST_PILLAR_PARENT_HEADER")?;
        let root = rlp::Rlp::new(&header).val_at::<ethereum_types::H256>(3)?;
        ensure!(
            block.period == 2
                && block.state_root == root
                && vote.period == 3
                && vote.block_hash == block.hash()
                && vote.recover_voter_address().is_some(),
            "PRODUCTION_TEST_SCHEDULED_PILLAR_IDENTITY"
        );
        Ok(true)
    }

    /// Sets the proposed-cache input for the closed finalization route test.
    /// This setup does not prove proposal admission.
    pub fn integration_retain_finalization_candidate(&self, block_rlp: &[u8]) -> Result<()> {
        let _action = self.source.mutation()?;
        let link = rustaxa_types::pbft::PbftBlockLink::try_from(
            rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(block_rlp),
        )?;
        self.core
            .pbft
            .publish_proposed_block_effect(block_rlp.to_vec())?;
        self.core
            .pbft
            .mark_proposed_block_valid(link.period, link.block_hash)
    }

    /// Installs explicit unchecked retained input before the test baseline.
    /// This does not model network admission or grant certificate authority.
    pub fn integration_retain_certificate_test_input(
        &self,
        canonical: &[u8],
        weight: u64,
        transplant: Option<(u64, [u8; 32])>,
    ) -> Result<()> {
        let _action = self.source.mutation()?;
        let raw = if let Some((round, secret)) = transplant {
            let input = crate::pbft_vote_validation::inspect_canonical_pbft_vote(canonical)?;
            let mut sortition = RlpStream::new_list(4);
            sortition
                .append(&input.period)
                .append(&round)
                .append(&input.step)
                .append(&input.vrf_proof.as_slice());
            let sortition = sortition.out().to_vec();
            let hash = crate::pbft_vote_validation::legacy_pbft_vote_signing_hash(
                input.block_hash,
                &sortition,
            );
            let (signature, recovery) =
                SigningKey::from_slice(&secret)?.sign_prehash_recoverable(hash.as_bytes())?;
            let mut signature = signature.to_bytes().to_vec();
            signature.push(recovery.to_byte());
            let mut vote = RlpStream::new_list(3);
            vote.append(&input.block_hash)
                .append(&sortition)
                .append(&signature);
            vote.out().to_vec()
        } else {
            canonical.to_vec()
        };
        let weighted = crate::pbft_vote_payload::build_weighted_pbft_vote_payload(&raw, weight)?;
        self.core
            .pbft
            .integration_retain_certificate_input(weighted.vote_rlp)
    }

    pub fn integration_certificate_input_snapshot(
        &self,
    ) -> Result<crate::pbft_vote_runtime::VerifiedVotesStateSnapshot> {
        let _action = self.source.read()?;
        self.core.pbft.verified_votes_state_snapshot()
    }

    pub fn integration_transition_database_images(
        &self,
    ) -> Result<(
        rustaxa_storage::integration_inspection::DatabaseImage,
        rustaxa_storage::integration_inspection::DatabaseImage,
    )> {
        let action = self.source.read()?;
        action.validate(self)?;
        Ok((
            self.core.storage.integration_database_image()?,
            self.source
                .concrete_state
                .integration_database_image(&action.guard)?,
        ))
    }

    /// Installs canonical functional inputs in this disposable fixture only.
    /// It does not claim that DAG admission or a certificate has passed.
    pub fn integration_transition_candidates(
        &self,
        transactions: &[Vec<u8>],
        secret: [u8; 32],
        timestamp: u64,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        self.integration_make_candidates(transactions, secret, timestamp, false)
    }

    pub fn integration_scheduler_candidate(
        &self,
        transactions: &[Vec<u8>],
        secret: [u8; 32],
        timestamp: u64,
    ) -> Result<Vec<u8>> {
        Ok(self
            .integration_make_candidates(transactions, secret, timestamp, true)?
            .0)
    }

    fn integration_make_candidates(
        &self,
        transactions: &[Vec<u8>],
        secret: [u8; 32],
        timestamp: u64,
        use_delayed_hash: bool,
    ) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        use rustaxa_types::DagBlock;
        let _action = self.source.mutation()?;
        let head = load_persisted_pbft_chain_head_identity(&self.core.storage)?
            .context("fixture PBFT head")?;
        ensure!(
            head.size == read_committed_identity(&self.core.storage)?.period.as_u64(),
            "transition fixture requires matching committed heads"
        );
        let key = SigningKey::from_slice(&secret)?;
        let reward_hashes: Vec<H256> = if head.size == 0 {
            Vec::new()
        } else {
            self.core
                .pbft
                .current_reward_vote_snapshot()?
                .records
                .into_iter()
                .map(|r| r.hash)
                .collect()
        };
        let (mut dag, mut transaction) = self.core.dag_transaction.lock_dag_and_transaction()?;
        let mut hashes = Vec::new();
        for raw in transactions {
            let h = fixture_hash(raw);
            if !transaction.sidecar.contains_non_finalized(h) {
                let count = transaction
                    .sidecar
                    .transaction_count()
                    .checked_add(1)
                    .context("fixture transaction count")?;
                self.core
                    .storage
                    .metadata()
                    .write_status_field(StatusField::TrxCount as u8, count)?;
                transaction.sidecar.set_transaction_count(count);
            }
            transaction.sidecar.insert_non_finalized(h, raw.clone())?;
            self.core.storage.transaction().write(h, raw)?;
            hashes.push(h);
        }
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11_u8; 80])
            .append(&vec![0x22_u8])
            .append(&vec![0x33_u8])
            .append(&1_u16);
        let mut block = DagBlock {
            pivot: dag.state.anchor(),
            level: dag
                .state
                .max_level()
                .checked_add(1)
                .context("fixture DAG level")?,
            timestamp,
            vdf: vdf.out().to_vec(),
            tips: Vec::new(),
            transactions: hashes,
            signature: [0; 65],
            gas_estimation: 63_000,
        };
        let (signature, recovery) =
            key.sign_prehash_recoverable(block.signing_hash().as_bytes())?;
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery.to_byte();
        let mut raw = RlpStream::new_list(8);
        raw.append(&block.pivot)
            .append(&block.level)
            .append(&block.timestamp)
            .append(&block.vdf)
            .append_list(&block.tips)
            .append_list(&block.transactions)
            .append(&block.signature.as_slice())
            .append(&block.gas_estimation);
        let raw = raw.out().to_vec();
        let facts = crate::dag::dag_manager_block_from_rlp(&raw)?;
        dag.state.add_block(facts.clone())?;
        crate::dag::save_dag_block_to_storage(
            &self.core.storage,
            facts.hash,
            block.level,
            0,
            &raw,
        )?;
        crate::dag::ensure_proposal_period_mapping(
            &self.core.storage,
            block.level,
            dag.state.period(),
        )?;
        let order = dag
            .state
            .compute_order(facts.hash)
            .context("fixture order")?;
        let mut order_bytes = RlpStream::new_list(1);
        order_bytes.begin_list(order.len());
        for hash in order {
            order_bytes.append(&hash);
        }
        let order_hash = fixture_hash(&order_bytes.out());
        let prior_hash = if use_delayed_hash {
            self.core
                .final_chain
                .pbft_final_chain_hash(head.size.checked_add(1).context("fixture period")?)?
                .context("fixture delayed final-chain hash")?
                .to_vec()
        } else {
            self.core
                .final_chain
                .block_hash(head.size.into())?
                .context("fixture final-chain hash")?
        };
        ensure!(prior_hash.len() == 32, "fixture final-chain hash width");
        let period = head.size.checked_add(1).context("fixture period")?;
        let policy = self
            .core
            .pbft
            .value_proposal_admission_request(period, H256::zero());
        let extra = if policy.extra_data_required {
            let mut out = RlpStream::new_list(6);
            out.append(&1_u16)
                .append(&0_u16)
                .append(&0_u16)
                .append(&0_u16)
                .append(&"rustaxa-fixture");
            if policy.pillar_block_required {
                let raw = self.core.pbft.current_pillar_block_rlp()?;
                let block = rustaxa_types::PillarBlock::decode_rlp(&raw)?;
                out.append(&block.hash());
            } else {
                out.append_empty_data();
            }
            Some(out.out().to_vec())
        } else {
            None
        };
        let candidate = |pivot: H256, ordered: H256| -> Result<Vec<u8>> {
            let fields = |out: &mut RlpStream| {
                out.append(&head.last_pbft_block_hash)
                    .append(&pivot)
                    .append(&ordered)
                    .append(&H256::from_slice(&prior_hash))
                    .append(
                        &head
                            .size
                            .checked_add(1)
                            .expect("fixture period was checked"),
                    )
                    .append(&timestamp)
                    .begin_list(reward_hashes.len());
                for hash in &reward_hashes {
                    out.append(hash);
                }
                if let Some(extra) = &extra {
                    out.append(extra);
                }
            };
            let fields_count = 7 + usize::from(extra.is_some());
            let mut unsigned = RlpStream::new_list(fields_count);
            fields(&mut unsigned);
            let (signature, recovery) =
                key.sign_prehash_recoverable(fixture_hash(&unsigned.out()).as_bytes())?;
            let mut sig = signature.to_bytes().to_vec();
            sig.push(recovery.to_byte());
            let mut signed = RlpStream::new_list(fields_count + 1);
            fields(&mut signed);
            signed.append(&sig);
            Ok(signed.out().to_vec())
        };
        Ok((
            candidate(facts.hash, order_hash)?,
            candidate(H256::zero(), H256::zero())?,
            candidate(facts.hash, H256::repeat_byte(0xa7))?,
        ))
    }
}

fn fixture_hash(bytes: &[u8]) -> H256 {
    let mut hash = Keccak::v256();
    let mut out = [0; 32];
    hash.update(bytes);
    hash.finalize(&mut out);
    H256(out)
}

/// Builds only a disposable historical fixture from an actual native selection.
/// The production loader must close and reopen these stores after this call.
pub fn prepare_disposable_native_fixture(
    selected: &rustaxa_native_config::SelectedProductionConfig,
) -> Result<String> {
    let mut preparation =
        crate::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let path = preparation.storage_path.clone();
    ensure!(
        !path.join("db").exists() && !path.join("state_db").exists(),
        "PRODUCTION_FIXTURE_PATH_ALREADY_USED"
    );
    preparation.storage_path = path.join("native-inputs");
    ensure!(
        !preparation.storage_path.exists(),
        "PRODUCTION_FIXTURE_INPUT_PATH_ALREADY_USED"
    );
    // This temporary descriptor is never supplied to the production factory.
    preparation.final_chain.state_api_committed_root = H256::zero();
    let native = preparation.bootstrap()?;
    let (batch, catalog) = native.final_chain.concrete_genesis_inputs_for_test()?;
    drop(native);
    let mut bootstrap = crate::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let chain: [u8; 32] = concrete_chain_config_identity(bootstrap.storage_genesis_hash)
        .try_into()
        .map_err(|_| anyhow::anyhow!("PRODUCTION_FIXTURE_CHAIN_WIDTH"))?;
    let concrete = ConcreteStateLifecycle::create_fresh_exclusive(
        path.join("state_db"),
        chain,
        batch,
        catalog,
    )?;
    let observation = concrete.observation()?;
    bootstrap.final_chain.state_api_committed_root = H256(observation.committed.state_root);
    bootstrap.final_chain.state_api_committed_period = 0;
    let genesis = bootstrap.storage_genesis_hash;
    let application = bootstrap.bootstrap()?;
    let header = application
        .consensus_query_api_for_bridge()
        .final_chain_block_by_number(0)?;
    ensure!(header.found, "PRODUCTION_FIXTURE_GENESIS_HEADER_MISSING");
    let (native, accounts) = application
        .final_chain
        .genesis_snapshots_for_integration_test()?;
    let empty = RlpStream::new_list(0).out();
    application
        .storage
        .final_chain()
        .write_block_header_with_snapshots(
            0,
            H256(header.hash),
            &header.stored_header_rlp,
            &empty,
            Some(&native),
            Some(&accounts),
        )?;
    let mut pairing = RlpStream::new_list(4);
    pairing
        .append(&1_u64)
        .append(&genesis.as_bytes())
        .append(&observation.identity.database_id.as_slice())
        .append(&observation.identity.chain_id.as_slice());
    application
        .storage
        .final_chain()
        .write_concrete_state_pairing(&pairing.out())?;
    let evidence = format!(
        "path={} genesis={genesis:?} period=0 concrete_root={:?}",
        path.display(),
        observation.committed.state_root
    );
    drop(application);
    drop(concrete);
    Ok(evidence)
}

pub fn signed_fixture_transaction(
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    nonce: u64,
) -> Result<PublicTransactionSubmissionRequest> {
    sign_fixture_transaction(
        selected,
        secret,
        nonce,
        21_000,
        Some(ethereum_types::H160::repeat_byte(0x79)),
        1,
        &[],
    )
}

pub fn signed_fixture_bridge_creation(
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
) -> Result<PublicTransactionSubmissionRequest> {
    // The getter returns the block timestamp. The system sender writes slot zero.
    let init = [
        0x60, 0x15, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x15, 0x60, 0x00, 0xf3, 0x33, 0x15, 0x60,
        0x0b, 0x57, 0x60, 0x01, 0x60, 0x00, 0x55, 0x00, 0x5b, 0x42, 0x60, 0x00, 0x52, 0x60, 0x20,
        0x60, 0x00, 0xf3,
    ];
    sign_fixture_transaction(selected, secret, 0, 200_000, None, 0, &init)
}

fn sign_fixture_transaction(
    selected: &rustaxa_native_config::SelectedProductionConfig,
    secret: [u8; 32],
    nonce: u64,
    gas: u64,
    receiver: Option<ethereum_types::H160>,
    value: u64,
    input: &[u8],
) -> Result<PublicTransactionSubmissionRequest> {
    let bootstrap = crate::production_bootstrap::application_bootstrap_from_selected(selected)?;
    let chain_id = bootstrap.consensus.pbft.network_identity.chain_id;
    let key = SigningKey::from_slice(&secret)?;
    let price = selected.transaction_min_gas_price().max(1);
    let to = receiver
        .as_ref()
        .map_or(&[][..], |address| address.as_bytes());
    let mut unsigned = RlpStream::new_list(9);
    unsigned
        .append(&nonce)
        .append(&price)
        .append(&gas)
        .append(&to)
        .append(&value)
        .append(&input)
        .append(&chain_id)
        .append(&0_u8)
        .append(&0_u8);
    let mut digest = [0; 32];
    let mut hash = Keccak::v256();
    hash.update(&unsigned.out());
    hash.finalize(&mut digest);
    let (signature, recovery) = key.sign_prehash_recoverable(&digest)?;
    let signature = signature.to_bytes();
    let mut signed = RlpStream::new_list(9);
    signed
        .append(&nonce)
        .append(&price)
        .append(&gas)
        .append(&to)
        .append(&value)
        .append(&input)
        .append(
            &(chain_id
                .checked_mul(2)
                .and_then(|v| v.checked_add(35 + u64::from(recovery.to_byte())))
                .context("PRODUCTION_FIXTURE_CHAIN_ID_OVERFLOW")?),
        )
        .append(&U256::from_big_endian(&signature[..32]))
        .append(&U256::from_big_endian(&signature[32..]));
    Ok(PublicTransactionSubmissionRequest {
        transaction_rlp: signed.out().to_vec(),
        expected_chain_id: chain_id,
        maximum_gas_limit: selected.transaction_max_gas(),
        minimum_gas_price: selected.transaction_min_gas_price().into(),
        last_block_number: 0,
        cornus_active: bootstrap.consensus.pbft.cornus_activation_period == 0,
    })
}
