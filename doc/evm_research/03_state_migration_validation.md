# State ownership, migration, and validation design

> Source assessment at the pinned baseline. Subsequent executable evidence and the resolved
> architecture recommendation are recorded in the [direction decision](07_direction_decision.md).

## Recommendation and confidence

Modernize the execution engine and state backend behind the existing Rust application boundary. Keep consensus,
FinalChain publication, rewards planning, and native contract domain logic in their existing Rust ownership. Preserve
the concrete database format initially where feasible, and treat a later physical format conversion as a separately
validated storage change. Existing-network compatibility includes both historical roots and future valid transactions
under existing rules; matching a few observed blocks is insufficient.

The source assessment supports this direction, but does not establish that a particular REVM integration is viable.
The [engine assessment](02_engines.md) leaves two integration depths open. The decisive experiments concern integer
widths, frame semantics, native storage visibility, exact commitments, and crash recovery. This document specifies
those experiments and migration acceptance criteria; it does not report their execution.

## Ownership and dependency boundaries

**Established:** `ConsensusExecutionPort` already separates concrete execution operations from Rust application
sequencing. Its operations include prior-state loading, system facts, transaction execution, rewards execution,
commit, discard, pillar reads, and DAG gas estimation. The private C++ `ExternalEvmStateOwner` serializes the Go
StateAPI and supplies execution/query leaves. Replacing it does not require recreating a FinalChain manager.[^1]

**Proposed:** use the following responsibility split. Crate names are illustrative, not new implementation commitments.

| Owner | Responsibility | Boundary invariant |
| --- | --- | --- |
| Existing `rustaxa-consensus` / FinalChain | Ordered period plan, native domain kernels, rewards plan, semantic validation, commit authorization and publication | Engine cannot select DAG ordering, publish a head, or independently authorize a period |
| Execution domain / `rustaxa-evm` | Taraxa transaction envelope, fork profile, interpreter integration, frames, checkpoints, native dispatch and execution results | Exact wide values and Taraxa rollback rules; no dependency on the application manager |
| State infrastructure / existing storage or focused state crate | Concrete account/slot/code access, versioned reads, trie persistence, state descriptor, provenance and staging | One authoritative concrete root; explicit pending versus committed views |
| Composition adapter | Implements execution and native-contract ports using existing Rust handles and staged domain state | No dependency cycle or reentry into the application lock |
| Existing bridge/shim surfaces | Temporary routing for remaining C++ callers and public query adapters | No second protocol implementation or silent production fallback |

The engine should define narrow task-oriented ports for native calls and state access. The composition adapter can
invoke existing Rust DPoS/slashing kernels. If their current location creates a dependency cycle, extract a small
shared domain unit or expose a staged kernel API; do not make the engine depend on the whole consensus application.
Keep hot paths statically dispatched where practical, canonical input bytes available, and bridge payloads plain.

**Established limitation:** `apply_dpos_mutation_transaction` is currently a private FinalChain method operating on
mutable snapshots and account maps. Existing code therefore supplies valuable domain behavior, not a ready-made
engine precompile adapter. The future adapter must define who handles call value, account nonce, gas, logs, failed
calls and persistence, so those effects occur exactly once.[^2]

## Native state cannot be regenerated from snapshots alone

**Established:** `canonical_concrete_precompile_storage` builds a map of candidate byte values for each raw key.
For example, an untouched validator may retain its older representation when its extended count is zero. The
validator permits matching alternatives. Its documentation also explicitly excludes removed historical iterable-map
prefixes whose preimages are absent from the compact native snapshot; the prior concrete root preserves those bytes.
`concrete_storage_values_equivalent` accepts overlapping candidates and treats some empty values as absent.[^2]

**Inference:** this is a semantic compatibility checker, not a unique byte-exact serializer for reconstructing the
whole state database. Selecting an arbitrary candidate or rebuilding all live rows at every fork could change a root
without changing the compact DPoS snapshot. Preserve unchanged raw bytes from the concrete prior state. For changed
rows, derive the exact writes and deletions made by the relevant native operation and fork, including insertion order,
historical counters and retained rows. Reuse the existing Rust mutation kernels, adding a canonical storage mutation
layer rather than duplicating their domain decisions.

**Proposed acceptance criterion:** each native operation produces an ordered, typed mutation transcript with exact
raw bytes and rollback disposition. The state backend applies that transcript against the paired prior concrete
state. Validate it independently against reference execution and existing semantic projection checks. The new writer
must not validate its own serialization solely by regenerating expected output with the same writer.

