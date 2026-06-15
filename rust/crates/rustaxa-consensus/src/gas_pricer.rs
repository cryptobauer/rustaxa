//! Deterministic gas-price oracle used by Rust-backed consensus shims.
//!
//! The oracle mirrors legacy `GasPricer` behavior without owning transaction
//! objects or storage. Callers provide either live transaction gas-price facts
//! or finalized-block gas-price facts loaded through a repository-specific
//! adapter. The module maintains the rolling history used by block gas pricing,
//! applies the configured minimum price floor, and intentionally ignores
//! zero-priced blocks exactly as the legacy implementation does.

use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::U256;
use rlp::Rlp;
use rustaxa_storage::Storage;
use std::collections::VecDeque;

const FINAL_CHAIN_META_LAST_NUMBER: u32 = 1;
const TRANSACTIONS_POS_IN_PERIOD_DATA: usize = 3;
const TRANSACTION_GAS_PRICE_POS_IN_RLP: usize = 1;

/// Runtime configuration for [`GasPriceOracle`].
///
/// `percentile` is the inclusive legacy percentile in the `0..=100` range.
/// `minimum_price` is applied as a floor to all public bids. `history_blocks`
/// bounds the finalized-block history used by block gas pricing.
/// `is_light_node` controls whether missing finalized transaction payloads stop
/// storage restoration or are treated as an error by the bridge adapter.
/// `blocks_gas_pricer` disables block-history updates when the node uses the
/// transaction-pool price path instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GasPricerConfig {
    pub percentile: u64,
    pub minimum_price: U256,
    pub history_blocks: usize,
    pub is_light_node: bool,
    pub blocks_gas_pricer: bool,
}

/// Rolling finalized-block gas-price oracle.
///
/// Invariants:
/// - history entries are non-zero minimum gas prices for non-empty finalized
///   blocks
/// - history length never exceeds `config.history_blocks`
/// - `latest_price` is either the configured minimum price or the selected
///   percentile value from history
/// - pool-mode bids do not mutate finalized-block history
#[derive(Debug)]
pub struct GasPriceOracle {
    config: GasPricerConfig,
    latest_price: U256,
    price_history: VecDeque<U256>,
}

impl GasPriceOracle {
    /// Creates an empty oracle from legacy-compatible configuration.
    ///
    /// Returns an error when `percentile` is outside the legacy `0..=100`
    /// range.
    pub fn new(config: GasPricerConfig) -> Result<Self> {
        ensure!(
            config.percentile <= 100,
            "gas price percentile must be <= 100"
        );
        Ok(Self {
            latest_price: config.minimum_price,
            price_history: VecDeque::with_capacity(config.history_blocks),
            config,
        })
    }

    /// Returns the block-history bid with the minimum-price floor applied.
    pub fn bid(&self) -> U256 {
        self.latest_price.max(self.config.minimum_price)
    }

    /// Returns the transaction-pool bid with the minimum-price floor applied.
    ///
    /// This path is used when legacy `GasPricer` is constructed without the
    /// finalized-block gas-pricer mode.
    pub fn bid_from_pool(&self, pool_price: U256) -> U256 {
        pool_price.max(self.config.minimum_price)
    }

    /// Records gas prices from a newly finalized non-empty block.
    ///
    /// Empty inputs and zero minimum prices leave the oracle unchanged. When
    /// the oracle is not configured for block-history pricing, the call is a
    /// deliberate no-op so the same bridge object can preserve the legacy
    /// `update` API in both modes.
    pub fn update_from_gas_prices<I>(&mut self, gas_prices: I)
    where
        I: IntoIterator<Item = U256>,
    {
        if !self.config.blocks_gas_pricer {
            return;
        }
        let Some(min_price) = gas_prices.into_iter().min() else {
            return;
        };
        if min_price.is_zero() {
            return;
        }
        self.push_back_price(min_price);
    }

    /// Restores one finalized block while walking history from newest to oldest.
    ///
    /// The restored price is pushed to the front of history so that after a
    /// reverse walk the deque has the same old-to-new order as the legacy
    /// circular buffer. Empty blocks and zero minimum prices are ignored.
    pub fn restore_finalized_block_gas_prices<I>(&mut self, gas_prices: I)
    where
        I: IntoIterator<Item = U256>,
    {
        if !self.config.blocks_gas_pricer || self.price_history.len() == self.config.history_blocks
        {
            return;
        }
        let Some(min_price) = gas_prices.into_iter().min() else {
            return;
        };
        if min_price.is_zero() {
            return;
        }
        self.push_front_price(min_price);
    }

