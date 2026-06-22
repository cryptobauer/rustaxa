use crate::final_chain::DposValidatorVoteCount;
use anyhow::{Context, Result, bail, ensure};
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use tiny_keccak::{Hasher, Keccak};

const WORD_SIZE: usize = 32;
const PILLAR_BLOCK_START_PREFIX: u64 = 32;
const PILLAR_BLOCK_STATIC_FIELDS: usize = 5;
const PILLAR_BLOCK_CHANGE_FIELDS: usize = 2;
const PILLAR_BLOCK_RLP_FIELDS: usize = 6;
const PILLAR_VOTE_RLP_FIELDS: usize = 3;
const PILLAR_BLOCK_DATA_RLP_FIELDS: usize = 2;
const OPTIMIZED_PILLAR_VOTES_BUNDLE_RLP_FIELDS: usize = 3;
const CURRENT_PILLAR_BLOCK_DATA_RLP_FIELDS: usize = 2;
const SIGNATURE_SIZE: usize = 65;
const COMPACT_SIGNATURE_SIZE: usize = 64;

/// Validator vote-count delta carried by a pillar block.
///
/// The address is encoded in canonical Ethereum/Taraxa byte order. The
/// `vote_count_change` value mirrors the C++ `int32_t` field and may be
/// positive or negative. RLP uses Aleth's legacy signed-integer convention,
/// while Solidity compatibility encoding uses a sign-extended 256-bit word.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ValidatorVoteCountChange {
    /// Validator account address whose vote count changed.
    pub address: H160,
    /// Signed vote-count delta since the previous pillar block.
    pub vote_count_change: i32,
}

impl ValidatorVoteCountChange {
    /// Encodes `[address, vote_count_change]` with the legacy C++ RLP shape.
    ///
    /// Negative signed integers are encoded the same way as Aleth `RLPStream`:
    /// a one-item RLP list containing the absolute value.
    pub fn encode_rlp(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(2);
        stream.append(&self.address);
        append_signed_i32_rlp(&mut stream, self.vote_count_change);
        stream.out().to_vec()
    }

    /// Decodes `[address, vote_count_change]` from legacy pillar-block RLP.
    ///
    /// The signed delta accepts the C++ negative-integer form and rejects values
    /// outside the `int32_t` domain used by `PillarBlock`.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        decode_vote_count_change_rlp(&Rlp::new(bytes))
    }
}

/// Deterministic pillar block payload used by pillar-chain and bridge logic.
///
/// This type models the byte-stable fields from C++ `PillarBlock` without
/// owning manager/network/storage behavior. Its compatibility methods preserve
/// the existing RLP, Solidity payload, and hash contracts: the block hash is
/// Keccak256 over `encode_solidity()`, and vote-count deltas are emitted in
/// caller-owned order.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarBlock {
    /// PBFT period this pillar block summarizes.
    pub period: u64,
    /// FinalChain state root for the pillar block period.
    pub state_root: H256,
    /// Previous pillar block hash, forming the pillar chain.
    pub previous_pillar_block_hash: H256,
    /// Bridge root recorded for the bridge epoch.
    pub bridge_root: H256,
    /// Bridge epoch number.
    pub epoch: u64,
    /// Ordered validator vote-count deltas.
    pub validator_vote_count_changes: Vec<ValidatorVoteCountChange>,
}

impl PillarBlock {
    /// Encodes the canonical six-field `PillarBlock` RLP.
    ///
    /// Field order matches C++ `RLP_FIELDS_DEFINE(PillarBlock, ...)`:
    /// `period`, `state_root`, `previous_pillar_block_hash`, `bridge_root`,
    /// `epoch`, and `validator_vote_count_changes`.
    pub fn encode_rlp(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(PILLAR_BLOCK_RLP_FIELDS);
        stream.append(&self.period);
        stream.append(&self.state_root);
        stream.append(&self.previous_pillar_block_hash);
        stream.append(&self.bridge_root);
        stream.append(&self.epoch);
        stream.begin_list(self.validator_vote_count_changes.len());
        for change in &self.validator_vote_count_changes {
            stream.append_raw(&change.encode_rlp(), 1);
        }
        stream.out().to_vec()
    }

