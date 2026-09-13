//! Ordered raw serialization for the first staged DPoS custody mutations.
//!
//! The semantic transition reuses FinalChain's existing delegate and V1/V2
//! undelegation and confirmation kernels. This module independently emits
//! the pinned Go storage-wrapper call order from the semantic state before and
//! after that transition. Aggregate vote and delegated-amount rows remain
//! deferred to [`FinalChainNativeSession::finish_rewards`]. V1/V2 confirmation
//! rewrites a surviving validator only once the immutable FinalChain Ficus
//! boundary is active, matching the reference's conditional storage call. The
//! bounded pre-Ficus check covers that omitted operation only: the reference
//! leaves a stale persisted undelegation count there, while Rust derives it
//! from the live queue, so later-call and reopen parity remain outside scope.
//! Snapshots with more than `u16::MAX` concurrent undelegations for one
//! validator also remain outside this adapter: the existing Rust kernel rejects
//! that invalid counter state instead of reproducing the reference's wrap.

use super::account::StagedDposAccountPort;
use super::raw::FinalChainNativeRawTrace;
use super::*;
use crate::dpos_reward_graph::{Node, NodeKey};
use std::collections::BTreeMap;

impl FinalChainNativeSession<'_> {
    pub(super) fn invoke_selected_custody(
        &mut self,
        transaction: DposTransaction,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        if !self.final_chain.magnolia_active(self.pending_period) {
            return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
        }

        let before = self.dpos_state.clone();
        let mut next = before.clone();
        let mut accounts = StagedDposAccountPort::from_state(state);
        let outcome = match transaction.clone() {
            DposTransaction::Delegate {
                delegator,
                validator,
                amount,
            } => self.final_chain.apply_dpos_delegate(
                &mut next,
                &mut accounts,
                delegator,
                validator,
                amount,
            ),
            DposTransaction::UndelegateV2 {
                delegator,
                validator,
                amount,
            } => self.final_chain.apply_dpos_undelegate_v2(
                &mut next,
                &mut accounts,
                delegator,
                validator,
                amount,
                self.pending_period,
            ),
            DposTransaction::Undelegate {
                delegator,
                validator,
                amount,
            } => self.final_chain.apply_dpos_undelegate(
                &mut next,
                &mut accounts,
                delegator,
                validator,
                amount,
                self.pending_period,
            ),
            DposTransaction::ConfirmUndelegate {
                delegator,
                validator,
            } => self.final_chain.apply_dpos_confirm_undelegate(
                &mut next,
                &mut accounts,
                delegator,
                validator,
                self.pending_period,
            ),
            DposTransaction::ConfirmUndelegateV2 {
                delegator,
                validator,
                id,
            } => self.final_chain.apply_dpos_confirm_undelegate_v2(
                &mut next,
                &mut accounts,
                delegator,
                validator,
                id,
                self.pending_period,
            ),
            _ => return Err(FinalChainNativeSessionError::UnsupportedOperation),
        }
        .map_err(map_kernel_error)?;

        let status = if outcome.status_code == 1 {
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: outcome
                    .contract_error
                    .as_ref()
                    .map(DposContractError::legacy_message)
                    .unwrap_or_default(),
            }
        };
        let (account_mutations, raw_mutations) = if outcome.status_code == 1 {
            let mut trace = FinalChainNativeRawTrace::new(state);
            self.serialize_selected_custody(&transaction, &before, &next, &mut trace)?;
            self.dpos_state = next;
            (accounts.into_mutations(), trace.finish())
        } else {
            (Vec::new(), Vec::new())
        };

        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status,
                gas_used: quote.required_gas,
                output: outcome.code_retval,
                account_mutations,
                raw_mutations,
                logs: outcome
                    .logs
                    .into_iter()
                    .map(|log| FinalChainCallLog {
                        address: log.address,
                        topics: log.topics,
                        data: log.data,
                    })
                    .collect(),
            },
        ))
    }

    fn serialize_selected_custody(
        &self,
        transaction: &DposTransaction,
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        match transaction {
            DposTransaction::Delegate {
                delegator,
                validator,
                ..
            } => self.serialize_delegate(*delegator, *validator, before, after, trace),
            DposTransaction::UndelegateV2 {
                delegator,
                validator,
                ..
            } => self.serialize_undelegate_v2(*delegator, *validator, before, after, trace),
            DposTransaction::Undelegate {
                delegator,
                validator,
                ..
            } => self.serialize_undelegate_v1(*delegator, *validator, before, after, trace),
            DposTransaction::ConfirmUndelegate {
                delegator,
                validator,
            } => self.serialize_confirm_v1(*delegator, *validator, before, after, trace),
            DposTransaction::ConfirmUndelegateV2 {
                delegator,
                validator,
                id,
            } => self.serialize_confirm_v2(*delegator, *validator, *id, before, after, trace),
            _ => Err(FinalChainNativeSessionError::UnsupportedOperation),
        }
    }

    fn serialize_delegate(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let mut nodes = NodeTrace::new(before)?;
        let current_block = before
            .reward_reference_graph
            .current_block()
            .map_err(domain)?;
        if !nodes.contains(validator, current_block) {
            let head = before
                .reward_reference_graph
                .read_validator_head(&validator)
                .map_err(domain)?;
            nodes.decrement_and_write(validator, head, trace)?;
        }

        let prior_delegation = delegation(before, validator, delegator);
        if prior_delegation.is_none() {
            checked_put(
                trace,
                delegation_key(validator, delegator),
                ExpectedRaw::Empty,
                encode_delegation(after, validator, delegator)?,
                "new delegation",
            )?;
            let before_items = before
                .delegator_validators
                .get(&delegator)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|address| address.to_vec())
                .collect::<Vec<_>>();
            create_iterable(
                trace,
                &delegator_validators_prefix(delegator),
                &before_items,
                validator.to_vec(),
            )?;
        } else {
            let cursor = before
                .reward_reference_graph
                .read_cursor(&validator, &delegator)
                .map_err(domain)?;
            nodes.decrement_and_write(validator, cursor, trace)?;
            checked_put(
                trace,
                delegation_key(validator, delegator),
                ExpectedRaw::Exact(encode_delegation(before, validator, delegator)?),
                encode_delegation(after, validator, delegator)?,
                "existing delegation",
            )?;
        }

        nodes.write_final(validator, current_block, after, trace)?;
        self.put_validator(before, after, validator, trace)?;
        put_rewards(before, after, validator, trace)
    }

    fn serialize_undelegate_v2(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        self.serialize_undelegate_principal(delegator, validator, before, after, trace)?;
        create_v2_queue(before, after, delegator, validator, trace)
    }

    fn serialize_undelegate_v1(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        self.serialize_undelegate_principal(delegator, validator, before, after, trace)?;
        create_v1_queue(before, after, delegator, validator, trace)
    }

    fn serialize_undelegate_principal(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let mut nodes = NodeTrace::new(before)?;
        let current_block = before
            .reward_reference_graph
            .current_block()
            .map_err(domain)?;
        if !nodes.contains(validator, current_block) {
            let head = before
                .reward_reference_graph
                .read_validator_head(&validator)
                .map_err(domain)?;
            nodes.decrement_and_write(validator, head, trace)?;
        }
        let cursor = before
            .reward_reference_graph
            .read_cursor(&validator, &delegator)
            .map_err(domain)?;
        nodes.decrement_and_write(validator, cursor, trace)?;

        let prior = encode_delegation(before, validator, delegator)?;
        if delegation(after, validator, delegator).is_some() {
            checked_put(
                trace,
                delegation_key(validator, delegator),
                ExpectedRaw::Exact(prior),
                encode_delegation(after, validator, delegator)?,
                "partial undelegation delegation",
            )?;
        } else {
            checked_put(
                trace,
                delegation_key(validator, delegator),
                ExpectedRaw::Exact(prior),
                Vec::new(),
                "removed delegation",
            )?;
            let before_items = before
                .delegator_validators
                .get(&delegator)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|address| address.to_vec())
                .collect::<Vec<_>>();
            remove_iterable(
                trace,
                &delegator_validators_prefix(delegator),
                &before_items,
                &validator,
            )?;
        }

        nodes.write_final(validator, current_block, after, trace)?;
        self.put_validator(before, after, validator, trace)?;
        put_rewards(before, after, validator, trace)
    }

    fn serialize_confirm_v1(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let entry = find_undelegation(before, delegator, validator)
            .ok_or_else(|| domain("successful V1 confirmation lost its prior queue entry"))?;
        checked_put(
            trace,
            undelegation_v1_key(delegator, validator),
            ExpectedRaw::Exact(encode_undelegation_v1(entry)),
            Vec::new(),
            "confirmed V1 undelegation",
        )?;

        let before_entries = before
            .undelegations
            .get(&delegator)
            .cloned()
            .unwrap_or_default();
        let validators = before_entries
            .iter()
            .map(|entry| entry.validator.to_vec())
            .collect::<Vec<_>>();
        remove_iterable(
            trace,
            &undelegation_v1_validators_prefix(delegator),
            &validators,
            &validator,
        )?;

        if after.total_stakes.contains_key(&validator) {
            if self.final_chain.ficus_active_at(self.pending_period) {
                self.put_validator(before, after, validator, trace)
            } else {
                Ok(())
            }
        } else if before.total_stakes.contains_key(&validator) {
            self.serialize_deleted_validator(validator, before, trace)
        } else {
            // A pre-Magnolia full undelegation may already have removed the
            // validator while retaining its V1 custody object. Go skips the
            // Magnolia validator branch when that legacy object is confirmed.
            Ok(())
        }
    }

    fn serialize_deleted_validator(
        &self,
        validator: [u8; 20],
        before: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let metadata = before
            .validator_metadata
            .get(&validator)
            .ok_or_else(|| domain("successful validator deletion lost its prior metadata"))?;
        let vrf_key = before
            .vrf_keys
            .get(&validator)
            .ok_or_else(|| domain("successful validator deletion lost its prior VRF key"))?;
        checked_put(
            trace,
            validator_key(validator),
            ExpectedRaw::OneOf(self.validator_input_encodings(before, validator)?),
            Vec::new(),
            "deleted validator",
        )?;
        checked_put(
            trace,
            validator_info_key(validator),
            ExpectedRaw::Exact(encode_validator_info(metadata)),
            Vec::new(),
            "deleted validator info",
        )?;
        checked_put(
            trace,
            validator_owner_key(validator),
            ExpectedRaw::Exact(metadata.owner.to_vec()),
            Vec::new(),
            "deleted validator owner",
        )?;
        checked_put(
            trace,
            validator_vrf_key(validator),
            ExpectedRaw::Exact(vrf_key.to_vec()),
            Vec::new(),
            "deleted validator VRF key",
        )?;
        checked_put(
            trace,
            rewards_key(validator),
            ExpectedRaw::Exact(encode_rewards(before, validator)),
            Vec::new(),
            "deleted validator rewards",
        )?;
        let validators = before
            .validator_order
            .iter()
            .map(|address| address.to_vec())
            .collect::<Vec<_>>();
        remove_iterable(trace, &[0, 5], &validators, &validator)?;
        let mut nodes = NodeTrace::new(before)?;
        let head = before
            .reward_reference_graph
            .read_validator_head(&validator)
            .map_err(domain)?;
        nodes.decrement_and_write(validator, head, trace)
    }

    fn serialize_confirm_v2(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        id: u64,
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let entry = find_undelegation_v2(before, delegator, validator, id)
            .ok_or_else(|| domain("successful confirmation lost its prior queue entry"))?;
        checked_put(
            trace,
            undelegation_v2_key(delegator, validator, id),
            ExpectedRaw::Exact(encode_undelegation_v2(entry)),
            Vec::new(),
            "confirmed V2 undelegation",
        )?;

        let before_groups = before
            .undelegations_v2
            .get(&delegator)
            .cloned()
            .unwrap_or_default();
        let group = before_groups
            .iter()
            .find(|group| group.validator == validator)
            .ok_or_else(|| domain("successful confirmation has no prior validator queue"))?;
        let ids = group
            .entries
            .iter()
            .map(|entry| entry.id.to_le_bytes().to_vec())
            .collect::<Vec<_>>();
        remove_iterable(
            trace,
            &undelegation_ids_prefix(delegator, validator),
            &ids,
            &id.to_le_bytes(),
        )?;
        let group_remains = after
            .undelegations_v2
            .get(&delegator)
            .is_some_and(|groups| groups.iter().any(|group| group.validator == validator));
        if !group_remains {
            let validators = before_groups
                .iter()
                .map(|group| group.validator.to_vec())
                .collect::<Vec<_>>();
            remove_iterable(
                trace,
                &undelegation_validators_prefix(delegator),
                &validators,
                &validator,
            )?;
        }

        if after.total_stakes.contains_key(&validator) {
            if self.final_chain.ficus_active_at(self.pending_period) {
                self.put_validator(before, after, validator, trace)
            } else {
                Ok(())
            }
        } else {
            Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
        }
    }

    fn put_validator(
        &self,
        before: &DposSnapshot,
        after: &DposSnapshot,
        validator: [u8; 20],
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        checked_put(
            trace,
            validator_key(validator),
            ExpectedRaw::OneOf(self.validator_input_encodings(before, validator)?),
            self.encode_validator_row(after, validator)?,
            "validator",
        )
    }

    fn validator_input_encodings(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
    ) -> std::result::Result<Vec<Vec<u8>>, FinalChainNativeSessionError> {
        let (stake, commission, last_change, reward_head, undelegations_count) =
            self.validator_facts(snapshot, validator)?;
        let mut legacy = rlp::RlpStream::new_list(4);
        legacy
            .append(&stake)
            .append(&commission)
            .append(&last_change)
            .append(&reward_head);
        let legacy = legacy.out().to_vec();
        let encoded = self.encode_validator_row(snapshot, validator)?;
        if self.final_chain.magnolia_active(self.pending_period) && undelegations_count == 0 {
            Ok(vec![encoded, legacy])
        } else {
            Ok(vec![encoded])
        }
    }
}

