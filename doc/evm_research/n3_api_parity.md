# N3 API implementation evidence

Status: gas search, bounded ordinary simulation and staged native simulation
are implemented, including persisted historical reads and reopen. Full N3
remains open: complete native coverage, historical API policy and Rust trace
collection/serialization.

## Existing ownership and contracts

`ExternalEvmStateOwner` currently owns general StateAPI calls and tracing.
DPoS direct calls already use the Rust query client. This milestone adds isolated
Rust backend APIs; it does not change those production routes or original C++.

`ConcreteStateRead` pins committed period/root and distinguishes absence,
tombstone, corrupt data and unavailable history. `ConcreteExecutionRead` also
admits prepared views and therefore alone is insufficient for public historical
queries. Simulations must borrow the committed port and discard a new journal
per invocation/probe. Native simulations additionally need private staged native
state with the correct delayed reader; dropping ordinary writes alone is not
enough to prove isolation.

The pinned Go `state_dry_runner/dry_runner.go` reads the requested block's state
and replaces the caller's nonce with the full-width stored sender nonce plus one.
It retains gas-price/value/fee behavior and formats ABI revert reasons. Its trace
runner instead starts at the preceding block (zero stays zero), applies ordered
state-prefix transactions, and traces the target sequence without resetting it.
These different state/nonce lifetimes must not be collapsed into one API.

Public future-block behavior is operation-specific in the current owner:
account/call can select the latest concrete state for native-only periods,
storage/code return zero/empty beyond concrete head, and traces reject periods
beyond that head. Missing retained data is not permission to invent an empty
historical state. RPC defaults, block selection and error formatting remain
separate composition work and must retain their reference behavior.

## Gas estimation kernel

`rustaxa-evm::estimate` preserves the existing `Eth.cpp` search: execute at the
cap, start the lower bound at returned gas used, probe midpoints, raise the lower
bound on code failure, terminate on consensus failure, and stop within the
integer five-percent band. Initial code failure is terminal. Callback failures
remain infrastructure errors and exact execution error strings are retained.

The finite reference harness extracts the actual unchanged search body from
`libraries/core_libs/network/rpc/eth/Eth.cpp`, compiles it in a small C++20 harness,
and commits its SHA-256 plus eight callback scenarios. Rust compares exact
results, errors and probe transcripts against that output. Reproduction:

```sh
python3 experiments/evm_feasibility/estimate_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test estimate_reference
make rewrite-validate-fast
```

The eight focused Rust tests also cover callback errors, impossible gas overrun
and nonprogress. For an inconsistent callback with a successful cap below 20,
zero gas used and a failing one-unit midpoint, legacy C++ repeats forever. Rust
explicitly returns `NonProgress` instead of reporting invented parity or success.
Ordinary admitted Taraxa transactions have intrinsic gas at least 21,000; this
is a defensive callback-contract edge, not an execution/protocol change.

Luna implemented the kernel in `task/evm-api-estimate` from `68d23f19f`; the lead
corrected/reviewed edge expectations and supplied the independent C++ extraction.
Independent Astra review approved source/contracts and the reference comparison.
This proves search policy only, not EVM execution, RPC routing or probe isolation.

## Ordinary simulation and probe isolation

`rustaxa-evm::simulation::simulate_ordinary` borrows a committed reader, requires
the block period to match, overrides the private request nonce with stored nonce
plus one, and runs the existing CALL/CREATE driver in a disposable journal. It
returns typed results and the fixed state identity, with no prepared mutations
or publication capability. Native calls fail explicitly pending their adapter.

The actual pinned Go DryRunner runs over an in-memory DB seeded through its
real incremental TrieSink. Both references produce identical six-case fixtures,
including a stored nonce above 256 bits, ignored stale and 512-bit supplied
nonces, storage-changing/value-bearing calls, a reason-bearing revert, CREATE
the zero-address gas-payment exception, and a return-data bounds failure. Reference state observations before
and after repeated calls are identical. Rust compares status, gas, output and
creation addresses. Exact RPC revert-string presentation is not yet implemented.

The return-data bounds case maps REVM `OutOfOffset` to the typed
`ReturnDataOutOfBounds` execution failure. The follow-up
[RETURNDATACOPY slice](s3_returndatacopy.md) implements Go's memory/gas-before-
bounds ordering, including overflow and reference-panic distinctions, and
compares 69 full-frame programs. Independent closeout review approved this
scoped behavior.

An integrated gas-search test runs seven fresh Rust simulations against the same
Go-derived state. Every probe sees the original slot seven and privately writes
eight; no probe sees another's write. Accounts, slots and code remain unchanged.
Other tests reject period mismatch before reading and preserve corrupt/unavailable
sender failures instead of converting them to an empty account.

```sh
python3 experiments/evm_feasibility/api_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test simulation_reference
```

