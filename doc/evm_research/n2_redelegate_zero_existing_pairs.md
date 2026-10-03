# Pre-Aspen-two zero redelegation with positive existing pairs

Baseline: direct full-source traces `dd0574e1f`; settled runtime `6b5228ad2`.
The staged adapter now admits amount0 only for distinct validators, both existing
positive caller pairs, retained positive validator stakes, complete histories,
post-fix active Magnolia/Ficus and authenticated zero relevant reward pools/indices.
Aspen part two keeps its normal zero-read contract failure and completed-call
sequence advancement. Zero+absent destination and present-zero source/destination
pairs remain unsupported. Other success scope guards and normal failure precedence
are unchanged. Kernel/serializers/admission/C++/storage owners are unchanged.

The untouched actual `zero_before_aspen_two` case in
[observation corpus](n2_redelegate_observations.md) supplies14 reads,12 ordered
writes, action quote80,000, transaction gas101,784, empty errors/output and one
zero-amount Redelegated log. Both stakes stay1,000 and total2,000. Zero principal
movement still updates cursor/node representations; the adapter uses its fresh
raw trace, clone, empty account port and source-first serializers before advancing.

The [focused tests](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate_zero_existing_tests.rs)
compare the actual ordered writes/log/output and unchanged numeric principal,
validator/membership order, kernel snapshot and delegation cursors/encoded rows
in pending and private synthetic historical sessions. Historical setup here is
an adapter check, not actual public DryRunner evidence. Successful sequence
advances; committed snapshots stay exact. Every14 cold read is corrupted and
reader-failed independently; each hard error poisons without semantic/sequence
advance. Every discovered warm read is corrupted after applying successful
zero effects. The warm check proves fresh authentication; it makes no repeated
Go zero-cache/write-count claim. Explicit absent/zero pair and Aspen2zero tests
preserve scope and normal zero-read failure behavior.

Luna medium supplied the bounded map; Astra medium accepted the narrow contract.
Sol medium implements directly; Astra medium final frozen review accepted all12
hashes without blocking findings; see `zero-existing-review.md` in artifacts.
First new test compared StoredDposTokenAmount encodings as principal equality;
actual mutation canonicalizes Fixed32 to Minimal. Corrected numeric equality
preserves exact raw-byte oracle and full kernel snapshot checks. First failed
run is retained. All31 redelegation tests and full EVM package tests pass;
actual dual-pin/control reproduction and affected check/clippy pass. ON bridge
build12 and all15 tests pass. Serial workspace fast passes.
Evidence uses `zero-existing-` in `/home/fry/artifacts/evm-redelegate-2026-10-03/`.
No model routing failure or billing inference is claimed.

The Go corpus lacks captured account balances. Independent pinned source reward-
transfer branch inspection plus authenticated zero rewards and Rust empty account
port support no native account effects; measured Go account-balance parity is
not claimed. Ordinary envelope accounting is separate. Actual zero-success
frames, API simulation, estimation and traces remain next composition work.
Full+new destination, source-validator deletion, reward-bearing/new-validator
and historical same-validator successes remain excluded. N1–N6/Milestone10,
real-history and production acceptance stay open. No fallback, broad gate or push.
