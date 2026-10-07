//! Authenticated staged retained-validator redelegation for complete, consistent snapshots.
//!
//! Every invocation uses a fresh raw trace. Normal failures authenticate their
//! cold Go read prefix before returning. Success requires post-fix distinct
//! validators, active Magnolia/Ficus and positive principal. Existing profiles
//! require zero reward pools and indices; the finite P2 partial below admits a
//! positive source pool with an untouched genesis graph. Before Aspen part two, zero amount requires both caller pairs
//! to be positive and present; it still advances reward cursors and emits writes.
//! Partial transfers admit existing or new destination pairs; full
//! caller-source removal admits an existing positive destination pair, or a new
//! pair when the caller has one source membership, or source first followed by
//! one distinct positive retained membership. Source-last admits the measured
//! bounded current-source node or the exact block-one genesis node topology.
//! New-pair insertion uses the
//! intermediate membership after swap removal. It normally requires both
//! current reward nodes absent; the exact source-current alias below is the
//! separately bounded additional node topology for full/new transfers. Both
//! validator stakes remain positive. A source-current full/new variant additionally
//! requires the shared exact count2/current cursor/head/zero-reward predicate; the
//! destination current node stays absent. Full/existing with a source current
//! node requires the companion exact two-pair [source,destination] predicate:
//! both current nodes count2, heads/cursors current and zero mirrors/pools.
//! A separate exact count3 predicate admits two validators with one common owner,
//! two positive delegators and both cursors current. It additionally authenticates
//! owner pairs and identities, complete caller/owner/validator membership ordering,
//! and deleted genesis nodes. It does not extend count2 eligibility or claim
//! physical enumeration of unrelated orphan nodes.
//! A separate exact count4 route admits the measured repeated owner partial
//! topology, with the same explicit retained-owner physical proof. It does not
//! widen count2/count3 eligibility or accept general higher counts.
//! Only source is restored; destination write_cursor retains its loaded increment.
//! Other full/existing current-source shapes are unsupported. Head, cursor,
//! current nodes and complete
//! membership ordering bind the semantic kernel to physical rows. The kernel
//! runs on a clone without account access for zero-reward routes. A separate
//! finite P2 partial profile authenticates a positive source pool and complete
//! retained-owner genesis graph, then uses lazy current account authority.
//! Source serialization precedes
//! destination serialization; only full success advances the session.
//! Reader/integrity failures abort through the enclosing session. Unsupported
//! success branches return an explicit scope error. Go block-cache/read-count
//! parity and historical same-validator success are excluded.

use super::*;

