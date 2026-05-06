use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeFinalChain;
use crate::ffi::BridgeStorage;
use rustaxa_consensus::FinalChain;

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
        .map(|validator| rustaxa_consensus::GenesisValidator {
            address: validator.address,
            vrf_key: validator.vrf_key,
            total_stake: validator.total_stake,
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
        Ok(match self.0.account(*address)? {
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
        })
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
        address: &[u8; 20],
    ) -> Result<u64, anyhow::Error> {
        self.0.dpos_eligible_vote_count(*address)
    }

    pub fn get_dpos_eligible_total_vote_count(
        self: &BridgeFinalChain,
    ) -> Result<u64, anyhow::Error> {
        self.0.dpos_eligible_total_vote_count()
    }

    pub fn get_dpos_is_eligible(
        self: &BridgeFinalChain,
        address: &[u8; 20],
    ) -> Result<bool, anyhow::Error> {
        self.0.dpos_is_eligible(*address)
    }

    pub fn estimate_call_gas(
        self: &BridgeFinalChain,
        gas_limit: u64,
    ) -> Result<u64, anyhow::Error> {
        self.0.estimate_call_gas(gas_limit)
    }

    pub fn finalize_block(
        self: &BridgeFinalChain,
        pbft_block_rlp: Vec<u8>,
        transactions: Vec<rustaxa_ffi::FinalizationTransaction>,
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
        let (block_header_rlp, receipts) = self.0.finalize_block(pbft_block_rlp, transactions)?;
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
