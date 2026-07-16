Use `$implement-rustaxa-consensus-slice` to work through the **Remaining Consensus Work Queue** in
`doc/consensus_rewrite_tracker.md`. Treat that queue as the execution-order source of truth, `PLAN.md` as the scope and
ownership-boundary source, `doc/consensus_consolidation_plan.md` as the detailed slice design/history, and
`doc/consensus_bridge_shim_audit.md` as the live bridge/shim deletion inventory. Start with `CRW-01`, then select only
`ready` required items whose dependencies are satisfied. Do not recreate completed ownership work or manufacture a
slice when no qualifying gap remains.

Mark the selected `CRW-*` item `active` when implementation begins and `complete` only after its completion condition,
required validation, documentation, and review have landed. Do not start a `blocked` item. Treat `CRW-N01` and
`CRW-E01` as scope-gated follow-ups that require an explicit task-owner decision and do not block the current
non-network/non-EVM consensus closeout. Update cross-cutting `CRW-07` in every slice that narrows or deletes bridge
handles, CXX carriers, module flags, shims, or compatibility-only tests.

Use RTK throughout. Decompose the dependency-ordered queue in the parent thread and let Codex route work to the relevant
configured custom agents from their descriptions; do not prescribe a fixed per-slice agent sequence. Use
`blockchain-engineer` only for explicitly scoped EVM, contract, signing, gas, slashing-transaction, or on-chain lifecycle
work.

Keep the selected tracker item, consolidation plan, and bridge audit synchronized; update `PLAN.md` only when scope,
architecture, or an accepted ownership boundary changes. Validate every slice at the required tier, obtain an
independent review approval before committing, and commit each coherent slice separately. Preserve
unrelated changes. Continue until no required `ready` items remain or progress is genuinely blocked by a named
dependency; report blockers precisely and do not count scope-gated follow-ups as blockers. Ask before required expensive
Tier 3 or differential validation.
