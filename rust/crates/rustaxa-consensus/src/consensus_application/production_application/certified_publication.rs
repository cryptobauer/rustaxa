//! One checked owner barrier for paired publication and compatible startup recovery.
use super::transition_preparation::{
    CertificateInput, capture, qualify_certificate_from_chain, retained_certificate_context,
    reward_votes,
};
use super::*;
use crate::concrete_state_projection::FinalChainConcreteStateProjection;
use crate::final_chain_execution::FinalChainEvmTransactionResult;
use crate::native_session::FinalChainFinishedNativeTransition;
use crate::transaction_service::TransactionServiceAccountNonceFact;
use rustaxa_storage::{ConcreteStateLifecycle, PreparedConcreteTransition};
use rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_storage_catalog;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;

const RULE_KEY: &[u8] = b"rustaxa/production-owner-historical-schedule/v1";

/// One construction role retained privately by the actual execution assembly.
/// This is not a vote admission or a serialized publication permit.
pub struct ProductionOwnerPublicationAuthority {
    issuer: Arc<()>,
    epoch: u64,
}

struct PublicationPermit {
    concrete_generation: u64,
}

pub(super) struct PublicationPredecessor {
    observation: ConcreteLifecycleObservation,
    permit: Option<PublicationPermit>,
}

impl PublicationPredecessor {
    pub(super) fn new(observation: ConcreteLifecycleObservation) -> Self {
        Self {
            observation,
            permit: None,
        }
    }

    fn attach_permit(&mut self) {
        self.permit = Some(PublicationPermit {
            concrete_generation: self.observation.generation,
        });
    }
}

impl ProductionOwnerPublicationAuthority {
    pub(super) fn validate(&self, application: &BoundConsensusApplication) -> Result<()> {
        let predecessor = application.publication_predecessor()?;
        ensure!(
            Arc::ptr_eq(&self.issuer, &application.source.identity)
                && self.epoch == application.transition_policy.epoch
                && predecessor.permit.as_ref().is_some_and(
                    |permit| permit.concrete_generation == predecessor.observation.generation
                ),
            "PRODUCTION_PUBLICATION_AUTHORITY_MISMATCH"
        );
        Ok(())
    }
}

fn rule_bytes(schedule: &PbftExecutionSchedule) -> Vec<u8> {
    let mut out = RlpStream::new_list(2);
    out.append(&1_u8).append(&schedule.digest().as_slice());
    out.out().to_vec()
}

pub(super) fn check_rule_binding(
    storage: &Storage,
    schedule: &PbftExecutionSchedule,
) -> Result<()> {
    if let Some(raw) = storage.get_raw(Column::Status, RULE_KEY)? {
        ensure!(
            raw == rule_bytes(schedule),
            "PRODUCTION_DURABLE_HISTORICAL_SCHEDULE_MISMATCH"
        );
    }
    Ok(())
}

