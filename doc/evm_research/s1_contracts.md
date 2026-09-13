# S1 execution and state contracts

Integration base: `ec5f37b41`. Implementation stays on `feat/rust/evm-state-db`;
task worktrees start on `task/evm-s0` and `task/evm-execution`. No production
dependency or routing to `rustaxa-evm` is installed. General application and
concrete RocksDB layout remain unchanged.

Status: S1 contracts and unlinked preflight composition complete, independently
reviewed and validated. S2/S3 implementation can proceed under these interfaces;
this is not execution, persistence or production-routing acceptance.

## Ownership and dependency direction

| Path | Writer | Responsibility |
| --- | --- | --- |
| `rust/crates/rustaxa-types/src/concrete_state.rs` | Lead | Shared concrete domain and read ports |
| `rust/crates/rustaxa-evm/src/contracts.rs` | Lead after execution-worker draft review | Execution, block-hash and native invocation contracts |
| `rust/crates/rustaxa-evm/src/{envelope,profile,journal,host,frame,result,types}.rs` | Execution worker | Taraxa mechanics over REVM interpreter components |
| `rust/crates/rustaxa-storage/src/concrete_state/` | State worker | Compatible codecs, qualified versioned readers, later prepared writer |
| `rust/crates/rustaxa-evm/tests/contracts.rs` and later period composition tests | Lead | Test-only composition using existing Rust owners |
| Existing consensus native kernel/composition modules | Lead | Staged native adapters; no duplicated business kernels |
| Snapshot-specific tooling/evidence | State worker | S0 qualification on independent copy |
| Workspace/crate manifests, lockfiles, `PLAN.md`, shared module declarations | Lead | Integration |

Storage and execution both use `rustaxa-types`; storage never depends on the
execution crate. Consensus remains dependent on existing storage and types.
The new execution crate uses consensus/storage only as development dependencies
for isolated tests. Existing `ConsensusExecutionPort`, its
`FinalChainExecutionLeaf` implementation, FinalChain sessions, and
`FinalChainExternalEvmStateCommitIntent` remain the composition and publication
boundaries. No parallel application manager or commit identity is introduced.

## Reader and value invariants

`ConcreteStateRead` is fixed to one `ConcreteStateIdentity` (period/root).
Constructing the descriptor is not evidence of retention or authorization to
adopt a database. Reader construction must prove coverage; a missing version
without coverage is `HistoryUnavailable`, not an empty account. Confirmed
pruning, selected tombstones, absence, malformed records, future periods,
identity mismatch and I/O failures are distinct outcomes.

Accounts retain `FinalChainNonce`, arbitrary-width unsigned persisted balance,
optional storage/code hashes and reference u64 code size. A separate record
retains the exact five-field physical RLP. The four-field commitment encoder
normalizes absent hashes but is never used to rewrite untouched physical rows.
The existing native `FinalChainAccountBalance` is bounded and must not become
the authoritative general concrete balance. Journal intermediates require
signed arithmetic: Go's zero-address affordability exemption can subtract a
fee beyond the sender's balance. Conversion to persisted unsigned state and
bounded native kernels must be explicit and reference-tested.

Addresses and `ConcreteStorageKey` enter unhashed. Storage follows the Go
mapping: account prefix `keccak(address)`, slot prefix
`keccak(address || keccak(slot))`, version suffix u64 big endian. Raw slot values
are arbitrary bytes, including more than an EVM word. `storage()` exposes
physical history; the executor gates semantic access by account lifecycle.
Code rows are unversioned and hash-addressed. Account reachability at the pinned
period governs code availability; missing referenced code or a size/hash
mismatch cannot turn into an empty program.

Source: pinned Go `taraxa/state/state_db/{main_trie,account_trie,block_reader}.go`
and `state_db_rocksdb/db.go`, reviewed independently against the Rust contract.

## Journal and native settlement invariants

- Ordinary balance, nonce, storage and logs follow reference frame undo.
  Envelope effects outside the frame survive its undo. Store original,
  current and new slot values separately for gas/refund evaluation.
- Native raw writes and ordinary writes have distinct same-transaction
  visibility. Ordinary reads ignore dirty native raw writes, raw reads ignore
  ordinary dirty writes. Flush applies ordinary writes then native raw writes;
  the raw lane wins overlapping keys.
- Raw writes survive frame undo only while the account survives. Reverting
  creation of a new account removes its body and its raw writes. Empty-account
  deletion and SELFDESTRUCT require reference-specific treatment.
