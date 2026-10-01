# N4 Rust-native transition: bounded slice contract

## Objective and decision

Determine whether a real, narrow Rust EndBlock/transition path can run through existing Rust owners and produce ordered effects comparable to the pinned cold Go witness. Implement only that bounded path and its focused tests if the state and lifecycle authority are sufficient. If they are not, return a concrete blocker with a fail-closed contract and rejection tests. A planner result, static source argument, or fabricated empty result is not execution evidence.

The expected candidate source is pinned Go revision `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`. The recorded pair head is H=25,706,949; parent is H−1=25,706,948 with root `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`. Candidate policy is non-pillar with empty rewards. The cold Go witness uses exactly two fixture reads. It is a cold source witness, not complete or authenticated runtime state. It does not run warm state, transactions, reward distribution, `PrepareCommit`, root calculation, publication, or adoption. See `n4_empty_native_go_cold.md`, `n4_empty_native_effects.md`, and `n4_empty_native_effects_contract.md` for exact methods and scope.

## Rust ownership map

Inspect these candidates before assigning implementation files:

- `rust/crates/rustaxa-consensus/src/final_chain.rs` owns `FinalChain`, `DposSnapshot`, reward planning, concrete projection replay, and publication. `external_evm_concrete_projection` performs transaction-bound replay against snapshots; `publish_external_evm_publication` only publishes a validated plan and explicitly does not execute EVM or mutate external staged state.
- `rust/crates/rustaxa-consensus/src/final_chain/native_session.rs` owns `FinalChainNativeSession`, its DPoS snapshots, and `prepare`/`invoke` calls. Its constructors create unpublished sessions from FinalChain-owned state; this is not by itself a whole EndBlock API.
- `rust/crates/rustaxa-consensus/src/final_chain/native_session/semantic_port.rs` and sibling modules implement narrow stateful kernels. `rewards.rs` and `reward_scheduler.rs` contain their existing reward/session boundaries.
- `rust/crates/rustaxa-evm/src/native.rs` provides `invoke_native` over explicit read and native-execution ports. Relevant existing tests include `rust/crates/rustaxa-evm/tests/native_session_reference.rs` and `native_adapter.rs`.

These are verified code locations and entry points, not proof that a complete lifecycle facade is already available. Confirm the actual call graph and state authority before choosing owned paths. If needed, add a narrow Rust test/diagnostic composition through existing kernels under the reviewed contract; a missing facade alone is migration work, not a reason to stop. Keep C++ outside this slice. Do not add a sparse-snapshot production API to make the fixture fit. Use a suitable complete-state fixture or document why required state or authority cannot be supplied safely.

## Authority and fail-closed rules

The current owner requires a complete `DposSnapshot` for native runtime state. The two-read Go fixture and the sparse diagnostics cannot stand in for it. Do not feed them to an API that treats its snapshot as complete, infer missing rows as zero/absent, or grant them adoption/publication authority. The witness records `producer_qualified=false`, `producer_configuration_qualified=false`, and `complete_native_snapshot_reconstructed=false`. Do not add a synthetic lifecycle that merely returns no effects.

An executed-zero result is acceptable only when the Rust lifecycle actually runs with its required complete owner state and records calls, reads, ordered mutations, writes, and terminal state. Compare those observations with the pinned Go witness only for the common cold branch and candidate policy. Synthetic complete-state fixtures can prove bounded test parity; they do not prove reconstruction or authentication of the mainnet parent state. Keep fixture identity separate from historical period/root labels. Missing authority or unsupported observations must return a typed error/blocker before mutation. Tests must show that out-of-scope reads, writes, root/publication requests, and mutations are rejected and leave state unchanged. Preserve distinctions between unavailable physical history, authenticated nonmembership, and zero values.

## Exit states and checks

**Executed:** named existing owner and exact complete input; real lifecycle methods ran; call/effect order and terminal result recorded; focused Rust tests compare the common Go behavior; unsupported operations fail closed; required validation and independent review pass.

**Blocked:** identify the exact missing complete state or unresolved owner authority and explain why a safe Rust composition cannot resolve it in this slice. Add meaningful bounded contract/rejection tests where possible; report no execution, parity, zero-effect, acceptance, or publication claim. State what evidence would unblock a separate next slice.

For either state, report touched paths, validation commands/results, failed or skipped requirements, reviewer findings, and remaining N4/M10 gates. Follow `doc/rewrite_validation_strategy.md`; honor standing authorization for applicable Tier 3 checks, and request approval only for gates that existing policy reserves for task-owner approval. Commit locally after review; do not push. M10 and the real-window exit in `10_existing_network_milestone.md` remain open even if this slice passes.
