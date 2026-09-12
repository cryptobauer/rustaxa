# EVM/state-db direction decision

**Recommendation: proceed with implementing a Rust execution/state backend using REVM's
interpreter beneath a Rustaxa-owned transaction/frame/host layer. Reuse existing Rust
consensus, FinalChain and native kernels. Retain RocksDB, the separate concrete state
database, and Taraxa's existing physical bytes and commitments for the first migration.**

The architecture feasibility question is resolved sufficiently to authorize an
implementation project. No further engine survey or open-ended research phase is
recommended. This is an engineering recommendation supported by bounded experiments,
not a claim that a complete backend or existing-network cutover is ready. This task
implements neither production routing nor protocol changes. Those remain outside its
authorization.

## Basis and alternatives

Evidence comprises the three original source reports, the
[initial probes](04_feasibility.md), [creation driver](05_creation_frames.md), and
[native/storage/crypto results](06_native_storage_crypto.md). Public and local pinned
Go references agree byte-for-byte over this synthetic corpus; that agreement does not
prove either revision is the binary deployed for every historical network period.

| Option | Decision and reason |
| --- | --- |
| Unmodified REVM framework | Reject for existing-network execution: compiled admission rejects a valid nonce skip; bounded account/transaction nonces and price cannot represent the historical domain |
| Customized REVM framework | Technically possible, but not preferred: replacing admission, account authority, creation, frame rollback, native dispatch and mixed rules removes much of its value. No equivalent full customized-framework prototype or performance comparison is claimed |
| REVM interpreter with Rustaxa host/frame policy | Select for implementation: exact wide nonce CREATE/CREATE2, nested rollback, real Rust native calls, tested mixed gas/opcodes and roots work without REVM source patches |
| evmone/EVMC | Do not pursue initially: assessed CREATE nonce width and state adaptation still need custom handling, while retaining another non-Rust execution boundary offers no demonstrated advantage here |
| Reth or another whole client | Reject for this migration: duplicates already migrated consensus/application/storage ownership and assumes an Ethereum execution/state model |
| Go reference backend | Retain as the parity oracle and recovery/cutover reference until the replacement passes acceptance gates; do not add a silent Rust production fallback |
| New Ethereum trie/account model or unified database | Defer: physical account encoding, wide values, versioned history and paired publication are compatibility requirements. No evidence justifies changing all of them together |

This is a correctness/ownership decision. No throughput improvement, engineering duration,
complete security equivalence or incremental-writer performance has been measured.
REVM's pinned dependency graph and API compose successfully with existing Rust crates;
future upgrades must rerun the corpus and review account-width, CREATE, gas and fork code.

## Intended ownership and integration depth

The existing Rust consensus application and FinalChain remain the protocol orchestrators.
They continue to select periods, rewards/system operations, finalized metadata and
publication. The replacement provides execution and concrete state through the existing
boundary described in [the ownership assessment](03_state_migration_validation.md).
Do not create a parallel FinalChain or move native business logic into an EVM wrapper.

The new Rust execution layer owns:

- Taraxa transaction admission, wide nonce/price authority, charge/nonce ordering,
  simulations, and output/error conversion.
- Frame creation and settlement, value movement, code installation, account lifecycle,
  logs/refunds and distinct ordinary/native/transient state effects.
- Explicit period/configuration-derived opcode, gas and precompile policy. Shared REVM
  instructions run only under this policy; a named Ethereum fork never defines Taraxa.
- Narrow native invocation ports that call existing Rust kernels and expose the exact
  successful state mutations needed by the concrete serializer.

REVM supplies instruction execution, stack, memory and usable frame requests. The
prototype's cloned maps, opcode allowlists, fixed-size balances, fixture admission and
private test overlay are disposable scaffolding. Production needs a journal and narrow
Rust task ports, not those fixture abstractions. Extract domain interfaces only where
needed to avoid a dependency cycle; preserve FinalChain's existing staged ownership.
Any C++ boundary work must use the repository's overlay policy.

