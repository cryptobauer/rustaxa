# S1 execution contract and S3 ownership map

Status: proposed S1 input for the execution worker. This note maps existing code
and pinned-reference behavior; it does not authorize production routing, change a
shared interface, or claim that S3 is implemented.

## Evidence boundary

The production integration base is `ec5f37b4145f5fd42a3125c1818af58fb1d6aead`.
The local Go execution reference is the `submodules/taraxa-evm` object
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`; the public reference used by the
checked-in exporter is `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`.
The fixture manifest records the exporter and artifact hashes. Both references
agree over the checked-in synthetic corpus, but the corpus does not prove the
deployed binary or every historical activation.

The implementation plan selects a Rustaxa-owned envelope, frame stack, host and
journal above the pinned REVM interpreter. It preserves the existing separate
application and concrete RocksDB databases. This map therefore excludes database
layout changes, publication authority, production selection and protocol changes.

## Existing Rust surfaces to reuse

| Surface | Existing source | Use and constraint |
| --- | --- | --- |
| Arbitrary-width nonce | `rust/crates/rustaxa-types/src/final_chain.rs`, `FinalChainNonce` | Reuse directly for transaction and account nonce, comparisons, canonical minimal bytes and unbounded successor. Never create a `u64` or `U256` shadow. |
| Gas and ordered position | same file, `FinalChainGas`, `FinalChainTransactionPosition` | Reuse for the `u64` gas domain and the existing finalized-order adapter. |
| Existing finalized request | same file, `FinalizationTransaction`; `rustaxa-consensus/src/final_chain_execution.rs`, `FinalChainEvmTransactionInput` and `FinalChainEvmExecutionRequest` | Keep as the current application-to-leaf adapter. Its gas price and value are deliberately `U256`; it is not the general internal execution numeric model. |
| Existing result/publication protocol | `rustaxa-consensus/src/final_chain_execution.rs`, `FinalChainEvmTransactionResult`, `FinalChainEvmExecutionReport`, `FinalChainExternalEvm*` | Adapt S3 results into this session protocol. Do not create another session, commit planner or publication manager. |
| Runtime execution port | `rustaxa-consensus/src/consensus_application_runtime.rs`, `ConsensusExecutionPort`; `final_chain_execution.rs`, `FinalChainExecutionLeaf` | Existing composition owns preflight, ordered execution, rewards, commit/discard and publication sequencing. The new crate has no authority to advance the committed generation. |
| DPoS/slashing business kernels | `rustaxa-consensus/src/final_chain.rs`, `decode_dpos_transaction_for_execution`, `apply_dpos_mutation_transaction`, `apply_slashing_transaction` | Reuse through a narrow consensus-owned staged adapter. These functions and their `DposSnapshot`/outcome types are private and cannot be imported by `rustaxa-evm`; cloning them would fork consensus semantics. |
| Native transcript validation | same file, `replay_concrete_precompile_invocation` and `validate_concrete_precompile_storage_transition`; `concrete_state_projection.rs`, `FinalChainConcreteInvocation` | Retain as integration validation. It already checks ABI classification, required gas, outcome/log/output, historical snapshots and exact storage deltas, but consumes a reported transcript rather than executing an EVM frame. |
| Transaction codec | `rustaxa-types/src/transaction.rs`, `LegacyTransactionEnvelope` | Reuse canonical RLP/hash/signature behavior at the legacy wire adapter. Its decoded nonce, price and value are `U256`, so it does not replace the general wide-value execution input. |
| Experimental driver | `experiments/evm_feasibility/src/{frames,host,profile,native}.rs` | Port proven rules and fixtures, not these types. `ProbeHost`, clone checkpoints, depth eight, fixture balances, fixed Istanbul selection and hand-written native serialization are intentionally test-only. |

The shared concrete-state domain belongs in `rustaxa-types`, and compatible
RocksDB implementations belong in `rustaxa-storage`. Storage must not depend on
the new execution crate. The provisional `concrete_state.rs` contract defines
`ConcreteStateIdentity`, `ConcreteAccountBalance`, `ConcreteAccountRecord`,
`ConcreteStorageKey`, `ConcreteRead`, `ConcreteReadError` and
`ConcreteStateRead`. Its immutable reader is bound to one period/root identity
and distinguishes present, absent and tombstoned data plus pruned, unavailable,
future, corrupt, identity and I/O failures. Account decoding uses
`FinalChainNonce` and a separate arbitrary-width unsigned persisted balance.
Exact physical account bytes remain paired evidence rather than fields whose
representation is silently normalized.

The concrete-state reader should expose account, raw storage and code reads. A
block-hash lookup is a separate application port because block history is not a
concrete `state_db` responsibility. The journal converts an absent ordinary slot
to EVM zero; the storage implementation must not collapse absence, a tombstone
and stored zero.

## Numeric contract

The pinned Go contract is wider than the EVM operand stack:

- `core/vm/evm.go:110-118` represents transaction gas price, nonce and value as
  `*big.Int`; gas remains `uint64`.
- `core/vm/interface.go:40-62` represents account balance, nonce and ordinary
  slot key/value as `*big.Int`.
- `core/vm/evm.go:264-345` performs fee multiplication, affordability,
  subtraction/refund and nonce admission in arbitrary-width integer arithmetic.
  Nonce skipping is enabled. Before Cornus, several envelope failures do not
  advance the nonce; at and after Cornus the specified failures set it to
  `transaction nonce + 1`.
- `core/vm/instructions.go:248-266,417-419,476-478` converts balance, call value
  and gas price with `uint256.FromBig`/`SetFromBig` and ignores the overflow
  result. Pinned `github.com/holiman/uint256` v1.2.4 retains the low 256 bits.

Consequently the internal S3 envelope should carry arbitrary-width non-negative
gas price and value alongside `FinalChainNonce`, and do fee/value arithmetic at
that width. Journal balances must be signed arbitrary-width values: the reference
exempts the zero/system sender from affordability checks and can temporarily
subtract below zero. Only a non-negative value can convert back into the shared
unsigned persisted `ConcreteAccountBalance`; that settlement needs an explicit
compatibility outcome rather than a cast. Conversion to an interpreter word is
an explicit low 256-bit projection matching the reference. The existing finalized
request adapter naturally admits only its current `U256` subset; that boundary
must return a typed conversion error for an unrepresentable future input rather
than truncate it.

## Proposed S3 contracts

Names remain subject to the lead-owned S1 shared contract, but responsibilities
should remain separated as follows.

`ExecutionTransaction` owns sender, optional receiver, `FinalChainNonce`, wide
gas price, wide value, `FinalChainGas`, calldata/initcode, canonical bytes and
kind/system facts. `BlockContext` owns period, author, timestamp, gas limit,
chain id, difficulty and an application-owned block-hash port. `ExecutionResult`
owns status, gas, output, optional attempted creation address (including failed
creation), ordered logs, and separate
execution/code and consensus/admission errors. Errors are typed internally; text
conversion occurs only at the current compatibility adapter.

`ExecutionState<R>` owns one immutable shared reader and a transaction journal.
The reader identity is checked once on construction and never changes within an
execution. A read after a pruning, corruption, identity or I/O error aborts the
execution as infrastructure failure; it cannot become a failed receipt or zero.

`JournalCheckpoint` is opaque. The journal has explicit lanes:

| Lane | Read visibility | Frame revert | Transaction boundary | Flush order |
| --- | --- | --- | --- | --- |
| Ordinary account/storage/code, balance and nonce | Ordinary overlay, then committed reader | Revert to checkpoint | Commit or discard with transaction | Before native raw storage |
| Logs and refund | Current frame/transaction | Revert to checkpoint | Receipt/refund settlement, then reset | Not concrete raw storage |
| Native raw storage | Raw overlay, then committed reader; ordinary reads do not see it | Survives for an existing account | Flush if the account survives | After ordinary storage, so raw wins on an overlapping key |
| Transient storage | Transaction-local transient overlay | Survives nested frame revert in the pinned behavior | Always clear | Never persisted |

Account lifecycle remains a separate journal fact. In the pinned
`state_evm.Account`, an irreversible raw setter has no undo callback, but creation
of an absent account does. Reverting that creation removes its account body and
its raw map. At flush, a still-empty newly created account is deleted even when it
has only raw dirty storage. This prevents a blanket rule that all raw writes
survive or that raw writes make an account exist.

The native boundary should be task oriented:

```text
NativeInvocation {
  transaction_index, sequence, depth, call_type,
  caller, contract, value, input, supplied_gas, profile
}
    -> NativeOutcome {
         status, required_gas, gas_used, output, logs,
         ordered_ordinary_account_mutations, ordered_raw_mutations
       }