impl BoundConsensusApplication {
    fn publication_predecessor(&self) -> Result<std::sync::MutexGuard<'_, PublicationPredecessor>> {
        self.predecessor
            .lock()
            .map_err(|_| anyhow::anyhow!("PRODUCTION_PREDECESSOR_LOCK_POISONED"))
    }

    pub(super) fn is_current_concrete_observation(
        &self,
        observation: &ConcreteLifecycleObservation,
    ) -> Result<bool> {
        Ok(&self.publication_predecessor()?.observation == observation)
    }

    /// Construction-only entry for the sole closed execution owner. The old
    /// read-only restore path cannot retrieve or upgrade to this capability.
    #[doc(hidden)]
    pub fn restore_existing_for_execution_owner(
        selected: &rustaxa_native_config::SelectedProductionConfig,
        storage: Arc<Storage>,
        owner: Arc<ApplicationOwner>,
        schedule: PbftExecutionSchedule,
        concrete: Arc<ApplicationConcreteState>,
    ) -> Result<(Self, ProductionOwnerPublicationAuthority)> {
        ensure!(
            owner.is_for(&storage) && concrete.is_for(&owner),
            "PRODUCTION_PUBLICATION_FACTORY_OWNER"
        );
        owner.claim_execution_publisher()?;
        let application = Self::restore_existing(selected, storage, owner, schedule, concrete)?;
        let authority = ProductionOwnerPublicationAuthority {
            issuer: Arc::clone(&application.source.identity),
            epoch: application.transition_policy.epoch,
        };
        application.publication_predecessor()?.attach_permit();
        Ok((application, authority))
    }

    #[doc(hidden)]
    pub fn scheduled_pillar_checkpoint(
        &self,
        certificate: &VerifiedCertificateEvidence,
    ) -> Result<Option<u64>> {
        ensure!(
            Arc::ptr_eq(&certificate.issuer, &self.source.identity),
            "PRODUCTION_PILLAR_FOREIGN_CERTIFICATE"
        );
        let period = certificate.request.context().facts().period.as_u64();
        let (ficus, interval) = self.core.pbft.pillar_schedule();
        if !crate::pbft_application_finalization::process_pillar_after_finalization(
            ficus, interval, period,
        ) {
            return Ok(None);
        }
        let prior = crate::pbft_application_finalization::pillar_anchor_state_period(
            period,
            self.core.final_chain.dpos_delegation_delay(),
        )?;
        ensure!(
            prior < period,
            "PRODUCTION_PILLAR_REQUIRES_PREPARED_CHECKPOINT"
        );
        Ok(Some(prior))
    }

    /// Consumes actual checked computation, rechecks its sources, and retires
    /// the graph before the first write. Only a qualified live successor can
    /// reactivate it. No host callback is used.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn publish_verified_transition(
        &self,
        authority: &ProductionOwnerPublicationAuthority,
        evidence: VerifiedCertificateEvidence,
        concrete: PreparedConcreteTransition,
        native: FinalChainFinishedNativeTransition,
        projection: FinalChainConcreteStateProjection,
        system_transactions: &[Vec<u8>],
        pillar: Option<(ConcreteStateIdentity, [u8; 32], [u8; 32])>,
        native_replay: (
            &crate::native_projection_context::FinalChainNativeProjectionContext,
            &crate::native_projection_context::FinalChainLifecycleAuthority,
        ),
        marker: Vec<u8>,
        results: Vec<FinalChainEvmTransactionResult>,
        stop: u8,
    ) -> Result<ConcreteStateIdentity> {
        #[cfg(not(feature = "production_integration_test"))]
        ensure!(stop == 0, "PRODUCTION_TEST_STOP_UNAVAILABLE");
        let action = self.source.read()?;
        action.validate(self)?;
        authority.validate(self)?;
        let facts = evidence.request.context().facts();
        let observation = self.source.concrete_state.observation(&action.guard)?;
        ensure!(
            Arc::ptr_eq(&evidence.issuer, &self.source.identity)
                && facts.state_api_epoch == self.transition_policy.epoch
                && facts.prior == observation.committed
                && self.is_current_concrete_observation(&observation)?
                && facts.schedule == self.source.schedule
                && read_committed_identity(&self.core.storage)? == facts.prior
                && concrete.next_identity().period == facts.period
                && concrete.next_identity().state_root == projection.post_rewards_state.root,
            "PRODUCTION_PUBLICATION_STALE_OR_FOREIGN_SOURCE"
        );
        if facts.owner_generation != action.generation && evidence.scheduler_selected {
            return Err(ScheduledCertificateStale.into());
        }
        ensure!(
            facts.owner_generation == action.generation,
            "PRODUCTION_PUBLICATION_STALE_OR_FOREIGN_SOURCE"
        );
        let synced = evidence.synced.as_ref();
        if let Some(selection) = synced
            && !self.core.pbft.sync_front_matches(&selection.queue)
        {
            return Err(ScheduledCertificateStale.into());
        }
        let inputs = capture(self, evidence.request.snapshot().block_rlp(), synced)?;
        let reward = reward_votes(self, evidence.request.snapshot().block_rlp())?;
        let snapshot = inputs.into_snapshot(
            evidence.request.context(),
            reward.0,
            super::transition_preparation::limits(),
        )?;
        let original = evidence.request.snapshot();
        ensure!(
            snapshot.digest() == original.digest()
                && snapshot.block_rlp() == original.block_rlp()
                && snapshot.dag_blocks() == original.dag_blocks()
                && snapshot.transactions() == original.transactions()
                && snapshot.finalized_exclusions() == original.finalized_exclusions()
                && snapshot.reward_votes() == original.reward_votes(),
            "PRODUCTION_PUBLICATION_ORDERED_INPUT_CHANGED"
        );
        let expected_marker =
            rustaxa_types::concrete_lifecycle::FinalChainConcreteExecutionMarker {
                identity: observation.identity,
                generation: observation
                    .generation
                    .checked_add(1)
                    .context("PRODUCTION_GENERATION_OVERFLOW")?,
                plan_hash: evidence.request.digest(),
                period: facts.period.as_u64(),
                prior_state: crate::concrete_state_projection::FinalChainConcreteState {
                    period: observation.committed.period.as_u64(),
                    root: observation.committed.state_root,
                },
                transactions_hash: snapshot.digest(),
                rewards_hash: super::transition_preparation::digest(
                    &snapshot.reward_votes().concat(),
                ),
            };
        ensure!(
            rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker(
                &expected_marker
            ) == marker,
            "PRODUCTION_PUBLICATION_MARKER_SOURCE_MISMATCH"
        );
        let cert = Rlp::new(&evidence.encoded);
        let round: u64 = cert.val_at(3)?;
        let current = retained_certificate_context(
            self,
            evidence.request.snapshot().block_rlp(),
            synced.map_or(CertificateInput::Verified(round), CertificateInput::Synced),
        )?;
        ensure!(
            current.encoded == evidence.encoded,
            "PRODUCTION_PUBLICATION_CERTIFICATE_CHANGED"
        );
        let current_native = match self.core.final_chain.capture_native_predecessor(
            facts.period,
            NativePredecessorLimits {
                max_retained_source_bytes: NATIVE_SOURCE_LIMIT,
            },
        )? {
            NativePredecessorCapture::Ready(source) => source,
            NativePredecessorCapture::Unavailable(reason) => {
                return Err(ProductionPreparationUnavailable(format!("{reason:?}")).into());
            }
        };
        ensure!(
            current_native.source_digest() == facts.native_prior_digest,
            "PRODUCTION_PUBLICATION_NATIVE_SOURCE_CHANGED"
        );
        let votes = cert
            .at(6)?
            .iter()
            .map(|v| v.as_val::<Vec<u8>>().map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>>>()?;
        let request = if let Some(selection) = synced {
            crate::pbft_application_finalization::PbftApplicationFinalizationRequest {
                period_data_rlp: selection.queue.period_data_rlp.clone(),
                current_cert_vote_rlps: votes,
                synchronous: true,
            }
        } else {
            crate::pbft_application_finalization::build_certified_application_finalization_request(
                &self.core.pbft,
                &self.core.dag_transaction,
                snapshot.block_rlp().to_vec(),
                votes,
                true,
            )?
        };
        let anchor_hash = snapshot.link().pivot_dag_block_hash;
        let anchor_rlp = if anchor_hash.is_zero() {
            &[][..]
        } else {
            snapshot
                .dag_blocks()
                .iter()
                .find(|block| block.hash == anchor_hash)
                .context("PRODUCTION_CERTIFIED_DAG_ANCHOR_UNAVAILABLE")?
                .block_rlp
                .as_slice()
        };
        let proposal_period_update = self
            .core
            .runtime
            .proposal_period_dag_level_update(anchor_rlp)?;
        let (plan, mut batch, scheduler) = native.prepare_owner_publication(
            &self.core.final_chain,
            &evidence.request,
            &request.period_data_rlp,
            proposal_period_update,
            current.blocks_per_year,
            &marker,
            &projection,
            system_transactions,
            native_replay,
            &results,
        )?;
        let live_cache = self.core.final_chain.prepare_owner_cache_install(
            &plan,
            scheduler,
            facts.state_api_epoch,
        )?;
        let nonces = self.prepare_owner_finalized_nonces(&action, facts.prior, &concrete)?;
        let retention = self
            .core
            .pbft
            .process_synced_policy()
            .recently_finalized_factor
            .saturating_mul(self.core.final_chain.dpos_delegation_delay());
        let dag_successor = self.core.dag_transaction.prepare_owner_dag_successor(
            anchor_hash,
            facts.period.as_u64(),
            snapshot.dag_blocks(),
            &request.period_data_rlp,
            retention,
            nonces,
            &mut batch,
        )?;
        let pillar_anchor = match (self.scheduled_pillar_checkpoint(&evidence)?, pillar) {
            (None, None) => None,
            (Some(expected), Some((identity, bridge_root, bridge_epoch))) => {
                let (_, checkpoint) = self
                    .source
                    .finalized_state_identities(Some(expected))?
                    .context("PRODUCTION_PILLAR_CHECKPOINT_MISSING")?;
                ensure!(
                    identity == checkpoint,
                    "PRODUCTION_PILLAR_CHECKPOINT_CHANGED"
                );
                Some(
                    crate::pbft_application_finalization::prepare_owner_pillar_anchor(
                        &self.core.pbft,
                        &self.core.final_chain,
                        facts.period.as_u64(),
                        checkpoint.state_root,
                        bridge_root,
                        bridge_epoch,
                    )?,
                )
            }
            _ => bail!("PRODUCTION_PILLAR_CHECKPOINT_REPORT_MISMATCH"),
        };
        let finalization = self.core.pbft.append_owner_finalization(
            &self.core.dag_transaction,
            request,
            &mut batch,
            evidence.scheduler_selected,
            pillar_anchor,
        )?;
        let next_generation = observation
            .generation
            .checked_add(1)
            .context("PRODUCTION_CONCRETE_GENERATION_EXHAUSTED")?;

        check_rule_binding(&self.core.storage, &self.source.schedule)?;
        self.core.storage.batch_put_raw(
            &mut batch,
            Column::Status,
            RULE_KEY,
            &rule_bytes(&self.source.schedule),
        )?;
        let reward = finalization.reward;
        let cursor = crate::pbft_vote_runtime::RewardVoteCursor {
            period: reward.period,
            round: reward.round,
            step: reward.step,
            block_hash: reward.block_hash,
        };
        let primary = self
            .source
            .owner
            .prepare_certified_primary_with_reward_reset(&action.guard, &batch, reward)?;
        let catalog = encode_concrete_storage_catalog(projection.storage.iter().map(|s| {
            ConcreteStorageSlot {
                address: s.contract,
                key: s.key,
            }
        }));
        let mut binding = RlpStream::new_list(8);
        binding
            .append(&1_u8)
            .append(&facts.schedule.genesis().as_slice())
            .append(&self.production_configuration.digest().as_slice())
            .append(&self.source.schedule.digest().as_slice())
            .append(&snapshot.block_rlp())
            .append(&round)
            .append(&evidence.encoded)
            .append(&evidence.request_digest().as_slice());
        let prepared = self.source.concrete_state.prepare_certified_state(
            &action.guard,
            concrete,
            plan.concrete_projection_hash,
            catalog,
            primary,
            binding.out().to_vec(),
        )?;
        action.validate(self)?;
        let permit = self
            .publication_predecessor()?
            .permit
            .take()
            .context("PRODUCTION_PUBLICATION_AUTHORITY_CONSUMED")?;
        ensure!(
            permit.concrete_generation == observation.generation,
            "PRODUCTION_PUBLICATION_AUTHORITY_MISMATCH"
        );
        self.source
            .active
            .store(false, std::sync::atomic::Ordering::Release);
        let (identity, continuation, reset_generation) = self
            .source
            .concrete_state
            .publish_certified_state_retaining_scope(action.guard, prepared, stop)?;
        live_cache.install(&plan, &continuation)?;
        let committed = self.core.pbft.verified_votes().commit_reward_vote_cursor(
            crate::pbft_vote_runtime::RewardVoteCursorCommitRequest {
                cursor,
                reset_generation: reset_generation.context("PRODUCTION_REWARD_RESET_MISSING")?,
            },
        )?;
        ensure!(
            committed.status != crate::pbft_vote_runtime::RewardVoteCursorCommitStatus::Rejected,
            "PRODUCTION_REWARD_CURSOR_REJECTED:{}",
            committed.error_code
        );
        self.core.pbft.install_owner_pillar_updates(
            finalization.pillar_reference,
            finalization.pillar_anchor,
        )?;
        if let Some(successor) = finalization.successor {
            let observed = self.source.concrete_state.observation(&continuation)?;
            ensure!(
                observed.committed == identity
                    && observed.identity == observation.identity
                    && observed.generation == next_generation
                    && observed.pending_marker_rlp.is_empty(),
                "PRODUCTION_SUCCESSOR_OBSERVATION_MISMATCH"
            );
            self.core
                .pbft
                .install_owner_successor(successor, &continuation)?;
            if let Some(selection) = synced {
                self.core.pbft.acknowledge_synced_period(&selection.queue)?;
            }
            self.core.dag_transaction.install_owner_dag_successor(
                dag_successor,
                finalization.sortition,
                &continuation,
            )?;
            let mut predecessor = self.publication_predecessor()?;
            *predecessor = PublicationPredecessor::new(observed);
            predecessor.attach_permit();
            self.source
                .active
                .store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(identity)
    }

    fn prepare_owner_finalized_nonces(
        &self,
        action: &BoundApplicationAction<'_>,
        prior: ConcreteStateIdentity,
        concrete: &PreparedConcreteTransition,
    ) -> Result<Vec<TransactionServiceAccountNonceFact>> {
        if !concrete.next_identity().period.as_u64().is_multiple_of(100) {
            return Ok(Vec::new());
        }
        let senders = self
            .core
            .dag_transaction
            .transaction_queue_proposable_accounts()?;
        self.with_finalized_state_in_action(action, None, |readers, identity| {
            use rustaxa_types::concrete_state::ConcreteRead;
            ensure!(
                identity == prior,
                "PRODUCTION_FINALIZED_NONCE_PREDECESSOR_MISMATCH"
            );
            senders
                .into_iter()
                .map(|sender| {
                    let row = match concrete.account_update(sender.0)? {
                        Some(row) => row,
                        None => readers.account_at(prior, sender.0)?,
                    };
                    let (account_found, account_nonce) = match row {
                        ConcreteRead::Present(row) => (
                            true,
                            bounded_account_integer(&row.account.nonce.to_bytes())?.to_big_endian(),
                        ),
                        ConcreteRead::Absent | ConcreteRead::Tombstone => (false, [0; 32]),
                    };
                    Ok(TransactionServiceAccountNonceFact {
                        sender: sender.0,
                        account_found,
                        account_nonce,
                    })
                })
                .collect()
        })?
        .context("PRODUCTION_FINALIZED_NONCE_CHECKPOINT_UNAVAILABLE")
    }

    /// Qualifies durable recovery before the factory exposes any application client.
    #[doc(hidden)]
    pub fn recover_existing_publication(
        selected: &rustaxa_native_config::SelectedProductionConfig,
        storage: &Arc<Storage>,
        owner: &ApplicationOwner,
        concrete: &mut ConcreteStateLifecycle,
    ) -> Result<()> {
        ensure!(owner.is_for(storage), "PRODUCTION_RECOVERY_FOREIGN_OWNER");
        let read = owner.try_read(READ_WAIT)?;
        let Some(intent) = storage.pending_certified_intent()? else {
            return Ok(());
        };
        let binding = Rlp::new(intent.binding());
        ensure!(
            binding.item_count()? == 8 && binding.val_at::<u8>(0)? == 1,
            "PRODUCTION_RECOVERY_BINDING_VERSION"
        );
        let bootstrap = crate::production_bootstrap::application_bootstrap_from_selected(selected)?;
        exact_status(
            storage,
            StatusField::DbMajorVersion,
            u64::from(bootstrap.schema_major),
        )?;
        exact_status(
            storage,
            StatusField::DbMinorVersion,
            u64::from(bootstrap.schema_minor),
        )?;
        exact_status(
            storage,
            StatusField::FinalChainRootPolicy,
            FINAL_CHAIN_CONCRETE_ROOT_POLICY_V1,
        )?;
        ensure!(
            storage
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none(),
            "PRODUCTION_RECOVERY_LEGACY_PUBLICATION_CONFLICT"
        );
        ensure!(
            storage.metadata().genesis_hash()?.as_deref()
                == Some(bootstrap.storage_genesis_hash.as_bytes()),
            "PRODUCTION_RECOVERY_GENESIS"
        );
        let genesis_raw = storage
            .final_chain()
            .block_header_raw(0)?
            .context("PRODUCTION_RECOVERY_GENESIS_HEADER")?;
        let genesis =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&genesis_raw))?
                .state_root;
        let input = &bootstrap.final_chain;
        let final_chain = FinalChain::new_with_genesis_state_root_and_ficus_activation(
            Arc::clone(storage),
            input.block_gas_limit,
            input.genesis_timestamp,
            genesis,
            true,
            input.genesis_accounts.clone(),
            input.genesis_validators.clone(),
            input.genesis_dpos.clone(),
            input.rewards.clone(),
            bootstrap.consensus.pbft.ficus_activation_period.into(),
        )?;
        let configuration = crate::ExecutionConfiguration::capture(
            storage,
            bootstrap.storage_genesis_hash,
            &final_chain,
            &bootstrap.consensus,
            input.bridge_contract_address,
        )?;
        let production = ProductionConfiguration::new(
            &configuration,
            &ProductionAdmissionPolicy::from_selected(selected),
        );
        let schedule = PbftExecutionSchedule::new(
            bootstrap.consensus.pbft.network_identity.chain_id,
            bootstrap.storage_genesis_hash.0,
            production.digest(),
            None,
        );
        check_rule_binding(storage, &schedule)?;
        ensure!(
            binding.val_at::<Vec<u8>>(1)? == bootstrap.storage_genesis_hash.as_bytes()
                && binding.val_at::<Vec<u8>>(2)? == production.digest()
                && binding.val_at::<Vec<u8>>(3)? == schedule.digest()
                && storage.metadata().pbft_execution_schedule_raw()?.is_none(),
            "PRODUCTION_RECOVERY_CONFIGURATION_OR_SCHEDULE"
        );
        let block: Vec<u8> = binding.val_at(4)?;
        let link = rustaxa_types::pbft::PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))?;
        let head = load_persisted_pbft_chain_head_identity(storage)?
            .context("PRODUCTION_RECOVERY_PBFT_HEAD")?;
        ensure!(
            read_committed_identity(storage)? == intent.prior().committed
                && head.size == intent.prior().committed.period.as_u64()
                && link.period == intent.next_identity()?.period.as_u64()
                && link.prev_block_hash == head.last_pbft_block_hash,
            "PRODUCTION_RECOVERY_HEAD_LINEAGE"
        );
        let observed = concrete.observation()?;
        ensure!(
            observed.identity == intent.prior().identity
                && observed.identity.chain_id.as_slice()
                    == concrete_chain_config_identity(bootstrap.storage_genesis_hash),
            "PRODUCTION_RECOVERY_PAIRING"
        );
        let pairing = storage
            .final_chain()
            .concrete_state_pairing_raw()?
            .context("PRODUCTION_RECOVERY_PAIRING_MISSING")?;
        let mut expected = RlpStream::new_list(4);
        expected
            .append(&1_u64)
            .append(&bootstrap.storage_genesis_hash.as_bytes())
            .append(&observed.identity.database_id.as_slice())
            .append(&observed.identity.chain_id.as_slice());
        ensure!(
            expected.out().as_ref() == pairing,
            "PRODUCTION_RECOVERY_PAIRING_MISMATCH"
        );
        let encoded: Vec<u8> = binding.val_at(6)?;
        let cert = Rlp::new(&encoded);
        let round: u64 = binding.val_at(5)?;
        ensure!(
            cert.item_count()? == 10
                && cert.val_at::<Vec<u8>>(0)? == b"rustaxa-verified-certificate-context"
                && cert.val_at::<u8>(1)? == 2
                && cert.val_at::<u64>(2)? == link.period
                && cert.val_at::<u64>(3)? == round
                && cert.val_at::<H256>(4)? == link.block_hash,
            "PRODUCTION_RECOVERY_CERTIFICATE_IDENTITY"
        );
        ensure!(
            encoded.len() <= 16 * 1024 * 1024
                && cert.at(6)?.item_count()? > 0
                && cert.at(6)?.item_count()? <= 65536,
            "PRODUCTION_RECOVERY_CERTIFICATE_LIMIT"
        );
        let request_digest: Vec<u8> = binding.val_at(7)?;
        ensure!(
            request_digest.as_slice() == intent.plan_hash()?,
            "PRODUCTION_RECOVERY_REQUEST_PLAN_MISMATCH"
        );
        let mut canonical = RlpStream::new_list(8);
        canonical
            .append(&1_u8)
            .append(&bootstrap.storage_genesis_hash.as_bytes())
            .append(&production.digest().as_slice())
            .append(&schedule.digest().as_slice())
            .append(&block)
            .append(&round)
            .append(&encoded)
            .append(&request_digest);
        ensure!(
            canonical.out().as_ref() == intent.binding(),
            "PRODUCTION_RECOVERY_BINDING_NONCANONICAL"
        );
        let mut voters = std::collections::BTreeSet::new();
        let mut records = Vec::new();
        for raw in cert.at(6)?.iter() {
            let vote_rlp: Vec<u8> = raw.as_val()?;
            let vote = crate::pbft_vote_validation::inspect_canonical_pbft_vote(&vote_rlp)?;
            ensure!(
                vote.signature_valid
                    && vote.block_hash == link.block_hash
                    && vote.period == link.period
                    && vote.round == round
                    && vote.step == cert.val_at::<u64>(5)?
                    && vote.vote_type == PbftVoteType::Cert
                    && vote.has_embedded_weight
                    && voters.insert(vote.recovered_voter),
                "PRODUCTION_RECOVERY_CERTIFICATE_SOURCE"
            );
            records.push(crate::pbft_vote_payload::PbftVotePayloadRecord {
                hash: vote.vote_hash,
                vote_rlp,
            });
        }
        ensure!(
            !records.is_empty() && records.len() <= 65536,
            "PRODUCTION_RECOVERY_CERTIFICATE_COUNT"
        );
        let policy = transition_preparation::TransitionPolicy::new(&bootstrap)?;
        let checked = qualify_certificate_from_chain(
            &final_chain,
            &policy,
            head.size,
            link.period,
            &records,
        )?;
        ensure!(
            checked == cert.val_at::<Vec<u8>>(9)?,
            "PRODUCTION_RECOVERY_CERTIFICATE_NATIVE_FACTS"
        );
        owner.recover_certified_state(read, concrete, intent)
    }
}