fn map_kernel_error(error: anyhow::Error) -> FinalChainNativeSessionError {
    match error.downcast::<FinalChainNativeStateReadError>() {
        Ok(error) => FinalChainNativeSessionError::StateRead(error),
        Err(error) => FinalChainNativeSessionError::Domain(error.to_string()),
    }
}

fn domain(error: impl std::fmt::Display) -> FinalChainNativeSessionError {
    FinalChainNativeSessionError::Domain(error.to_string())
}

#[derive(Clone)]
enum ExpectedRaw {
    Exact(Vec<u8>),
    OneOf(Vec<Vec<u8>>),
    Empty,
    LittleEndianZero(usize),
}

impl ExpectedRaw {
    fn matches(&self, read: &ConcreteRead<Vec<u8>>) -> bool {
        match self {
            Self::Exact(expected) => match read {
                ConcreteRead::Present(value) => value == expected,
                ConcreteRead::Absent | ConcreteRead::Tombstone => expected.is_empty(),
            },
            Self::OneOf(expected) => match read {
                ConcreteRead::Present(value) => expected.contains(value),
                ConcreteRead::Absent | ConcreteRead::Tombstone => {
                    expected.iter().any(Vec::is_empty)
                }
            },
            Self::Empty => match read {
                ConcreteRead::Present(value) => value.is_empty(),
                ConcreteRead::Absent | ConcreteRead::Tombstone => true,
            },
            Self::LittleEndianZero(width) => match read {
                ConcreteRead::Present(value) => {
                    value.is_empty()
                        || (value.len() == *width && value.iter().all(|byte| *byte == 0))
                }
                ConcreteRead::Absent | ConcreteRead::Tombstone => true,
            },
        }
    }
}

