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
            } = validator;
            rustaxa_consensus::GenesisValidator {
                address,
                vrf_key,
                total_stake,
                metadata: rustaxa_consensus::GenesisValidatorMetadata {
                    owner,
                    commission,
                    description,
                    endpoint,
                },
            }
        })
        .collect();
    let final_chain = FinalChain::new(
        storage.0.clone(),
        block_gas_limit,
        genesis_timestamp,
        genesis_accounts,
        genesis_validators,
        rustaxa_consensus::GenesisDposConfig {
            eligibility_balance_threshold: genesis_dpos_config.eligibility_balance_threshold,
            vote_eligibility_balance_step: genesis_dpos_config.vote_eligibility_balance_step,
            validator_maximum_stake: genesis_dpos_config.validator_maximum_stake,
            dag_vdf_sortition_total_vote_count_until_period: genesis_dpos_config
                .dag_vdf_sortition_total_vote_count_until_period,
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
                transaction_hashes: dag_block
                    .transaction_hashes
                    .into_iter()
                    .map(|hash| hash.hash)
                    .collect(),
            })
            .collect();
        let (block_header_rlp, receipts) =
            self.0
                .finalize_block(pbft_block_rlp, transactions, finalized_dag_blocks)?;
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