    /// Decodes the canonical six-field `PillarBlock` RLP.
    ///
    /// Invalid field counts and out-of-range signed deltas are reported as
    /// errors instead of being coerced, keeping malformed storage/network bytes
    /// visible to future Rust callers.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        if rlp.item_count()? != PILLAR_BLOCK_RLP_FIELDS {
            bail!("pillar block RLP must contain exactly six items");
        }

        let changes_rlp = rlp.at(5)?;
        let mut validator_vote_count_changes = Vec::with_capacity(changes_rlp.item_count()?);
        for change_rlp in changes_rlp.iter() {
            validator_vote_count_changes.push(decode_vote_count_change_rlp(&change_rlp)?);
        }

        Ok(Self {
            period: rlp.val_at(0)?,
            state_root: rlp.val_at(1)?,
            previous_pillar_block_hash: rlp.val_at(2)?,
            bridge_root: rlp.val_at(3)?,
            epoch: rlp.val_at(4)?,
            validator_vote_count_changes,
        })
    }

    /// Encodes the pillar block with the legacy Solidity-compatible layout.
    ///
    /// The layout matches C++ `PillarBlock::encodeSolidity()`:
    /// `[start_prefix, period, state_root, previous_hash, bridge_root, epoch,
    /// changes_offset, changes_len, [address, signed_delta]...]`, where each
    /// entry is a 32-byte ABI word and signed deltas are sign-extended.
    pub fn encode_solidity(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            (1 + PILLAR_BLOCK_STATIC_FIELDS
                + PILLAR_BLOCK_CHANGE_FIELDS
                + (self.validator_vote_count_changes.len() * PILLAR_BLOCK_CHANGE_FIELDS))
                * WORD_SIZE,
        );

        out.extend_from_slice(&u256_word(U256::from(PILLAR_BLOCK_START_PREFIX)));
        out.extend_from_slice(&u256_word(U256::from(self.period)));
        out.extend_from_slice(self.state_root.as_bytes());
        out.extend_from_slice(self.previous_pillar_block_hash.as_bytes());
        out.extend_from_slice(self.bridge_root.as_bytes());
        out.extend_from_slice(&u256_word(U256::from(self.epoch)));

        let changes_offset = (1 + PILLAR_BLOCK_STATIC_FIELDS) * WORD_SIZE;
        out.extend_from_slice(&u256_word(U256::from(changes_offset)));
        out.extend_from_slice(&u256_word(U256::from(
            self.validator_vote_count_changes.len(),
        )));

        for change in &self.validator_vote_count_changes {
            out.extend_from_slice(&address_word(change.address));
            out.extend_from_slice(&signed_i32_word(change.vote_count_change));
        }

        out
    }

    /// Decodes the legacy Solidity-compatible pillar block payload.
    ///
    /// The decoder validates the fixed offset, dynamic array length, and signed
    /// delta word shape so malformed bridge payloads cannot be mistaken for
    /// canonical pillar block data.
    pub fn decode_solidity(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len()
                >= (1 + PILLAR_BLOCK_STATIC_FIELDS + PILLAR_BLOCK_CHANGE_FIELDS) * WORD_SIZE,
            "pillar block Solidity payload is too short"
        );
        ensure!(
            bytes.len().is_multiple_of(WORD_SIZE),
            "pillar block Solidity payload length must be word-aligned"
        );

        let start_prefix = u64_word(&bytes[0..WORD_SIZE])?;
        ensure!(
            start_prefix == PILLAR_BLOCK_START_PREFIX,
            "invalid pillar block Solidity start prefix"
        );

        let changes_offset = usize_word(&bytes[6 * WORD_SIZE..7 * WORD_SIZE])?;
        ensure!(
            changes_offset == (1 + PILLAR_BLOCK_STATIC_FIELDS) * WORD_SIZE,
            "invalid pillar block vote-count changes offset"
        );
        let changes_len = usize_word(&bytes[7 * WORD_SIZE..8 * WORD_SIZE])?;
        let expected_words = changes_len
            .checked_mul(PILLAR_BLOCK_CHANGE_FIELDS)
            .and_then(|dynamic_words| {
                (1 + PILLAR_BLOCK_STATIC_FIELDS + PILLAR_BLOCK_CHANGE_FIELDS)
                    .checked_add(dynamic_words)
            })
            .context("pillar block Solidity changes length overflows usize")?;
        let expected_len = expected_words
            .checked_mul(WORD_SIZE)
            .context("pillar block Solidity payload length overflows usize")?;
        ensure!(
            bytes.len() == expected_len,
            "pillar block Solidity payload length does not match changes length"
        );

        let mut validator_vote_count_changes = Vec::with_capacity(changes_len);
        let mut offset = 8 * WORD_SIZE;
        for _ in 0..changes_len {
            let address = h160_word(&bytes[offset..offset + WORD_SIZE])?;
            let vote_count_change =
                signed_i32_from_word(&bytes[offset + WORD_SIZE..offset + (2 * WORD_SIZE)])?;
            validator_vote_count_changes.push(ValidatorVoteCountChange {
                address,
                vote_count_change,
            });
            offset += PILLAR_BLOCK_CHANGE_FIELDS * WORD_SIZE;
        }

        Ok(Self {
            period: u64_word(&bytes[WORD_SIZE..2 * WORD_SIZE])?,
            state_root: H256::from_slice(&bytes[2 * WORD_SIZE..3 * WORD_SIZE]),
            previous_pillar_block_hash: H256::from_slice(&bytes[3 * WORD_SIZE..4 * WORD_SIZE]),
            bridge_root: H256::from_slice(&bytes[4 * WORD_SIZE..5 * WORD_SIZE]),
            epoch: u64_word(&bytes[5 * WORD_SIZE..6 * WORD_SIZE])?,
            validator_vote_count_changes,
        })
    }

    /// Returns the legacy pillar block hash.
    ///
    /// C++ computes this lazily as `dev::sha3(encodeSolidity())`; Rust keeps the
    /// same definition and leaves caching to future call sites that need it.
    pub fn hash(&self) -> H256 {
        keccak256(&self.encode_solidity())
    }
}

/// Minimal pillar vote shape used by canonical vote and optimized bundle codecs.
///
/// RLP stores the full 65-byte signature; Solidity compatibility encoding
/// stores compact EIP-2098 `(r, vs)` words. The type also owns deterministic
/// author recovery because the signed/unsigned hash contract is part of the
/// canonical pillar-vote byte format.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarVote {
    /// PBFT period the vote is for.
    pub period: u64,
    /// Pillar block hash voted on.
    pub block_hash: H256,
    /// Recoverable 65-byte vote signature.
    pub signature: [u8; SIGNATURE_SIZE],
}

impl PillarVote {
    /// Encodes `[period, block_hash, signature]` with the standard C++ RLP shape.
    pub fn encode_rlp(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(PILLAR_VOTE_RLP_FIELDS);
        stream.append(&self.period);
        stream.append(&self.block_hash);
        stream.append(&self.signature.as_slice());
        stream.out().to_vec()
    }

