//! Fixed staged application-only H-to-P acquisition; no state writes or scans.
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use rustaxa_snapshot_qualifier::{
    head_history as head, parent_history as parent, paths, qualification,
};
use rustaxa_storage::{
    FinalChainRepository, MetadataRepository, PeriodRepository, TransactionRepository,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

/// Separate H/P accepted budgets and all returned bytes, including rejection.
/// A failed observation seals the collector before another logical read.
#[derive(Default)]
struct PointBudget {
    calls: usize,
    returned_h: usize,
    returned_p: usize,
    accepted_h: usize,
    accepted_p: usize,
    failed: bool,
    observations: Vec<Value>,
}
impl PointBudget {
    fn observe(&mut self, name: &str, read: impl FnOnce() -> Result<Vec<u8>>) -> Result<Vec<u8>> {
        ensure!(
            !self.failed && self.calls < 14,
            "sealed point-read budget before {name}"
        );
        let is_h = self.calls < 8;
        self.calls += 1;
        let value = match read() {
            Ok(value) => value,
            Err(error) => {
                self.failed = true;
                self.observations
                    .push(json!({"name": name, "read_error": format!("{error:#}")}));
                return Err(error).with_context(|| format!("read {name}"));
            }
        };
        let (total, accepted, row_cap, total_cap) = if is_h {
            (
                &mut self.returned_h,
                &mut self.accepted_h,
                head::MAX_ROW_BYTES,
                head::MAX_TOTAL_BYTES,
            )
        } else {
            (
                &mut self.returned_p,
                &mut self.accepted_p,
                parent::MAX_ROW_BYTES,
                parent::MAX_TOTAL_BYTES,
            )
        };
        let sum = total.checked_add(value.len());
        self.failed = sum.is_none() || value.len() > row_cap || sum.is_some_and(|n| n > total_cap);
        if let Some(sum) = sum {
            *total = sum;
        }
        self.observations.push(
            json!({"name": name, "returned_bytes": value.len(), "accepted": !self.failed,
            "raw_hex": if self.failed { None } else { Some(hex::encode(&value)) }}),
        );
        ensure!(!self.failed, "point byte cap exceeded at {name}");
        *accepted = *total;
        Ok(value)
    }
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: parent_history_authentication COPY OUTPUT")?,
    );
    let output = PathBuf::from(args.next().context("missing output")?);
    ensure!(args.next().is_none(), "unexpected argument");
    let guarded = paths::validate(&input, &output)?;
    let database = qualification::open_application(&guarded.application)?;
    let chain = FinalChainRepository::new(database.clone());
    let metadata = MetadataRepository::new(database.clone());
    let periods = PeriodRepository::new(database.clone());
    let transactions = TransactionRepository::new(database);
    let mut budget = PointBudget::default();
    let mut stage = "H observations";
    let result = (|| -> Result<Value> {
        let hash = H256::from_slice(&hex::decode(head::HEAD_HASH)?);
        let h = head::HeadHistoryRows {
            head: budget.observe("H head", || chain.meta_value(1)?.context("missing head"))?,
            genesis: budget.observe("genesis", || {
                metadata.genesis_hash()?.context("missing genesis")
            })?,
            period_data: budget.observe("H PeriodData", || periods.data_raw(head::H))?,
            receipts: budget.observe("H receipts", || periods.receipt(head::H))?,
            header: budget.observe("H header", || {
                chain.block_header_raw(head::H)?.context("missing H header")
            })?,
            hash_by_number: budget.observe("H hash", || {
                chain
                    .block_hash_by_number(head::H)?
                    .context("missing H hash")
            })?,
            number_by_hash: budget.observe("H number", || {
                chain.block_number_by_hash(hash)?.context("missing H index")
            })?,
            system_hashes: budget.observe("H systems", || {
                transactions.period_system_hashes_rlp(head::H)
            })?,
        };
        stage = "H authentication";
        let context = parent::ParentLinkContext::authenticate_successor(&h)?;
        let mut p = parent::ParentHistoryRows::default();
        stage = "P header observations";
        p.header = budget.observe("P header", || {
            chain
                .block_header_raw(parent::P)?
                .context("missing P header")
        })?;
        p.period_data = budget.observe("P PeriodData", || periods.data_raw(parent::P))?;
        stage = "P header and PBFT linkage";
        context.authenticate_header(&p.header, &p.period_data)?;
        stage = "P indexes";
        p.hash_by_number = budget.observe("P hash", || {
            chain
                .block_hash_by_number(parent::P)?
                .context("missing P hash")
        })?;
        p.number_by_hash = budget.observe("P number", || {
            chain
                .block_number_by_hash(context.expected_hash())?
                .context("missing P index")
        })?;
        context.authenticate_indexes(&p.hash_by_number, &p.number_by_hash)?;
        stage = "P systems";
        p.system_hashes = budget.observe("P systems", || {
            transactions.period_system_hashes_rlp(parent::P)
        })?;
        ensure!(
            p.system_hashes.is_empty(),
            "P system transactions unsupported"
        );
        stage = "P receipts";
        p.receipts = budget.observe("P receipts", || periods.receipt(parent::P))?;
        stage = "complete P authentication";
        let witness = context.authenticate_rows(&p)?;
        ensure!(budget.calls == 14, "complete acquisition count differs");
        Ok(
            json!({"status": "complete_p_lists_authenticated", "witness": witness, "head_rows": h, "parent_rows": p}),
        )
    })();
    let successful = result.is_ok();
    let authentication = match result {
        Ok(value) => value,
        Err(error) => {
            json!({"status": "p_lists_rejected", "stage": stage, "error": format!("{error:#}")})
        }
    };
    let report = json!({"schema": 1, "input_copy": guarded.input, "open_mode": "application read_only; no state open",
        "period": parent::P, "successor_period": head::H, "authentication": authentication,
        "actual": {"point_calls": budget.calls, "returned_h_bytes": budget.returned_h, "returned_p_bytes": budget.returned_p,
            "accepted_h_bytes": budget.accepted_h, "accepted_p_bytes": budget.accepted_p}, "observations": budget.observations,
        "bounds": {"point_calls": 14, "h_row_bytes": head::MAX_ROW_BYTES, "h_total_bytes": head::MAX_TOTAL_BYTES,
            "p_row_bytes": parent::MAX_ROW_BYTES, "p_total_bytes": parent::MAX_TOTAL_BYTES,
            "p_items_per_list": parent::MAX_ITEMS, "p_item_bytes": parent::MAX_ITEM_BYTES, "p_rlp_nodes": parent::MAX_NODES, "rlp_depth": 64,
            "native_address_candidates": 0, "proof_key_candidates": 0},
        "source_pins": {"adapter_source_sha256": hex::encode(Sha256::digest(include_bytes!("parent_history_authentication.rs"))),
            "verifier_source_sha256": hex::encode(Sha256::digest(include_bytes!("../parent_history.rs"))),
            "head_verifier_source_sha256": hex::encode(Sha256::digest(include_bytes!("../head_history.rs")))},
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
        "P authentication rejected; diagnostic observations saved"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_and_byte_caps_count_rejection_and_seal_before_followup() {
        let mut budget = PointBudget::default();
        for _ in 0..14 {
            budget.observe("empty", || Ok(vec![])).unwrap();
        }
        assert!(budget.observe("extra", || panic!("extra read")).is_err());
        let mut budget = PointBudget::default();
        assert!(
            budget
                .observe("large H", || Ok(vec![0; head::MAX_ROW_BYTES + 1]))
                .is_err()
        );
        assert_eq!(
            (budget.calls, budget.returned_h, budget.accepted_h),
            (1, head::MAX_ROW_BYTES + 1, 0)
        );
        assert!(
            budget
                .observe("after rejection", || panic!("extra read"))
                .is_err()
        );
        let mut budget = PointBudget::default();
        for _ in 0..8 {
            budget.observe("H", || Ok(vec![])).unwrap();
        }
        for _ in 0..2 {
            budget
                .observe("P", || Ok(vec![0; parent::MAX_ROW_BYTES]))
                .unwrap();
        }
        assert!(budget.observe("P total overflow", || Ok(vec![1])).is_err());
        assert_eq!(
            (budget.returned_p, budget.accepted_p),
            (parent::MAX_TOTAL_BYTES + 1, parent::MAX_TOTAL_BYTES)
        );
        assert!(
            budget
                .observe("after overflow", || panic!("extra read"))
                .is_err()
        );
        let mut budget = PointBudget::default();
        for _ in 0..8 {
            budget.observe("H", || Ok(vec![])).unwrap();
        }
        assert!(
            budget
                .observe("P row overflow", || Ok(vec![0; parent::MAX_ROW_BYTES + 1]))
                .is_err()
        );
        assert_eq!(
            (budget.calls, budget.returned_p, budget.accepted_p),
            (9, parent::MAX_ROW_BYTES + 1, 0)
        );
        assert!(
            budget
                .observe("after P row rejection", || panic!("extra read"))
                .is_err()
        );
        let mut budget = PointBudget::default();
        assert!(
            budget
                .observe("missing", || Err(anyhow::anyhow!("missing row")))
                .is_err()
        );
        assert_eq!(budget.calls, 1);
        assert!(
            budget
                .observe("after missing", || panic!("extra read"))
                .is_err()
        );
    }
}
