# Independent retained-head reward planner comparison

Continuation from `e63be522d`, on `feat/rust/evm-state-db`.
The [exact report](n4_independent_reward_plan.json) reproduces **all typed reward
statistics** using independently reconstructed inputs. Raw RLP bytes still differ:
the retained legacy validator order differs from Rust's deterministic order.
Neither mismatch nor historical evidence was rewritten.

## Inputs and execution boundary

`reward_inputs --independent-artifacts` embeds SHA-bound vote, rate and retained
config artifacts from `e63be522d`. It binds H, P, Q, delayed D root/header, exact
candidate config, working-copy identity and the signed certificate block hash.
Fresh canonical signatures/voters must pair exactly with all 19 recorded voters.
Author comes from the signed PBFT block; total is **534,879**, annualized rate
**9,275,294**, and weights sum **714**. Existing RewardsStatsRuntime applies the
committee cap of **1,000**. Expected BlockStats is decoded only after planning;
it supplies no author, weights, total or rate.

The tool makes the same seven application point reads as the recovered diagnostic
and performs no new concrete-state authentication or VRF verification. Those
facts are reused exact historical evidence. Transaction and DAG facts still come
from the pinned PeriodData/receipt rows. Candidate policy, historical reward-cache
closure and the original producer remain unqualified.

The resulting plan caches the current period, does not clear the cache, and
emits zero distributions with concrete reward payload `c0`. This is an offline
planner result, not native EndBlock execution, a complete reward transition,
reward-root qualification, adoption or publication.

Report SHA-256: `f88d67ecbd06d6c483466290262fabe8fb03e4532164cf0311fdd63232bcb0e9`.
Tool source SHA-256: `9d91a94502da658d175eaeb3536ff0c816a62b8a6792cea25732a18ed9d24483`.
Candidate 566-byte row SHA-256:
`85e2aacceab96cfd4c5fe92773540106761af6f2939da1c70161b40bb99c07a9`.

```sh
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml --bin reward_inputs -- --independent-artifacts local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/independent-reward-plan.json
```

Use a fresh exclusive report filename for reruns. The supplied `data/` tree was
never opened by this diagnostic.

## Review and validation

Five targeted tests cover pinned-byte mutations, mixed identities/policy,
missing or duplicate voter mappings, certificate-hash mismatch and independence
from erased expected-output fields. Root reran them successfully; strict targeted
clippy, formatting and whitespace checks pass. Astra medium independently
approved final source and exact report fingerprints, seven-read bounds and
qualification labels.

Root also reran the legacy seven-read mode. Every output field matches
`n4_restored_reward_inputs.json` except the source fingerprint, preserving the
historical candidate behavior. The explicit `.githooks/pre-commit` fast gate passed, including all workspace
checks/tests, 1,444 consensus tests and structural guards. Logs remain under
ignored `local/evm-state-db/reports/`. This is offline Tier 1 validation; no
production, storage-library or C++ implementation changed.
