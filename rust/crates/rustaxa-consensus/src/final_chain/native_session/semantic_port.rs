//! Task-oriented semantic state for the DPoS `setCommission` kernel.
//!
//! The materialized adapter preserves the existing complete [`DposSnapshot`]
//! behavior. The checkpoint adapter operates only on the exact validator and
//! owner rows supplied by an identity-pinned concrete reader with its current
//! execution journal layered above it. Neither adapter grants publication,
//! complete-snapshot, or absence authority.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::final_chain) struct CommissionMetadata {
    owner: [u8; 20],
    commission: u16,
    last_change: u64,
}

/// Minimal mutable state required by the existing `setCommission` rules.
///
/// Implementations are bound to one validator. Missing semantic metadata is a
/// normal non-validator result. Corrupt or unavailable physical dependencies
/// must fail before the kernel is called and must never become that result.
pub(in crate::final_chain) trait CommissionStatePort {
    /// Returns the bound validator address used by the emitted event.
    fn validator(&self) -> [u8; 20];

    /// Returns the current owner and commission fields, when the validator has
    /// semantic metadata.
    fn metadata(&self) -> Option<CommissionMetadata>;

    /// Checks the existing Rust invariant that semantic metadata does not
    /// outlive the canonical validator record.
    fn validate_validator_record(&self) -> Result<(), anyhow::Error>;

    /// Replaces only commission and last-change fields after all rules pass.
    fn set_commission(&mut self, commission: u16, last_change: u64);
}

/// Complete-snapshot adapter retained by normal FinalChain execution.
pub(in crate::final_chain) struct SnapshotCommissionPort<'a> {
    snapshot: &'a mut DposSnapshot,
    validator: [u8; 20],
}

impl<'a> SnapshotCommissionPort<'a> {
    /// Binds the complete materialized snapshot to one validator mutation.
    pub(in crate::final_chain) fn new(snapshot: &'a mut DposSnapshot, validator: [u8; 20]) -> Self {
        Self {
            snapshot,
            validator,
        }
    }
}

impl CommissionStatePort for SnapshotCommissionPort<'_> {
    fn validator(&self) -> [u8; 20] {
        self.validator
    }

    fn metadata(&self) -> Option<CommissionMetadata> {
        self.snapshot
            .validator_metadata
            .get(&self.validator)
            .map(|metadata| CommissionMetadata {
                owner: metadata.owner,
                commission: metadata.commission,
                last_change: metadata.last_commission_change,
            })
    }

    fn validate_validator_record(&self) -> Result<(), anyhow::Error> {
        if !self.snapshot.total_stakes.contains_key(&self.validator)
            && FinalChain::dpos_validator_owned_rows_exist(self.snapshot, self.validator)
        {
            anyhow::bail!(
                "DPoS validator snapshot inconsistency: orphan rows found without stake row"
            );
        }
        Ok(())
    }

    fn set_commission(&mut self, commission: u16, last_change: u64) {
        let metadata = self
            .snapshot
            .validator_metadata
            .get_mut(&self.validator)
            .expect("metadata was read before mutation");
        metadata.commission = commission;
        metadata.last_commission_change = last_change;
    }
}

/// Exact validator-row adapter for an authenticated checkpoint plus journal.
///
/// Construction validates required `Present` rows and decodes the complete
/// validator value without narrowing its stake. The caller remains responsible
/// for obtaining both observations through [`FinalChainNativeStateRead`] at one
/// exact identity with every earlier execution effect overlaid. This type
/// cannot read a database, publish state, or represent other DPoS rows.
pub(super) struct CheckpointJournalCommissionPort {
    validator: [u8; 20],
    owner: [u8; 20],
    facts: ValidatorFacts,
    previous: CommissionMetadata,
    extended_validator_active: bool,
    replacement: Option<Vec<u8>>,
}