Sol authored the independent public API oracle and initial comparisons in
`task/evm-api-oracle`; Astra authored the facade and integrated probe/negative
tests. The fixture manifest records exact hashes and source pins. This is
immutable in-memory state evidence, extended by the persisted historical tests
below. Native simulation, traces and complete RPC behavior remain required.


## Trace reference and required observer boundary

`trace_reference.py` runs the actual `TraceRunner.Trace` from both immutable
pins over the API oracle's committed trie. Seven scenarios cover storage,
ordered prefix plus two targets, preserved stale nonce, revert, creation,
empty code and return-data bounds. Each runs the structured logger and the
OpenEthereum `trace`, `vmTrace`, combined and neither-selected configurations.
Both pins emit identical artifacts and leave committed observations unchanged.
This is reference evidence. A typed append-only Rust trace collector now exists
(`trace.rs`), with phase-dependent opcode facts and attempted storage tracking.
Driver hooks and exact Go-compatible serializers remain under implementation;
the collector alone does not establish trace API parity.

Unlike `DryRunner.Apply`, tracing starts at `max(block - 1, 0)` and preserves
the supplied nonce. Prefix and target transactions share a disposable block
state in order; the logger resets per target. Structured output includes
`gas`, `failed`, `returnValue` and `structLogs`; each opcode records PC, opcode,
pre-charge gas, computed cost, depth, stack, expanded memory and the logger's
per-address attempted-SSTORE map. That map is not a committed storage snapshot
and is not rolled back with execution. Error fields preserve Go JSON shape.

Sources: `state_dry_runner/trace_runner.go`, `core/vm/logger.go` and
`core/vm/oelogger.go` in the pinned Go tree. `Debug.cpp:parse_tracking_parms`
accepts a nonempty array, recognizes `trace` and `vmTrace`, and currently ignores
`stateDiff` and other strings. Selecting neither causes the actual Go tracer to
panic through its nil result pointer in all seven scenarios. The oracle catches
and records that panic outside the unchanged entrypoint. Rust must expose an
explicit unsupported/reference-failure outcome rather than fabricate successful
trace output; no original C++/Go fix or production policy change is authorized.

Implementation must add an optional typed observer to the existing frame driver
and profile opcode-cost boundary, preserving the unobserved execution path.
Capture CALL/CREATE entry/exit, pre-op state plus charged dynamic cost, fault
ordering, SELFDESTRUCT and journal facts without rereading them as committed
state. Structured and OpenEthereum serializers consume those facts; neither
may execute a second interpreter to guess costs. Native calls must use the same
sequence/private native session as traced period execution. Historical reopen,
missing dependencies, nested revert/static/delegate/native calls and exact JSON
comparison are still required before N3 acceptance.

```sh
python3 experiments/evm_feasibility/trace_reference.py
```


## Persisted historical simulation and queries

The API oracle now exports its actual TrieSink seed rows. Rust materializes those
bytes in a disposable RocksDB using the existing Go-column/CF mapping and period
suffix codec, rather than rebuilding expected trie nodes. The existing Rust
concrete writer creates a distinct newer state with a zero-balance sender.
Tests open the older Go state through `ConcreteStateReader` while pinning that
newer committed descriptor; older reads and execution must not see the newer
balance. Test-only descriptor setup is not an adoption or publication API.

Across two reader reopens, tests compare exact account RLP, code bytes, physical
slot values and authenticated logical slot membership, then all six Go calls
twice and the seven-probe gas search. Every probe still sees slot seven and
privately writes eight. All default/CF1–CF8 key/value rows remain byte-identical.
Separate fixtures remove a required sender version, code row or slot row and
prove the exact unavailable-history error survives simulation and reopen without
committed mutations. Previous Go API and trace observations/results remain
unchanged; only the seed rows were added to their state observations.

This closes the bounded ordinary persisted-reader/reopen evidence gap. It does
not close native simulation, public RPC block/default/error policy, Rust tracing,
legacy bootstrap, full retention qualification or reference-binary reopen.


## Disposable native simulation composition

`simulate_with_native` reuses the historical nonce policy and existing native
CALL/CREATE driver. Its factory receives the exact validated committed identity
and must create a fresh unpublished native adapter authenticated to that state.
The facade owns and drops the port, journal and zero-based consensus sequence
on every return path; its result contains no mutation or publication authority.
Period mismatch and sender-read failures precede factory construction.

The focused scripted-port test verifies repeated probes start sequence zero,
ignore the stale supplied nonce, return identical results, preserve committed
accounts and drop the port on successful execution or infrastructure failure.
This is ownership/driver evidence only. The real FinalChain simulation wrapper,
staged current-state queries, frozen delayed eligibility and actual Go DryRunner
native corpus are separate coupled slices still under implementation.

Independent source review approved the facade contract. Strict affected-target
clippy and `make rewrite-validate-fast` passed, including all 24 native-driver
and eight historical simulation tests. No production route selects this API.