## State codecs and physical layout

The initial compatibility reader must distinguish database encoding from commitment encoding. An Ethereum account
convenience type is insufficient: Taraxa stores an arbitrary-width nonce and a code-size field, while the root hashes
a normalized four-field account. Generic raw-leaf trie components remain candidates, as assessed in phase 2.[^3]

| Item | Established encoding or behavior | Migration requirement |
| --- | --- | --- |
| Account database value | RLP of nonce, balance, optional storage root, optional code hash, code size | Decode and preserve arbitrary-width nonce and absent-field representation |
| Account commitment value | Four-field RLP; absent roots/code hashes replaced by their empty hashes | Exclude code size from the commitment; preserve exact normalization |
| Ordinary storage | Nonzero integer bytes; zero removes the leaf; hash encoding wraps stored bytes in an RLP string | Do not confuse stored bytes with the encoded trie value |
| Native raw storage | Arbitrary bytes; empty removes the leaf; same string wrapping for commitment | No universal U256 decoding or normalization |
| Main trie key | Keccak(address) | Hash address exactly once |
| Account trie key | Keccak(32-byte slot), including a raw native key | Preserve slot padding and hash stage |
| Versioned storage lookup key | Keccak(address concatenated with the account trie key), followed by big-endian u64 period | Do not use the plain trie key as the database key |
| Trie node reference | Non-root RLP shorter than 32 bytes is embedded; otherwise hashed; root is hashed | Exercise 31/32-byte boundaries and root special case |
| Empty root | Keccak of the RLP empty byte string, despite the `EmptyRLPListHash` name | Copy the definition, not an interpretation of its name |

These encodings are defined by account/slot schemas, TrieSink, trie hashing and the versioned key implementation.[^3][^4]
The inspected column-family numbering is also part of an initial compatible reader/writer.[^5]

| RocksDB column-family name | Content |
| --- | --- |
| `default` | State descriptor and concrete lifecycle metadata |
| `1` | Code by code hash |
| `2` | Main trie nodes |
| `3` | Versioned main trie values |
| `4` | Account trie nodes |
| `5` | Versioned account trie values |
| `6` | Most recent main trie values |
| `7` | Most recent account trie values |
| `8` | DPoS configuration changes |

A historical read uses `SeekForPrev` on the hash-plus-period key and verifies the hash prefix. An empty value at that
version means absence; searching farther back would resurrect deleted state. Latest-value columns are a separate
lookup optimization whose consistency must be preserved. A missing required trie node or database read error must
not be silently converted into an empty account or zero slot.[^5]

Retain custom persisted node decoding until a replacement is verified. A correct Ethereum MPT root algorithm does
not imply that it can directly read Taraxa's persisted node representation. Root equivalence and physical database
compatibility are separate gates.

## State visibility and rollback

**Established:** ordinary `GetState` checks `StorageDirty`, while `GetRawState` checks `RawStorageDirty`; each then uses
its respective committed read path. The ordinary getter does not simply read the raw dirty map. TrieSink applies
ordinary writes before raw writes within an account update. Consequently, a single immediately merged slot map could
change intra-transaction visibility or collision precedence, even if the eventual physical key is shared.[^6]

Native irreversible writes increment the account modification count without registering an undo callback. Ordinary
account changes have undo callbacks, and transient storage has separate transaction lifecycle behavior described in
phase 1. Account creation, deletion, cleanup and outer-frame rollback may interact with those rules; do not generalize
one setter into a claim that every native effect always survives every failure.[^6]

**Proposed:** specify distinct views and checkpoints before selecting a journal implementation:

- The committed historical reader is pinned to the period/root required by the caller.
- A period overlay accumulates completed transaction effects and system/reward effects in the prescribed order.
- A transaction maintains ordinary dirty/original values, raw dirty values, transient values, logs and account lifecycle.
- A frame checkpoint records which categories roll back, remain visible, or survive to the transaction checkpoint.
- Finalization converts these views into exact account and storage bytes, then commits their root together with metadata.

Map each operation to the legacy getter/setter path and checkpoint boundary. Include ordinary/raw writes to the same
key, inner and outer reverts, static/delegate calls, creation and destruction, repeated transactions, and native calls
on both existing and newly created accounts. Preserve observed legacy behavior initially; any intentional correction
belongs to a later protocol change with its own activation rule.

