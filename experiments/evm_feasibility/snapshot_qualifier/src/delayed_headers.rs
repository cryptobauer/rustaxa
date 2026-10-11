//! Seven header commitments linked to the fixed, revalidated local H interval.
//! No state, list-root, native-value, producer or public-finality authority follows.
use crate::{
    artifact_admission as admission, head_history as head, interval_history as interval,
    parent_history as parent,
};
use admission::{parse_unique, sha};
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_types::{
    FinalChainBlockNumber, PbftBlockMetadata, StoredFinalChainBlockHeader,
    codec::rlp::{
        final_chain::{
            LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
            StoredBlockHeaderRlpOwned,
        },
        pbft::SignedPbftBlockRlp,
    },
    pbft::PbftBlockLink,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub const FIRST: u64 = 25_706_894;
pub const LAST: u64 = 25_706_900;
pub const COUNT: usize = 7;
pub const ROW_CAP: usize = 262_144;
pub const TOTAL_CAP: usize = 3_932_200;
pub const CALL_CAP: usize = 17;
pub const OUTPUT_CAP: usize = 16 * 1024 * 1024;
pub const INPUT_PATH: &str = "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/interval-history-authentication.json";
pub const INPUT_SIZE: usize = 2_766_744;
pub const INPUT_SHA: &str = "3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5";
const PROFILE: &str = "sealed900_894_local_finalchain_and_signed_pbft";

/// Fixed admitted evidence. Fields cannot be supplied by a caller or report.
pub struct Evidence {
    rows: interval::IntervalHistoryRows,
    historical_pins: Value,
}
impl Evidence {
    /// Reads one pinned regular artifact with cap-plus-one before parsing.
    /// Duplicate keys, floats, malformed typed bytes and altered interval fail.
    /// No database is opened; embedded paths are never followed.
    pub fn load_fixed() -> Result<Self> {
        let path = Path::new(INPUT_PATH);
        ensure!(
            fs::canonicalize(path)? == path,
            "noncanonical evidence path"
        );
        let bytes = admission::evidence(path, path, INPUT_SHA, INPUT_SIZE, INPUT_SIZE)?;
        Self::decode(&bytes)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == INPUT_SIZE && sha(bytes) == INPUT_SHA,
            "evidence pin differs"
        );
        Self::decode_admitted(bytes)
    }
    fn decode_admitted(bytes: &[u8]) -> Result<Self> {
        let value = parse_unique(bytes)?;
        ensure!(
            value["schema"] == 1
                && value["authentication"]["status"] == "complete_interval_lists_authenticated"
                && value["anchor_period"] == head::H
                && value["first_period"] == parent::MIN_PERIOD
                && value["last_period"] == parent::P,
            "evidence identity differs"
        );
        let rows = serde_json::from_value(value["authentication"]["rows"].clone())?;
        let evidence = Self {
            rows,
            historical_pins: value["source_pins"].clone(),
        };
        evidence.context()?;
        Ok(evidence)
    }
    fn context(&self) -> Result<HeaderContext> {
        let witness = interval::authenticate_interval(&self.rows)?;
        ensure!(
            witness.point_value_bytes == 420_457,
            "reused byte count differs"
        );
        let boundary = &self.rows.predecessors[interval::PERIODS - 1];
        let stored = decode_header(&boundary.header)?;
        let signed = Rlp::new(&boundary.period_data).at(0)?;
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        ensure!(link.period == LAST + 1, "boundary period differs");
        Ok(HeaderContext {
            period: LAST,
            expected_final: stored.parent_hash,
            expected_pbft: link.prev_block_hash,
        })
    }
}

