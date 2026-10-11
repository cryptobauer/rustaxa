//! Pure locally authenticated parent reward fields and exact missing-input manifest.
//! No weights, rates, minimum-difficulty policy or reward runtime is supplied.
use crate::{
    head_history::{canonical_bounded, receipt_bounded},
    interval_history::{self, IntervalHistoryRows},
    reward_votes,
};
use anyhow::{Context, Result, ensure};
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use rustaxa_consensus::pbft_vote_validation::{
    PbftCanonicalVoteInspectionStatus, inspect_canonical_pbft_vote,
};
use rustaxa_types::{
    DagBlock, LegacyTransactionEnvelope, PbftBlockMetadata, StoredFinalChainBlockHeader,
    codec::rlp::{
        dag::{DagBlockRlp, FinalizedDagBlockBundleRlp},
        final_chain::StoredBlockHeaderRlp,
        pbft::SignedPbftBlockRlp,
    },
    pbft::PbftBlockLink,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use tiny_keccak::{Hasher, Keccak};
const FIRST: u64 = 25_706_901;
const LAST: u64 = 25_706_948;
/// Expected output metadata only. Raw cache values never enter field calculation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheExpectedRow {
    pub period: u64,
    pub bytes: u64,
    pub sha256: String,
}
/// Full49-row retained metadata; H is admitted then excluded from parent comparison.
#[derive(Clone, Debug)]
pub struct ParentCacheMetadata {
    pub rows: Vec<CacheExpectedRow>,
}
/// Partial signed field evidence and null missing facts; no runtime or owner token.
#[derive(Debug)]
pub struct ParentRewardFieldReport {
    pub value: Value,
}
fn hash(b: &[u8]) -> H256 {
    let mut h = Keccak::v256();
    h.update(b);
    let mut out = [0; 32];
    h.finalize(&mut out);
    H256(out)
}
fn hx(b: impl AsRef<[u8]>) -> String {
    hex::encode(b)
}
/// Ordered-map compact JSON preserves arrays; floats are excluded by this schema.
pub fn normalized_digest(v: &Value) -> Result<String> {
    ensure_no_floats(v)?;
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(
        &sorted_value(v),
    )?)))
}
fn sorted_value(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = serde_json::Map::new();
            for (k, v) in m.iter().collect::<BTreeMap<_, _>>() {
                out.insert(k.clone(), sorted_value(v));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted_value).collect()),
        _ => v.clone(),
    }
}
fn ensure_no_floats(v: &Value) -> Result<()> {
    match v {
        Value::Number(n) => ensure!(!n.is_f64(), "float outside digest schema"),
        Value::Array(a) => {
            for v in a {
                ensure_no_floats(v)?
            }
        }
        Value::Object(m) => {
            for v in m.values() {
                ensure_no_floats(v)?
            }
        }
        _ => {}
    }
    Ok(())
}
fn scalar_u64(rlp: &Rlp<'_>, index: usize) -> Result<u64> {
    let n: u64 = rlp.val_at(index)?;
    let mut r = RlpStream::new();
    r.append(&n);
    ensure!(
        r.out().as_ref() == rlp.at(index)?.as_raw(),
        "noncanonical integer"
    );
    Ok(n)
}
fn scalar_u16(rlp: &Rlp<'_>, index: usize) -> Result<u16> {
    let n: u16 = rlp.val_at(index)?;
    let mut r = RlpStream::new();
    r.append(&n);
    ensure!(
        r.out().as_ref() == rlp.at(index)?.as_raw(),
        "noncanonical difficulty"
    );
    Ok(n)
}
/// C++ calculateOrderHash uses one outer list containing the ordered hash list.
fn rlp_size(payload: usize) -> Result<usize> {
    let overhead = if payload <= 55 {
        1
    } else {
        1 + ((usize::BITS - payload.leading_zeros()) as usize).div_ceil(8)
    };
    payload
        .checked_add(overhead)
        .context("projected RLP byte overflow")
}
fn order_hash(hashes: &[H256]) -> H256 {
    if hashes.is_empty() {
        H256::zero()
    } else {
        let mut r = RlpStream::new_list(1);
        r.append_list(hashes);
        hash(&r.out())
    }
}
type DagInputs = Vec<(H160, Vec<H256>)>;
fn dags(period: &Rlp<'_>, signed: &Rlp<'_>) -> Result<(Vec<Value>, DagInputs)> {
    let bundle = period.at(2)?;
    let commitment: H256 = signed.val_at(2)?;
    let pivot: H256 = signed.val_at(1)?;
    if bundle.as_raw() == [0x80] {
        ensure!(
            commitment.is_zero() && pivot.is_zero(),
            "empty DAG commitment/pivot mismatch"
        );
        return Ok((vec![], vec![]));
    }
    ensure!(
        bundle.is_list() && bundle.item_count()? == 3,
        "DAG bundle arity"
    );
    let hashes = bundle.at(0)?;
    let indexes = bundle.at(1)?;
    let compact = bundle.at(2)?;
    let count = compact.item_count()?;
    ensure!(
        count <= 10 && indexes.item_count()? == count,
        "DAG count/index cap"
    );
    ensure!(
        hashes.item_count()? <= 256,
        "DAG transaction dictionary cap"
    );
    for h in hashes.iter() {
        let _: H256 = h.as_val()?;
    }
    let mut facts = Vec::new();
    let mut inputs = Vec::new();
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();
    for position in 0..count {
        let index_list = indexes.at(position)?;
        ensure!(
            index_list.is_list() && index_list.item_count()? <= 256,
            "DAG indexes cap"
        );
        for j in 0..index_list.item_count()? {
            let index = scalar_u64(&index_list, j)?;
            ensure!(index < hashes.item_count()? as u64, "DAG index range");
        }
        let source = compact.at(position)?;
        ensure!(
            source.item_count()? == 7 && source.as_raw().len() <= 16384,
            "compact DAG source cap/shape"
        );
        let mut tx_payload = 0usize;
        for j in 0..index_list.item_count()? {
            let n = scalar_u64(&index_list, j)?;
            tx_payload = tx_payload
                .checked_add(hashes.at(usize::try_from(n)?)?.as_raw().len())
                .context("expanded tx bytes overflow")?;
        }
        let mut projected = rlp_size(tx_payload)?;
        for field in 0..7 {
            projected = projected
                .checked_add(source.at(field)?.as_raw().len())
                .context("expanded DAG bytes overflow")?;
        }
        ensure!(rlp_size(projected)? <= 16384, "projected DAG item byte cap");
        let raw = FinalizedDagBlockBundleRlp::new(bundle.as_raw()).canonical_block_rlp(position)?;
        ensure!(raw.len() <= 16384, "DAG item cap");
        canonical_bounded(&raw, 32768)?;
        let rlp = Rlp::new(&raw);
        ensure!(rlp.item_count()? == 8, "DAG canonical arity");
        let block = DagBlock::try_from(DagBlockRlp::new(&raw))?;
        let author = block.recover_sender().context("DAG signature recovery")?;
        let mut canonical = RlpStream::new_list(8);
        canonical
            .append(&block.pivot)
            .append(&block.level)
            .append(&block.timestamp)
            .append(&block.vdf)
            .append_list(&block.tips)
            .append_list(&block.transactions)
            .append(&block.signature.as_slice())
            .append(&block.gas_estimation);
        ensure!(
            canonical.out().as_ref() == raw,
            "DAG scalar re-encoding mismatch"
        );
        canonical_bounded(&block.vdf, 32768)?;
        let vdf = Rlp::new(&block.vdf);
        ensure!(vdf.is_list() && vdf.item_count()? == 4, "VDF shape");
        let difficulty = scalar_u16(&vdf, 3)?;
        let signed_hash = hash(&raw);
        ensure!(seen.insert(signed_hash), "duplicate DAG hash");
        ordered.push(signed_hash);
        facts.push(json!({"position":position,"signed_hash_hex":hx(signed_hash),"signing_hash_hex":hx(block.signing_hash()),"author_hex":hx(author),"raw_difficulty":difficulty,"raw_difficulty_rlp_hex":hx(vdf.at(3)?.as_raw()),"vdf_hex":hx(&block.vdf),"vdf_validity_qualified":false,"transaction_hashes_hex":block.transactions.iter().map(hx).collect::<Vec<_>>()}));
        inputs.push((author, block.transactions));
    }
    ensure!(
        order_hash(&ordered) == commitment,
        "signed DAG order commitment mismatch"
    );
    ensure!(
        !pivot.is_zero() && ordered.contains(&pivot),
        "signed DAG pivot missing"
    );
    Ok((facts, inputs))
}
fn certs(period_data: &[u8], signed: &Rlp<'_>, s: u64) -> Result<Value> {
    let period = Rlp::new(period_data);
    let bundle = period.at(1)?;
    ensure!(
        bundle.is_list() && bundle.item_count()? == 5,
        "certificate bundle shape"
    );
    for i in 1..=3 {
        scalar_u64(&bundle, i)?;
    }
    let count = bundle.at(4)?.item_count()?;
    ensure!(count > 0 && count <= 20, "certificate cap");
    let optimized = bundle.at(4)?;
    for vote in optimized.iter() {
        ensure!(
            vote.item_count()? == 2 && vote.as_raw().len() <= 16384,
            "optimized vote source byte cap/shape"
        );
        let proof = vote.at(0)?;
        let signature = vote.at(1)?;
        ensure!(signature.data()?.len() == 65, "source vote signature width");
        proof.data()?;
        let mut sortition_payload = proof.as_raw().len();
        for field in 1..=3 {
            sortition_payload = sortition_payload
                .checked_add(bundle.at(field)?.as_raw().len())
                .context("sortition projected overflow")?;
        }
        let sortition_size = rlp_size(sortition_payload)?;
        let sortition_as_data_size = rlp_size(sortition_size)?;
        let projected = 33usize
            .checked_add(sortition_as_data_size)
            .and_then(|n| n.checked_add(signature.as_raw().len()))
            .context("canonical vote projected overflow")?;
        ensure!(
            rlp_size(projected)? <= 16384,
            "projected canonical vote byte cap"
        );
    }
    let reconstructed = reward_votes::reconstruct_cert_votes(period_data)?;
    ensure!(
        reconstructed.certified_period.checked_add(1) == Some(s) && reconstructed.step == 3,
        "certificate period/step mismatch"
    );
    let previous: H256 = signed.val_at(0)?;
    let reward_hashes: Vec<H256> = signed.list_at(6)?;
    ensure!(
        reward_hashes.len() == count
            && reward_hashes.iter().copied().collect::<BTreeSet<_>>().len() == count,
        "signed reward hashes count/duplicate"
    );
    let mut seen_hash = BTreeSet::new();
    let mut seen_voter = BTreeSet::new();
    let mut votes = Vec::new();
    let mut wire = Vec::new();
    for (position, raw) in reconstructed.canonical.iter().enumerate() {
        ensure!(raw.len() <= 16384, "vote byte cap");
        canonical_bounded(raw, 32768)?;
        let rlp = Rlp::new(raw);
        ensure!(rlp.item_count()? == 3, "vote arity/weight");
        let nested: Vec<u8> = rlp.val_at(1)?;
        canonical_bounded(&nested, 32768)?;
        let sortition = Rlp::new(&nested);
        ensure!(sortition.item_count()? == 4, "sortition shape");
        for i in 0..3 {
            scalar_u64(&sortition, i)?;
        }
        let inspection = inspect_canonical_pbft_vote(raw)?;
        ensure!(
            inspection.status == PbftCanonicalVoteInspectionStatus::Valid
                && inspection.signature_valid
                && !inspection.has_embedded_weight,
            "vote signature/status/weight"
        );
        ensure!(
            inspection.period == reconstructed.certified_period
                && inspection.round == reconstructed.round
                && inspection.step == 3
                && inspection.block_hash == previous,
            "vote identity mismatch"
        );
        let mut encoded = RlpStream::new_list(3);
        encoded
            .append(&inspection.block_hash)
            .append(&nested)
            .append(&rlp.at(2)?.data()?);
        ensure!(
            encoded.out().as_ref() == raw,
            "vote canonical re-encoding mismatch"
        );
        ensure!(
            seen_hash.insert(inspection.vote_hash) && seen_voter.insert(inspection.recovered_voter),
            "duplicate vote hash/voter"
        );
        wire.push(inspection.vote_hash);
        votes.push(json!({"position":position,"vote_hash_hex":hx(inspection.vote_hash),"signing_hash_hex":hx(inspection.signing_hash),"voter_hex":hx(inspection.recovered_voter),"raw_hex":hx(raw),"signature_valid":true,"weight":null,"weight_qualified":false}));
    }
    ensure!(
        seen_hash == reward_hashes.iter().copied().collect(),
        "signed certificate reward-hash set mismatch"
    );
    let q = reconstructed
        .certified_period
        .checked_sub(1)
        .context("query period underflow")?;
    let delayed = q.checked_sub(5).context("delayed period underflow")?;
    Ok(
        json!({"certified_period":reconstructed.certified_period,"round":reconstructed.round,"step":3,"block_hash_hex":hx(previous),"predecessor_body_retained":s>FIRST,"query_period":q,"delayed_period":delayed,"votes":votes,"wire_vote_hashes_hex":wire.iter().map(hx).collect::<Vec<_>>(),"signed_reward_hashes_hex":reward_hashes.iter().map(hx).collect::<Vec<_>>(),"wire_order_equals_signed_order":wire==reward_hashes,"wire_order_digest":normalized_digest(&json!(wire.iter().map(hx).collect::<Vec<_>>()))?}),
    )
}
fn transactions(
    period_data: &[u8],
    receipts: &[u8],
    header: &StoredFinalChainBlockHeader,
    dag_inputs: &[(H160, Vec<H256>)],
) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>)> {
    let txs = Rlp::new(period_data).at(3)?;
    let receipts = Rlp::new(receipts);
    ensure!(
        txs.item_count()? == receipts.item_count()? && txs.item_count()? <= 256,
        "transaction/receipt count"
    );
    let mut facts = Vec::new();
    let mut by_hash = BTreeMap::new();
    let mut gas = 0u64;
    for (position, (tx, receipt)) in txs.iter().zip(receipts.iter()).enumerate() {
        let tx = LegacyTransactionEnvelope::decode(tx.as_raw())?;
        ensure!(
            tx.signature_valid && tx.sender.is_some() && tx.chain_id == 841,
            "transaction signature/chain"
        );
        let (_, used, cumulative, _) = receipt_bounded(receipt.as_raw(), 32768)?;
        gas = gas.checked_add(used).context("gas sum overflow")?;
        ensure!(gas == cumulative, "cumulative receipt gas mismatch");
        let fee = tx
            .gas_price
            .checked_mul(U256::from(used))
            .context("transaction fee overflow")?;
        ensure!(
            by_hash.insert(tx.hash, (position, fee)).is_none(),
            "duplicate transaction hash"
        );
        facts.push(json!({"position":position,"hash_hex":hx(tx.hash),"gas_price":tx.gas_price.to_string(),"gas_used":used,"cumulative_gas":cumulative,"receipt_hash_hex":hx(hash(receipt.as_raw())),"fee":fee.to_string(),"first_dag_owner":null,"assignment":"unassigned"}));
    }
    ensure!(gas == header.gas_used.as_u64(), "header gas used mismatch");
    let mut consumed = BTreeSet::new();
    let mut authors = BTreeMap::<H160, U256>::new();
    let mut ignored = Vec::new();
    for (dpos, (author, hashes)) in dag_inputs.iter().enumerate() {
        for (tpos, hash) in hashes.iter().enumerate() {
            if let Some((position, fee)) = by_hash.get(hash) {
                if consumed.insert(*hash) {
                    facts[*position]["first_dag_owner"] =
                        json!({"dag_position":dpos,"author_hex":hx(author)});
                    facts[*position]["assignment"] = json!("assigned");
                    let total = authors.entry(*author).or_default();
                    *total = total.checked_add(*fee).context("author fee overflow")?;
                }
            } else {
                ignored.push(
                    json!({"dag_position":dpos,"transaction_position":tpos,"hash_hex":hx(hash)}),
                );
            }
        }
    }
    Ok((
        facts,
        authors
            .into_iter()
            .map(|(a, f)| json!({"author_hex":hx(a),"fee":f.to_string()}))
            .collect(),
        ignored,
    ))
}
/// Rebuilds all local-chain witnesses and derives only independently available fields.
/// Every signature/order/count failure returns no report. No expected output enters.
pub fn derive_available_fields(rows: &IntervalHistoryRows) -> Result<Value> {
    let witness = interval_history::authenticate_interval(rows)?;
    ensure!(
        witness.point_value_bytes == 420457,
        "fixed decoded bytes drift"
    );
    let mut periods = Vec::new();
    let mut count_votes = 0usize;
    let mut count_dags = 0usize;
    let mut count_txs = 0usize;
    let mut roots = BTreeMap::new();
    for (raw, w) in rows.predecessors.iter().zip(&witness.periods) {
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw.header))?;
        roots.insert(w.period,json!({"period":w.period,"header_hash_hex":w.header_hash_hex,"state_root_hex":hx(header.state_root),"header_sha256":hx(Sha256::digest(&raw.header)),"source_authenticated_to_local_h":true}));
        let period = Rlp::new(&raw.period_data);
        let signed = period.at(0)?;
        let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        let (dags, dag_inputs) = dags(&period, &signed)?;
        count_dags = count_dags
            .checked_add(dags.len())
            .context("DAG total overflow")?;
        ensure!(count_dags <= 139, "DAG total cap");
        let cert = certs(&raw.period_data, &signed, w.period)?;
        count_votes = count_votes
            .checked_add(cert["votes"].as_array().context("votes")?.len())
            .context("vote total overflow")?;
        ensure!(count_votes <= 914, "vote total cap");
        let (txs, fees, ignored) =
            transactions(&raw.period_data, &raw.receipts, &header, &dag_inputs)?;
        count_txs = count_txs
            .checked_add(txs.len())
            .context("transaction total overflow")?;
        ensure!(count_txs <= 583, "transaction total cap");
        periods.push(json!({"period":w.period,"header_hash_hex":w.header_hash_hex,"pbft_signed_hash_hex":hx(link.block_hash),"pbft_recovered_author_hex":hx(metadata.author),"pbft_previous_hash_hex":hx(link.prev_block_hash),"pbft_pivot_hash_hex":hx(link.pivot_dag_block_hash),"source_period_data_sha256":hx(Sha256::digest(&raw.period_data)),"source_receipts_sha256":hx(Sha256::digest(&raw.receipts)),"minimum_difficulty_unqualified":true,"dags":dags,"certificate":cert,"transactions":txs,"author_fees":fees,"ignored_nonfinalized_transactions":ignored}));
    }
    ensure!(
        count_votes == 914 && count_dags == 139 && count_txs == 583,
        "fixed aggregate counts drift"
    );
    periods.reverse();
    let mut delayed = BTreeMap::<u64, Value>::new();
    let mut voter_keys = BTreeMap::<(u64, String, String), Vec<Value>>::new();
    for period in &periods {
        let d = period["certificate"]["delayed_period"]
            .as_u64()
            .context("D")?;
        ensure!(
            (FIRST - 7..=LAST - 7).contains(&d),
            "delayed identity outside exact interval"
        );
        delayed.entry(d).or_insert_with(||json!({"delayed_period":d,"root_source":roots.get(&d),"root_unresolved_reason":if roots.contains_key(&d){None}else{Some("historical_header_missing")},"total_votes_requirement":{"account":"0xfe","logical_key":"04","value":null,"qualified":false},"jailed_list_requirement":{"account":"0xee","logical_key":"leftpad32(02)","value":null,"qualified":false},"nonempty_jail_total_correction":"requires separate reviewed correction facts"}));
        for vote in period["certificate"]["votes"].as_array().context("votes")? {
            let voter = vote["voter_hex"].as_str().context("voter")?.to_owned();
            for key in [
                "fe_validator_00_00_address",
                "fe_vrf_00_04_address",
                "ee_jail_00_address",
            ] {
                voter_keys
                    .entry((d, voter.clone(), key.to_owned()))
                    .or_default()
                    .push(json!({"period":period["period"],"vote_hash_hex":vote["vote_hash_hex"]}));
            }
        }
    }
    ensure!(
        delayed.len() == 48 && voter_keys.len() <= 3 * 914,
        "missing manifest bounds"
    );
    let missing = json!({"delayed_roots_and_totals":delayed.into_values().collect::<Vec<_>>(),"voter_keys":voter_keys.into_iter().map(|((d,a,k),refs)|json!({"delayed_period":d,"voter_hex":a,"semantic_key":k,"source_references":refs,"value":null,"proof":null,"qualified":false})).collect::<Vec<_>>(),"period_lambda":periods.iter().map(|p|json!({"period":p["period"],"status":"period_lambda_selection_missing","selected_raw_key":null,"selected_raw_value":null,"selected_update_period":null,"nearest_at_or_before_provenance":null,"policy_id":null,"source_classification":null,"rate":null,"qualified":false})).collect::<Vec<_>>(),"dag_reward_difficulty_policy":{"status":"dag_reward_difficulty_policy_unqualified","threshold":null,"predicate":null,"source_pin":null}});
    Ok(
        json!({"status":"available_fields_only_missing_inputs","periods":periods,"missing_inputs":missing,"actual":{"parent_periods":48,"certificate_entries":count_votes,"dag_blocks":count_dags,"transactions":count_txs,"decoded_raw_bytes":witness.point_value_bytes},"qualification":{"regenerated_local_chain_authenticated":true,"dag_commitments_and_signatures":true,"certificate_signature_and_hash_binding":true,"partial_fields_derived":true,"minimum_difficulty_unqualified":true,"vote_weights_qualified":false,"vote_totals_qualified":false,"rate_inputs_qualified":false,"typed_cache_equal":false,"raw_cache_equal":false,"historical_cache_authority":false,"semantic_snapshot_complete":false,"historical_key_completeness":false,"owner_authority":false,"reward_transition_executed":false,"production_routing_authorized":false,"adoption_authorized":false,"independent_public_finality":false}}),
    )
}
/// Attaches admitted expected hashes after available calculation; no typed cache equality.
pub fn derive_parent_reward_fields(
    rows: &IntervalHistoryRows,
    metadata: &ParentCacheMetadata,
) -> Result<ParentRewardFieldReport> {
    let mut value = derive_available_fields(rows)?;
    ensure!(metadata.rows.len() == 49, "cache metadata count");
    let mut total = 0u64;
    for (i, row) in metadata.rows.iter().enumerate() {
        ensure!(
            row.period == FIRST + i as u64
                && row.bytes <= 1024 * 1024
                && row.sha256.len() == 64
                && row
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "cache metadata identity/order/hash"
        );
        total = total
            .checked_add(row.bytes)
            .context("cache metadata byte overflow")?;
    }
    let key_bytes = (metadata.rows.len() as u64)
        .checked_mul(8)
        .context("cache key byte overflow")?;
    ensure!(
        total == 26614
            && key_bytes == 392
            && total.checked_add(key_bytes) == Some(27006)
            && metadata.rows[48].bytes == 566,
        "all cache metadata key/value bytes"
    );
    let parents = &metadata.rows[..48];
    ensure!(
        parents.iter().map(|r| r.bytes).sum::<u64>() == 26048,
        "parent cache metadata bytes"
    );
    value["cache_metadata_bytes"] = json!({"all_value_bytes":26614,"all_key_bytes":392,"all_key_and_value_bytes":27006,"parent_value_bytes":26048,"parent_key_bytes":384,"parent_key_and_value_bytes":26432,"head_value_bytes":566});
    value["missing_inputs"]["expected_cache_rows"]=json!(parents.iter().map(|r|json!({"period":r.period,"expected_bytes":r.bytes,"expected_sha256":r.sha256,"raw_expected_rlp_missing":true,"qualified":false})).collect::<Vec<_>>());
    value["digests"] = json!({"available_fields_digest":normalized_digest(&value["periods"] )?,"missing_manifest_digest":normalized_digest(&value["missing_inputs"])?});
    Ok(ParentRewardFieldReport { value })
}
/// Enforces the narrowed output schema before digests and publication.
/// Rejects extra authority/root fields, removed reward predicates and filled gaps.
pub fn validate_field_report(v: &Value) -> Result<()> {
    let top = [
        "status",
        "periods",
        "missing_inputs",
        "actual",
        "qualification",
        "cache_metadata_bytes",
        "digests",
        "schema",
        "scope",
        "base_commit",
        "source_manifest",
        "inputs",
        "reference_policy",
        "bounds",
        "runtime",
        "implementation_pins",
    ];
    let m = v.as_object().context("report object")?;
    ensure!(
        m.keys().all(|k| top.contains(&k.as_str())),
        "unknown report key/root"
    );
    for k in &top[..7] {
        ensure!(m.contains_key(*k), "missing core report key");
    }
    ensure!(
        v["status"] == "available_fields_only_missing_inputs",
        "report status"
    );
    let flags = [
        ("regenerated_local_chain_authenticated", true),
        ("dag_commitments_and_signatures", true),
        ("certificate_signature_and_hash_binding", true),
        ("partial_fields_derived", true),
        ("minimum_difficulty_unqualified", true),
        ("vote_weights_qualified", false),
        ("vote_totals_qualified", false),
        ("rate_inputs_qualified", false),
        ("typed_cache_equal", false),
        ("raw_cache_equal", false),
        ("historical_cache_authority", false),
        ("semantic_snapshot_complete", false),
        ("historical_key_completeness", false),
        ("owner_authority", false),
        ("reward_transition_executed", false),
        ("production_routing_authorized", false),
        ("adoption_authorized", false),
        ("independent_public_finality", false),
    ];
    ensure!(
        v["qualification"]
            .as_object()
            .context("qualification")?
            .len()
            == flags.len(),
        "extra/missing authority flag"
    );
    for (k, b) in flags {
        ensure!(v["qualification"][k] == b, "authority flag mismatch");
    }
    fn no_removed_fields(v: &Value) -> Result<()> {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    ensure!(
                        ![
                            "minimum_difficulty",
                            "is_minimum_difficulty",
                            "reward_eligibility",
                            "dag_reward_counters",
                            "dag_reward_counter",
                            "per_author_dag_reward_counters",
                            "expected_root",
                            "expected_state_root",
                            "injected_expected_state_root"
                        ]
                        .contains(&k.as_str()),
                        "removed reward/root field"
                    );
                    no_removed_fields(v)?;
                }
            }
            Value::Array(a) => {
                for v in a {
                    no_removed_fields(v)?
                }
            }
            _ => {}
        }
        Ok(())
    }
    no_removed_fields(v)?;
    let periods = v["periods"].as_array().context("periods")?;
    ensure!(periods.len() == 48, "output period count");
    for (i, p) in periods.iter().enumerate() {
        ensure!(
            p["period"] == FIRST + i as u64 && p["minimum_difficulty_unqualified"] == true,
            "output period identity/policy"
        );
    }
    let missing = &v["missing_inputs"];
    let roots = missing["delayed_roots_and_totals"]
        .as_array()
        .context("missing roots")?;
    ensure!(roots.len() == 48, "missing root count");
    for (i, r) in roots.iter().enumerate() {
        let d = FIRST - 7 + i as u64;
        ensure!(r["delayed_period"] == d, "missing root identity");
        if d < FIRST {
            ensure!(
                r["root_source"].is_null()
                    && r["root_unresolved_reason"] == "historical_header_missing",
                "missing root filled"
            );
        } else {
            ensure!(
                r["root_source"]["period"] == d,
                "known delayed root identity"
            );
        }
        for k in ["total_votes_requirement", "jailed_list_requirement"] {
            ensure!(
                r[k]["value"].is_null() && r[k]["qualified"] == false,
                "missing total/list filled"
            );
        }
    }
    let voters = missing["voter_keys"].as_array().context("missing voters")?;
    ensure!(voters.len() <= 2742, "voter requirements cap");
    for row in voters {
        ensure!(
            row["value"].is_null() && row["proof"].is_null() && row["qualified"] == false,
            "missing voter filled"
        );
    }
    let lambdas = missing["period_lambda"]
        .as_array()
        .context("missing lambdas")?;
    ensure!(lambdas.len() == 48, "missing lambda count");
    for (i, row) in lambdas.iter().enumerate() {
        ensure!(
            row["period"] == FIRST + i as u64
                && row["status"] == "period_lambda_selection_missing"
                && row["qualified"] == false,
            "lambda identity/classification"
        );
        for k in [
            "selected_raw_key",
            "selected_raw_value",
            "selected_update_period",
            "nearest_at_or_before_provenance",
            "policy_id",
            "source_classification",
            "rate",
        ] {
            ensure!(
                row.get(k).is_some_and(Value::is_null),
                "missing lambda field filled/omitted"
            );
        }
    }
    let cache = missing["expected_cache_rows"]
        .as_array()
        .context("missing cache rows")?;
    ensure!(cache.len() == 48, "expected cache count");
    for (i, row) in cache.iter().enumerate() {
        ensure!(
            row["period"] == FIRST + i as u64
                && row["raw_expected_rlp_missing"] == true
                && row["qualified"] == false,
            "expected raw cache claimed"
        );
    }
    let policy = &missing["dag_reward_difficulty_policy"];
    ensure!(
        policy["status"] == "dag_reward_difficulty_policy_unqualified"
            && policy["threshold"].is_null()
            && policy["predicate"].is_null()
            && policy["source_pin"].is_null(),
        "difficulty policy filled"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rows() -> IntervalHistoryRows {
        serde_json::from_slice(include_bytes!(
            "../tests/fixtures/interval_history_rows.json"
        ))
        .unwrap()
    }
    fn replace(raw: &[u8], index: usize, value: &[u8]) -> Vec<u8> {
        let r = Rlp::new(raw);
        let mut out = RlpStream::new_list(r.item_count().unwrap());
        for i in 0..r.item_count().unwrap() {
            out.append_raw(
                if i == index {
                    value
                } else {
                    r.at(i).unwrap().as_raw()
                },
                1,
            );
        }
        out.out().to_vec()
    }
    #[test]
    fn real_signed_fields_and_missing_inputs_are_separate() {
        let result = derive_available_fields(&rows()).unwrap();
        assert_eq!(result["actual"]["certificate_entries"], 914);
        assert_eq!(result["actual"]["dag_blocks"], 139);
        assert_eq!(result["periods"].as_array().unwrap().len(), 48);
        assert_eq!(
            result["missing_inputs"]["delayed_roots_and_totals"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["root_source"].is_null())
                .count(),
            7
        );
        assert_eq!(
            result["missing_inputs"]["period_lambda"]
                .as_array()
                .unwrap()
                .len(),
            48
        );
        assert_eq!(
            result["qualification"]["minimum_difficulty_unqualified"],
            true
        );
        assert_eq!(result["qualification"]["vote_weights_qualified"], false);
        for p in result["periods"].as_array().unwrap() {
            assert!(p.get("dag_reward_counters").is_none());
            assert_eq!(p["minimum_difficulty_unqualified"], true);
            for d in p["dags"].as_array().unwrap() {
                assert!(d.get("minimum_difficulty").is_none());
            }
            for t in p["transactions"].as_array().unwrap() {
                assert!(t["fee"].is_string());
            }
        }
        let expected = normalized_digest(&result["periods"]).unwrap();
        let mut changed = result.clone();
        changed["missing_inputs"]["expected_cache_rows"] = json!([{"sha256":"different"}]);
        assert_eq!(expected, normalized_digest(&changed["periods"]).unwrap());
        assert_ne!(
            normalized_digest(&result).unwrap(),
            normalized_digest(&changed).unwrap()
        );
    }
    #[test]
    fn dag_order_signature_and_bundle_bounds_fail_closed() {
        let rows = rows();
        let raw = rows
            .predecessors
            .iter()
            .find(|r| {
                Rlp::new(&r.period_data)
                    .at(2)
                    .unwrap()
                    .item_count()
                    .unwrap_or(0)
                    == 3
            })
            .unwrap();
        let period = Rlp::new(&raw.period_data);
        let signed = period.at(0).unwrap();
        let bundle = period.at(2).unwrap();
        let good = dags(&period, &signed).unwrap();
        assert!(!good.0.is_empty());
        let hashes = good
            .0
            .iter()
            .map(|d| {
                H256::from_slice(&hex::decode(d["signed_hash_hex"].as_str().unwrap()).unwrap())
            })
            .collect::<Vec<_>>();
        let mut flat = RlpStream::new_list(hashes.len());
        for h in &hashes {
            flat.append(h);
        }
        assert_ne!(order_hash(&hashes), hash(&flat.out()));
        let mutated = replace(signed.as_raw(), 2, &rlp::encode(&H256::zero()));
        assert!(dags(&period, &Rlp::new(&mutated)).is_err());
        let mutated = replace(signed.as_raw(), 1, &rlp::encode(&H256::zero()));
        assert!(dags(&period, &Rlp::new(&mutated)).is_err());
        let compact = bundle.at(2).unwrap();
        let first = compact.at(0).unwrap();
        let invalid = replace(first.as_raw(), 5, &rlp::encode(&vec![0u8; 65]));
        let blocks = replace(compact.as_raw(), 0, &invalid);
        let mutated_bundle = replace(bundle.as_raw(), 2, &blocks);
        let mutated_period = replace(&raw.period_data, 2, &mutated_bundle);
        assert!(dags(&Rlp::new(&mutated_period), &signed).is_err());
        let mismatched = replace(bundle.as_raw(), 1, &[0xc0]);
        let changed = replace(&raw.period_data, 2, &mismatched);
        assert!(dags(&Rlp::new(&changed), &signed).is_err());
        let first_index = bundle.at(1).unwrap().at(0).unwrap();
        let invalid = replace(first_index.as_raw(), 0, &rlp::encode(&u64::MAX));
        let index_lists = replace(bundle.at(1).unwrap().as_raw(), 0, &invalid);
        let bad = replace(bundle.as_raw(), 1, &index_lists);
        let bad = replace(&raw.period_data, 2, &bad);
        assert!(dags(&Rlp::new(&bad), &signed).is_err());
        assert!(scalar_u16(&Rlp::new(&hex::decode("c3820010").unwrap()), 0).is_err());
    }
    #[test]
    fn source_and_projected_caps_precede_dag_and_certificate_decoding() {
        let rows = rows();
        let raw = rows
            .predecessors
            .iter()
            .find(|r| {
                Rlp::new(&r.period_data)
                    .at(2)
                    .unwrap()
                    .item_count()
                    .unwrap_or(0)
                    == 3
            })
            .unwrap();
        let period = Rlp::new(&raw.period_data);
        let signed = period.at(0).unwrap();
        let compact = period.at(2).unwrap().at(2).unwrap().at(0).unwrap();
        for (target, message) in [
            (16385, "compact DAG source cap/shape"),
            (16380, "projected DAG item byte cap"),
        ] {
            let initial = replace(compact.as_raw(), 3, &rlp::encode(&vec![0u8; 16000]));
            let padding = target - (initial.len() - 16000);
            let source = replace(compact.as_raw(), 3, &rlp::encode(&vec![0u8; padding]));
            assert_eq!(source.len(), target);
            let mut hashes = RlpStream::new_list(1);
            hashes.append(&H256::zero());
            let mut indexes = RlpStream::new_list(1);
            indexes.begin_list(1).append(&0u64);
            let mut blocks = RlpStream::new_list(1);
            blocks.append_raw(&source, 1);
            let mut bundle = RlpStream::new_list(3);
            bundle
                .append_raw(&hashes.out(), 1)
                .append_raw(&indexes.out(), 1)
                .append_raw(&blocks.out(), 1);
            let changed = replace(&raw.period_data, 2, &bundle.out());
            assert_eq!(
                dags(&Rlp::new(&changed), &signed).unwrap_err().to_string(),
                message
            );
        }
        let raw = &rows.predecessors[0];
        let period = Rlp::new(&raw.period_data);
        let signed = period.at(0).unwrap();
        let bundle = period.at(1).unwrap();
        let first = bundle.at(4).unwrap().at(0).unwrap();
        for (target, message) in [
            (16385, "optimized vote source byte cap/shape"),
            (16380, "projected canonical vote byte cap"),
        ] {
            let initial = replace(first.as_raw(), 0, &rlp::encode(&vec![0u8; 16000]));
            let padding = target - (initial.len() - 16000);
            let source = replace(first.as_raw(), 0, &rlp::encode(&vec![0u8; padding]));
            assert_eq!(source.len(), target);
            let mut votes = RlpStream::new_list(1);
            votes.append_raw(&source, 1);
            let changed_bundle = replace(bundle.as_raw(), 4, &votes.out());
            let changed = replace(&raw.period_data, 1, &changed_bundle);
            assert_eq!(
                certs(&changed, &signed, LAST).unwrap_err().to_string(),
                message
            );
        }
    }
    #[test]
    fn cert_status_set_binding_and_wire_order_are_checked() {
        let rows = rows();
        let raw = &rows.predecessors[0];
        let s = LAST;
        let period = Rlp::new(&raw.period_data);
        let signed = period.at(0).unwrap();
        let original = certs(&raw.period_data, &signed, s).unwrap();
        let bundle = period.at(1).unwrap();
        let votes = bundle.at(4).unwrap();
        let mut reversed = RlpStream::new_list(votes.item_count().unwrap());
        for i in (0..votes.item_count().unwrap()).rev() {
            reversed.append_raw(votes.at(i).unwrap().as_raw(), 1);
        }
        let bundle_changed = replace(bundle.as_raw(), 4, &reversed.out());
        let changed = replace(&raw.period_data, 1, &bundle_changed);
        let reordered = certs(&changed, &signed, s).unwrap();
        assert_ne!(
            original["wire_order_digest"],
            reordered["wire_order_digest"]
        );
        assert_eq!(
            original["signed_reward_hashes_hex"],
            reordered["signed_reward_hashes_hex"]
        );
        let mut bad = RlpStream::new_list(0);
        let signed_bad = replace(signed.as_raw(), 6, &bad.out());
        assert!(certs(&raw.period_data, &Rlp::new(&signed_bad), s).is_err());
        bad = RlpStream::new_list(votes.item_count().unwrap());
        for _ in 0..votes.item_count().unwrap() {
            bad.append_raw(votes.at(0).unwrap().as_raw(), 1);
        }
        let b = replace(bundle.as_raw(), 4, &bad.out());
        assert!(certs(&replace(&raw.period_data, 1, &b), &signed, s).is_err());
        let b = replace(bundle.as_raw(), 1, &rlp::encode(&(s - 2)));
        assert!(certs(&replace(&raw.period_data, 1, &b), &signed, s).is_err());
        let first = votes.at(0).unwrap();
        let invalid = replace(first.as_raw(), 1, &rlp::encode(&vec![0u8; 65]));
        let v = replace(votes.as_raw(), 0, &invalid);
        let b = replace(bundle.as_raw(), 4, &v);
        assert!(certs(&replace(&raw.period_data, 1, &b), &signed, s).is_err());
    }
    #[test]
    fn fees_gas_and_digest_identity_do_not_use_expected_cache() {
        let rows = rows();
        let raw = &rows.predecessors[0];
        let header =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&raw.header)).unwrap();
        let period = Rlp::new(&raw.period_data);
        let signed = period.at(0).unwrap();
        let (_, inputs) = dags(&period, &signed).unwrap();
        let (tx, fees, _) =
            transactions(&raw.period_data, &raw.receipts, &header, &inputs).unwrap();
        assert!(!tx.is_empty());
        assert!(!fees.is_empty());
        let receipts = Rlp::new(&raw.receipts);
        let first = receipts.at(0).unwrap();
        let wrong = replace(first.as_raw(), 2, &rlp::encode(&0u64));
        let bad = replace(&raw.receipts, 0, &wrong);
        assert!(transactions(&raw.period_data, &bad, &header, &inputs).is_err());
        assert_eq!(
            normalized_digest(&json!({"b":2,"a":1})).unwrap(),
            normalized_digest(&json!({"a":1,"b":2})).unwrap()
        );
        assert!(normalized_digest(&json!({"float":1.5})).is_err());
        assert!(U256::MAX.checked_mul(U256::from(2)).is_none());
        let mut incomplete = rows.clone();
        incomplete.predecessors.pop();
        assert!(derive_available_fields(&incomplete).is_err());
    }

    #[test]
    fn metadata_key_and_value_accounting_and_expected_hash_independence() {
        let metadata = ParentCacheMetadata {
            rows: (0..49)
                .map(|i| CacheExpectedRow {
                    period: FIRST + i,
                    bytes: if i == 0 {
                        2548
                    } else if i == 48 {
                        566
                    } else {
                        500
                    },
                    sha256: "00".repeat(32),
                })
                .collect(),
        };
        let raw = rows();
        let valid = derive_parent_reward_fields(&raw, &metadata).unwrap().value;
        assert_eq!(valid["cache_metadata_bytes"]["all_value_bytes"], 26614);
        assert_eq!(
            valid["cache_metadata_bytes"]["all_key_and_value_bytes"],
            27006
        );
        validate_field_report(&valid).unwrap();
        for (scope, key, value) in [
            ("qualification", "owner_authority", json!(true)),
            ("qualification", "unexpected_authority", json!(false)),
            ("root", "expected_state_root", json!("00".repeat(32))),
        ] {
            let mut bad = valid.clone();
            if scope == "root" {
                bad[key] = value;
            } else {
                bad[scope][key] = value;
            }
            assert!(validate_field_report(&bad).is_err());
        }
        let mut bad = valid.clone();
        bad["periods"][0]["dag_reward_counters"] = json!({});
        assert!(validate_field_report(&bad).is_err());
        let mut bad = valid.clone();
        bad["periods"][0]["minimum_difficulty_unqualified"] = json!(false);
        assert!(validate_field_report(&bad).is_err());
        let mut bad = valid.clone();
        bad["missing_inputs"]["period_lambda"][0]
            .as_object_mut()
            .unwrap()
            .remove("policy_id");
        assert!(validate_field_report(&bad).is_err());
        let mut bad = valid.clone();
        bad["missing_inputs"]["delayed_roots_and_totals"][0]["root_source"] =
            json!({"period":FIRST});
        assert!(validate_field_report(&bad).is_err());
        let mut changed = metadata.clone();
        changed.rows[0].sha256 = "01".repeat(32);
        let other = derive_parent_reward_fields(&raw, &changed).unwrap().value;
        assert_eq!(
            valid["digests"]["available_fields_digest"],
            other["digests"]["available_fields_digest"]
        );
        assert_ne!(
            valid["digests"]["missing_manifest_digest"],
            other["digests"]["missing_manifest_digest"]
        );
        changed.rows[0].bytes += 392;
        assert!(derive_parent_reward_fields(&raw, &changed).is_err());
        let mut changed = metadata.clone();
        changed.rows[1].period = FIRST;
        assert!(derive_parent_reward_fields(&raw, &changed).is_err());
        assert!(derive_parent_reward_fields(&raw, &ParentCacheMetadata { rows: vec![] }).is_err());
    }
}
