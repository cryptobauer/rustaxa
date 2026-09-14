# Existing-network milestone progress checkpoint

Date: 2026-09-13. Milestone 10 remains **in progress**, not accepted. The earlier
mixed-period milestone 09 stays complete within its synthetic four-period scope;
the broader implementation plan 08 remains open.

The fresh Luna-first continuation is recorded in the
[restart checkpoint](n6_restart_checkpoint.md), the current handoff authority.
It integrates reviewed epochs, default structured traces, staged claims and the
shared setCommission kernel and the reviewed reward scheduler lifecycle.
The scheduler evidence is linked from that checkpoint. The detailed
2026-09-13 task descriptions below are historical and are superseded by that
checkpoint and the linked per-slice evidence.

## Integrated evidence

- Estimation preserves the extracted C++ search policy, with explicit rejection
  of its pathological nonprogress edge. Ordinary simulation uses fresh private
  journals and full-width stored-nonce replacement. Six actual Go DryRunner
  cases and repeated seven-probe estimation compare exact execution results.
- Ficus MCOPY, Cacti P-256, BLS registry remapping and historical Falcon kernels
  are implemented. BLS has 210 dual-pin cases, including independently reviewed
  GLV scalar-boundary corrections. Falcon has 41 cases, including signed ABI
  length, declared-versus-clamped length and reference-panic distinctions.
  Both have full top-level frame gas/value/error tests. This does not close all
  stateful native selectors or historical frame/profile behavior.
- Ordered singleton-validator Aspen2 reward distributions reproduce migration,
  per-distribution yield and supply, custody, raw writes and final semantic state.
  Zero configured yield skips rewards but flushes deferred end-block state;
  the interrupted worker's implementation was recovered and tested by the lead.
  Multi-entry maps and scoped committed-parent jailed cleanup are now integrated;
  bounded scheduler lifecycle is now integrated. Full existing-network
  publication/reopen, fault recovery and redelegation acceptance remain open.
- The read-only copied-head preflight executes all 19 real signed transactions,
  matches receipts and cumulative gas, and derives the retained head root from
  20 final account mutations using 116 calculated trie rows. It neither exposes
  nor persists those rows. Rewards/system-input qualification, writer bootstrap
  and authoritative full-period replay remain open.
- Seven actual Go TraceRunner scenarios compare five tracer configurations at
  both source pins. Prefix state and supplied nonce differ from DryRunner;
  neither-selected options cause a recorded reference panic. A typed Rust trace
  collector, driver hooks and default structured serialization are integrated.
  Native/nested/OpenEthereum/RPC trace acceptance remains open.

## Validation and review

Targeted Rust storage writer tests (7), read-only API compile-fail documentation,
the required `rust_storage_tests` C++ bridge build with 12 jobs and all four
bridge tests passed. Focused EVM primitive/frame/simulation tests, three reward
tests and affected-target strict clippy passed. The snapshot qualifier compiles
with the integrated dependencies. `make rewrite-validate-fast` passed; no expensive storage differential, broad replay, power-loss
campaign, production routing or protocol change was performed.

Independent review approved the scoped estimator, ordinary simulation, P-256,
MCOPY, corrected BLS, read-only historical preparation and ordered Aspen2 slice.
The reviewer found the Falcon decoder issues; the final seven-case correction
was implemented by the lead after tool-reported worker routing failures. The
reviewer subsequently resumed successfully and approved the zero-yield follow-up,
RETURNDATACOPY ordering and persisted historical API isolation. Falcon field
ordering passed review; one allocation edge required an additional Go witness
and fallible Rust allocation. The reviewer approved that correction and the scoped shared integration;
targeted Falcon/native-driver tests and the fast gate passed.

The configured worker tools previously reported usage-limit errors; this did
not establish actual account quota exhaustion. After the owner clarified that
usage remained available, the existing reviewer, native and state threads
resumed successfully. No current routing failure was observed on those threads.
Their partial work was inspected rather than discarded. No unreviewed worker
output is treated as approval. Luna was approved as Spark's bounded fallback;
an additional helper launch also hit the agent-thread limit. Model assignments
remain recorded in milestone 10; they were not silently reassigned.

