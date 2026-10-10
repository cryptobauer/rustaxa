//! Complete P lists authenticated through the accepted local H successor.
//! This module verifies byte witnesses; it neither executes nor imports state.
use crate::head_history::{self as head, canonical_bounded, ordered_root, receipt_bounded};
use anyhow::{Context, Result, ensure};
use ethereum_types::{H256, U256};
use rlp::{Rlp, RlpStream};
use rustaxa_types::pbft::PbftBlockLink;
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

pub const P: u64 = head::H - 1;
/// Fixed diagnostic lower endpoint; never a retention boundary.
pub const MIN_PERIOD: u64 = 25_706_901;
pub const MAX_ROW_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_BYTES: usize = 512 * 1024;
pub const MAX_ITEMS: usize = 256;
pub const MAX_ITEM_BYTES: usize = 16 * 1024;
pub const MAX_NODES: usize = 32768;

/// Six exact P application rows, in the approved acquisition order.
/// Missing required rows are errors; empty system bytes are format-defined.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParentHistoryRows {
    pub header: Vec<u8>,
    pub period_data: Vec<u8>,
    pub hash_by_number: Vec<u8>,
    pub number_by_hash: Vec<u8>,
    pub system_hashes: Vec<u8>,
    pub receipts: Vec<u8>,
}
impl ParentHistoryRows {
    pub fn values(&self) -> [&[u8]; 6] {
        [
            &self.header,
            &self.period_data,
            &self.hash_by_number,
            &self.number_by_hash,
            &self.system_hashes,
            &self.receipts,
        ]
    }
}