    /// Decodes `[period, block_hash, signature]` from standard pillar vote RLP.
    ///
    /// The signature must be the full recoverable 65-byte signature used by
    /// C++ `sig_t`.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        if rlp.item_count()? != PILLAR_VOTE_RLP_FIELDS {
            bail!("pillar vote RLP must contain exactly three items");
        }
        let signature_bytes = rlp.at(2)?.data()?;
        ensure!(
            signature_bytes.len() == SIGNATURE_SIZE,
            "pillar vote RLP signature must be 65 bytes"
        );
        let mut signature = [0u8; SIGNATURE_SIZE];
        signature.copy_from_slice(signature_bytes);
        Ok(Self {
            period: rlp.val_at(0)?,
            block_hash: rlp.val_at(1)?,
            signature,
        })
    }

    /// Encodes the legacy Solidity-compatible pillar vote payload.
    ///
    /// With `include_signature = true`, the signature is compacted to the EIP-2098
    /// `(r, vs)` pair used by C++ `CompactSignatureStruct`. With
    /// `include_signature = false`, only `[period, block_hash]` is emitted.
    pub fn encode_solidity(&self, include_signature: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(if include_signature {
            4 * WORD_SIZE
        } else {
            2 * WORD_SIZE
        });
        out.extend_from_slice(&u256_word(U256::from(self.period)));
        out.extend_from_slice(self.block_hash.as_bytes());
        if include_signature {
            let (r, vs) = compact_signature(&self.signature);
            out.extend_from_slice(&r);
            out.extend_from_slice(&vs);
        }
        out
    }

    /// Returns the EIP-2098 compact signature words used by legacy JSON.
    ///
    /// Outputs:
    /// - `r`: the first 32 bytes of the recoverable signature.
    /// - `vs`: the second 32 bytes with recovery parity encoded in the high
    ///   bit, matching C++ `CompactSignatureStruct`.
    ///
    /// Edge behavior:
    /// - This is a pure formatting helper and does not validate the signature.
    ///   Public/query adapters use it to preserve existing JSON for stored
    ///   pillar votes.
    pub fn compact_signature_words(&self) -> ([u8; WORD_SIZE], [u8; WORD_SIZE]) {
        compact_signature(&self.signature)
    }

    /// Decodes the legacy Solidity-compatible pillar vote payload.
    ///
    /// Two-word payloads carry no signature and return a zeroed signature. Four
    /// word payloads decode the compact `(r, vs)` form back into the 65-byte C++
    /// recoverable signature layout.
    pub fn decode_solidity(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == 2 * WORD_SIZE || bytes.len() == 4 * WORD_SIZE,
            "pillar vote Solidity payload must contain two or four words"
        );
        let mut signature = [0u8; SIGNATURE_SIZE];
        if bytes.len() == 4 * WORD_SIZE {
            signature = expand_compact_signature(&bytes[2 * WORD_SIZE..4 * WORD_SIZE])?;
        }
        Ok(Self {
            period: u64_word(&bytes[0..WORD_SIZE])?,
            block_hash: H256::from_slice(&bytes[WORD_SIZE..2 * WORD_SIZE]),
            signature,
        })
    }

    /// Returns the legacy pillar vote hash.
    ///
    /// C++ computes `sha3(encodeSolidity(include_signature))`; callers choose
    /// the unsigned or signed form with `include_signature`.
    pub fn hash(&self, include_signature: bool) -> H256 {
        keccak256(&self.encode_solidity(include_signature))
    }

    /// Recovers the validator address that signed this pillar vote.
    ///
    /// The signed message is the legacy unsigned pillar-vote hash,
    /// `hash(false)`, and the signature is the 65-byte recoverable ECDSA
    /// signature stored in C++ `sig_t` order. Invalid signatures return `None`
    /// so callers can treat peer-supplied malformed votes as ordinary
    /// validation failures instead of panics or transport errors.
    pub fn recover_voter_address(&self) -> Option<H160> {
        recover_address(&self.signature, &self.hash(false))
    }

    /// Returns whether this vote carries a recoverable nonzero signer address.
    ///
    /// This mirrors the C++ `Vote::verifyVote()` contract: successful public-key
    /// recovery is the validity check. The recovered address is not checked
    /// against DPoS state here; consensus callers must perform eligibility and
    /// weight lookups separately for the vote period.
    pub fn verify_signature(&self) -> bool {
        self.recover_voter_address()
            .is_some_and(|address| address != H160::zero())
    }
}

/// Typed pillar block data payload.
///
/// C++ `PillarBlockData` is encoded as `[pillar_block_rlp,
/// optimized_pillar_votes_bundle_rlp]`. This type materializes the nested
/// block and optimized votes while preserving the same byte contract when
/// re-encoded.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarBlockData {
    /// Typed pillar block payload.
    pub pillar_block: PillarBlock,
    /// Materialized votes from the optimized pillar-votes bundle.
    pub pillar_votes: Vec<PillarVote>,
}

impl PillarBlockData {
    /// Encodes `[pillar_block_rlp, optimized_pillar_votes_bundle_rlp]`.
    pub fn encode_rlp(&self) -> Result<Vec<u8>> {
        let mut stream = RlpStream::new_list(PILLAR_BLOCK_DATA_RLP_FIELDS);
        stream.append_raw(&self.pillar_block.encode_rlp(), 1);
        stream.append_raw(
            &encode_optimized_pillar_votes_bundle_rlp(&self.pillar_votes)?,
            1,
        );
        Ok(stream.out().to_vec())
    }

