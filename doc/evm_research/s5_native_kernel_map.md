# S5 native kernel integration map

Status: bounded design map. This document does not add a production route, change a protocol rule, or authorize
publication from the EVM executor. It defines the smallest test-only S5 step that can reuse the existing Rust
FinalChain kernel for `setCommission(address,uint16)` without making consensus depend on the EVM crate.

## Existing boundaries

The relevant code already separates most of the required responsibilities. The remaining work is a staged adapter
and an operation-specific serializer, rather than another DPoS implementation.

| Existing surface | What it establishes | What it must not become |
| --- | --- | --- |
| [`NativeExecutionPort`](../../rust/crates/rustaxa-evm/src/contracts.rs) | The EVM-facing two-phase `prepare`/`invoke` contract, exact invocation identity, journal reads, action-gas quote, ordinary mutations, exact raw mutations and logs | A FinalChain owner, persistence API, or place to copy DPoS rules |
| [`experiments/evm_feasibility/src/native.rs`](../../experiments/evm_feasibility/src/native.rs) | A disposable private-child-module probe that calls the real decoder and mutation kernel, serializes one validator row and compares the Go fixtures | A production module or general native serializer; its fixed 20,000-gas insertion and hand-built parent result are corpus scaffolding |
| `FinalChain::dpos_snapshot_at_finalized_block` | Exact snapshot lookup for the requested finalized block, with missing history reported as an error | A fallback to genesis or a current mutable global snapshot |
| `decode_dpos_transaction_for_execution` | Fork-aware selector classification, including unrecognized input, malformed mutations and method availability | An EVM envelope or call-frame decoder |
| `FinalChain::dpos_call_required_gas` and `dpos_transaction_required_gas` | Payability and method gas policy; `setCommission` has a 20,000 action-gas requirement in the pinned profile | Total transaction or CALL-opcode gas accounting |
| `FinalChain::apply_dpos_mutation_transaction` | Mutation dispatch against caller-owned staged `DposSnapshot` and account state; `apply_dpos_commission_update` enforces owner, maximum, frequency and delta rules and emits the existing log | Fee charging, nonce advancement, call-value movement, frame rollback, receipt building or publication |
| `FinalChain::external_evm_concrete_projection` | Period-order replay and validation of a root-bound concrete transcript against a staged DPoS snapshot | The online EVM host or a serializer for raw writes |
| `FinalChain::replay_concrete_precompile_invocation` | Independent replay of supplied gas, status, output and logs for DPoS/slashing invocations | Nested `FinalChain::call`, or proof that a normalized snapshot is the byte-exact raw database image |
| `canonical_concrete_precompile_storage` and transition validators | Candidate generation for validating a complete supplied projection, including tolerated historical validator encodings | An executable write plan. It synthesizes live snapshot rows and cannot recover removed iterable entries, write order, tombstones or untouched-byte provenance |

`rustaxa-consensus` currently has no dependency on `rustaxa-evm`. The EVM crate uses consensus only as a development
dependency for tests. Preserve that direction: the staged kernel API owns plain consensus-domain request/result types,
and a composition-local wrapper implements `NativeExecutionPort` by converting those types. The first wrapper can live
in test composition. A later production wrapper belongs at the application composition boundary selected by the lead;
it does not require either domain crate to import the other.

Calling the existing whole `FinalChain::call` for a yielded child frame is not an acceptable shortcut. That method owns
top-level call simulation and finalized-state selection. Nesting it would select the wrong state after an earlier
same-period mutation and risk repeating envelope gas, value and nonce work. The session calls the decoder, gas helper and
business kernel directly, without taking a publication lock or re-entering FinalChain orchestration.

## Smallest staged session

Add a consensus-owned child module beside `final_chain.rs`, provisionally `final_chain/native_session.rs`. A child
module can wrap the private `DposSnapshot` and existing private kernel helpers without widening those implementation
types. Public request/result structs expose only stable domain facts. Names below are proposals; their fields and
invariants are the contract that matters.