/// Sealed local successor context; construction requires the entire H verifier.
/// Fields are private so callers cannot substitute unverified predecessor hashes.
pub struct ParentLinkContext {
    period: u64,
    expected_hash: H256,
    expected_pbft: H256,
}
impl ParentLinkContext {
    /// Revalidates every H observation and derives both predecessor hash domains.
    pub fn authenticate_successor(rows: &head::HeadHistoryRows) -> Result<Self> {
        head::authenticate_head_history(rows)?;
        let stored =
            StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&rows.header))?;
        let period = Rlp::new(&rows.period_data);
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(period.at(0)?.as_raw()))?;
        Ok(Self {
            period: P,
            expected_hash: stored.parent_hash,
            expected_pbft: link.prev_block_hash,
        })
    }
    /// Exact next period carried by this sealed context.
    pub fn period(&self) -> u64 {
        self.period
    }
    /// Checks a forward index immediately, before the reverse-index read.
    pub fn authenticate_forward_index(&self, hash: &[u8]) -> Result<()> {
        ensure!(
            hash == self.expected_hash.as_bytes(),
            "predecessor forward hash mismatch"
        );
        Ok(())
    }
    /// Consumes this context and advances only after full predecessor success.
    /// The fixed lower endpoint returns no next context; it cannot overrun.
    pub fn authenticate_and_advance(
        self,
        rows: &ParentHistoryRows,
    ) -> Result<(ParentHistoryAuthentication, Option<Self>)> {
        ensure!(
            (MIN_PERIOD..=P).contains(&self.period),
            "interval period outside fixed bounds"
        );
        let witness = self.authenticate_rows(rows)?;
        let next = if self.period == MIN_PERIOD {
            None
        } else {
            let stored =
                StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&rows.header))?;
            let period = Rlp::new(&rows.period_data);
            let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(period.at(0)?.as_raw()))?;
            Some(Self {
                period: self.period.checked_sub(1).context("period underflow")?,
                expected_hash: stored.parent_hash,
                expected_pbft: link.prev_block_hash,
            })
        };
        Ok((witness, next))
    }
    /// Returns the authenticated successor's FinalChain predecessor hash.
    pub fn expected_hash(&self) -> H256 {
        self.expected_hash
    }
    /// Checks bounded canonical P header/period and both distinct predecessor links.
    /// This stage must pass before the adapter reads P indexes or receipt rows.
    pub fn authenticate_header(&self, header: &[u8], period_data: &[u8]) -> Result<()> {
        let _ = self.header_context(header, period_data)?;
        Ok(())
    }
    fn header_context(
        &self,
        header: &[u8],
        period_data: &[u8],
    ) -> Result<(StoredFinalChainBlockHeader, PbftBlockMetadata, H256)> {
        ensure!(
            header.len() <= MAX_ROW_BYTES && period_data.len() <= MAX_ROW_BYTES,
            "P header/period row cap exceeded"
        );
        canonical_bounded(header, MAX_NODES)?;
        let period = Rlp::new(period_data);
        ensure!(period.item_count()? == 4, "P period arity differs");
        item_shapes(period.at(3)?.as_raw())?;
        canonical_bounded(period_data, MAX_NODES)?;
        let signed = period.at(0)?;
        ensure!(
            matches!(signed.item_count()?, 8 | 9),
            "P signed PBFT arity differs"
        );
        let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(signed.as_raw()))?;
        ensure!(
            metadata.period == self.period && link.period == self.period,
            "P PBFT period differs"
        );
        let typed_period: u64 = signed.val_at(4)?;
        let timestamp: u64 = signed.val_at(5)?;
        ensure!(
            typed_period == self.period && timestamp == metadata.timestamp,
            "P PBFT scalar differs"
        );
        for (position, value) in [(4, typed_period), (5, timestamp)] {
            let mut scalar = RlpStream::new();
            scalar.append(&value);
            ensure!(
                scalar.out().as_ref() == signed.at(position)?.as_raw(),
                "P PBFT scalar not canonical"
            );
        }
        ensure!(
            link.block_hash == self.expected_pbft,
            "H to P PBFT link mismatch"
        );
        let stored = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(header))?;
        ensure!(
            StoredBlockHeaderRlpOwned::from(&stored).as_bytes() == header,
            "P stored header not canonical"
        );
        let legacy = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(StoredBlockHeaderRlp::new(header), 315_000_000, 0)
                .signed_pbft_block(SignedPbftBlockRlp::new(signed.as_raw()))
                .block_number(FinalChainBlockNumber::new(self.period)),
        )?;
        ensure!(
            legacy.hash()? == self.expected_hash,
            "H to P FinalChain link mismatch"
        );
        Ok((stored, metadata, link.block_hash))
    }
    /// Checks exact hash and little-endian number indexes before later reads.
    pub fn authenticate_indexes(&self, hash: &[u8], number: &[u8]) -> Result<()> {
        ensure!(
            hash == self.expected_hash.as_bytes() && number == self.period.to_le_bytes(),
            "P indexes mismatch"
        );
        Ok(())
    }
    /// Authenticates complete P lists; failure returns no partial witness.
    /// Total reward and DAG/vote children are preserved observations, not authority.
    pub fn authenticate_rows(
        &self,
        rows: &ParentHistoryRows,
    ) -> Result<ParentHistoryAuthentication> {
        let mut total = 0_usize;
        for row in rows.values() {
            ensure!(row.len() <= MAX_ROW_BYTES, "P row cap exceeded");
            total = total
                .checked_add(row.len())
                .context("P total byte overflow")?;
            ensure!(total <= MAX_TOTAL_BYTES, "P total cap exceeded");
        }
        let (stored, metadata, pbft_hash) = self.header_context(&rows.header, &rows.period_data)?;
        self.authenticate_indexes(&rows.hash_by_number, &rows.number_by_hash)?;
        ensure!(
            rows.system_hashes.is_empty(),
            "P system transactions unsupported"
        );
        let period = Rlp::new(&rows.period_data);
        let txs = items(period.at(3)?.as_raw())?;
        let receipts = items(&rows.receipts)?;
        ensure!(
            txs.len() == receipts.len(),
            "P signed/receipt counts differ"
        );
        ensure!(
            ordered_root(&txs) == stored.transactions_root,
            "P transaction root mismatch"
        );
        ensure!(
            ordered_root(&receipts) == stored.receipts_root,
            "P receipt root mismatch"
        );
        let mut facts = Vec::with_capacity(txs.len());
        for (position, (tx, receipt)) in txs.iter().zip(&receipts).enumerate() {
            let envelope = signed_envelope(tx)?;
            let (status, gas, cumulative, logs) = receipt_bounded(receipt, MAX_NODES)?;
            facts.push(json!({"position": position, "transaction_hash_hex": hex::encode(envelope.hash),
                "sender_hex": envelope.sender.map(hex::encode), "receiver_hex": envelope.receiver.map(hex::encode),
                "chain_id": envelope.chain_id, "nonce_hex": format!("{:x}", envelope.nonce),
                "gas_price_hex": format!("{:x}", envelope.gas_price), "gas_limit": envelope.gas,
                "value_hex": format!("{:x}", envelope.value), "input_hex": hex::encode(&envelope.data),
                "kind": if envelope.receiver.is_some() { "call" } else { "create" },
                "receipt_status": status, "gas_used": gas, "cumulative_gas_used": cumulative, "logs": logs}));
        }
        Ok(ParentHistoryAuthentication {
            period: self.period,
            successor_period: self.period + 1,
            header_hash_hex: hex::encode(self.expected_hash),
            pbft_hash_hex: hex::encode(pbft_hash),
            author_hex: hex::encode(metadata.author),
            timestamp: metadata.timestamp,
            header_gas_limit: 315_000_000,
            transactions_root_hex: hex::encode(stored.transactions_root),
            receipts_root_hex: hex::encode(stored.receipts_root),
            signed_transactions_hex: txs.iter().map(hex::encode).collect(),
            receipts_hex: receipts.iter().map(hex::encode).collect(),
            position_facts: facts,
            point_value_bytes: total,
            native_candidates: vec![],
            source_authenticated_to_local_h: true,
            historical_key_completeness: false,
            semantic_snapshot_complete: false,
        })
    }
}

