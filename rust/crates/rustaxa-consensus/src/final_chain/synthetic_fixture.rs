//! Explicit fixture-only nonzero native/header initialization.
//!
//! Concrete roots must already have been computed from the complete declared
//! logical inventory by the incremental writer. No production execution marker
//! or imported-state adoption capability is created here. Subsequent execution
//! uses the existing application owner and ordinary publication/recovery paths.
use super::*;
use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStateRead, ConcreteStorageKey};

#[derive(Clone, Debug, Eq, PartialEq)]
struct DonorPolicy {
    genesis_root: H256,
    rewards: FinalChainRewardsConfig,
    amounts: [DposTokenAmount; 4],
    distances: [u64; 7],
    periods: [u64; 5],
    batched: bool,
}

/// Authenticated native state produced by an actual task-owned fixture owner.
/// Fields are private: callers cannot import a Go snapshot or assert flags.
pub struct FinalChainSyntheticDonor {
    source: rustaxa_types::concrete_state::ConcreteStateIdentity,
    snapshot: DposSnapshot,
    snapshot_hash: [u8; 32],
    policy: DonorPolicy,
    raw: BTreeMap<ConcreteStorageSlot, Option<Vec<u8>>>,
    present_accounts: BTreeSet<[u8; 20]>,
}

impl FinalChainSyntheticDonor {
    pub fn source_identity(&self) -> rustaxa_types::concrete_state::ConcreteStateIdentity {
        self.source
    }
    pub fn native_snapshot_hash(&self) -> [u8; 32] {
        self.snapshot_hash
    }
}

impl FinalChain {
    /// Plans bound rewards from explicitly declared synthetic component facts.
    ///
    /// This test-only seam preserves the private planner's head, generation and
    /// request checks and returns its opaque plan unchanged. It does not write
    /// storage, publish a period or authenticate a real-network input bundle.
    #[cfg(feature = "synthetic-fixtures")]
    pub fn plan_synthetic_external_evm_rewards_stats(
        &self,
        request_id: [u8; 32],
        fact: FinalizedRewardsPeriodFact,
    ) -> Result<FinalChainPreparedExternalEvmRewardsStatsPlan> {
        self.plan_external_evm_rewards_stats(request_id, fact)
    }

    fn synthetic_donor_policy(&self) -> DonorPolicy {
        DonorPolicy {
            genesis_root: self.genesis_state_root,
            rewards: self.rewards_config.clone(),
            amounts: [
                self.dpos_eligibility_balance_threshold,
                self.dpos_vote_eligibility_balance_step,
                self.dpos_validator_maximum_stake,
                self.dpos_minimum_deposit,
            ],
            distances: [
                self.dpos_delegation_delay,
                self.dpos_delegation_locking_period,
                self.dpos_cornus_delegation_locking_period,
                self.dpos_cacti_delegation_locking_period,
                u64::from(self.dpos_commission_change_delta),
                u64::from(self.dpos_commission_change_frequency),
                self.dag_vdf_sortition_max_vote_count,
            ],
            periods: [
                self.dpos_cornus_period.as_u64(),
                self.dpos_phalaenopsis_period.as_u64(),
                self.dpos_cacti_period.as_u64(),
                self.ficus_activation_period.as_u64(),
                self.dag_vdf_sortition_total_vote_count_until_period
                    .as_u64(),
            ],
            batched: self.synthetic_batched_lifecycle,
        }
    }

    fn synthetic_donor_era(&self, period: u64) -> Vec<bool> {
        [
            self.rewards_config.fix_redelegate_block_num,
            self.rewards_config.magnolia_period,
            self.rewards_config.phalaenopsis_period,
            self.rewards_config.fix_claim_all_block_num,
            self.rewards_config.aspen_part_one_period,
            self.rewards_config.aspen_part_two_period,
            self.ficus_activation_period,
            self.dpos_cornus_period,
            self.dpos_cacti_period,
        ]
        .iter()
        .map(|boundary| period >= boundary.as_u64())
        .collect()
    }

