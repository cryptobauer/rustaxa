//! Fixed staged application-only H-to-P acquisition; no state writes or scans.
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use rustaxa_snapshot_qualifier::{
    head_history as head, interval_history as interval, parent_history as parent, paths,
    qualification,
};
use rustaxa_storage::{
    FinalChainRepository, MetadataRepository, PeriodRepository, TransactionRepository,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

/// Fixed interval collector with separate per-period and aggregate counters.
#[derive(Default)]
struct PointBudget {
    calls: usize,
    returned_h: usize,
    returned_p: usize,
    accepted_h: usize,
    accepted_p: usize,
    period_returned: usize,
    failed: bool,
    observations: Vec<Value>,
}
impl PointBudget {
    fn begin_period(&mut self, period: u64) -> Result<()> {
        ensure!(
            !self.failed && (parent::MIN_PERIOD..=parent::P).contains(&period),
            "invalid next period"
        );
        ensure!(
            self.calls == 8 + ((parent::P - period) as usize) * 6,
            "point acquisition period drift"
        );
        self.period_returned = 0;
        Ok(())
    }
    fn observe(&mut self, name: &str, read: impl FnOnce() -> Result<Vec<u8>>) -> Result<Vec<u8>> {
        ensure!(
            !self.failed && self.calls < interval::MAX_CALLS,
            "sealed point-read budget before {name}"
        );
        let is_h = self.calls < 8;
        self.calls += 1;
        let value = match read() {
            Ok(value) => value,
            Err(error) => {
                self.failed = true;
                self.observations
                    .push(json!({"name":name,"read_error":format!("{error:#}")}));
                return Err(error).with_context(|| format!("read {name}"));
            }
        };
        let result = (|| -> Result<()> {
            let total = if is_h {
                &mut self.returned_h
            } else {
                &mut self.returned_p
            };
            *total = total
                .checked_add(value.len())
                .context("returned byte overflow")?;
            if is_h {
                ensure!(
                    value.len() <= head::MAX_ROW_BYTES && self.returned_h <= head::MAX_TOTAL_BYTES,
                    "H point byte cap exceeded"
                );
            } else {
                self.period_returned = self
                    .period_returned
                    .checked_add(value.len())
                    .context("period byte overflow")?;
                ensure!(
                    value.len() <= parent::MAX_ROW_BYTES
                        && self.period_returned <= parent::MAX_TOTAL_BYTES,
                    "period point byte cap exceeded"
                );
            }
            let combined = self
                .returned_h
                .checked_add(self.returned_p)
                .context("interval byte overflow")?;
            ensure!(
                combined <= interval::MAX_TOTAL_BYTES,
                "global point byte cap exceeded"
            );
            Ok(())
        })();
        self.failed = result.is_err();
        self.observations.push(
            json!({"name":name,"returned_bytes":value.len(),"accepted":!self.failed,
            "raw_hex": if self.failed { None } else { Some(hex::encode(&value)) }}),
        );
        result.with_context(|| format!("observe {name}"))?;
        if is_h {
            self.accepted_h = self.returned_h;
        } else {
            self.accepted_p = self.returned_p;
        }
        Ok(value)
    }
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: interval_history_authentication COPY OUTPUT")?,
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
    let mut stage = "H observations".to_owned();
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
        stage = "H authentication".into();
        let mut next = Some(parent::ParentLinkContext::authenticate_successor(&h)?);
        let mut predecessors = Vec::with_capacity(interval::PERIODS);
        for period in (parent::MIN_PERIOD..=parent::P).rev() {
            stage = format!("period {period} acquisition");
            budget.begin_period(period)?;
            let context = next.take().context("premature lower endpoint")?;
            ensure!(context.period() == period, "sealed next period drift");
            let mut p = parent::ParentHistoryRows {
                header: budget.observe(&format!("{period} header"), || {
                    chain
                        .block_header_raw(period)?
                        .context("missing predecessor header")
                })?,
                period_data: budget
                    .observe(&format!("{period} PeriodData"), || periods.data_raw(period))?,
                ..Default::default()
            };
            stage = format!("period {period} header and PBFT links");
            context.authenticate_header(&p.header, &p.period_data)?;
            stage = format!("period {period} indexes");
            p.hash_by_number = budget.observe(&format!("{period} hash"), || {
                chain
                    .block_hash_by_number(period)?
                    .context("missing predecessor hash")
            })?;
            context.authenticate_forward_index(&p.hash_by_number)?;
            p.number_by_hash = budget.observe(&format!("{period} number"), || {
                chain
                    .block_number_by_hash(context.expected_hash())?
                    .context("missing predecessor index")
            })?;
            context.authenticate_indexes(&p.hash_by_number, &p.number_by_hash)?;
            stage = format!("period {period} systems");
            p.system_hashes = budget.observe(&format!("{period} systems"), || {
                transactions.period_system_hashes_rlp(period)
            })?;
            ensure!(
                p.system_hashes.is_empty(),
                "system transactions unsupported"
            );
            stage = format!("period {period} receipts");
            p.receipts =
                budget.observe(&format!("{period} receipts"), || periods.receipt(period))?;
            stage = format!("period {period} full authentication");
            let (_, successor) = context.authenticate_and_advance(&p)?;
            next = successor;
            predecessors.push(p);
        }
        ensure!(
            next.is_none() && budget.calls == interval::MAX_CALLS,
            "fixed interval endpoint or count differs"
        );
        let rows = interval::IntervalHistoryRows {
            head: h,
            predecessors,
        };
        stage = "complete interval and factual inventory".into();
        let witness = interval::authenticate_interval(&rows)?;
        Ok(json!({"status":"complete_interval_lists_authenticated","witness":witness,"rows":rows}))
    })();
    let mut successful = result.is_ok();
    let authentication = match result {
        Ok(value) => value,
        Err(error) => {
            json!({"status":"interval_rejected","stage":stage,"error":format!("{error:#}")})
        }
    };
    let actual = json!({"point_calls":budget.calls,"returned_h_bytes":budget.returned_h,"returned_predecessor_bytes":budget.returned_p,
        "accepted_h_bytes":budget.accepted_h,"accepted_predecessor_bytes":budget.accepted_p});
    let report = json!({"schema":1,"input_copy":guarded.input,"open_mode":"application read_only; no state open",
        "first_period":parent::MIN_PERIOD,"last_period":parent::P,"anchor_period":head::H,
        "authentication":authentication,"actual":actual,"observations":budget.observations,
        "bounds":{"point_calls":interval::MAX_CALLS,"h_row_bytes":head::MAX_ROW_BYTES,"h_total_bytes":head::MAX_TOTAL_BYTES,
            "p_row_bytes":parent::MAX_ROW_BYTES,"p_period_bytes":parent::MAX_TOTAL_BYTES,"total_bytes":interval::MAX_TOTAL_BYTES,
            "items_per_list":parent::MAX_ITEMS,"item_bytes":parent::MAX_ITEM_BYTES,"rlp_nodes_per_row":parent::MAX_NODES,"rlp_depth":64,
            "direct_native_records":interval::PERIODS*parent::MAX_ITEMS,"serialized_output_bytes":interval::MAX_OUTPUT_BYTES,
            "native_address_candidates":0,"proof_key_candidates":0},
        "source_pins":{"adapter_source_sha256":hex::encode(Sha256::digest(include_bytes!("interval_history_authentication.rs"))),
            "verifier_source_sha256":hex::encode(Sha256::digest(include_bytes!("../interval_history.rs"))),
            "parent_source_sha256":hex::encode(Sha256::digest(include_bytes!("../parent_history.rs"))),
            "head_source_sha256":hex::encode(Sha256::digest(include_bytes!("../head_history.rs")))},
        "qualification":{"independent_public_finality":false,"historical_key_completeness":false,"semantic_snapshot_complete":false,
            "historical_owner_ready":false,"reward_transition_executed":false,"adoption_authorized":false,"production_routing_authorized":false},
        "reference_policy_assumption":"owner-supplied default mainnet, likely v1.14.1; actual producer unverified"});
    let bytes = match interval::encode_bounded(&report, interval::MAX_OUTPUT_BYTES) {
        Ok(bytes) => bytes,
        Err(error) => {
            successful = false;
            interval::encode_bounded(
                &json!({"schema":1,"actual":actual,
            "authentication":{"status":"output_cap_rejected","error":format!("{error:#}")},"serialized_output_cap":interval::MAX_OUTPUT_BYTES}),
                interval::MAX_OUTPUT_BYTES,
            )?
        }
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(guarded.output)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    ensure!(
        successful,
        "interval authentication rejected; diagnostic saved"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn period_reset_global_and_rejected_accounting_stop_before_extra_reads() {
        let mut budget = PointBudget::default();
        for _ in 0..8 {
            budget.observe("H", || Ok(vec![])).unwrap();
        }
        budget.begin_period(parent::P).unwrap();
        for i in 0..6 {
            budget
                .observe("P", || {
                    Ok(vec![0; if i < 2 { parent::MAX_ROW_BYTES } else { 0 }])
                })
                .unwrap();
        }
        assert_eq!(budget.returned_p, parent::MAX_TOTAL_BYTES);
        budget.begin_period(parent::P - 1).unwrap();
        budget.observe("next", || Ok(vec![1])).unwrap();
        assert_eq!(budget.period_returned, 1);
        assert_eq!(budget.returned_p, parent::MAX_TOTAL_BYTES + 1);
        assert!(budget.begin_period(parent::P - 2).is_err());
        let mut budget = PointBudget::default();
        budget.calls = 8;
        budget.begin_period(parent::P).unwrap();
        assert!(
            budget
                .observe("oversize", || Ok(vec![0; parent::MAX_ROW_BYTES + 1]))
                .is_err()
        );
        assert_eq!(budget.returned_p, parent::MAX_ROW_BYTES + 1);
        assert_eq!(budget.accepted_p, 0);
        assert!(budget.observe("after", || panic!("extra read")).is_err());
        let mut budget = PointBudget::default();
        budget.calls = interval::MAX_CALLS;
        assert!(budget.observe("extra", || panic!("extra read")).is_err());
        let mut budget = PointBudget::default();
        budget.calls = 8;
        budget.returned_p = interval::MAX_TOTAL_BYTES;
        assert!(budget.observe("global overflow", || Ok(vec![1])).is_err());
        assert!(
            budget
                .observe("after global", || panic!("extra read"))
                .is_err()
        );
        let mut budget = PointBudget {
            calls: interval::MAX_CALLS - 1,
            returned_h: head::MAX_TOTAL_BYTES,
            returned_p: interval::PERIODS * parent::MAX_TOTAL_BYTES - 1,
            period_returned: parent::MAX_TOTAL_BYTES - 1,
            ..Default::default()
        };
        budget
            .observe("exact global maximum", || Ok(vec![1]))
            .unwrap();
        assert_eq!(
            budget.returned_h + budget.returned_p,
            interval::MAX_TOTAL_BYTES
        );
        assert_eq!(budget.calls, interval::MAX_CALLS);
        assert!(
            budget
                .observe("after exact maximum", || panic!("extra read"))
                .is_err()
        );
    }
}
