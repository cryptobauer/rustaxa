//! Eight read-only application observations for the exact retained H list witness.
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use rustaxa_snapshot_qualifier::{head_history as history, paths, qualification};
use rustaxa_storage::{
    FinalChainRepository, MetadataRepository, PeriodRepository, TransactionRepository,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

/// Adapter budget checked after each returned row, before another point read.
#[derive(Default)]
struct PointBudget {
    calls: usize,
    bytes: usize,
}
impl PointBudget {
    fn observe(&mut self, name: &str, read: impl FnOnce() -> Result<Vec<u8>>) -> Result<Vec<u8>> {
        ensure!(self.calls < 8, "point-read cap exceeded before {name}");
        self.calls += 1;
        let value = read().with_context(|| format!("read {name}"))?;
        self.bytes = self
            .bytes
            .checked_add(value.len())
            .context("point-byte overflow")?;
        ensure!(
            value.len() <= history::MAX_ROW_BYTES,
            "{name} row byte cap exceeded"
        );
        ensure!(
            self.bytes <= history::MAX_TOTAL_BYTES,
            "point-byte aggregate cap exceeded at {name}"
        );
        Ok(value)
    }
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: head_history_authentication COPY OUTPUT")?,
    );
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let guarded = paths::validate(&input, &output)?;
    // This scope has no state reader, inventory, replay or write capability.
    let database = qualification::open_application(&guarded.application)?;
    let chain = FinalChainRepository::new(database.clone());
    let metadata = MetadataRepository::new(database.clone());
    let periods = PeriodRepository::new(database.clone());
    let transactions = TransactionRepository::new(database);
    let hash = H256::from_slice(&hex::decode(history::HEAD_HASH)?);
    let mut budget = PointBudget::default();
    let rows = history::HeadHistoryRows {
        head: budget.observe("head", || chain.meta_value(1)?.context("missing head"))?,
        genesis: budget.observe("genesis", || {
            metadata.genesis_hash()?.context("missing genesis")
        })?,
        period_data: budget.observe("PeriodData H", || periods.data_raw(history::H))?,
        receipts: budget.observe("receipt list H", || periods.receipt(history::H))?,
        header: budget.observe("stored header H", || {
            chain
                .block_header_raw(history::H)?
                .context("missing H header")
        })?,
        hash_by_number: budget.observe("hash-by-number H", || {
            chain
                .block_hash_by_number(history::H)?
                .context("missing H hash")
        })?,
        number_by_hash: budget.observe("number-by-pinned-H-hash", || {
            chain
                .block_number_by_hash(hash)?
                .context("missing H reverse index")
        })?,
        system_hashes: budget.observe("system transaction hashes H", || {
            transactions.period_system_hashes_rlp(history::H)
        })?,
    };
    ensure!(budget.calls == 8, "H observation count differs");
    let result = history::authenticate_head_history(&rows);
    let successful = result.is_ok();
    let authentication = match result {
        Ok(witness) => json!({"status": "complete_h_lists_authenticated", "witness": witness}),
        Err(error) => json!({"status": "h_lists_rejected", "error": format!("{error:#}")}),
    };
    let report = json!({"schema": 1, "input_copy": guarded.input, "open_mode": "application read_only; no state database open",
        "period": history::H, "local_head_hash_hex": history::HEAD_HASH,
        "actual": {"point_calls": budget.calls, "accepted_point_bytes": budget.bytes},
        "bounds": {"point_calls": 8, "row_bytes": history::MAX_ROW_BYTES, "total_bytes": history::MAX_TOTAL_BYTES,
            "item_bytes": history::MAX_ITEM_BYTES, "signed_items": history::ITEM_COUNT, "receipt_items": history::ITEM_COUNT,
            "native_calls": 0, "proof_candidates": 0, "rlp_nodes_per_row": 4096, "rlp_depth": 64},
        "source_pins": {"binary_sha256": hex::encode(Sha256::digest(include_bytes!("head_history_authentication.rs"))),
            "verifier_sha256": hex::encode(Sha256::digest(include_bytes!("../head_history.rs")))},
        "rows": rows, "authentication": authentication,
        "qualification": {"independent_public_finality": false, "historical_key_completeness": false, "semantic_snapshot_complete": false,
            "historical_owner_ready": false, "reward_transition_executed": false, "adoption_authorized": false, "production_routing_authorized": false},
        "reference_policy_assumption": "owner-supplied default mainnet, likely v1.14.1; actual producer unverified"});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(guarded.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    ensure!(
        successful,
        "H list authentication rejected; observations saved"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapter_caps_stop_before_another_call() {
        let mut budget = PointBudget::default();
        for _ in 0..8 {
            budget.observe("empty", || Ok(Vec::new())).unwrap();
        }
        assert!(
            budget
                .observe("extra", || panic!("over-limit call"))
                .is_err()
        );
        let mut budget = PointBudget::default();
        assert!(
            budget
                .observe("large", || Ok(vec![0; history::MAX_ROW_BYTES + 1]))
                .is_err()
        );
        assert_eq!(budget.calls, 1);
        assert_eq!(budget.bytes, history::MAX_ROW_BYTES + 1);
        let mut budget = PointBudget::default();
        for _ in 0..2 {
            budget
                .observe("row", || Ok(vec![0; history::MAX_ROW_BYTES]))
                .unwrap();
        }
        assert!(budget.observe("overflow", || Ok(vec![1])).is_err());
        assert_eq!(budget.calls, 3);
        assert_eq!(budget.bytes, history::MAX_TOTAL_BYTES + 1);
    }
}