impl FinalChainNativeSession<'_> {
    /// Quotes a recognized redelegation selector before Go-compatible ABI unpack.
    /// Funding, historical depth and nonpayability precede argument errors. This
    /// method performs no storage reads; invocation authenticates business facts.
    pub(in crate::final_chain::native_session) fn prepare_redelegate(
        &mut self,
        request: &FinalChainNativeRequest,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        let selector = DposTransaction::MalformedMutation {
            selector: DPOS_REDELEGATE_SELECTOR,
        };
        let admission = self
            .final_chain
            .native_invocation_admission(
                &selector,
                request.period,
                request.depth,
                request.value.value(),
                request.supplied_gas,
                None,
            )
            .map_err(|error| {
                self.aborted = true;
                map_kernel_error(error)
            })?;
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: admission.required_gas,
        };
        let kind = if let Some(failure) = admission.failure {
            use super::super::super::native_admission::NativeAdmissionFailure as Failure;
            match failure {
                Failure::InsufficientGas => PreparedKind::InsufficientGas,
                Failure::NestedBeforeFix => PreparedKind::NestedCallRejected,
                Failure::NonPayable => PreparedKind::NonPayable,
            }
        } else {
            match decode_redelegate(&request.input, request.caller) {
                Ok(transaction) => PreparedKind::SelectedCustody(transaction),
                Err(error) => PreparedKind::AbiFailure(error),
            }
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind,
        });
        Ok(quote)
    }

    /// Authenticates, executes and serializes one prepared redelegation.
    pub(super) fn invoke_redelegate(
        &mut self,
        transaction: DposTransaction,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let DposTransaction::Redelegate {
            delegator,
            from,
            to,
            amount,
        } = transaction
        else {
            return Err(FinalChainNativeSessionError::UnsupportedOperation);
        };
        let before = &self.dpos_state;
        let amount_value = u256_from_big_endian(&amount);
        let mut trace = FinalChainNativeRawTrace::new(state);

        // The only zero-read business failures precede all validator lookups.
        let early_failure = (self.pending_period
            > self.final_chain.rewards_config.fix_redelegate_block_num
            && from == to)
            || (self.final_chain.aspen_part_two_active(self.pending_period)
                && amount_value.is_zero());
        if !early_failure {
            self.authenticate_redelegate_validator(from, &mut trace)?;
            if before.total_stakes.contains_key(&from) {
                authenticate(
                    &mut trace,
                    rewards_key(from),
                    ExpectedRaw::Exact(encode_rewards(before, from)),
                    "source rewards",
                )?;
                self.authenticate_redelegate_validator(to, &mut trace)?;
                if let Some(to_stake) = before.total_stakes.get(&to) {
                    authenticate(
                        &mut trace,
                        rewards_key(to),
                        ExpectedRaw::Exact(encode_rewards(before, to)),
                        "destination rewards",
                    )?;
                    let maximum = self.final_chain.dpos_validator_maximum_stake.as_u256();
                    let exceeds_cap = !maximum.is_zero()
                        && to_stake
                            .as_u256()
                            .checked_add(amount_value)
                            .is_none_or(|value| value > maximum);
                    if !exceeds_cap {
                        authenticate_delegation(before, from, delegator, &mut trace)?;
                    }
                }
            }
        }
        // Never let a semantic failure escape before its raw authentication.
        let failure = self
            .final_chain
            .dpos_redelegate_contract_failure(
                before,
                delegator,
                from,
                to,
                amount_value,
                self.pending_period,
            )
            .map_err(map_kernel_error)?;
        if let Some(error) = failure {
            return Ok(redelegate_result(
                DposApplyOutcome::mutation_contract_failure(error),
                quote,
                Vec::new(),
            ));
        }

        // The measured reward-bearing partial has its own finite authority
        // profile. Every zero-reward path retains its existing authentication.
        if before
            .delegator_rewards
            .get(&from)
            .is_some_and(|pool| !pool.is_zero())
        {
            return self.invoke_rewarded_partial(
                DposTransaction::Redelegate {
                    delegator,
                    from,
                    to,
                    amount,
                },
                quote,
                state,
                trace,
            );
        }

        let source_principal = delegation(before, from, delegator)
            .ok_or_else(|| domain("redelegation source principal is absent"))?;
        let full_new =
            amount_value == source_principal && delegation(before, to, delegator).is_none();
        // Removal leaves a borrowed empty slice or the single retained member.
        // Source-last still needs the shared current-node predicate below.
        let original_members = before
            .delegator_validators
            .get(&delegator)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let intermediate_members = match original_members {
            [source] if *source == from => Some(&original_members[1..]),
            [source, third]
                if *source == from
                    && *third != from
                    && *third != to
                    && delegation(before, *third, delegator)
                        .is_some_and(|principal| !principal.is_zero()) =>
            {
                Some(&original_members[1..])
            }
            [third, source]
                if *source == from
                    && *third != from
                    && *third != to
                    && delegation(before, *third, delegator)
                        .is_some_and(|principal| !principal.is_zero()) =>
            {
                Some(&original_members[..1])
            }
            _ => None,
        };
        if !self.final_chain.magnolia_active(self.pending_period)
            || !self.final_chain.ficus_active_at(self.pending_period)
            || self.pending_period <= self.final_chain.rewards_config.fix_redelegate_block_num
            || from == to
            || (amount_value.is_zero()
                && (source_principal.is_zero() || delegation(before, to, delegator).is_none()))
            || amount_value > source_principal
            // Duplicate and longer removal/append topologies remain excluded.
            || (full_new && intermediate_members.is_none())
            || delegation(before, to, delegator).is_some_and(|value| value.is_zero())
            || before
                .total_stakes
                .get(&from)
                .is_none_or(|value| value.as_u256() <= amount_value)
            || before
                .total_stakes
                .get(&to)
                .is_none_or(StoredDposTokenAmount::is_zero)
        {
            return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
        }
        if !before.delegation_ledger_history_complete
            || !before.redelegate_same_validator_history_complete
        {
            return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
        }
        validate_dpos_principal_ledger(before).map_err(map_kernel_error)?;
        if full_new {
            // Keep both-absent profiles and admit only the separately measured
            // source-current alias shape through shared kernel eligibility.
            let current = before
                .reward_reference_graph
                .current_block()
                .map_err(domain)?;
            let bounded_source = self
                .final_chain
                .bounded_redelegate_loaded_source_node(
                    before,
                    delegator,
                    from,
                    to,
                    amount_value,
                    self.pending_period,
                )
                .map_err(map_kernel_error)?;
            if original_members.len() == 2
                && original_members[1] == from
                && bounded_source.is_none()
                && !self.bounded_source_last_absent(delegator, from, to, amount_value)?
            {
                return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
            }
            for validator in [from, to] {
                match before.reward_reference_graph.load_node(&NodeKey {
                    validator,
                    block: current,
                }) {
                    Ok(_) if validator == from && bounded_source.is_some() => {}
                    Ok(_) => return Err(FinalChainNativeSessionError::CustodyScopeUnsupported),
                    Err(DposRewardGraphError::MissingNode { .. }) => {}
                    Err(error) => return Err(domain(error)),
                }
            }
        }
        let both_current_three = amount_value == source_principal
            && !full_new
            && self
                .final_chain
                .bounded_both_current_three_loaded_source_node(
                    before,
                    delegator,
                    from,
                    to,
                    amount_value,
                    self.pending_period,
                )
                .map_err(map_kernel_error)?
                .is_some();
        let both_current_four = amount_value == source_principal
            && !full_new
            && self
                .final_chain
                .bounded_both_current_four_loaded_source_node(
                    before,
                    delegator,
                    from,
                    to,
                    amount_value,
                    self.pending_period,
                )
                .map_err(map_kernel_error)?
                .is_some();
        if amount_value == source_principal && !full_new {
            let current = before
                .reward_reference_graph
                .current_block()
                .map_err(domain)?;
            match before.reward_reference_graph.load_node(&NodeKey {
                validator: from,
                block: current,
            }) {
                Ok(_) => {
                    if self
                        .final_chain
                        .bounded_existing_destination_loaded_source_node(
                            before,
                            delegator,
                            from,
                            to,
                            amount_value,
                            self.pending_period,
                        )
                        .map_err(map_kernel_error)?
                        .is_none()
                        && !both_current_three
                        && !both_current_four
                    {
                        return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
                    }
                }
                Err(DposRewardGraphError::MissingNode { .. }) => {}
                Err(error) => return Err(domain(error)),
            }
        }
        authenticate_delegation(before, to, delegator, &mut trace)?;
        for validator in [from, to] {
            authenticate_reward_scope(before, validator, delegator, &mut trace)?;
            authenticate_membership(
                &before.validator_order,
                &before.total_stakes.keys().copied().collect::<Vec<_>>(),
                &[0, 5],
                validator,
                &mut trace,
            )?;
            let members = before
                .delegator_validators
                .get(&delegator)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let expected = before
                .delegations
                .iter()
                .filter_map(|(validator, rows)| rows.contains_key(&delegator).then_some(*validator))
                .collect::<Vec<_>>();
            if delegation(before, validator, delegator).is_some() {
                authenticate_membership(
                    members,
                    &expected,
                    &delegator_validators_prefix(delegator),
                    validator,
                    &mut trace,
                )?;
            } else {
                authenticate_new_membership(
                    members,
                    &expected,
                    &delegator_validators_prefix(delegator),
                    validator,
                    &mut trace,
                )?;
            }
        }

        if both_current_three {
            // Bind the extra identities and current cursors used only by the
            // count3 predicate. Existing profiles keep their authentication stream.
            let owner = before.validator_metadata[&from].owner;
            for prefix in [&[0, 5][..], &delegator_validators_prefix(owner)] {
                authenticate(
                    &mut trace,
                    iterable_count_key(prefix),
                    ExpectedRaw::Exact(2_u32.to_le_bytes().to_vec()),
                    "count3 ordered membership count",
                )?;
                for (position, validator) in [(1, from), (2, to)] {
                    authenticate(
                        &mut trace,
                        iterable_item_key(prefix, position),
                        ExpectedRaw::Exact(validator.to_vec()),
                        "count3 ordered membership item",
                    )?;
                }
            }
            for validator in [from, to] {
                authenticate(
                    &mut trace,
                    validator_owner_key(validator),
                    ExpectedRaw::Exact(owner.to_vec()),
                    "count3 validator owner",
                )?;
                authenticate_delegation(before, validator, owner, &mut trace)?;
                authenticate_reward_scope(before, validator, owner, &mut trace)?;
                authenticate_membership(
                    &[from, to],
                    &[from, to],
                    &delegator_validators_prefix(owner),
                    validator,
                    &mut trace,
                )?;
                authenticate(
                    &mut trace,
                    reward_node_key(validator, 0),
                    ExpectedRaw::Empty,
                    "count3 deleted genesis node",
                )?;
            }
        }

        if both_current_four {
            // Bind the extra identities and current cursors used only by the
            // count4 predicate. Existing profiles keep their authentication stream.
            let owner = before.validator_metadata[&from].owner;
            for prefix in [&[0, 5][..], &delegator_validators_prefix(owner)] {
                authenticate(
                    &mut trace,
                    iterable_count_key(prefix),
                    ExpectedRaw::Exact(2_u32.to_le_bytes().to_vec()),
                    "count4 ordered membership count",
                )?;
                for (position, validator) in [(1, from), (2, to)] {
                    authenticate(
                        &mut trace,
                        iterable_item_key(prefix, position),
                        ExpectedRaw::Exact(validator.to_vec()),
                        "count4 ordered membership item",
                    )?;
                }
            }
            for validator in [from, to] {
                authenticate(
                    &mut trace,
                    validator_owner_key(validator),
                    ExpectedRaw::Exact(owner.to_vec()),
                    "count4 validator owner",
                )?;
                authenticate_delegation(before, validator, owner, &mut trace)?;
                authenticate_reward_scope(before, validator, owner, &mut trace)?;
                authenticate_membership(
                    &[from, to],
                    &[from, to],
                    &delegator_validators_prefix(owner),
                    validator,
                    &mut trace,
                )?;
                authenticate(
                    &mut trace,
                    reward_node_key(validator, 0),
                    ExpectedRaw::Empty,
                    "count4 deleted genesis node",
                )?;
            }
        }

        let mut next = before.clone();
        // An empty account context cannot read accounts or produce effects.
        let mut accounts = StagedDposAccountPort::new([]).map_err(map_kernel_error)?;
        let outcome = self
            .final_chain
            .apply_dpos_redelegate(
                &mut next,
                &mut accounts,
                delegator,
                from,
                to,
                amount,
                self.pending_period,
            )
            .map_err(map_kernel_error)?;
        if outcome.status_code != 1 || !accounts.into_mutations().is_empty() {
            return Err(domain(
                "authenticated redelegation kernel violated its success scope",
            ));
        }
        self.serialize_undelegate_principal(delegator, from, before, &next, &mut trace)?;
        if full_new {
            // Source removal leaves the retained member in the intermediate
            // order. Preserve original delegation/reward authority
            // while appending into the authenticated intermediate order.
            self.serialize_delegate_with_membership(
                delegator,
                to,
                before,
                &next,
                Some(
                    intermediate_members
                        .ok_or(FinalChainNativeSessionError::CustodyScopeUnsupported)?,
                ),
                &mut trace,
            )?;
        } else {
            self.serialize_delegate(delegator, to, before, &next, &mut trace)?;
        }
        self.dpos_state = next;
        Ok(redelegate_result(outcome, quote, trace.finish()))
    }

    fn authenticate_redelegate_validator(
        &self,
        validator: [u8; 20],
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> Result<(), FinalChainNativeSessionError> {
        let expected = if self.dpos_state.total_stakes.contains_key(&validator) {
            ExpectedRaw::OneOf(self.validator_input_encodings(&self.dpos_state, validator)?)
        } else {
            ExpectedRaw::Empty
        };
        authenticate(
            trace,
            validator_key(validator),
            expected,
            "redelegation validator",
        )
    }
}