impl CheckpointJournalCommissionPort {
    /// Decodes the two exact rows needed by `setCommission`.
    ///
    /// Absent or tombstoned dependencies are integrity failures. Infrastructure
    /// and historical-coverage failures occur while obtaining these reads and
    /// remain typed [`FinalChainNativeStateReadError`] values at that boundary.
    fn from_reads(
        validator: [u8; 20],
        validator_read: &ConcreteRead<Vec<u8>>,
        owner_read: &ConcreteRead<Vec<u8>>,
        extended_validator_active: bool,
    ) -> std::result::Result<Self, FinalChainNativeSessionError> {
        let ConcreteRead::Present(bytes) = validator_read else {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator raw row is not present".to_owned(),
            ));
        };
        let facts = decode_validator_facts(bytes, extended_validator_active)?;
        let ConcreteRead::Present(owner) = owner_read else {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission owner raw row is not present".to_owned(),
            ));
        };
        let owner: [u8; 20] = owner.as_slice().try_into().map_err(|_| {
            FinalChainNativeSessionError::RawIntegrity(
                "setCommission owner raw row has the wrong length".to_owned(),
            )
        })?;
        Ok(Self {
            validator,
            owner,
            facts,
            previous: CommissionMetadata {
                owner,
                commission: facts.1,
                last_change: facts.2,
            },
            extended_validator_active,
            replacement: None,
        })
    }

    /// Revalidates one prepared checkpoint against current journal-overlay
    /// reads and the complete session snapshot, then opens the kernel adapter.
    ///
    /// Both current reads must byte-match the prepared observations. The raw
    /// validator's complete fields and owner must also match the authoritative
    /// session snapshot. State transport failures remain at the caller's typed
    /// read boundary; this function distinguishes physical integrity failures.
    pub(super) fn from_authenticated_reads(
        snapshot: &DposSnapshot,
        validator: [u8; 20],
        prepared_validator: &ConcreteRead<Vec<u8>>,
        prepared_owner: &ConcreteRead<Vec<u8>>,
        current_validator: &ConcreteRead<Vec<u8>>,
        current_owner: &ConcreteRead<Vec<u8>>,
        extended_validator_active: bool,
    ) -> std::result::Result<Self, FinalChainNativeSessionError> {
        if current_validator != prepared_validator || current_owner != prepared_owner {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission raw observations changed after preparation".to_owned(),
            ));
        }
        let ConcreteRead::Present(validator_bytes) = current_validator else {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator raw row is not present".to_owned(),
            ));
        };
        let expected_facts = snapshot_validator_facts(snapshot, validator)?;
        if extended_validator_active
            && expected_facts.4 != 0
            && super::exact_rlp(validator_bytes, "setCommission validator row")?
                .item_count()
                .map_err(|error| super::raw_integrity("validator item count", error))?
                == 4
        {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator row has the wrong extended shape".to_owned(),
            ));
        }
        if decode_validator_facts(validator_bytes, extended_validator_active)? != expected_facts {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator raw/domain facts disagree".to_owned(),
            ));
        }
        let expected_owner = snapshot
            .validator_metadata
            .get(&validator)
            .ok_or_else(|| {
                FinalChainNativeSessionError::RawIntegrity(
                    "setCommission validator metadata is absent".to_owned(),
                )
            })?
            .owner;
        let ConcreteRead::Present(owner_bytes) = current_owner else {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission owner raw row is not present".to_owned(),
            ));
        };
        if owner_bytes.as_slice() != expected_owner {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission owner raw/domain facts disagree".to_owned(),
            ));
        }
        Self::from_reads(
            validator,
            current_validator,
            current_owner,
            extended_validator_active,
        )
    }

    /// Takes the exact raw replacement and prior identity produced by success.
    ///
    /// `None` means the kernel did not authorize a mutation. The returned
    /// value grants authority only to update the already authenticated complete
    /// session snapshot; it cannot publish either storage representation.
    pub(super) fn take_authenticated_update(&mut self) -> Option<AuthenticatedCommissionUpdate> {
        self.replacement
            .take()
            .map(|replacement| AuthenticatedCommissionUpdate {
                validator: self.validator,
                previous: self.previous,
                commission: self.facts.1,
                last_change: self.facts.2,
                replacement,
            })
    }

    fn encode_facts(&self) -> Vec<u8> {
        let (stake, commission, last_change, reward_head, undelegations_count) = self.facts;
        let mut legacy = rlp::RlpStream::new_list(4);
        legacy
            .append(&stake)
            .append(&commission)
            .append(&last_change)
            .append(&reward_head);
        let legacy = legacy.out().to_vec();
        if !self.extended_validator_active {
            return legacy;
        }
        let mut extended = rlp::RlpStream::new_list(2);
        extended.append_raw(&legacy, 1).append(&undelegations_count);
        extended.out().to_vec()
    }
}

/// Successful checkpoint-kernel output bound to its prior semantic identity.
///
/// Fields remain private so callers cannot manufacture authority to update the
/// complete session snapshot. This value authorizes no database publication.
pub(super) struct AuthenticatedCommissionUpdate {
    validator: [u8; 20],
    previous: CommissionMetadata,
    commission: u16,
    last_change: u64,
    replacement: Vec<u8>,
}

