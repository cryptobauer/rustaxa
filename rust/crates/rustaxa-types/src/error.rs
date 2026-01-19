use thiserror::Error;

#[derive(Error, Debug)]
pub enum TypesError {
    #[error("RLP decode error: {0}")]
    RlpDecode(#[from] rlp::DecoderError),

    #[error("Other error: {0}")]
    Other(String),
}
