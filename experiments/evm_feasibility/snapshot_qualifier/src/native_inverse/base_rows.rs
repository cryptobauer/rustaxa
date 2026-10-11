//! Pure interfaces for a later saved-row ledger. No reader, callback, membership,
//! physical status, catalog completeness or union authority is provided here.
//! Existing private codecs perform canonical byte-for-byte re-encoding. Every
//! public raw decoder checks the hard row cap before parsing or allocation.

use super::*;

/// Hard input limit for these interfaces only; existing analyzer limits are unchanged.
pub const MAX_BASE_CODEC_ROW_BYTES: usize = 4096;

/// Retained validator encoding. Extended rows always carry an undelegation count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatorEncoding {
    /// Four-field ValidatorV1 row, with no undelegation count.
    V1,
    /// Two-field row containing ValidatorV1 and its u16 undelegation count.
    ExtendedV1,
}

/// Canonical validator fields; stake has arbitrary width within the input cap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Validator {
    /// Exact retained representation, distinct from protocol version authority.
    pub encoding: ValidatorEncoding,
    /// Unsigned stake, with no 256-bit restriction.
    pub stake: BigUint,
    /// Stored commission, bounded to u16 by the codec.
    pub commission: u16,
    /// Stored last change period, bounded to u64.
    pub last_commission_change: u64,
    /// Stored reward graph cursor, bounded to u64; no membership inference.
    pub reward_head: u64,
    /// Present exactly for ExtendedV1; zero remains Some(0).
    pub undelegations_count: Option<u16>,
}

/// Exact metadata bytes; empty or leading-zero strings are valid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Info {
    /// Description bytes, without UTF-8 or integer interpretation.
    pub description: Vec<u8>,
    /// Endpoint bytes, without UTF-8 or integer interpretation.
    pub endpoint: Vec<u8>,
}

/// Two canonical unsigned validator rewards, arbitrary width within the row cap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rewards {
    /// Stored delegator rewards; not a computed distribution.
    pub delegator: BigUint,
    /// Stored commission rewards; not a computed distribution.
    pub commission: BigUint,
}

/// Canonical graph node values, without graph completeness authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardNode {
    /// Arbitrary-width unsigned reward per stake.
    pub reward_per_stake: BigUint,
    /// Stored u32 reference count; does not authorize discovery.
    pub count: u32,
}

/// Canonical delegation row, without iterable membership authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delegation {
    /// Arbitrary-width unsigned stake.
    pub stake: BigUint,
    /// Stored u64 graph cursor.
    pub last_updated: u64,
}

/// Supported fixed global fields. Field selection is separate from value encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ScalarField {
    /// Field 4: compact unsigned u64.
    EligibleVotes = 4,
    /// Field 5: arbitrary-width compact unsigned integer.
    TotalStake = 5,
    /// Field 6: arbitrary-width compact unsigned integer.
    MintedTokens = 6,
    /// Field 7: arbitrary-width compact unsigned integer.
    TotalSupply = 7,
    /// Field 8: canonical RLP u64.
    Yield = 8,
}
impl TryFrom<u8> for ScalarField {
    type Error = anyhow::Error;
    /// Reject unsupported field bytes; this conversion never reads state.
    fn try_from(value: u8) -> Result<Self> {
        match value {
            4 => Ok(Self::EligibleVotes),
            5 => Ok(Self::TotalStake),
            6 => Ok(Self::MintedTokens),
            7 => Ok(Self::TotalSupply),
            8 => Ok(Self::Yield),
            _ => bail!("unsupported base scalar field {value}"),
        }
    }
}

/// Typed scalar value; fixed-width values cannot be confused with amount fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scalar {
    /// Compact field 4 value.
    EligibleVotes(u64),
    /// Compact field 5 value.
    TotalStake(BigUint),
    /// Compact field 6 value, only when a row is supplied; no tombstone default.
    MintedTokens(BigUint),
    /// Compact field 7 value.
    TotalSupply(BigUint),
    /// RLP field 8 value.
    Yield(u64),
}

fn bounded(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_BASE_CODEC_ROW_BYTES,
        "base codec row byte cap"
    );
    Ok(())
}

