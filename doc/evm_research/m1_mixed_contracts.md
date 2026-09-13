# M1 mixed-period implementation handoff

Status: in progress from `606d89c12`; the milestone is not complete. This file records
actual ownership, initial workload and contracts under review. It accompanies
[the M1–M6 milestone](09_mixed_period_milestone.md).

## Work ownership

| Owner/model | Worktree/branch | Owned work |
| --- | --- | --- |
| Lead / Astra | root `feat/rust/evm-state-db` | Shared admission rules, FinalChain validator/API, invocation-context contract, composed period adapter, documentation and integration |
| Execution / Sol | `/tmp/rustaxa-mixed-native`, `task/evm-mixed-native` | Staged native/rewards and selected custody adapters; coordinate parent FinalChain edits with lead |
| State / Sol | `/tmp/rustaxa-mixed-genesis`, `task/evm-mixed-genesis` | New `final_chain/mixed_genesis_tests.rs`: independent Rust genesis and reward-planner witness |
| Oracle / Sol | `/tmp/rustaxa-mixed-oracle`, `task/evm-mixed-oracle` | New `mixed_period_reference*` exporters, typed per-pin adapters and `mixed_*` fixtures |
| Helper / Terra | `/tmp/rustaxa-mixed-ordering`, `task/evm-mixed-ordering` | Projection decoder association/sequence validation and focused regressions |
| Reviewer / Astra | read-only across worktrees | Independent reference, contract and integration review |

All task worktrees start at `606d89c12`. Earlier isolated drafts have already been
recovered into the feature branch; they are preserved and must not be resumed or
reapplied. Sol is available in this continuation. Spark completed a bounded source map,
then hit its reported quota; the mechanical ordering task was explicitly reassigned to
Terra. The lead alone owns `/build` and integration commits.

## Initial reference witness

The first witness uses actual Go `StateTransition.Init` and incremental TrieSink state,
not reconstructed native maps. The proposed inputs are chain 841; key `01` repeated
32 times for the funded sender/validator owner; validator `000…0031`; delegator
`000…0032`; allocations 1,000,000 and 2,000; genesis delegation 1,000; commission zero;
threshold 100, vote step 10, maximum stake 1,000,000 and minimum deposit 1.

Delegation delay and all base/Cornus/Cacti locking periods are one. Blocks per year is
one, yield is 20 percent, author reward 10 percent and DAG reward 50 percent. Magnolia,
Aspen part one, Ficus and Cornus activate at genesis; Aspen part two and Cacti are beyond
the fixture. The manifest must carry remaining effective defaults and maximum supply.
The synthetic finalized DAG/certificate facts use committee size one and derive vote
maximum from actual eligibility; arbitrary addresses are not claimed as signature-verified
PBFT authors. Transaction inputs themselves are signed and independently decoded.

Period one initially contains a signed nonce-zero transfer of seven with gas price one.
Actual execution fees feed the real rewards path. The source-derived 200 minted-reward
expectation must be established by execution and Rust planning before M1 closes. Native
custody transactions and further periods extend this witness after that checkpoint.

Export configured allocations separately from actual post-genesis balances: Go debits
the delegator and credits DPoS custody; Rust's existing constructor consumes effective
allocations and performs its custody credit once. Validate canonical native/catalog state
before beginning a session.

## Reference modes and evidence

The public Go pin lacks concrete observer/catalog APIs. Use typed per-pin adapters and
three runs: public batched, local batched, and local concrete observer. Do not claim a
public observer run. Compare shared execution/rewards/root/receipt behavior, account and
slot bytes, and explicitly report any extra retained intermediate trie nodes. Preserve
pending empty-byte tombstones as values; a historical absence read has a different contract.
Catalog identities are exported from local actual initialization and cross-checked against
public genesis rows/root, not fabricated for the public API.

## Contracts being resolved

- Bind each consensus invocation to its enclosing transaction and a contiguous period
  sequence beginning at zero. Empty transactions and stateless calls consume no sequence.
  Keep the canonical thirteen-field projection/fifteen-field invocation encoding stable.
- Reuse recognized native quote/funding/depth/payability rules in staged execution and
  independent replay. Full call value reaches classification before conversion. Compare
  typed native failure text, not only success/failure; insufficient child gas charges zero.
- Selected CALL/STATICCALL admission requires the code and state addresses to be DPoS.
  Retain effective static context locally; do not reconstruct inherited static mode from
  the legacy call-kind field. CALLCODE/DELEGATECALL native execution remains unsupported.
- Payable/withdrawal replay requires current invocation-time ordinary accounts. Design an
  opt-in Rust context sidecar bound to exact projection/identity/plan and invocation IDs;
  preserve the legacy production entry and wire format. Replay native effects against
  scratch ordinary accounts, retaining semantic/raw state but letting final transaction
  projections determine surviving ordinary state. The sidecar is trusted ordinary-EVM
  evidence, like existing transaction account effects, not an intermediate trie proof.
