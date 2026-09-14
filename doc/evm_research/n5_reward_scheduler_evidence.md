# Reward scheduler lifecycle evidence

Integration corrections are committed as `cb1e133f4`.
The bounded reward scheduler slice is integrated as `f3f6bb7a8` and
`51c408c20` on `feat/rust/evm-state-db`, from the reviewed isolated commits
`621241a863e` and `e393f542b`. This closes the saved scheduler implementation
review, not N5 or the existing-network milestone.

## Ownership and lifecycle

The Rust runtime starts unbound. Only the fresh application factory's joint
StateAPI/FinalChain startup operation can establish startup authority; ordinary
recovery cannot invent it. Actual native sessions and reward projection replay
use the same authenticated StateAPI epoch basis.

Pending publication retains its durable marker through DPoS/account RAM,
reward-stat and scheduler installation. Runtime epochs are never persisted as
authority: restoring an omitted intent epoch first requires an exact armed
scheduler binding. A rejected report cannot complete startup or reopen.

Verified discard evidence survives a later failed verification read, allowing
an exact same-process retry while ordinary admission remains closed. Pre-marker
discard cleanup requires that evidence. Marker absence alone cannot reset a
live scheduler. Duplicate older publication cannot rewind newer live account
snapshots.

## Reference and regression evidence

The joined tests in
`rustaxa-consensus/src/final_chain/current_rewards_scheduler_reference_tests.rs`
compare actual Rust native reward serialization, ordered slashing writes and
scheduler transitions with both regenerated Go fixture pins over periods 2–4.
They check prior rows, exact mutations and semantic state, including a verified
epoch reset and subsequent arm/install. A separate public publication regression
checks that an older duplicate cannot overwrite newer persisted/live accounts.

The Go exporter rebuilds actual StateTransition over committed in-memory state.
This witnesses constructor-level timer reset; it does not invoke the production
Go StateAPI discard API, prove durable discard, or inject power-loss faults.
Rust lifecycle and mixed/persisted adapters provide separate bounded recovery
coverage. Full existing-network paired adoption and publication remain open.

Independent Astra review approved the completed worker source and then the
integrated pending-intent, missing-marker and committed-descriptor boundaries.
The combined build found duplicate epoch fields from automatic fixture merging;
the lead removed only those duplicates. Fresh mixed and persisted suites passed
5 and 3 tests respectively.

## Native system-fact acceptance

The integration also centralizes system-fact identity validation in Rust before
transaction planning. Returned request ID and period must match the native
request; they are never overwritten to conceal a mismatch. Rust binds pillar,
contract-address and gas-limit policy while preserving StateAPI observations.
The bridge retains transport failure diagnostics. Two targeted regressions
check both identity mismatches and owner-policy precedence. Independent Astra
review approved this contraction.

The bridge is 4,618 lines, below its prior 4,619-line budget, with unchanged
function/carrier/handle counts. No guard or expectation was weakened.

## Integrated lead validation

On the feature branch with the fixture merge corrections and native system-fact
binding, `CMAKE_BUILD_PARALLEL_LEVEL=12 make rewrite-validate-final-chain`
passed. The fast gate ran formatting, clippy, workspace tests (including 1,441
consensus unit tests), both structural guards and whitespace checks. Existing
clippy warnings remain; no warning policy was relaxed.

With `RUSTAXA_ENABLE:BOOL=ON`, all required subsystem targets actually built and
ran: `rust_consensus_tests` 15/15, `state_api_test` 3/3, and `rpc_test` 50/50.
The target also rebuilt `taraxad` and passed `--version`; that CLI check alone is
not a full node startup/recovery claim. No required target was skipped.
The additional storage bridge build/test passed 4/4 with CMake `--parallel 12`.
Targeted clang-format passed for all five changed main-only C++ paths; no
upstream-owned C++ path was touched.

The fast gate is exactly the command called by `.githooks/pre-commit`.
Since `core.hooksPath` is unset, these explicit runs, not normal commit behavior,
establish validation. No broad differential, replay or power-loss campaign ran.
