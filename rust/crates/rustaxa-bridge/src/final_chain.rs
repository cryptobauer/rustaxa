use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeFinalChain;
use crate::ffi::BridgeFinalChainExecutionSession;
use crate::ffi::BridgeStorage;
use rustaxa_consensus::{
    Account, FinalChain, FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE,
    FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY,
};

const PBFT_FINAL_CHAIN_FACT_STATUS_READY: u8 = 0;
const PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE: u8 = 1;
const PBFT_FINAL_CHAIN_FACT_STATUS_INVALID: u8 = 2;

fn account_to_lookup(account: Option<Account>) -> rustaxa_ffi::AccountLookup {
    match account {
        Some(account) => rustaxa_ffi::AccountLookup {
            found: true,
            nonce: account.nonce,
            balance: account.balance,
            storage_root_hash: account.storage_root_hash,
            code_hash: account.code_hash,
            code_size: account.code_size,
        },
        None => rustaxa_ffi::AccountLookup {
            found: false,
            nonce: 0,
            balance: vec![],
            storage_root_hash: [0; 32],
            code_hash: [0; 32],
            code_size: 0,
        },
    }
}

fn pbft_final_chain_hash_result(
    status: u8,
    expected_hash: [u8; 32],
    actual_hash: [u8; 32],
    error_code: impl Into<String>,
) -> rustaxa_ffi::PbftFinalChainHashResult {
    rustaxa_ffi::PbftFinalChainHashResult {
        status,
        expected_hash,
        actual_hash,
        error_code: error_code.into(),
    }
}

fn finalization_transaction_from_ffi(
    transaction: rustaxa_ffi::FinalizationTransaction,
) -> rustaxa_consensus::FinalizationTransaction {
    rustaxa_consensus::FinalizationTransaction {
        hash: transaction.hash,
        sender: transaction.sender,
        receiver: if transaction.receiver_found {
            Some(transaction.receiver)
        } else {
            None
        },
        nonce: transaction.nonce,
        value: transaction.value,
        gas_price: transaction.gas_price,
        gas_limit: transaction.gas_limit,
        data: transaction.data,
        rlp: transaction.rlp,
    }
}

fn finalized_dag_block_from_ffi(
    dag_block: rustaxa_ffi::FinalizationDagBlock,
) -> rustaxa_consensus::FinalizationDagBlock {
    rustaxa_consensus::FinalizationDagBlock {
        author: dag_block.author,
        difficulty: dag_block.difficulty,
        transaction_hashes: dag_block
            .transaction_hashes
            .into_iter()
            .map(|hash| hash.hash)
            .collect(),
    }
}

fn reward_cert_vote_from_ffi(
    vote: rustaxa_ffi::RewardsCertVoteFact,
) -> rustaxa_consensus::RewardCertVoteFact {
    rustaxa_consensus::RewardCertVoteFact {
        voter: vote.voter.into(),
        weight: vote.weight,
        period: vote.period,
    }
}

fn final_chain_execution_request_from_ffi(
    request: rustaxa_ffi::FinalChainExecutionRequest,
) -> rustaxa_consensus::FinalChainExecutionRequest {
    rustaxa_consensus::FinalChainExecutionRequest {
        pbft_block_rlp: request.pbft_block_rlp,
        transactions: request
            .transactions
            .into_iter()
            .map(finalization_transaction_from_ffi)
            .collect(),
        finalized_dag_blocks: request
            .finalized_dag_blocks
            .into_iter()
            .map(finalized_dag_block_from_ffi)
            .collect(),
        blocks_per_year: request.blocks_per_year,
        cert_votes: request
            .cert_votes
            .into_iter()
            .map(reward_cert_vote_from_ffi)
            .collect(),
        block_gas_limit: request.block_gas_limit,
        mode: request.mode,
    }
}

fn final_chain_execution_request_from_compat(
    pbft_block_rlp: Vec<u8>,
    transactions: Vec<rustaxa_ffi::FinalizationTransaction>,
    finalized_dag_blocks: Vec<rustaxa_ffi::FinalizationDagBlock>,
    blocks_per_year: u32,
    cert_votes: Vec<rustaxa_ffi::RewardsCertVoteFact>,
) -> rustaxa_consensus::FinalChainExecutionRequest {
    final_chain_execution_request_from_ffi(rustaxa_ffi::FinalChainExecutionRequest {
        pbft_block_rlp,
        transactions,
        finalized_dag_blocks,
        blocks_per_year,
        cert_votes,
        block_gas_limit: 0,
        mode: FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY,
    })
}

fn evm_transaction_input_to_ffi(
    transaction: rustaxa_consensus::FinalChainEvmTransactionInput,
) -> rustaxa_ffi::FinalChainEvmTransactionInput {
    let (receiver_found, receiver) = match transaction.receiver {
        Some(receiver) => (true, receiver),
        None => (false, [0; 20]),
    };
    rustaxa_ffi::FinalChainEvmTransactionInput {
        position: transaction.position,
        hash: transaction.hash,
        sender: transaction.sender,
        receiver_found,
        receiver,
        nonce: transaction.nonce,
        value: transaction.value,
        gas_price: transaction.gas_price,
        gas_limit: transaction.gas_limit,
        data: transaction.data,
        rlp: transaction.rlp,
        kind: transaction.kind,
        is_system: transaction.is_system,
    }
}

fn system_transaction_request_to_ffi(
    request: rustaxa_consensus::FinalChainSystemTransactionRequest,
) -> rustaxa_ffi::FinalChainSystemTransactionRequest {
    rustaxa_ffi::FinalChainSystemTransactionRequest {
        request_id: request.request_id,
        period: request.period,
        regular_transaction_count: request.regular_transaction_count,
    }
}

fn system_transaction_report_from_ffi(
    report: rustaxa_ffi::FinalChainSystemTransactionReport,
) -> rustaxa_consensus::FinalChainSystemTransactionReport {
    rustaxa_consensus::FinalChainSystemTransactionReport {
        request_id: report.request_id,
        period: report.period,
        transactions: report
            .transactions
            .into_iter()
            .map(|transaction| transaction.data)
            .collect(),
    }
}

fn evm_request_to_ffi(
    request: rustaxa_consensus::FinalChainEvmExecutionRequest,
) -> rustaxa_ffi::FinalChainEvmExecutionRequest {
    rustaxa_ffi::FinalChainEvmExecutionRequest {
        request_id: request.request_id,
        period: request.period,
        block_author: request.block_author,
        timestamp: request.timestamp,
        block_gas_limit: request.block_gas_limit,
        transactions: request
            .transactions
            .into_iter()
            .map(evm_transaction_input_to_ffi)
            .collect(),
    }
}

fn evm_rewards_request_to_ffi(
    request: rustaxa_consensus::FinalChainEvmRewardsRequest,
) -> rustaxa_ffi::FinalChainEvmRewardsRequest {
    rustaxa_ffi::FinalChainEvmRewardsRequest {
        request_id: request.request_id,
        period: request.period,
        block_author: request.block_author,
        block_gas_used: request.block_gas_used,
        transaction_gas_used: request.transaction_gas_used,
        transaction_fees: request
            .transaction_fees
            .into_iter()
            .map(|data| rustaxa_ffi::ReceiptRlp { data })
            .collect(),
        finalized_dag_block_count: request.finalized_dag_block_count,
    }
}

