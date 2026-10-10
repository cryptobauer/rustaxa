# P2/P1 claim-all trace gas boundaries

The producer runs the actual Go TraceRunner on both pinned revisions. It uses
block2 and reader1 over the unchanged canonical92-row P1. The root is
`0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b`.
It runs seven transaction caps with an empty prefix and with one ordinary
zero-value self-transfer prefix. Every case runs twice from fresh private state.
The original target nonce is `2^512 + prefix_length`.

| Transaction cap | Native quote / Run | Transaction gas | Result |
| --- | --- | --- | --- |
| 21271 | Neither | 21271 | Intrinsic gas too low |
| 21272, 21273, 90000, 111271 | Quote90000; no Run | 21272 | Out of gas |
| 111272, 111273 | Quote90000; Run | 111272 | Success; payout396 |

All cases advance the target nonce once. The ordinary prefix consumes21000 gas
and advances the sender nonce once before the target. It leaves balances fixed.
Underfunded cases have10 ordered native read events, no ordinary account reads,
and no writes, balance changes or logs. Funded cases have48 native read events,
one native balance getter,12 ordered raw writes, two balance changes and one log.
The quote uses the same lazy votes400/principal4000/absent-total-supply/mint800
setup and count2 as the accepted P1 boundary oracle.

Copied-source observers record the actual prefix and target results in both
controls. Controls remove the native read, raw write, balance and account getter
taps. Original input, default trace JSON, parent error, nonce, post-state, repeats
and complete P1 stay equal. The shared fixture commit hook is inert; this profile
does not capture or claim P0 state. Manifest hashes bind all copied sources,
helpers, support, baselines, outputs and observer patches.

Run the adjacent `native_reward_claim_all_underfunded_trace_reference.py` with
`--record` to write fixtures; run without arguments to reproduce exact bytes.
Initial outputs were saved before outcome assertions. Validators also compare
the accepted direct P2/P1 boundary and funded trace fixtures.

This fixture provides finite Go API evidence. Rust parity, arbitrary bootstrap
fault timing, general traces, real roots, recovery and production routing require
separate acceptance.