```

The frame/envelope owner applies fees, value transfer, nonce changes, static and
payability admission, action-gas settlement and account lifecycle exactly once.
The native adapter owns ABI decoding, historical method availability, business
state transition and exact serialization of touched rows. Its mutable prepare
step may reproduce historical lazy cache preparation while emitting no business,
account, raw or log mutation. The quote is bound to the exact invocation. If gas
is insufficient, invocation does not run the business kernel, though the
reference-shaped preparation cache may remain. Both phases read current account
existence/nonce/balance and the native raw lane through a narrow journal port so
earlier ordinary transfers and rollbacks are authoritative. The adapter never
publishes FinalChain state or reenters a held FinalChain lock.

Native ordinary account effects are an ordered enum of balance replacement,
nonce replacement and touch/existence transitions, with expected current values
where applicable. The executor validates and applies them sequentially through
the ordinary journal so frame revert can undo them. Raw put values are nonempty
by construction because empty bytes denote deletion in the reference. Native
semantic cache lifetime is separate from journal account lifecycle: reference
caches may survive undo or removal of a newly created raw account. Exact cache
lifetime and native serialization remain the S5 consensus-adapter concern; S3
must not invent an opaque successor identifier. Logs likewise enter the ordinary
frame journal. The historical native profile must permit the observed STATICCALL
mutation and preserve native raw writes across own/outer frame revert while
reverting their logs. Contract failures retain their exact compatibility payload.

`FrameDriver` handles CALL, CALLCODE, DELEGATECALL, STATICCALL, CREATE and
CREATE2 yields. It owns checkpoints, value context, static propagation, depth,
gas return/consumption, return-data insertion, child settlement and code deposit.
CREATE derives its address from the caller and exact arbitrary-width nonce. The
attempted address remains in the result even when creation fails, including an
insufficient-transfer envelope failure. The caller increment occurs before
collision checking and before the child checkpoint, so child failure retains
that increment; an enclosing frame revert can still remove it.

`ExecutionProfile` is a Taraxa activation set and instruction/gas table, not an
Ethereum `SpecId`. The REVM `SpecId` may select shared interpreter machinery, but
mixed Taraxa rules are installed explicitly: historical SSTORE costs/refunds,
PUSH0, legacy transient opcodes `0xb3`/`0xb4`, and Cacti activation of standard
`0x5c`/`0x5d`. Dependency upgrades must rerun width, frame and profile fixtures
and audit new REVM branches that inspect bounded account nonce.

## S3 file ownership

After S1 lands, the following files form the proposed boundary below
`rust/crates/rustaxa-evm/src/`:

| File | Responsibility |
| --- | --- |
| `lib.rs` | Lead-owned documented public exports and crate invariants only. |
| `types.rs` | Lead-owned execution/native contracts: wide transaction/block/frame/result values and explicit conversion to/from current Rustaxa types. |
| `envelope.rs` | Ordered transaction admission, fee/nonce/intrinsic-gas behavior, simulation distinctions and result settlement. |
| `profile.rs` | Taraxa activation flags, instruction table and gas schedule construction. |
| `journal.rs` | Account lifecycle, original/current/new ordinary storage, raw and transient overlays, logs/refunds and checkpoints. |
| `host.rs` | Narrow REVM `Host` implementation over the journal and lead-owned external block-hash/native ports. |
| `frame.rs` | Frame stack, all call/create kinds, gas/value/return-data/code settlement. |
| `native.rs` | Lead-owned native invocation port contract; no FinalChain kernel implementation. |
| `result.rs` | Receipt-facing typed status/log/output conversion and invariants. |

The lead retains workspace manifests/lockfiles, shared `rustaxa-types` contracts,
crate exports and execution/native contracts, the consensus-native adapter,
composition and `FinalChainExternalEvm*` integration. The execution worker starts
with `envelope.rs`, `profile.rs`, `journal.rs` and `frame.rs`; host/result work is
assigned only after their shared inputs settle.
The state worker retains concrete codecs/readers/writers in `rustaxa-storage`.

## Reviewable implementation order

1. Land the S1 shared types and immutable read trait, then create the unlinked
   `rustaxa-evm` crate with conversion tests. No production crate depends on it.
2. Implement journal account lifecycle and the four mutation lanes against a
   deterministic in-memory reader. Close absence/tombstone/error behavior before
   interpreter integration.
3. Implement the general envelope and wide arithmetic, including pre/post-Cornus
   failure ordering and simulation behavior.
4. Implement the mixed instruction/gas profile and host ordinary/transient
   methods, then match E1-E3 single-frame fixtures.
5. Implement CALL-family frame settlement, followed by CREATE/CREATE2 using the
   exact nonce. Reuse REVM interpreter actions; do not copy the probe's account
   model or clone-checkpoint strategy.
6. Add the typed native port and transcript settlement. The consensus-owned
   kernel adapter follows as a separate assigned change after its contract is
   reviewed.
7. Stop at the bounded S3/S4 checkpoint: simple transfer and storage-changing
   call through the existing FinalChain test composition, before expanding all
   native/precompile operations.

## Targeted validation

Each S3 commit should run `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm --all-targets -- -D warnings`,
`cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm`, and `git diff --check`.
Run `make rewrite-validate-fast` before closing the slice.

Behavioral fixtures must include the existing 16 creation cases, 16 envelope
cases, seven opcode observations, raw/account lifecycle and overlap cases, and
the six actual native-call cases against both pinned references. Add focused
cases before claiming general behavior for:

- gas price, transaction value and account balance above 256 bits, checking both
  wide envelope arithmetic and low-word opcode projection;
- zero/system-sender negative intermediate balance and its exact transaction and
  persistence settlement;
- top-level CREATE with insufficient transfer value, whose nonce result follows
  a different ordering from ordinary successful creation;
- every CALL family with success, REVERT, exceptional halt, value failure,
  static propagation, depth and gas-return boundaries;
- complete SSTORE original/current/new/refund and low-gas/stipend ordering;
- transient survival through child revert and clearing at transaction commit;
- ordinary/raw overlap in both write orders and absent/new/suicided account
  lifecycles;
- native own-frame and parent-frame reverts, historical STATICCALL mutation,
  malformed ABI, insufficient gas and every historical activation exception.

No storage differential, broad replay, full CTest, fault campaign or sustained
workload is part of this mapping task. Those gates require the documented owner
approval and, for production routing, separate authorization.
