use crate::dag::DagBlock;
use anyhow::{Result, bail};
use rlp::{DecoderError, Rlp};

/// Canonical eight-field DAG block RLP bytes.
#[derive(Debug, Clone, Copy)]
pub struct DagBlockRlp<'a>(&'a [u8]);

impl<'a> DagBlockRlp<'a> {
    /// Wraps raw canonical DAG block RLP bytes for typed decoding.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
}

/// Compact finalized DAG block bundle RLP bytes stored inside period data.
///
/// The finalized bundle layout is `[ordered_transaction_hashes,
/// transaction_indexes_per_block, compact_dag_blocks]`. Each compact block is
/// the seven-field C++ finalized representation without inline transaction
/// hashes. Call `reconstruct_finalized_dag_block_rlp` to materialize the
/// canonical eight-field DAG block RLP expected by normal DAG block decoders.
#[derive(Debug, Clone, Copy)]
pub struct FinalizedDagBlockBundleRlp<'a>(&'a [u8]);

impl<'a> FinalizedDagBlockBundleRlp<'a> {
    /// Wraps raw finalized DAG block bundle RLP bytes.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
}

impl TryFrom<DagBlockRlp<'_>> for DagBlock {
    type Error = anyhow::Error;

    fn try_from(value: DagBlockRlp<'_>) -> Result<Self, Self::Error> {
        Ok(decode_dag_block_rlp(&Rlp::new(value.0))?)
    }
}

fn decode_dag_block_rlp(rlp: &Rlp<'_>) -> Result<DagBlock, DecoderError> {
    let mut iter = rlp.iter();
    Ok(DagBlock {
        pivot: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_val()?,
        level: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_val()?,
        timestamp: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_val()?,
        vdf: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_val()?,
        tips: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_list()?,
        transactions: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_list()?,
        signature: {
            let rlp = iter.next().ok_or(DecoderError::RlpIsTooShort)?;
            let sig_bytes = rlp.data()?;
            if sig_bytes.len() != 65 {
                return Err(DecoderError::Custom("Invalid signature length"));
            }
            let mut signature = [0u8; 65];
            signature.copy_from_slice(sig_bytes);
            signature
        },
        gas_estimation: iter.next().ok_or(DecoderError::RlpIsTooShort)?.as_val()?,
    })
}

