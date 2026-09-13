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
