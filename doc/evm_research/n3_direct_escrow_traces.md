# Direct escrow default structured traces

This test-only slice uses the existing `run_structured_trace_with_native`
facade, private pending session, period-wide sequence and frame journal. Actual
pinned Go `TraceRunner.Trace` supplies independent outputs. Runtime code and
the metadata corpus remain unchanged.

The [exporter](../../experiments/evm_feasibility/native_escrow_trace_reference.go)
uses the unchanged complete synthetic H=1 seed and traces period 2. Phalaenopsis,
Magnolia, Ficus and Cornus are active; Aspen part two and Cacti are inactive.
The [harness](../../experiments/evm_feasibility/native_escrow_trace_reference.py)
executes both archived pins, hashes support/exporter/harness/output bytes, and
requires exact declared target counts, gas and status. Every sequence repeats
with identical output and diagnostics; committed snapshots remain unchanged.

Seven sequences cover a zero value target, value 42 target, value-one prefix
then value-42 target, two value targets, native gas failure then a valid target,
failed prefix then valid target, and a preserved stale nonce. Admitted nonces
increase across prefix and target requests; the stale case fails before native
preparation. Targets return empty data and empty structured opcode logs. Normal
success uses 22,272 gas; native failure uses 21,272; stale admission consumes
the supplied 100,000 gas in the legacy structured result.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/escrow_trace.rs)
runs each sequence twice across two physical reader reopens: 28 runs, 44
transaction attempts and 40 native preparations. A fresh real pending session
owns the full prefix/target sequence. The shared test observer verifies sequence
IDs, transaction positions, period, depth, caller, input and value, and confirms
one port disposal per run. Request nonces stay unchanged. Every structured result
matches Go exactly, and exact committed rows stay unchanged.

An injected infrastructure failure after the successful value-one prefix must
report target stage/index zero, return no partial trace and dispose the private
port and journal. Normal native gas failure still permits the next target. This
composes the existing frame transfer/rollback contract; it does not export or
compare private account snapshots. Standalone frame tests separately compare
exact account balances and parent rollback.

## Checks and limits

Both pinned oracles passed. The first Rust compile ran before fixture generation
completed and failed on missing include files. After generation and formatting,
all 12 persisted simulation/trace tests passed. The failed and corrected outputs
are retained. Dual-pin reproduction, workspace fast and frozen independent
review passed. Independent Astra medium review accepted all nine frozen
source/fixture/report hashes without findings. Requested and confirmed routes
were Sol medium implementation and Astra medium review; no routing failures.
Complete logs use the `escrow-trace-` prefix
under `/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

This is direct default structured trace coverage with bounded values and a fixed
semantic owner. Nested targets, delayed eligibility queries, OpenEthereum,
period zero, full-width funding, semantic-owner reopen, real historical inputs
and production acceptance remain outside scope. Milestone 10 and N1–N6 are open.