/// Decode an exact ValidatorV1 or nested extended row, after the 4096-byte guard.
/// Reject malformed/noncanonical/trailing RLP and u16/u64 overflow. Amount width
/// is bounded only by the row cap. The encoding/count invariant is preserved.
pub fn validator(bytes: &[u8]) -> Result<Validator> {
    bounded(bytes)?;
    let row = decode_validator(bytes)?;
    let encoding = match (row.encoding, row.undelegations_count) {
        ("validator_v1", None) => ValidatorEncoding::V1,
        ("embedded_validator_v1_with_count", Some(_)) => ValidatorEncoding::ExtendedV1,
        _ => bail!("retained validator encoding/count mismatch"),
    };
    Ok(Validator {
        encoding,
        stake: row.stake,
        commission: row.commission,
        last_commission_change: row.last_commission_change,
        reward_head: row.reward_head,
        undelegations_count: row.undelegations_count,
    })
}
/// Decode exactly two metadata byte strings after the 4096-byte guard.
/// Reject list fields, malformed/noncanonical/trailing RLP; preserve bytes exactly.
pub fn info(bytes: &[u8]) -> Result<Info> {
    bounded(bytes)?;
    let (description, endpoint) = decode_pair_bytes(bytes, "base validator info")?;
    Ok(Info {
        description,
        endpoint,
    })
}
/// Decode two arbitrary-width unsigned rewards after the 4096-byte guard.
/// Reject list integers, leading zeros and malformed/noncanonical/trailing RLP.
pub fn rewards(bytes: &[u8]) -> Result<Rewards> {
    bounded(bytes)?;
    let (delegator, commission) = decode_pair_uints(bytes, "base validator rewards")?;
    Ok(Rewards {
        delegator,
        commission,
    })
}
/// Decode reward-per-stake and u32 count after the 4096-byte guard.
/// Reject noncanonical/trailing RLP, invalid integers and count overflow.
pub fn reward_node(bytes: &[u8]) -> Result<RewardNode> {
    bounded(bytes)?;
    let (reward_per_stake, count) = decode_reward_node(bytes)?;
    Ok(RewardNode {
        reward_per_stake,
        count,
    })
}
/// Decode arbitrary-width stake and u64 cursor after the 4096-byte guard.
/// Reject noncanonical/trailing RLP, invalid integers and cursor overflow.
pub fn delegation(bytes: &[u8]) -> Result<Delegation> {
    bounded(bytes)?;
    let row = decode_delegation(bytes)?;
    Ok(Delegation {
        stake: row.stake,
        last_updated: row.last_updated,
    })
}
/// Decode one supported global field after the 4096-byte guard. Compact zero is
/// empty bytes; RLP yield zero is 80. Reject leading zeros, width overflow, and
/// malformed/noncanonical/trailing yield RLP. Never infer presence from bytes.
pub fn scalar(field: ScalarField, bytes: &[u8]) -> Result<Scalar> {
    bounded(bytes)?;
    match field {
        ScalarField::EligibleVotes => Ok(Scalar::EligibleVotes(decode_compact_u64(
            bytes,
            "base votes",
        )?)),
        ScalarField::Yield => Ok(Scalar::Yield(decode_rlp_u64(bytes, "base yield")?)),
        _ => {
            canonical_uint(bytes, "base amount")?;
            let amount = BigUint::from_bytes_be(bytes);
            Ok(match field {
                ScalarField::TotalStake => Scalar::TotalStake(amount),
                ScalarField::MintedTokens => Scalar::MintedTokens(amount),
                ScalarField::TotalSupply => Scalar::TotalSupply(amount),
                _ => unreachable!("fixed-width fields handled above"),
            })
        }
    }
}
/// Decode exact address20 after the byte guard; empty and wrong widths fail.
pub fn address(bytes: &[u8]) -> Result<[u8; 20]> {
    bounded(bytes)?;
    bytes
        .try_into()
        .context("base address must contain 20 bytes")
}
/// Decode exact VRF32 after the byte guard; empty and wrong widths fail.
pub fn vrf(bytes: &[u8]) -> Result<[u8; 32]> {
    bounded(bytes)?;
    bytes.try_into().context("base VRF must contain 32 bytes")
}
/// Decode exactly four LE bytes after the byte guard. Zero is valid; membership
/// and one-based position bounds belong to the caller, not this value codec.
pub fn index(bytes: &[u8]) -> Result<u32> {
    bounded(bytes)?;
    decode_le_u32(bytes, "base index")
}

