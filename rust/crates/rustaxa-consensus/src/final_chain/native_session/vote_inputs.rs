//! Stateless diagnostic decoding of authenticated historical vote inputs.
//!
//! Callers must reconcile raw rows with successful path proofs before passing
//! optional bytes here. `None` means proven logical nonmembership, never a
//! failed physical read. This facade reuses native validator decoding and the
//! existing stake-to-vote kernel without constructing or publishing a snapshot.
use super::*;
use rlp::RlpStream;

/// Candidate policy selected at the consensus request period. Eligibility and
/// jail rules are evaluated separately at the delayed storage period supplied
/// to the voter decoder. Producer configuration authority is caller-owned.
#[derive(Clone, Copy, Debug)]
pub struct HistoricalVotePolicy {
    pub threshold: DposTokenAmount,
    pub step: DposTokenAmount,
    pub maximum_stake: DposTokenAmount,
    pub magnolia_period: u64,
    pub cacti_period: u64,
}

impl HistoricalVotePolicy {
    /// Rejects zero step/maximum and inconsistent threshold bounds before any
    /// stake calculation; this diagnostic does not relax native max-stake rules.
    pub fn validate(self) -> Result<(), anyhow::Error> {
        if self.step.is_zero()
            || self.maximum_stake.is_zero()
            || self.threshold.as_u256() > self.maximum_stake.as_u256()
        {
            anyhow::bail!("invalid historical vote policy threshold/step/maximum");
        }
        Ok(())
    }
}

/// Independently decoded voter facts. Absence yields zero stake/count; missing
/// or zero VRF keys yield `None`. No vote weight or retained expected output is
/// accepted as an input. Exact stake bytes are canonical big-endian U256.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalVoterInputs {
    pub stake_be: [u8; 32],
    pub eligible_vote_count: u64,
    pub vrf_key: Option<[u8; 32]>,
    pub jail_until: Option<u64>,
    pub jailed_at_effective_period: bool,
}

/// Decodes the authenticated global total only when the authenticated jailed
/// list is empty. A nonempty list needs a separately bounded semantic contract:
/// Go subtracts all stored entries while Rust's full-snapshot helper filters
/// active jails. Total is compact big-endian uint64, not RLP; absent is zero.
/// Malformed widths/canonical encodings fail and a zero total is rejected.
pub fn historical_total_votes(
    total: Option<&[u8]>,
    jailed_list: Option<&[u8]>,
) -> Result<u64, anyhow::Error> {
    if let Some(bytes) = jailed_list {
        let list = exact_rlp(bytes, "historical jailed list")?;
        if !list.is_list() || list.item_count()? != 0 || bytes != [0xc0] {
            anyhow::bail!(
                "nonempty or malformed jailed list: bounded total reconstruction pending"
            );
        }
    }
    let bytes = total.unwrap_or_default();
    if bytes.len() > 8 || bytes.first() == Some(&0) {
        anyhow::bail!("historical total votes is not compact big-endian uint64");
    }
    let value = bytes
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
    if value == 0 {
        anyhow::bail!("historical total votes is zero");
    }
    Ok(value)
}