    /// Decodes typed `PillarBlockData` from `[pillar_block, optimized_votes]`.
    ///
    /// The nested vote bundle uses the optimized C++ layout rather than a list
    /// of full `PillarVote` RLP objects.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        let raw = RawPillarBlockData::decode_rlp(bytes)?;
        Ok(Self {
            pillar_block: PillarBlock::decode_rlp(&raw.pillar_block_rlp)?,
            pillar_votes: decode_optimized_pillar_votes_bundle_rlp(&raw.pillar_votes_bundle_rlp)?,
        })
    }
}

/// Raw pillar block data RLP payload with preserved nested bytes.
///
/// This is useful for storage/transcript parity when a caller needs to inspect
/// or relay `PillarBlockData` without decoding the nested pillar block or votes.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RawPillarBlockData {
    /// Raw canonical `PillarBlock` RLP bytes.
    pub pillar_block_rlp: Vec<u8>,
    /// Raw optimized pillar-votes bundle RLP bytes.
    pub pillar_votes_bundle_rlp: Vec<u8>,
}

impl RawPillarBlockData {
    /// Encodes `[pillar_block_rlp, optimized_pillar_votes_bundle_rlp]`.
    ///
    /// The nested values must already be valid RLP items. They are appended raw
    /// to preserve canonical bytes exactly rather than decode/re-encode them.
    pub fn encode_rlp(&self) -> Result<Vec<u8>> {
        ensure!(
            !self.pillar_block_rlp.is_empty(),
            "pillar block data requires non-empty pillar block RLP"
        );
        ensure!(
            !self.pillar_votes_bundle_rlp.is_empty(),
            "pillar block data requires non-empty pillar votes bundle RLP"
        );

        let mut stream = RlpStream::new_list(PILLAR_BLOCK_DATA_RLP_FIELDS);
        stream.append_raw(&self.pillar_block_rlp, 1);
        stream.append_raw(&self.pillar_votes_bundle_rlp, 1);
        Ok(stream.out().to_vec())
    }

    /// Decodes the outer two-item `PillarBlockData` shape while preserving bytes.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        if rlp.item_count()? != PILLAR_BLOCK_DATA_RLP_FIELDS {
            bail!("pillar block data RLP must contain exactly two items");
        }
        Ok(Self {
            pillar_block_rlp: rlp.at(0)?.as_raw().to_vec(),
            pillar_votes_bundle_rlp: rlp.at(1)?.as_raw().to_vec(),
        })
    }
}

/// Validator vote-count snapshot stored with current pillar block data.
///
/// The RLP shape mirrors C++ `state_api::ValidatorVoteCount`:
/// `[address, vote_count]`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ValidatorVoteCount {
    /// Validator account address.
    pub address: H160,
    /// Eligible vote count.
    pub vote_count: u64,
}

impl ValidatorVoteCount {
    /// Encodes `[address, vote_count]` with the legacy storage RLP shape.
    pub fn encode_rlp(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(2);
        stream.append(&self.address);
        stream.append(&self.vote_count);
        stream.out().to_vec()
    }

    /// Decodes `[address, vote_count]` from the current pillar data snapshot.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        decode_validator_vote_count_rlp(&Rlp::new(bytes))
    }
}

impl From<DposValidatorVoteCount> for ValidatorVoteCount {
    fn from(value: DposValidatorVoteCount) -> Self {
        Self {
            address: H160::from_slice(&value.address),
            vote_count: value.vote_count,
        }
    }
}

impl From<ValidatorVoteCount> for DposValidatorVoteCount {
    fn from(value: ValidatorVoteCount) -> Self {
        Self {
            address: value.address.0,
            vote_count: value.vote_count,
        }
    }
}

/// Current pillar-chain state persisted by storage.
///
/// C++ stores this as `[pillar_block, vote_counts]`. It is separate from
/// `PillarBlockData`, which is the network/period-data shape containing a
/// pillar block plus optimized pillar-vote bundle.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CurrentPillarBlockDataDb {
    /// Current in-progress pillar block.
    pub pillar_block: PillarBlock,
    /// Validator vote-count snapshot used to derive future deltas.
    pub vote_counts: Vec<ValidatorVoteCount>,
}

impl CurrentPillarBlockDataDb {
    /// Encodes `[pillar_block, vote_counts]` with the legacy storage RLP shape.
    pub fn encode_rlp(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(CURRENT_PILLAR_BLOCK_DATA_RLP_FIELDS);
        stream.append_raw(&self.pillar_block.encode_rlp(), 1);
        stream.begin_list(self.vote_counts.len());
        for vote_count in &self.vote_counts {
            stream.append_raw(&vote_count.encode_rlp(), 1);
        }
        stream.out().to_vec()
    }

    /// Decodes `[pillar_block, vote_counts]` from current pillar storage bytes.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        if rlp.item_count()? != CURRENT_PILLAR_BLOCK_DATA_RLP_FIELDS {
            bail!("current pillar block data RLP must contain exactly two items");
        }
        let vote_counts_rlp = rlp.at(1)?;
        let mut vote_counts = Vec::with_capacity(vote_counts_rlp.item_count()?);
        for vote_count_rlp in vote_counts_rlp.iter() {
            vote_counts.push(decode_validator_vote_count_rlp(&vote_count_rlp)?);
        }
        Ok(Self {
            pillar_block: PillarBlock::decode_rlp(rlp.at(0)?.as_raw())?,
            vote_counts,
        })
    }
}