The state layer owns raw account/slot/code access, Taraxa codecs, an incremental trie
writer, versioned reads and prepared concrete commits. Reuse existing Rust types and
storage infrastructure. Keep the five-field physical/four-field commitment distinction,
arbitrary-width account nonce and exact raw precompile values. An independent triehash
recalculation remains a test oracle; it is not a hot-path storage design.

## Exact native mutation contract

The native proof settles whether existing business kernels can be reused. It does not
make a normalized native snapshot a database image. The production adapter must retain
original bytes and encoding provenance and emit operation-specific mutations:

1. Read the raw row and relevant native domain state at the current staged generation.
2. Invoke the existing period-correct Rust kernel. Separate historical rule corrections
   where reference behavior differs, including pre-Magnolia pending counts.
3. Serialize only touched rows in the representation required by that operation and fork.
   Preserve untouched historical bytes, iterable ordering, reverse indices and tombstones.
4. Apply ordered writes to the concrete overlay and maintain domain/raw consistency for
   subsequent calls. Do not use projection-equivalence candidate selection as serialization.
5. Settle each effect according to its reference lifetime. Native raw writes and transient
   effects cannot simply share ordinary EVM rollback; new-account removal is a separate
   condition. Preserve observed pre-fix nested-call rejection and static native mutation.
6. Compute storage and account roots from the exact resulting bytes, then compare logs,
   receipts and subsequent-period behavior before publication.

Full DPoS/slashing/reward operation coverage belongs to implementation. The six actual
setCommission cases establish the port and serializer mechanics, including combined
roots, without duplicating business rules. The earlier map/rollback cases establish
additional requirements beyond this one method.

## Database, migration and failure policy