/// Decodes proof-backed validator/VRF/jail rows at the delayed effective period.
/// Validator codecs and stake voting reuse existing native owners. VRF must be
/// exactly32 bytes when present; all-zero denotes no key. Jail is canonical RLP
/// uint64 and inclusive at the effective period. Magnolia activates jailing and
/// Cacti suppresses jailed voter counts. Decode/overflow/policy errors fail.
pub fn historical_voter_inputs(
    policy: HistoricalVotePolicy,
    effective_period: u64,
    validator: Option<&[u8]>,
    vrf: Option<&[u8]>,
    jail: Option<&[u8]>,
) -> Result<HistoricalVoterInputs, anyhow::Error> {
    policy.validate()?;
    let stake = match validator {
        Some(bytes) => {
            semantic_port::decode_validator_facts(
                bytes,
                effective_period >= policy.magnolia_period,
            )?
            .0
        }
        None => U256::zero(),
    };
    let stake_be = stake.to_big_endian();
    let mut eligible_vote_count = dpos_vote_count(
        &stake_be,
        policy.threshold,
        policy.step,
        policy.maximum_stake,
    )?;
    let vrf_key = match vrf {
        Some(bytes) => {
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("historical VRF key must be exactly32 bytes"))?;
            (key != [0; 32]).then_some(key)
        }
        None => None,
    };
    let jail_until = jail
        .map(|bytes| -> Result<u64, anyhow::Error> {
            let rlp = exact_rlp(bytes, "historical jail block")?;
            let block: u64 = rlp.as_val()?;
            let mut encoded = RlpStream::new();
            encoded.append(&block);
            if encoded.out().as_ref() != bytes {
                anyhow::bail!("historical jail block is noncanonical");
            }
            Ok(block)
        })
        .transpose()?;
    let jailed_at_effective_period = effective_period >= policy.magnolia_period
        && jail_until.is_some_and(|until| until >= effective_period);
    if effective_period >= policy.cacti_period && jailed_at_effective_period {
        eligible_vote_count = 0;
    }
    Ok(HistoricalVoterInputs {
        stake_be,
        eligible_vote_count,
        vrf_key,
        jail_until,
        jailed_at_effective_period,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;
    fn policy() -> HistoricalVotePolicy {
        HistoricalVotePolicy {
            threshold: DposTokenAmount::try_from_be_slice(&[10]).unwrap(),
            step: DposTokenAmount::try_from_be_slice(&[2]).unwrap(),
            maximum_stake: DposTokenAmount::try_from_be_slice(&[100]).unwrap(),
            magnolia_period: 5,
            cacti_period: 9,
        }
    }
    fn validator(stake: u64, extended: bool) -> Vec<u8> {
        let mut legacy = RlpStream::new_list(4);
        legacy
            .append(&stake)
            .append(&0_u16)
            .append(&0_u64)
            .append(&0_u64);
        if !extended {
            return legacy.out().to_vec();
        }
        let mut nested = RlpStream::new_list(2);
        nested.append_raw(&legacy.out(), 1).append(&0_u16);
        nested.out().to_vec()
    }
    #[test]
    fn historical_facade_reuses_vote_kernel_and_delayed_jail_boundary() {
        assert_eq!(historical_total_votes(Some(&[1, 0]), None).unwrap(), 256);
        assert_eq!(
            historical_total_votes(Some(&[1]), Some(&[0xc0])).unwrap(),
            1
        );
        for bytes in [&[0xc1, 0x80][..], &[0x80], &[0xc0, 0]] {
            assert!(historical_total_votes(Some(&[1]), Some(bytes)).is_err());
        }
        for total in [&[][..], &[0], &[0, 1], &[1; 9]] {
            assert!(historical_total_votes(Some(total), None).is_err());
        }
        let jail = rlp::encode(&10_u64);
        let active = historical_voter_inputs(
            policy(),
            10,
            Some(&validator(20, true)),
            Some(&[1; 32]),
            Some(&jail),
        )
        .unwrap();
        assert_eq!(active.eligible_vote_count, 0);
        assert!(active.jailed_at_effective_period);
        assert_eq!(
            historical_voter_inputs(policy(), 11, Some(&validator(20, false)), None, Some(&jail))
                .unwrap()
                .eligible_vote_count,
            10
        );
        assert_eq!(
            historical_voter_inputs(policy(), 6, Some(&validator(20, true)), None, None)
                .unwrap()
                .eligible_vote_count,
            10
        );
        assert_eq!(
            historical_voter_inputs(policy(), 8, Some(&validator(20, true)), None, Some(&jail))
                .unwrap()
                .eligible_vote_count,
            10
        );
        assert_eq!(
            historical_voter_inputs(policy(), 10, None, None, None)
                .unwrap()
                .eligible_vote_count,
            0
        );
    }
    #[test]
    fn historical_facade_rejects_bad_rows_and_policy() {
        for row in [validator(101, true), vec![0xc0], vec![0x80]] {
            assert!(historical_voter_inputs(policy(), 10, Some(&row), None, None).is_err());
        }
        assert!(historical_voter_inputs(policy(), 10, None, Some(&[1; 31]), None).is_err());
        assert!(historical_voter_inputs(policy(), 10, None, None, Some(&[0x81, 0x00])).is_err());
        assert!(
            historical_voter_inputs(
                HistoricalVotePolicy {
                    step: DposTokenAmount::zero(),
                    ..policy()
                },
                10,
                None,
                None,
                None
            )
            .is_err()
        );
        assert_eq!(
            historical_voter_inputs(policy(), 10, None, Some(&[0; 32]), None)
                .unwrap()
                .vrf_key,
            None
        );
    }
}