## Persisted native DryRunner parity

The actual pinned `state_dry_runner.DryRunner.Apply` corpus now covers six cases
using a committed StateTransition-generated H=1 trie. Genesis validator stake90
is below threshold100; a finalized delegation20 raises current eligibility to
11 votes. Delay1 still reads H=0 and returns zero votes. An ordinary wrapper
delegates25, queries its new current delegation, then queries delayed votes: it
returns `[25,0]`, uses94934 gas and emits the exact Delegated log. Repeating the
call leaves all committed Go observations unchanged. Malformed one/two-word
queries, a missing validator and nonzero high address bits pin exact errors and
gas. Both immutable pins produce identical typed-hex-log artifacts.

Rust materializes all78 physical seed rows in a disposable concrete database.
It independently creates the corresponding H0/H1 semantic DPoS history through
the public FinalChain constructor and native finalization, then executes every
case through `simulate_with_native` and the owning historical native wrapper.
All six cases match exact gas, status/error, return bytes, logs and full-width
nonce policy, twice across concrete-reader reopen. All default/CF1–CF8 rows are
unchanged; the finalized native stake remains110.

The fixture's `CompleteSeedReader` has absence authority only because the Go
export contains its complete, unpruned creation history. An expected physical
prefix that disappears stays `HistoryUnavailable`; an unlisted missing row
becomes absent only after an authenticated logical nonmembership proof. The
negative fixture deletes all versions of a known slot and verifies failure after
reopen. This adapter is test-only and cannot establish equivalent authority for
the mainnet light-node copy. No imported checkpoint or production route is added.

Independent source review approved this bounded composition and absence boundary.
Reproduction commands:

```sh
python3 -O experiments/evm_feasibility/native_simulation_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test native_simulation_reference
```

This closes the bounded staged-native/current-query/delayed-query simulation
comparison. Full native method coverage, complete RPC defaults/error policy,
traces and qualified existing-network bootstrap remain open.

Integrated validation: all four native simulation/support tests and strict
affected-target clippy passed; `make rewrite-validate-fast` passed. The
Rust-enabled CMake storage and consensus bridge targets built with 12 jobs;
all four storage tests and the four focused FinalChain/account-query/result
bridge tests passed. No expensive acceptance campaign was run.


## Exact revert-reason bytes

`revert::revert_reason_bytes` follows pinned Go `abi.UnpackRevert` for
`Error(string)`: full256-bit offset/length bounds, unaligned/zero offsets,
empty reasons and accepted trailing bytes. It borrows the existing input and
never allocates from a declared ABI length. `dry_run_revert_diagnostic` adds
the same `: ` suffix as `DryRunner.Apply` only after successful decoding.
Reasons remain bytes because Go strings admit invalid UTF-8; RPC JSON string
normalization is a separate still-open policy. Native contract errors are not
passed through this ordinary-REVERT helper.

The 17-case dual-pin ABI corpus is byte-identical and reproducible under Python
`-O`; the existing eight simulation tests now compare the full actual DryRunner
revert diagnostic as well as typed status and return bytes. Independent review,
strict affected-target clippy and `make rewrite-validate-fast` passed.

```sh
python3 -O experiments/evm_feasibility/revert_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test revert_reference --test simulation_reference
```

## Historical block selection and bounded trace hooks

`query_policy::select_historical_query` preserves operation-specific defaults and
future-period selection without opening a reader. Account and ordinary call
requests default to the FinalChain head then clamp to the concrete head; storage
and code default to concrete head and produce zero/empty only for explicit future
requests. Direct DPoS calls retain the requested/default FinalChain period. Trace
rejects periods above concrete head before header lookup; its selected period B
is the execution header, with state max(B-1,0) selected separately by TraceRunner.

The reference harness extracts the five complete current C++ owner methods,
substituting only recording leaves and available headers. All 188 cases match
Rust, including unequal heads, explicit genesis and u64 boundaries. This assumes
stable caller-observed heads and a readable owner. It proves selection, not RPC
parsing, concurrent lifecycle behavior, header availability or leaf-error policy.
In particular, future composition must retain original account request context:
the C++ clamped future branch and ordinary historical branch catch leaf errors
differently. Missing historical data never becomes a future-request constant.

The reviewed driver now emits bounded opcode facts from the same interpreter.
Six focused tests cover four actual Go scenarios, unsupported gas/nested-frame
paths, and exact host-error precedence. It preserves duplicate fault rows for
REVERT and return-data bounds, and emits no guessed row for unsupported gas,
early-fault or CALL/CREATE suspension paths. Frame settlement events and complete
structured/OpenEthereum output remain separate work. Current oracle refund
witnesses are zero; nonzero refund timing is source-reviewed only.

```sh
python3 -O experiments/evm_feasibility/query_policy_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test query_policy_reference --test trace_driver_reference
```