/// Private consuming context; only the verified 901 boundary can create it.
struct HeaderContext {
    period: u64,
    expected_final: H256,
    expected_pbft: H256,
}
fn decode_header(raw: &[u8]) -> Result<StoredFinalChainBlockHeader> {
    ensure!(
        !raw.is_empty() && raw.len() <= ROW_CAP,
        "header missing/row cap"
    );
    head::canonical_bounded(raw, parent::MAX_NODES)?;
    ensure!(
        Rlp::new(raw).is_list() && Rlp::new(raw).item_count()? == 7,
        "stored header arity differs"
    );
    let stored = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(raw))?;
    ensure!(
        StoredBlockHeaderRlpOwned::from(&stored).as_bytes() == raw,
        "stored header round trip differs"
    );
    Ok(stored)
}
impl HeaderContext {
    fn check(
        &self,
        header: &[u8],
        period_data: &[u8],
    ) -> Result<(StoredFinalChainBlockHeader, PbftBlockLink)> {
        let stored = decode_header(header)?;
        ensure!(
            !period_data.is_empty() && period_data.len() <= ROW_CAP,
            "PeriodData empty/row cap"
        );
        head::canonical_bounded(period_data, parent::MAX_NODES)?;
        let period = Rlp::new(period_data);
        ensure!(
            period.is_list() && period.item_count()? == 4,
            "PeriodData arity differs"
        );
        let txs = period.at(3)?;
        ensure!(
            txs.is_list() && txs.item_count()? <= parent::MAX_ITEMS,
            "transaction shape count"
        );
        for i in 0..txs.item_count()? {
            ensure!(
                txs.at(i)?.as_raw().len() <= parent::MAX_ITEM_BYTES,
                "transaction shape bytes"
            );
        }
        let signed = period.at(0)?;
        ensure!(
            signed.is_list() && matches!(signed.item_count()?, 8 | 9),
            "signed PBFT arity differs"
        );
        let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        ensure!(
            metadata.period == self.period && link.period == self.period,
            "signed period differs"
        );
        for (position, value) in [(4, self.period), (5, metadata.timestamp)] {
            let mut scalar = RlpStream::new();
            scalar.append(&value);
            ensure!(
                scalar.out().as_ref() == signed.at(position)?.as_raw(),
                "noncanonical signed scalar"
            );
        }
        ensure!(
            link.block_hash == self.expected_pbft,
            "signed PBFT successor link differs"
        );
        let legacy = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(StoredBlockHeaderRlp::new(header), 315_000_000, 0)
                .signed_pbft_block(SignedPbftBlockRlp::new(signed.as_raw()))
                .block_number(FinalChainBlockNumber::new(self.period)),
        )?;
        ensure!(
            legacy.hash()? == self.expected_final,
            "FinalChain successor link differs"
        );
        Ok((stored, link))
    }
    fn advance(self, header: &[u8], period_data: &[u8]) -> Result<(Value, Option<Self>)> {
        ensure!(
            (FIRST..=LAST).contains(&self.period),
            "sealed endpoint differs"
        );
        let (stored, link) = self.check(header, period_data)?;
        let row = json!({"period":self.period, "header_hex":hex::encode(header),
            "period_data_hex":hex::encode(period_data), "header_bytes":header.len(),
            "period_data_bytes":period_data.len(), "header_sha256":sha(header),
            "period_data_sha256":sha(period_data), "finalchain_hash_hex":hex::encode(self.expected_final),
            "signed_pbft_hash_hex":hex::encode(link.block_hash), "state_root_hex":hex::encode(stored.state_root),
            "successor_period":self.period+1, "successor_finalchain_predecessor_hex":hex::encode(self.expected_final),
            "successor_pbft_predecessor_hex":hex::encode(self.expected_pbft),
            "finalchain_link_checked":true, "pbft_link_checked":true,
            "unresolved_parent_finalchain_hex":hex::encode(stored.parent_hash),
            "unresolved_parent_pbft_hex":hex::encode(link.prev_block_hash)});
        let next = (self.period > FIRST).then(|| Self {
            period: self.period - 1,
            expected_final: stored.parent_hash,
            expected_pbft: link.prev_block_hash,
        });
        Ok((row, next))
    }
}