fn execution_step_to_ffi(
    step: rustaxa_consensus::FinalChainExecutionStep,
) -> rustaxa_ffi::FinalChainExecutionStep {
    rustaxa_ffi::FinalChainExecutionStep {
        status: step.status,
        action: step.action,
        period: step.period,
        external_evm_transaction_count: step.external_evm_transaction_count,
        evm_request: evm_request_to_ffi(step.evm_request),
        evm_rewards_request: evm_rewards_request_to_ffi(step.evm_rewards_request),
        system_transaction_request: system_transaction_request_to_ffi(
            step.system_transaction_request,
        ),
        error_code: step.error_code,
    }
}

fn evm_report_from_ffi(
    report: rustaxa_ffi::FinalChainEvmExecutionReport,
) -> rustaxa_consensus::FinalChainEvmExecutionReport {
    rustaxa_consensus::FinalChainEvmExecutionReport {
        request_id: report.request_id,
        status: report.status,
        state_root: report.state_root,
        cumulative_gas_used: report.cumulative_gas_used,
        results: report
            .results
            .into_iter()
            .map(|result| rustaxa_consensus::FinalChainEvmTransactionResult {
                position: result.position,
                hash: result.hash,
                status: result.status,
                gas_used: result.gas_used,
                cumulative_gas_used: result.cumulative_gas_used,
                receipt_rlp: result.receipt_rlp,
                logs: result
                    .logs
                    .into_iter()
                    .map(|log| rustaxa_consensus::FinalChainEvmLog {
                        address: log.address,
                        topics: log
                            .topics
                            .into_iter()
                            .map(|topic| rustaxa_consensus::FinalChainEvmLogTopic {
                                topic: topic.topic,
                            })
                            .collect(),
                        data: log.data,
                    })
                    .collect(),
                new_contract_address: if result.new_contract_address_found {
                    Some(result.new_contract_address)
                } else {
                    None
                },
                code_error: result.code_error,
                consensus_error: result.consensus_error,
            })
            .collect(),
    }
}

fn evm_rewards_report_from_ffi(
    report: rustaxa_ffi::FinalChainEvmRewardsReport,
) -> rustaxa_consensus::FinalChainEvmRewardsReport {
    rustaxa_consensus::FinalChainEvmRewardsReport {
        request_id: report.request_id,
        period: report.period,
        status: report.status,
        state_root: report.state_root,
        total_reward: report.total_reward,
    }
}

fn external_evm_commit_plan_to_ffi(
    plan: rustaxa_consensus::FinalChainExternalEvmCommitPlan,
) -> rustaxa_ffi::FinalChainExternalEvmCommitPlan {
    rustaxa_ffi::FinalChainExternalEvmCommitPlan {
        request_id: plan.request_id,
        period: plan.period,
        post_execution_state_root: plan.post_execution_state_root,
        state_root: plan.state_root,
        total_reward: plan.total_reward,
        transactions_root: plan.transactions_root,
        receipts_root: plan.receipts_root,
        header_log_bloom: plan.header_log_bloom,
        indexed_log_bloom: plan.indexed_log_bloom,
        receipts_rlp: plan.receipts_rlp,
        encoded_receipts: plan
            .encoded_receipts
            .into_iter()
            .map(|data| rustaxa_ffi::ReceiptRlp { data })
            .collect(),
        gas_used: plan.gas_used,
        executed_dag_blocks: plan.executed_dag_blocks,
        executed_transactions: plan.executed_transactions,
        regular_transaction_count: plan.regular_transaction_count,
        system_transaction_count: plan.system_transaction_count,
        error_code: plan.error_code,
    }
}

fn external_evm_publication_plan_to_ffi(
    plan: rustaxa_consensus::FinalChainExternalEvmPublicationPlan,
) -> rustaxa_ffi::FinalChainExternalEvmPublicationPlan {
    rustaxa_ffi::FinalChainExternalEvmPublicationPlan {
        request_id: plan.request_id,
        plan_id: plan.plan_id,
        period: plan.period,
        block_hash: plan.block_hash,
        block_header_rlp: plan.block_header_rlp,
        stored_header_rlp: plan.stored_header_rlp,
        receipts_rlp: plan.receipts_rlp,
        indexed_log_bloom: plan.indexed_log_bloom,
        system_transaction_hashes_rlp: plan.system_transaction_hashes_rlp,
        transaction_publications: plan
            .transaction_publications
            .into_iter()
            .map(
                |publication| rustaxa_ffi::FinalChainExternalEvmTransactionPublication {
                    transaction_hash: publication.transaction_hash,
                    position: publication.position,
                    is_system: publication.is_system,
                    receipt_rlp: publication.receipt_rlp,
                },
            )
            .collect(),
        executed_dag_blocks: plan.executed_dag_blocks,
        executed_transactions: plan.executed_transactions,
        error_code: plan.error_code,
    }
}

fn external_evm_lifecycle_report_from_ffi(
    report: rustaxa_ffi::FinalChainExternalEvmLifecycleReport,
) -> rustaxa_consensus::FinalChainExternalEvmLifecycleReport {
    rustaxa_consensus::FinalChainExternalEvmLifecycleReport {
        request_id: report.request_id,
        plan_id: report.plan_id,
        period: report.period,
        post_execution_state_root: report.post_execution_state_root,
        post_rewards_state_root: report.post_rewards_state_root,
        publication_block_hash: report.publication_block_hash,
        status: report.status,
        error_code: report.error_code,
    }
}

fn external_evm_commit_decision_to_ffi(
    decision: rustaxa_consensus::FinalChainExternalEvmCommitDecision,
) -> rustaxa_ffi::FinalChainExternalEvmCommitDecision {
    rustaxa_ffi::FinalChainExternalEvmCommitDecision {
        request_id: decision.request_id,
        plan_id: decision.plan_id,
        period: decision.period,
        publication_block_hash: decision.publication_block_hash,
        status: decision.status,
        error_code: decision.error_code,
    }
}

fn commit_report_to_ffi(
    report: rustaxa_consensus::FinalChainExecutionCommitReport,
) -> rustaxa_ffi::FinalChainExecutionCommitReport {
    rustaxa_ffi::FinalChainExecutionCommitReport {
        status: report.status,
        period: report.period,
        block_header_rlp: report.block_header_rlp,
        receipts: report
            .receipts
            .into_iter()
            .map(|data| rustaxa_ffi::ReceiptRlp { data })
            .collect(),
        gas_used: report.gas_used,
        executed_dag_blocks: report.executed_dag_blocks,
        executed_transactions: report.executed_transactions,
        error_code: report.error_code,
    }
}

pub fn create_final_chain(
    storage: &BridgeStorage,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    genesis_accounts: Vec<rustaxa_ffi::GenesisAccount>,
    genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
    genesis_dpos_config: rustaxa_ffi::GenesisDposConfig,
) -> Result<Box<BridgeFinalChain>, anyhow::Error> {
    create_final_chain_with_rewards_config(
        storage,
        block_gas_limit,
        genesis_timestamp,
        genesis_accounts,
        genesis_validators,
        genesis_dpos_config,
        rustaxa_ffi::FinalChainRewardsConfig {
            committee_size: 0,
            magnolia_period: 0,
            aspen_part_one_period: u64::MAX,
            fix_claim_all_block_num: u64::MAX,
            aspen_part_two_period: 0,
            max_block_author_reward_percent: 0,
            dag_proposers_reward_percent: 0,
            yield_percentage: 0,
            dpos_blocks_per_year: 0,
            dpos_delegation_locking_period: 0,
            cornus_period: 0,
            cornus_delegation_locking_period: 0,
            genesis_balance_sum: Vec::new(),
            aspen_max_supply: Vec::new(),
            aspen_generated_rewards: Vec::new(),
            cacti_period: 0,
            cacti_delegation_locking_period: 0,
            magnolia_jail_time: 0,
            cacti_jail_time: 0,
            frequency_rules: Vec::new(),
        },
    )
}

