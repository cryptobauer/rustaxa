# Authenticated new destination delegation

Baseline: `b56071288`. Narrow N2 extension to positive partial redelegation
into an existing positive-stake validator where the caller has no delegation.
It preserves post-fix active Magnolia/Ficus, retained positive source stake,
complete histories/order, zero relevant pools/indices and no account access.
Present zero-principal destination pairs remain excluded. No full-source,
new-validator, reward-bearing or historical same-validator success is added.

The new exporter/harness `native_redelegate_new_destination_reference.{go,py}`
keeps accepted old corpora unchanged. Both pinned archives execute 300/300 and
50/50 sequences in one actual StateTransition block, with source d1 stake1,000
and destination ownera1 stake1,000. Minimumdeposit100 does not prohibit a new
50-unit destination pair. Each first call has15 cold Go reads and14 writes;
Go cache repeat has0 reads and10 writes. Actual transaction gas is101,912 for300
and101,848 for50; action quote80,000. Zero-price/zero-value balances remain
caller3,000/other0/contract2,000 after each call; caller nonce advances0/1/2.
Native principal/cursor/membership facts are read through actual Delegations
owners over committed period1 after EndBlock. Other delegator principal,
cursor and membership stay unchanged. Both uninstrumented controls preserve
all execution and native/account facts. Manifests record sources, support,
observer code/targets, pins and all outputs/control hashes.

The adapter authenticates destination raw absence and exact semantic graph
MissingCursor after complete provenance, plus redundant reward-cursor absence.
It does not invent a physical cursor key or decrement a nonexistent cursor.
Head/current nodes and exact decrements remain authenticated. Complete semantic
caller order must agree with all delegation rows. Existing source position,
absent destination reverse position, absent append slot and exact old count
are authenticated in the same invocation-local trace. Both retained validator
positions remain required. The unchanged kernel runs on a clone with empty
account context; existing source and destination serializers run in that order;
only full success advances state. Repeat uses the existing-destination branch.
No standalone delegate minimum guard is added.

Six new tests cover exact first/repeat kernel and staged parity, principal,
cursors and both memberships, all16 first-call Rust read corruptions and reader
failure positions, all12 warm row corruptions, incomplete/duplicate order,
orphan graph/redundant zero cursors, incomplete graph, nonzero indices/pools,
new-node count overflow, excluded zero pair and normal cap/source precedence.
Rust read counts16/12 are repeated authentication assertions, not Go cache parity.
Every hard failure poisons the session, returns no effects and preserves exact
semantic state and sequence. The prior deletion-only excluded test now checks
its actual aggregate-integrity error: deleting principal alone is inconsistent,
not the consistent new destination now admitted. Other accepted tests are unchanged.
This slice tests pending staged sessions; new-branch actual frames/historical
API composition remain separate, with existing regressions preserved.

First full commands/outputs/exits have `new-destination-` prefixes under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Corrections retained: native
inspection before EndBlock saw deferred raw rows; moved native facts to committed
reader, removed a post-commit live-account read after its null-state panic;
bounded edit assertion left runtime written and test file unchanged, then fixed
its exact span; add required reward-index constructor label; compare public
human error text `count overflow` rather than enum Debug name. The first fast
run used the earlier count-text assertion and failed that new test; final rerun
passes. Actual dual-pin/control reproduction, all25 redelegate tests,
all10 frame+15 historical API tests, affected check/clippy and ON bridge build12/
all15 tests pass. Storage/upstream C++ are unchanged. Independent Astra medium review accepted all11 frozen hashes with no blockers;
final serial fast passes. Review record: new-destination-review.md in artifacts.
The review explicitly accepted50/50 and the actual other-delegator identitya1.

Routes: Sol medium direct implementation, Luna medium bounded map and Astra
medium bounded contract/final review within redelegation family, no routing
failures. Contract record is new-destination-contract-review.md in artifacts.
No production cutover, broad gate or real-network acceptance is claimed.
Next: actual new-destination frames and disposable historical API composition.
N1–N6 and Milestone10 remain open.
