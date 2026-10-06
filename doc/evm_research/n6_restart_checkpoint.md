# Restart checkpoint: full+new historical simulation next

Branch `feat/rust/evm-state-db`; base `049ec5e73`. Accepted local work this run:
preparation `5a74db354`, zero simulation `1c5de6f75`, estimation `aa6ad522b`,
direct traces `e4c22cd7f`, one-member full+new staged slice `ae6035912`, and its frames (see Git history).
[Scope and checks](n2_redelegate_full_new.md), [scorecard](../codex_slice_scorecard.md).
No push or production routing. N1–N6/Milestone10 remain open.

Lead Sol medium session `01a10f48-83ee-7a32-8bff-722d10c36359`.
Persistent run records: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`.
Reuse `quota-budget.json` without initialization. Baseline66% remaining at
`2026-10-06T03:36:59.581Z`; target56%; reset1791584594. Latest63% at
`2026-10-06T04:16:01.545Z`:3 points consumed,7 remain. No billing inference.

All three zero API derivatives passed parity/package/serial fast and independent
Sol review. Full+new staged scope passed actual dual-pin/control parity,
consensus/EVM/check/Clippy/serial fast/whitespace and ON bridge12/all15. Fresh Sol
accepted15 frozen hashes; final pending gates subsequently completed. Only caller
order[31], absent32, amount1000, retained validators and absent current nodes are
newly admitted. Longer orders, existing current nodes, rewards and deletion remain
excluded. Details and first logs/reviews are in the run records.

Completed mechanical contract: `full-new-frames-contract.md`, seven actual
frame cases. New exporter/harness and Rust test passed actual reproduction, package/serial
fast and corrected independent review. Reuse first-attempt
full-new oracle seed only; frame configuration and accounts differ from that
StateTransition seed and must be recorded separately. Check every intermediate
raw expectation, deleted/reused caller item/count, first retained native overlay,
second missing-source failure, parent account/log rollback and committed head.
Runtime unchanged retains staged ON bridge evidence.

Next accepted one-case contract: `full-new-simulation-contract.md`. Create complete
actual H1 with d1/a1 balances4000/2000/supply6000, caller only31, a1 both.
Price0, wide d1 nonce2^264+5 precommit and supplied2^512. Both actual/current
semantic H1 reward nodes must remain absent; no pendingH2 substitution. Public
semantic owner finalizes once and loads H1 on restart. Eight disposable probes,
exact committed caller/other facts and physical byte disposal, required parity/
package/serial fast/independent Sol review before local commit.

Unknown producer identity, overrides and capture command remain unknown. Do not
repeat requests or invent facts. Qualified real-window/root parity and N4–N6 remain
open; broad gates require exact preparation and approval. Do not mutate supplied
data or upstream C++, add fallback, push or widen production authority.