impl FinalChainNativeSession<'_> {
    /// Admits only the measured source-last genesis shape at block one.
    /// No graph node is invented or changed. Required history/graph errors
    /// propagate; complete but different topologies return false. Physical
    /// authentication and both serializers still run through the shared trace.
    pub(super) fn bounded_source_last_absent(
        &self,
        caller: [u8; 20],
        source: [u8; 20],
        destination: [u8; 20],
        amount: U256,
    ) -> Result<bool, FinalChainNativeSessionError> {
        let before = &self.dpos_state;
        let members = before.delegator_validators.get(&caller).map(Vec::as_slice);
        let Some([retained, last]) = members else {
            return Ok(false);
        };
        if *last != source
            || *retained == source
            || *retained == destination
            || source == destination
            || amount.is_zero()
            || delegation(before, source, caller) != Some(amount)
            || delegation(before, destination, caller).is_some()
            || delegation(before, *retained, caller).is_none_or(|value| value.is_zero())
            || before
                .delegations
                .values()
                .filter(|rows| rows.contains_key(&caller))
                .count()
                != 2
            || before
                .total_stakes
                .get(&source)
                .is_none_or(|row| row.as_u256() <= amount)
            || [destination, *retained].iter().any(|v| {
                before
                    .total_stakes
                    .get(v)
                    .is_none_or(StoredDposTokenAmount::is_zero)
            })
            || self.pending_period.as_u64() != 1
            || self.pending_period <= self.final_chain.rewards_config.fix_redelegate_block_num
            || !self.final_chain.magnolia_active(self.pending_period)
            || !self.final_chain.ficus_active_at(self.pending_period)
            || self.final_chain.aspen_part_two_active(self.pending_period)
        {
            return Ok(false);
        }
        if !before.delegation_ledger_history_complete
            || !before.redelegate_same_validator_history_complete
        {
            return Err(domain(
                "source-last genesis requires complete principal history",
            ));
        }
        validate_dpos_principal_ledger(before).map_err(map_kernel_error)?;
        let graph = &before.reward_reference_graph;
        if graph.current_block().map_err(domain)? != 1 {
            return Ok(false);
        }
        let zero = DposRewardIndex::zero();
        for (validator, count, has_caller) in [
            (source, 3, true),
            (destination, 2, false),
            (*retained, 3, true),
        ] {
            if before
                .redelegate_same_validator_corruption
                .contains(&validator)
                || graph.read_validator_head(&validator).map_err(domain)? != 0
                || graph.is_stale_head(&validator).map_err(domain)?
                || before
                    .validator_reward_per_stake
                    .get(&validator)
                    .is_some_and(|row| row.index != zero)
                || before
                    .delegator_rewards
                    .get(&validator)
                    .is_some_and(|row| !row.is_zero())
                || before
                    .commission_rewards
                    .get(&validator)
                    .is_some_and(|row| !row.is_zero())
            {
                return Ok(false);
            }
            let old = graph
                .load_node(&NodeKey {
                    validator,
                    block: 0,
                })
                .map_err(domain)?;
            if old.count != count || old.reward_per_stake != zero {
                return Ok(false);
            }
            match graph.load_node(&NodeKey {
                validator,
                block: 1,
            }) {
                Err(DposRewardGraphError::MissingNode {
                    validator: actual,
                    block: 1,
                }) if actual == validator => {}
                Ok(_) => return Ok(false),
                Err(error) => return Err(domain(error)),
            }
            let mirror = before
                .delegation_reward_cursors
                .get(&validator)
                .and_then(|rows| rows.get(&caller));
            if has_caller {
                if graph.read_cursor(&validator, &caller).map_err(domain)? != 0
                    || mirror.is_none_or(|row| row.index != zero)
                {
                    return Ok(false);
                }
            } else {
                if mirror.is_some() {
                    return Ok(false);
                }
                match graph.read_cursor(&validator, &caller) {
                    Err(DposRewardGraphError::MissingCursor {
                        validator: actual,
                        delegator,
                    }) if actual == validator && delegator == caller => {}
                    Ok(_) => return Ok(false),
                    Err(error) => return Err(domain(error)),
                }
            }
        }
        Ok(true)
    }
}