/// Encodes C++'s optimized pillar-votes bundle layout.
///
/// The bundle is `[block_hash, period, [signature...]]`; it intentionally does
/// not contain full `PillarVote` RLP objects. All votes must target the same
/// block hash and period because those values are stored once at bundle level.
pub fn encode_optimized_pillar_votes_bundle_rlp(votes: &[PillarVote]) -> Result<Vec<u8>> {
    let Some(reference_vote) = votes.last() else {
        bail!("optimized pillar votes bundle requires at least one vote");
    };
    ensure!(
        votes
            .iter()
            .all(|vote| vote.block_hash == reference_vote.block_hash
                && vote.period == reference_vote.period),
        "optimized pillar votes bundle requires matching period and block hash"
    );

    let mut stream = RlpStream::new_list(OPTIMIZED_PILLAR_VOTES_BUNDLE_RLP_FIELDS);
    stream.append(&reference_vote.block_hash);
    stream.append(&reference_vote.period);
    stream.begin_list(votes.len());
    for vote in votes {
        stream.append(&vote.signature.as_slice());
    }
    Ok(stream.out().to_vec())
}

/// Decodes C++'s optimized pillar-votes bundle layout into per-vote records.
///
/// Each signature item must be exactly 65 bytes, matching the C++ `sig_t`
/// recoverable signature shape.
pub fn decode_optimized_pillar_votes_bundle_rlp(bytes: &[u8]) -> Result<Vec<PillarVote>> {
    let rlp = Rlp::new(bytes);
    if rlp.item_count()? != OPTIMIZED_PILLAR_VOTES_BUNDLE_RLP_FIELDS {
        bail!("optimized pillar votes bundle RLP must contain exactly three items");
    }

    let block_hash: H256 = rlp.val_at(0)?;
    let period: u64 = rlp.val_at(1)?;
    let signatures = rlp.at(2)?;
    let mut votes = Vec::with_capacity(signatures.item_count()?);
    for signature_rlp in signatures.iter() {
        let signature_bytes = signature_rlp.data()?;
        if signature_bytes.len() != SIGNATURE_SIZE {
            bail!("optimized pillar vote signature must be 65 bytes");
        }
        let mut signature = [0u8; SIGNATURE_SIZE];
        signature.copy_from_slice(signature_bytes);
        votes.push(PillarVote {
            period,
            block_hash,
            signature,
        });
    }
    Ok(votes)
}

fn address_word(address: H160) -> [u8; WORD_SIZE] {
    let mut out = [0u8; WORD_SIZE];
    out[12..].copy_from_slice(address.as_bytes());
    out
}

fn signed_i32_word(value: i32) -> [u8; WORD_SIZE] {
    let mut out = if value < 0 {
        [0xffu8; WORD_SIZE]
    } else {
        [0u8; WORD_SIZE]
    };
    out[WORD_SIZE - 4..].copy_from_slice(&value.to_be_bytes());
    out
}

fn signed_i32_from_word(word: &[u8]) -> Result<i32> {
    ensure!(word.len() == WORD_SIZE, "signed int word must be 32 bytes");
    let mut low = [0u8; 4];
    low.copy_from_slice(&word[WORD_SIZE - 4..]);
    let value = i32::from_be_bytes(low);
    let expected_prefix = if value < 0 { 0xff } else { 0x00 };
    ensure!(
        word[..WORD_SIZE - 4]
            .iter()
            .all(|byte| *byte == expected_prefix),
        "signed int word is not canonical int32 sign extension"
    );
    Ok(value)
}

fn append_signed_i32_rlp(stream: &mut RlpStream, value: i32) {
    if value < 0 {
        stream.begin_list(1);
        stream.append(&value.unsigned_abs());
    } else {
        stream.append(&(value as u32));
    }
}

fn decode_signed_i32_rlp(rlp: &Rlp<'_>) -> Result<i32> {
    if rlp.is_list() {
        ensure!(
            rlp.item_count()? == 1,
            "negative signed RLP integer must be a one-item list"
        );
        let magnitude: u32 = rlp.val_at(0)?;
        ensure!(
            magnitude <= (i32::MAX as u32) + 1,
            "negative signed RLP integer is outside int32 range"
        );
        if magnitude == (i32::MAX as u32) + 1 {
            Ok(i32::MIN)
        } else {
            Ok(-(magnitude as i32))
        }
    } else {
        let value: u32 = rlp.as_val()?;
        ensure!(
            value <= i32::MAX as u32,
            "positive signed RLP integer is outside int32 range"
        );
        Ok(value as i32)
    }
}

fn u256_word(value: U256) -> [u8; WORD_SIZE] {
    value.to_big_endian()
}

fn usize_word(word: &[u8]) -> Result<usize> {
    let value = u64_word(word)?;
    usize::try_from(value).context("word value does not fit usize")
}

fn u64_word(word: &[u8]) -> Result<u64> {
    ensure!(word.len() == WORD_SIZE, "uint64 word must be 32 bytes");
    ensure!(
        word[..WORD_SIZE - 8].iter().all(|byte| *byte == 0),
        "uint64 word has non-zero high bytes"
    );
    let mut low = [0u8; 8];
    low.copy_from_slice(&word[WORD_SIZE - 8..]);
    Ok(u64::from_be_bytes(low))
}

fn h160_word(word: &[u8]) -> Result<H160> {
    ensure!(word.len() == WORD_SIZE, "address word must be 32 bytes");
    ensure!(
        word[..12].iter().all(|byte| *byte == 0),
        "address word has non-zero padding"
    );
    Ok(H160::from_slice(&word[12..]))
}

fn compact_signature(signature: &[u8; SIGNATURE_SIZE]) -> ([u8; WORD_SIZE], [u8; WORD_SIZE]) {
    let mut r = [0u8; WORD_SIZE];
    r.copy_from_slice(&signature[..WORD_SIZE]);
    let mut vs = [0u8; WORD_SIZE];
    vs.copy_from_slice(&signature[WORD_SIZE..2 * WORD_SIZE]);
    if signature[64] & 1 == 1 {
        vs[0] |= 0x80;
    } else {
        vs[0] &= 0x7f;
    }
    (r, vs)
}