```text
FinalChain::begin_native_session(pending_period, expected_parent)
    -> FinalChainNativeSession {
         pending_period,
         next_sequence,
         dpos_state,             // prior finalized head, advanced for pending_period
         dpos_gas_state,         // separate same-period gas/cache view
         delayed_read_caches,    // empty; reserved for later methods
         prepared_call           // at most one, exact request plus raw observations
       }

FinalChainNativeRequest {
  transaction_position, sequence, period, depth,
  call_kind, is_static,
  caller, code_address, state_address,
  value: FinalChainNativeValue, input, supplied_gas
}

FinalChainNativeSession::prepare(request, current_state)
    -> FinalChainNativeGasQuote { transaction_position, sequence, required_gas }

FinalChainNativeSession::invoke(request, quote, current_state)
    -> InsufficientGas { required_gas }
     | Completed {
         status, gas_used, output,
         ordinary_account_mutations,
         raw_mutations, logs, diagnostic
       }
```

`FinalChainNativeValue` is a consensus-owned `BigUint` newtype, or an equivalent canonical arbitrary-width unsigned
byte representation. It must not reuse the existing `FinalChainTransactionValue`, which is limited to 256 bits, or
import EVM-owned `ExecutionValue`. Zero/nonzero payability testing occurs before any checked `u256` conversion. The
first nonpayable `setCommission` step therefore handles values wider than 256 bits without narrowing; support for a
future payable native method whose value exceeds 256 bits remains an explicit kernel-boundary gap.

`current_state` is a small consensus-owned read trait with exact account and native-raw methods. A wrapper around the
S1 `NativeJournalRead` implements it in the composition layer. This keeps consensus independent of EVM while ensuring
that preparation sees earlier same-transaction value movement, ordinary rollback and native raw writes. Its raw result
retains `Present(bytes)`, `Absent` and `Tombstone`; an I/O, identity, pruning or corruption error is an abort, not a
contract failure.

Session construction checks that `expected_parent` is the currently selected finalized head, clones
`dpos_snapshot_at_finalized_block(expected_parent)`, and applies the existing pending-block reward-reference advance
used by `external_evm_concrete_projection`. It does not publish the clone. The session is single-period and ordered:
`sequence` must increase by one, only one quote may be outstanding, and `invoke` must match every request field and the
quote retained by `prepare`. Insufficient child gas retains preparation cache but runs no business kernel and emits no
ordinary, raw or log effects.

Every successful `invoke` boundary consumes its quote and advances the port-call sequence, whether it returns
`Completed` or `InsufficientGas`; lazy semantic/read caches prepared for reference behavior may survive. An integrity
error aborts the period. A pre-fix nested call rejected by the frame host never calls `NativeExecutionPort::prepare`
and does not consume this port-call sequence. The executor assigns `NativeInvocationId.sequence` when it actually
enters the port and separately retains the rejected call in its execution trace.

For the first step, the session admits only the DPoS address, `CALL` and `STATICCALL`, and the decoded
`SetCommission` variant. The executor performs the historical pre-fix nested-call admission before `prepare`; the Go
fixture proves that this rejection has zero kernel calls. `CALLCODE`, `DELEGATECALL`, DPoS reads, other mutations and
slashing return an explicit unsupported adapter error in this bounded test route. They cannot fall through to
`FinalChain::call`.

Preparation performs these operations without changing business state:

1. Check the request period, address, supported call kind and next identity. Bind `depth`, `is_static`, caller,
   code/state addresses, full-width value, calldata and supplied gas as well as the identity.
2. Apply the existing fork-aware nonpayable/selector policy and call `decode_dpos_transaction_for_execution`. Require
   exactly `SetCommission { owner: caller, validator, commission }` for this first implementation.
3. Compute action gas through the same helper used by `dpos_call_required_gas`. The expected quote for the fixture is
   20,000. Post-Cornus `setCommission` with any nonzero full-width value instead prepares a terminal nonpayable result
   with required gas zero. Its `invoke` returns the exact contract failure with zero native gas and no
   kernel/account/raw/log effects. The child-frame failure rolls back the frame-owned CALL value. Pre-Cornus behavior
   is outside this first adapter and remains explicitly unsupported rather than silently applying the post-Cornus rule.
