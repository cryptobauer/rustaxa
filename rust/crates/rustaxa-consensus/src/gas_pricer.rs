//! Deterministic gas-price oracle used by Rust-backed consensus shims.
//!
//! The oracle mirrors legacy `GasPricer` behavior without owning transaction
//! objects or storage. Callers provide either live transaction gas-price facts
//! or finalized-block gas-price facts loaded through a repository-specific
//! adapter. The module maintains the rolling history used by block gas pricing,
//! applies the configured minimum price floor, and intentionally ignores
//! zero-priced blocks exactly as the legacy implementation does.

use anyhow::{Result, ensure};
use ethereum_types::U256;
use std::collections::VecDeque;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