/// Rebuilds canonical DAG block RLP from a compact finalized period bundle.
///
/// C++ mapping: `decodeDAGBlockBundleRlp(uint64_t, dev::RLP const&)` plus
/// `DagBlock(dev::RLP const&, vec_trx_t&&)`. The period bundle stores
/// transaction hashes once, per-block transaction indexes, and compact
/// seven-field DAG block RLPs without transaction lists. This function selects
/// the compact block at `position`, resolves its transaction indexes, and
/// returns canonical eight-field DAG block RLP bytes.
pub fn reconstruct_finalized_dag_block_rlp(
    bundle: FinalizedDagBlockBundleRlp<'_>,
    position: usize,
) -> Result<Vec<u8>> {
    const BUNDLE_FIELD_COUNT: usize = 3;
    const COMPACT_DAG_BLOCK_FIELD_COUNT: usize = 7;

    let bundle_rlp = Rlp::new(bundle.0);
    if bundle_rlp.item_count()? != BUNDLE_FIELD_COUNT {
        bail!("invalid finalized DAG block bundle field count");
    }

    let ordered_transaction_hashes = bundle_rlp.at(0)?;
    let transaction_indexes = bundle_rlp.at(1)?;
    let compact_blocks = bundle_rlp.at(2)?;

    if position >= compact_blocks.item_count()? {
        bail!("finalized DAG block bundle position out of range");
    }

    let block_rlp = compact_blocks.at(position)?;
    if block_rlp.item_count()? != COMPACT_DAG_BLOCK_FIELD_COUNT {
        bail!("invalid compact finalized DAG block field count");
    }

    let index_list = transaction_indexes.at(position)?;

    let mut stream = rlp::RlpStream::new_list(8);
    for field in 0..5 {
        stream.append_raw(block_rlp.at(field)?.as_raw(), 1);
    }

    stream.begin_list(index_list.item_count()?);
    for encoded_index in index_list.iter() {
        let transaction_index: usize = encoded_index.as_val()?;
        if transaction_index >= ordered_transaction_hashes.item_count()? {
            bail!("finalized DAG block transaction index out of range");
        }
        stream.append_raw(
            ordered_transaction_hashes.at(transaction_index)?.as_raw(),
            1,
        );
    }

    for field in 5..COMPACT_DAG_BLOCK_FIELD_COUNT {
        stream.append_raw(block_rlp.at(field)?.as_raw(), 1);
    }

    Ok(stream.out().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use rlp::RlpStream;

    fn dag_block_rlp(signature: &[u8]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(8);
        stream.append(&H256::from_low_u64_be(1));
        stream.append(&10u64);
        stream.append(&1234u64);
        stream.append(&vec![1u8, 2, 3]);
        stream.begin_list(1);
        stream.append(&H256::from_low_u64_be(2));
        stream.begin_list(1);
        stream.append(&H256::from_low_u64_be(3));
        stream.append(&signature);
        stream.append(&99u64);
        stream.out().to_vec()
    }

    #[test]
    fn decodes_dag_block_rlp() {
        let signature = vec![7u8; 65];
        let block = DagBlock::try_from(DagBlockRlp::new(&dag_block_rlp(&signature))).unwrap();

        assert_eq!(block.pivot, H256::from_low_u64_be(1));
        assert_eq!(block.level, 10);
        assert_eq!(block.timestamp, 1234);
        assert_eq!(block.vdf, vec![1, 2, 3]);
        assert_eq!(block.tips, vec![H256::from_low_u64_be(2)]);
        assert_eq!(block.transactions, vec![H256::from_low_u64_be(3)]);
        assert_eq!(block.signature, [7u8; 65]);
        assert_eq!(block.gas_estimation, 99);
    }

    #[test]
    fn rejects_dag_block_rlp_with_invalid_signature_length() {
        let err = DagBlock::try_from(DagBlockRlp::new(&dag_block_rlp(&[0u8; 64]))).unwrap_err();

        assert!(err.to_string().contains("Invalid signature length"));
    }

    fn compact_dag_block_rlp() -> Vec<u8> {
        let mut stream = RlpStream::new_list(7);
        stream.append(&H256::zero());
        stream.append(&10u64);
        stream.append(&123456789u64);
        stream.append(&vec![1u8, 2, 3]);
        stream.begin_list(0);
        stream.append(&vec![0u8; 65]);
        stream.append(&1000u64);
        stream.out().to_vec()
    }

    #[test]
    fn reconstructs_finalized_dag_block_bundle_as_canonical_rlp() {
        let transactions = vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)];

        let mut bundle = RlpStream::new_list(3);
        bundle.begin_list(transactions.len());
        for hash in &transactions {
            bundle.append(hash);
        }
        bundle.begin_list(1);
        bundle.begin_list(transactions.len());
        for idx in 0..transactions.len() {
            bundle.append(&idx);
        }
        bundle.begin_list(1);
        bundle.append_raw(&compact_dag_block_rlp(), 1);
        let bundle = bundle.out();

        let full_rlp =
            reconstruct_finalized_dag_block_rlp(FinalizedDagBlockBundleRlp::new(&bundle), 0)
                .unwrap();
        let full = Rlp::new(&full_rlp);

        assert_eq!(full.item_count().unwrap(), 8);
        let decoded_transactions: Vec<H256> = full.list_at(5).unwrap();
        assert_eq!(decoded_transactions, transactions);
        assert_eq!(full.val_at::<Vec<u8>>(6).unwrap(), vec![0u8; 65]);
        assert_eq!(full.val_at::<u64>(7).unwrap(), 1000u64);
    }
}