4. Short-circuit a terminal nonpayable request or `supplied_gas < required_gas` after binding the quote. These paths do
   not read validator/owner raw rows or validate their domain state, because the reference exits before the business
   storage kernel. They consume the quote at `invoke` with no business effects.
5. Only for a zero-value request with sufficient supplied gas, read the current validator row at native logical key
   `keccak256(0x0000 || validator)` and the owner row at `keccak256(0x0003 || validator)` from the raw lane. Decode the
   validator row as the fork-selected RLP shape and compare stake, commission, last commission-change block,
   reward-reference head and, when present, undelegation count with the staged snapshot. The owner row is not RLP:
   require exactly 20 raw bytes equal to staged `metadata.owner`. Reject absence, tombstone, malformed validator RLP,
   malformed owner length or any raw/domain mismatch as an integrity error.
6. Retain the exact validator bytes/read classification and all decoded facts with the quote. For this admitted path,
   `invoke` rereads the observations and rejects a mismatch before calling the kernel.

For a gas-admitted zero-value call, invocation clones the staged DPoS state, calls
`apply_dpos_mutation_transaction` exactly once with the already decoded transaction, and serializes the changed row
before swapping the clone into the live session. This copy-then-swap order ensures that an infrastructure or
serialization error exposes neither a partial semantic advance nor a raw mutation. The terminal nonpayable case and
insufficient gas never call the kernel. A normal DPoS business rejection after kernel admission is different: it
returns the kernel's failure status/output, charges the accepted 20,000 action gas, and emits no validator write or log.

A successful `setCommission` advances the session's semantic snapshot immediately and returns the exact raw operation
and kernel log. A later EVM frame revert does not rewind this semantic advance or the raw lane. This is required so a
later invocation in the same period sees the commission written by the earlier call even if its containing frame later
reverts. The ordinary frame journal independently decides whether returned ordinary effects and logs survive.

At the end of the test transaction stream, compare the session's encoded DPoS snapshot at the pre-cleanup,
pre-reward boundary with an independently replayed snapshot at that same boundary. The current
`external_evm_concrete_projection` continues beyond this point through slashing cleanup, fee/minted rewards and final
encoding, so it does not expose a directly comparable checkpoint today. The bounded test should extract a private
helper from its existing head-clone, reward-reference advance and ordered invocation replay, returning a cloned
transaction-boundary snapshot before cleanup. The normal projection path then runs unchanged and still validates the
post-reward concrete projection. FinalChain remains the only owner that can accept either staged snapshot for
publication. The first test route should pass the session only through the existing isolated S4/S5 test composition
and must not add a production selector or bridge route.

## Exact `setCommission` raw serializer

The operation touches one DPoS raw row. Its native logical key is
`keccak256(0x0000 || validator_address)`; the concrete account-storage trie applies its separate trie-path hashing.
The returned `NativeRawMutation.expected` is the exact current raw read captured at preparation, and the operation is a
nonempty put. The serializer must not emit the owner or membership rows and must not regenerate the complete DPoS map.

The encoded validator payload uses the post-kernel values:

```text
ValidatorV1 = RLP.list(
  total_stake,
  commission,
  last_commission_change,
  reward_reference_head
)

legacy active representation   = ValidatorV1
extended active representation = RLP.list(ValidatorV1, undelegations_count)
```

Fork selection comes from the session's configured period. Under the extended profile, a preexisting four-field row
with zero undelegations is valid historical input, but a successful mutation writes the nested two-field extended
representation. It is not a flattened five-field list. The existing projection helper may independently verify the
result, but its alternative candidates must never choose the executable bytes.

For the pinned fixture, validator `0x0000000000000000000000000000000000000031` starts with:

```text
key   4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a
old   c6822710648080
new   c9c782271081c8018080
```