/// Explicit validator payload field selector, with no arbitrary prefix admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ValidatorField {
    /// Validator record under 00 00.
    Record = 0,
    /// Metadata under 00 01.
    Info = 1,
    /// Rewards under 00 02.
    Rewards = 2,
    /// Owner under 00 03; validator is the key suffix.
    Owner = 3,
    /// VRF under 00 04.
    Vrf = 4,
}
/// Derive K(00 || field || validator20). No membership or state access.
pub fn validator_key(field: ValidatorField, validator: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[0, field as u8], &validator])
}
/// Derive K(01 || validator20 || compact_BE(cursor)). Zero has empty cursor bytes.
/// A supplied cursor grants no graph discovery or membership authority.
pub fn reward_node_key(validator: [u8; 20], cursor: u64) -> ConcreteStorageKey {
    storage_key(&[&[1], &validator, &compact_u64_bytes(cursor)])
}
/// Derive K(02 01 || delegator20 || 02 || position_LE4), using the retained alias.
/// Position bounds and one-based membership belong to the caller.
pub fn delegation_item_key(delegator: [u8; 20], position: u32) -> ConcreteStorageKey {
    iterable_item_key_for(&delegator_validators_prefix(delegator), position)
}
/// Derive K(02 01 || delegator20 || 02 || validator20), using the retained alias.
/// No reverse-map membership or enumeration authority is supplied.
pub fn delegation_reverse_key(delegator: [u8; 20], validator: [u8; 20]) -> ConcreteStorageKey {
    iterable_position_key_for(&delegator_validators_prefix(delegator), &validator)
}
/// Derive K(02 00 || validator20 || delegator20); argument order is explicit.
pub fn delegation_record_key(validator: [u8; 20], delegator: [u8; 20]) -> ConcreteStorageKey {
    storage_key(&[&[2, 0], &validator, &delegator])
}
/// Derive the supported single-field logical key K(field4..8), without state access.
pub fn scalar_key(field: ScalarField) -> ConcreteStorageKey {
    storage_key(&[&[field as u8]])
}
/// Return K(logical_key32), the trie path; not the preimage's first hash.
pub fn trie_path(key: ConcreteStorageKey) -> [u8; 32] {
    keccak256(&key.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }
    #[test]
    fn literal_validator_variants_and_integer_boundaries() {
        let plain = validator(&bytes("c401020304")).unwrap();
        assert_eq!(
            plain,
            Validator {
                encoding: ValidatorEncoding::V1,
                stake: 1u32.into(),
                commission: 2,
                last_commission_change: 3,
                reward_head: 4,
                undelegations_count: None
            }
        );
        let extended = validator(&bytes("c6c40102030480")).unwrap();
        assert_eq!(extended.encoding, ValidatorEncoding::ExtendedV1);
        assert_eq!(extended.undelegations_count, Some(0));
        let max = validator(&bytes("d68082ffff88ffffffffffffffff88ffffffffffffffff")).unwrap();
        assert_eq!(max.commission, u16::MAX);
        assert_eq!(max.last_commission_change, u64::MAX);
        assert_eq!(max.reward_head, u64::MAX);
        assert_eq!(
            validator(&bytes("c8c40102030482ffff"))
                .unwrap()
                .undelegations_count,
            Some(u16::MAX)
        );
        for bad in [
            "c50102030405",
            "c5c301020380",
            "c701830100000304",
            "c801020389010000000000000000",
            "c40102030400",
            "c48200010304",
            "c4c0020304",
        ] {
            assert!(validator(&bytes(bad)).is_err(), "{bad}");
        }
    }
    #[test]
    fn byte_metadata_and_arbitrary_width_unsigned_values() {
        assert_eq!(
            info(&bytes("c480820001")).unwrap(),
            Info {
                description: vec![],
                endpoint: vec![0, 1]
            }
        );
        assert_eq!(
            rewards(&bytes("c28080")).unwrap(),
            Rewards {
                delegator: BigUint::default(),
                commission: BigUint::default()
            }
        );
        assert_eq!(
            rewards(&bytes("c20102")).unwrap(),
            Rewards {
                delegator: 1u32.into(),
                commission: 2u32.into()
            }
        );
        let mut raw = vec![0xe3, 0xa1, 1];
        raw.extend([0u8; 32]);
        raw.push(0x80);
        assert_eq!(rewards(&raw).unwrap().delegator, BigUint::from(1u8) << 256);
        for bad in ["c2820001", "c2c080", "c2010200", "c481010180", "f8020102"] {
            assert!(rewards(&bytes(bad)).is_err(), "{bad}");
        }
        assert!(info(&bytes("c2c080")).is_err());
    }
    #[test]
    fn reward_count_delegation_cursor_and_scalar_wire_types() {
        assert_eq!(
            reward_node(&bytes("c68084ffffffff")).unwrap().count,
            u32::MAX
        );
        assert!(reward_node(&bytes("c780850100000000")).is_err());
        assert_eq!(
            delegation(&bytes("ca8088ffffffffffffffff"))
                .unwrap()
                .last_updated,
            u64::MAX
        );
        assert!(delegation(&bytes("cb8089010000000000000000")).is_err());
        assert_eq!(
            scalar(ScalarField::EligibleVotes, &[]).unwrap(),
            Scalar::EligibleVotes(0)
        );
        assert_eq!(
            scalar(ScalarField::EligibleVotes, &[255; 8]).unwrap(),
            Scalar::EligibleVotes(u64::MAX)
        );
        assert!(scalar(ScalarField::EligibleVotes, &[1; 9]).is_err());
        assert_eq!(
            scalar(ScalarField::Yield, &[0x80]).unwrap(),
            Scalar::Yield(0)
        );
        assert_eq!(
            scalar(ScalarField::Yield, &bytes("88ffffffffffffffff")).unwrap(),
            Scalar::Yield(u64::MAX)
        );
        for bad in [
            vec![],
            vec![0],
            bytes("8101"),
            bytes("89010000000000000000"),
            vec![0xc0],
        ] {
            assert!(scalar(ScalarField::Yield, &bad).is_err());
        }
        for field in [
            ScalarField::TotalStake,
            ScalarField::MintedTokens,
            ScalarField::TotalSupply,
        ] {
            assert!(scalar(field, &[0]).is_err());
            assert!(scalar(field, &[1; 33]).is_ok());
            assert!(scalar(field, &[]).is_ok());
        }
        for field in [0, 3, 9, 255] {
            assert!(ScalarField::try_from(field).is_err());
        }
    }
    #[test]
    fn hard_cap_precedes_all_decoding_and_exact_fixed_widths() {
        let oversized = vec![0xff; MAX_BASE_CODEC_ROW_BYTES + 1];
        let failures = [
            validator(&oversized).unwrap_err(),
            info(&oversized).unwrap_err(),
            rewards(&oversized).unwrap_err(),
            reward_node(&oversized).unwrap_err(),
            delegation(&oversized).unwrap_err(),
            scalar(ScalarField::Yield, &oversized).unwrap_err(),
            address(&oversized).unwrap_err(),
            vrf(&oversized).unwrap_err(),
            index(&oversized).unwrap_err(),
        ];
        for error in failures {
            assert_eq!(error.to_string(), "base codec row byte cap");
        }
        assert!(scalar(ScalarField::TotalSupply, &[1; MAX_BASE_CODEC_ROW_BYTES]).is_ok());
        assert_eq!(address(&[1; 20]).unwrap(), [1; 20]);
        assert_eq!(vrf(&[2; 32]).unwrap(), [2; 32]);
        assert_eq!(index(&[0x78, 0x56, 0x34, 0x12]).unwrap(), 0x12345678);
        assert_eq!(index(&[0; 4]).unwrap(), 0);
        for width in [0, 19, 21] {
            assert!(address(&vec![0; width]).is_err());
        }
        for width in [0, 31, 33] {
            assert!(vrf(&vec![0; width]).is_err());
        }
        for width in [0, 3, 5] {
            assert!(index(&vec![0; width]).is_err());
        }
    }
    #[test]
    fn each_rlp_family_rejects_trailing_nonminimal_and_wrong_shapes() {
        for bad in ["c2808000", "c2c080", "f8028080", "c3b80180", "c2820001"] {
            let raw = bytes(bad);
            assert!(info(&raw).is_err(), "info {bad}");
            assert!(rewards(&raw).is_err(), "rewards {bad}");
            assert!(reward_node(&raw).is_err(), "node {bad}");
            assert!(delegation(&raw).is_err(), "delegation {bad}");
        }
        assert!(validator(&bytes("c9c40102030483010000")).is_err());
        assert!(validator(&bytes("c4c0020304")).is_err());
        assert!(validator(&bytes("cd01020389010000000000000000")).is_err());
        assert_eq!(
            reward_node(&bytes("c28080")).unwrap(),
            RewardNode {
                reward_per_stake: BigUint::default(),
                count: 0
            }
        );
        assert_eq!(
            delegation(&bytes("c20102")).unwrap(),
            Delegation {
                stake: 1u32.into(),
                last_updated: 2
            }
        );
        let mut raw = vec![0xe3, 0xa1, 1];
        raw.extend([0u8; 32]);
        raw.push(0x80);
        assert_eq!(
            reward_node(&raw).unwrap().reward_per_stake,
            BigUint::from(1u8) << 256
        );
        assert_eq!(delegation(&raw).unwrap().stake, BigUint::from(1u8) << 256);
        let mut raw = vec![0xe5, 0xa1, 1];
        raw.extend([0u8; 32]);
        raw.extend([0x80; 3]);
        assert_eq!(validator(&raw).unwrap().stake, BigUint::from(1u8) << 256);
    }
    #[test]
    fn literal_layout_preimages_and_distinct_hash_levels() {
        let validator = [0x11; 20];
        let delegator = [0x22; 20];
        let cases = [
            (
                validator_key(ValidatorField::Owner, validator),
                bytes("00031111111111111111111111111111111111111111"),
            ),
            (
                reward_node_key(validator, 0),
                bytes("011111111111111111111111111111111111111111"),
            ),
            (
                reward_node_key(validator, 256),
                bytes("0111111111111111111111111111111111111111110100"),
            ),
            (
                delegation_item_key(delegator, 0x12345678),
                bytes("020122222222222222222222222222222222222222220278563412"),
            ),
            (
                delegation_reverse_key(delegator, validator),
                bytes(
                    "02012222222222222222222222222222222222222222021111111111111111111111111111111111111111",
                ),
            ),
            (
                delegation_record_key(validator, delegator),
                bytes(
                    "020011111111111111111111111111111111111111112222222222222222222222222222222222222222",
                ),
            ),
            (scalar_key(ScalarField::Yield), vec![8]),
        ];
        for (key, preimage) in cases {
            assert_eq!(key.0, keccak256(&preimage));
            assert_eq!(trie_path(key), keccak256(&keccak256(&preimage)));
            assert_ne!(key.0, trie_path(key));
        }
        assert_eq!(
            hex::encode(validator_seed_item_key(128).0),
            "634cff04eb2d3d263206e30d949eef029317db184545f623c4ae5465078992f2"
        );
        // Independent OpenSSL KECCAK-256 vector for preimage 08, then its hash.
        assert_eq!(
            hex::encode(scalar_key(ScalarField::Yield).0),
            "d33e25809fcaa2b6900567812852539da8559dc8b76a7ce3fc5ddd77e8d19a69"
        );
        assert_eq!(
            hex::encode(trie_path(scalar_key(ScalarField::Yield))),
            "a93e930762556e73ce5ea92c5387f25fddc1c0f45d5bbe24b40c7501fe1a1516"
        );
        assert_ne!(
            delegation_record_key(validator, delegator),
            delegation_record_key(delegator, validator)
        );
        assert_eq!(
            validator_key(ValidatorField::Owner, validator),
            validator_seed_owner_key(validator)
        );
    }
}
