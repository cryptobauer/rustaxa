# S3 nested frame execution map

This map fixes the next bounded execution boundary before implementation. It compares
the two pinned Taraxa Go references (`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`
and `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`) with REVM revision
`6014612c86f3690e4e9173a8c4deade396af398d`. The Go frame paths are identical
between the pins except for the local concrete-invocation observations described
below. This is a source map and proposed test boundary. It does not claim nested
execution, native-adapter, transaction-buffering or production-routing parity.

## Selected boundary

Keep REVM responsible for instruction decoding, stack and memory checks, dynamic
opcode gas, EIP-150 forwarding and producing `InterpreterAction`. A Rustaxa-owned
iterative driver should consume each `FrameInput`, operate on the existing
`ExecutionJournal`, create a child interpreter or route a native call, and insert the
result into the suspended parent. REVM's general `EthFrame` cannot own this step: its
CREATE path uses a `u64` nonce and its own journal/account lifecycle, while Taraxa
requires an arbitrary-width `FinalChainNonce`, distinct ordinary/raw rollback lanes,
and existing Rust native kernels.

The smallest implementation is one explicit stack of active interpreters. Each stack
entry retains its interpreter, the journal checkpoint owned by that frame, and the
parent insertion facts (`CALL` output-memory range or attempted CREATE address). On a
yield, copy `CallInput::SharedBuffer` from the parent memory before a child can return
or reuse that context; REVM explicitly warns that the range can be overwritten. Then
push one child entry, or synthesize a typed immediate/native result. On return, settle
the top checkpoint and insert into the parent. This preserves the existing journal's
nested undo chain without cloning state or adopting REVM's transaction framework.

REVM already emits the required context in `CallInputs`: code address, storage target,
effective caller, transfer or apparent value, inherited static flag, forwarded gas,
input and return-memory range. `CreateInputs` supplies initcode, value, scheme and
forwarded gas, but its `created_address(u64)` helper must not be used. Rustaxa must use
`frame::create_address` with the journal's exact nonce. See pinned REVM
[`contract.rs`, lines 140-162 and 165-246](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/instructions/contract.rs),
[`call_inputs.rs`, lines 9-22 and 132-224](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/interpreter_action/call_inputs.rs),
and [`create_inputs.rs`, lines 37-105](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/interpreter_action/create_inputs.rs).

## CALL-family rules

| Rule | Required driver behavior | Pinned evidence |
| --- | --- | --- |
| Opcode gas | Let the selected REVM instruction table charge base/dynamic/memory/value/new-account costs and deduct the requested child gas before yielding. The emitted `gas_limit` is the forwarded amount plus the 2,300 stipend for nonzero `CALL`/`CALLCODE` value; only the pre-stipend forwarded amount was deducted from the parent. | Go [`gas.go`, lines 67-87 and 405-457](../../submodules/taraxa-evm/core/vm/gas.go) and [`instructions.go`, lines 651-702](../../submodules/taraxa-evm/core/vm/instructions.go); REVM [`call_helpers.rs`, lines 53-113](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/instructions/contract/call_helpers.rs). |
| EIP-150 cap | After opcode base and memory costs, cap requested gas at `available - available/64`. A request wider than `u64` clamps to this cap when EIP-150 is active. | Go [`gas.go`, lines 67-87](../../submodules/taraxa-evm/core/vm/gas.go). |
| Pre-entry failure | Depth and insufficient balance return all supplied child gas, empty output and failure to the parent. Go rejects only when its current `self.depth > 1024`; because `run` increments depth on frame entry, the iterative driver's active-frame count must be mapped to that predicate rather than copying REVM's generic boundary mechanically. The zero address is exempt from affordability checks. `CALLCODE` checks affordability but performs no transfer. | Go [`evm.go`, lines 443-455 and 613-625](../../submodules/taraxa-evm/core/vm/evm.go). |
| Checkpoint | Create the child checkpoint after depth/affordability checks and before transfer, STATICCALL touch or native execution. Commit it on child success; revert ordinary account/storage/log/refund effects on REVERT or failure. A committed child remains undoable by an enclosing checkpoint. | Go [`evm.go`, lines 443-480 and 558-567](../../submodules/taraxa-evm/core/vm/evm.go); Rust [`journal.rs`, lines 309-336](../../rust/crates/rustaxa-evm/src/journal.rs). |
| Empty ordinary CALL | A zero-value `CALL` to an absent, non-native address succeeds immediately with full child gas and creates or touches no account. Other empty-code calls still settle their transfer/touch checkpoint. | Go [`evm.go`, lines 458-486 and 552-557](../../submodules/taraxa-evm/core/vm/evm.go). |
| Context | `CALL`: callee storage/code, current account as caller, transfer value. `CALLCODE`: current storage, callee code, current account as caller, apparent requested value, no transfer. `DELEGATECALL`: current storage, callee code, inherited caller and inherited value, no transfer. `STATICCALL`: callee storage/code, current account as caller, value zero, inherited read-only mode and zero-balance touch. | Go [`evm.go`, lines 474-514](../../submodules/taraxa-evm/core/vm/evm.go) and [`contract.go`, lines 45-72](../../submodules/taraxa-evm/core/vm/contract.go); REVM [`contract.rs`, lines 179-245](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/instructions/contract.rs). |
| Static propagation | Once a bytecode frame is static, every descendant bytecode frame remains static. State-writing opcodes and nonzero-value `CALL` fail before mutation. This restriction does not suppress the pinned historical native STATICCALL mutation. | Go [`evm.go`, lines 613-629 and 668-677](../../submodules/taraxa-evm/core/vm/evm.go), plus the `staticcall-mutation` native fixture. |
| Result insertion | Push one only for success and zero for REVERT/failure. Replace parent return-data with exact child output for every CALL-family completion; copy `min(output length, requested length)` into parent memory only on success or REVERT. Leave the rest of the output region unchanged. Return unused child gas on success/REVERT and on a pre-entry rejection; an ordinary exceptional bytecode halt returns zero child gas. Propagate child refund only on success. | Go [`instructions.go`, lines 651-750](../../submodules/taraxa-evm/core/vm/instructions.go); REVM [`frame.rs`, lines 463-560 and 574-625](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/handler/src/frame.rs). |

