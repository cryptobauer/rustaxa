//! Pure native projection and storage planning for the closed Rust owner.
use super::reward_scheduler::{
    FinalChainPreparedRewardSchedulerSuccessor, FinalChainRewardSchedulerPublicationBinding,
    FinalChainRewardSchedulerRuntime,
};
use super::*;
use crate::concrete_state_projection::{
    FinalChainConcreteStateProvenance, concrete_state_bytes_digest,
    encode_concrete_state_provenance,
};
use crate::final_chain_execution::*;
use crate::native_session::FinalChainFinishedNativeTransition;
use crate::pbft_execution_validation::PbftExecutionRequest;
use rustaxa_storage::StorageWriteBatch;

impl FinalChainFinishedNativeTransition {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_owner_publication(
        &self,
        owner: &Arc<FinalChain>,
        request: &PbftExecutionRequest,
        period_data: &[u8],
        proposal_period_update: crate::final_chain_execution::FinalChainProposalPeriodDagLevelUpdate,
        blocks_per_year: u32,
        marker_raw: &[u8],
        projection: &FinalChainConcreteStateProjection,
        system_transactions: &[Vec<u8>],
        native_replay: (
            &crate::native_projection_context::FinalChainNativeProjectionContext,
            &crate::native_projection_context::FinalChainLifecycleAuthority,
        ),
        results: &[FinalChainEvmTransactionResult],
    ) -> Result<(
        FinalChainExternalEvmPublicationPlan,
        StorageWriteBatch,
        FinalChainPreparedRewardSchedulerSuccessor,
    )> {
        ensure!(
            Arc::ptr_eq(&self.owner, owner)
                && self.request_id == request.digest()
                && self.state_api_epoch == request.context().facts().state_api_epoch
                && request.context().facts().concrete_generation.checked_add(1)
                    == Some(self.generation),
            "PRODUCTION_NATIVE_PUBLICATION_OWNER_MISMATCH"
        );
        self.source.validate(owner)?;
        self.context.validate(owner)?;
        let facts = request.context().facts();
        let marker = decode_concrete_execution_marker(marker_raw)?;
        ensure!(
            marker.plan_hash == request.digest()
                && marker.period == facts.period.as_u64()
                && projection.plan_hash == request.digest()
                && projection.identity == marker.identity
                && projection.generation == marker.generation
                && projection.prior_state == marker.prior_state
                && projection.post_rewards_state.period == facts.period.as_u64()
                && projection.transaction_effects.len() == results.len()
                && results.len()
                    == request.snapshot().transactions().len() + system_transactions.len(),
            "PRODUCTION_PUBLICATION_PROJECTION_BINDING"
        );
        let reward_weights = request
            .snapshot()
            .reward_votes()
            .iter()
            .map(|v| {
                crate::pbft_vote_validation::inspect_canonical_pbft_vote(v)
                    .map(|v| v.embedded_weight)
            })
            .collect::<Result<Vec<_>>>()?;
        let execution =
            crate::pbft_application_finalization::final_chain_execution_request_from_period_data(
                period_data,
                request.snapshot().reward_votes(),
                &reward_weights,
                blocks_per_year,
                facts.gas_limit.into(),
            )?;
        ensure!(
            execution.transactions.len() + system_transactions.len() == results.len(),
            "PRODUCTION_PERIOD_TRANSACTION_COUNT"
        );
        let mut transactions = execution
            .transactions
            .iter()
            .enumerate()
            .map(|(index, tx)| {
                Ok(FinalChainEvmTransactionInput {
                    position: u32::try_from(index)?.into(),
                    hash: tx.hash,
                    sender: tx.sender,
                    receiver: tx.receiver,
                    nonce: tx.nonce.clone(),
                    value: tx.value,
                    gas_price: tx.gas_price,
                    gas_limit: tx.gas_limit,
                    data: tx.data.clone(),
                    rlp: tx.rlp.clone(),
                    kind: if tx.receiver.is_none() {
                        FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CREATE
                    } else {
                        FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL
                    },
                    is_system: false,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        transactions.extend(decode_system_transaction_inputs(
            transactions.len(),
            system_transactions,
        )?);
        for ((result, tx), effect) in results
            .iter()
            .zip(&transactions)
            .zip(&projection.transaction_effects)
        {
            ensure!(
                result.hash == tx.hash
                    && result.position == tx.position
                    && effect.transaction_rlp
                        == crate::concrete_state_projection::encode_concrete_evm_transaction(tx)
                    && effect.execution_result_rlp
                        == crate::concrete_state_projection::encode_concrete_execution_result(
                            result
                        ),
                "PRODUCTION_PUBLICATION_TRANSACTION_BINDING"
            );
        }
        let projection_raw =
            crate::concrete_state_projection::encode_concrete_state_projection(projection);
        ensure!(
            crate::concrete_state_projection::decode_concrete_state_projection(&projection_raw)?
                == *projection,
            "PRODUCTION_PUBLICATION_PROJECTION_CANONICAL"
        );
        let projection_hash = concrete_state_bytes_digest(&projection_raw);
        let (context, authority) = native_replay;
        let transaction_ids = transactions
            .iter()
            .map(|tx| (tx.position.as_u32(), tx.hash))
            .collect::<Vec<_>>();
        crate::native_projection_context::validate_lifecycle_authority(
            context,
            authority,
            request.digest(),
            self.state_api_epoch,
            projection_hash,
            &transaction_ids,
        )?;
        context.validate_binding(request.digest(), projection)?;

        let total_reward = self.outcome.total_reward.to_fixed_be_bytes().to_vec();
        let (dpos, accounts, scheduler) = owner.external_evm_concrete_projection_with_basis(
            FinalChainProjectionIdentity {
                request_id: request.digest(),
                state_api_epoch: self.state_api_epoch,
                block_number: facts.period,
                block_author: facts.author,
            },
            &transactions,
            projection,
            &self.plan,
            &total_reward,
            (Some(context), Some(&self.delayed)),
            Some(self.basis),
        )?;
        ensure!(
            encode_dpos_snapshot_rlp(&self.outcome.dpos_snapshot)? == dpos,
            "PRODUCTION_NATIVE_REPLAY_SNAPSHOT_MISMATCH"
        );
        self.source.validate(owner)?;
        self.context.validate(owner)?;
        let provenance = encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: projection.post_rewards_state,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash: projection.catalog_hash,
        });
        let prior = FinalChainExternalEvmCommittedStateDescriptor {
            period: facts.prior.period,
            state_root: facts.prior.state_root,
        };
        let evm = FinalChainEvmExecutionRequest {
            request_id: request.digest(),
            state_api_epoch: self.state_api_epoch,
            period: facts.period,
            prior_state: prior,
            concrete_marker_rlp: marker_raw.to_vec(),
            concrete_plan_hash: marker.plan_hash,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            block_author: facts.author,
            timestamp: facts.timestamp,
            block_gas_limit: facts.gas_limit.into(),
            transactions,
        };
        let mut cumulative = 0_u64;
        for result in results {
            cumulative = cumulative
                .checked_add(result.gas_used.as_u64())
                .context("PRODUCTION_CUMULATIVE_GAS_OVERFLOW")?;
            ensure!(
                result.cumulative_gas_used.as_u64() == cumulative,
                "PRODUCTION_CUMULATIVE_GAS_MISMATCH"
            );
            ensure!(
                result.status <= 1
                    && ((result.status == 1)
                        == (result.code_error.is_empty() && result.consensus_error.is_empty())),
                "PRODUCTION_TRANSACTION_STATUS_MISMATCH"
            );
        }
        let report = FinalChainEvmExecutionReport {
            request_id: request.digest(),
            state_api_epoch: self.state_api_epoch,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: prior,
            concrete_marker_rlp: marker_raw.to_vec(),
            concrete_plan_hash: marker.plan_hash,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            post_transaction_state_root: projection.post_transaction_state.root,
            cumulative_gas_used: cumulative.into(),
            results: results.to_vec(),
        };
        let rewards = FinalChainEvmRewardsReport {
            request_id: request.digest(),
            state_api_epoch: self.state_api_epoch,
            period: facts.period,
            status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
            prior_state: prior,
            post_transaction_state_root: projection.post_transaction_state.root,
            post_rewards_state_root: projection.post_rewards_state.root,
            concrete_marker_rlp: marker_raw.to_vec(),
            concrete_plan_hash: marker.plan_hash,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            concrete_projection_rlp: projection_raw,
            concrete_projection_hash: projection_hash,
            concrete_provenance_rlp: provenance,
            total_reward,
        };
        let metadata = request.snapshot().metadata();
        let commit = build_external_evm_commit_plan(
            &execution,
            metadata,
            facts.period,
            &evm,
            &report,
            &rewards,
        )?;
        let mut plan = build_external_evm_publication_plan(
            owner,
            request.snapshot().block_rlp(),
            metadata,
            facts.period,
            &evm,
            &commit,
        )?;
        plan.dpos_snapshot_rlp = dpos;
        plan.account_snapshot_rlp = accounts;
        plan.rewards_stats_update = owner.validate_external_evm_rewards_stats_plan(&self.plan)?;
        plan.proposal_period_dag_level_update = proposal_period_update;
        plan.plan_id = final_chain_external_evm_publication_plan_id(&plan);
        let indexes = plan
            .transaction_publications
            .iter()
            .map(|tx| FinalChainTransactionIndexUpdate {
                transaction_hash: H256(tx.transaction_hash),
                position: tx.position.as_u32(),
                is_system: tx.is_system,
                system_transaction_rlp: tx.is_system.then_some(tx.transaction_rlp.as_slice()),
                receipt_rlp: &tx.receipt_rlp,
            })
            .collect::<Vec<_>>();
        let update = plan.proposal_period_dag_level_update.has_update.then_some(
            rustaxa_storage::FinalChainProposalPeriodDagLevelUpdate {
                level: plan.proposal_period_dag_level_update.level,
                period: facts.period.as_u64(),
            },
        );
        let batch = owner
            .storage
            .final_chain()
            .prepare_block_header_with_snapshots_execution_status_and_rewards_stats(
                facts.period.as_u64(),
                H256(plan.block_hash),
                &plan.stored_header_rlp,
                &plan.receipts_rlp,
                Some(&plan.dpos_snapshot_rlp),
                Some(&plan.account_snapshot_rlp),
                Some(owner.finalization_execution_status(
                    plan.executed_dag_blocks,
                    plan.executed_transactions,
                )?),
                external_evm_rewards_stats_storage_update(&plan.rewards_stats_update)?,
                Some(FinalChainLogBloomIndexUpdate {
                    block_number: facts.period.as_u64(),
                    bloom: &plan.indexed_log_bloom,
                }),
                &indexes,
                Some(FinalChainPeriodSystemTransactionsUpdate {
                    period: facts.period.as_u64(),
                    hashes_rlp: &plan.system_transaction_hashes_rlp,
                }),
                update,
                false,
            )?;
        Ok((plan, batch, scheduler))
    }
}

pub(crate) struct PreparedOwnerFinalChainCaches<'a> {
    chain: &'a FinalChain,
    prior: FinalChainBlockNumber,
    rewards_generation: u64,
    plan_id: [u8; 32],
    dpos: DposSnapshot,
    accounts: HashMap<[u8; 20], Account>,
    prior_scheduler: FinalChainRewardSchedulerRuntime,
    next_scheduler: FinalChainRewardSchedulerRuntime,
}

impl FinalChain {
    pub(crate) fn prepare_owner_cache_install(
        &self,
        plan: &FinalChainExternalEvmPublicationPlan,
        scheduler: FinalChainPreparedRewardSchedulerSuccessor,
        state_api_epoch: u64,
    ) -> Result<PreparedOwnerFinalChainCaches<'_>> {
        let prior = self.last_block_number_typed()?;
        ensure!(
            prior.checked_next() == Some(plan.period),
            "PRODUCTION_CACHE_INSTALL_PRIOR"
        );
        let dpos = decode_dpos_snapshot_rlp(&plan.dpos_snapshot_rlp)?;
        let accounts = decode_account_snapshot_rlp(&plan.account_snapshot_rlp)?;
        let rewards = self
            .rewards_stats_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain rewards stats runtime lock poisoned"))?;
        ensure!(
            rewards.durable_head == prior && rewards.generation.checked_add(1).is_some(),
            "PRODUCTION_CACHE_INSTALL_REWARDS_PRIOR"
        );
        let rewards_generation = rewards.generation;
        drop(rewards);
        let marker = decode_concrete_execution_marker(&plan.concrete_marker_rlp)?;
        let prior_scheduler = self
            .reward_scheduler_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain reward scheduler runtime lock poisoned"))?
            .clone();
        let next_scheduler = prior_scheduler.prepare_paired_install(
            scheduler,
            FinalChainRewardSchedulerPublicationBinding {
                request_id: plan.request_id,
                expected_parent: prior,
                period: plan.period,
                state_api_epoch,
                concrete_database_id: marker.identity.database_id,
                concrete_generation: marker.generation,
                concrete_projection_hash: plan.concrete_projection_hash,
                publication_plan_id: plan.plan_id,
            },
        )?;
        Ok(PreparedOwnerFinalChainCaches {
            chain: self,
            prior,
            rewards_generation,
            plan_id: plan.plan_id,
            dpos,
            accounts,
            prior_scheduler,
            next_scheduler,
        })
    }
}