## Commit protocol and recovery

**Established:** Rust `db/` and Go `state_db/` are separate databases. Go's state batch binds the state descriptor,
approved concrete provenance, storage catalog and pending-marker removal. Staging checks database identity,
generation, period and prior commitment; an identical retry is allowed while conflicting staging is rejected.
Rust separately persists a pending-publication marker and later publishes its FinalChain rows.[^7][^8]

The existing design is a useful protocol to preserve, but source-level durability needs careful qualification.
Rust pairing and pending-publication writes explicitly request synchronous writes. The inspected Go concrete writes
use default write options, and the Rust final publication calls the default `commit_batch`. RocksDB distinguishes
atomic batches from synchronous persistence: default writes can return before persistent storage has received them.
A process-restart check alone therefore does not establish a power-loss guarantee.[^7][^8][^9]

**Inference:** audit ordering across both databases under the actual filesystem, RocksDB versions and write options.
There is no source evidence here that an observed production failure occurred. The finding is an unresolved
cross-database durability obligation, not proof that every current recovery scenario is broken.

**Proposed future protocol:** retain existing plan/provenance identities and authorization, with explicit durability
boundaries. Implementation must reconcile the following sequence with the existing recovery state machine:

1. Verify the paired database identities, chain identity, generation, prior root and period. Pin the prior state.
2. Stage the exact execution intent before mutation, using the required durability policy.
3. Execute transactions, native changes and rewards in private staged views. Produce the final root and transcript.
4. Have Rust validate the report and durably persist the pending publication/authorization before concrete commit.
5. Atomically commit concrete state, descriptor, provenance, catalog and staging removal; ensure required state
   durability before allowing a later publication to become durable.
6. Publish the corresponding Rust FinalChain rows and clear pending publication in one atomic batch, with an explicit
   durability policy for successful acknowledgment. Expose the new head only after publication succeeds.

A synchronous Rust marker does not itself synchronize the other database. The exact placement of syncs and any safe
batching optimization must be justified by failure tests and measured cost, rather than added or removed by analogy.
An I/O error after an attempted write can have an ambiguous outcome: reload and reconcile durable identities instead
of assuming failure means nothing was written.

The following is a proposed recovery decision table, not a claim that every row is already implemented:

| Durable state after restart | Required outcome |
| --- | --- |
| Both databases agree at prior generation, no pending work | Resume normally |
| Exact staged intent exists; concrete state remains at prior generation | Reconcile with Rust pending state, then deterministically replay or discard through the authorized recovery path |
| Concrete state advanced once; matching Rust pending publication exists | Verify root, provenance and authorized report before completing publication exactly once |
| Both databases agree at new generation | Treat retries idempotently; clean only metadata proven redundant |
| Rust published head is ahead of available concrete state | Stop normal service and require defined recovery; never invent a matching root or silently accept empty state |
| Concrete state ahead without matching authorization, conflicting identities, or more than one unexplained generation | Reject automatic adoption and retain evidence for diagnosis |
| Corrupt/missing catalog, trie node or incompatible metadata version | Return a typed recovery error; no silent fallback to Go or a newly initialized database |

Recovery must finish before startup exposes a usable head, consensus resumes, or RPC reads a mismatched pair. Public
receipts, state queries and pillar inputs must agree on the same committed generation. Background readers must not
accidentally observe the execution overlay.

## Migration, snapshot import, and rollback

**Preferred first operational path:** work from a quiesced, verified copy of an already paired Rustaxa database set.
Keep the original pair untouched. Record chain/config identity, source revisions, both heads, root, provenance,
generation, catalog, database options, file checksums and retained-history range. A RocksDB checkpoint supplies a
consistent copy of one database; separate checkpoints still need an application-level common boundary.[^10]

| Migration option | Advantage | Remaining proof / reason to defer |
| --- | --- | --- |
| Rust reads and writes existing concrete format on a copied pair | Smallest simultaneous semantic and format change | Exact node/value codecs, column families, markers, options and cross-version RocksDB behavior |
| Offline conversion to a new physical state layout | Can simplify Rust persistence and later maintenance | Full retained-root equivalence, history/tombstones, resumable import, provenance transition and independent verification |
| Replay from genesis | Strong cumulative execution evidence and fresh storage | Requires complete ordered inputs and native/system facts for all periods; archive availability and cost are unresolved |
| Import public Taraxa snapshot | Potentially practical bootstrap source | Public snapshot alone does not establish a compatible Rust native snapshot, historical reward graph, catalog or paired provenance |