pub fn create_final_chain_with_rewards_config(
    storage: &BridgeStorage,
    block_gas_limit: u64,
    genesis_timestamp: u64,
    genesis_accounts: Vec<rustaxa_ffi::GenesisAccount>,
    genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
    genesis_dpos_config: rustaxa_ffi::GenesisDposConfig,
    rewards_config: rustaxa_ffi::FinalChainRewardsConfig,
) -> Result<Box<BridgeFinalChain>, anyhow::Error> {
    let genesis_accounts = genesis_accounts
        .into_iter()
        .map(|account| rustaxa_consensus::GenesisAccount {
            address: account.address,
            balance: account.balance,
        })
        .collect();
    let genesis_validators = genesis_validators
        .into_iter()
        .map(|validator| {
            let rustaxa_ffi::GenesisValidator {
                address,
                owner,
                vrf_key,
                commission,
                description,
                endpoint,
                total_stake,
                delegations,
            } = validator;
            rustaxa_consensus::GenesisValidator {
                address,
                vrf_key,
                total_stake,
                delegations: delegations
                    .into_iter()
                    .map(|delegation| (delegation.delegator, delegation.stake))
                    .collect(),
                metadata: rustaxa_consensus::GenesisValidatorMetadata {
                    owner,
                    commission,
                    description,
                    endpoint,
                },
            }
        })
        .collect();
    let final_chain = FinalChain::new_with_rewards_config(
        storage.0.clone(),
        block_gas_limit,
        genesis_timestamp,
        genesis_accounts,
        genesis_validators,
        rustaxa_consensus::GenesisDposConfig {
            eligibility_balance_threshold: genesis_dpos_config.eligibility_balance_threshold,
            vote_eligibility_balance_step: genesis_dpos_config.vote_eligibility_balance_step,
            validator_maximum_stake: genesis_dpos_config.validator_maximum_stake,
            minimum_deposit: genesis_dpos_config.minimum_deposit,
            commission_change_delta: genesis_dpos_config.commission_change_delta,
            commission_change_frequency: genesis_dpos_config.commission_change_frequency,
            delegation_delay: genesis_dpos_config.delegation_delay,
            dag_vdf_sortition_total_vote_count_until_period: genesis_dpos_config
                .dag_vdf_sortition_total_vote_count_until_period,
        },
        rustaxa_consensus::FinalChainRewardsConfig {
            committee_size: rewards_config.committee_size,
            magnolia_period: rewards_config.magnolia_period,
            aspen_part_one_period: rewards_config.aspen_part_one_period,
            fix_claim_all_block_num: rewards_config.fix_claim_all_block_num,
            aspen_part_two_period: rewards_config.aspen_part_two_period,
            max_block_author_reward_percent: rewards_config.max_block_author_reward_percent,
            dag_proposers_reward_percent: rewards_config.dag_proposers_reward_percent,
            yield_percentage: rewards_config.yield_percentage,
            dpos_blocks_per_year: rewards_config.dpos_blocks_per_year,
            dpos_delegation_locking_period: rewards_config.dpos_delegation_locking_period,
            cornus_period: rewards_config.cornus_period,
            cornus_delegation_locking_period: rewards_config.cornus_delegation_locking_period,
            genesis_balance_sum: rewards_config.genesis_balance_sum,
            aspen_max_supply: rewards_config.aspen_max_supply,
            aspen_generated_rewards: rewards_config.aspen_generated_rewards,
            cacti_period: rewards_config.cacti_period,
            cacti_delegation_locking_period: rewards_config.cacti_delegation_locking_period,
            magnolia_jail_time: rewards_config.magnolia_jail_time,
            cacti_jail_time: rewards_config.cacti_jail_time,
            rewards_distribution_frequency: rewards_config
                .frequency_rules
                .into_iter()
                .map(|rule| (rule.from_period, rule.frequency))
                .collect(),
        },
    )?;
    Ok(Box::new(BridgeFinalChain(final_chain)))
}

pub fn create_final_chain_execution_session(
    final_chain: &BridgeFinalChain,
    request: rustaxa_ffi::FinalChainExecutionRequest,
) -> Result<Box<BridgeFinalChainExecutionSession>, anyhow::Error> {
    let _ = final_chain;
    Ok(Box::new(BridgeFinalChainExecutionSession {
        state: rustaxa_consensus::create_final_chain_execution_session(
            final_chain_execution_request_from_ffi(request),
        ),
    }))
}

pub fn final_chain_execution_session_commit(
    final_chain: &BridgeFinalChain,
    session: Box<BridgeFinalChainExecutionSession>,
) -> Result<rustaxa_ffi::FinalChainExecutionCommitReport, anyhow::Error> {
    rustaxa_consensus::commit_final_chain_execution_session(&final_chain.0, session.state)
        .map(commit_report_to_ffi)
}

pub fn abort_final_chain_execution_session(session: Box<BridgeFinalChainExecutionSession>) {
    let _ = rustaxa_consensus::abort_final_chain_execution_session(session.state);
}

impl BridgeFinalChainExecutionSession {
    pub fn final_chain_execution_session_next(
        &mut self,
    ) -> Result<rustaxa_ffi::FinalChainExecutionStep, anyhow::Error> {
        Ok(execution_step_to_ffi(
            rustaxa_consensus::final_chain_execution_session_next(&mut self.state),
        ))
    }

    pub fn final_chain_execution_session_report_evm(
        &mut self,
        report: rustaxa_ffi::FinalChainEvmExecutionReport,
    ) -> Result<rustaxa_ffi::FinalChainExecutionStep, anyhow::Error> {
        Ok(execution_step_to_ffi(
            rustaxa_consensus::final_chain_execution_session_report_evm(
                &mut self.state,
                evm_report_from_ffi(report),
            ),
        ))
    }

    pub fn final_chain_execution_session_report_system_transactions(
        &mut self,
        report: rustaxa_ffi::FinalChainSystemTransactionReport,
    ) -> Result<rustaxa_ffi::FinalChainExecutionStep, anyhow::Error> {
        Ok(execution_step_to_ffi(
            rustaxa_consensus::final_chain_execution_session_report_system_transactions(
                &mut self.state,
                system_transaction_report_from_ffi(report),
            ),
        ))
    }

    pub fn final_chain_execution_session_plan_external_evm_commit(
        &mut self,
        rewards_report: rustaxa_ffi::FinalChainEvmRewardsReport,
    ) -> Result<rustaxa_ffi::FinalChainExternalEvmCommitPlan, anyhow::Error> {
        Ok(external_evm_commit_plan_to_ffi(
            rustaxa_consensus::final_chain_execution_session_plan_external_evm_commit(
                &mut self.state,
                evm_rewards_report_from_ffi(rewards_report),
            ),
        ))
    }

    pub fn final_chain_execution_session_report_external_evm_lifecycle(
        &mut self,
        report: rustaxa_ffi::FinalChainExternalEvmLifecycleReport,
    ) -> Result<rustaxa_ffi::FinalChainExternalEvmCommitDecision, anyhow::Error> {
        Ok(external_evm_commit_decision_to_ffi(
            rustaxa_consensus::final_chain_execution_session_report_external_evm_lifecycle(
                &mut self.state,
                external_evm_lifecycle_report_from_ffi(report),
            ),
        ))
    }
}