/// Exact task-oriented point read, including owner API and physical key binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Head,
    Genesis,
    Header(u64),
    PeriodData(u64),
}
impl Read {
    fn descriptor(&self) -> Value {
        let (api, column, key, period) = match self {
            Self::Head => (
                "FinalChainRepository::meta_value(1)",
                "FinalChainMeta",
                hex::encode(1_u32.to_le_bytes()),
                Some(head::H),
            ),
            Self::Genesis => (
                "MetadataRepository::genesis_hash",
                "Genesis",
                hex::encode(0_i32.to_le_bytes()), // storage::SINGLE_VALUE_KEY; owner still performs read
                None,
            ),
            Self::Header(p) => (
                "FinalChainRepository::block_header_raw",
                "FinalChainBlkByNumber",
                hex::encode(p.to_le_bytes()),
                Some(*p),
            ),
            Self::PeriodData(p) => (
                "PeriodRepository::data_raw",
                "PeriodData",
                hex::encode(p.to_le_bytes()),
                Some(*p),
            ),
        };
        json!({"api":api,"column":column,"key":key,"period":period})
    }
}
/// The application adapter provides stored rows only. None is missing; errors
/// are I/O failures, and the PeriodData owner's empty/missing ambiguity remains.
pub trait PointSource {
    fn read(&mut self, request: &Read) -> Result<Option<Vec<u8>>>;
}
#[derive(Default)]
struct Budget {
    attempted: usize,
    returned_calls: usize,
    returned: usize,
    accepted: usize,
    accepted_calls: usize,
    pair: usize,
    sealed: bool,
    observations: Vec<Value>,
    current: Option<Read>,
    unavailable: bool,
}
impl Budget {
    fn fetch(&mut self, source: &mut impl PointSource, request: Read) -> Result<Vec<u8>> {
        ensure!(
            !self.sealed && self.attempted < CALL_CAP,
            "sealed point-call cap"
        );
        self.current = Some(request.clone());
        self.attempted += 1;
        let result = source.read(&request);
        let mut observation = request.descriptor();
        match result {
            Err(error) => {
                observation["read_error"] = json!(format!("{error:#}"));
                self.observations.push(observation);
                self.sealed = true;
                Err(error)
            }
            Ok(raw) => {
                self.returned_calls += 1;
                observation["missing"] = json!(raw.is_none());
                let len = raw.as_ref().map_or(0, Vec::len);
                observation["returned_bytes"] = json!(len);
                // Oversized rejected values retain length/hash, never unbounded hex.
                observation["raw_hex"] = if len <= ROW_CAP {
                    raw.as_ref().map(hex::encode).into()
                } else {
                    Value::Null
                };
                observation["raw_sha256"] = raw.as_ref().map(|b| sha(b)).into();
                observation["accepted"] = json!(false);
                observation["capture_kind"] = json!(if len > ROW_CAP {
                    "oversized_length_and_digest_only"
                } else {
                    "complete_raw"
                });
                self.observations.push(observation);
                self.returned = self
                    .returned
                    .checked_add(len)
                    .context("returned overflow")?;
                if matches!(request, Read::Header(p) | Read::PeriodData(p) if p != head::H) {
                    self.pair = self.pair.checked_add(len).context("pair overflow")?;
                }
                if len > ROW_CAP || self.pair > 2 * ROW_CAP || self.returned > TOTAL_CAP {
                    self.sealed = true;
                    anyhow::bail!("returned row/pair/global byte cap");
                }
                if raw.is_none() || len == 0 {
                    self.unavailable = true;
                    self.sealed = true;
                    anyhow::bail!("missing/empty row; PeriodData owner maps absence to empty");
                }
                Ok(raw.unwrap())
            }
        }
    }
    fn accept(&mut self, bytes: usize) -> Result<()> {
        self.accepted = self
            .accepted
            .checked_add(bytes)
            .context("accepted overflow")?;
        self.accepted_calls += 1;
        self.observations
            .last_mut()
            .context("missing observation")?["accepted"] = json!(true);
        Ok(())
    }
}