## Current coupled work

The historical native session, frozen delayed reads, consuming simulation facade
and six-case persisted Go DryRunner comparison are integrated and reviewed
(`46eaf2e9e`, `0a01539d9`, `8aff5384e`). The multi-validator reward-map slice is
also integrated (`c535f4327`), with both observed Go orders and explicit bounded
physical-inventory qualifications. V1 custody and its actual Go oracle are
integrated as `71f6d9ab2` and `a6d800912`; direct Rust corpus assertions are
now integrated as `ee9fc24f3`, alongside retained existing lifecycle tests.
Exact Go revert diagnostic bytes are implemented as `4c7c77983`, including
non-UTF-8 reasons and 17 independent decoder witnesses.

1. Complete remaining stateful adapters and reward boundaries through existing
   Rust kernels. Scoped committed-parent jailed cleanup is integrated as
   `25c9e41e1`; it rejects unsupported cached-future and decreasing-duration
   cases. V1/V2 cancellation and scoped accrued-reward comparisons are
   integrated. Process-lifetime binding now has bounded reviewed coverage;
   actual durable StateAPI faults and existing-network adoption remain open.
2. Add trace facts to the existing iterative frame driver and serializers over
   those facts. The reviewed typed collector is integrated as `7a005eae9` and
   `d21d1769a`. Opcode hooks, default structured serialization and actual single-target
   driver-to-JSON comparisons are integrated. Cumulative refund/empty-code fixes
   have scoped sequence witnesses; the disposable default TraceRunner is integrated
   as `b936558ab`. Native/nested/OpenEthereum/RPC acceptance remains open. No second VM or
   production route is authorized.
3. Replace full account-snapshot authority for an explicit offline checkpoint
   with authenticated concrete point reads plus a touched overlay. The reviewed
   checkpoint readers and live inventories exist; the exact-identity native
   adapter is integrated as `b73055338`. It preserves full-width account values
   and physical raw-read classifications beneath current journal overlays.
   Sparse maps must not acquire complete-snapshot authority.
4. Finish native semantic reconstruction and catalog qualification before
   adoption. All H-5..H headers/roots/native accounts and sampled paths are
   readable; no sampled dependency is missing. Full head DPoS and slashing live
   inventories are authenticated. Strict inverse decoding plus 290 known
   validator/owner candidates, including the reviewed seeded undelegation
   extension, explain 2,282 of 23,278 live DPoS rows; 20,996 remain unexplained. This is partial coverage, not native bootstrap authority.
   Lite pruning is not established as the cause
   of the gap. Missing logical preimages, inverse decoding, exhaustive historical
   leaf-version checks and deleted-history/catalog policy remain distinct work.
5. Only after those inputs close, implement offline paired adoption/recovery:
   exclusive ownership of both disposable DBs, durable application intent first,
   concrete provenance/catalog batch second, application checkpoint batch last,
   exact idempotent resume and conflicting-marker rejection. This remains a
   design direction, not an implemented or accepted adoption path.
6. Finish complete historical RPC policy, native estimation/traces and integrated
   real-window recovery. Request approval for expensive acceptance campaigns
   with exact commands/data/bounds. No acceptance item can be waived because an
   agent cannot start or because a bounded synthetic fixture passes.

The original `/tmp/snapshot-litenode` remains preserved. The likely producer
commit remains owner-reported, and the exact executable and capture procedure
remain unknown. All copied-state results retain those provenance limits.


Continuation handoffs, exact model-request status and distinct capacity/usage
blockers are recorded in [the agent ledger](n6_agent_handoffs.md). No unchanged
routing failure was retried and no substitute helper was launched for Luna.


The continuation commits are `f7e3fa49f` (return-data copy ordering) and
`00195b807` (persisted historical API isolation). The fast gate, 23 native-driver
and eight simulation tests, strict affected-target clippy and dual-pin exporter
reproduction passed. Independent review and the remaining N2–N6 work remain open.
