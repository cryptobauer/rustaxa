Use `$implement-rustaxa-consensus-slice` to work through the **Remaining Consensus Work Queue** in
`doc/consensus_rewrite_tracker.md`. Treat that queue as the execution-order source of truth, `PLAN.md` as the scope and
ownership-boundary source, `doc/consensus_bridge_shim_audit.md` as the live bridge/shim deletion inventory, and
`doc/rewrite_validation_strategy.md` as validation policy. Select only `ready` items whose dependencies are satisfied.
Do not recreate completed ownership work or manufacture a slice when no qualifying gap remains.

Mark the selected queue item `active` when implementation begins and `complete` only after its completion condition,
required validation, documentation, and review have landed. Do not start a `blocked` item. Treat network, execution,
or other scope-gated work as ordinary queue items only when the tracker records explicit task-owner authorization.
Update the live bridge inventory in every slice that narrows or deletes bridge handles, CXX carriers, module flags,
shims, consumers, or compatibility-only tests.

Use RTK throughout. Decompose the dependency-ordered queue in the parent thread and route each material workstream to the
configured custom agent specified by the skill's matching role rule. Use only the roles whose scope applies rather than
invoking every role in a fixed sequence. Use `blockchain-engineer` only for explicitly scoped EVM, contract, signing, gas,
slashing-transaction, or on-chain lifecycle work.

Treat custom-agent routing as satisfied only when the spawned thread identifies the configured agent role. A generic
thread or similarly named task path is not evidence that the role or its configured model was selected. If the active
runtime cannot select or confirm the required role, stop before delegated implementation or review, report the runtime
limitation precisely, and do not substitute a generic worker or claim that the configured model ran.

Keep the selected tracker item and bridge audit synchronized; update `PLAN.md` only when scope,
architecture, or an accepted ownership boundary changes. Validate every slice at the required tier, obtain an
independent review approval before committing, and commit each coherent slice separately. Preserve
unrelated changes. Continue until no required `ready` items remain or progress is genuinely blocked by a named
dependency; report blockers precisely and do not count scope-gated follow-ups as blockers. Tier 3 test gates are
explicitly pre-approved whenever the agent judges them warranted; this standing authorization includes the expensive
`scripts/storage_conformance_diff.sh` storage differential whenever it is required or warranted. Run those gates without
requesting additional task-owner confirmation. This includes the FinalChain parity target whenever the selected slice
touches FinalChain behavior or its retained concrete-state boundary.
