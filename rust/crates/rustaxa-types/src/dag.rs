use anyhow::{Result, anyhow};
use ethereum_types::H256;
use rlp::{Decodable, DecoderError, Rlp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagBlock {
    pub pivot: H256,
    pub level: u64,
    pub timestamp: u64,
    pub vdf: Vec<u8>,
    pub tips: Vec<H256>,
    pub transactions: Vec<H256>,
    pub signature: [u8; 65],
    pub gas_estimation: u64,
}

impl DagBlock {
    pub fn from_rlp_bytes(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        Self::decode(&rlp).map_err(|e| anyhow!("RLP decode error: {}", e))
    }
}

impl Decodable for DagBlock {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
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
}
