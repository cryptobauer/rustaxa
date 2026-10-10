//! Byte-input authentication of the complete nineteen-item retained H window.
//! The anchor is accepted local evidence, not independent consensus finality.
//! No transaction execution, native candidate extraction or owner import occurs.
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use keccak_hasher::KeccakHasher;
use rlp::{Rlp, RlpStream};
use rustaxa_types::{
    FinalChainBlockNumber, LegacyTransactionEnvelope, PbftBlockMetadata,
    StoredFinalChainBlockHeader,
    codec::rlp::{
        final_chain::{
            LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
            StoredBlockHeaderRlpOwned,
        },
        pbft::SignedPbftBlockRlp,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const H: u64 = 25_706_949;
pub const HEAD_HASH: &str = "e5b61be65207997d9d30536500b4bd8be3fc2bbe3ec00c38343f20282c6e5065";
pub const GENESIS_HASH: &str = "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377";
pub const MAX_ROW_BYTES: usize = 8192;
pub const MAX_TOTAL_BYTES: usize = 16 * 1024;
pub const MAX_ITEM_BYTES: usize = 4096;
pub const ITEM_COUNT: usize = 19;
const REPLAY: &[u8] = include_bytes!("../../../../doc/evm_research/n4_replay_preflight.json");
const ANCHOR: &[u8] = include_bytes!("../../../../doc/evm_research/n4_restored_reward_inputs.json");

/// Exact eight point observations from the immutable application adapter.
/// Empty system-hash bytes mean no persisted system hash list in this profile.
/// All other required rows must match exact lengths and accepted byte pins.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadHistoryRows {
    pub head: Vec<u8>,
    pub genesis: Vec<u8>,
    pub period_data: Vec<u8>,
    pub receipts: Vec<u8>,
    pub header: Vec<u8>,
    pub hash_by_number: Vec<u8>,
    pub number_by_hash: Vec<u8>,
    pub system_hashes: Vec<u8>,
}
impl HeadHistoryRows {
    /// Returns all admitted observations in the contract's fixed read order.
    pub fn values(&self) -> [&[u8]; 8] {
        [
            &self.head,
            &self.genesis,
            &self.period_data,
            &self.receipts,
            &self.header,
            &self.hash_by_number,
            &self.number_by_hash,
            &self.system_hashes,
        ]
    }
}

/// Locally anchored complete list witness, preserving ordered raw root values.
/// It cannot supply historical key completeness, native effects or finality.
#[derive(Clone, Debug, Serialize)]
pub struct HeadHistoryAuthentication {
    pub reconstructed_head_hash_hex: String,
    pub transactions_root_hex: String,
    pub receipts_root_hex: String,
    pub signed_transactions_hex: Vec<String>,
    pub receipts_hex: Vec<String>,
    pub envelope_facts: Vec<Value>,
    pub point_value_bytes: usize,
    pub source_authenticated_to_local_h: bool,
    pub native_candidates: Vec<Value>,
    pub historical_key_completeness: bool,
    pub semantic_snapshot_complete: bool,
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn bounded_rows(rows: &HeadHistoryRows) -> Result<usize> {
    let mut total = 0_usize;
    for row in rows.values() {
        ensure!(
            row.len() <= MAX_ROW_BYTES,
            "head history row byte cap exceeded"
        );
        total = total
            .checked_add(row.len())
            .context("head history byte overflow")?;
        ensure!(
            total <= MAX_TOTAL_BYTES,
            "head history total byte cap exceeded"
        );
    }
    Ok(total)
}

/// Validates canonical, fully consumed RLP recursively before semantic decode.
/// Per-row size, 4,096 nodes and depth 64 bound the recursive work.
fn canonical(bytes: &[u8]) -> Result<()> {
    canonical_bounded(bytes, 4096)
}
/// Shared complete canonical RLP check; callers bound row bytes independently.
pub(crate) fn canonical_bounded(bytes: &[u8], max_nodes: usize) -> Result<()> {
    fn rebuild(bytes: &[u8], depth: usize, nodes: &mut usize, max_nodes: usize) -> Result<Vec<u8>> {
        *nodes += 1;
        ensure!(
            depth <= 64 && *nodes <= max_nodes,
            "RLP traversal cap exceeded"
        );
        let row = Rlp::new(bytes);
        ensure!(
            row.payload_info()?.total() == bytes.len(),
            "RLP trailing bytes"
        );
        let mut stream = RlpStream::new();
        if row.is_list() {
            let count = row.item_count()?;
            ensure!(count <= max_nodes, "RLP child cap exceeded");
            stream.begin_list(count);
            for position in 0..count {
                stream.append_raw(
                    &rebuild(row.at(position)?.as_raw(), depth + 1, nodes, max_nodes)?,
                    1,
                );
            }
        } else {
            stream.append(&row.data()?);
        }
        let encoded = stream.out().to_vec();
        ensure!(encoded == bytes, "noncanonical RLP");
        Ok(encoded)
    }
    rebuild(bytes, 0, &mut 0, max_nodes)?;
    Ok(())
}

fn list_items(bytes: &[u8], count: usize) -> Result<Vec<&[u8]>> {
    canonical(bytes)?;
    let list = Rlp::new(bytes);
    ensure!(
        list.is_list() && list.item_count()? == count,
        "ordered item count differs"
    );
    (0..count)
        .map(|position| {
            let raw = list.at(position)?.as_raw();
            ensure!(
                raw.len() <= MAX_ITEM_BYTES,
                "ordered item byte cap exceeded"
            );
            Ok(raw)
        })
        .collect()
}

fn receipt(bytes: &[u8]) -> Result<(u8, u64, u64, usize)> {
    receipt_bounded(bytes, 4096)
}
/// Shared five-field Taraxa receipt decoder with canonical scalar re-encoding.
pub(crate) fn receipt_bounded(bytes: &[u8], max_nodes: usize) -> Result<(u8, u64, u64, usize)> {
    canonical_bounded(bytes, max_nodes)?;
    let row = Rlp::new(bytes);
    ensure!(row.item_count()? == 5, "receipt arity differs");
    let status: u8 = row.val_at(0)?;
    ensure!(status <= 1, "receipt status differs");
    let gas: u64 = row.val_at(1)?;
    let cumulative: u64 = row.val_at(2)?;
    let logs = row.at(3)?;
    ensure!(logs.is_list(), "receipt logs are not a list");
    let mut encoded = RlpStream::new_list(5);
    encoded
        .append(&status)
        .append(&gas)
        .append(&cumulative)
        .begin_list(logs.item_count()?);
    for index in 0..logs.item_count()? {
        let log = logs.at(index)?;
        ensure!(log.item_count()? == 3, "log arity differs");
        let address = log.at(0)?.data()?;
        ensure!(address.len() == 20, "log address width differs");
        let topics = log.at(1)?;
        ensure!(topics.is_list(), "log topics are not a list");
        encoded
            .begin_list(3)
            .append(&address)
            .begin_list(topics.item_count()?);
        for position in 0..topics.item_count()? {
            let topic = topics.at(position)?.data()?;
            ensure!(topic.len() == 32, "topic width differs");
            encoded.append(&topic);
        }
        encoded.append(&log.at(2)?.data()?);
    }
    let created = row.at(4)?.data()?;
    ensure!(
        created.is_empty() || created.len() == 20,
        "created address sentinel differs"
    );
    encoded.append(&created);
    ensure!(
        encoded.out().as_ref() == bytes,
        "receipt scalar is not canonical"
    );
    Ok((status, gas, cumulative, logs.item_count()?))
}

pub(crate) fn ordered_root(items: &[&[u8]]) -> H256 {
    H256::from_slice(triehash::ordered_trie_root::<KeccakHasher, _>(items.iter().copied()).as_ref())
}
fn authenticate_lists(
    transactions: Vec<&[u8]>,
    receipts: Vec<&[u8]>,
    tx_root: H256,
    receipt_root: H256,
) -> Result<()> {
    ensure!(
        transactions.len() == ITEM_COUNT && receipts.len() == ITEM_COUNT,
        "complete list count differs"
    );
    ensure!(
        ordered_root(&transactions) == tx_root,
        "transaction trie root mismatch"
    );
    ensure!(
        ordered_root(&receipts) == receipt_root,
        "receipt trie root mismatch"
    );
    Ok(())
}

/// Authenticates exact H raw observations before returning any source witness.
///
/// Checks all byte/resource pins, local indexes, signed PBFT context and the
/// reconstructed Ethereum-format H hash. Both complete ordered trie roots must
/// match its header before fresh signed-envelope/receipt verification against
/// accepted same-position evidence. Failures return no partial/candidate output.
/// This ordinary-transfer-only profile rejects systems and emits zero native
/// candidates. It makes no older-period, public-finality or owner claim.
pub fn authenticate_head_history(rows: &HeadHistoryRows) -> Result<HeadHistoryAuthentication> {
    let total = bounded_rows(rows)?;
    ensure!(
        sha(REPLAY) == "d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437"
            && sha(ANCHOR) == "bf1292101a4afb406489cbb029e51c7fca23e6d3b2ee260684ce0880b6ba0907",
        "accepted evidence drift"
    );
    let replay: Value = serde_json::from_slice(REPLAY)?;
    let anchor: Value = serde_json::from_slice(ANCHOR)?;
    ensure!(
        rows.head == H.to_le_bytes() && rows.number_by_hash == H.to_le_bytes(),
        "head/number index mismatch"
    );
    ensure!(
        hex::encode(&rows.genesis) == GENESIS_HASH
            && hex::encode(&rows.hash_by_number) == HEAD_HASH,
        "genesis/hash index mismatch"
    );
    ensure!(
        rows.system_hashes.is_empty(),
        "unsupported system transaction hashes"
    );
    for (name, bytes) in [
        ("period_data", &rows.period_data),
        ("receipts", &rows.receipts),
        ("header", &rows.header),
    ] {
        ensure!(
            anchor["rows"][name]["bytes"].as_u64() == Some(bytes.len() as u64)
                && anchor["rows"][name]["sha256"].as_str() == Some(sha(bytes).as_str()),
            "{name} byte pin mismatch"
        );
        canonical(bytes)?;
    }
    let period = Rlp::new(&rows.period_data);
    ensure!(period.item_count()? == 4, "period envelope arity differs");
    let pbft_raw = period.at(0)?.as_raw();
    let pbft = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(pbft_raw))?;
    ensure!(
        pbft.period == H
            && hex::encode(pbft.author) == "a6cec53b4d7920709f05ca1f7ac0d66338d29cb9"
            && pbft.timestamp == 1772723363,
        "signed PBFT context mismatch"
    );
    let stored = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&rows.header))?;
    ensure!(
        StoredBlockHeaderRlpOwned::from(&stored).as_bytes() == rows.header,
        "stored header is not canonical"
    );
    let legacy = LegacyBlockHeaderRlp::try_from(
        // FinalChain's persisted header uses genesis.pbft.gas_limit. The later
        // Cornus proposal/replay limit is a different context (see source review).
        LegacyBlockHeaderRlpInput::new(StoredBlockHeaderRlp::new(&rows.header), 315_000_000, 0)
            .signed_pbft_block(SignedPbftBlockRlp::new(pbft_raw))
            .block_number(FinalChainBlockNumber::new(H)),
    )?;
    let head_hash = legacy.hash()?;
    ensure!(
        hex::encode(head_hash) == HEAD_HASH,
        "reconstructed H hash mismatch"
    );
    let transactions = list_items(period.at(3)?.as_raw(), ITEM_COUNT)?;
    let receipts = list_items(&rows.receipts, ITEM_COUNT)?;
    authenticate_lists(
        transactions.clone(),
        receipts.clone(),
        stored.transactions_root,
        stored.receipts_root,
    )?;
    let expected = replay["envelope_classification"]["entries"]
        .as_array()
        .context("missing accepted envelope facts")?;
    let outcomes = replay["outcomes"]
        .as_array()
        .context("missing accepted receipt facts")?;
    ensure!(
        expected.len() == ITEM_COUNT && outcomes.len() == ITEM_COUNT,
        "accepted evidence count differs"
    );
    let mut facts = Vec::new();
    for (position, (raw, raw_receipt)) in transactions.iter().zip(&receipts).enumerate() {
        let tx = LegacyTransactionEnvelope::decode(raw)?;
        ensure!(
            tx.signature_valid
                && tx.sender.is_some()
                && tx.chain_id == 841
                && tx.receiver.is_some()
                && tx.data.is_empty(),
            "signed ordinary envelope mismatch"
        );
        let fact = json!({"position": position, "hash_hex": hex::encode(tx.hash), "rlp_sha256": sha(raw),
            "sender_hex": tx.sender.map(hex::encode), "receiver_hex": tx.receiver.map(hex::encode),
            "nonce_hex": format!("{:x}", tx.nonce), "gas_price_hex": format!("{:x}", tx.gas_price), "gas_limit": tx.gas,
            "chain_id": tx.chain_id, "value_hex": format!("{:x}", tx.value), "input_hex": hex::encode(&tx.data), "kind": "call"});
        ensure!(
            fact == expected[position],
            "signed envelope facts differ at position {position}"
        );
        let (status, gas, cumulative, logs) = receipt(raw_receipt)?;
        let outcome = &outcomes[position];
        ensure!(
            outcome["position"].as_u64() == Some(position as u64)
                && outcome["expected_receipt_sha256"].as_str() == Some(sha(raw_receipt).as_str())
                && outcome["status"] == status
                && outcome["gas_used"] == gas
                && outcome["cumulative_gas_used"] == cumulative
                && outcome["logs"] == logs,
            "same-position receipt differs at position {position}"
        );
        facts.push(fact);
    }
    Ok(HeadHistoryAuthentication {
        reconstructed_head_hash_hex: hex::encode(head_hash),
        transactions_root_hex: hex::encode(stored.transactions_root),
        receipts_root_hex: hex::encode(stored.receipts_root),
        signed_transactions_hex: transactions.iter().map(hex::encode).collect(),
        receipts_hex: receipts.iter().map(hex::encode).collect(),
        envelope_facts: facts,
        point_value_bytes: total,
        source_authenticated_to_local_h: true,
        native_candidates: Vec::new(),
        historical_key_completeness: false,
        semantic_snapshot_complete: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows() -> HeadHistoryRows {
        serde_json::from_slice(include_bytes!("../tests/fixtures/head_history_rows.json")).unwrap()
    }
    #[test]
    fn exact_retained_h_authenticates_complete_raw_lists() {
        let witness = authenticate_head_history(&rows()).unwrap();
        assert_eq!(witness.reconstructed_head_hash_hex, HEAD_HASH);
        assert_eq!(witness.signed_transactions_hex.len(), ITEM_COUNT);
        assert_eq!(witness.receipts_hex.len(), ITEM_COUNT);
        assert_eq!(witness.point_value_bytes, 7973);
        assert!(witness.native_candidates.is_empty());
        assert!(!witness.historical_key_completeness && !witness.semantic_snapshot_complete);
    }
    #[test]
    fn every_required_observation_and_resource_pin_fails_closed() {
        for field in 0..8 {
            let mut changed = rows();
            let values = [
                &mut changed.head,
                &mut changed.genesis,
                &mut changed.period_data,
                &mut changed.receipts,
                &mut changed.header,
                &mut changed.hash_by_number,
                &mut changed.number_by_hash,
                &mut changed.system_hashes,
            ];
            values.into_iter().nth(field).unwrap().push(0);
            assert!(
                authenticate_head_history(&changed).is_err(),
                "field {field}"
            );
        }
        let mut changed = rows();
        changed.period_data.clear();
        assert!(authenticate_head_history(&changed).is_err());
        changed = rows();
        changed.period_data = vec![0; MAX_ROW_BYTES + 1];
        assert!(bounded_rows(&changed).is_err());
        changed = rows();
        changed.period_data = vec![0; MAX_ROW_BYTES];
        changed.receipts = vec![0; MAX_ROW_BYTES];
        assert!(bounded_rows(&changed).is_err());
    }
    #[test]
    fn cpp_reference_vectors_and_real_ordered_values_match() {
        for (items, expected) in [
            (
                vec![],
                "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            ),
            (
                vec![&[1][..], &[2][..]],
                "08693bf7194a9e5481f518cc27be677512d038ddfb72263e13f0663e299ce2d8",
            ),
            (
                vec![&[2][..], &[1][..]],
                "282aa29e17ea3cf1485631b15779554a538f86892c53451d622ac4c0701dce5e",
            ),
        ] {
            assert_eq!(hex::encode(ordered_root(&items)), expected);
        }
        let data = rows();
        let period = Rlp::new(&data.period_data);
        let txs = list_items(period.at(3).unwrap().as_raw(), ITEM_COUNT).unwrap();
        let receipts = list_items(&data.receipts, ITEM_COUNT).unwrap();
        let tx_root = H256::from_slice(
            &hex::decode("899f44457d384d45b5bd704ce5e091e966f00ed93dab3c62b73816cd867fe654")
                .unwrap(),
        );
        let receipt_root = H256::from_slice(
            &hex::decode("84c077c745d619c706fb637ec0c68966e5a057ea63ed78e39e496d72f7a4f721")
                .unwrap(),
        );
        authenticate_lists(txs.clone(), receipts.clone(), tx_root, receipt_root).unwrap();
        let mut reordered = txs.clone();
        reordered.swap(0, 1);
        assert!(authenticate_lists(reordered, receipts.clone(), tx_root, receipt_root).is_err());
        let mut reordered = receipts.clone();
        reordered.swap(0, 1);
        assert!(authenticate_lists(txs.clone(), reordered, tx_root, receipt_root).is_err());
        let mut omitted = txs.clone();
        omitted.pop();
        assert!(authenticate_lists(omitted, receipts.clone(), tx_root, receipt_root).is_err());
        let mut inserted = receipts.clone();
        inserted.push(receipts[0]);
        assert!(authenticate_lists(txs.clone(), inserted, tx_root, receipt_root).is_err());
        let mut wrong = receipts.clone();
        wrong[0] = receipts[1];
        assert!(authenticate_lists(txs, wrong, tx_root, receipt_root).is_err());
    }
    #[test]
    fn canonical_receipts_and_complete_rlp_reject_malformed_inputs() {
        assert_eq!(
            receipt(&[0xc7, 1, 0x82, 0x52, 8, 1, 0xc0, 0x80]).unwrap(),
            (1, 21000, 1, 0)
        );
        for raw in [
            vec![0xc0],
            vec![0xc5, 2, 1, 1, 0xc0, 0x80],
            vec![0xc6, 0x81, 0, 1, 1, 0xc0, 0x80],
            vec![0xc5, 1, 1, 1, 0xc0, 0xc0],
        ] {
            assert!(receipt(&raw).is_err());
        }
        assert!(canonical(&[0xc0, 0]).is_err());
        assert!(canonical(&[0x81, 1]).is_err());
        let mut nested = vec![0x80];
        for _ in 0..65 {
            let mut s = RlpStream::new_list(1);
            s.append_raw(&nested, 1);
            nested = s.out().to_vec();
        }
        assert!(canonical(&nested).is_err());
    }
    #[test]
    fn list_count_item_size_and_root_mismatch_fail_closed() {
        assert!(list_items(&[0xc0], ITEM_COUNT).is_err());
        let oversized = vec![1; MAX_ITEM_BYTES + 1];
        let mut list = RlpStream::new_list(1);
        list.append(&oversized.as_slice());
        assert!(list_items(&list.out(), 1).is_err());
        let values = vec![&[1][..]; ITEM_COUNT];
        assert!(authenticate_lists(values.clone(), values, H256::zero(), H256::zero()).is_err());
    }
}
