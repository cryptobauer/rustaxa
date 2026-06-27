//! CXX bridge adapter for Rust-owned gas-price oracle state.
//!
//! C++ preserves the legacy `GasPricer` constructor and live transaction
//! pointers. This adapter owns deterministic gas-price history, percentile
//! calculation, minimum-price flooring, and finalized-history restoration
//! through the Rust storage repository.

use crate::ffi::rustaxa_ffi::{GasPricerConfig, GasPricerGasPrice};
use crate::ffi::{BridgeGasPricer, BridgeStorage};
use anyhow::{anyhow, Result};
use ethereum_types::U256;
use rustaxa_consensus::gas_pricer::{GasPriceOracle, GasPricerConfig as DomainGasPricerConfig};
use std::sync::Mutex;

/// Creates a Rust gas-price oracle using legacy-compatible configuration.
pub fn create_gas_pricer(config: GasPricerConfig) -> Result<Box<BridgeGasPricer>> {
    let domain_config = DomainGasPricerConfig {
        percentile: config.percentile,
        minimum_price: from_bridge_u256(&config.minimum_price),
        history_blocks: config.history_blocks,
        is_light_node: config.is_light_node,
        blocks_gas_pricer: config.blocks_gas_pricer,
    };
    Ok(Box::new(BridgeGasPricer(
        Mutex::new(GasPriceOracle::new(domain_config)?),
        None,
    )))
}

/// Creates a Rust gas-price oracle that owns the Rust storage handle used for
/// finalized-history restoration.
///
/// The bridge clones the `Arc<Storage>` from `BridgeStorage`, restores history
/// before returning the opaque handle, and keeps that storage alive with the
/// gas-pricer runtime. C++ can then construct the Rust-mode `GasPricer` without
/// retaining or passing `DbStorage` into a separate init step.
pub fn create_gas_pricer_from_storage(
    config: GasPricerConfig,
    storage: &BridgeStorage,
) -> Result<Box<BridgeGasPricer>> {
    let pricer = create_gas_pricer(config)?;
    {
        let mut oracle = pricer.lock()?;
        oracle.restore_from_storage(storage.0.as_ref())?;
    }
    let BridgeGasPricer(oracle, _) = *pricer;
    Ok(Box::new(BridgeGasPricer(oracle, Some(storage.0.clone()))))
}

impl BridgeGasPricer {
    /// Returns the current block-history bid with the configured minimum floor.
    pub fn gas_pricer_bid(&self) -> Result<[u8; 32]> {
        let oracle = self.lock()?;
        Ok(to_bridge_u256(oracle.bid()))
    }

    /// Returns the transaction-pool bid with the configured minimum floor.
    pub fn gas_pricer_bid_from_pool(&self, pool_price: &[u8; 32]) -> Result<[u8; 32]> {
        let oracle = self.lock()?;
        Ok(to_bridge_u256(
            oracle.bid_from_pool(from_bridge_u256(pool_price)),
        ))
    }