Go concrete activation rejects markerless non-genesis state under the inspected policy. A public snapshot import
therefore requires an explicit, independently checked bootstrap/conversion design; deleting markers or relabeling a
database is not a migration. Rust compact snapshots do not contain every arbitrary contract leaf or historical native
row, so they cannot replace the concrete database.[^2][^7]

Taraxa's public documentation advertises light snapshots and says full snapshots were discontinued. No full historical
corpus or verified snapshot artifact is available in this assessment. Thus neither full replay nor arbitrary historical
RPC coverage is currently an executable acceptance gate; acquiring the required data is a named prerequisite.[^11]

Keep the RocksDB dependency/format choice conservative during first adoption. RocksDB documents backward compatibility
with qualifications and limited forward compatibility. Verify the exact versions, codecs and options used by each
reader before opening copied data; successful Rust writes do not prove that the old binary can reopen that directory.
Rollback should initially restore the retained database pair and reference binary, then catch up, rather than reopen
mutated data with an older engine.[^12]

Cutover must occur at a committed period boundary with no pending execution or pruning work. Identify exactly which
binary owns writes and stop the other writer. A restored pair may be behind the network; recovery/catch-up is part of
the rollback procedure. There is no proposed automatic per-transaction fallback or dual production writer.

## Pruning and historical reads

**Established:** the current owner refuses pruning while concrete execution is staged and supplies a sequence of roots
to retain. The Go backend prunes versioned values, reconstructs the main-node column by dropping/recreating it, and
retains account nodes reachable from retained roots. This is more than a RocksDB compaction setting.[^1][^5]

**Proposed:** preserve the externally required history horizon and all native delayed-state dependencies. Pin roots
needed by active readers, recovery, rewards, DPoS, pillar construction and any supported historical query. An offline
migration must retain the last applicable value before the cutoff as well as later versions; tombstones must continue
to prevent older values reappearing.

Design pruning as recoverable maintenance with explicit progress/epoch metadata or a verified replacement generation.
Do not mechanically port destructive column recreation while assuming crash safety. Test interruption during marking,
value deletion, node replacement and metadata publication. Verify every retained root and declared query horizon after
restart; reject unsupported historical reads consistently with the public contract.

## Evidence corpus and acceptance criteria

The following experiment inventory is **proposed future engineering work**. It is deliberately separate from this
source assessment. Existing fixtures are useful inputs, but disabled tests, shared encoders and native-only semantic
checks cannot independently establish concrete execution parity.[^13]

| ID | Experiment and discriminating cases | Required evidence |
| --- | --- | --- |
| E1 | Compare REVM framework and interpreter-host routes: nonce above u64, nonce successor above U256, wide gas price, nested CREATE/CREATE2 and collision/revert | Exact values remain authoritative throughout; documented local replacement surface and compatible dependency graph |
| E2 | Envelope ordering: affordability boundary, nonce skipping/too-low, intrinsic failure, zero sender, call/create, simulation | Reference gas, charged fee, nonce, return/error classification, account changes and root |
| E3 | Opcode/gas profile at every relevant fork: SSTORE original/current/new, CALL stipends, memory, refunds, SELFDESTRUCT, aliases, transient rollback | Instruction and transaction results match the Taraxa profile; standard Ethereum tests supplement only shared semantics |
| E4 | Native calls within nested frames, ordinary/raw overlapping keys, old and new account lifecycle, historical validator encodings | Exact raw mutations, surviving/reverted effects, logs, gas, native snapshots and root |
| E5 | Account/slot/node codecs: huge nonce, absent roots, zero/tombstone, short/long RLP and embedded-node boundary | Byte-exact database fixtures and independent commitment results |
| E6 | Precompiles at fork boundaries: BLS table remapping, malformed points, P256, Falcon selector/encoding/failure cases | Output bytes, gas and failure behavior against the pinned Go reference; no substitution based only on algorithm name |
| E7 | Genesis plus complete periods around each activation, including empty/native-only periods, system changes and rewards | Root, receipt bytes/root, logs/bloom, gas, finalized header/hash, native projection and subsequent-period behavior |
| E8 | State read API, call/estimateGas, DAG estimation, traces, pillar/rewards delayed reads at retained and pruned periods | Existing public behavior and committed-generation isolation |
| E9 | Process termination at every staging/commit/publication boundary, retries and injected write errors | Exactly-once recovery, no mismatched published pair and explicit ambiguous-write handling |
| E10 | Storage fault/power-loss simulation and interrupted pruning/import | Persistence ordering, complete retained roots, no silent data resurrection or unsupported adoption |
| E11 | Paired copy open/import/reopen, reference rollback and catch-up | Verified identities, checksums, retained history and exact roots before and after recovery |
| E12 | Sustained sync/finalization/query workload on matching datasets | Latency distribution, throughput, memory, disk amplification, sync cost and correctness maintained under load |

