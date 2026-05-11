use anyhow::{Result, bail, ensure};
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use tiny_keccak::{Hasher, Keccak};

const WORD_SIZE: usize = 32;
const PILLAR_BLOCK_START_PREFIX: u64 = 32;
const PILLAR_BLOCK_STATIC_FIELDS: usize = 5;
const PILLAR_BLOCK_CHANGE_FIELDS: usize = 2;

/// Validator vote-count delta carried by a pillar block.
///
/// The address is encoded in canonical Ethereum/Taraxa byte order. The
/// `vote_count_change` value mirrors the C++ `int32_t` field and may be
/// positive or negative; Solidity compatibility encoding represents it as a
/// sign-extended 256-bit word.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ValidatorVoteCountChange {
    /// Validator account address whose vote count changed.
    pub address: H160,
    /// Signed vote-count delta since the previous pillar block.
    pub vote_count_change: i32,
}

/// Deterministic pillar block payload used by pillar-chain and bridge logic.
///
/// This type models the byte-stable fields from C++ `PillarBlock` without
/// owning manager/network/storage behavior. Its compatibility methods preserve
/// the existing Solidity payload and hash contract: the block hash is Keccak256
/// over `encode_solidity()`, and vote-count deltas are emitted in caller-owned
/// order.
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

/// Minimal pillar vote shape used by the optimized pillar-votes bundle codec.
///
/// Full vote signing and author recovery remain outside this type. The C++
/// optimized bundle stores one shared `block_hash`, one shared `period`, and a
/// list of 65-byte signatures; this struct is the materialized per-vote view of
/// that layout.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarVote {
    /// PBFT period the vote is for.
    pub period: u64,
    /// Pillar block hash voted on.
    pub block_hash: H256,
    /// Recoverable 65-byte vote signature.
    pub signature: [u8; 65],
}

/// Opaque pillar block data RLP payload.
///
/// C++ `PillarBlockData` is encoded as `[pillar_block_rlp,
/// optimized_pillar_votes_bundle_rlp]`. This wrapper preserves both nested RLP
/// payloads exactly, so storage/network consumers can validate the outer shape
/// without forcing a full pillar block or vote model before those slices land.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PillarBlockData {
    /// Raw canonical `PillarBlock` RLP bytes.
    pub pillar_block_rlp: Vec<u8>,
    /// Raw optimized pillar-votes bundle RLP bytes.
    pub pillar_votes_bundle_rlp: Vec<u8>,
}

impl PillarBlock {
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

    /// Returns the legacy pillar block hash.
    ///
    /// C++ computes this lazily as `dev::sha3(encodeSolidity())`; Rust keeps the
    /// same definition and leaves caching to future call sites that need it.
    pub fn hash(&self) -> H256 {
        keccak256(&self.encode_solidity())
    }
}

impl PillarBlockData {
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

        let mut stream = RlpStream::new_list(2);
        stream.append_raw(&self.pillar_block_rlp, 1);
        stream.append_raw(&self.pillar_votes_bundle_rlp, 1);
        Ok(stream.out().to_vec())
    }

    /// Decodes the outer two-item `PillarBlockData` shape while preserving bytes.
    ///
    /// The nested `PillarBlock` and optimized votes bundle are not interpreted
    /// here; callers can decode the votes bundle with
    /// [`decode_optimized_pillar_votes_bundle_rlp`] when needed.
    pub fn decode_rlp(bytes: &[u8]) -> Result<Self> {
        let rlp = Rlp::new(bytes);
        if rlp.item_count()? != 2 {
            bail!("pillar block data RLP must contain exactly two items");
        }
        Ok(Self {
            pillar_block_rlp: rlp.at(0)?.as_raw().to_vec(),
            pillar_votes_bundle_rlp: rlp.at(1)?.as_raw().to_vec(),
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

    let mut stream = RlpStream::new_list(3);
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
    if rlp.item_count()? != 3 {
        bail!("optimized pillar votes bundle RLP must contain exactly three items");
    }

    let block_hash: H256 = rlp.val_at(0)?;
    let period: u64 = rlp.val_at(1)?;
    let signatures = rlp.at(2)?;
    let mut votes = Vec::with_capacity(signatures.item_count()?);
    for signature_rlp in signatures.iter() {
        let signature_bytes = signature_rlp.data()?;
        if signature_bytes.len() != 65 {
            bail!("optimized pillar vote signature must be 65 bytes");
        }
        let mut signature = [0u8; 65];
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

fn u256_word(value: U256) -> [u8; WORD_SIZE] {
    value.to_big_endian()
}

fn keccak256(data: &[u8]) -> H256 {
    let mut out = [0u8; WORD_SIZE];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut out);
    H256(out)
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
    fn optimized_pillar_votes_bundle_round_trips_shared_vote_facts() {
        let votes = vec![
            PillarVote {
                period: 12,
                block_hash: H256::from_low_u64_be(34),
                signature: [0x11; 65],
            },
            PillarVote {
                period: 12,
                block_hash: H256::from_low_u64_be(34),
                signature: [0x22; 65],
            },
        ];

        let bundle = encode_optimized_pillar_votes_bundle_rlp(&votes).unwrap();
        let decoded = decode_optimized_pillar_votes_bundle_rlp(&bundle).unwrap();

        assert_eq!(decoded, votes);
    }

    #[test]
    fn pillar_block_data_preserves_nested_rlp_payloads() {
        let votes = vec![PillarVote {
            period: 12,
            block_hash: H256::from_low_u64_be(34),
            signature: [0x33; 65],
        }];
        let votes_bundle = encode_optimized_pillar_votes_bundle_rlp(&votes).unwrap();
        let mut block_rlp = RlpStream::new_list(1);
        block_rlp.append(&0xabcdu64);
        let block_rlp = block_rlp.out().to_vec();
        let data = PillarBlockData {
            pillar_block_rlp: block_rlp.clone(),
            pillar_votes_bundle_rlp: votes_bundle.clone(),
        };

        let encoded = data.encode_rlp().unwrap();
        let decoded = PillarBlockData::decode_rlp(&encoded).unwrap();

        assert_eq!(decoded.pillar_block_rlp, block_rlp);
        assert_eq!(decoded.pillar_votes_bundle_rlp, votes_bundle);
    }

    #[test]
    fn optimized_pillar_votes_bundle_rejects_malformed_inputs() {
        assert!(encode_optimized_pillar_votes_bundle_rlp(&[]).is_err());
        assert!(
            encode_optimized_pillar_votes_bundle_rlp(&[
                PillarVote {
                    period: 12,
                    block_hash: H256::from_low_u64_be(34),
                    signature: [0x11; 65],
                },
                PillarVote {
                    period: 13,
                    block_hash: H256::from_low_u64_be(34),
                    signature: [0x22; 65],
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

        assert!(PillarBlockData::decode_rlp(&malformed.out()).is_err());
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
    }
}