    /// Updates finalized-block history from live transaction gas-price facts.
    pub fn gas_pricer_update(&self, gas_prices: Vec<GasPricerGasPrice>) -> Result<()> {
        let mut oracle = self.lock()?;
        oracle.update_from_gas_prices(
            gas_prices
                .into_iter()
                .map(|gas_price| from_bridge_u256(&gas_price.price)),
        );
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, GasPriceOracle>> {
        let _storage_owner = &self.1;
        self.0
            .lock()
            .map_err(|_| anyhow!("gas pricer mutex poisoned"))
    }
}

fn from_bridge_u256(value: &[u8; 32]) -> U256 {
    U256::from_big_endian(value)
}

fn to_bridge_u256(value: U256) -> [u8; 32] {
    value.to_big_endian()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use rlp::RlpStream;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn be(value: u64) -> [u8; 32] {
        U256::from(value).to_big_endian()
    }

    #[test]
    fn bridge_update_and_bid_match_percentile() {
        let oracle = create_gas_pricer(GasPricerConfig {
            percentile: 50,
            minimum_price: be(1),
            history_blocks: 100,
            is_light_node: false,
            blocks_gas_pricer: true,
        })
        .unwrap();

        for price in [1, 2, 3, 4, 5] {
            oracle
                .gas_pricer_update(vec![GasPricerGasPrice { price: be(price) }])
                .unwrap();
        }

        assert_eq!(oracle.gas_pricer_bid().unwrap(), be(3));
    }

    #[test]
    fn bridge_create_from_storage_populates_percentile_history() {
        let storage = crate::storage::create_storage(
            unique_storage_path("gas_pricer_create_from_storage_ok").as_str(),
        )
        .unwrap();
        init_gas_pricer_storage(&storage, &[(2, &[9, 5]), (1, &[8])]).unwrap();
        seed_last_finalized_block(&storage, 2).unwrap();

        let oracle = create_gas_pricer_from_storage(
            GasPricerConfig {
                percentile: 50,
                minimum_price: be(1),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: true,
            },
            &storage,
        )
        .unwrap();

        assert_eq!(oracle.gas_pricer_bid().unwrap(), be(5));
    }

    #[test]
    fn bridge_create_from_storage_light_node_stops_on_missing_period_data() {
        let storage = crate::storage::create_storage(
            unique_storage_path("gas_pricer_create_from_storage_light").as_str(),
        )
        .unwrap();
        init_gas_pricer_storage(&storage, &[(2, &[9])]).unwrap();
        seed_last_finalized_block(&storage, 3).unwrap();

        let oracle = create_gas_pricer_from_storage(
            GasPricerConfig {
                percentile: 100,
                minimum_price: be(7),
                history_blocks: 10,
                is_light_node: true,
                blocks_gas_pricer: true,
            },
            &storage,
        )
        .unwrap();

        // Period 3 is missing and light mode should stop initialization instead of erroring.
        assert_eq!(oracle.gas_pricer_bid().unwrap(), be(7));
    }

    #[test]
    fn bridge_create_from_storage_full_node_fails_on_missing_period_data() {
        let storage = crate::storage::create_storage(
            unique_storage_path("gas_pricer_create_from_storage_full").as_str(),
        )
        .unwrap();
        init_gas_pricer_storage(&storage, &[(2, &[9])]).unwrap();
        seed_last_finalized_block(&storage, 3).unwrap();

        let err = match create_gas_pricer_from_storage(
            GasPricerConfig {
                percentile: 100,
                minimum_price: be(1),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: true,
            },
            &storage,
        ) {
            Ok(_) => {
                panic!("expected missing period data to fail full-node gas-pricer restoration")
            }
            Err(err) => err.to_string(),
        };

        assert!(err.contains("missing finalized transactions for block 3"));
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

    fn unique_storage_path(tag: &str) -> String {
        let mut path = env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos());
        path.push(format!("{tag}-{}-{}", std::process::id(), nanos));
        path.to_string_lossy().to_string()
    }

    fn seed_last_finalized_block(storage: &BridgeStorage, block: u64) -> Result<()> {
        storage
            .0
            .final_chain()
            .write_block_header(block, H256::zero(), &[], &[])?;
        Ok(())
    }

    fn init_gas_pricer_storage(storage: &BridgeStorage, blocks: &[(u64, &[u64])]) -> Result<()> {
        for &(period, prices) in blocks {
            let mut period_rlp = RlpStream::new_list(4);
            period_rlp.append_empty_data();
            period_rlp.append_empty_data();
            period_rlp.begin_list(0);
            period_rlp.begin_list(prices.len());
            for &gas_price in prices {
                append_transaction(&mut period_rlp, gas_price);
            }
            storage.save_period_data(period, period_rlp.out().to_vec())?;
        }
        Ok(())
    }
}