fn expand_compact_signature(compact_signature: &[u8]) -> Result<[u8; SIGNATURE_SIZE]> {
    ensure!(
        compact_signature.len() == COMPACT_SIGNATURE_SIZE,
        "compact signature must be 64 bytes"
    );
    let mut signature = [0u8; SIGNATURE_SIZE];
    signature[..WORD_SIZE].copy_from_slice(&compact_signature[..WORD_SIZE]);
    signature[WORD_SIZE..2 * WORD_SIZE].copy_from_slice(&compact_signature[WORD_SIZE..]);
    signature[64] = signature[WORD_SIZE] >> 7;
    signature[WORD_SIZE] &= 0x7f;
    Ok(signature)
}

fn decode_vote_count_change_rlp(rlp: &Rlp<'_>) -> Result<ValidatorVoteCountChange> {
    if rlp.item_count()? != 2 {
        bail!("validator vote-count change RLP must contain exactly two items");
    }
    Ok(ValidatorVoteCountChange {
        address: rlp.val_at(0)?,
        vote_count_change: decode_signed_i32_rlp(&rlp.at(1)?)?,
    })
}

fn decode_validator_vote_count_rlp(rlp: &Rlp<'_>) -> Result<ValidatorVoteCount> {
    if rlp.item_count()? != 2 {
        bail!("validator vote-count RLP must contain exactly two items");
    }
    Ok(ValidatorVoteCount {
        address: rlp.val_at(0)?,
        vote_count: rlp.val_at(1)?,
    })
}

fn keccak256(data: &[u8]) -> H256 {
    let mut out = [0u8; WORD_SIZE];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut out);
    H256(out)
}