Track zero-based bytecode-frame depth in each stack entry. Go increments `self.depth`
only while `run` is active, so its value is `entry.depth + 1`; native calls do not add
another interpreter depth. The next yielded child depth and `NativeInvocation.depth`
are both `parent.depth + 1`, while admission compares the current active bytecode count
to Go's strict `> 1024` predicate.

The parent interpreter must remain suspended while the child runs. Child REVERT and
ordinary execution errors are results of the CALL opcode: they update its stack,
memory and return-data, then parent execution continues. Only a host, journal, code
integrity or native-port error aborts the pending period instead of becoming a CALL
failure. The Go loop replaces `last_retval` with the exact result returned by every
CALL-family opcode and clears it when a new child supplies no bytes; see
[`evm.go`, lines 627-629 and 707-729](../../submodules/taraxa-evm/core/vm/evm.go).

Child refund has one owner. A successful child merges its signed interpreter refund
delta into its immediate suspended parent's gas tracker; failed children contribute
none. If that parent later reverts, its accumulated descendant delta is discarded. The
intermediate delta may be negative when a child restores an ancestor's clear and must
not be rejected or narrowed. Only after the root returns may the final aggregate be
validated and registered once in `ExecutionJournal`, where the envelope applies its
cap. Child insertion must not also write the same refund directly to the journal.

## CREATE-family rules

REVM should continue to charge memory, CREATE/CREATE2 base cost and CREATE2 hash cost,
then forward `remaining - remaining/64`; the driver consumes the yielded initcode and
scheme. Go performs depth and zero-address-aware affordability checks before any
mutation. On admission, derive CREATE from the caller's exact current nonce (or CREATE2
from salt/initcode hash), increment the caller before collision detection, and retain
that increment on collision or child failure. The child checkpoint begins only after
the collision check. Within it, initialize the target nonce to one, transfer value,
run legacy initcode, enforce the 24,576-byte runtime limit, and charge 200 gas per
runtime byte. An enclosing parent revert can still undo the creator nonce increment.
See Go [`instructions.go`, lines 588-648](../../submodules/taraxa-evm/core/vm/instructions.go),
[`gas.go`, lines 341-370](../../submodules/taraxa-evm/core/vm/gas.go), and
[`evm.go`, lines 345-436](../../submodules/taraxa-evm/core/vm/evm.go).

On success, commit the child checkpoint, return unused gas, push the attempted address
and clear CREATE return-data. On REVERT, revert the checkpoint, return unused gas,
push zero and retain only revert bytes as CREATE return-data. Collision, code-size,
code-deposit and other exceptional failures revert the child, consume its supplied
gas, push zero and clear CREATE return-data. Depth and insufficient-balance rejection
occur before entry, preserve supplied gas, push zero and clear return-data. Existing
`frame::ChildFrameStatus` and `settle_create_child` already encode these categories.

## Native calls

Classify `CallInputs.bytecode_address` before ordinary code execution. Build exactly one
`NativeInvocation` from the period-local transaction position and monotonic sequence,
effective depth/kind/static mode, effective caller, native contract address, storage
context address, full value, copied input and emitted child gas. `CALLCODE` uses the
current account as `state_address`; `DELEGATECALL` also inherits its parent's caller and
value. The full value comes from Rustaxa frame context, because REVM's opcode word is
only the low 256-bit projection.

