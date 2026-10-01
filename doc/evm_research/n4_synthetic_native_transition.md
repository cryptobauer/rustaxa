# N4 executed synthetic native nonboundary transition

## Result and authority

The complete two-validator synthetic fixture now executes the real Rust
`plan_external_evm_rewards_stats -> begin_native_session_bound -> finish_rewards`
path. Period 1 and distribution frequency 2 select an empty distribution plan
with cached-period intent. The terminal result has zero reward, zero account
mutations, zero raw mutations, and an unchanged complete semantic snapshot.

Fresh immutable Go `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` execution runs
`Init -> BeginBlock -> EndBlock -> Close` with the same synthetic validator,
balance, DPoS and hardfork inputs. It observes zero raw writes, zero TrieSink
mutation attempts, zero backend Put attempts and zero Commit attempts. No reward
distribution is requested; Go EndBlock has no minted-reward return value. Thus
zero mint is established by this no-distribution/no-mutation execution, while
Rust returns a typed zero reward. This is terminal effect parity for this cold
nonboundary branch, not exact read-set or complete physical snapshot parity.

Producer qualification, historical/mainnet snapshot completeness, real-window
acceptance, publication and adoption remain false. No production source route
changed. The historical two-read witness and its report are unchanged.

## Complete synthetic inputs and setup

The [shared manifest](../../experiments/evm_feasibility/fixtures/synthetic_native_transition_input.json)
records all Go configuration fields and reward facts. Validators end in `31` and
`41`; owners/delegators end in `32` and `42`. Stakes are 1000 and 2000,
commissions are 100 and 2500, and both VRF keys are 32 zero bytes. Genesis
balances before delegation are 2000 and 3000. Both owners retain 1000; DPoS
custody is 3000. Rust's constructor takes post-delegation ordinary balances and
creates custody from validator principal, as its API requires. Supply input is
5000, max supply 6000, yield 1, and blocks per year 10. The empty finalized
period fact has no transactions, DAG blocks or certificate votes. The real Rust
planner derives eligible vote count 10 from committee size; genesis DPoS votes
are 300. These are distinct inputs.

The independent Go fixture uses direct DPoS `ApplyGenesis` with a
`TransitionState` and value-only setup output. It also applies the pinned
Cornus-at-genesis DPoS code and op-stack precompile code. Setup captures 17
account updates and 27 raw updates. It serializes account/raw values directly,
without a trie, root computation, PrepareCommit or publication. Code setup is
necessary: omitting it caused the first cold run to attempt an Aspen code write,
which the observer rejected. This fixture error was corrected before parity.

The cold constructor receives parent period 0 and nonempty synthetic root
label `0x1234`. Pinned Init only enters its genesis Commit branch for an empty
state root, so this label selects the supplied synthetic state and skips root
creation. The label is not an authenticated root. All observed lifecycle reads
come from the independently constructed complete synthetic owner state.

## Observed reads and differences

Rust observes these ordered DPoS origins, derived from its complete snapshot:

| Slot | Canonical raw key | Actual value |
| --- | --- | --- |
| 4 | `f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393` | `012c` |
| 5 | `dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795` | `0bb8` |

Rust makes no ordinary account read. Go makes two backend reads in order:
DPoS account in column 2, then the absent jail-list row in column 4. The
[Go output](../../experiments/evm_feasibility/fixtures/synthetic_native_transition_go.json)
contains exact physical keys, values and presence bits. The
[Rust output](../../experiments/evm_feasibility/fixtures/synthetic_native_transition_rust.json)
contains actual raw reads, all canonical genesis rows and effects. A cold Go
DPoS contract stays lazily uninitialized in this branch; its `EndBlockCall`
returns before reading counters. Rust explicitly validates slots 4 and 5.
The read sets therefore differ.

All 27 independently generated Go raw rows match Rust canonical rows byte for
byte. Rust canonical reconstruction has 29 rows. The two extra rows are:

- DPoS slot 6 (`d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873`), an empty minted-token value; Go leaves it absent. Pinned DPoS `lazy_init` initializes minted tokens to zero and decodes an empty present value as zero. This cold branch does not read it.
- Slashing slot 2, RLP `c0`; Go leaves it absent. Pinned slashing `Reader.GetJailedValidators` starts with a nil list and decodes a present nonempty payload. Absence and `c0` each have length zero, so `CleanupJailedValidators` returns without writes. Rust also has an empty semantic list and performs no raw read for cleanup.

These differences are retained. They do not establish byte-identical full raw
snapshots. Aspen slots 6/7/8 remain absent in the Rust test state adapter, as in
the existing complete fixture; canonical export separately records the slot-6
representation above.

## Rejections and instrumentation

Four origin controls remove or malform slots 4 and 5. Each returns the existing
`RawIntegrity` error, leaves semantic state unchanged, exposes no successful
effects, and aborts later completion. A changed request returns
`RewardsPlanMismatch` before reads. Period 2 is rejected by the constructor as
`PendingPeriodMismatch`. An unbound session returns `UnboundRewards`, and a
changed rewards runtime generation fails plan validation before any read.
Existing native-session binding coverage remains intact.

The separate Go runner archives the immutable revision under persistent
`/home/fry/artifacts`, checks six source pins, and changes only observer entry
points in that archive. TrieSink StartMutation/Delete reject attempts. A
hash-guarded synchronous raw-write observer records actual ordered raw writes;
its observed list is empty. Separate-process StartMutation and Delete controls
each reject exactly one attempted mutation. Archive, patched source, helper,
fixture, harness and runner hashes are in the frozen Go output. Setup effects
are counted separately from the measured lifecycle.

## Validation and reproduction

Tier 1 plus focused bridge checks covers this test-only change; no production
authority or bridge route changed. Commands executed:

```sh
python3 experiments/evm_feasibility/synthetic_native_transition_reference.py /home/fry/artifacts/n4-synthetic-go-source-freeze.json
RUSTAXA_SYNTHETIC_REWARDS_EVIDENCE=/home/fry/artifacts/n4-synthetic-rust-source-freeze.json cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus nonboundary -- --nocapture
cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus final_chain::native_session::rewards::tests -- --nocapture
cmake --build /build --target rust_consensus_tests rust_storage_tests --parallel 12
/build/bin/rust_consensus_tests
/build/bin/rust_storage_tests
make rewrite-validate-fast
```

The focused nonboundary tests pass 3/3. The rewards module passes 16/16.
Consensus bridge tests pass 15/15; storage bridge tests pass 4/4. CMake cache
has `RUSTAXA_ENABLE:BOOL=ON`. The final fast gate passes formatting, clippy,
workspace tests, both structural guards and whitespace checks. Existing lint
warnings remain; one workspace test is ignored by its existing declaration.
No expensive differential or broad replay gate ran. Full output is retained in
`/home/fry/artifacts/n4-synthetic-fast-freeze.log`.

Use a new output file under `/home/fry/artifacts` to reproduce the Go run.
The runner refuses existing outputs and retains each disposable source archive.
No supplied DB, historical copy, transaction, DistributeRewards, PrepareCommit,
root computation, publication or adoption path was used by the synthetic case.
The mandated workspace/bridge tests exercise their existing isolated fixtures.

## Source/evidence freeze and routing

Baseline: `b279b0943bbe5c47942800bcb35b88005ecd0a22`, branch
`feat/rust/evm-state-db`. Preparation edits and unrelated files remain intact.
Requested lead route: `gpt-6.1-sol`, medium, successfully started and ran.
Luna medium completed the bounded read-only startup API check before source
edits. The live runtime/model metadata and account usage snapshots were not
exposed to the implementation lead. No quota/billing usage is inferred.
One selected-model capacity failure interrupted the lead; root requested one
resume on the same thread, which succeeded. No substitute route was used.