The old row is `[10000, 100, 0, 0]`. The successful period-1 update to commission 200 produces
`[[10000, 200, 1, 0], 0]`. The serializer obtains stake, reward head and undelegation count from the verified staged
snapshot; it does not infer them from defaults. Exact old bytes remain attached to the mutation so the EVM adapter can
detect an intervening raw-lane change.

## Single ownership of gas, value, nonce and rollback

| Fact/effect | Owner | `setCommission` rule |
| --- | --- | --- |
| Transaction admission, intrinsic gas, gas price/cap, fee debit/refund and receipt gas | EVM envelope | Never repeated by the native session |
| CALL-family base/dynamic gas, EIP-150 forwarding, supplied child gas and returned unused gas | Frame driver | The 20,000 quote is native action gas, not total call or transaction gas |
| Native required gas and business invocation admission | Staged native session | `prepare` quotes; `invoke` charges exactly the accepted quote or reports insufficient gas without invoking |
| CALL value transfer and rollback | Frame driver ordinary journal | The session binds full-width value and uses it for payability or payable argument injection only; it never transfers it |
| Transaction sender nonce and CREATE-family creator nonce | Envelope/frame driver | The session never increments either nonce |
| DPoS business account transfers | Existing kernel, returned as expected/replacement ordinary mutations | Empty for `setCommission`; the adapter applies future effects through the ordinary journal exactly once |
| DPoS raw writes | Existing kernel state plus operation-specific serializer; applied by the EVM raw lane | Survive containing-frame revert while the existing DPoS account survives |
| DPoS logs and any ordinary/refund effects | EVM ordinary frame journal | Roll back with the containing frame |
| Semantic DPoS session state | Consensus-owned session | Advances with a successful raw mutation and is not rewound by ordinary frame rollback |

The native session does not return a refund mutation for `setCommission`. Any refund already present in the EVM
journal remains the frame driver's responsibility. Total observed transaction gas in the fixture is 41,763 for CALL
and 41,760 for STATICCALL; those totals include the surrounding parent program and therefore are not values the native
session should reproduce as its quote.

The pinned reference has an additional account-lifecycle exception: reverting creation of a previously absent account
removes that account and its raw map, and transaction flush removes an otherwise empty new account even if it has only
raw dirty state. The first serializer targets the already existing DPoS contract account, so it does not solve this
general case. Expansion must preserve session-cache lifetime separately while letting the EVM journal determine whether
a new account and its raw effects reach the concrete projection.

## Reference behavior and current checks

The checked-in Go exporter runs the real Taraxa EVM and DPoS contract from two exact revisions:

- public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`;
- local/mainline comparison `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.

[`reference.py`](../../experiments/evm_feasibility/reference.py) archives each revision into a disposable tree and
compiles [`reference_native.go`](../../experiments/evm_feasibility/reference_native.go) there. The checked-in manifest
records Go 1.24.4, exporter SHA-256
`27e2cbc001a989a441d8a5fdc6e09b0b89bf501673d5e175e0aa959c56acdbf5`, and byte-identical public/local artifacts with
SHA-256 `b73ce3ae99ab4a24b7cd272c395c7b41a0750686a0d156cfbd5c15db9b1cb593`.

The six real EVM cases establish:

| Case | Exact observed native behavior |
| --- | --- |
| Successful CALL | One validator put, one commission log, true child result, 41,763 total gas and matching raw/storage/account roots |
| Parent REVERT after successful CALL | The same validator put and storage root survive; log disappears, parent reports `execution reverted`, and ordinary account root follows rollback |
| Wrong owner | Kernel runs and returns false; no raw write or log; accepted native action gas is consumed |
| Commission 10,001 | Kernel returns false; no raw write or log; accepted native action gas is consumed |
| Before nested-call fix | Host returns false before the kernel; the probe observes zero kernel calls and no raw write/log |
| STATICCALL | Historical profile permits the mutation: the same raw put and log occur, with 41,760 total gas |

The disposable Rust probe calls `decode_dpos_transaction_for_execution` and
`apply_dpos_mutation_transaction`, checks the exact ordered write above, and independently reconstructs the storage
root, account leaves and combined account-state root for all six cases against both Go artifacts. Its locked command is:

```sh
python3 experiments/evm_feasibility/native.py
```

The separate journal exporter pins the same two Go revisions and exercises actual `TransitionState`, `TrieSink` and
fresh-reader reopening. Its base, extended and mutator manifests record byte-identical reference outputs. Those cases
show that for an existing account an irreversible raw write survives frame rollback while ordinary storage, logs and
refund are restored; transaction completion clears logs/refund and transient state, and flush applies ordinary writes
before raw writes. They also establish the new-account exception described above. Reproduce them with:

```sh
python3 experiments/evm_feasibility/journal_reference.py
python3 experiments/evm_feasibility/journal_reference.py --extended
python3 experiments/evm_feasibility/journal_reference.py --mutators
```

These are synthetic memory-backed fixtures. They do not prove RocksDB continuation, mainnet historical serializer
coverage, all native return payloads, every CALL family member or period publication.

## Ordered expansion after the first adapter

Expansion should proceed by native operation, with each step adding a real Go fixture and an exact write-set serializer
before enabling that operation in the adapter.

1. Complete DPoS mutations. Reuse `decode_dpos_transaction_for_execution`, value injection and
   `apply_dpos_mutation_transaction`. Add operation-owned serializers for validator info, registration, delegation,
   undelegation V1/V2, redelegation, reward claims and claim-all. Each serializer must record actual ordered puts and
   empty-byte deletions, including iterable count/item/reverse-index moves and reward-reference graph rows. Load only
   the account working set required by the kernel and return its balance/nonce/touch delta through S1 ordinary
   mutations. Do not diff `canonical_concrete_precompile_storage` to manufacture the write order.
2. Add DPoS reads and gas caches. Preserve the existing distinction among current staged mutation state,
   claim-all gas state and delayed/frozen eligibility reads. Verify malformed ABI, method availability, payability,
   exact output and cache survival after insufficient gas. Reads emit no raw mutation.
3. Add slashing. Reuse `decode_slashing_transaction` and `apply_slashing_transaction`, including the delayed validator
   snapshot and lazily frozen read snapshot. Add proof/jail operation serializers only after the Go corpus records their
   exact keys, ordered iterable changes and tombstones. Before Magnolia, retain ordinary empty-account behavior instead
   of registering the slashing precompile.
4. Keep block rewards and system effects under FinalChain publication. The session may stage semantic facts needed by
   later calls, but reward graph advancement, cleanup, fee rewards, minted rewards and final snapshot insertion remain
   in the existing ordered FinalChain plan. Add serializers for their raw effects from reference evidence before a
   complete S5 period claims parity.
5. Implement stateless precompiles in the EVM execution/profile module. Crypto/math precompiles need exact address,
   activation, malformed-input, output and gas corpora, including the pinned FN-DSA compatibility dependency. They do
   not need a FinalChain session and must not create a consensus-to-EVM dependency.

For every expansion, the existing canonical projection remains a second, independent semantic check at period
finalization. It can reject missing or incorrect supplied rows. It cannot authorize a new serializer by itself because
it lacks physical history and does not describe write order or tombstone mechanics.

## Bounded limitations and exit test

The first implementation is complete only when an isolated EVM composition uses the S1 `NativeExecutionPort` adapter
and the consensus-owned session to pass all six existing `native_calls` rows against both pins, including exact quote,
kernel-call count, ordered raw mutation, status/output, log rollback and final roots. Add a same-period two-call test so
the second prepare reads the first call's irreversible raw row and staged commission. Run the existing journal corpus to
guard the lane lifetime, plus focused `rustaxa-consensus` and `rustaxa-evm` tests. This is bounded validation and does not
require a repo-wide or storage differential gate.

That exit test does not authorize production registration, publication, snapshot import, recovery, pruning or protocol
changes. It leaves full DPoS/slashing serializers, delayed read methods, CALLCODE/DELEGATECALL semantics, native account
lifecycle, reward/system writes, mainnet historical replay and RocksDB reopen as explicit S5/S6/S7 work.