fn checked_put(
    trace: &mut FinalChainNativeRawTrace<'_>,
    key: ConcreteStorageKey,
    expected: ExpectedRaw,
    replacement: Vec<u8>,
    label: &str,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    let current = trace.current(DPOS_CONTRACT_ADDRESS, key)?;
    if !expected.matches(&current) {
        return Err(FinalChainNativeSessionError::RawIntegrity(format!(
            "{label} raw/domain facts disagree"
        )));
    }
    trace.put(DPOS_CONTRACT_ADDRESS, key, replacement)
}

struct NodeTrace {
    nodes: BTreeMap<NodeKey, Node>,
}

impl NodeTrace {
    fn new(snapshot: &DposSnapshot) -> std::result::Result<Self, FinalChainNativeSessionError> {
        Ok(Self {
            nodes: snapshot
                .reward_reference_graph
                .live_nodes()
                .map_err(domain)?
                .into_iter()
                .collect(),
        })
    }

    fn contains(&self, validator: [u8; 20], block: u64) -> bool {
        self.nodes.contains_key(&NodeKey { validator, block })
    }

    fn decrement_and_write(
        &mut self,
        validator: [u8; 20],
        block: u64,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let key = NodeKey { validator, block };
        let before = self
            .nodes
            .get(&key)
            .cloned()
            .ok_or_else(|| domain("reward-reference node is absent"))?;
        let count = before
            .count
            .checked_sub(1)
            .ok_or_else(|| domain("reward-reference node count underflow"))?;
        let after = Node {
            reward_per_stake: before.reward_per_stake.clone(),
            count,
        };
        checked_put(
            trace,
            reward_node_key(validator, block),
            ExpectedRaw::Exact(encode_node(&before)),
            if count == 0 {
                Vec::new()
            } else {
                encode_node(&after)
            },
            "decremented reward-reference node",
        )?;
        if count == 0 {
            self.nodes.remove(&key);
        } else {
            self.nodes.insert(key, after);
        }
        Ok(())
    }

