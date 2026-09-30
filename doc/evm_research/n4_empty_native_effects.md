# Conditional empty native-effects witness

From `36d2efa9f`, the reviewed [bounded contract](n4_empty_native_effects_contract.md)
was executed successfully on the guarded independent working copy. The
[exact report](n4_empty_native_effects.json) records only three top-level parent
state calls, in addition to fixed pair qualification and owner-internal paths.

At parent 25,706,948/root `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`:

- `0xfe` authenticated account is Present with **code_size 3,000**.
- `0xee` jailed-list key leftpad32(`02`) is physically Present `c0` and
  independently authenticated Member `c0`.
- The existing Rust system planner receives candidate non-pillar schedule
  H+5=25,706,954, remainder 2,954 modulo 4,000, and emits zero system transactions.
  Bridge-state fields are explicitly unread dead-branch placeholders.

The pinned historical transfer witness proves no receiver code, native calls or
code/storage mutations. The independent planner provides empty distributions;
EndBlock remains required. Under the pinned Go source and candidate policy,
positive DPoS code excludes Aspen installation, H excludes exact Cornus and
redelegation-fix writes, untouched deferred counters do not write, and an empty
jailed list makes cleanup inert for cold or warm timers. This is a **conditional
source-derived argument**, not execution of Rust EndBlock or proof of the complete
reward transition/root. Producer, full native state, adoption and publication
qualification flags remain false.

Historical artifacts retain their original paths and source hashes. Fresh state
observations are recorded separately. Any missing/zero-code account, nonempty or
nonmember jail list, raw/proof mismatch or read failure writes a partial report
and returns failure; no extra reads are attempted to overcome those gates.

Report SHA-256: `c5cc64181cd6611648c975220d29403eafd3e9d4995b34fd90bf54c03cb870ba`.
Source SHA-256: `43e852fa5162d72e69caa0ed45a3a866aece2951a18958094ea45be967fea0a2`.

```sh
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml --bin empty_native_effects -- local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/empty-native-effects-structured.json
```

Use a fresh exclusive output filename. The tool performs no transaction replay,
code-byte lookup, separate nonce lookup, counter/supply read or inventory scan.

## Validation and review

All **51 qualifier tests** passed, including five new identity/artifact/parent
rejection tests. Strict all-target clippy and formatting passed. Root explicitly
ran `.githooks/pre-commit`; all workspace checks/tests and structural guards
passed, including 1,444 consensus tests. Logs remain in the ignored reports tree.
This is Tier 1 experimental-only validation: no production Rust, storage-library,
C++ or bridge implementation changed in this slice.

Astra high mapped the historical contract and immutable source pins. Astra medium
independently approved frozen source and exact execution report, including the
failure exit after partial-report creation and the conditional qualification
boundary. Neither reviewer opened a database or ran a build.

The full [content comparison](n4_reward_input_content_comparison.json) after the
bounded input probes found source and independent copy still identical to the
original preserved manifest: 1,100,450 regular files, 9,826,980,174 logical bytes,
SHA-256 `d6f7d58c7e9ff8d7d734ea5104fc2bdd593cbfbcecf949f1ccec7a5ec259fccb`.
Full manifests remain ignored local evidence. A subsequent reporter-only rerun
used the same three read-only calls to expose exact account RLP as structured
fixture data; observations were unchanged and targeted tests/clippy passed again.