/// Complete ordered P witness with locally linked byte evidence only.
#[derive(Debug, Serialize)]
pub struct ParentHistoryAuthentication {
    pub period: u64,
    pub successor_period: u64,
    pub header_hash_hex: String,
    pub pbft_hash_hex: String,
    pub author_hex: String,
    pub timestamp: u64,
    pub header_gas_limit: u64,
    pub transactions_root_hex: String,
    pub receipts_root_hex: String,
    pub signed_transactions_hex: Vec<String>,
    pub receipts_hex: Vec<String>,
    pub position_facts: Vec<Value>,
    pub point_value_bytes: usize,
    pub native_candidates: Vec<Value>,
    pub source_authenticated_to_local_h: bool,
    pub historical_key_completeness: bool,
    pub semantic_snapshot_complete: bool,
}
fn item_shapes(bytes: &[u8]) -> Result<usize> {
    let list = Rlp::new(bytes);
    ensure!(list.is_list(), "P items not a list");
    let count = list.item_count()?;
    ensure!(count <= MAX_ITEMS, "P item count cap exceeded");
    for position in 0..count {
        ensure!(
            list.at(position)?.as_raw().len() <= MAX_ITEM_BYTES,
            "P item byte cap exceeded"
        );
    }
    Ok(count)
}
fn items(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    let count = item_shapes(bytes)?;
    canonical_bounded(bytes, MAX_NODES)?;
    let list = Rlp::new(bytes);
    (0..count)
        .map(|position| {
            let raw = list.at(position)?.as_raw();
            ensure!(raw.len() <= MAX_ITEM_BYTES, "P item byte cap exceeded");
            Ok(raw)
        })
        .collect()
}
fn signed_envelope(raw: &[u8]) -> Result<LegacyTransactionEnvelope> {
    canonical_bounded(raw, MAX_NODES)?;
    let tx = LegacyTransactionEnvelope::decode(raw)?;
    ensure!(
        tx.signature_valid && tx.sender.is_some() && tx.chain_id == 841,
        "P signed envelope mismatch"
    );
    let parsed = Rlp::new(raw);
    let mut encoded = RlpStream::new_list(9);
    encoded
        .append(&tx.nonce)
        .append(&tx.gas_price)
        .append(&tx.gas);
    encoded
        .append(&parsed.at(3)?.data()?)
        .append(&tx.value)
        .append(&tx.data.as_slice());
    for position in 6..9 {
        encoded.append(&parsed.val_at::<U256>(position)?);
    }
    ensure!(
        encoded.out().as_ref() == raw,
        "P transaction scalar not canonical"
    );
    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn h() -> head::HeadHistoryRows {
        serde_json::from_slice(include_bytes!("../tests/fixtures/head_history_rows.json")).unwrap()
    }
    fn p() -> ParentHistoryRows {
        serde_json::from_slice(include_bytes!("../tests/fixtures/parent_history_rows.json"))
            .unwrap()
    }
    fn replace(raw: &[u8], position: usize, item: &[u8]) -> Vec<u8> {
        let old = Rlp::new(raw);
        let mut out = RlpStream::new_list(old.item_count().unwrap());
        for i in 0..old.item_count().unwrap() {
            out.append_raw(
                if i == position {
                    item
                } else {
                    old.at(i).unwrap().as_raw()
                },
                1,
            );
        }
        out.out().to_vec()
    }
    #[test]
    fn captured_p_links_both_hash_domains_and_cpp_roots() {
        let context = ParentLinkContext::authenticate_successor(&h()).unwrap();
        let witness = context.authenticate_rows(&p()).unwrap();
        assert_eq!(
            witness.header_hash_hex,
            "40fdf17fb90098b4e044bc51547c3bf6bc15f5e126797563c2e66510b8e637a2"
        );
        assert_eq!(
            witness.pbft_hash_hex,
            "e0922e40b22925af2a1e9360c566a6fbdd517adafd5500183fc5287e48613589"
        );
        assert_eq!(
            witness.transactions_root_hex,
            "429184cba1894049b3bada4b52ee87688a95873e5f0e9aaa07e02b06dcc71472"
        );
        assert_eq!(
            witness.receipts_root_hex,
            "f64e487e2da052888b3c9f53960ce988ee6729a44679142add86861bf1478470"
        );
        assert_eq!(witness.signed_transactions_hex.len(), 1);
        assert_eq!(witness.receipts_hex.len(), 1);
        assert_eq!(witness.point_value_bytes, 5277);
        assert!(witness.native_candidates.is_empty());
        assert!(!witness.historical_key_completeness && !witness.semantic_snapshot_complete);
        let wrong = ParentLinkContext {
            period: P,
            expected_hash: H256::zero(),
            expected_pbft: context.expected_pbft,
        };
        assert!(
            wrong
                .authenticate_rows(&p())
                .unwrap_err()
                .to_string()
                .contains("FinalChain")
        );
        let wrong = ParentLinkContext {
            period: P,
            expected_hash: context.expected_hash,
            expected_pbft: H256::zero(),
        };
        assert!(
            wrong
                .authenticate_rows(&p())
                .unwrap_err()
                .to_string()
                .contains("PBFT")
        );
    }
    #[test]
    fn changed_observations_missing_systems_and_counts_reject() {
        let context = ParentLinkContext::authenticate_successor(&h()).unwrap();
        for field in 0..6 {
            let mut changed = p();
            let values = [
                &mut changed.header,
                &mut changed.period_data,
                &mut changed.hash_by_number,
                &mut changed.number_by_hash,
                &mut changed.system_hashes,
                &mut changed.receipts,
            ];
            values.into_iter().nth(field).unwrap().push(0);
            assert!(
                context.authenticate_rows(&changed).is_err(),
                "field {field}"
            );
        }
        for receipt in [vec![], vec![0xc0], vec![0xc1, 0x80]] {
            let mut changed = p();
            changed.receipts = receipt;
            assert!(context.authenticate_rows(&changed).is_err());
        }
        let mut changed = p();
        changed.period_data = replace(&changed.period_data, 3, &[0xc0]);
        assert!(context.authenticate_rows(&changed).is_err());
        let mut changed = p();
        changed.header = vec![0; MAX_ROW_BYTES];
        changed.period_data = vec![0; MAX_ROW_BYTES];
        changed.receipts = vec![1];
        assert!(
            context
                .authenticate_rows(&changed)
                .unwrap_err()
                .to_string()
                .contains("total cap")
        );
    }
    #[test]
    fn signed_pbft_and_transaction_failures_reject_canonical_tampering() {
        let context = ParentLinkContext::authenticate_successor(&h()).unwrap();
        let original = p();
        let period = Rlp::new(&original.period_data);
        let pbft = period.at(0).unwrap();
        for raw in [
            vec![0xc0],
            replace(pbft.as_raw(), 4, &[0x01]),
            replace(pbft.as_raw(), 5, &[0x00]),
        ] {
            let changed = replace(&original.period_data, 0, &raw);
            assert!(
                context
                    .authenticate_header(&original.header, &changed)
                    .is_err()
            );
        }
        let mut signature = RlpStream::new();
        signature.append(&vec![0; 65].as_slice());
        let bad = replace(
            pbft.as_raw(),
            pbft.item_count().unwrap() - 1,
            &signature.out(),
        );
        assert!(
            context
                .authenticate_header(&original.header, &replace(&original.period_data, 0, &bad))
                .is_err()
        );
        let mut unsupported = RlpStream::new_list(10);
        for _ in 0..10 {
            unsupported.append_empty_data();
        }
        assert!(
            context
                .authenticate_header(
                    &original.header,
                    &replace(&original.period_data, 0, &unsupported.out())
                )
                .is_err()
        );
        let tx = period.at(3).unwrap().at(0).unwrap();
        assert!(signed_envelope(tx.as_raw()).is_ok());
        let mut v = RlpStream::new();
        v.append(&1719u64); // EIP-155 chain 842
        assert!(signed_envelope(&replace(tx.as_raw(), 6, &v.out())).is_err());
        assert!(signed_envelope(&replace(tx.as_raw(), 7, &[0x80])).is_err());
        assert!(signed_envelope(&replace(tx.as_raw(), 0, &[0])).is_err());
        let mut trailing = tx.as_raw().to_vec();
        trailing.push(0);
        assert!(signed_envelope(&trailing).is_err());
    }
    #[test]
    fn exact_list_and_item_caps_preserve_raw_bytes() {
        let mut list = RlpStream::new_list(MAX_ITEMS);
        for _ in 0..MAX_ITEMS {
            list.append_empty_data();
        }
        assert_eq!(items(&list.out()).unwrap().len(), MAX_ITEMS);
        let mut item = RlpStream::new();
        item.append(&vec![1; MAX_ITEM_BYTES - 3].as_slice());
        assert_eq!(item.as_raw().len(), MAX_ITEM_BYTES);
        let mut list = RlpStream::new_list(1);
        list.append_raw(item.as_raw(), 1);
        assert_eq!(items(&list.out()).unwrap()[0].len(), MAX_ITEM_BYTES);
    }
    #[test]
    fn rlp_node_child_depth_boundaries_and_item_admission_are_exact() {
        for (children, passes) in [
            (MAX_NODES - 1, true),
            (MAX_NODES, false),
            (MAX_NODES + 1, false),
        ] {
            let mut list = RlpStream::new_list(children);
            for _ in 0..children {
                list.append_empty_data();
            }
            assert_eq!(canonical_bounded(&list.out(), MAX_NODES).is_ok(), passes);
        }
        let mut nested = vec![0x80];
        for _ in 0..64 {
            let mut list = RlpStream::new_list(1);
            list.append_raw(&nested, 1);
            nested = list.out().to_vec();
        }
        assert!(canonical_bounded(&nested, MAX_NODES).is_ok());
        let mut list = RlpStream::new_list(1);
        list.append_raw(&nested, 1);
        assert!(canonical_bounded(&list.out(), MAX_NODES).is_err());
        let context = ParentLinkContext::authenticate_successor(&h()).unwrap();
        let mut over = RlpStream::new_list(MAX_ITEMS + 1);
        for _ in 0..=MAX_ITEMS {
            over.append_raw(&nested, 1);
        }
        assert!(
            items(&over.as_raw())
                .unwrap_err()
                .to_string()
                .contains("item count")
        );
        let changed = replace(&p().period_data, 3, over.as_raw());
        assert!(
            context
                .authenticate_header(&p().header, &changed)
                .unwrap_err()
                .to_string()
                .contains("item count")
        );
        let mut over = RlpStream::new_list(1);
        over.append(&vec![1; MAX_ITEM_BYTES].as_slice());
        assert!(
            items(over.as_raw())
                .unwrap_err()
                .to_string()
                .contains("item byte")
        );
        let changed = replace(&p().period_data, 3, over.as_raw());
        assert!(
            context
                .authenticate_header(&p().header, &changed)
                .unwrap_err()
                .to_string()
                .contains("item byte")
        );
    }
    #[test]
    fn missing_rows_caps_and_systems_fail_without_witness() {
        let h: head::HeadHistoryRows =
            serde_json::from_slice(include_bytes!("../tests/fixtures/head_history_rows.json"))
                .unwrap();
        let context = ParentLinkContext::authenticate_successor(&h).unwrap();
        assert!(
            context
                .authenticate_rows(&ParentHistoryRows::default())
                .is_err()
        );
        assert!(
            context
                .authenticate_indexes(&[0; 32], &P.to_le_bytes())
                .is_err()
        );
        let mut over = ParentHistoryRows::default();
        over.header = vec![0; MAX_ROW_BYTES + 1];
        assert!(context.authenticate_rows(&over).is_err());
        assert!(items(&[]).is_err());
        assert!(items(&[0xc0]).unwrap().is_empty());
        let mut list = RlpStream::new_list(MAX_ITEMS + 1);
        for _ in 0..=MAX_ITEMS {
            list.append_empty_data();
        }
        assert!(items(&list.out()).is_err());
        let mut list = RlpStream::new_list(1);
        list.append(&vec![0; MAX_ITEM_BYTES].as_slice());
        assert!(items(&list.out()).is_err());
    }
}