    fn write_final(
        &mut self,
        validator: [u8; 20],
        block: u64,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let key = NodeKey { validator, block };
        let final_node = after
            .reward_reference_graph
            .load_node(&key)
            .map_err(domain)?;
        let expected = self
            .nodes
            .get(&key)
            .map(|node| ExpectedRaw::Exact(encode_node(node)))
            .unwrap_or(ExpectedRaw::Empty);
        checked_put(
            trace,
            reward_node_key(validator, block),
            expected,
            encode_node(&final_node),
            "current reward-reference node",
        )?;
        self.nodes.insert(key, final_node);
        Ok(())
    }
}

fn delegation(snapshot: &DposSnapshot, validator: [u8; 20], delegator: [u8; 20]) -> Option<U256> {
    snapshot
        .delegations
        .get(&validator)
        .and_then(|delegations| delegations.get(&delegator))
        .map(StoredDposTokenAmount::as_u256)
}

fn encode_delegation(
    snapshot: &DposSnapshot,
    validator: [u8; 20],
    delegator: [u8; 20],
) -> std::result::Result<Vec<u8>, FinalChainNativeSessionError> {
    let stake = delegation(snapshot, validator, delegator)
        .ok_or_else(|| domain("delegation row is absent"))?;
    let block = snapshot
        .reward_reference_graph
        .read_cursor(&validator, &delegator)
        .map_err(domain)?;
    let mut row = rlp::RlpStream::new_list(2);
    row.append(&stake).append(&block);
    Ok(row.out().to_vec())
}