    /// Restores finalized-block gas-price history directly from Rust storage.
    ///
    /// Inputs:
    /// - `storage`: native Rust storage handle containing FinalChain metadata
    ///   and finalized period-data rows.
    ///
    /// Outputs:
    /// - Mutates the oracle by walking finalized blocks from newest to oldest
    ///   until configured history is full, genesis is reached, or light-node
    ///   storage is missing a period-data row.
    ///
    /// Invariants and edge behavior:
    /// - Full nodes treat missing finalized period data as an error because the
    ///   deterministic gas-price history would be incomplete.
    /// - Light nodes stop restoration on the first missing period data row,
    ///   matching the legacy storage-backed gas-pricer behavior.
    /// - If FinalChain `LAST_NUMBER` is missing, restoration is a no-op.
    pub fn restore_from_storage(&mut self, storage: &Storage) -> Result<()> {
        let latest_number = latest_final_chain_number(storage)?;
        let mut block_number = latest_number;

        while block_number > 0 && !self.history_full() {
            let period_data = storage
                .period()
                .data_raw(block_number)
                .with_context(|| format!("load period data for finalized block {block_number}"))?;
            if period_data.is_empty() {
                if self.is_light_node() {
                    break;
                }
                return Err(anyhow!(
                    "missing finalized transactions for block {block_number} on full node"
                ));
            }

            let gas_prices = gas_prices_from_period_data(&period_data).with_context(|| {
                format!("decode transaction gas prices for period {block_number}")
            })?;
            self.restore_finalized_block_gas_prices(gas_prices);
            block_number -= 1;
        }
        Ok(())
    }

    /// Returns true when the finalized-block history is at configured capacity.
    pub fn history_full(&self) -> bool {
        self.price_history.len() == self.config.history_blocks
    }

    /// Returns true when missing finalized transaction payloads should stop
    /// restoration instead of failing.
    pub fn is_light_node(&self) -> bool {
        self.config.is_light_node
    }

    fn push_back_price(&mut self, price: U256) {
        if self.config.history_blocks == 0 {
            return;
        }
        if self.price_history.len() == self.config.history_blocks {
            self.price_history.pop_front();
        }
        self.price_history.push_back(price);
        self.recalculate_latest_price();
    }

    fn push_front_price(&mut self, price: U256) {
        if self.config.history_blocks == 0 || self.history_full() {
            return;
        }
        self.price_history.push_front(price);
        self.recalculate_latest_price();
    }

    fn recalculate_latest_price(&mut self) {
        if self.price_history.is_empty() {
            return;
        }
        let mut sorted_prices = self.price_history.iter().copied().collect::<Vec<_>>();
        sorted_prices.sort_unstable();
        let index = (sorted_prices.len() - 1) * self.config.percentile as usize / 100;
        let new_price = sorted_prices[index];
        if !new_price.is_zero() {
            self.latest_price = new_price;
        }
    }
}

fn latest_final_chain_number(storage: &Storage) -> Result<u64> {
    let Some(raw) = storage
        .final_chain()
        .meta_value(FINAL_CHAIN_META_LAST_NUMBER)
        .with_context(|| "load final chain LAST_NUMBER from metadata")?
    else {
        return Ok(0);
    };
    ensure!(
        raw.len() == std::mem::size_of::<u64>(),
        "invalid final-chain LAST_NUMBER payload size: {}",
        raw.len()
    );
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&raw);
    Ok(u64::from_le_bytes(bytes))
}