pub fn final_chain_execution_session_plan_external_evm_publication(
    final_chain: &BridgeFinalChain,
    session: &mut BridgeFinalChainExecutionSession,
) -> Result<rustaxa_ffi::FinalChainExternalEvmPublicationPlan, anyhow::Error> {
    Ok(external_evm_publication_plan_to_ffi(
        rustaxa_consensus::final_chain_execution_session_plan_external_evm_publication(
            &final_chain.0,
            &mut session.state,
        ),
    ))
}

impl BridgeFinalChain {
    pub fn get_last_block_number(self: &BridgeFinalChain) -> Result<u64, anyhow::Error> {
        self.0.last_block_number()
    }

    pub fn get_block_number(
        self: &BridgeFinalChain,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::FinalChainBlockNumberLookup, anyhow::Error> {
        Ok(match self.0.block_number(*hash)? {
            Some(value) => rustaxa_ffi::FinalChainBlockNumberLookup { found: true, value },
            None => rustaxa_ffi::FinalChainBlockNumberLookup {
                found: false,
                value: 0,
            },
        })
    }

    pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .block_hash(num)
            .map_err(|e| anyhow::anyhow!(e))?
            .unwrap_or_default())
    }

    pub fn get_block_header(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.block_header(num)?.unwrap_or_default())
    }

    pub fn get_transaction_location(
        self: &BridgeFinalChain,
        hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.transaction_location(*hash)?.unwrap_or_default())
    }

    pub fn get_transaction_count(
        self: &BridgeFinalChain,
        period: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0.transaction_count(period)
    }

    /// Returns finalized block numbers whose Rust FinalChain bloom index
    /// contains the supplied query bloom over the inclusive block range.
    pub fn get_blocks_with_bloom(
        self: &BridgeFinalChain,
        bloom: &[u8; 256],
        from: u64,
        to: u64,
    ) -> Result<Vec<u64>, anyhow::Error> {
        self.0.with_block_bloom(bloom, from, to)
    }

    pub fn get_account(
        self: &BridgeFinalChain,
        address: &[u8; 20],
    ) -> Result<rustaxa_ffi::AccountLookup, anyhow::Error> {
        Ok(account_to_lookup(self.0.account(*address)?))
    }

    pub fn get_account_at_block(
        self: &BridgeFinalChain,
        block_number: u64,
        address: &[u8; 20],
    ) -> Result<rustaxa_ffi::AccountLookup, anyhow::Error> {
        Ok(account_to_lookup(
            self.0.account_at_block(block_number, *address)?,
        ))
    }

    pub fn get_vrf_key(
        self: &BridgeFinalChain,
        address: &[u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .vrf_key(*address)?
            .map(|key| key.to_vec())
            .unwrap_or_default())
    }

    /// Returns a block-scoped validator VRF key for the C++ FinalChain shim.
    ///
    /// Inputs are a finalized block number and validator address. The output is
    /// the raw 32-byte VRF key, or an empty vector when the block snapshot
    /// exists but does not contain the validator. Missing snapshots propagate as
    /// errors so Rust mode does not fall back to genesis or latest state.
    pub fn get_vrf_key_at_block(
        self: &BridgeFinalChain,
        block_number: u64,
        address: &[u8; 20],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .vrf_key_at_block(block_number, *address)?
            .map(|key| key.to_vec())
            .unwrap_or_default())
    }

    pub fn get_dpos_eligible_vote_count(
        self: &BridgeFinalChain,
        block_number: u64,
        address: &[u8; 20],
    ) -> Result<u64, anyhow::Error> {
        self.0.dpos_eligible_vote_count(block_number, *address)
    }

    pub fn get_dpos_eligible_total_vote_count(
        self: &BridgeFinalChain,
        block_number: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0.dpos_eligible_total_vote_count(block_number)
    }

    pub fn get_dpos_is_eligible(
        self: &BridgeFinalChain,
        block_number: u64,
        address: &[u8; 20],
    ) -> Result<bool, anyhow::Error> {
        self.0.dpos_is_eligible(block_number, *address)
    }

    /// Returns DagManager authorization facts for staged VDF/DPoS checks.
    ///
    /// Missing DPoS snapshots are surfaced as
    /// `DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE` in the returned payload,
    /// while other FinalChain errors remain hard failures.
    pub fn get_dag_dpos_authorization_facts(
        self: &BridgeFinalChain,
        block_number: u64,
        sender: &[u8; 20],
    ) -> Result<rustaxa_ffi::DagDposAuthorizationFacts, anyhow::Error> {
        let facts = self.0.dag_dpos_authorization_facts(block_number, *sender)?;
        Ok(rustaxa_ffi::DagDposAuthorizationFacts {
            vrf_key_found: facts.vrf_key_found,
            vrf_key: facts
                .vrf_key
                .map(|vrf_key| vrf_key.to_vec())
                .unwrap_or_default(),
            sender_eligible_vote_count: facts.sender_eligible_vote_count,
            vdf_sortition_max_vote_count: facts.vdf_sortition_max_vote_count,
            eligibility_status: facts.eligibility_status,
        })
    }

    pub fn get_dpos_validators_total_stakes(
        self: &BridgeFinalChain,
        block_number: u64,
    ) -> Result<Vec<rustaxa_ffi::DposValidatorStake>, anyhow::Error> {
        Ok(self
            .0
            .dpos_validators_total_stakes(block_number)?
            .into_iter()
            .map(|stake| rustaxa_ffi::DposValidatorStake {
                address: stake.address,
                stake: stake.stake,
            })
            .collect())
    }

    pub fn get_dpos_total_amount_delegated(
        self: &BridgeFinalChain,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        self.0.dpos_total_amount_delegated(block_number)
    }

    pub fn get_dpos_yield(
        self: &BridgeFinalChain,
        block_number: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0.dpos_yield(block_number)
    }

    pub fn get_dpos_total_supply(
        self: &BridgeFinalChain,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        self.0.dpos_total_supply(block_number)
    }

    pub fn get_dpos_validators_eligible_vote_counts(
        self: &BridgeFinalChain,
        block_number: u64,
    ) -> Result<Vec<rustaxa_ffi::DposValidatorVoteCount>, anyhow::Error> {
        Ok(self
            .0
            .dpos_validators_eligible_vote_counts(block_number)?
            .into_iter()
            .map(|vote_count| rustaxa_ffi::DposValidatorVoteCount {
                address: vote_count.address,
                vote_count: vote_count.vote_count,
            })
            .collect())
    }

    pub fn estimate_call_gas(
        self: &BridgeFinalChain,
        gas_limit: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0.estimate_call_gas(gas_limit)
    }

    pub fn call(
        self: &BridgeFinalChain,
        request: rustaxa_ffi::FinalChainCall,
    ) -> Result<rustaxa_ffi::FinalChainCallOutcome, anyhow::Error> {
        let outcome = self.0.call(rustaxa_consensus::FinalChainCallRequest {
            block_number: request.block_number,
            sender: request.sender,
            receiver: if request.receiver_found {
                Some(request.receiver)
            } else {
                None
            },
            value: request.value,
            gas_price: request.gas_price,
            gas_limit: request.gas_limit,
            input: request.input,
        })?;
        Ok(rustaxa_ffi::FinalChainCallOutcome {
            code_retval: outcome.code_retval,
            gas_used: outcome.gas_used,
            code_err: outcome.code_err,
            consensus_err: outcome.consensus_err,
        })
    }

    pub fn finalize_block(
        self: &BridgeFinalChain,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<rustaxa_ffi::FinalizationTransaction>,
        finalized_dag_blocks: Vec<rustaxa_ffi::FinalizationDagBlock>,
    ) -> Result<rustaxa_ffi::FinalizationOutcome, anyhow::Error> {
        self.finalize_block_with_rewards_context(
            pbft_block_rlp,
            transactions,
            finalized_dag_blocks,
            0,
        )
    }

    pub fn finalize_block_with_rewards_context(
        self: &BridgeFinalChain,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<rustaxa_ffi::FinalizationTransaction>,
        finalized_dag_blocks: Vec<rustaxa_ffi::FinalizationDagBlock>,
        blocks_per_year: u32,
    ) -> Result<rustaxa_ffi::FinalizationOutcome, anyhow::Error> {
        self.finalize_block_with_rewards_facts(
            pbft_block_rlp,
            transactions,
            finalized_dag_blocks,
            blocks_per_year,
            Vec::new(),
        )
    }

    pub fn finalize_block_with_rewards_facts(
        self: &BridgeFinalChain,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<rustaxa_ffi::FinalizationTransaction>,
        finalized_dag_blocks: Vec<rustaxa_ffi::FinalizationDagBlock>,
        blocks_per_year: u32,
        cert_votes: Vec<rustaxa_ffi::RewardsCertVoteFact>,
    ) -> Result<rustaxa_ffi::FinalizationOutcome, anyhow::Error> {
        let request = final_chain_execution_request_from_compat(
            pbft_block_rlp,
            transactions,
            finalized_dag_blocks,
            blocks_per_year,
            cert_votes,
        );
        let mut session = BridgeFinalChainExecutionSession {
            state: rustaxa_consensus::create_final_chain_execution_session(request),
        };
        let step = session.final_chain_execution_session_next()?;
        if step.action != FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE {
            anyhow::bail!(
                "Rust FinalChain execution runtime rejected finalize request: {}",
                step.error_code
            );
        }
        let report = final_chain_execution_session_commit(self, Box::new(session))?;
        if !report.error_code.is_empty() {
            anyhow::bail!(
                "Rust FinalChain execution runtime failed finalize request: {}",
                report.error_code
            );
        }
        Ok(rustaxa_ffi::FinalizationOutcome {
            block_header_rlp: report.block_header_rlp,
            receipts: report.receipts,
        })
    }

    pub fn get_transaction_rlps(
        self: &BridgeFinalChain,
        period: u64,
    ) -> Result<Vec<rustaxa_ffi::TxRlp>, anyhow::Error> {
        Ok(self
            .0
            .transaction_rlps(period)?
            .into_iter()
            .map(|data| rustaxa_ffi::TxRlp { data })
            .collect())
    }

    pub fn get_transaction_receipt(
        self: &BridgeFinalChain,
        period: u64,
        position: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction_receipt_rlp(period, position)?
            .unwrap_or_default())
    }

    /// Collects FinalChain facts needed by PBFT manager proposal, validation,
    /// and wallet-eligibility decisions.
    ///
    /// The input uses PBFT period numbering and address order from C++. The
    /// output preserves that order for address facts. Missing delayed
    /// FinalChain headers or DPoS snapshots are returned as explicit status
    /// data, while malformed storage and bridge infrastructure failures still
    /// propagate as errors.
    pub fn collect_pbft_final_chain_facts(
        self: &BridgeFinalChain,
        request: rustaxa_ffi::PbftFinalChainFactRequest,
    ) -> Result<rustaxa_ffi::PbftFinalChainFacts, anyhow::Error> {
        let last_block_number = self.0.last_block_number()?;
        let final_chain_hash =
            if request.collect_final_chain_hash || request.validate_candidate_final_chain_hash {
                let expected_hash = self.0.pbft_final_chain_hash(request.period)?;
                match expected_hash {
                    Some(expected_hash)
                        if request.validate_candidate_final_chain_hash
                            && expected_hash != request.candidate_final_chain_hash =>
                    {
                        pbft_final_chain_hash_result(
                            PBFT_FINAL_CHAIN_FACT_STATUS_INVALID,
                            expected_hash,
                            request.candidate_final_chain_hash,
                            "PBFT_FINAL_CHAIN_HASH_MISMATCH",
                        )
                    }
                    Some(expected_hash) => pbft_final_chain_hash_result(
                        PBFT_FINAL_CHAIN_FACT_STATUS_READY,
                        expected_hash,
                        request.candidate_final_chain_hash,
                        "",
                    ),
                    None => pbft_final_chain_hash_result(
                        PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE,
                        [0; 32],
                        request.candidate_final_chain_hash,
                        "PBFT_FINAL_CHAIN_HASH_MISSING",
                    ),
                }
            } else {
                pbft_final_chain_hash_result(
                    PBFT_FINAL_CHAIN_FACT_STATUS_READY,
                    [0; 32],
                    request.candidate_final_chain_hash,
                    "",
                )
            };

        let (total_vote_count_status, has_total_vote_count, total_vote_count, total_error) =
            if request.collect_total_vote_count {
                match self.0.dpos_eligible_total_vote_count(request.period) {
                    Ok(value) => (
                        PBFT_FINAL_CHAIN_FACT_STATUS_READY,
                        true,
                        value,
                        String::new(),
                    ),
                    Err(err) => (
                        PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE,
                        false,
                        0,
                        format!("PBFT_FINAL_CHAIN_TOTAL_VOTES_UNAVAILABLE: {err}"),
                    ),
                }
            } else {
                (PBFT_FINAL_CHAIN_FACT_STATUS_READY, false, 0, String::new())
            };

        let mut address_facts = Vec::new();
        if request.collect_address_vote_counts {
            address_facts.reserve(request.addresses.len());
            for address in request.addresses {
                match self
                    .0
                    .dpos_eligible_vote_count(request.period, address.address)
                {
                    Ok(vote_count) => {
                        address_facts.push(rustaxa_ffi::PbftFinalChainAddressFact {
                            address: address.address,
                            status: PBFT_FINAL_CHAIN_FACT_STATUS_READY,
                            eligible: vote_count > 0,
                            vote_count,
                            error_code: String::new(),
                        });
                    }
                    Err(err) => {
                        address_facts.push(rustaxa_ffi::PbftFinalChainAddressFact {
                            address: address.address,
                            status: PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE,
                            eligible: false,
                            vote_count: 0,
                            error_code: format!("PBFT_FINAL_CHAIN_ADDRESS_FACT_UNAVAILABLE: {err}"),
                        });
                    }
                }
            }
        }

        let address_facts_ready = address_facts
            .iter()
            .all(|fact| fact.status == PBFT_FINAL_CHAIN_FACT_STATUS_READY);
        let all_ready = final_chain_hash.status == PBFT_FINAL_CHAIN_FACT_STATUS_READY
            && total_vote_count_status == PBFT_FINAL_CHAIN_FACT_STATUS_READY
            && address_facts_ready;
        let status = if all_ready {
            PBFT_FINAL_CHAIN_FACT_STATUS_READY
        } else if final_chain_hash.status == PBFT_FINAL_CHAIN_FACT_STATUS_INVALID {
            PBFT_FINAL_CHAIN_FACT_STATUS_INVALID
        } else {
            PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE
        };
        let error_code = if status == PBFT_FINAL_CHAIN_FACT_STATUS_READY {
            String::new()
        } else if final_chain_hash.status != PBFT_FINAL_CHAIN_FACT_STATUS_READY {
            final_chain_hash.error_code.clone()
        } else if !total_error.is_empty() {
            total_error.clone()
        } else {
            "PBFT_FINAL_CHAIN_ADDRESS_FACTS_UNAVAILABLE".to_string()
        };

        Ok(rustaxa_ffi::PbftFinalChainFacts {
            status,
            last_block_number,
            final_chain_hash,
            total_vote_count_status,
            has_total_vote_count,
            total_vote_count,
            address_facts,
            error_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
    use ethereum_types::H256;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_consensus::dag;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn u256_be(value: u64) -> Vec<u8> {
        let bytes = value.to_be_bytes();
        let first_nonzero = bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len());
        bytes[first_nonzero..].to_vec()
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after UNIX_EPOCH")
            .as_nanos();
        let process_id = std::process::id();
        std::env::temp_dir().join(format!("{prefix}_{process_id}_{now_ns}"))
    }

    fn genesis_validator(address: [u8; 20], stake: u64) -> rustaxa_ffi::GenesisValidator {
        rustaxa_ffi::GenesisValidator {
            address,
            owner: [0u8; 20],
            vrf_key: [address[0]; 32],
            commission: 0,
            description: "".to_string(),
            endpoint: "".to_string(),
            total_stake: u256_be(stake),
            delegations: vec![rustaxa_ffi::GenesisDelegation {
                delegator: address,
                stake: u256_be(stake),
            }],
        }
    }

    fn make_final_chain(
        storage_path: &str,
        genesis_validators: Vec<rustaxa_ffi::GenesisValidator>,
    ) -> Box<BridgeFinalChain> {
        let storage = create_storage(storage_path).expect("storage should initialize");
        create_final_chain(
            &storage,
            0,
            0,
            vec![],
            genesis_validators,
            rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: u256_be(1_000),
                vote_eligibility_balance_step: u256_be(1_000),
                validator_maximum_stake: u256_be(30_000),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize")
    }

    fn ffi_transaction(
        hash_byte: u8,
        receiver_found: bool,
        receiver: [u8; 20],
        data: Vec<u8>,
    ) -> rustaxa_ffi::FinalizationTransaction {
        rustaxa_ffi::FinalizationTransaction {
            hash: [hash_byte; 32],
            sender: [1; 20],
            receiver_found,
            receiver,
            nonce: 0,
            value: vec![0],
            gas_price: vec![0],
            gas_limit: 21_000,
            data,
            rlp: vec![hash_byte],
        }
    }

    fn ffi_evm_result(
        transaction: &rustaxa_ffi::FinalChainEvmTransactionInput,
        gas_used: u64,
        cumulative_gas_used: u64,
    ) -> rustaxa_ffi::FinalChainEvmTransactionResult {
        let mut result = rustaxa_ffi::FinalChainEvmTransactionResult {
            position: transaction.position,
            hash: transaction.hash,
            status: 1,
            gas_used,
            cumulative_gas_used,
            receipt_rlp: Vec::new(),
            logs: vec![rustaxa_ffi::FinalChainEvmLog {
                address: [0x44; 20],
                topics: vec![rustaxa_ffi::FinalChainEvmLogTopic { topic: [0x55; 32] }],
                data: vec![0x66],
            }],
            new_contract_address_found: true,
            new_contract_address: [0x77; 20],
            code_error: String::new(),
            consensus_error: String::new(),
        };
        result.receipt_rlp = ffi_evm_receipt_rlp(&result);
        result
    }

    fn ffi_evm_receipt_rlp(result: &rustaxa_ffi::FinalChainEvmTransactionResult) -> Vec<u8> {
        let mut stream = RlpStream::new_list(5);
        stream.append(&result.status);
        stream.append(&result.gas_used);
        stream.append(&result.cumulative_gas_used);
        stream.begin_list(result.logs.len());
        for log in &result.logs {
            stream.begin_list(3);
            stream.append(&log.address.as_slice());
            stream.begin_list(log.topics.len());
            for topic in &log.topics {
                stream.append(&topic.topic.as_slice());
            }
            stream.append(&log.data.as_slice());
        }
        if result.new_contract_address_found {
            stream.append(&result.new_contract_address.as_slice());
        } else {
            stream.append(&0u8);
        }
        stream.out().to_vec()
    }

    fn external_evm_publication_fixture(
        prefix: &str,
    ) -> (
        PathBuf,
        Box<BridgeFinalChain>,
        Box<BridgeFinalChainExecutionSession>,
        rustaxa_ffi::FinalChainExternalEvmCommitPlan,
        rustaxa_ffi::FinalChainExternalEvmPublicationPlan,
    ) {
        let temp_dir = unique_temp_dir(prefix);
        let storage_path = temp_dir.to_str().expect("temp path should be utf-8");
        let final_chain = make_final_chain(storage_path, vec![]);
        let mut session = create_final_chain_execution_session(
            &final_chain,
            rustaxa_ffi::FinalChainExecutionRequest {
                pbft_block_rlp: signed_pbft_block_rlp(7),
                transactions: vec![
                    ffi_transaction(1, true, [9; 20], Vec::new()),
                    ffi_transaction(2, true, [8; 20], vec![0xaa]),
                ],
                finalized_dag_blocks: Vec::new(),
                blocks_per_year: 0,
                cert_votes: Vec::new(),
                block_gas_limit: 1_000_000,
                mode: rustaxa_consensus::FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
            },
        )
        .expect("session should be created");
        let system_step = session
            .final_chain_execution_session_next()
            .expect("session step should convert");
        let step = session
            .final_chain_execution_session_report_system_transactions(
                rustaxa_ffi::FinalChainSystemTransactionReport {
                    request_id: system_step.system_transaction_request.request_id,
                    period: 7,
                    transactions: Vec::new(),
                },
            )
            .expect("system transaction report should convert");
        let rewards = session
            .final_chain_execution_session_report_evm(rustaxa_ffi::FinalChainEvmExecutionReport {
                request_id: step.evm_request.request_id,
                status: rustaxa_consensus::FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
                state_root: [0x11; 32],
                cumulative_gas_used: 2,
                results: vec![
                    ffi_evm_result(&step.evm_request.transactions[0], 1, 1),
                    ffi_evm_result(&step.evm_request.transactions[1], 1, 2),
                ],
            })
            .expect("typed report should convert");
        assert_eq!(
            rewards.action,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS
        );
        let plan = session
            .final_chain_execution_session_plan_external_evm_commit(
                rustaxa_ffi::FinalChainEvmRewardsReport {
                    request_id: step.evm_request.request_id,
                    period: 7,
                    status: rustaxa_consensus::FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
                    state_root: [0x22; 32],
                    total_reward: vec![0x33],
                },
            )
            .expect("commit plan should convert");
        let publication =
            final_chain_execution_session_plan_external_evm_publication(&final_chain, &mut session)
                .expect("publication plan should convert");
        (temp_dir, final_chain, session, plan, publication)
    }

    fn signed_pbft_block_rlp(period: u64) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let timestamp = 1234u64;
        let mut unsigned_stream = RlpStream::new_list(7);
        append_pbft_block_fields(&mut unsigned_stream, period, timestamp);
        let message_hash = keccak256(&unsigned_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed_stream = RlpStream::new_list(8);
        append_pbft_block_fields(&mut signed_stream, period, timestamp);
        signed_stream.append(&signature_bytes);
        signed_stream.out().to_vec()
    }

    fn append_pbft_block_fields(stream: &mut RlpStream, period: u64, timestamp: u64) {
        stream.append(&H256::from_low_u64_be(10));
        stream.append(&H256::from_low_u64_be(11));
        stream.append(&H256::from_low_u64_be(12));
        stream.append(&H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&timestamp);
        stream.begin_list(0);
    }

    fn keccak256(data: &[u8]) -> H256 {
        use tiny_keccak::{Hasher, Keccak};

        let mut hasher = Keccak::v256();
        hasher.update(data);
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        H256::from(output)
    }

    #[test]
    fn bridge_get_dpos_authorization_facts_prefers_snapshot_status() {
        let validator = [0xA1u8; 20];
        let ineligible = [0xA2u8; 20];
        let temp_dir = unique_temp_dir("rustaxa_bridge_final_chain_authorization_facts");
        let storage_path = temp_dir.to_str().expect("temp path should be utf-8");
        let final_chain = make_final_chain(
            storage_path,
            vec![
                genesis_validator(validator, 10_000),
                genesis_validator(ineligible, 999),
            ],
        );
        let eligible = final_chain
            .get_dag_dpos_authorization_facts(0, &validator)
            .expect("eligible facts should be available");
        assert!(eligible.vrf_key_found);
        assert_eq!(eligible.vrf_key, vec![0xA1; 32]);
        assert_eq!(eligible.sender_eligible_vote_count, 10);
        assert_eq!(eligible.vdf_sortition_max_vote_count, 30);
        assert_eq!(
            eligible.eligibility_status,
            dag::DAG_VERIFY_DPOS_STATUS_ELIGIBLE
        );

        let missing_snapshot = final_chain
            .get_dag_dpos_authorization_facts(1, &validator)
            .expect("snapshot should return unavailable status as data");
        assert!(missing_snapshot.vrf_key_found);
        assert_eq!(missing_snapshot.sender_eligible_vote_count, 0);
        assert_eq!(missing_snapshot.vdf_sortition_max_vote_count, 0);
        assert_eq!(
            missing_snapshot.eligibility_status,
            dag::DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE
        );

        drop(final_chain);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_collects_pbft_final_chain_facts_from_rust_runtime() {
        let validator = [0xB1u8; 20];
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_final_chain_facts");
        let storage_path = temp_dir.to_str().expect("temp path should be utf-8");
        let final_chain =
            make_final_chain(storage_path, vec![genesis_validator(validator, 10_000)]);

        let ready = final_chain
            .collect_pbft_final_chain_facts(rustaxa_ffi::PbftFinalChainFactRequest {
                period: 0,
                candidate_final_chain_hash: [0; 32],
                collect_final_chain_hash: true,
                validate_candidate_final_chain_hash: true,
                collect_total_vote_count: true,
                collect_address_vote_counts: true,
                addresses: vec![rustaxa_ffi::PbftFinalChainFactAddress { address: validator }],
            })
            .expect("ready PBFT facts should not throw");
        assert_eq!(ready.status, PBFT_FINAL_CHAIN_FACT_STATUS_READY);
        assert_eq!(ready.last_block_number, 0);
        assert_eq!(
            ready.final_chain_hash.status,
            PBFT_FINAL_CHAIN_FACT_STATUS_READY
        );
        assert_eq!(ready.final_chain_hash.expected_hash, [0; 32]);
        assert!(ready.has_total_vote_count);
        assert_eq!(ready.total_vote_count, 10);
        assert_eq!(ready.address_facts.len(), 1);
        assert_eq!(
            ready.address_facts[0].status,
            PBFT_FINAL_CHAIN_FACT_STATUS_READY
        );
        assert!(ready.address_facts[0].eligible);
        assert_eq!(ready.address_facts[0].vote_count, 10);

        let mismatch = final_chain
            .collect_pbft_final_chain_facts(rustaxa_ffi::PbftFinalChainFactRequest {
                period: 0,
                candidate_final_chain_hash: [0xCC; 32],
                collect_final_chain_hash: true,
                validate_candidate_final_chain_hash: true,
                collect_total_vote_count: false,
                collect_address_vote_counts: false,
                addresses: vec![],
            })
            .expect("mismatch should be returned as data");
        assert_eq!(mismatch.status, PBFT_FINAL_CHAIN_FACT_STATUS_INVALID);
        assert_eq!(
            mismatch.final_chain_hash.status,
            PBFT_FINAL_CHAIN_FACT_STATUS_INVALID
        );
        assert_eq!(
            mismatch.final_chain_hash.error_code,
            "PBFT_FINAL_CHAIN_HASH_MISMATCH"
        );

        let unavailable = final_chain
            .collect_pbft_final_chain_facts(rustaxa_ffi::PbftFinalChainFactRequest {
                period: 1,
                candidate_final_chain_hash: [0; 32],
                collect_final_chain_hash: true,
                validate_candidate_final_chain_hash: true,
                collect_total_vote_count: true,
                collect_address_vote_counts: true,
                addresses: vec![rustaxa_ffi::PbftFinalChainFactAddress { address: validator }],
            })
            .expect("missing snapshot/header should be returned as data");
        assert_eq!(unavailable.status, PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE);
        assert_eq!(
            unavailable.final_chain_hash.status,
            PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE
        );
        assert!(!unavailable.has_total_vote_count);
        assert_eq!(
            unavailable.address_facts[0].status,
            PBFT_FINAL_CHAIN_FACT_STATUS_UNAVAILABLE
        );

        drop(final_chain);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_execution_session_requests_external_evm_for_contract_work() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_final_chain_execution_session");
        let storage_path = temp_dir.to_str().expect("temp path should be utf-8");
        let final_chain = make_final_chain(storage_path, vec![]);
        let mut session = create_final_chain_execution_session(
            &final_chain,
            rustaxa_ffi::FinalChainExecutionRequest {
                pbft_block_rlp: signed_pbft_block_rlp(7),
                transactions: vec![
                    ffi_transaction(1, true, [9; 20], Vec::new()),
                    ffi_transaction(2, true, [8; 20], vec![0xaa]),
                    ffi_transaction(3, false, [0; 20], Vec::new()),
                ],
                finalized_dag_blocks: Vec::new(),
                blocks_per_year: 0,
                cert_votes: Vec::new(),
                block_gas_limit: 1_000_000,
                mode: rustaxa_consensus::FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
            },
        )
        .expect("session should be created");

        let step = session
            .final_chain_execution_session_next()
            .expect("session step should convert");

        assert_eq!(
            step.status,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS
        );
        assert_eq!(
            step.action,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        assert_eq!(step.system_transaction_request.period, 7);
        assert_eq!(step.system_transaction_request.regular_transaction_count, 3);
        let step = session
            .final_chain_execution_session_report_system_transactions(
                rustaxa_ffi::FinalChainSystemTransactionReport {
                    request_id: step.system_transaction_request.request_id,
                    period: 7,
                    transactions: Vec::new(),
                },
            )
            .expect("system transaction report should convert");
        assert_eq!(
            step.status,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM
        );
        assert_eq!(
            step.action,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM
        );
        assert_eq!(step.period, 7);
        assert_eq!(step.evm_request.block_gas_limit, 1_000_000);
        assert_eq!(step.external_evm_transaction_count, 2);
        assert_eq!(step.evm_request.transactions.len(), 3);
        assert_eq!(step.evm_request.transactions[0].position, 0);
        assert!(step.evm_request.transactions[0].receiver_found);
        assert_eq!(step.evm_request.transactions[1].position, 1);
        assert!(step.evm_request.transactions[1].receiver_found);
        assert_eq!(step.evm_request.transactions[2].position, 2);
        assert!(!step.evm_request.transactions[2].receiver_found);

        drop(final_chain);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_execution_session_builds_external_evm_commit_plan() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_final_chain_execution_report");
        let storage_path = temp_dir.to_str().expect("temp path should be utf-8");
        let final_chain = make_final_chain(storage_path, vec![]);
        let mut session = create_final_chain_execution_session(
            &final_chain,
            rustaxa_ffi::FinalChainExecutionRequest {
                pbft_block_rlp: signed_pbft_block_rlp(7),
                transactions: vec![
                    ffi_transaction(1, true, [9; 20], Vec::new()),
                    ffi_transaction(2, true, [8; 20], vec![0xaa]),
                ],
                finalized_dag_blocks: Vec::new(),
                blocks_per_year: 0,
                cert_votes: Vec::new(),
                block_gas_limit: 1_000_000,
                mode: rustaxa_consensus::FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
            },
        )
        .expect("session should be created");
        let step = session
            .final_chain_execution_session_next()
            .expect("session step should convert");
        assert_eq!(
            step.action,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        let step = session
            .final_chain_execution_session_report_system_transactions(
                rustaxa_ffi::FinalChainSystemTransactionReport {
                    request_id: step.system_transaction_request.request_id,
                    period: 7,
                    transactions: Vec::new(),
                },
            )
            .expect("system transaction report should convert");

        let rewards = session
            .final_chain_execution_session_report_evm(rustaxa_ffi::FinalChainEvmExecutionReport {
                request_id: step.evm_request.request_id,
                status: rustaxa_consensus::FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
                state_root: [0x11; 32],
                cumulative_gas_used: 2,
                results: vec![
                    ffi_evm_result(&step.evm_request.transactions[0], 1, 1),
                    ffi_evm_result(&step.evm_request.transactions[1], 1, 2),
                ],
            })
            .expect("typed report should convert");

        assert_eq!(
            rewards.status,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_REWARDS
        );
        assert_eq!(
            rewards.action,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS
        );
        assert_eq!(rewards.evm_rewards_request.block_gas_used, 2);
        assert_eq!(rewards.evm_rewards_request.transaction_gas_used, vec![1, 1]);

        let plan = session
            .final_chain_execution_session_plan_external_evm_commit(
                rustaxa_ffi::FinalChainEvmRewardsReport {
                    request_id: step.evm_request.request_id,
                    period: 7,
                    status: rustaxa_consensus::FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
                    state_root: [0x22; 32],
                    total_reward: vec![0x33],
                },
            )
            .expect("commit plan should convert");

        assert!(plan.error_code.is_empty());
        assert_eq!(plan.period, 7);
        assert_eq!(plan.post_execution_state_root, [0x11; 32]);
        assert_eq!(plan.state_root, [0x22; 32]);
        assert_eq!(plan.total_reward, vec![0x33]);
        assert_eq!(plan.gas_used, 2);
        assert_eq!(plan.executed_transactions, 2);
        assert_eq!(plan.regular_transaction_count, 2);
        assert_eq!(plan.system_transaction_count, 0);
        assert_eq!(plan.encoded_receipts.len(), 2);
        assert_eq!(plan.header_log_bloom.len(), 256);
        assert_eq!(plan.indexed_log_bloom.len(), 256);

        let publication =
            final_chain_execution_session_plan_external_evm_publication(&final_chain, &mut session)
                .expect("publication plan should convert");
        assert!(publication.error_code.is_empty());
        assert_eq!(publication.period, 7);
        assert_ne!(publication.plan_id, [0; 32]);
        assert!(!publication.block_header_rlp.is_empty());
        assert!(!publication.stored_header_rlp.is_empty());
        assert_eq!(publication.receipts_rlp, plan.receipts_rlp);
        assert_eq!(publication.transaction_publications.len(), 2);
        assert!(!publication.transaction_publications[0].is_system);
        assert!(publication.system_transaction_hashes_rlp.len() == 1);

        let step = session
            .final_chain_execution_session_next()
            .expect("lifecycle step should convert");
        assert_eq!(
            step.status,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE
        );
        assert_eq!(
            step.action,
            rustaxa_consensus::FINAL_CHAIN_EXECUTION_ACTION_REPORT_EXTERNAL_EVM_LIFECYCLE
        );

        let decision = session
            .final_chain_execution_session_report_external_evm_lifecycle(
                rustaxa_ffi::FinalChainExternalEvmLifecycleReport {
                    request_id: publication.request_id,
                    plan_id: publication.plan_id,
                    period: publication.period,
                    post_execution_state_root: plan.post_execution_state_root,
                    post_rewards_state_root: plan.state_root,
                    publication_block_hash: publication.block_hash,
                    status: rustaxa_consensus::FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
                    error_code: String::new(),
                },
            )
            .expect("lifecycle decision should convert");
        assert_eq!(
            decision.status,
            rustaxa_consensus::FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH
        );
        assert_eq!(decision.request_id, publication.request_id);
        assert_eq!(decision.plan_id, publication.plan_id);
        assert_eq!(decision.publication_block_hash, publication.block_hash);
        assert!(decision.error_code.is_empty());

        drop(final_chain);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_execution_session_rejects_external_evm_lifecycle_plan_mismatch() {
        let (temp_dir, final_chain, mut session, plan, publication) =
            external_evm_publication_fixture(
                "rustaxa_bridge_final_chain_execution_lifecycle_mismatch",
            );
        let mut wrong_plan_id = publication.plan_id;
        wrong_plan_id[0] ^= 0xff;

        let decision = session
            .final_chain_execution_session_report_external_evm_lifecycle(
                rustaxa_ffi::FinalChainExternalEvmLifecycleReport {
                    request_id: publication.request_id,
                    plan_id: wrong_plan_id,
                    period: publication.period,
                    post_execution_state_root: plan.post_execution_state_root,
                    post_rewards_state_root: plan.state_root,
                    publication_block_hash: publication.block_hash,
                    status: rustaxa_consensus::FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
                    error_code: String::new(),
                },
            )
            .expect("lifecycle rejection should convert");

        assert_eq!(
            decision.status,
            rustaxa_consensus::FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED
        );
        assert_eq!(decision.period, 7);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_LIFECYCLE_PLAN_ID_MISMATCH"
        );

        drop(final_chain);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_compat_finalizer_rejects_external_evm_before_commit() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_final_chain_execution_reject");
        let storage_path = temp_dir.to_str().expect("temp path should be utf-8");
        let final_chain = make_final_chain(storage_path, vec![]);

        let error = match final_chain.finalize_block_with_rewards_facts(
            signed_pbft_block_rlp(7),
            vec![ffi_transaction(1, true, [8; 20], vec![0xaa])],
            Vec::new(),
            0,
            Vec::new(),
        ) {
            Ok(_) => panic!("external EVM transaction should not commit through native runtime"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("FINAL_CHAIN_EXECUTION_REQUIRES_EXTERNAL_EVM"));

        drop(final_chain);
        let _ = fs::remove_dir_all(temp_dir);
    }
}
