# Direct native structured trace composition

Base: `33e8002ac`. Objective: compose default structured tracing with real native
metadata state across an ordered prefix/target sequence. No production route,
publication, supplied-data mutation or C++ change.

## Contract and implementation

Independent Astra medium contract review approved the bounded design before
runtime implementation. Pinned Go `TraceRunner.Trace` uses one disposable state
at max(B-1,0), supplied nonces, live native block state, unobserved prefixes and a
fresh structured logger per target. It does not reset the journal between calls.
Direct native calls execute no interpreter and therefore have no opcode rows.

`trace_runner::run_structured_trace_with_native` validates the exact preceding
reader period before factory invocation or state access. The factory receives
both concrete identity and execution period and owns semantic/configuration
authentication. One private port, sequence and journal span all calls and are
dropped on return or error. Prefixes reuse native CALL/CREATE drivers. Direct
native CALL targets reuse the same driver with empty collectors; other targets
retain the existing bounded traced driver and explicit unsupported paths.
Factory errors have a separate typed variant. Driver errors retain prefix/target
stage and index; normal code/consensus failures produce result rows and permit
later targets. No prepared state, write set or publication authority escapes.

The public API explicitly requires method-specific compatibility with Go's live
native reader factory. A pending Rust session at B=2 over H=1 proves current
metadata/update/query semantics, not all delayed queries. A DryRunner historical
session is not substituted. Period-zero native sessions, nested target tracing,
OpenEthereum modes and complete native trace coverage remain unqualified.

## References and tests

The new exporter uses actual `TraceRunner.Trace` from both immutable Go pins,
the existing complete H=1 native seed and metadata ABI helper. It runs seven
sequences twice, compares exact results/diagnostics and complete before/after
snapshots. The harness additionally checks that prefix and target updates reach
queries and that a failed target is followed by a successful target. Unchanged
shared exporter hashes, harness and output hashes are recorded in the manifest.

Rust compares exact JSON values/shapes, including empty `structLogs`, across two
physical reader reopens and two fresh ports per sequence: 28 trace runs with 52
prefix/target transaction attempts. Four stale-nonce attempts stop before native
preparation; the other 48 native calls have exact contiguous invocation IDs,
transaction positions, period, depth, caller and input. Supplied nonces stay
unchanged. An injected infrastructure error after a mutating prefix returns no
partial trace and drops its port; committed physical rows remain unchanged.
Separate no-read tests prove period mismatch precedes the factory and factory
failure precedes account/storage/code access.

The first compile found a test transaction-position width error (u64 versus
u32). The next run exposed repeated nonces: later reference calls were stopping
at admission, so the exporter now supplies increasing nonces for state-visibility
cases. The stale nonce remains a separate rejection case. Parsing zero then
exposed its canonical empty-byte nonce representation; that test conversion was
corrected. No existing test was changed and runtime results already matched Go.
Complete failed/corrected logs and exit codes are retained at
`/home/fry/artifacts/evm-branch-2026-10-01-2233/metadata-trace-*`.

All 30 focused native-simulation/trace runner/driver/serializer tests passed.
Both pinned metadata trace fixtures matched and reproduced. Workspace fast gate
passed. Independent frozen Astra medium review accepted the sources/evidence
without findings. No shim or bridge API is added
or changed; validation exercises the new public Rust facade directly.

Requested and confirmed implementation/contract routes: Sol medium and Astra
medium. Final review requested and confirmed Astra medium. No routing failures.
Allowance remained 26% after validation/review; this is not task billing.
N3 and Milestone 10 remain open.