Each fixture needs a versioned manifest: reference source/binary identity, chain/genesis/config hash, fork flags, prior
period/root and complete required state, canonical transaction bytes and order, environment fields, system/reward facts,
expected outputs, provenance and generation. Hash the fixture files. Distinguish intentionally invalid protocol inputs
from corrupted fixture/infrastructure failures. If an engine fails to load state, the harness must fail rather than
compare two empty outcomes.

Capture both the public-release EVM behavior and local concrete-lifecycle reference where relevant. Phase 1 identifies
local behavioral changes; agreement with the local fork alone cannot resolve whether a difference from the public
reference is authorized for existing networks. Categorize every mismatch, minimize it, and preserve the regression case.
Never normalize away a root, gas, receipt, raw-byte or failure-class difference to obtain a pass.

At each activation use the preceding period, activation period, following period and a multi-period continuation.
Include combined and out-of-order configured forks on test/dev profiles, zero activations and disabled sentinels.
Synthetic cases cover valid but historically unobserved wide values; historical replay establishes cumulative behavior
on real data. Neither substitutes for the other.

Start with narrow package and bridge checks for implemented slices, then follow the repository's subsystem and
production-authority tiers. Every storage module change includes the Rust storage bridge gate. Production routing
requires differential parity and an appropriate Rust-enabled startup/sync/finalization/RPC smoke test. Expensive
repo-wide and storage differential runs remain subject to the repository's separate authorization rule.[^14]

## Performance and operational decision gates

No throughput, memory, database-size or engineering-duration estimate is supported by this source assessment.
Collect a reference baseline before setting numerical regression budgets. Separate execution, native calls, hashing,
RocksDB reads/writes, synchronous persistence, serialization and bridge overhead; otherwise a faster interpreter may
hide a slower overall period pipeline.

Measure representative empty, native-heavy, contract-heavy, storage-heavy and mixed periods on fixed hardware and
datasets, with warm/cold cache conditions identified. Include tail latency, peak resident memory, staging size, disk
space during import, pruning pauses, restart time and sustained catch-up. Compare roots and public results throughout.
Do not commit to parallel transaction execution in the initial migration: shared native state and rollback behavior
make serial parity the prerequisite for a separate concurrency design.

## Decision ledger and next work

| Decision | Current conclusion | Condition to revisit |
| --- | --- | --- |
| Compatibility | Preserve existing-network behavior first | Only an independently agreed protocol upgrade changes rules |
| Consensus ownership | Keep current Rust application/FinalChain and reuse native kernels | Extract narrow domain APIs when needed to avoid cycles |
| Engine | Prefer REVM components; framework versus interpreter-host remains open | E1–E4 demonstrate a bounded maintainable integration, or reveal a blocker |
| Whole-client adoption | Do not adopt Reth wholesale for this migration | A concrete benefit must outweigh duplicated client/state ownership |
| State commitment | Preserve Taraxa bytes and roots; assess raw trie components | E5 and E7 establish exact commitment equivalence |
| Physical persistence | Prefer existing format and separate databases first | E9–E11 establish a safer or materially simpler alternative |
| Native serializer | Add exact mutation serialization around existing domain logic | E4 proves historical bytes and rollback; projection candidates alone are insufficient |
| Crypto dependencies | Select by exact historical encoding and behavior | E6 closes algorithm/version compatibility, especially Falcon |
| Deployment | No production switch justified yet | Required parity, recovery, migration and operational gates pass |
| Ethereum feature upgrades | Defer semantic upgrades beyond this migration | Separate specification, activation and ecosystem validation |