fn recover_address(signature: &[u8; SIGNATURE_SIZE], message_hash: &H256) -> Option<H160> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    if signature[64] > 3 {
        return None;
    }
    let recovery_id = RecoveryId::try_from(signature[64]).ok()?;
    let signature = Signature::try_from(&signature[..64]).ok()?;
    let recovered_key =
        VerifyingKey::recover_from_prehash(message_hash.as_bytes(), &signature, recovery_id)
            .ok()?;
    let uncompressed = recovered_key.to_encoded_point(false);
    let public_key_hash = keccak256(&uncompressed.as_bytes()[1..]);
    Some(H160::from_slice(&public_key_hash.as_bytes()[12..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => panic!("invalid hex nibble"),
        }
    }

    fn hex_bytes(value: &str) -> Vec<u8> {
        let chunks = value.as_bytes().chunks_exact(2);
        assert!(chunks.remainder().is_empty());
        chunks
            .map(|chunk| (hex_nibble(chunk[0]) << 4) | hex_nibble(chunk[1]))
            .collect()
    }

    fn h256_hex(value: &str) -> H256 {
        H256(hex_bytes(value).try_into().unwrap())
    }

    fn signature(byte: u8, recovery_id: u8) -> [u8; SIGNATURE_SIZE] {
        let mut signature = [byte; SIGNATURE_SIZE];
        signature[64] = recovery_id;
        signature
    }

    fn signed_pillar_vote(seed: u8, period: u64, block_hash: H256) -> (PillarVote, H160) {
        use k256::ecdsa::SigningKey;

        let signing_key = SigningKey::from_slice(&[seed; WORD_SIZE]).unwrap();
        let mut vote = PillarVote {
            period,
            block_hash,
            signature: [0u8; SIGNATURE_SIZE],
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(vote.hash(false).as_bytes())
            .unwrap();
        let signature_bytes = signature.to_bytes();
        vote.signature[..2 * WORD_SIZE].copy_from_slice(&signature_bytes);
        vote.signature[64] = recovery_id.to_byte();

        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        let voter = H160::from_slice(&public_key_hash.as_bytes()[12..]);

        (vote, voter)
    }

    fn pillar_fixture() -> PillarBlock {
        PillarBlock {
            period: 11,
            state_root: H256::from_low_u64_be(22),
            previous_pillar_block_hash: H256::from_low_u64_be(33),
            bridge_root: H256::from_low_u64_be(44),
            epoch: 55,
            validator_vote_count_changes: vec![
                ValidatorVoteCountChange {
                    address: H160::from_low_u64_be(1),
                    vote_count_change: -1,
                },
                ValidatorVoteCountChange {
                    address: H160::from_low_u64_be(2),
                    vote_count_change: 2,
                },
                ValidatorVoteCountChange {
                    address: H160::from_low_u64_be(3),
                    vote_count_change: -3,
                },
                ValidatorVoteCountChange {
                    address: H160::from_low_u64_be(4),
                    vote_count_change: 4,
                },
                ValidatorVoteCountChange {
                    address: H160::from_low_u64_be(5),
                    vote_count_change: -5,
                },
            ],
        }
    }

    #[test]
    fn pillar_block_solidity_encoding_matches_cpp_fixture() {
        let expected = hex_bytes(concat!(
            "0000000000000000000000000000000000000000000000000000000000000020",
            "000000000000000000000000000000000000000000000000000000000000000b",
            "0000000000000000000000000000000000000000000000000000000000000016",
            "0000000000000000000000000000000000000000000000000000000000000021",
            "000000000000000000000000000000000000000000000000000000000000002c",
            "0000000000000000000000000000000000000000000000000000000000000037",
            "00000000000000000000000000000000000000000000000000000000000000c0",
            "0000000000000000000000000000000000000000000000000000000000000005",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000000000000000000000000000000000000000003",
            "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffd",
            "0000000000000000000000000000000000000000000000000000000000000004",
            "0000000000000000000000000000000000000000000000000000000000000004",
            "0000000000000000000000000000000000000000000000000000000000000005",
            "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffb"
        ));

        assert_eq!(pillar_fixture().encode_solidity(), expected);
    }

    #[test]
    fn pillar_block_hash_matches_cpp_solidity_hash_definition() {
        assert_eq!(
            pillar_fixture().hash(),
            h256_hex("d683e69bd0a4b315549bcc3d171b4ab25c277774131c4a37b8ae5815b9cd87b4")
        );
    }

    #[test]
    fn validator_vote_count_change_rlp_round_trips_negative() {
        let change = ValidatorVoteCountChange {
            address: H160::from_low_u64_be(9),
            vote_count_change: -123,
        };

        let decoded = ValidatorVoteCountChange::decode_rlp(&change.encode_rlp()).unwrap();

        assert_eq!(decoded, change);
    }

    #[test]
    fn validator_vote_count_change_rlp_rejects_shape_mismatch() {
        let mut malformed = RlpStream::new_list(1);
        malformed.append(&H160::zero());

        assert!(ValidatorVoteCountChange::decode_rlp(&malformed.out()).is_err());
    }

    #[test]
    fn pillar_block_rlp_round_trips_with_mixed_deltas() {
        let block = pillar_fixture();

        let decoded = PillarBlock::decode_rlp(&block.encode_rlp()).unwrap();

        assert_eq!(decoded, block);
    }

    #[test]
    fn pillar_block_solidity_round_trips_and_rejects_bad_layout() {
        let block = pillar_fixture();

        let decoded = PillarBlock::decode_solidity(&block.encode_solidity()).unwrap();

        assert_eq!(decoded, block);

        let mut bad_offset = block.encode_solidity();
        bad_offset[(6 * WORD_SIZE) + WORD_SIZE - 1] = 0xbf;
        assert!(PillarBlock::decode_solidity(&bad_offset).is_err());

        let mut bad_sign_extension = block.encode_solidity();
        bad_sign_extension[9 * WORD_SIZE] = 0;
        assert!(PillarBlock::decode_solidity(&bad_sign_extension).is_err());

        assert!(PillarBlock::decode_solidity(&block.encode_solidity()[..13]).is_err());
    }

    #[test]
    fn pillar_vote_rlp_round_trips() {
        let vote = PillarVote {
            period: 12,
            block_hash: H256::from_low_u64_be(34),
            signature: signature(0x44, 1),
        };

        let decoded = PillarVote::decode_rlp(&vote.encode_rlp()).unwrap();

        assert_eq!(decoded, vote);
    }

    #[test]
    fn pillar_vote_recovers_voter_address_from_signature() {
        let (vote, voter) = signed_pillar_vote(0x21, 12, H256::from_low_u64_be(34));

        assert_eq!(vote.recover_voter_address(), Some(voter));
        assert!(vote.verify_signature());
    }

    #[test]
    fn pillar_vote_rejects_out_of_range_recovery_id() {
        let (mut vote, _) = signed_pillar_vote(0x22, 12, H256::from_low_u64_be(34));
        vote.signature[64] = 4;

        assert_eq!(vote.recover_voter_address(), None);
        assert!(!vote.verify_signature());
    }

    #[test]
    fn pillar_vote_solidity_compact_signature_matches_cpp_fixtures() {
        let mut first_signature = [0u8; SIGNATURE_SIZE];
        first_signature[..WORD_SIZE].copy_from_slice(&hex_bytes(
            "68a020a209d3d56c46f38cc50a33f704f4a9a10a59377f8dd762ac66910e9b90",
        ));
        first_signature[WORD_SIZE..2 * WORD_SIZE].copy_from_slice(&hex_bytes(
            "7e865ad05c4035ab5792787d4a0297a43617ae897930a6fe4d822b8faea52064",
        ));
        let first_vote = PillarVote {
            period: 12,
            block_hash: H256::from_low_u64_be(34),
            signature: first_signature,
        };
        assert_eq!(
            &first_vote.encode_solidity(true)[2 * WORD_SIZE..],
            &hex_bytes(concat!(
                "68a020a209d3d56c46f38cc50a33f704f4a9a10a59377f8dd762ac66910e9b90",
                "7e865ad05c4035ab5792787d4a0297a43617ae897930a6fe4d822b8faea52064"
            ))
        );

        let mut second_signature = [0u8; SIGNATURE_SIZE];
        second_signature[..WORD_SIZE].copy_from_slice(&hex_bytes(
            "9328da16089fcba9bececa81663203989f2df5fe1faa6291a45381c81bd17f76",
        ));
        second_signature[WORD_SIZE..2 * WORD_SIZE].copy_from_slice(&hex_bytes(
            "139c6d6b623b42da56557e5e734a43dc83345ddfadec52cbe24d0cc64f550793",
        ));
        second_signature[64] = 1;
        let second_vote = PillarVote {
            period: 12,
            block_hash: H256::from_low_u64_be(34),
            signature: second_signature,
        };
        assert_eq!(
            &second_vote.encode_solidity(true)[2 * WORD_SIZE..],
            &hex_bytes(concat!(
                "9328da16089fcba9bececa81663203989f2df5fe1faa6291a45381c81bd17f76",
                "939c6d6b623b42da56557e5e734a43dc83345ddfadec52cbe24d0cc64f550793"
            ))
        );
        assert_eq!(
            PillarVote::decode_solidity(&second_vote.encode_solidity(true))
                .unwrap()
                .signature,
            second_signature
        );
    }

    #[test]
    fn pillar_vote_solidity_round_trips_without_signature() {
        let vote = PillarVote {
            period: 12,
            block_hash: H256::from_low_u64_be(34),
            signature: signature(0x55, 1),
        };

        let encoded = vote.encode_solidity(false);
        let decoded = PillarVote::decode_solidity(&encoded).unwrap();

        assert_eq!(encoded.len(), 2 * WORD_SIZE);
        assert_eq!(decoded.period, vote.period);
        assert_eq!(decoded.block_hash, vote.block_hash);
        assert_eq!(decoded.signature, [0u8; SIGNATURE_SIZE]);
    }

    #[test]
    fn optimized_pillar_votes_bundle_round_trips_shared_vote_facts() {
        let votes = vec![
            PillarVote {
                period: 12,
                block_hash: H256::from_low_u64_be(34),
                signature: signature(0x11, 0),
            },
            PillarVote {
                period: 12,
                block_hash: H256::from_low_u64_be(34),
                signature: signature(0x22, 1),
            },
        ];

        let bundle = encode_optimized_pillar_votes_bundle_rlp(&votes).unwrap();
        let decoded = decode_optimized_pillar_votes_bundle_rlp(&bundle).unwrap();

        assert_eq!(decoded, votes);
    }

    #[test]
    fn pillar_block_data_round_trips_typed_and_raw_payloads() {
        let votes = vec![
            PillarVote {
                period: 12,
                block_hash: H256::from_low_u64_be(34),
                signature: signature(0x33, 0),
            },
            PillarVote {
                period: 12,
                block_hash: H256::from_low_u64_be(34),
                signature: signature(0x44, 1),
            },
        ];
        let data = PillarBlockData {
            pillar_block: pillar_fixture(),
            pillar_votes: votes,
        };

        let encoded = data.encode_rlp().unwrap();
        let decoded = PillarBlockData::decode_rlp(&encoded).unwrap();
        let raw = RawPillarBlockData::decode_rlp(&encoded).unwrap();

        assert_eq!(decoded, data);
        assert_eq!(raw.pillar_block_rlp, data.pillar_block.encode_rlp());
        assert_eq!(
            raw.pillar_votes_bundle_rlp,
            encode_optimized_pillar_votes_bundle_rlp(&data.pillar_votes).unwrap()
        );
        assert_eq!(
            RawPillarBlockData::decode_rlp(&raw.encode_rlp().unwrap()).unwrap(),
            raw
        );
    }

    #[test]
    fn optimized_pillar_votes_bundle_rejects_malformed_inputs() {
        assert!(encode_optimized_pillar_votes_bundle_rlp(&[]).is_err());
        assert!(
            encode_optimized_pillar_votes_bundle_rlp(&[
                PillarVote {
                    period: 12,
                    block_hash: H256::from_low_u64_be(34),
                    signature: signature(0x11, 0),
                },
                PillarVote {
                    period: 13,
                    block_hash: H256::from_low_u64_be(34),
                    signature: signature(0x22, 1),
                },
            ])
            .is_err()
        );

        let mut malformed = RlpStream::new_list(3);
        malformed.append(&H256::from_low_u64_be(34));
        malformed.append(&12u64);
        malformed.begin_list(1);
        malformed.append(&[0x44u8; 64].as_slice());

        assert!(decode_optimized_pillar_votes_bundle_rlp(&malformed.out()).is_err());
    }

    #[test]
    fn pillar_block_data_rejects_invalid_outer_shape() {
        let mut malformed = RlpStream::new_list(1);
        malformed.append(&1u64);
        let malformed = malformed.out();

        assert!(PillarBlockData::decode_rlp(&malformed).is_err());
        assert!(RawPillarBlockData::decode_rlp(&malformed).is_err());
    }

    #[test]
    fn current_pillar_block_data_db_round_trips() {
        let data = CurrentPillarBlockDataDb {
            pillar_block: pillar_fixture(),
            vote_counts: vec![
                ValidatorVoteCount {
                    address: H160::from_low_u64_be(1),
                    vote_count: 7,
                },
                ValidatorVoteCount {
                    address: H160::from_low_u64_be(2),
                    vote_count: 11,
                },
            ],
        };

        let decoded = CurrentPillarBlockDataDb::decode_rlp(&data.encode_rlp()).unwrap();

        assert_eq!(decoded, data);
    }

    #[test]
    fn current_pillar_block_data_db_rejects_malformed_entries() {
        let mut malformed = RlpStream::new_list(2);
        malformed.append_raw(&pillar_fixture().encode_rlp(), 1);
        malformed.begin_list(1);
        malformed.begin_list(1);
        malformed.append(&H160::zero());

        assert!(CurrentPillarBlockDataDb::decode_rlp(&malformed.out()).is_err());
    }

    #[test]
    fn empty_pillar_block_encodes_dynamic_array_header_without_entries() {
        let block = PillarBlock {
            period: 123,
            state_root: H256::from_low_u64_be(456),
            previous_pillar_block_hash: H256::from_low_u64_be(789),
            bridge_root: H256::from_low_u64_be(789),
            epoch: 0,
            validator_vote_count_changes: Vec::new(),
        };

        let encoded = block.encode_solidity();

        assert_eq!(
            encoded.len(),
            (1 + PILLAR_BLOCK_STATIC_FIELDS + 2) * WORD_SIZE
        );
        assert_eq!(
            &encoded[6 * WORD_SIZE..7 * WORD_SIZE],
            &u256_word(U256::from(192usize))
        );
        assert_eq!(
            &encoded[7 * WORD_SIZE..8 * WORD_SIZE],
            &u256_word(U256::zero())
        );
        assert_eq!(PillarBlock::decode_solidity(&encoded).unwrap(), block);
    }
}
