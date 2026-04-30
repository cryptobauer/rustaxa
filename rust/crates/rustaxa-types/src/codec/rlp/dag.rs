use crate::dag::DagBlock;
use anyhow::Result;
use rlp::{DecoderError, Rlp};

#[derive(Debug, Clone, Copy)]
pub struct DagBlockRlp<'a>(&'a [u8]);

impl<'a> DagBlockRlp<'a> {
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
}
