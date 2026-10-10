# Literal H1 claim-all trace over P0

This profile calls actual `TraceRunner.Trace` at block1 with an empty prefix,
one direct claim-all target, gas200000, zero value and the original nonce2^512.
Trace selects historical reader0. It does not use the supplied reader factory
to substitute P1, and it does not replace the transaction nonce.

The unchanged canonical reward-only history produces an actual genesis P0.
A copy-only commit observer captures its computed root and all82 physical rows,
with column counts14/7/17/11/33. The root is
`58a5dd667bda27fcf89ee0739bb030f469ff2317b3a3a0035dcbee9a046b813f`.
The producer checks every captured row through historical0 in the completed
database, checks its P0 account/native view, and separately preserves all92 P1
rows and their committed root after each call and repeat.

Actual results match on both pins and controls: transaction gas111272,
native quote90000, successful Run,12 raw writes, no balance effects or logs.
Native quote/Run contain48 ordered logical/backend storage events and zero
observed balance/nonce getter calls. The supplied nonce remains2^512; the
discarded sender account ends at2^512+1. Default structured JSON is stored
separately from the native observations.

Observed copies and controls use the same history and copy-only P0 capture,
quote/Run and target-result observers. Controls omit read/raw/balance taps.
The manifest binds source/helpers, instrumented source, patches and all output
bytes. Each owner is fresh, and each case repeats twice. This establishes finite
source-fixture provenance, not Rust behavior, real-chain input or root parity.

Run `python3 experiments/evm_feasibility/native_reward_claim_all_h1_trace_reference.py`
to reproduce. `--record` writes fixtures; `--capture PATH` keeps first output
before validation. Capture scratch uses persistent artifacts.
