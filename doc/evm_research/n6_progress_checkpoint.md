# Existing-network milestone progress checkpoint

Date: 2026-09-13. Milestone 10 remains **in progress**, not accepted. The earlier
mixed-period milestone 09 stays complete within its synthetic four-period scope;
the broader implementation plan 08 remains open.

## Integrated evidence

- Estimation preserves the extracted C++ search policy, with explicit rejection
  of its pathological nonprogress edge. Ordinary simulation uses fresh private
  journals and full-width stored-nonce replacement. Six actual Go DryRunner
  cases and repeated seven-probe estimation compare exact execution results.
- Ficus MCOPY, Cacti P-256, BLS registry remapping and historical Falcon kernels
  are implemented. BLS has 210 dual-pin cases, including independently reviewed
  GLV scalar-boundary corrections. Falcon has 40 cases, including signed ABI
  length, declared-versus-clamped length and reference-panic distinctions.
  Both have full top-level frame gas/value/error tests. This does not close all
  stateful native selectors or historical frame/profile behavior.
- Ordered singleton-validator Aspen2 reward distributions reproduce migration,
  per-distribution yield and supply, custody, raw writes and final semantic state.
  Zero configured yield skips rewards but flushes deferred end-block state;
  the interrupted worker's implementation was recovered and tested by the lead.
  Multi-entry maps, jailed cleanup and the redelegation correction remain open.
- The read-only copied-head preflight executes all 19 real signed transactions,
  matches receipts and cumulative gas, and derives the retained head root from
  20 final account mutations using 116 calculated trie rows. It neither exposes
  nor persists those rows. Rewards/system-input qualification, writer bootstrap
  and authoritative full-period replay remain open.
- Seven actual Go TraceRunner scenarios compare five tracer configurations at
  both source pins. Prefix state and supplied nonce differ from DryRunner;
  neither-selected options cause a recorded reference panic. Rust trace
  collection/serialization is not implemented by this oracle.

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
was implemented by the lead after worker quota exhaustion. Independent closeout
review of that correction, zero-yield follow-up and final shared integration
remains required. Passing fixtures are not a substitute for that review.

The configured worker agents all reported a usage limit during the next slices.
Their partial work was inspected rather than discarded. No unreviewed worker
output is treated as approval. Luna was approved as Spark's bounded fallback;
an additional helper launch also hit the agent-thread limit. Model assignments
remain recorded in milestone 10; they were not silently reassigned.

## Resume at the coupled dependencies

1. Finish independent review of the final integrated corrections. Implement the
   already specified RETURNDATACOPY override: Go computes/charges memory and copy
   cost before source bounds, unlike the default REVM opcode. Preserve the
   Istanbul table and local Ficus/Cacti overrides; compare combined errors against
   both pinned references before claiming that gap closed.
2. Extend the Rust native session with staged query ownership and delayed reads,
   then compose disposable native simulation and repeated estimation. A hybrid
   query against unchanged FinalChain state would misread earlier native writes.
   Consume a fresh private native session, journal and period sequence together;
   return no publication authority. The last worker created
   `/tmp/rustaxa-evm-native-simulation` at `ddc013cad` but made no implementation.
3. Complete the remaining stateful adapters and reward boundaries through the
   existing Rust kernels. Multi-validator Go map iteration is nondeterministic:
   compare actual permutations and prove final-state commutativity, while keeping
   intermediate retained-node differences explicit. The recovered zero-yield
   patch is separate from that still-unimplemented multi-validator work.
4. Replace full account-snapshot authority for an explicit offline checkpoint
   with authenticated concrete point reads plus a touched overlay. Sparse maps
   must not inherit the existing API's absent-account semantics. On restart,
   persist a checkpoint identity plus qualified deltas, not a sparse map labelled
   as a complete snapshot. Normal production constructors remain untouched.
5. Qualify native history and catalog authority before metadata adoption. The
   current head needs DPoS state for H-5 through H, interval reward rows, cursor
   inputs and corruption/redelegation facts. Traversing live indexed rows cannot
   prove the current catalog's all-ever-tracked/deleted-slot lineage. Define an
   explicit authenticated checkpoint baseline with unknown-slot rejection and
   retention inventory, or obtain the missing historical evidence. Never set
   existing completeness flags from a live-only traversal.
6. Only after those inputs close, implement offline paired adoption/recovery:
   exclusive ownership of both disposable DBs, durable application intent first,
   concrete provenance/catalog batch second, application checkpoint batch last,
   exact idempotent resume and conflicting-marker rejection. This is a design
   direction reviewed for constraints, not an approved adoption implementation.
   The state worktree `/tmp/rustaxa-evm-bootstrap-recovery` remains clean.
7. Add trace facts to the existing frame driver and serializers over those facts,
   then persisted historical API/reopen tests and integrated real-window recovery.
   Request approval for any expensive acceptance campaign using exact commands,
   data and bounds. No acceptance item can be waived because quota is exhausted.

The original `/tmp/snapshot-litenode` remains preserved. The likely producer
commit remains owner-reported, and the exact executable and capture procedure
remain unknown. All copied-state results retain those provenance limits.
