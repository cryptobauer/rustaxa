//! Read-only availability check for the three N4 native-state dependencies.
//! Complete live inventories do not establish semantic or historical catalog completeness.
use anyhow::{Context, Result, ensure};
use rocksdb::IteratorMode;
use rustaxa_consensus::{RewardsStatsPeriodRlp, decode_rewards_block_distributions};
use rustaxa_snapshot_qualifier::{native_inverse, paths, qualification};
use rustaxa_storage::{
    Column, ConcreteCheckpointReaders, ConcreteStorageInventoryLimits, FinalChainRepository,
};
use rustaxa_types::{
    FinalChainBlockNumber, StoredFinalChainBlockHeader,
    codec::rlp::final_chain::StoredBlockHeaderRlp,
    concrete_state::{ConcreteRead, ConcreteStateIdentity},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

const PERIODS: [u64; 3] = [25_706_948, 25_706_944, 25_706_942];
const LIMITS: ConcreteStorageInventoryLimits = ConcreteStorageInventoryLimits {
    max_nodes: 50_000,
    max_leaves: 50_000,
    max_value_bytes: 32 * 1024 * 1024,
};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: snapshot_input_availability COPY OUTPUT")?,
    );
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let paths = paths::validate(&input, &output)?;
    let (_, head, pair) = qualification::qualify(&paths.application, &paths.state)?;
    let application = qualification::open_application(&paths.application)?;
    let chain = FinalChainRepository::new(application.clone());
    let mut native = Vec::new();
    for period in PERIODS {
        let result = (|| -> Result<Value> {
            let raw = chain
                .block_header_raw(period)?
                .context("missing finalized header")?;
            let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw))?;
            let identity = ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(period),
                state_root: header.state_root.into(),
            };
            let readers =
                ConcreteCheckpointReaders::open_read_only(&paths.state, head, [identity, head])?;
            let mut accounts = Vec::new();
            for suffix in [0xfe, 0xee] {
                let mut address = [0; 20];
                address[19] = suffix;
                let result = readers.storage_inventory_at(identity, address, LIMITS);
                let summary = match result {
                    Ok(ConcreteRead::Present(inventory)) => {
                        let inverse = if suffix == 0xfe {
                            match native_inverse::analyze_native_head(&inventory, |key| {
                                readers.storage_at(identity, address, key)
                            }) {
                                Ok(coverage) => {
                                    json!({"status": "partial_inverse_completed", "coverage": coverage})
                                }
                                Err(error) => {
                                    json!({"status": "partial_inverse_failed", "error": format!("{error:#}")})
                                }
                            }
                        } else {
                            Value::Null
                        };
                        json!({"address_hex": hex::encode(address), "status": "complete_live_inventory", "storage_root_hex": inventory.storage_root.map(hex::encode), "nodes_visited": inventory.nodes_visited, "live_entries": inventory.entries.len(), "value_bytes": inventory.value_bytes, "entries_sha256": native_inverse::digest_entries(&inventory.entries), "inverse": inverse})
                    }
                    Ok(ConcreteRead::Absent) => {
                        json!({"address_hex": hex::encode(address), "status": "authenticated_account_absent"})
                    }
                    Ok(ConcreteRead::Tombstone) => {
                        json!({"address_hex": hex::encode(address), "status": "authenticated_account_tombstone"})
                    }
                    Err(error) => {
                        json!({"address_hex": hex::encode(address), "status": "inventory_failed", "error": format!("{error:?}")})
                    }
                };
                accounts.push(summary);
            }
            Ok(
                json!({"period": period, "header_sha256": hex::encode(Sha256::digest(raw)), "state_root_hex": hex::encode(identity.state_root), "root_authenticated": true, "accounts": accounts, "rust_dpos_sidecar_present": chain.dpos_snapshot_raw(period)?.is_some()}),
            )
        })();
        native.push(match result {
            Ok(value) => value,
            Err(error) => json!({"period": period, "status": "dependency_check_failed", "error": format!("{error:#}")}),
        });
    }
    // Bound both row count and bytes; never load an unbounded cache into memory.
    let cache = (|| -> Result<Value> {
        let cf = application
            .cf_handle(Column::BlockRewardsStats.name())
            .context("missing rewards column")?;
        let mut rows = Vec::new();
        let mut bytes = 0usize;
        for item in application.iterator_cf(&cf, IteratorMode::Start) {
            let (key, value) = item?;
            ensure!(rows.len() < 128, "reward cache exceeds 128-row bound");
            bytes += key.len() + value.len();
            ensure!(bytes <= 1024 * 1024, "reward cache exceeds 1MiB bound");
            let period = u64::from_le_bytes(
                key.as_ref()
                    .try_into()
                    .context("invalid cache period key")?,
            );
            let decoded = decode_rewards_block_distributions(&[RewardsStatsPeriodRlp {
                period,
                data: value.to_vec(),
            }])?;
            ensure!(decoded.len() == 1, "reward row did not decode exactly once");
            rows.push(json!({"period": period, "bytes": value.len(), "sha256": hex::encode(Sha256::digest(&value))}));
        }
        rows.sort_by_key(|row| row["period"].as_u64().unwrap());
        let periods = rows
            .iter()
            .map(|row| row["period"].as_u64().unwrap())
            .collect::<Vec<_>>();
        let candidate_parent = (25_706_901..=25_706_948).collect::<Vec<_>>();
        let retained_parent = periods
            .iter()
            .copied()
            .filter(|p| *p <= 25_706_948)
            .collect::<Vec<_>>();
        Ok(
            json!({"status": "complete_bounded_column_decoded", "rows": rows, "bytes": bytes, "candidate_frequency": 100, "candidate_parent_cache_periods_complete": retained_parent == candidate_parent, "candidate_parent_cache_excludes_head": true, "historical_parent_cache_proven": false}),
        )
    })();
    let cache = match cache {
        Ok(value) => value,
        Err(error) => json!({"status": "cache_check_failed", "error": format!("{error:#}")}),
    };
    let report = json!({"schema": 1, "open_mode": "read_only", "input_copy": paths.input, "pair": pair, "native_dependencies": native, "reward_cache": cache, "inventory_bounds_per_account": {"nodes": LIMITS.max_nodes, "leaves": LIMITS.max_leaves, "value_bytes": LIMITS.max_value_bytes}, "tool_source_sha256": hex::encode(Sha256::digest(include_bytes!("snapshot_input_availability.rs"))), "qualification": {"complete_semantic_snapshots": false, "producer_policy_qualified": false, "historical_owner_constructed": false, "reward_transition_executed": false}});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