The [architecture overview](architecture_overview.md#data-commitments-and-physical-databases) clarifies the distinction
between permanent logical data responsibilities and optional physical database separation. The initial compatible
layout below is a migration baseline; evaluate consolidation before implementing production persistence. This does
not change the selected REVM integration or authorize a database conversion.

Keep separate application and concrete state databases initially. Their common logical
period does not provide atomic or durable cross-database publication. Implement the
prepared-state identity, recovery reconciliation and persistence ordering described in
[the state design](03_state_migration_validation.md), with these acceptance invariants:

- Readers use one committed generation. Staged execution never leaks into public queries.
- Durable concrete data and its verified identity precede durable application publication.
  Treat uncertain write outcomes explicitly; recovery reconciles markers and roots before
  serving or retrying a period. Never acknowledge a pair based solely on in-memory order.
- Pruning respects every retained/recovery root and is restartable. Interrupted deletion
  or import cannot resurrect tombstoned values or expose a partially imported generation.
- Bootstrap uses an independently verified pair or a defined reconstruction procedure.
  Record network/genesis/config, source revision/build, period/header/root, content hashes
  and retained history. A directory name or a single RPC root is insufficient provenance.
- Rollback uses a verified recoverable pair and a proven reference reopen/catch-up procedure.
  Physical-format compatibility is a target, not permission to alternate writers blindly.

Existing Rust publication infrastructure is the starting point, but neither source review
nor in-memory trie tests prove durability. No storage-format conversion, destructive
pruning, database unification or public snapshot adoption was performed during research.

## Evidence closure and implementation acceptance

The original E1–E12 groups are retained as the implementation/release acceptance matrix.
“Architecture decision complete” does not mean all of these groups have passed.

| Group | Evidence available now | Required before accepting the corresponding implementation |
| --- | --- | --- |
| E1 engine | Compiled width/admission comparison; 16 creation scenarios with exact roots; actual native handoff | Complete frame lifecycle, all call types, depth/value/code boundaries and maintained dependency upgrade guard |
| E2 envelope | 16 dual-reference cases cover width, skips, charging and Cornus distinctions | Implement and compare full envelope, simulation, signed admission, errors, receipts and complete post-state |
| E3 rules | Seven Go observations; six executed mixed-profile comparisons with gas/refunds/state | Full SSTORE matrix, stipend/memory/SELFDESTRUCT, nested transient effects and configured fork combinations |
| E4 native | Raw rollback/lifecycle, six iterable stages and six real native-call cases with exact writes/roots | Every native mutation/system action, overlapping raw/ordinary keys, historical encodings and kernel exceptions |
| E5 state | 14 independent commitment cases; eight physical-node history stages; corruption rejection | Rust incremental/versioned writer, RocksDB reopen, code store, complete account lifecycle and codec fuzzing |
| E6 crypto | 15 historical Falcon cases distinguish compatible 0.3.0 from incompatible 0.4.0 | Complete ABI/malformed/gas/fork matrix for Falcon, P256, BLS and existing precompiles; dependency/security review |
| E7 period replay | Source-derived activation matrix and pinned synthetic references | Provenance-qualified historical and genesis/fork continuations with roots, receipts, header hashes, native state and rewards |
| E8 public APIs | Existing boundary/read-generation requirements mapped | Query/call/estimate/trace/delayed reads at retained and pruned periods, plus appropriate subsystem smoke tests |
| E9 recovery | Publication/reconciliation design and current durability gaps identified | Kill/retry/write-error injection at every prepare/commit/publish step on the implemented backend |
| E10 faults/pruning | Fault and retained-root invariants defined | Power-loss/storage-fault and interrupted import/pruning validation |
| E11 migration | Paired identity/import/rollback protocol specified; bounded acquisition attempted | Verified database pair, compatible import/reopen, reference rollback and catch-up |
| E12 operations | Workload dimensions and baseline procedure specified | Measured sustained performance and resources on matching data/hardware, with agreed numerical budgets |

E7/E11 data acquisition is the current external blocker. Official endpoint attempts from
this environment failed; no historical archive was acquired. Resolve it through a
reachable archive or verified operator export before making an existing-network delivery
commitment. Synthetic fixtures permit implementation to begin, but cannot waive that gate.
Other outstanding rows require the actual backend to test; extending a toy executor to
pretend to pass operational gates would not reduce that work.

## Bounded implementation sequence and stop conditions

Proceed in reviewable feature slices when implementation is authorized:

1. **Concrete read/codec slice:** production-quality codecs and versioned read-only access
   against copied reference data, retaining the existing Rust state boundary. In parallel
   with normal development, secure the provenance-qualified replay dataset. Stop adoption
   of any data whose identity or historical coverage cannot be established.
2. **Execution and mutation slice:** explicit profile/envelope/frame journals plus narrow
   existing-kernel ports and exact native serializers. Grow the checked-in differential
   corpus operation by operation. Stop if a needed behavior forces hidden bounded state,
   silent fallback or an unmaintainable dependency patch; revisit the engine decision with
   that concrete counterexample. No REVM patches are required by current evidence.
3. **Incremental persistence and period slice:** complete writer, prepared commits and
   finalized-period execution through existing Rust ownership. Pass exact E1–E7 behavior,
   including historical native exceptions; an unexplained root/receipt mismatch blocks it.
4. **Operational and cutover slice:** E8–E12, paired bootstrap/rollback, repository storage
   bridge and required differential/smoke gates. Obtain the repository-required approval
   before expensive gates. Set performance budgets from the reference baseline before
   declaring success. Production routing needs its own authorized, validated change.

These are implementation deliverables, not a request for another broad research round.
The project can choose this direction now. If maintaining historical semantics or acquiring
qualified data is unacceptable, the existing-network rewrite should not proceed; a new-chain
protocol design would be a different project requiring explicit authorization.

## Closeout

Executable closeout evidence is committed as `7f8ce5c36`, following creation checkpoint
`33ff2c8e7`; both are on `feat/rust/evm-state-db`.

Research is complete for the direction decision. Twelve isolated Rust tests and the real
native-kernel overlay test pass against reproducible dual-reference artifacts. Isolated
formatting/strict Clippy, overlay validation and the repository fast gate pass. Overlay
Clippy reports 86 existing production-test warnings; its new probe denies warnings.
No full-node, expensive differential or power-loss gate has been run, and no production
backend is claimed. All experiments and documentation remain isolated on the feature branch;
an additional experimental branch is unnecessary because they cannot affect node routing.