/// Acquires exactly three guards then seven pairs. Each check must pass before
/// the next read. Any failure ends acquisition and exposes no successful prefix.
/// Returned diagnostics are bounded; only all-seven success exposes root rows.
pub fn acquire(
    evidence: &Evidence,
    source: &mut impl PointSource,
    source_pins: Value,
) -> Result<Value> {
    validate_source_pins(evidence, &source_pins)?;
    let context = evidence.context()?; // before any source read
    acquire_sealed(evidence, context, source, source_pins)
}
fn acquire_sealed(
    evidence: &Evidence,
    context: HeaderContext,
    source: &mut impl PointSource,
    source_pins: Value,
) -> Result<Value> {
    let mut next = Some(context);
    let mut budget = Budget::default();
    let mut stage = "head_identity";
    let result = (|| -> Result<Vec<Value>> {
        let raw = budget.fetch(source, Read::Head)?;
        ensure!(
            raw.len() == 8 && raw == evidence.rows.head.head && raw == head::H.to_le_bytes(),
            "head identity drift"
        );
        budget.accept(raw.len())?;
        stage = "genesis_identity";
        let raw = budget.fetch(source, Read::Genesis)?;
        ensure!(
            raw.len() == 32 && raw == evidence.rows.head.genesis,
            "genesis identity drift"
        );
        budget.accept(raw.len())?;
        stage = "h_header_identity";
        let raw = budget.fetch(source, Read::Header(head::H))?;
        ensure!(raw == evidence.rows.head.header, "H header identity drift");
        budget.accept(raw.len())?;
        let mut roots = Vec::with_capacity(COUNT);
        for period in (FIRST..=LAST).rev() {
            ensure!(
                budget.attempted == 3 + 2 * (LAST - period) as usize,
                "read-order drift"
            );
            budget.pair = 0;
            let context = next.take().context("premature sealed endpoint")?;
            ensure!(context.period == period, "sealed period drift");
            stage = "stored_header";
            let header = budget.fetch(source, Read::Header(period))?;
            decode_header(&header)?; // no PeriodData read after malformed header
            budget.accept(header.len())?;
            stage = "period_data_and_both_links";
            let data = budget.fetch(source, Read::PeriodData(period))?;
            let (witness, successor) = context.advance(&header, &data)?;
            budget.accept(data.len())?;
            roots.push(witness);
            next = successor;
        }
        ensure!(
            next.is_none() && roots.len() == COUNT && budget.attempted == CALL_CAP,
            "incomplete sealed range"
        );
        Ok(roots)
    })();
    budget.sealed = true;
    let successful = result.is_ok();
    let authentication = match result {
        Ok(roots) => {
            json!({"status":"seven_delayed_headers_authenticated_to_local_h","rows":roots,"failure":null})
        }
        Err(error) => json!({"status":if budget.unavailable {"unavailable"} else {"rejected"},
            "rows":null,"failure":{"stage":stage,"request":budget.current.as_ref().map(Read::descriptor),
                "reason":format!("{error:#}")}}),
    };
    let mut report = json!({"schema":1,"profile":PROFILE,
        "input_copy":"/workspaces/rustaxa-evm/local/evm-state-db/snapshot-litenode-copy",
        "input_evidence":{"path":INPUT_PATH,"bytes":INPUT_SIZE,"sha256":INPUT_SHA,
            "range":[parent::MIN_PERIOD,parent::P],"reused_calls":296,"reused_raw_bytes":420457},
        "source_pins":source_pins,"policy":{"owner_reference":"default-mainnet; likely v1.14.1",
            "producer_verified":false,"header_gas_limit":315000000,"chain_id":841,
            "h":head::H,"p":parent::P,"session_e":25706944,"h_certificate_d":25706942},
        "bounds":{"point_calls":CALL_CAP,"row_bytes":ROW_CAP,"pair_bytes":2*ROW_CAP,
            "returned_bytes":TOTAL_CAP,"rlp_nodes":parent::MAX_NODES,"rlp_children":parent::MAX_NODES,
            "rlp_depth":64,"transaction_items":parent::MAX_ITEMS,"transaction_item_bytes":parent::MAX_ITEM_BYTES,
            "output_bytes":OUTPUT_CAP,"trie_calls":0},
        "actual":{"attempted_calls":budget.attempted,"returned_calls":budget.returned_calls,"returned_bytes":budget.returned,
            "accepted_calls":budget.accepted_calls,"accepted_bytes":budget.accepted,"trie_calls":0},
        "authentication":authentication,"observations":budget.observations,
        "qualification":{"headers_and_roots_authenticated_to_local_h":successful,
            "independent_public_finality":false,"producer_verified":false,"state_available":false,
            "historical_key_completeness":false,"semantic_snapshot_complete":false,
            "reward_weights_qualified":false,"reward_totals_qualified":false,"reward_rates_qualified":false,
            "reward_cache_equality":false,"historical_owner_ready":false,"reward_transition_executed":false,
            "adoption_authorized":false,"production_routing_authorized":false}});
    report["content_sha256"] = json!(sha(&serde_json::to_vec(&report)?));
    interval::encode_bounded(&report, OUTPUT_CAP)?;
    Ok(report)
}