- Transient writes survive frame undo under historical rules and clear at
  transaction completion. Do not import Ethereum transient rollback policy.
- Native serialization emits only touched rows and ordered iterable mutations.
  Unchanged bytes and semantic/raw consistency survive subsequent calls.
  Projection-equivalence candidates are not serializers.
- Frame settlement owns value transfer, gas, code deposit, nonce and errors
  exactly once. Native adapters must separate ordinary balance/log outcomes
  from irreversible raw/semantic changes and must not charge gas or transfer
  call value a second time.

The typed native port receives a current-journal read view and separates mutable
reference cache preparation from gas-admitted business invocation. Preparation
must bind every invocation field; matching the invocation ID alone is insufficient.
Native ordinary effects include ordered balance, nonce and touch operations;
implicit account creation must precede a dependent nonce effect. Native failure
payloads and attempted creation addresses survive the exact reference error path.
Empty raw puts are rejected because the reference represents them as deletion.
Native cache lifetime remains distinct from journal rollback, including caches
that survive removal of a newly created account.

Negative intermediate balance word projection follows the pinned uint256
conversion modulo 2^256. A negative terminal balance is explicitly unsupported
at the unsigned persistence boundary: Go's physical account encoder ignores an
RLP negative-integer error and can produce malformed output. Rejecting that
boundary is not claimed as terminal Go parity; S3/S4 must prove the relevant
reachability/settlement before claiming complete execution compatibility.

Source: pinned Go `state_evm/{account,transition_state}.go`,
`state_transition/trie_sink.go` and `core/vm/evm.go`;
dual-reference raw rollback/native/creation fixtures under
`experiments/evm_feasibility/fixtures`. Those fixtures specify behavior but do
not yet validate the new general journal/frame implementation.

## Persistence and visibility

The existing FinalChain intent identifies the prior/new period and roots,
request, publication plan, block hash and exact marker/provenance bytes.
Preparing concrete state never authorizes application publication. Concrete
data and its descriptor/provenance must be durable before FinalChain publishes
the application batch and advances query visibility. An uncertain write outcome
requires reconciliation of the exact durable intent and observed roots before
retry; it is not equivalent to a rejected write. Query readers pin the published
generation while execution mutates a private pending generation.

S0 may discover an upstream markerless pair. Root equality alone does not
manufacture Rustaxa lifecycle authorization. S1's test composition deliberately
returns an unqualified-bootstrap report even for equal descriptors. S7 must
define and test paired bootstrap/recovery before such imports are adopted.

## Validation and limits

Use Tier 1 for these unlinked contracts plus the focused `rustaxa-evm` contract
test. The test borrows real existing `Storage` and `FinalChain`, exercises the
existing execution-port/leaf composition, rejects skewed period/root, refuses
unqualified adoption/commit and verifies the published head is unchanged.
Its descriptor-only reader rejects row access. It is not S2 read parity, S4
persisted execution, or a startup/recovery claim.

S2 requires synthetic and S0-qualified real physical comparisons plus storage
bridge tests. S3 requires dual-reference envelope/journal/frame evidence. S4
must demonstrate complete persisted period continuation before broad opcode
and native expansion. E1–E12 remain the acceptance matrix; contracts alone
close no execution, historical replay or operational acceptance group.

Independent Astra review approved the shared read signatures with the explicit
code hash/reachability clarifications now included. Spark was attempted through
fixed supported roles for bounded tooling/maps; service usage limits prevented
execution until 2026-09-13 04:44 UTC. Assigned Sol workers performed the work;
no substitute is reported as Spark.

Executed for the initial shared-contract chunk: `cargo test --manifest-path
rust/Cargo.toml -p rustaxa-evm --test contracts` and `make rewrite-validate-fast`
both pass. The final focused test also checks chain/config mismatch and exact
refusal/request identity. Existing workspace Clippy warnings and the inventory
guard's missing retired shim-directory diagnostic remain; both structural
guards pass. No storage implementation or C++ changed in this chunk, so no
storage bridge build, expensive differential, or upstream C++ audit is claimed.

Final S1 validation also passes `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-evm` (four contract tests and the composition test) and a fresh
`make rewrite-validate-fast` after linking the reviewed contracts into the new
crate. Independent source-to-contract review resolved eight initial findings
before S3 was released. The [additive journal oracle](journal_contract_evidence.md)
provides dual-reference rollback/physical-byte inputs for S3; it does not count
as a passing Rust journal implementation.