Call mutable `prepare` against the current authoritative journal, then call `invoke`
with the exact same invocation and quote. Preparation may populate reference-shaped
semantic caches but emits no business effects. If supplied gas is below the quote, do
not invoke the business kernel or apply mutations. A completed result must charge the
quoted gas exactly. Apply ordered ordinary mutations and logs through the child
checkpoint. Apply raw mutations through the journal's historically irreversible lane;
they survive own-frame and enclosing-frame rollback, except where creation-account undo
removes the new account's lanes. Preserve native contract failure payload and output.
Port/integrity errors abort the pending period and must never be converted to bytecode
failure or a legacy fallback. These invariants are defined in
[`contracts.rs`, lines 328-403 and 540-687](../../rust/crates/rustaxa-evm/src/contracts.rs).

The pinned native path quotes before running, charges only the quote, and does not force
remaining gas to zero on native errors. It records own-frame and enclosing-frame
rollback disposition separately because raw effects survive while ordinary effects and
logs revert. It also invokes a native contract even under STATICCALL, so the historical
`staticcall-mutation` case must remain enabled. See local-pin Go
[`evm.go`, lines 516-567](../../submodules/taraxa-evm/core/vm/evm.go) and the six
[`reference_native.go` cases](../../experiments/evm_feasibility/reference_native.go).

Native settlement therefore needs a distinct result path from ordinary REVM halts. A
native contract error returns `supplied_gas - quoted_gas` to the parent, pushes zero,
and copies output memory only if the exact failure is REVERT. Quote OOG invokes no
business method, returns all supplied gas, pushes zero and copies no output. The
period-local sequence still advances for every observed native attempt. Generic REVM
`handle_reservoir_remaining_gas` would burn the remaining gas for these non-REVERT
errors and cannot be used unchanged.

## Transaction and persistence ownership

The iterative loop is transaction-local. It must neither decide when trie work becomes
observable nor retain a child frame across `settle_transaction`.

The public pin batches account flush output through `CommitTransaction`, clears logs,
refund and transient state at that transaction boundary, and later drains dirty account
sinks during `Commit`. Its `PendingBlockState.Put` queues asynchronous writes and does
not provide an in-memory pending-read map. See public-pin
`taraxa/state/state_evm/transition_state.go:153-197` and
`taraxa/state/state_db_rocksdb/latest_state.go:82-144` at `6c7e5338`.

The local concrete pin preserves the same transaction/frame behavior and adds projection
tracking plus a pending-write view. Its active concrete observer calls
`PrepareIntermediateRoot`, which calls `state.Commit()` and `trie_sink.Commit()` while
retaining the pending block I/O for subsequent work. That is a per-transaction observer
mode and unloads account caches; it is not evidence that public-mainnet execution has
the local pending-write behavior. See local-pin
[`transition_state.go`, lines 207-264](../../submodules/taraxa-evm/taraxa/state/state_evm/transition_state.go),
[`state_transition.go`, lines 271-299](../../submodules/taraxa-evm/taraxa/state/state_transition/state_transition.go),
and [`latest_state.go`, lines 101-149](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/latest_state.go).

Therefore the next frame tests may compare transaction-local journal facts and, when
using the S4 concrete observer fixture, explicit per-transaction physical versions. They
must not infer general same-block raw visibility, public-mainnet batching behavior or a
production flush policy from the local observer. A higher-level FinalChain/storage
composition owns period ordering, transaction settlement, optional concrete observer
flushes, rewards/end-block mutations, projection approval and final publication.

## Smallest acceptance corpus

Use the checked fixtures directly; do not transcribe expected gas or mutation values.

1. Run all 16 `creation_frames` cases from `fixtures/public.json` and `local.json`
   through the iterative loop. They jointly exercise wide CREATE/CREATE2, collision,
   child REVERT/invalid, parent REVERT, grandchild creation and code deposit/OOG.
2. Run `nested-transient-revert` from both fixture files. It proves CALL gas/result
   insertion and the pinned transient lane surviving child REVERT.
3. Run all six `native_calls` cases: success, parent REVERT, two contract failures,
   pre-fix nested rejection and historical STATICCALL mutation. Compare quote/charged
   gas, exact output/error, ordered raw changes, ordinary account facts and logs.
4. Add a small Rust-only insertion matrix for each CALL kind covering success, REVERT,
   exceptional halt, depth, insufficient balance, return-memory truncation and stipend
   return. It checks local bookkeeping but is not Go parity evidence.
5. Run `static-rejection` from `fixtures/sstore_public.json` and
   `sstore_local.json`. Its bytecode child attempts SSTORE under STATICCALL; the parent
   continues with a zero success word, total gas used 87,268, zero refund and unchanged
   child slot zero. Keep it distinct from native `staticcall-mutation`, which records
   the historical native exception. The checked exporter is
   [`sstore_reference.go`, lines 103-117](../../experiments/evm_feasibility/sstore_reference.go).

These tests are package-local and targeted. Broad replay, storage differential, fault
campaigns and production routing remain outside this slice and require their documented
authorization.