/// Replays a bounded strict capture offline. Regenerates all guards, counters,
/// raw hashes and both chain domains; edited fields or partial success fail.
/// Source pins are capture metadata; callers must freeze them independently.
pub fn replay(evidence: &Evidence, bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= OUTPUT_CAP, "capture cap");
    let value = parse_unique(bytes)?;
    validate_source_pins(evidence, &value["source_pins"])?;
    let observations = value["observations"]
        .as_array()
        .context("observations array")?;
    ensure!(observations.len() <= CALL_CAP, "observation count cap");
    struct Recorded<'a> {
        observations: &'a [Value],
        offset: usize,
        oversized: bool,
    }
    impl PointSource for Recorded<'_> {
        fn read(&mut self, request: &Read) -> Result<Option<Vec<u8>>> {
            let row = self
                .observations
                .get(self.offset)
                .context("capture ended")?;
            self.offset += 1;
            let descriptor = request.descriptor();
            for (key, expected) in descriptor.as_object().unwrap() {
                ensure!(&row[key] == expected, "captured read binding differs");
            }
            if row["capture_kind"] == "oversized_length_and_digest_only" {
                ensure!(
                    self.offset == self.observations.len(),
                    "oversized capture must end acquisition"
                );
                let len = row["returned_bytes"]
                    .as_u64()
                    .context("oversized length type")?;
                ensure!(
                    len > ROW_CAP as u64
                        && len <= usize::MAX as u64
                        && row["missing"] == false
                        && row["accepted"] == false
                        && row["raw_hex"].is_null(),
                    "oversized rejection shape"
                );
                ensure_hex(row["raw_sha256"].as_str().context("oversized digest")?, 64)?;
                self.oversized = true;
                anyhow::bail!("omitted oversized bytes; rejection metadata only");
            }
            if let Some(error) = row["read_error"].as_str() {
                anyhow::bail!("{error}");
            }
            if row["missing"] == true {
                return Ok(None);
            }
            let raw = row["raw_hex"]
                .as_str()
                .context("capture lacks bounded raw bytes")?;
            ensure!(
                raw.len() <= 2 * ROW_CAP
                    && raw
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
                "captured hex malformed"
            );
            Ok(Some(hex::decode(raw)?))
        }
    }
    let mut source = Recorded {
        observations,
        offset: 0,
        oversized: false,
    };
    let mut regenerated = acquire(evidence, &mut source, value["source_pins"].clone())?;
    if source.oversized {
        // All earlier reads were replayed exactly. Only the final rejected
        // length/digest is admitted as metadata, never as authenticated bytes.
        let last = observations.last().context("oversized observation")?;
        let index = source.offset - 1;
        let descriptor = regenerated["observations"][index].clone();
        let mut expected = descriptor;
        expected.as_object_mut().unwrap().remove("read_error");
        for key in [
            "missing",
            "returned_bytes",
            "raw_hex",
            "raw_sha256",
            "accepted",
            "capture_kind",
        ] {
            expected[key] = last[key].clone();
        }
        ensure!(&expected == last, "oversized observation keys differ");
        regenerated["observations"][index] = expected;
        let returned = regenerated["actual"]["returned_bytes"]
            .as_u64()
            .unwrap()
            .checked_add(last["returned_bytes"].as_u64().unwrap())
            .context("oversized counter overflow")?;
        regenerated["actual"]["returned_bytes"] = json!(returned);
        regenerated["actual"]["returned_calls"] =
            json!(regenerated["actual"]["returned_calls"].as_u64().unwrap() + 1);
        regenerated["authentication"]["failure"]["reason"] =
            json!("returned row/pair/global byte cap");
        regenerated
            .as_object_mut()
            .unwrap()
            .remove("content_sha256");
        regenerated["content_sha256"] = json!(sha(&serde_json::to_vec(&regenerated)?));
    }
    ensure!(
        source.offset == observations.len() && regenerated == value,
        "capture replay differs"
    );
    Ok(regenerated)
}