/// Fixed-width Go ABI unpack: declaration order, dirty address high bytes and
/// trailing bytes are accepted. Length errors identify the first missing word.
fn decode_redelegate(input: &[u8], delegator: [u8; 20]) -> Result<DposTransaction, String> {
    let data = input
        .get(4..)
        .ok_or_else(|| "redelegation selector is absent".to_owned())?;
    let word = |offset: usize| {
        data.get(offset..offset + 32).ok_or_else(|| {
            format!(
                "abi: cannot marshal in to go type: length insufficient {} require {}",
                data.len(),
                offset + 32,
            )
        })
    };
    let from = word(0)?[12..]
        .try_into()
        .expect("ABI address has twenty bytes");
    let to = word(32)?[12..]
        .try_into()
        .expect("ABI address has twenty bytes");
    let amount = word(64)?.to_vec();
    Ok(DposTransaction::Redelegate {
        delegator,
        from,
        to,
        amount,
    })
}

fn authenticate(
    trace: &mut FinalChainNativeRawTrace<'_>,
    key: ConcreteStorageKey,
    expected: ExpectedRaw,
    label: &str,
) -> Result<(), FinalChainNativeSessionError> {
    if !expected.matches(&trace.current(DPOS_CONTRACT_ADDRESS, key)?) {
        return Err(FinalChainNativeSessionError::RawIntegrity(format!(
            "{label} raw/domain facts disagree"
        )));
    }
    Ok(())
}

