use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeFinalChain;
use crate::ffi::BridgeStorage;
use rustaxa_consensus::{Account, FinalChain};

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
            rewards_distribution_frequency: rewards_config
                .frequency_rules
                .into_iter()
                .map(|rule| (rule.from_period, rule.frequency))
                .collect(),
        },
    )?;
    Ok(Box::new(BridgeFinalChain(final_chain)))
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
        let transactions = transactions
            .into_iter()
            .map(|transaction| rustaxa_consensus::FinalizationTransaction {
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
            })
            .collect();
        let finalized_dag_blocks = finalized_dag_blocks
            .into_iter()
            .map(|dag_block| rustaxa_consensus::FinalizationDagBlock {
                author: dag_block.author,
                difficulty: dag_block.difficulty,
                transaction_hashes: dag_block
                    .transaction_hashes
                    .into_iter()
                    .map(|hash| hash.hash)
                    .collect(),
            })
            .collect();
        let cert_votes = cert_votes
            .into_iter()
            .map(|vote| rustaxa_consensus::RewardCertVoteFact {
                voter: vote.voter.into(),
                weight: vote.weight,
                period: vote.period,
            })
            .collect();
        let (block_header_rlp, receipts) = self.0.finalize_block_with_rewards_facts(
            pbft_block_rlp,
            transactions,
            finalized_dag_blocks,
            blocks_per_year,
            cert_votes,
        )?;
        Ok(rustaxa_ffi::FinalizationOutcome {
            block_header_rlp,
            receipts: receipts
                .into_iter()
                .map(|data| rustaxa_ffi::ReceiptRlp { data })
                .collect(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::create_storage;
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
}