/// Current compiled dependencies and executable, separate from historical pins.
pub fn source_pins(evidence: &Evidence) -> Result<Value> {
    let executable = std::env::current_exe()?;
    let bytes = admission::read_capped(fs::File::open(&executable)?, 256 * 1024 * 1024)?;
    Ok(json!({"historical_capture":evidence.historical_pins,
        "current_compiled":compiled_pins(),
        "executable":{"path":executable,"bytes":bytes.len(),"sha256":sha(&bytes)}}))
}
fn compiled_pins() -> Value {
    macro_rules! pin { ($p:literal) => {{ let b = include_bytes!($p); json!({"path":$p,"bytes":b.len(),"sha256":sha(b)}) }}; }
    json!([
        pin!("delayed_headers.rs"),
        pin!("bin/delayed_headers.rs"),
        pin!("bin/parent_cache_fields/admission.rs"),
        pin!("interval_history.rs"),
        pin!("parent_history.rs"),
        pin!("head_history.rs"),
        pin!("paths.rs"),
        pin!("qualification.rs"),
        pin!("../Cargo.lock"),
        pin!("../../../../rust/crates/rustaxa-types/src/codec/rlp/pbft.rs"),
        pin!("../../../../rust/crates/rustaxa-types/src/codec/rlp/final_chain.rs"),
        pin!("../../../../rust/crates/rustaxa-types/src/pbft.rs"),
        pin!("../../../../rust/crates/rustaxa-types/src/final_chain.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/final_chain.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/period.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/metadata.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/lib.rs")
    ])
}
fn ensure_hex(hex: &str, len: usize) -> Result<()> {
    ensure!(
        hex.len() == len
            && hex
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "fixed lowercase hex differs"
    );
    Ok(())
}
fn exact_keys(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().context("object type")?;
    ensure!(
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)),
        "fixed keys differ"
    );
    Ok(())
}
fn validate_source_pins(evidence: &Evidence, pins: &Value) -> Result<()> {
    exact_keys(
        pins,
        &["historical_capture", "current_compiled", "executable"],
    )?;
    ensure!(
        pins["historical_capture"] == evidence.historical_pins
            && pins["current_compiled"] == compiled_pins(),
        "compiled/historical pins differ"
    );
    let executable = &pins["executable"];
    exact_keys(executable, &["path", "bytes", "sha256"])?;
    let path = executable["path"].as_str().context("executable path")?;
    ensure!(
        Path::new(path).is_absolute(),
        "executable path must be absolute metadata"
    );
    let len = executable["bytes"].as_u64().context("executable size")?;
    ensure!(len > 0 && len <= 256 * 1024 * 1024, "executable cap");
    ensure_hex(
        executable["sha256"].as_str().context("executable digest")?,
        64,
    )
}
/// Encode completely before an exclusive synced publication. Existing output
/// survives; temporary files are removed on both success and failure.
pub fn publish(output: &Path, value: &Value) -> Result<()> {
    admission::outside_db(&fs::canonicalize(
        output.parent().context("output parent")?,
    )?)?;
    admission::publish_exclusive(output, &interval::encode_bounded(value, OUTPUT_CAP)?)
}

#[cfg(test)]
#[path = "delayed_headers_tests.rs"]
mod tests;