fn authenticate_delegation(
    snapshot: &DposSnapshot,
    validator: [u8; 20],
    delegator: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> Result<(), FinalChainNativeSessionError> {
    let expected = if delegation(snapshot, validator, delegator).is_some() {
        ExpectedRaw::Exact(encode_delegation(snapshot, validator, delegator)?)
    } else {
        ExpectedRaw::Empty
    };
    authenticate(
        trace,
        delegation_key(validator, delegator),
        expected,
        "redelegation delegation",
    )
}

fn authenticate_reward_scope(
    snapshot: &DposSnapshot,
    validator: [u8; 20],
    delegator: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> Result<(), FinalChainNativeSessionError> {
    let graph = &snapshot.reward_reference_graph;
    let current = graph.current_block().map_err(domain)?;
    let head = graph.read_validator_head(&validator).map_err(domain)?;
    let cursor = if delegation(snapshot, validator, delegator).is_some() {
        Some(graph.read_cursor(&validator, &delegator).map_err(domain)?)
    } else {
        // Absence requires complete provenance and both semantic cursor views.
        // The physical cursor block is part of the authenticated delegation row.
        match graph.read_cursor(&validator, &delegator) {
            Err(DposRewardGraphError::MissingCursor {
                validator: missing_validator,
                delegator: missing_delegator,
            }) if missing_validator == validator && missing_delegator == delegator => {}
            Ok(_) => {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "absent redelegation destination has a graph cursor".to_owned(),
                ));
            }
            Err(error) => return Err(domain(error)),
        }
        if snapshot
            .delegation_reward_cursors
            .get(&validator)
            .is_some_and(|rows| rows.contains_key(&delegator))
        {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "absent redelegation destination has a reward cursor".to_owned(),
            ));
        }
        None
    };
    let nodes = NodeTrace::new(snapshot)?;
    // A checkpoint creation consumes the old head; both source and existing
    // destination delegation consume their cursor; a new destination has none.
    // Shared nodes need two refs.
    let mut decrements = BTreeMap::<u64, u32>::new();
    if !nodes.contains(validator, current) {
        *decrements.entry(head).or_default() += 1;
    }
    if let Some(cursor) = cursor {
        *decrements.entry(cursor).or_default() += 1;
    }
    for block in std::iter::once(head)
        .chain(cursor)
        .chain(std::iter::once(current))
    {
        let node = nodes.nodes.get(&NodeKey { validator, block });
        let expected = node
            .map(|node| ExpectedRaw::Exact(encode_node(node)))
            .unwrap_or(ExpectedRaw::Empty);
        authenticate(
            trace,
            reward_node_key(validator, block),
            expected,
            "redelegation reward node",
        )?;
        if block != current && node.is_none() {
            return Err(domain("redelegation reward reference is absent"));
        }
        if let Some(node) = node {
            if node.count < decrements.get(&block).copied().unwrap_or_default() {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "redelegation reward node count is insufficient".to_owned(),
                ));
            }
            if node.reward_per_stake != DposRewardIndex::zero() {
                return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
            }
        }
    }
    if snapshot
        .delegator_rewards
        .get(&validator)
        .is_some_and(|value| !value.is_zero())
        || snapshot
            .commission_rewards
            .get(&validator)
            .is_some_and(|value| !value.is_zero())
        || snapshot
            .validator_reward_per_stake
            .get(&validator)
            .is_some_and(|value| value.index != DposRewardIndex::zero())
        || snapshot
            .delegation_reward_cursors
            .get(&validator)
            .and_then(|rows| rows.get(&delegator))
            .is_some_and(|value| value.index != DposRewardIndex::zero())
    {
        return Err(FinalChainNativeSessionError::CustodyScopeUnsupported);
    }
    Ok(())
}

