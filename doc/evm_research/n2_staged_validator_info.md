# Bounded staged validator metadata

Base: `4148a1a`, following the approved preparation commit on
`feat/rust/evm-state-db`. This is an N1 contract and N2/N3 session slice.
Milestone 10 and the complete execution/API matrices remain open.

## Contract and ownership

The selected method is `setValidatorInfo(address,string,string)`, selector
`0babea4c`. The pinned public Go revision is
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`; the local comparison revision is
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. Both actual EVM exporters ran from
disposable archives. Neither checked-out submodule nor existing fixtures were
changed. The new fixture is synthetic, with explicit accounts and sufficient
raw rows for this operation; it is not a complete historical state.

FinalChain's existing `apply_dpos_validator_info_update` kernel owns endpoint
and description byte limits (50 and 100), endpoint-first error precedence,
ownership, semantic replacement and `ValidatorInfoSet`. The new session adapter
authenticates current owner, info and iterable-position rows, in that order,
against its complete semantic snapshot. Success writes one info row encoded as
RLP `[description, endpoint]`, including identical replacements and empty strings.
It emits no ordinary account mutation. Length failures read no raw rows; a
wrong owner reads only the owner row. A read failure or raw/domain mismatch
aborts the unpublished session before semantic state advances or effects escape.
Pending execution and finalized historical simulation use this same adapter.

The source-reviewed Go contract has independent owner and info rows. Absent
owner reads as zero address; a matching owner with absent info produces
`Validator does not exist`. Its metadata writer checks iterable membership and
panics for absent/zero position. Rust's semantic metadata map and orphan-stake
invariant are different representations. This slice accepts consistent complete
Rust snapshots, authenticates their raw rows, and rejects inconsistent state.
The zero-caller/absent-owner case remains an explicit unsupported operation.
It is not evidence of full sparse-state equivalence.

The Go corpus brackets mainnet-reference fix-redelegate period 3,091,000 and
Cornus period 15,610,000. Its state, balances, VM flags and reward configuration
are synthetic; it does not claim the complete current Cacti profile. Existing
Rust session constructors reject pre-Cornus periods. Eleven post-Cornus cases
are compared through both public session types at rebased synthetic periods
1 and 0 with equivalent active native admission rules. Three pre-Cornus cases
remain Go contract evidence only. No history is fabricated to admit them.

Malformed ABI remains rejected by the existing unsupported-operation path.
Go's exact malformed and noncanonical dynamic-ABI contract is a next dependency:
selector gas/admission precedes unpack, dirty address high bytes are ignored,
dynamic offsets need not be aligned/disjoint, and unused padding is optional.
The current Rust decoder must not be presented as full Go ABI parity. Delegate
and call-code session kinds also remain unsupported. Static native mutation
is preserved for the supported decoded method.

## Executed evidence

The new Go exporter observes the actual `RequiredGas` and `Run` interface without
changing returned values, errors or state operations. Initial cache reads are
recorded separately from measured operation reads. It executes real nested CALL,
STATICCALL and parent-REVERT bytecode. Both pinned outputs are byte-identical.
The fixture records ABI bytes, caller, period, value, native funding and quote,
native and parent outcomes, exact ordered raw reads/writes, terminal raw info,
logs and transaction gas. Rust compares native quote/effects, not the complete
transaction gas accounting or fee/nonce envelope.

The 14 cases cover arbitrary string bytes, empty replacement, maximum lengths,
endpoint-first and description errors, wrong owner, missing validator, static
mutation, parent revert, 19,999 versus 20,000 native gas, neighboring activation
rules and value admission. The Rust corpus tests 22 session executions for the
11 admitted post-Cornus cases, matching native outputs/errors/gas, ordered read
keys, writes and mutation-free ordinary accounts. Parent revert comparison
models the frame owner's removal of returned logs while retaining native raw
effects; full Rust EVM-frame/API composition is still open.

Additional tests reject absent, tombstone and malformed owner/info/membership
rows, plus reader failure, without advancing semantic state. They confirm that
the session remains aborted. Two updates followed by a current-state query
exercise the evolving historical simulation view and unchanged finalized state.

First targeted run failed because the reused Rust fixture had owner `...bb`
while the new Go fixture has owner `...a1`. A dedicated Rust owner constructor
now matches the Go input identity. Oracle outputs and assertions were preserved.
Five targeted tests then passed, including two existing kernel tests. Independent
review found that the two direct-call value cases recorded the wrapper's
configured gas rather than actual native funding. The observer now records
`ctx.Gas` and depth at `RequiredGas`; the Rust comparison consumes those observed
values without a special depth override. Fresh pinned outputs and required Rust
checks replace that incorrect funding evidence.

Validation: targeted consensus tests passed; both pinned Go exporters passed;
Rust-enabled `/build` consensus bridge build and all 15 bridge tests passed.
`RUSTAXA_ENABLE:BOOL=ON` and the source-tree path were checked. The workspace fast
gate is recorded in the linked artifact directory. No storage-module or C++
source was changed. No expensive broad or differential gate was requested or run.

Complete logs, commands' exit records and source freeze:
`/home/fry/artifacts/evm-branch-2026-10-01-2233/`.
The first check had an existing unused-import warning in
`consensus_application.rs`; it was not changed by this slice.

## Review and remaining work

Sol medium implemented directly. Luna medium completed the bounded startup map.
Astra medium completed read-only contract review and is the independent reviewer
after source/evidence freeze. Requested routes are explicit; current log metadata
confirmed Sol, Luna and Astra medium. No routing failure occurred. The preparation
commit had separate Sol medium review, one quota-reader fix and one regression
test correction. Account allowance was 29% at startup and 28% after targeted
metadata/bridge checks; those are shared-account observations, not task billing.

Independent Astra medium correction review accepted the second source freeze
with no remaining findings. Corrected oracle reproduction, five targeted tests
and the workspace fast gate passed. The production Rust source did not change
after the successful bridge build/tests. Next ready work is the
metadata ABI/admission dependency and actual Rust frame/API composition. Current
Cacti coverage, other native methods, producer identity/runtime overrides,
complete historical inputs, real-window replay, receipts, roots, publication,
recovery, retained-reader acceptance and N6 remain open. Supplied data and
historical copies were not opened. Nothing was pushed or routed to production.