fn gas_prices_from_period_data(period_data: &[u8]) -> Result<Vec<U256>> {
    let rlp = Rlp::new(period_data);
    let transactions = rlp.at(TRANSACTIONS_POS_IN_PERIOD_DATA)?;
    let mut gas_prices = Vec::with_capacity(transactions.item_count()?);
    for transaction in transactions.iter() {
        gas_prices.push(transaction.val_at(TRANSACTION_GAS_PRICE_POS_IN_RLP)?);
    }
    Ok(gas_prices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, Storage};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config(percentile: u64, minimum_price: u64, history_blocks: usize) -> GasPricerConfig {
        GasPricerConfig {
            percentile,
            minimum_price: U256::from(minimum_price),
            history_blocks,
            is_light_node: false,
            blocks_gas_pricer: true,
        }
    }

    #[test]
    fn update_matches_legacy_percentile_selection() {
        let mut oracle = GasPriceOracle::new(config(50, 1, 200)).unwrap();

        for price in [1, 2, 3, 4, 5] {
            oracle.update_from_gas_prices([U256::from(price)]);
        }

        assert_eq!(oracle.bid(), U256::from(3));
    }

    #[test]
    fn history_capacity_drops_oldest_update() {
        let mut oracle = GasPriceOracle::new(config(0, 1, 3)).unwrap();

        for price in [5, 6, 7, 8] {
            oracle.update_from_gas_prices([U256::from(price)]);
        }

        assert_eq!(
            oracle.price_history.iter().copied().collect::<Vec<_>>(),
            vec![U256::from(6), U256::from(7), U256::from(8)]
        );

        assert_eq!(oracle.bid(), U256::from(6));
    }

    #[test]
    fn restore_pushes_newest_to_front_while_walking_backwards() {
        let mut oracle = GasPriceOracle::new(config(100, 1, 3)).unwrap();

        oracle.restore_finalized_block_gas_prices([U256::from(9)]);
        oracle.restore_finalized_block_gas_prices([U256::from(4)]);
        oracle.restore_finalized_block_gas_prices([U256::from(7)]);
        oracle.restore_finalized_block_gas_prices([U256::from(99)]);

        assert_eq!(
            oracle.price_history.iter().copied().collect::<Vec<_>>(),
            vec![U256::from(7), U256::from(4), U256::from(9)]
        );
        assert!(oracle.history_full());
        assert_eq!(oracle.bid(), U256::from(9));
    }

    #[test]
    fn restore_ignores_zero_min_price_block() {
        let mut oracle = GasPriceOracle::new(config(50, 1, 3)).unwrap();

        oracle.restore_finalized_block_gas_prices([U256::from(0), U256::from(7)]);
        oracle.restore_finalized_block_gas_prices([U256::from(5)]);

        assert_eq!(
            oracle.price_history.iter().copied().collect::<Vec<_>>(),
            vec![U256::from(5)]
        );
        assert_eq!(oracle.bid(), U256::from(5));
    }

    #[test]
    fn pool_bid_applies_minimum_without_mutating_history() {
        let oracle = GasPriceOracle::new(config(50, 10, 10)).unwrap();

        assert_eq!(oracle.bid_from_pool(U256::from(3)), U256::from(10));
        assert_eq!(oracle.bid(), U256::from(10));
    }

    #[test]
    fn rejects_invalid_percentile() {
        let err = GasPriceOracle::new(config(101, 1, 1)).unwrap_err();

        assert!(err.to_string().contains("percentile"));
    }

    #[test]
    fn restore_from_storage_populates_percentile_history() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_gas_pricer_restore_ok");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            init_gas_pricer_storage(&storage, &[(2, &[9, 5]), (1, &[8])]).unwrap();
            seed_last_finalized_block(&storage, 2).unwrap();

            let mut oracle = GasPriceOracle::new(config(50, 1, 10)).unwrap();

            oracle.restore_from_storage(&storage).unwrap();

            assert_eq!(oracle.bid(), U256::from(5));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn restore_from_storage_light_node_stops_on_missing_period_data() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_gas_pricer_restore_light");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            init_gas_pricer_storage(&storage, &[(2, &[9])]).unwrap();
            seed_last_finalized_block(&storage, 3).unwrap();

            let mut light_config = config(100, 7, 10);
            light_config.is_light_node = true;
            let mut oracle = GasPriceOracle::new(light_config).unwrap();

            oracle.restore_from_storage(&storage).unwrap();

            assert_eq!(oracle.bid(), U256::from(7));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn restore_from_storage_full_node_fails_on_missing_period_data() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_gas_pricer_restore_full");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            init_gas_pricer_storage(&storage, &[(2, &[9])]).unwrap();
            seed_last_finalized_block(&storage, 3).unwrap();

            let mut oracle = GasPriceOracle::new(config(100, 1, 10)).unwrap();

            let err = oracle
                .restore_from_storage(&storage)
                .unwrap_err()
                .to_string();

            assert!(err.contains("missing finalized transactions for block 3"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn period_data_decoder_reads_legacy_transaction_gas_price_field() {
        let mut period = RlpStream::new_list(4);
        period.append_empty_data();
        period.begin_list(0);
        period.begin_list(0);
        period.begin_list(2);
        append_transaction(&mut period, 9);
        append_transaction(&mut period, 4);

        let prices = gas_prices_from_period_data(&period.out()).unwrap();

        assert_eq!(prices, vec![U256::from(9), U256::from(4)]);
    }

    fn append_transaction(stream: &mut RlpStream, gas_price: u64) {
        stream.begin_list(9);
        stream.append(&0u64);
        stream.append(&gas_price);
        stream.append(&21000u64);
        stream.append_empty_data();
        stream.append(&0u64);
        stream.append_empty_data();
        stream.append(&27u64);
        stream.append(&1u64);
        stream.append(&1u64);
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn seed_last_finalized_block(storage: &Storage, block: u64) -> Result<()> {
        storage
            .final_chain()
            .write_block_header(block, H256::zero(), &[], &[])
    }

    fn init_gas_pricer_storage(storage: &Storage, blocks: &[(u64, &[u64])]) -> Result<()> {
        for &(period, prices) in blocks {
            let mut period_rlp = RlpStream::new_list(4);
            period_rlp.append_empty_data();
            period_rlp.append_empty_data();
            period_rlp.begin_list(0);
            period_rlp.begin_list(prices.len());
            for &gas_price in prices {
                append_transaction(&mut period_rlp, gas_price);
            }
            storage.period().write(period, &period_rlp.out())?;
        }
        Ok(())
    }
}