fn authenticate_membership(
    order: &[[u8; 20]],
    expected: &[[u8; 20]],
    prefix: &[u8],
    item: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> Result<(), FinalChainNativeSessionError> {
    let ordered = order
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if ordered.len() != order.len() || ordered != expected.iter().copied().collect() {
        return Err(FinalChainNativeSessionError::RawIntegrity(
            "redelegation membership ordering is incomplete".to_owned(),
        ));
    }
    let position = order
        .iter()
        .position(|value| *value == item)
        .ok_or_else(|| domain("redelegation member is absent"))?;
    let position = u32::try_from(position + 1).map_err(domain)?;
    authenticate(
        trace,
        iterable_position_key(prefix, &item),
        ExpectedRaw::Exact(position.to_le_bytes().to_vec()),
        "redelegation membership",
    )
}

/// Authenticates append authority against a complete semantic caller list.
/// Existing source membership remains; every new physical slot must be absent.
/// Reads share the invocation trace with the serializer's second checks.
fn authenticate_new_membership(
    order: &[[u8; 20]],
    expected: &[[u8; 20]],
    prefix: &[u8],
    item: [u8; 20],
    trace: &mut FinalChainNativeRawTrace<'_>,
) -> Result<(), FinalChainNativeSessionError> {
    let ordered = order
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if ordered.len() != order.len()
        || ordered != expected.iter().copied().collect()
        || ordered.contains(&item)
    {
        return Err(FinalChainNativeSessionError::RawIntegrity(
            "redelegation append ordering is incomplete or already contains destination".to_owned(),
        ));
    }
    let count = u32::try_from(order.len()).map_err(domain)?;
    let position = count
        .checked_add(1)
        .ok_or_else(|| domain("redelegation append position overflow"))?;
    authenticate(
        trace,
        iterable_position_key(prefix, &item),
        ExpectedRaw::Empty,
        "new redelegation membership",
    )?;
    authenticate(
        trace,
        iterable_item_key(prefix, position),
        ExpectedRaw::Empty,
        "new redelegation item",
    )?;
    authenticate(
        trace,
        iterable_count_key(prefix),
        ExpectedRaw::Exact(count.to_le_bytes().to_vec()),
        "new redelegation count",
    )
}

fn redelegate_result(
    outcome: DposApplyOutcome,
    quote: FinalChainNativeGasQuote,
    raw_mutations: Vec<FinalChainNativeRawMutation>,
) -> FinalChainNativeInvocationResult {
    redelegate_result_with_accounts(outcome, quote, raw_mutations, Vec::new())
}

fn redelegate_result_with_accounts(
    outcome: DposApplyOutcome,
    quote: FinalChainNativeGasQuote,
    raw_mutations: Vec<FinalChainNativeRawMutation>,
    account_mutations: Vec<FinalChainNativeOrdinaryMutation>,
) -> FinalChainNativeInvocationResult {
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
    FinalChainNativeInvocationResult::Completed(FinalChainNativeOutcome {
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
    })
}

impl FinalChainNativeSession<'_> {
    /// Executes only the measured P2 positive-source-pool partial topology.
    /// Complete two-validator/common-owner authority is authenticated before
    /// lazy account access. The existing kernel and source/destination codecs
    /// produce separate ordered account and raw vectors; their mixed publication
    /// order and parent-frame behavior are outside this method's parity scope.
    /// Errors discard scratch state and effects. A publication error must abort
    /// the enclosing session; this method is not a journal rollback owner.
    fn invoke_rewarded_partial(
        &mut self,
        transaction: DposTransaction,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
        mut trace: FinalChainNativeRawTrace<'_>,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let DposTransaction::Redelegate {
            delegator,
            from,
            to,
            amount,
        } = transaction
        else {
            return Err(FinalChainNativeSessionError::UnsupportedOperation);
        };
        let before = &self.dpos_state;
        let owner =
            self.rewarded_partial_owner(delegator, from, to, u256_from_big_endian(&amount))?;
        for validator in [from, to] {
            self.authenticate_redelegate_validator(validator, &mut trace)?;
            authenticate(
                &mut trace,
                rewards_key(validator),
                ExpectedRaw::Exact(encode_rewards(before, validator)),
                "rewarded partial pool/commission",
            )?;
            authenticate(
                &mut trace,
                validator_owner_key(validator),
                ExpectedRaw::Exact(owner.to_vec()),
                "rewarded partial owner",
            )?;
            for account in [delegator, owner] {
                authenticate_delegation(before, validator, account, &mut trace)?;
            }
            let node = before
                .reward_reference_graph
                .load_node(&NodeKey {
                    validator,
                    block: 0,
                })
                .map_err(domain)?;
            authenticate(
                &mut trace,
                reward_node_key(validator, 0),
                ExpectedRaw::Exact(encode_node(&node)),
                "rewarded partial genesis node",
            )?;
            authenticate(
                &mut trace,
                reward_node_key(validator, 2),
                ExpectedRaw::Empty,
                "rewarded partial absent current node",
            )?;
        }
        for prefix in [
            &[0, 5][..],
            &delegator_validators_prefix(delegator),
            &delegator_validators_prefix(owner),
        ] {
            authenticate(
                &mut trace,
                iterable_count_key(prefix),
                ExpectedRaw::Exact(2_u32.to_le_bytes().to_vec()),
                "rewarded partial membership count",
            )?;
            for (position, validator) in [(1, from), (2, to)] {
                authenticate(
                    &mut trace,
                    iterable_item_key(prefix, position),
                    ExpectedRaw::Exact(validator.to_vec()),
                    "rewarded partial membership item",
                )?;
                authenticate(
                    &mut trace,
                    iterable_position_key(prefix, &validator),
                    ExpectedRaw::Exact(position.to_le_bytes().to_vec()),
                    "rewarded partial membership position",
                )?;
            }
        }
        let mut next = before.clone();
        let mut accounts = StagedDposAccountPort::from_state(state);
        let outcome = self
            .final_chain
            .apply_dpos_redelegate(
                &mut next,
                &mut accounts,
                delegator,
                from,
                to,
                amount,
                self.pending_period,
            )
            .map_err(map_kernel_error)?;
        if outcome.status_code != 1 {
            return Err(domain(
                "authenticated rewarded partial kernel violated success scope",
            ));
        }
        self.serialize_undelegate_principal(delegator, from, before, &next, &mut trace)?;
        self.serialize_delegate(delegator, to, before, &next, &mut trace)?;
        let account_mutations = accounts.into_mutations();
        self.dpos_state = next;
        Ok(redelegate_result_with_accounts(
            outcome,
            quote,
            trace.finish(),
            account_mutations,
        ))
    }

    /// Returns the retained owner only for the complete measured finite graph.
    /// Values and addresses remain domain inputs; missing provenance or mirrors
    /// are hard errors. Complete but unsupported topology returns a scope error.
    fn rewarded_partial_owner(
        &self,
        caller: [u8; 20],
        from: [u8; 20],
        to: [u8; 20],
        amount: U256,
    ) -> Result<[u8; 20], FinalChainNativeSessionError> {
        let s = &self.dpos_state;
        let unsupported = || FinalChainNativeSessionError::CustodyScopeUnsupported;
        if !s.delegation_ledger_history_complete || !s.redelegate_same_validator_history_complete {
            return Err(domain(
                "rewarded partial requires complete principal and reward history",
            ));
        }
        if !s.redelegate_same_validator_corruption.is_empty() {
            return Err(unsupported());
        }
        validate_dpos_principal_ledger(s).map_err(map_kernel_error)?;
        if self.pending_period.as_u64() != 2
            || !self.final_chain.magnolia_active(self.pending_period)
            || !self.final_chain.ficus_active_at(self.pending_period)
            || self.pending_period < self.final_chain.rewards_config.aspen_part_one_period
            || self.final_chain.aspen_part_two_active(self.pending_period)
            || self.pending_period <= self.final_chain.rewards_config.fix_redelegate_block_num
            || from == to
            || caller == from
            || caller == to
            || caller == DPOS_CONTRACT_ADDRESS
            || amount.is_zero()
            || delegation(s, from, caller).is_none_or(|v| amount >= v)
            || delegation(s, to, caller).is_none_or(|v| v.is_zero())
            || s.validator_order.as_slice() != [from, to].as_slice()
            || s.total_stakes.len() != 2
            || s.validator_metadata.len() != 2
            || s.vrf_keys.len() != 2
            || s.delegations.len() != 2
            || s.delegator_validators.len() != 2
            || !s.validator_reward_per_stake.is_empty()
            || s.delegation_reward_cursors.len() != 2
            || !s.undelegations.is_empty()
            || !s.undelegations_v2.is_empty()
            || !s.undelegation_v2_last_ids.is_empty()
            || !s.slashing_jailed_validators.is_empty()
            || !s.slashing_jail_blocks.is_empty()
            || !s.slashing_double_voting_proofs.is_empty()
            || s.delegator_rewards
                .keys()
                .chain(s.commission_rewards.keys())
                .any(|v| *v != from && *v != to)
        {
            return Err(unsupported());
        }
        let owner = s
            .validator_metadata
            .get(&from)
            .ok_or_else(|| domain("rewarded partial source metadata absent"))?
            .owner;
        if owner == caller
            || owner == from
            || owner == to
            || owner == DPOS_CONTRACT_ADDRESS
            || s.validator_metadata
                .get(&to)
                .is_none_or(|row| row.owner != owner)
            || [from, to].iter().any(|v| !s.vrf_keys.contains_key(v))
            || [caller, owner].iter().any(|a| {
                s.delegator_validators.get(a).map(Vec::as_slice) != Some([from, to].as_slice())
            })
            || [from, to].iter().any(|v| {
                s.delegations.get(v).is_none_or(|rows| {
                    rows.len() != 2
                        || [caller, owner]
                            .iter()
                            .any(|a| rows.get(a).is_none_or(StoredDposTokenAmount::is_zero))
                })
            })
            || s.delegator_rewards.get(&from).is_none_or(|v| v.is_zero())
            || s.commission_rewards.get(&from).is_none_or(|v| v.is_zero())
            || s.delegator_rewards.get(&to).is_some_and(|v| !v.is_zero())
            || s.commission_rewards.get(&to).is_some_and(|v| !v.is_zero())
        {
            return Err(unsupported());
        }
        let graph = &s.reward_reference_graph;
        if graph.current_block().map_err(domain)? != 2
            || graph.live_nodes().map_err(domain)?.len() != 2
        {
            return Err(unsupported());
        }
        let zero = DposRewardIndex::zero();
        for validator in [from, to] {
            let node = graph
                .load_node(&NodeKey {
                    validator,
                    block: 0,
                })
                .map_err(domain)?;
            if node.count != 3
                || node.reward_per_stake != zero
                || graph.read_validator_head(&validator).map_err(domain)? != 0
                || graph.is_stale_head(&validator).map_err(domain)?
            {
                return Err(unsupported());
            }
            match graph.load_node(&NodeKey {
                validator,
                block: 2,
            }) {
                Err(DposRewardGraphError::MissingNode { .. }) => {}
                Ok(_) => return Err(unsupported()),
                Err(e) => return Err(domain(e)),
            }
            // Genesis has no checkpointed validator-index mirrors. Require
            // that exact absence above; node/head authority supplies index0.
            let cursors = s
                .delegation_reward_cursors
                .get(&validator)
                .ok_or_else(|| domain("rewarded partial cursor mirrors absent"))?;
            if cursors.len() != 2 {
                return Err(unsupported());
            }
            for account in [caller, owner] {
                if graph.read_cursor(&validator, &account).map_err(domain)? != 0 {
                    return Err(unsupported());
                }
                let mirror = cursors
                    .get(&account)
                    .ok_or_else(|| domain("rewarded partial cursor mirror absent"))?;
                if mirror.index != zero {
                    return Err(unsupported());
                }
            }
        }
        let stake = s.delegations[&from][&caller].amount();
        if self
            .final_chain
            .pending_delegator_reward(s, from, caller, stake)
            .map_err(map_kernel_error)?
            .is_zero()
        {
            return Err(unsupported());
        }
        Ok(owner)
    }
}