fn encode_node(node: &Node) -> Vec<u8> {
    let reward = node.reward_per_stake.to_canonical_bytes();
    let mut row = rlp::RlpStream::new_list(2);
    row.append(&reward.as_slice()).append(&node.count);
    row.out().to_vec()
}

fn put_rewards(
    before: &DposSnapshot,
    after: &DposSnapshot,
    validator: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    checked_put(
        trace,
        rewards_key(validator),
        ExpectedRaw::Exact(encode_rewards(before, validator)),
        encode_rewards(after, validator),
        "validator rewards",
    )
}

fn encode_rewards(snapshot: &DposSnapshot, validator: [u8; 20]) -> Vec<u8> {
    let mut row = rlp::RlpStream::new_list(2);
    row.append(
        &snapshot
            .delegator_rewards
            .get(&validator)
            .map(StoredDposTokenAmount::as_u256)
            .unwrap_or_default(),
    );
    row.append(
        &snapshot
            .commission_rewards
            .get(&validator)
            .map(StoredDposTokenAmount::as_u256)
            .unwrap_or_default(),
    );
    row.out().to_vec()
}

fn create_v2_queue(
    before: &DposSnapshot,
    after: &DposSnapshot,
    delegator: [u8; 20],
    validator: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    let previous_id = before
        .undelegation_v2_last_ids
        .get(&delegator)
        .copied()
        .unwrap_or_default();
    let id = after
        .undelegation_v2_last_ids
        .get(&delegator)
        .copied()
        .ok_or_else(|| domain("new V2 undelegation id is absent"))?;
    checked_put(
        trace,
        last_v2_id_key(delegator),
        if previous_id == 0 {
            ExpectedRaw::LittleEndianZero(8)
        } else {
            ExpectedRaw::Exact(previous_id.to_le_bytes().to_vec())
        },
        id.to_le_bytes().to_vec(),
        "V2 undelegation last id",
    )?;
    let entry = find_undelegation_v2(after, delegator, validator, id)
        .ok_or_else(|| domain("new V2 undelegation entry is absent"))?;
    checked_put(
        trace,
        undelegation_v2_key(delegator, validator, id),
        ExpectedRaw::Empty,
        encode_undelegation_v2(entry),
        "new V2 undelegation",
    )?;

    let before_groups = before
        .undelegations_v2
        .get(&delegator)
        .cloned()
        .unwrap_or_default();
    let prior_ids = before_groups
        .iter()
        .find(|group| group.validator == validator)
        .map(|group| {
            group
                .entries
                .iter()
                .map(|entry| entry.id.to_le_bytes().to_vec())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    create_iterable(
        trace,
        &undelegation_ids_prefix(delegator, validator),
        &prior_ids,
        id.to_le_bytes().to_vec(),
    )?;
    if prior_ids.is_empty() {
        let prior_validators = before_groups
            .iter()
            .map(|group| group.validator.to_vec())
            .collect::<Vec<_>>();
        create_iterable(
            trace,
            &undelegation_validators_prefix(delegator),
            &prior_validators,
            validator.to_vec(),
        )?;
    }
    Ok(())
}

fn create_v1_queue(
    before: &DposSnapshot,
    after: &DposSnapshot,
    delegator: [u8; 20],
    validator: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    let entry = find_undelegation(after, delegator, validator)
        .ok_or_else(|| domain("new V1 undelegation entry is absent"))?;
    checked_put(
        trace,
        undelegation_v1_key(delegator, validator),
        ExpectedRaw::Empty,
        encode_undelegation_v1(entry),
        "new V1 undelegation",
    )?;
    let validators = before
        .undelegations
        .get(&delegator)
        .into_iter()
        .flatten()
        .map(|entry| entry.validator.to_vec())
        .collect::<Vec<_>>();
    create_iterable(
        trace,
        &undelegation_v1_validators_prefix(delegator),
        &validators,
        validator.to_vec(),
    )
}

fn encode_undelegation_v1(entry: &DposUndelegation) -> Vec<u8> {
    let mut row = rlp::RlpStream::new_list(2);
    row.append(&entry.amount.as_u256()).append(&entry.block);
    row.out().to_vec()
}

fn encode_undelegation_v2(entry: &DposUndelegationV2Entry) -> Vec<u8> {
    let mut base = rlp::RlpStream::new_list(2);
    base.append(&entry.amount.as_u256()).append(&entry.block);
    let base = base.out().to_vec();
    let mut row = rlp::RlpStream::new_list(2);
    row.append_raw(&base, 1).append(&entry.id);
    row.out().to_vec()
}

fn create_iterable(
    trace: &mut FinalChainNativeRawTrace<'_>,
    prefix: &[u8],
    before: &[Vec<u8>],
    item: Vec<u8>,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    if before.iter().any(|existing| existing == &item) {
        return Err(domain("iterable-map create found an existing item"));
    }
    let position = u32::try_from(before.len() + 1)
        .map_err(|_| domain("iterable-map position exceeds uint32"))?;
    checked_put(
        trace,
        iterable_item_key(prefix, position),
        ExpectedRaw::Empty,
        item.clone(),
        "iterable-map new item",
    )?;
    checked_put(
        trace,
        iterable_position_key(prefix, &item),
        ExpectedRaw::Empty,
        position.to_le_bytes().to_vec(),
        "iterable-map new position",
    )?;
    checked_put(
        trace,
        iterable_count_key(prefix),
        if before.is_empty() {
            ExpectedRaw::LittleEndianZero(4)
        } else {
            ExpectedRaw::Exact((before.len() as u32).to_le_bytes().to_vec())
        },
        position.to_le_bytes().to_vec(),
        "iterable-map create count",
    )
}

fn remove_iterable(
    trace: &mut FinalChainNativeRawTrace<'_>,
    prefix: &[u8],
    before: &[Vec<u8>],
    item: &[u8],
) -> std::result::Result<(), FinalChainNativeSessionError> {
    let index = before
        .iter()
        .position(|existing| existing.as_slice() == item)
        .ok_or_else(|| domain("iterable-map remove item is absent"))?;
    let position =
        u32::try_from(index + 1).map_err(|_| domain("iterable-map position exceeds uint32"))?;
    let count =
        u32::try_from(before.len()).map_err(|_| domain("iterable-map count exceeds uint32"))?;
    let last = before
        .last()
        .ok_or_else(|| domain("iterable-map remove has zero count"))?;
    if position == count {
        checked_put(
            trace,
            iterable_item_key(prefix, position),
            ExpectedRaw::Exact(item.to_vec()),
            Vec::new(),
            "iterable-map last item",
        )?;
        checked_put(
            trace,
            iterable_position_key(prefix, item),
            ExpectedRaw::Exact(position.to_le_bytes().to_vec()),
            Vec::new(),
            "iterable-map last position",
        )?;
    } else {
        checked_put(
            trace,
            iterable_position_key(prefix, last),
            ExpectedRaw::Exact(count.to_le_bytes().to_vec()),
            position.to_le_bytes().to_vec(),
            "iterable-map moved position",
        )?;
        checked_put(
            trace,
            iterable_item_key(prefix, position),
            ExpectedRaw::Exact(item.to_vec()),
            last.clone(),
            "iterable-map replacement item",
        )?;
        checked_put(
            trace,
            iterable_position_key(prefix, item),
            ExpectedRaw::Exact(position.to_le_bytes().to_vec()),
            Vec::new(),
            "iterable-map removed position",
        )?;
        checked_put(
            trace,
            iterable_item_key(prefix, count),
            ExpectedRaw::Exact(last.clone()),
            Vec::new(),
            "iterable-map old last item",
        )?;
    }
    checked_put(
        trace,
        iterable_count_key(prefix),
        ExpectedRaw::Exact(count.to_le_bytes().to_vec()),
        (count - 1).to_le_bytes().to_vec(),
        "iterable-map remove count",
    )
}

fn validator_key(validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &validator]))
}