The next bounded implementation proposal should cover E1–E5 and fixture acquisition first, with a stop/go review before
building a complete backend. Its deliverables are a reproducible reference corpus, a comparison of the two REVM
integration depths, a canonical native mutation design, and one independently verified state commitment path. It must
list any unavoidable upstream library patches and how they will be maintained. Crypto feasibility can then be evaluated
against the same corpus before broad period replay and operational migration work.

The source research is complete enough to select this direction and define the remaining evidence. It does not support
a final engine integration choice, an effort estimate, guaranteed archive availability, or a claim of network parity.
Those are explicit outcomes of the next experimental phase.

## Sources

Repository source links refer to the revisions recorded in [baseline.json](baseline.json). Numbered notes identify the
functions and files underlying established findings. External documentation was accessed on 2026-09-12; it describes
library contracts, not measured behavior of this deployment. Future design and experiment requirements are analytical
recommendations derived from these sources and the first two assessments.

[^1]: [ConsensusExecutionPort](../../rust/crates/rustaxa-consensus/src/consensus_application_runtime.rs), [private concrete owner](../../libraries/core_libs/consensus/src/application/external_evm_state_owner.cpp), including `prune` and query/execution methods.
[^2]: [Rust FinalChain](../../rust/crates/rustaxa-consensus/src/final_chain.rs), `apply_dpos_mutation_transaction`, `external_evm_concrete_projection`, `canonical_concrete_precompile_storage`, `insert_concrete_storage_value`, and `concrete_storage_values_equivalent`.
[^3]: [Account storage and commitment codec](../../submodules/taraxa-evm/taraxa/state/state_db/main_trie.go), [storage codec and database key](../../submodules/taraxa-evm/taraxa/state/state_db/account_trie.go), [TrieSink](../../submodules/taraxa-evm/taraxa/state/state_transition/trie_sink.go).
[^4]: [Trie hash encoder](../../submodules/taraxa-evm/taraxa/trie/hash_encoder.go), [state constants](../../submodules/taraxa-evm/taraxa/state/state_common/index.go), [versioned key](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/misc.go).
[^5]: [Go state database](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/db.go), column-family initialization, `block_state_reader.Get`, `Prune` and helpers; [column identifiers](../../submodules/taraxa-evm/taraxa/state/state_db/db.go); [latest-state views](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/latest_state.go).
[^6]: [Account views and mutations](../../submodules/taraxa-evm/taraxa/state/state_evm/account.go), [transition checkpoints](../../submodules/taraxa-evm/taraxa/state/state_evm/transition_state.go), [TrieSink.Update](../../submodules/taraxa-evm/taraxa/state/state_transition/trie_sink.go).
[^7]: [Concrete state provenance/staging](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/concrete_state.go), [concrete commit](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/latest_state.go), [concrete metadata types](../../submodules/taraxa-evm/taraxa/state/state_db/concrete_state.go).
[^8]: [Rust publication and marker batches](../../rust/crates/rustaxa-storage/src/final_chain.rs), [RocksDB write options](../../rust/crates/rustaxa-storage/src/db.rs), [application recovery](../../rust/crates/rustaxa-consensus/src/final_chain.rs).
[^9]: RocksDB project, [Basic Operations: Atomic Updates and Synchronous Writes](https://github.com/facebook/rocksdb/wiki/Basic-Operations), documentation accessed 2026-09-12.
[^10]: RocksDB project, [Checkpoints](https://github.com/facebook/rocksdb/wiki/Checkpoints), documentation accessed 2026-09-12; Taraxa `DB.Snapshot` in the [Go backend](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/db.go).
[^11]: Taraxa Project, [Syncing From Snapshot](https://taraxa.gitbook.io/taraxa-network/node-setup/syncing-from-snapshot), accessed 2026-09-12. See [reference evidence limits](01_compatibility.md).
[^12]: RocksDB project, [RocksDB Compatibility Between Different Releases](https://github.com/facebook/rocksdb/wiki/RocksDB-Compatibility-Between-Different-Releases), documentation accessed 2026-09-12.
[^13]: [FinalChain tests](../../tests/final_chain_test.cpp), [StateAPI tests](../../tests/state_api_test.cpp), [Rust FinalChain tests and projection helpers](../../rust/crates/rustaxa-consensus/src/final_chain.rs); [phase 1 corpus assessment](01_compatibility.md).
[^14]: [Rewrite validation strategy](../rewrite_validation_strategy.md), tiers 1–3 and correctness rules; repository [AGENTS.md](../../AGENTS.md), storage validation and production routing requirements.