/// Applies one authenticated checkpoint result to the complete session state.
///
/// The prior owner, commission and last-change values must still match. The
/// canonical validator-row invariant is rechecked before mutation. On success,
/// only the two commission fields change and the exact raw replacement is
/// returned for the journal; this seam cannot publish the snapshot or raw row.
pub(super) fn apply_authenticated_checkpoint_to_snapshot(
    snapshot: &mut DposSnapshot,
    update: AuthenticatedCommissionUpdate,
) -> Result<Vec<u8>, anyhow::Error> {
    let mut port = SnapshotCommissionPort::new(snapshot, update.validator);
    anyhow::ensure!(
        port.metadata() == Some(update.previous),
        "setCommission checkpoint no longer matches complete snapshot"
    );
    port.validate_validator_record()?;
    port.set_commission(update.commission, update.last_change);
    Ok(update.replacement)
}

impl CommissionStatePort for CheckpointJournalCommissionPort {
    fn validator(&self) -> [u8; 20] {
        self.validator
    }

    fn metadata(&self) -> Option<CommissionMetadata> {
        Some(CommissionMetadata {
            owner: self.owner,
            commission: self.facts.1,
            last_change: self.facts.2,
        })
    }

    fn validate_validator_record(&self) -> Result<(), anyhow::Error> {
        // Required construction from a Present validator row already proves
        // this operation's canonical record. It grants no authority over any
        // other row or over physical absence.
        Ok(())
    }

    fn set_commission(&mut self, commission: u16, last_change: u64) {
        self.facts.1 = commission;
        self.facts.2 = last_change;
        self.replacement = Some(self.encode_facts());
    }
}

/// Applies the existing `setCommission` business rules through one task port.
///
/// Contract failures do not mutate the port. A future last-change value remains
/// an explicit Rust snapshot invariant failure, matching the pre-existing Rust
/// kernel even though malformed historical state was not qualified against the
/// legacy unsigned-subtraction behavior.
pub(in crate::final_chain) fn apply_set_commission<P: CommissionStatePort>(
    port: &mut P,
    owner: [u8; 20],
    commission: u16,
    block_number: FinalChainBlockNumber,
    commission_change_frequency: u32,
    commission_change_delta: u16,
) -> Result<DposApplyOutcome, anyhow::Error> {
    let Some(metadata) = port.metadata() else {
        return Ok(DposApplyOutcome::mutation_contract_failure(
            DposContractError::WrongOwnerAcc,
        ));
    };
    if metadata.owner != owner {
        return Ok(DposApplyOutcome::mutation_contract_failure(
            DposContractError::WrongOwnerAcc,
        ));
    }
    if commission > DPOS_MAX_COMMISSION {
        return Ok(DposApplyOutcome::mutation_contract_failure(
            DposContractError::CommissionOverflow,
        ));
    }
    if metadata.last_change > block_number.as_u64() {
        anyhow::bail!(
            "DPoS validator snapshot inconsistency: last commission change is in the future"
        );
    }
    port.validate_validator_record()?;
    if commission_change_frequency != 0
        && block_number.as_u64() - metadata.last_change < u64::from(commission_change_frequency)
    {
        return Ok(DposApplyOutcome::mutation_contract_failure(
            DposContractError::ForbiddenCommissionChange,
        ));
    }
    if commission_change_delta != 0
        && commission.abs_diff(metadata.commission) > commission_change_delta
    {
        return Ok(DposApplyOutcome::mutation_contract_failure(
            DposContractError::ForbiddenCommissionChange,
        ));
    }
    port.set_commission(commission, block_number.as_u64());
    Ok(DposApplyOutcome::success(vec![dpos_commission_set_log(
        port.validator(),
        commission,
    )?]))
}

fn decode_validator_facts(
    bytes: &[u8],
    extended_validator_active: bool,
) -> std::result::Result<ValidatorFacts, FinalChainNativeSessionError> {
    let rlp = super::exact_rlp(bytes, "setCommission validator row")?;
    if extended_validator_active {
        return match rlp
            .item_count()
            .map_err(|error| super::raw_integrity("validator item count", error))?
        {
            4 => super::decode_legacy_validator(&rlp, 0),
            2 => {
                let legacy = rlp
                    .at(0)
                    .map_err(|error| super::raw_integrity("extended validator body", error))?;
                let count = rlp
                    .val_at(1)
                    .map_err(|error| super::raw_integrity("extended undelegation count", error))?;
                super::decode_legacy_validator(&legacy, count)
            }
            _ => Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator row has the wrong extended shape".to_owned(),
            )),
        };
    }
    if rlp
        .item_count()
        .map_err(|error| super::raw_integrity("validator item count", error))?
        != 4
    {
        return Err(FinalChainNativeSessionError::RawIntegrity(
            "setCommission validator row has the wrong legacy shape".to_owned(),
        ));
    }
    super::decode_legacy_validator(&rlp, 0)
}

