//! Fixed partial history authentication and factual direct-native-call inventory.
//! Success covers this interval only; no ABI semantics or state import occurs.
use crate::{
    head_history as head,
    parent_history::{
        self as parent, ParentHistoryAuthentication, ParentHistoryRows, ParentLinkContext,
    },
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};

pub const PERIODS: usize = 48;
pub const MAX_CALLS: usize = 8 + PERIODS * 6;
pub const MAX_TOTAL_BYTES: usize = head::MAX_TOTAL_BYTES + PERIODS * parent::MAX_TOTAL_BYTES;
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;

/// Complete raw fixed interval, in descending chain acquisition order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntervalHistoryRows {
    pub head: head::HeadHistoryRows,
    pub predecessors: Vec<ParentHistoryRows>,
}
/// All-period success witness. Direct calls are factual records, not key candidates.
#[derive(Debug, Serialize)]
pub struct IntervalHistoryAuthentication {
    pub first_period: u64,
    pub last_period: u64,
    pub anchor_hash_hex: String,
    pub periods: Vec<ParentHistoryAuthentication>,
    pub direct_native_calls: Vec<Value>,
    pub point_value_bytes: usize,
    pub native_address_candidates: Vec<Value>,
    pub proof_key_candidates: Vec<Value>,
    pub source_authenticated_to_local_h: bool,
    pub historical_key_completeness: bool,
    pub semantic_snapshot_complete: bool,
}
/// Verifies all 48 predecessors before exposing any interval witness or inventory.
/// Wrong order, skipped periods, broken links, invalid rows and cap breaches error.
/// The lower endpoint's unresolved predecessor is retained only in raw observations.
pub fn authenticate_interval(rows: &IntervalHistoryRows) -> Result<IntervalHistoryAuthentication> {
    ensure!(
        rows.predecessors.len() == PERIODS,
        "fixed interval count differs"
    );
    let mut total = rows.head.values().iter().try_fold(0_usize, |total, row| {
        total.checked_add(row.len()).context("H byte overflow")
    })?;
    for p in &rows.predecessors {
        for row in p.values() {
            total = total
                .checked_add(row.len())
                .context("interval byte overflow")?;
            ensure!(total <= MAX_TOTAL_BYTES, "interval total byte cap exceeded");
        }
    }
    let mut next = Some(ParentLinkContext::authenticate_successor(&rows.head)?);
    let mut witnesses = Vec::with_capacity(PERIODS);
    for (offset, rows) in rows.predecessors.iter().enumerate() {
        let context = next.take().context("premature lower endpoint")?;
        ensure!(
            context.period() == parent::P - offset as u64,
            "next period state differs"
        );
        let (witness, successor) = context.authenticate_and_advance(rows)?;
        witnesses.push(witness);
        next = successor;
    }
    ensure!(
        next.is_none() && witnesses.last().map(|p| p.period) == Some(parent::MIN_PERIOD),
        "fixed lower endpoint differs"
    );
    let calls = factual_calls(&witnesses)?;
    Ok(IntervalHistoryAuthentication {
        first_period: parent::MIN_PERIOD,
        last_period: parent::P,
        anchor_hash_hex: head::HEAD_HASH.into(),
        periods: witnesses,
        direct_native_calls: calls,
        point_value_bytes: total,
        native_address_candidates: vec![],
        proof_key_candidates: vec![],
        source_authenticated_to_local_h: true,
        historical_key_completeness: false,
        semantic_snapshot_complete: false,
    })
}
fn factual_calls(periods: &[ParentHistoryAuthentication]) -> Result<Vec<Value>> {
    let mut calls = Vec::new();
    for (period_index, period) in periods.iter().enumerate() {
        for fact in &period.position_facts {
            let receiver = fact["receiver_hex"].as_str();
            if !matches!(
                receiver,
                Some(
                    "00000000000000000000000000000000000000fe"
                        | "00000000000000000000000000000000000000ee"
                )
            ) {
                continue;
            }
            let input = fact["input_hex"]
                .as_str()
                .context("missing authenticated input")?;
            let input_bytes = hex::decode(input)?;
            ensure!(
                calls.len() < PERIODS * parent::MAX_ITEMS,
                "native inventory cap exceeded"
            );
            calls.push(json!({"period": period.period, "position": fact["position"],
                "transaction_hash_hex": fact["transaction_hash_hex"], "sender_hex": fact["sender_hex"],
                "receiver_hex": fact["receiver_hex"], "value_hex": fact["value_hex"],
                "input_hex": input, "input_bytes": input_bytes.len(), "selector_hex": input_bytes.get(..4).map(hex::encode),
                "receipt_status": fact["receipt_status"], "period_witness_index": period_index,
                "raw_transaction_field": "periods[period_witness_index].signed_transactions_hex[position]",
                "raw_receipt_and_logs_field": "periods[period_witness_index].receipts_hex[position]"}));
        }
    }
    Ok(calls)
}
/// Encodes a whole report within a fixed byte cap before an output file is created.
/// Overflow returns an error, so no truncated success witness is published.
pub fn encode_bounded(value: &Value, limit: usize) -> Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(io::Error::other("serialized output byte cap exceeded"));
            }
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: vec![],
        limit,
    };
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    Ok(output.bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn complete() -> IntervalHistoryRows {
        serde_json::from_slice(include_bytes!(
            "../tests/fixtures/interval_history_rows.json"
        ))
        .unwrap()
    }
    #[test]
    fn captured_multi_hop_interval_seals_lower_endpoint_and_has_no_native_calls() {
        let rows = complete();
        let witness = authenticate_interval(&rows).unwrap();
        assert_eq!(witness.periods.len(), PERIODS);
        assert_eq!(witness.point_value_bytes, 420457);
        assert_eq!(
            witness
                .periods
                .iter()
                .map(|p| p.position_facts.len())
                .sum::<usize>(),
            583
        );
        assert!(witness.direct_native_calls.is_empty());
        assert!(
            witness.native_address_candidates.is_empty() && witness.proof_key_candidates.is_empty()
        );
        assert!(!witness.historical_key_completeness && !witness.semantic_snapshot_complete);
        let mut next = Some(ParentLinkContext::authenticate_successor(&rows.head).unwrap());
        for (offset, rows) in rows.predecessors.iter().enumerate() {
            let context = next.take().unwrap();
            assert_eq!(context.period(), parent::P - offset as u64);
            let (_, successor) = context.authenticate_and_advance(rows).unwrap();
            next = successor;
        }
        assert!(next.is_none());
    }
    #[test]
    fn interior_final_and_order_faults_never_return_an_interval_witness() {
        for position in [1, 24, PERIODS - 1] {
            let mut changed = complete();
            changed.predecessors[position].receipts.push(0);
            assert!(authenticate_interval(&changed).is_err());
        }
        let mut changed = complete();
        changed.predecessors.swap(20, 21);
        assert!(authenticate_interval(&changed).is_err());
        let mut changed = complete();
        changed.predecessors.pop();
        assert!(authenticate_interval(&changed).is_err());
        let mut changed = complete();
        changed.predecessors.push(p());
        assert!(authenticate_interval(&changed).is_err());
        let mut changed = complete();
        changed.predecessors[24] = changed.predecessors[23].clone();
        assert!(authenticate_interval(&changed).is_err());
        let mut changed = complete();
        changed.predecessors[PERIODS - 1].system_hashes = vec![0xc0];
        assert!(authenticate_interval(&changed).is_err());
    }
    fn h() -> head::HeadHistoryRows {
        serde_json::from_slice(include_bytes!("../tests/fixtures/head_history_rows.json")).unwrap()
    }
    fn p() -> ParentHistoryRows {
        serde_json::from_slice(include_bytes!("../tests/fixtures/parent_history_rows.json"))
            .unwrap()
    }
    #[test]
    fn sealed_advance_and_incomplete_interval_cannot_publish() {
        let context = ParentLinkContext::authenticate_successor(&h()).unwrap();
        let (_, next) = context.authenticate_and_advance(&p()).unwrap();
        let context = next.unwrap();
        assert_eq!(context.period(), parent::P - 1);
        assert!(context.authenticate_and_advance(&p()).is_err()); // repeated/incorrect period
        assert!(
            authenticate_interval(&IntervalHistoryRows {
                head: h(),
                predecessors: vec![p()]
            })
            .is_err()
        );
        assert!(
            authenticate_interval(&IntervalHistoryRows {
                head: h(),
                predecessors: vec![p(); PERIODS]
            })
            .is_err()
        );
    }
    #[test]
    fn output_exact_cap_and_overflow_publish_no_partial_bytes() {
        let value = json!({"x":1});
        let bytes = encode_bounded(&value, 1024).unwrap();
        assert_eq!(encode_bounded(&value, bytes.len()).unwrap(), bytes);
        assert!(encode_bounded(&value, bytes.len() - 1).is_err());
    }
    #[test]
    fn factual_selection_includes_failed_and_short_calls_excludes_ordinary_and_creates() {
        let context = ParentLinkContext::authenticate_successor(&h()).unwrap();
        let mut witness = context.authenticate_rows(&p()).unwrap();
        let base = witness.position_facts[0].clone();
        let mut failed = base.clone();
        failed["receiver_hex"] = json!("00000000000000000000000000000000000000fe");
        failed["receipt_status"] = json!(0);
        failed["input_hex"] = json!("01");
        let mut successful = base.clone();
        successful["receiver_hex"] = json!("00000000000000000000000000000000000000ee");
        successful["input_hex"] = json!("1122334400");
        let mut create = base.clone();
        create["receiver_hex"] = Value::Null;
        create["logs"] = json!(5);
        witness.position_facts = vec![base, failed, successful, create];
        let calls = factual_calls(&[witness]).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["receipt_status"], 0);
        assert!(calls[0]["selector_hex"].is_null());
        assert_eq!(calls[1]["selector_hex"], "11223344");
    }
}