- Native account ports must preserve signed, arbitrary-width journal balances. Reuse
  business kernels through narrow account-transfer ports rather than hidden clamped or
  truncated account copies. Existing fixed-width consensus snapshots remain an explicit
  representability boundary to resolve before claiming composed width coverage.
- Bind the staged session to the application request at construction; consume it once
  for rewards. Reuse existing reward planning/deltas/cleanup/corrections and keep the
  rewards-runtime storage update and publication with the existing outer session.
- Read complete prior/touched catalog values from the exact prepared view, including
  tombstones. A catalog hash or semantic reconstruction alone cannot authenticate values.
  Ordered native raw mutations need operation-owned serializers, not sorted final-map diffs.

M1 closes only after the actual oracle/Rust genesis and nonzero-reward witness, workload
manifest and these contracts pass review. Subsequent slice status and reproduction evidence
will be linked here; no root, replay or publication parity is claimed by this handoff alone.

## Reviewed implementation checkpoints

The initial witness is committed separately from the larger signed workload. Its three
Go modes reproduce byte-for-byte with the normal and optimized Python runner:

```sh
python3 experiments/evm_feasibility/mixed_period_reference.py
python3 -O experiments/evm_feasibility/mixed_period_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus mixed_genesis
```

The witness binds execution fields to decoded signed transaction bytes, proves the local
native catalog covers all 15 live genesis trie paths, and matches public/local batched
CF1–CF5 exactly. Local observer execution retains one additional CF2 post-transaction
node, named in `mixed_comparison.json`; execution, final root and all other physical rows
agree. Actual fees are 21,000 and minted reward is 200. Rust independently derives the
same complete validator reward-stat set from the declared finalized facts and existing
rewards planner. This witness does not claim the arbitrary validator address `000…0031`
is a recoverable PBFT signer.

Actual Go initialization exposed an existing serializer mismatch: all five current DPoS
iterable families construct prefixes with spare backing capacity. `IterableMapReader.Init`
appends discriminator zero and then discriminator two to that same prefix; the latter
overwrites the former. The scoped Rust DPoS helper now preserves the observed discriminator
two for item-at-position keys. The witness requires every observed genesis row to match;
V2 creation/deletion rows still require the mixed workload oracle.

Completed reviewed chunks also share selected admission ordering and exact native errors,
reject wrong transaction association or noncontiguous invocation sequence, introduce the
full signed native account port, and distinguish a zero debit's ensure-existence behavior
from a zero credit's touch behavior. Targeted tests and `make rewrite-validate-fast` pass
through the iterable correction. These isolated checks do not establish persisted mixed
parity or complete the milestone.

The opt-in genesis metadata hydrator restores only configured accounts. The caller includes
the declared zero-balance Cornus code accounts, establishes the concrete database pairing,
resolves any pending publication, and then enriches the genesis snapshot from the fixed
period-zero reader. Numeric balances and nonces must agree; code bytes/hash/size must agree.
At a later head only historical snapshot zero changes. No database is adopted or written
by this method. The existing bounded snapshot constructor alone still lacks this metadata.

The composed workload uses a known PBFT signing key (`09` repeated 32 times), with validator
address `58da990a8f4a3a6ca7cb6315d68a140105917352`. Its owner is the deterministic future
contract created by sender key `01` at nonce eight. That contract calls DPoS through ordinary
CALL, including calls whose ancestor reverts; native DELEGATECALL/CALLCODE remain rejected.
The larger workload has separate fixtures and requalifies its changed allocations and
validator/owner addresses before integration acceptance.

## Opt-in replay and reward contract

The application may pass a public opaque prepared reward plan to a Rust staged-native leaf;
its fields and constructors remain crate-private. The session binds the request at begin,
validates the actual plan's head/generation/period at finish, and is consumed once. The
application still owns rewards-runtime updates, publication and recovery. Existing CXX
payloads and request-only leaves retain their current behavior.

The Rust-only sidecar binds the request ID and exact canonical projection digest. It carries
one complete context per consensus invocation, unique actually consumed account/raw reads,
and original ordered ordinary/raw effects; rewards have one terminal context. A second
private native session replays those facts with the same business kernels and operation
serializers. Ordinary scratch state is discarded between calls, while final transaction
account projections own what survived rollback. Existing canonical semantic checks and
the Go oracle remain independent evidence; reuse is not an independent serializer.

Raw classifications follow their actual observation boundary. A deletion reads as
`Present(empty)` immediately in a journal, but can become a physical tombstone or proved
absence after settlement. No unavailable-history error is normalized. The bounded DPoS
account has preexisting Cornus code and admitted calls cannot delete/recreate it; no claim
is made that every raw mutation in an arbitrary account survives all EVM rollback.

The aggregate eligible-vote and delegated-amount raw rows flush at Go `EndBlockCall`, not
after each invocation or `PrepareIntermediateRoot`. Integration must preserve this deferred
write behavior rather than require every semantic change to have an immediate raw row.