/// Reads the complete validator tuple used to authenticate raw checkpoints.
///
/// Missing stake or metadata and unavailable reward-graph facts remain typed
/// session errors. No field is narrowed, mutated, serialized or published.
pub(super) fn snapshot_validator_facts(
    snapshot: &DposSnapshot,
    validator: [u8; 20],
) -> std::result::Result<ValidatorFacts, FinalChainNativeSessionError> {
    let stake = snapshot
        .total_stakes
        .get(&validator)
        .ok_or_else(|| {
            FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator stake is absent".to_owned(),
            )
        })?
        .as_u256();
    let metadata = snapshot.validator_metadata.get(&validator).ok_or_else(|| {
        FinalChainNativeSessionError::RawIntegrity(
            "setCommission validator metadata is absent".to_owned(),
        )
    })?;
    let reward_head = snapshot
        .reward_reference_graph
        .read_validator_head(&validator)
        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
    Ok((
        stake,
        metadata.commission,
        metadata.last_commission_change,
        reward_head,
        checked_undelegations_count(snapshot, validator)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALIDATOR: [u8; 20] = [0x11; 20];
    const OWNER: [u8; 20] = [0x22; 20];

    fn legacy_row(facts: ValidatorFacts) -> Vec<u8> {
        let mut stream = rlp::RlpStream::new_list(4);
        stream
            .append(&facts.0)
            .append(&facts.1)
            .append(&facts.2)
            .append(&facts.3);
        stream.out().to_vec()
    }

    fn extended_row(facts: ValidatorFacts) -> Vec<u8> {
        let legacy = legacy_row(facts);
        let mut stream = rlp::RlpStream::new_list(2);
        stream.append_raw(&legacy, 1).append(&facts.4);
        stream.out().to_vec()
    }

    #[test]
    fn checkpoint_commission_preserves_full_width_and_untouched_fields() {
        let stake = U256::from_big_endian(&[0xff; 32]);
        let facts = (stake, 100, 7, u64::MAX - 1, 19);
        let mut port = CheckpointJournalCommissionPort::from_reads(
            VALIDATOR,
            &ConcreteRead::Present(extended_row(facts)),
            &ConcreteRead::Present(OWNER.to_vec()),
            true,
        )
        .unwrap();

        let outcome =
            apply_set_commission(&mut port, OWNER, 125, FinalChainBlockNumber::new(10), 0, 0)
                .unwrap();

        assert_eq!(outcome.status_code, 1);
        let replacement = port.take_authenticated_update().unwrap().replacement;
        assert_eq!(
            replacement,
            extended_row((stake, 125, 10, u64::MAX - 1, 19))
        );
        let decoded = decode_validator_facts(&replacement, true).unwrap();
        assert_eq!(decoded, (stake, 125, 10, u64::MAX - 1, 19));
    }

    #[test]
    fn active_extended_serializer_upgrades_legacy_fallback() {
        let facts = (U256::from(99), 100, 7, 8, 0);
        let mut port = CheckpointJournalCommissionPort::from_reads(
            VALIDATOR,
            &ConcreteRead::Present(legacy_row(facts)),
            &ConcreteRead::Present(OWNER.to_vec()),
            true,
        )
        .unwrap();

        apply_set_commission(&mut port, OWNER, 101, FinalChainBlockNumber::new(8), 0, 0).unwrap();

        let replacement = port.take_authenticated_update().unwrap().replacement;
        assert_eq!(replacement, extended_row((U256::from(99), 101, 8, 8, 0)));
        assert_eq!(rlp::Rlp::new(&replacement).item_count().unwrap(), 2);
        assert_eq!(
            decode_validator_facts(&replacement, true).unwrap(),
            (U256::from(99), 101, 8, 8, 0)
        );
    }

    #[test]
    fn checkpoint_dependencies_fail_closed_and_failure_does_not_replace() {
        let facts = (U256::from(99), 100, 7, 8, 0);
        assert!(matches!(
            CheckpointJournalCommissionPort::from_reads(
                VALIDATOR,
                &ConcreteRead::Tombstone,
                &ConcreteRead::Present(OWNER.to_vec()),
                true,
            ),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));
        assert!(matches!(
            CheckpointJournalCommissionPort::from_reads(
                VALIDATOR,
                &ConcreteRead::Present(extended_row(facts)),
                &ConcreteRead::Absent,
                true,
            ),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));

        let mut port = CheckpointJournalCommissionPort::from_reads(
            VALIDATOR,
            &ConcreteRead::Present(extended_row(facts)),
            &ConcreteRead::Present(OWNER.to_vec()),
            true,
        )
        .unwrap();
        let outcome = apply_set_commission(
            &mut port,
            [0x33; 20],
            101,
            FinalChainBlockNumber::new(8),
            0,
            0,
        )
        .unwrap();
        assert_eq!(outcome.status_code, 0);
        assert!(port.take_authenticated_update().is_none());
    }
}