impl PreparedOwnerFinalChainCaches<'_> {
    pub(crate) fn install(
        self,
        plan: &FinalChainExternalEvmPublicationPlan,
        scope: &rustaxa_storage::application_control::ApplicationReadScope<'_>,
    ) -> Result<()> {
        let chain = self.chain;
        let period = plan.period.as_u64();
        let storage = chain.storage.final_chain();
        ensure!(
            scope.is_current()
                && self.plan_id == plan.plan_id
                && self.prior.checked_next() == Some(plan.period)
                && chain.last_block_number_typed()? == plan.period
                && chain.block_hash(plan.period)?.as_deref() == Some(plan.block_hash.as_slice())
                && chain.block_number(plan.block_hash)? == Some(period)
                && storage.dpos_snapshot_raw(period)?.as_deref()
                    == Some(plan.dpos_snapshot_rlp.as_slice())
                && storage.account_snapshot_raw(period)?.as_deref()
                    == Some(plan.account_snapshot_rlp.as_slice())
                && chain.verify_external_evm_rewards_stats_update(&plan.rewards_stats_update)?,
            "PRODUCTION_CACHE_INSTALL_COMMITTED_STATE_MISMATCH"
        );
        let (durable_head, rewards_runtime) =
            rewards_stats_runtime_from_storage(&chain.storage, &chain.rewards_config, false)?;
        chain.insert_dpos_snapshot(plan.period, self.dpos)?;
        chain.insert_account_snapshot(plan.period, self.accounts)?;
        let mut rewards = chain
            .rewards_stats_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain rewards stats runtime lock poisoned"))?;
        ensure!(
            durable_head == plan.period
                && rewards.durable_head == self.prior
                && rewards.generation == self.rewards_generation,
            "PRODUCTION_CACHE_INSTALL_REWARDS_SOURCE_CHANGED"
        );
        rewards.durable_head = durable_head;
        rewards.runtime = rewards_runtime;
        rewards.generation = self.rewards_generation + 1;
        drop(rewards);
        let mut scheduler = chain
            .reward_scheduler_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("final-chain reward scheduler runtime lock poisoned"))?;
        ensure!(
            *scheduler == self.prior_scheduler,
            "PRODUCTION_CACHE_INSTALL_REWARD_SCHEDULER_SOURCE_CHANGED"
        );
        *scheduler = self.next_scheduler;
        Ok(())
    }
}