fn validator_info_key(validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[0, 1], &validator]))
}

fn rewards_key(validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[0, 2], &validator]))
}

fn validator_owner_key(validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[0, 3], &validator]))
}

fn validator_vrf_key(validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[0, 4], &validator]))
}

fn delegation_key(validator: [u8; 20], delegator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[2, 0], &validator, &delegator]))
}

fn reward_node_key(validator: [u8; 20], block: u64) -> ConcreteStorageKey {
    let block = concrete_bigint_u64_bytes(block);
    ConcreteStorageKey(concrete_storage_key(&[&[1], &validator, &block]))
}

fn last_v2_id_key(delegator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[3, 4], &delegator]))
}

fn undelegation_v2_key(delegator: [u8; 20], validator: [u8; 20], id: u64) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[
        &[3, 0],
        &delegator,
        &validator,
        &id.to_le_bytes(),
    ]))
}

fn undelegation_v1_key(delegator: [u8; 20], validator: [u8; 20]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[&[3, 0], &validator, &delegator]))
}

fn delegator_validators_prefix(delegator: [u8; 20]) -> Vec<u8> {
    [&[2, 1][..], &delegator].concat()
}

fn undelegation_validators_prefix(delegator: [u8; 20]) -> Vec<u8> {
    [&[3, 2][..], &delegator].concat()
}

fn undelegation_v1_validators_prefix(delegator: [u8; 20]) -> Vec<u8> {
    [&[3, 1][..], &delegator].concat()
}

fn undelegation_ids_prefix(delegator: [u8; 20], validator: [u8; 20]) -> Vec<u8> {
    [&[3, 3][..], &delegator, &validator].concat()
}

fn iterable_item_key(prefix: &[u8], position: u32) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[
        prefix,
        &[2],
        &position.to_le_bytes(),
    ]))
}

fn iterable_position_key(prefix: &[u8], item: &[u8]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[prefix, &[2], item]))
}

fn iterable_count_key(prefix: &[u8]) -> ConcreteStorageKey {
    ConcreteStorageKey(concrete_storage_key(&[prefix, &[1]]))
}

fn encode_validator_info(metadata: &DposValidatorMetadata) -> Vec<u8> {
    let mut row = rlp::RlpStream::new_list(2);
    row.append(&metadata.description.as_slice())
        .append(&metadata.endpoint.as_slice());
    row.out().to_vec()
}