Start observation was 2026-10-01 14:42:44 UTC; source/evidence freeze was
approximately 15:01 UTC (about 18 minutes, including interruption and checks).
Four correction groups were required: fixture balance type API; constructor
period rejection and missing Go genesis code; explicit raw-write observer and
input naming; shared-input checks and precise representation labels. There was
no product/authority change or expected-result relaxation. Independent final
review and local commit belong to root and are still pending at this handoff.

Frozen SHA-256 values:

| Path | SHA-256 |
| --- | --- |
| `rust/crates/rustaxa-consensus/src/final_chain/native_session/rewards.rs` | `ab24a7ec16781c10ac727cbad6205035dc70140b7affd78014f55dd6c079cec9` |
| `experiments/evm_feasibility/synthetic_native_transition_reference.go` | `fdf4e21acee05534bb907838c0bb5c4f0c6b45f33082e839eefbaa9278568175` |
| `experiments/evm_feasibility/synthetic_native_transition_reference.py` | `d9dc8857c53c5ac7d8b8094becb41963cb4ea28a60e9a01ef3873259a98dd47f` |
| `experiments/evm_feasibility/fixtures/synthetic_native_transition_input.json` | `68ea481e6907f6490119f4b2a605d499180fd2d4f0dd439a9349dfb02a9040f1` |
| `experiments/evm_feasibility/fixtures/synthetic_native_transition_go.json` | `4eae0a98e2688c71d4cd305098d05350fea59a0ab5e3e5636566bf4452404f29` |
| `experiments/evm_feasibility/fixtures/synthetic_native_transition_rust.json` | `4ab076146deccefbde3c77bfaf9878fda0b7449a68f377842e92e9413daf420e` |


## Independent review and local closeout

Independent review ran after the source/evidence freeze on an explicitly
requested `gpt-6-astra`, medium route. The reviewer started and completed a
read-only review in about five minutes, with no blocking findings. Exact runtime
metadata and usage snapshots were unavailable. The reviewer verified all nine
freeze entries, six source pins, archive and observer hashes, actual lifecycle
paths, negative cases, 27 matching rows, and fast-gate evidence. The frozen
source and fixture outputs were not changed after review. This section and the
checkpoint are closeout documentation added after that review.

The contract's “32-byte delegated amount” is incorrect about the physical
width. The numeric domain is U256, but physical slot 5 is compact: pinned Go writes
`amount_delegated.Bytes()` and reads `SetBytes`; Rust `concrete_u256_bytes`
removes leading zeroes. The observed `0bb8` is correct for 3000. No assertion,
oracle or execution result was changed to match the contract's width wording.

Root reran both bridge binaries after review and retained their complete output:
`/home/fry/artifacts/n4-synthetic-consensus-bridge-review.log` (15/15, exit 0) and
`/home/fry/artifacts/n4-synthetic-storage-bridge-review.log` (4/4, exit 0).
The fast log independently includes the 16 rewards tests and three nonboundary
cases. Earlier narrow outputs also passed, but had no separate shell log.

Two nonblocking regression limits remain: the Go runner records the ordered
raw-write list without asserting that it is empty; the frozen observed list is
empty. Shared-input assertions do not bind every possible future manifest field
edit. Future fixture changes require renewed review and execution.

This local commit contains the reviewed test implementation, synthetic harness,
shared manifest, observed outputs, report and checkpoint handoff. The existing
preparation edits remain in the working tree. No push was made. Producer
qualification and the real-window gate remain open.


## Fixture-hardening follow-up — 2026-10-01

The two regression limits above are retained as historical review findings.
The [fixture-hardening report](n4_synthetic_fixture_hardening.md) closes them
with complete input contracts, raw-write rejection, fresh evidence, first-run
logs and independent Sol-medium review. The old frozen outputs and source pins
remain unchanged. Producer qualification and the real-window gate remain open.