    /// Captures only an actually finalized, fully paired donor with execution
    /// provenance. No caller-supplied semantic snapshot enters this token.
    pub fn capture_synthetic_fixture_donor(
        &self,
        owner: &rustaxa_storage::ConcreteStateLifecycle,
        proof_reader: &rustaxa_storage::ConcreteStateReader,
        known_history: &BTreeSet<[u8; 32]>,
    ) -> Result<FinalChainSyntheticDonor, anyhow::Error> {
        let observation = owner.observation()?;
        let provenance =
            rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_state_provenance(
                &observation.provenance_rlp,
            )?;
        let source = observation.committed;
        anyhow::ensure!(
            observation.identity.chain_id == self.concrete_chain_identity()?
                && observation.pending_marker_rlp.is_empty()
                && self.last_block_number_typed()? == source.period
                && provenance.plan_hash != [0; 32]
                && provenance.projection_hash != [0; 32],
            "SYNTHETIC_DONOR_NOT_EXECUTED_AND_PAIRED"
        );
        let header_raw = self
            .storage
            .final_chain()
            .block_header_raw(source.period.as_u64())?
            .ok_or_else(|| anyhow::anyhow!("SYNTHETIC_DONOR_HEADER_MISSING"))?;
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header_raw))?;
        anyhow::ensure!(
            header.state_root.0 == source.state_root,
            "SYNTHETIC_DONOR_HEADER_ROOT"
        );
        let mut snapshot = self.dpos_snapshot_at_finalized_block(source.period)?;
        let encoded = encode_dpos_snapshot_rlp(&snapshot)?;
        anyhow::ensure!(
            self.storage
                .final_chain()
                .dpos_snapshot_raw(source.period.as_u64())?
                .as_deref()
                == Some(encoded.as_slice()),
            "SYNTHETIC_DONOR_NATIVE_SNAPSHOT"
        );
        let reader = owner.prior_reader();
        anyhow::ensure!(
            proof_reader.identity() == source && reader.identity() == source,
            "SYNTHETIC_DONOR_READER_IDENTITY"
        );
        anyhow::ensure!(
            &proof_reader.synthetic_storage_history_prefixes()? == known_history,
            "SYNTHETIC_DONOR_HISTORY_INVENTORY"
        );
        let mut raw = BTreeMap::new();
        for slot in rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog(
            &observation.catalog_rlp,
        )? {
            let value = match reader.storage(slot.address, ConcreteStorageKey(slot.key)) {
                Ok(ConcreteRead::Present(value)) if !value.is_empty() => Some(value),
                Ok(ConcreteRead::Absent | ConcreteRead::Tombstone | ConcreteRead::Present(_)) => {
                    None
                }
                Err(
                    error @ rustaxa_types::concrete_state::ConcreteReadError::HistoryUnavailable(_),
                ) => {
                    anyhow::ensure!(
                        !known_history.contains(&rustaxa_storage::storage_version_prefix(
                            slot.address,
                            ConcreteStorageKey(slot.key)
                        )) && matches!(
                            proof_reader
                                .verify_storage_path(slot.address, ConcreteStorageKey(slot.key))?,
                            rustaxa_storage::ConcreteStoragePath::NonMember
                        ),
                        "SYNTHETIC_DONOR_HISTORY_UNAVAILABLE:{error:?}"
                    );
                    None
                }
                Err(error) => return Err(error.into()),
            };
            if value.is_none() {
                anyhow::ensure!(
                    matches!(
                        proof_reader
                            .verify_storage_path(slot.address, ConcreteStorageKey(slot.key))?,
                        rustaxa_storage::ConcreteStoragePath::NonMember
                    ),
                    "SYNTHETIC_DONOR_ABSENCE_PROOF"
                );
            }
            anyhow::ensure!(
                raw.insert(slot, value).is_none(),
                "SYNTHETIC_DONOR_DUPLICATE_SLOT"
            );
        }
        // Full catalog values above are authenticated against the one committed
        // source root, including explicit NonMember proofs for retained absences.
        let bound_rows = raw
            .iter()
            .map(|(slot, value)| ((slot.address, slot.key), value.clone()))
            .collect();
        snapshot = stage_validator_rows_from_authenticated_catalog(
            &snapshot,
            &bound_rows,
            source.period >= self.rewards_config.magnolia_period,
        )?;
        let canonical = canonical_concrete_precompile_storage(
            &snapshot,
            source.period >= self.rewards_config.magnolia_period,
        )?;
        for ((address, key), allowed) in canonical {
            let value = raw
                .get(&ConcreteStorageSlot { address, key })
                .cloned()
                .flatten()
                .unwrap_or_default();
            let mut jail_list = [0; 32];
            jail_list[31] = 2;
            let fresh_jail = address == SLASHING_CONTRACT_ADDRESS
                && key == jail_list
                && value.is_empty()
                && snapshot.slashing_jailed_validators.is_empty()
                && allowed.contains(&vec![0xc0]);
            anyhow::ensure!(
                allowed.contains(&value) || fresh_jail,
                "SYNTHETIC_DONOR_NATIVE_RAW_MISMATCH"
            );
        }
        let present_accounts = self
            .current_account_snapshot()?
            .into_keys()
            .collect::<BTreeSet<_>>();
        for address in &present_accounts {
            anyhow::ensure!(
                matches!(reader.account(*address)?, ConcreteRead::Present(_)),
                "SYNTHETIC_DONOR_ACCOUNT_INVENTORY"
            );
        }
        Ok(FinalChainSyntheticDonor {
            source,
            snapshot,
            snapshot_hash: concrete_storage_key(&[&encoded]),
            policy: self.synthetic_donor_policy(),
            raw,
            present_accounts,
        })
    }
    /// Selects the pinned public Go batched transaction lifecycle in a fixture.
    /// Concrete StateAPI execution remains the default. This is a construction
    /// input, retained separately from durable state and the cleanup timer.
    pub fn configure_synthetic_batched_lifecycle(&mut self) {
        self.synthetic_batched_lifecycle = true;
    }
    /// Binds an exact independently captured pre-Aspen C++ scheduling vector to
    /// one fixture distribution. Runtime processing checks complete membership
    /// and preserves every supplied period occurrence without normalization.
    pub fn configure_synthetic_legacy_distribution_order(
        &self,
        period: u64,
        order: Vec<u64>,
    ) -> Result<(), anyhow::Error> {
        let frequency = rewards_distribution_frequency(
            &self.rewards_config.rewards_distribution_frequency,
            period.into(),
        );
        anyhow::ensure!(
            period < self.rewards_config.aspen_part_one_period.as_u64(),
            "SYNTHETIC_LEGACY_ORDER_SCOPE"
        );
        // Frequency one already has the deterministic current-period singleton.
        // Authenticate that input without installing a fixture order override.
        if frequency == 1 {
            anyhow::ensure!(order == [period], "SYNTHETIC_LEGACY_ORDER_MEMBERSHIP");
            return Ok(());
        }
        anyhow::ensure!(
            frequency > 1 && period.is_multiple_of(u64::from(frequency)),
            "SYNTHETIC_LEGACY_ORDER_SCOPE"
        );
        let unique = order.iter().copied().collect::<BTreeSet<_>>();
        anyhow::ensure!(
            !order.is_empty() && unique.len() == order.len() && unique.last() == Some(&period),
            "SYNTHETIC_LEGACY_ORDER_MEMBERSHIP"
        );
        self.synthetic_legacy_distribution_orders
            .lock()
            .map_err(|_| anyhow::anyhow!("synthetic distribution order lock"))?
            .insert(period, order);
        Ok(())
    }

    /// Pairs one synthetic prehistory coordinate with its concrete reader.
    ///
    /// The first coordinate requires an empty application head; further seed
    /// coordinates must be consecutive. All native state is the independently
    /// configured genesis ledger plus explicit supply/yield rows, checked byte
    /// for byte against the concrete reader. Every ordinary account comes from
    /// that same reader. `present_native_slots` and `present_accounts` are the
    /// complete declared creation inventory, authenticated by the fixture owner
    /// through independent Go/root reconstruction. This seam does not infer
    /// absence for arbitrary readers and is unavailable in production builds.
    ///
    /// Pending reward facts must cover the exact active unfinished interval.
    /// They are processed by RewardsStatsRuntime, persisted before LAST_NUMBER,
    /// and reloaded by the normal constructor on concrete close/reopen.
    pub fn initialize_synthetic_fixture_snapshot(
        &self,
        owner: &rustaxa_storage::ConcreteStateLifecycle,
        present_accounts: &[[u8; 20]],
        present_native_slots: &[ConcreteStorageSlot],
        pending_facts: Vec<FinalizedRewardsPeriodFact>,
    ) -> Result<(), anyhow::Error> {
        self.initialize_synthetic_fixture_snapshot_inner(
            owner,
            present_accounts,
            present_native_slots,
            pending_facts,
            None,
        )
    }

    /// Creates a new synthetic retained state from independently executed donor
    /// facts. This does not claim replay of the omitted interval or copy timers.
    pub fn initialize_synthetic_fixture_snapshot_from_donor(
        &self,
        owner: &rustaxa_storage::ConcreteStateLifecycle,
        present_accounts: &[[u8; 20]],
        present_native_slots: &[ConcreteStorageSlot],
        pending_facts: Vec<FinalizedRewardsPeriodFact>,
        donor: &FinalChainSyntheticDonor,
    ) -> Result<(), anyhow::Error> {
        self.initialize_synthetic_fixture_snapshot_inner(
            owner,
            present_accounts,
            present_native_slots,
            pending_facts,
            Some(donor),
        )
    }

    fn initialize_synthetic_fixture_snapshot_inner(
        &self,
        owner: &rustaxa_storage::ConcreteStateLifecycle,
        present_accounts: &[[u8; 20]],
        present_native_slots: &[ConcreteStorageSlot],
        pending_facts: Vec<FinalizedRewardsPeriodFact>,
        donor: Option<&FinalChainSyntheticDonor>,
    ) -> Result<(), anyhow::Error> {
        let observation = owner.observation()?;
        let provenance =
            rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_state_provenance(
                &observation.provenance_rlp,
            )
            .map_err(|error| anyhow::anyhow!(error))?;
        anyhow::ensure!(
            observation.pending_marker_rlp.is_empty()
                && provenance.plan_hash == [0; 32]
                && provenance.projection_hash == [0; 32]
                && provenance.transactions_hash == [0; 32]
                && provenance.rewards_hash == [0; 32],
            "SYNTHETIC_SEED_CONCRETE_ALREADY_EXECUTED"
        );
        let reader = owner.prior_reader();
        let identity = reader.identity();
        let head = self.last_block_number_typed()?;
        anyhow::ensure!(!identity.period.is_genesis(), "SYNTHETIC_SEED_ZERO_PERIOD");
        anyhow::ensure!(
            head.is_genesis() || head.checked_next() == Some(identity.period),
            "SYNTHETIC_SEED_CHRONOLOGY"
        );
        anyhow::ensure!(
            self.storage
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none(),
            "SYNTHETIC_SEED_ALREADY_EXECUTING"
        );
        self.verify_or_initialize_concrete_state_pairing(observation.identity)?;
        let accounts_set = present_accounts.iter().copied().collect::<BTreeSet<_>>();
        let slots_set = present_native_slots
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            accounts_set.len() == present_accounts.len()
                && slots_set.len() == present_native_slots.len(),
            "SYNTHETIC_SEED_DUPLICATE_IDENTITY"
        );
        if let Some(donor) = donor {
            anyhow::ensure!(
                donor.policy == self.synthetic_donor_policy()
                    && donor.source.period < identity.period
                    && donor.source.state_root == identity.state_root
                    && self.synthetic_donor_era(donor.source.period.as_u64())
                        == self.synthetic_donor_era(identity.period.as_u64()),
                "SYNTHETIC_DONOR_POLICY_ERA_OR_ROOT"
            );
            anyhow::ensure!(
                accounts_set == donor.present_accounts,
                "SYNTHETIC_DONOR_TARGET_ACCOUNT_INVENTORY"
            );
            anyhow::ensure!(
                slots_set
                    == donor
                        .raw
                        .iter()
                        .filter_map(|(slot, value)| value.as_ref().map(|_| *slot))
                        .collect::<BTreeSet<_>>(),
                "SYNTHETIC_DONOR_TARGET_SLOT_INVENTORY"
            );
            let target_created =
                rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog(
                    &observation.catalog_rlp,
                )?
                .into_iter()
                .collect::<BTreeSet<_>>();
            for (slot, expected) in &donor.raw {
                let actual = match reader.storage(slot.address, ConcreteStorageKey(slot.key)) {
                    Ok(ConcreteRead::Present(value)) if !value.is_empty() => Some(value),
                    Ok(
                        ConcreteRead::Absent | ConcreteRead::Tombstone | ConcreteRead::Present(_),
                    ) => None,
                    Err(
                        error
                        @ rustaxa_types::concrete_state::ConcreteReadError::HistoryUnavailable(
                            _,
                        ),
                    ) => {
                        // Every token None has a real source NonMember proof.
                        // The independently computed equal root transfers that
                        // logical proof, only to this never-executed initializer
                        // and a key outside its complete creation inventory.
                        anyhow::ensure!(
                            expected.is_none()
                                && !target_created.contains(slot)
                                && !slots_set.contains(slot),
                            "SYNTHETIC_TARGET_HISTORY_UNAVAILABLE:{error:?}"
                        );
                        None
                    }
                    Err(error) => return Err(error.into()),
                };
                anyhow::ensure!(&actual == expected, "SYNTHETIC_DONOR_TARGET_RAW_MISMATCH");
            }
        }
        let mut snapshot = if let Some(donor) = donor {
            let mut snapshot = donor.snapshot.clone();
            let before = canonical_concrete_precompile_storage(
                &snapshot,
                identity.period >= self.rewards_config.magnolia_period,
            )?;
            self.advance_reward_reference_graph_block(&mut snapshot, identity.period)?;
            anyhow::ensure!(
                before
                    == canonical_concrete_precompile_storage(
                        &snapshot,
                        identity.period >= self.rewards_config.magnolia_period
                    )?,
                "SYNTHETIC_DONOR_GRAPH_CHANGED_DURABLE_ROWS"
            );
            snapshot
        } else {
            let mut scaffold =
                self.dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)?;
            // This new imported identity is not the owned block0 creation.
            // Its complete root/catalog, not constructor genesis policy, must
            // establish shapes. The published genesis snapshot is unchanged.
            anyhow::ensure!(
                scaffold.retained_undelegation_counts.is_empty(),
                "SYNTHETIC_SEED_SCAFFOLD_COUNTER_AUTHORITY"
            );
            scaffold.validator_row_authority = ValidatorRowAuthority::CompatibilityOnly;
            scaffold
        };
        let donor_supply = snapshot.aspen_supply_state.clone();
        let present = |key: u8| {
            slots_set.contains(&ConcreteStorageSlot {
                address: DPOS_CONTRACT_ADDRESS,
                key: concrete_storage_key(&[&[key]]),
            })
        };
        let read = |key: u8| -> Result<Vec<u8>, anyhow::Error> {
            match reader.storage(
                DPOS_CONTRACT_ADDRESS,
                ConcreteStorageKey(concrete_storage_key(&[&[key]])),
            )? {
                ConcreteRead::Present(value) => Ok(value),
                _ => anyhow::bail!("SYNTHETIC_SEED_NATIVE_ROW_MISSING:{key}"),
            }
        };
        if present(7) {
            anyhow::ensure!(present(8) && !present(6), "SYNTHETIC_SEED_SUPPLY_SHAPE");
            let raw_supply = read(7)?;
            anyhow::ensure!(
                !raw_supply.is_empty() && raw_supply.len() <= 32 && raw_supply[0] != 0,
                "SYNTHETIC_SEED_SUPPLY_WIDTH"
            );
            let supply = U256::from_big_endian(&raw_supply);
            let current_yield: u64 = Rlp::new(&read(8)?).as_val()?;
            snapshot.aspen_supply_state = AspenSupplyState::Migrated {
                total_supply: StoredDposTokenAmount::canonical_u256_after_mutation(supply),
                current_yield: AspenYield(current_yield),
            };
        } else {
            anyhow::ensure!(!present(8), "SYNTHETIC_SEED_YIELD_WITHOUT_SUPPLY");
            let minted = if present(6) {
                let raw = read(6)?;
                anyhow::ensure!(
                    !raw.is_empty() && raw.len() <= 32 && raw[0] != 0,
                    "SYNTHETIC_SEED_MINTED_WIDTH"
                );
                U256::from_big_endian(&raw)
            } else {
                U256::zero()
            };
            snapshot.aspen_supply_state = AspenSupplyState::Unmigrated {
                minted_tokens: StoredDposTokenAmount::canonical_u256_after_mutation(minted),
            };
        }
        if donor.is_some() {
            anyhow::ensure!(
                snapshot.aspen_supply_state == donor_supply,
                "SYNTHETIC_DONOR_SUPPLY_CHANGED"
            );
        }
        self.validate_aspen_supply_state(identity.period, &snapshot.aspen_supply_state)?;
        // Bind from the actual complete root catalog, never the caller's sparse
        // expected rows. All changes stay on this unpublished clone until every
        // native/account/catalog check below has succeeded.
        let mut bound_rows = donor
            .map(|token| {
                token
                    .raw
                    .iter()
                    .map(|(slot, value)| ((slot.address, slot.key), value.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        for slot in rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog(
            &observation.catalog_rlp,
        )? {
            if slot.address != DPOS_CONTRACT_ADDRESS && slot.address != SLASHING_CONTRACT_ADDRESS {
                continue;
            }
            let value = match reader.storage(slot.address, ConcreteStorageKey(slot.key)) {
                Ok(ConcreteRead::Present(value)) if !value.is_empty() => Some(value),
                Ok(ConcreteRead::Absent | ConcreteRead::Tombstone | ConcreteRead::Present(_)) => {
                    None
                }
                Err(
                    error @ rustaxa_types::concrete_state::ConcreteReadError::HistoryUnavailable(_),
                ) => {
                    anyhow::ensure!(
                        donor.is_some_and(|token| token.raw.get(&slot) == Some(&None)),
                        "SYNTHETIC_SEED_CATALOG_HISTORY_UNAVAILABLE:{error:?}"
                    );
                    None
                }
                Err(error) => return Err(error.into()),
            };
            bound_rows.insert((slot.address, slot.key), value);
        }
        let authenticated_present = bound_rows
            .iter()
            .filter_map(|((address, key), value)| {
                value.as_ref().map(|_| ConcreteStorageSlot {
                    address: *address,
                    key: *key,
                })
            })
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            authenticated_present == slots_set,
            "SYNTHETIC_SEED_FULL_NATIVE_CATALOG_MISMATCH"
        );
        snapshot = stage_validator_rows_from_authenticated_catalog(
            &snapshot,
            &bound_rows,
            identity.period >= self.rewards_config.magnolia_period,
        )?;
        let canonical = canonical_concrete_precompile_storage(
            &snapshot,
            identity.period >= self.rewards_config.magnolia_period,
        )?;
        for slot in &slots_set {
            anyhow::ensure!(
                slot.address == DPOS_CONTRACT_ADDRESS || slot.address == SLASHING_CONTRACT_ADDRESS,
                "SYNTHETIC_SEED_NON_NATIVE_SLOT"
            );
            let ConcreteRead::Present(value) =
                reader.storage(slot.address, ConcreteStorageKey(slot.key))?
            else {
                anyhow::bail!("SYNTHETIC_SEED_KNOWN_ROW_MISSING");
            };
            let semantic = canonical
                .get(&(slot.address, slot.key))
                .is_some_and(|allowed| allowed.contains(&value));
            let donor_extra = !canonical.contains_key(&(slot.address, slot.key))
                && donor.is_some_and(|donor| donor.raw.get(slot) == Some(&Some(value.clone())));
            anyhow::ensure!(
                semantic || donor_extra,
                "SYNTHETIC_SEED_NATIVE_SNAPSHOT_MISMATCH:{:?}:{:?}",
                slot.address,
                slot.key
            );
            if let Some(donor) = donor {
                anyhow::ensure!(
                    donor.raw.get(slot) == Some(&Some(value)),
                    "SYNTHETIC_DONOR_UNDECLARED_TARGET_ROW"
                );
            }
        }
        for ((address, key), values) in &canonical {
            if !slots_set.contains(&ConcreteStorageSlot {
                address: *address,
                key: *key,
            }) {
                let mut jail_list_key = [0; 32];
                jail_list_key[31] = 2;
                // Go's fresh slashing reader has an empty list before any proof
                // creates this row. The complete seed inventory authenticates
                // that precise absence; it is not a reader-error fallback.
                let fresh_empty_jail_list = *address == SLASHING_CONTRACT_ADDRESS
                    && *key == jail_list_key
                    && snapshot.slashing_jailed_validators.is_empty()
                    && values.contains(&vec![0xc0]);
                anyhow::ensure!(
                    values.iter().any(Vec::is_empty) || fresh_empty_jail_list,
                    "SYNTHETIC_SEED_INCOMPLETE_NATIVE_INVENTORY:{key:?}"
                );
            }
        }
        let mut accounts = HashMap::new();
        for address in accounts_set {
            let ConcreteRead::Present(record) = reader.account(address)? else {
                anyhow::bail!("SYNTHETIC_SEED_ACCOUNT_MISSING");
            };
            if let Some(hash) = record.account.code_hash {
                let ConcreteRead::Present(code) = reader.code(hash)? else {
                    anyhow::bail!("SYNTHETIC_SEED_CODE_MISSING")
                };
                anyhow::ensure!(
                    code.len() as u64 == record.account.code_size
                        && concrete_storage_key(&[&code]) == hash,
                    "SYNTHETIC_SEED_CODE_IDENTITY"
                );
            }
            accounts.insert(
                address,
                Account {
                    nonce: record.account.nonce,
                    balance: rustaxa_types::FinalChainAccountBalance::from_snapshot_bytes(
                        &record
                            .account
                            .balance
                            .value()
                            .to_bytes_be()
                            .into_iter()
                            .skip_while(|byte| *byte == 0)
                            .collect::<Vec<_>>(),
                    )?,
                    storage_root_hash: record.account.storage_root.unwrap_or_default(),
                    code_hash: record.account.code_hash.unwrap_or_default(),
                    code_size: record.account.code_size,
                },
            );
        }
        let freq = rewards_distribution_frequency(
            &self.rewards_config.rewards_distribution_frequency,
            identity.period,
        );
        let p = identity.period.as_u64();
        let rule_start = self
            .rewards_config
            .rewards_distribution_frequency
            .iter()
            .filter(|(from, _)| *from <= identity.period)
            .map(|(from, _)| from.as_u64())
            .max()
            .unwrap_or(0);
        let expected = if freq > 1 && !p.is_multiple_of(u64::from(freq)) {
            (rule_start.max(p - p % u64::from(freq) + 1)..=p).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        anyhow::ensure!(
            pending_facts
                .iter()
                .map(|fact| fact.period)
                .collect::<Vec<_>>()
                == expected,
            "SYNTHETIC_SEED_PENDING_REWARD_INTERVAL"
        );
        let mut runtime = RewardsStatsRuntime::new(
            RewardsStatsConfig {
                committee_size: self.rewards_config.committee_size,
                magnolia_period: self.rewards_config.magnolia_period.as_u64(),
                aspen_part_one_period: self.rewards_config.aspen_part_one_period.as_u64(),
            },
            self.rewards_config
                .rewards_distribution_frequency
                .iter()
                .map(|(from, frequency)| RewardsFrequencyRule {
                    from_period: from.as_u64(),
                    frequency: *frequency,
                })
                .collect(),
            Vec::new(),
        )?;
        for fact in pending_facts {
            let plan = runtime.process_period(fact);
            anyhow::ensure!(
                plan.status == RewardsStatsStatus::Applied
                    && plan.cache_current_period
                    && plan.distribution_stats.is_empty(),
                "SYNTHETIC_SEED_INVALID_PENDING_REWARD_FACT"
            );
        }
        for row in runtime.cached_stats_rlp() {
            self.storage
                .metadata()
                .write_block_rewards_stats(row.period, &row.data)?;
        }
        let parent_hash = if head.is_genesis() {
            H256::zero()
        } else {
            H256::from_slice(
                &self
                    .storage
                    .final_chain()
                    .block_hash_by_number(head.as_u64())?
                    .ok_or_else(|| anyhow::anyhow!("SYNTHETIC_SEED_PARENT_HEADER_MISSING"))?,
            )
        };
        let header = StoredFinalChainBlockHeader {
            parent_hash,
            state_root: H256(identity.state_root),
            transactions_root: empty_trie_root(),
            receipts_root: empty_trie_root(),
            log_bloom: rustaxa_types::FinalChainLogBloom::ZERO,
            gas_used: rustaxa_types::FinalChainGas::ZERO,
            total_reward: DposTokenAmount::zero(),
        };
        let raw = StoredBlockHeaderRlpOwned::from(&header);
        let hash = H256(concrete_storage_key(&[raw.as_bytes()]));
        self.storage
            .final_chain()
            .write_block_header_with_snapshots(
                p,
                hash,
                raw.as_bytes(),
                &[0xc0],
                Some(&encode_dpos_snapshot_rlp(&snapshot)?),
                Some(&encode_account_snapshot_rlp(&accounts)),
            )?;
        self.insert_dpos_snapshot(identity.period, snapshot)?;
        self.insert_account_snapshot(identity.period, accounts)?;
        self.install_rewards_stats_runtime_snapshot(identity.period, runtime)?;
        *self
            .reward_scheduler_runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("synthetic seed scheduler lock"))? =
            reward_scheduler::FinalChainRewardSchedulerRuntime::new(identity.period);
        Ok(())
    }
}
