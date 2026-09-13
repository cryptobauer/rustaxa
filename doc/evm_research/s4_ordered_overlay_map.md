# S4 ordered concrete observer overlay

This is a source-derived contract for the next bounded storage slice. It leaves
the existing finite adapter in `tests/persisted_period_reference.rs` unchanged.
The proposed general overlay is not implemented or validated by this document.
It does not authorize production routing, imported-state bootstrap, a database
layout change, or a change to public-mainnet transaction cache lifetimes.

The reference pins are public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`
and local `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`, as recorded by
[`reference.py`](../../experiments/evm_feasibility/reference.py). Links to
submodule files below show the local pin; the public comparison must use its
identified revision rather than treating local concrete hooks as upstream behavior.

## Ownership and selected lifetime

Storage should own the ordered physical overlay, compatible trie updates,
canonical account rows, and preparation tokens. Execution should own transaction
and frame journals, ordinary/raw lane visibility, gas originals, native semantic
caches, and the decision to settle a transaction. Existing FinalChain ownership
continues to choose phase order, validate reports and native/reward effects,
authorize the concrete commit, and publish the application generation.

This follows the existing split between
[`ExecutionJournal::settle_transaction`](../../rust/crates/rustaxa-evm/src/journal.rs),
[`ConcreteStateWriter`](../../rust/crates/rustaxa-storage/src/concrete_state/writer.rs),
and [`ConcreteStateLifecycle`](../../rust/crates/rustaxa-storage/src/concrete_state/lifecycle.rs).
Do not make storage depend on `rustaxa-evm` or `rustaxa-consensus`. The adapter
translates settled journal facts into narrow storage-domain operations. It must
not compute independent trie roots or supply authoritative storage-root hints.

The selected overlay models the **local active concrete observer** boundary:

1. Execute one transaction against a fixed prepared view.
2. Settle its journal and apply its ordered sink operations in memory.
3. Finish account tries and the main trie; expose the actual next prepared root
   and physical account rows to the next transaction.
4. Retain all resulting rows until the existing approved atomic period commit.

Local [`PrepareIntermediateRoot`](../../submodules/taraxa-evm/taraxa/state/state_transition/state_transition.go)
calls `state.Commit()` and `trie_sink.Commit()` while retaining pending I/O.
[`CommitTransaction` and `Commit`](../../submodules/taraxa-evm/taraxa/state/state_evm/transition_state.go)
together unload all transaction account bodies. The next transaction therefore
starts with fresh ordinary originals and account/code caches, reading the actual
prepared account storage root and latest physical slot bytes. Logs, refund and
transient state reset at the transaction boundary. Native-kernel semantic caches
have their own pinned lifetime; this storage overlay neither resets nor recreates
them implicitly.

## Ordered operations, not cumulative final maps

The current `ConcreteStateMutationBatch` represents one unique account/slot map
against the durable prior. Replaying cumulative maps against that same prior is
insufficient for a general period. Later phases must instead update the preceding
prepared root and its physical overlay. Durable lineage remains the original
committed descriptor throughout this work; an intermediate root is not a new
published generation.

The minimal API shape is an extension of the staged lifecycle, with names still
subject to implementation review:

```text
apply_observer_phase(&mut lifecycle, phase_delta)
    -> Result<latest_preparation_and_exact_changed_accounts>
prepared_view(&lifecycle)
    -> fixed borrowed execution reader for the latest prepared phase
commit_approved(lifecycle, latest_preparation, existing_approval)
    -> existing durable lifecycle observation
```

`phase_delta` contains unique account operations and code insertions plus ordered
slot operations. It does not carry a caller-chosen prior root. The lifecycle uses
its own latest prepared phase, authenticates prior dependencies, and derives all
new roots. Returned account records contain exact physical bytes for report
projection, including explicit deletion facts. The sealed preparation binds the
original durable prior, next period/root, writer, marker and private phase
sequence. The view exposes no writer or commit method.

A minimal proposed phase input contains changed account metadata/deletions, code
insertions, and an **ordered sequence of slot puts/deletes**. The adapter emits
ordinary writes before raw writes, preserving both operations when the same key
appears in both lanes. Within each lane the settled journal already provides its
final per-key operation. Storage need not know which lane produced an operation,
but it must preserve the supplied order and reject invalid operation shapes.
Absent/empty live values remain distinct: an empty value is an explicit delete.

[`TrieSinkAccountMutation.Update`](../../submodules/taraxa-evm/taraxa/state/state_transition/trie_sink.go)
applies every ordinary operation before every raw operation to the same mutable
account trie. [`trie.Writer.Delete`](../../submodules/taraxa-evm/taraxa/trie/writer.go)
writes a physical tombstone only when deletion finds a logical member. A put is
materialized through trie commit; a successful delete can produce a tombstone
even if no final member remains. Thus equal final roots do not imply equal
physical writes.

Required examples:

| Input history | Required outcome | Incorrect shortcut |
| --- | --- | --- |
| Previously missing slot: ordinary `Put(1)`, then raw `Delete`, in one settlement | Empty final logical slot and a physical CF5 tombstone: the delete found the just-inserted member | Collapse to one delete against the initial trie and omit the tombstone |
| Nonmember slot with an orphan physical value: delete only | No new physical tombstone; preserve the previous physical selection | Treat every delete request as a physical deletion |
| Transaction 1 changes a slot; transaction 2 deletes its account | Delete main-trie membership and emit the account CF3 tombstone; retain produced CF5 rows and CF4 nodes | Remove the address from the cumulative slot map or delete all its physical storage |
| An account is deleted and then recreated in a later observer phase | Recreate from an absent account with a nil logical storage root; preserve old physical slot history | Reapply the final upsert against the original durable account and restore its old storage root |
| A later phase overwrites a slot at the same period | Replace that period's final CF5 row while retaining already produced content-addressed nodes | Invent an extra version period for the intermediate phase |

The first example is a source-derived acceptance requirement, not an existing
direct fixture claim. Add an independent pinned case before implementing its
physical-row assertion.

Account deletion is a main-trie operation, not a physical storage range erase.
[`Account.flush`](../../submodules/taraxa-evm/taraxa/state/state_evm/account.go)
omits that transaction's account storage changes when the account is suicided or
modified and EIP-161-empty. The journal must make that decision before handing
the phase to storage. Storage rejects a contradictory same-phase account delete
with new slot writes; it still retains rows already produced by earlier phases.
This contract does not add unsupported SELFDESTRUCT execution.

## Fixed prepared views and original values

After each observer phase, storage provides a borrowed, read-only view of exactly
that prepared state. Advancing the overlay requires exclusive mutation, so the
caller must drop the view and its execution journal first. Reads must not switch
to a moving latest map while an interpreter holds a view.

The view returns:

- Exact account RLP with the storage root derived from the current logical trie,
  plus account absence/tombstone selected for that phase.
- Exact physical slot bytes or tombstones, selected from produced period rows
  first and then the pinned committed reader. Account lifecycle and trie
  membership do not filter this physical lookup.
- Code bytes from immutable staged content first and then the committed store,
  retaining hash and declared-size checks at the existing boundaries.

The current [`ConcreteStateRead`](../../rust/crates/rustaxa-types/src/concrete_state.rs)
explicitly promises committed access and excludes execution overlays. A general
implementation must resolve this contract explicitly, rather than copy the finite
test's fabricated reader identity. Prefer a narrow shared execution-read port
with the same account/slot/code operations and a fixed view identity: existing
committed readers can be adapted into it, while the prepared view implements it
without becoming a public committed-state query surface. Keep concrete storage
and execution independent through the shared domain crate. No second state
manager or general database abstraction is needed.

Retain a private preparation sequence or phase token even when consecutive phases
produce the same period/root. It distinguishes prepared ownership from canonical
state identity. A borrowed view does not itself attest FinalChain approval or
historical coverage.

Within each fresh transaction journal, `original` is the first ordinary committed
value from this fixed view and `current` is the ordinary dirty value. Raw dirty
bytes do not replace the original/current ordinary lane during that transaction.
After observer settlement, a fresh journal reads the new physical winner, including
raw bytes, subject to account/root behavior below. Gas/refund accounting must not
reuse the previous transaction's original cache.

## Orphan slots and unknown history

[`Account.GetRawState` and `get_committed_state`](../../submodules/taraxa-evm/taraxa/state/state_evm/account.go)
make logical account lifetime different from physical slot history:

- An absent account returns no raw value and ordinary zero in execution.
- A newly recreated existing account with a nil storage root ordinarily reads
  zero. Its raw reads can expose old physical slots retained after deletion.
- Once an account has a nonnil storage root, an uncached ordinary read uses the
  physical slot lookup even if that slot is not a member of the current logical
  trie. The existing nil-root/orphan journal fixtures demonstrate this distinction.
- Raw reads do not create a persistent read cache. `RawStorageDirty` is a dirty
  lane and is cleared after an updated account is flushed. A storage overlay must
  not invent a cross-transaction raw cache to mimic native semantic caches.

Preserve exact old CF5 bytes across deletion/recreation. Never synthesize the
storage root from a map of known physical keys: such a map can include orphans
and can omit retained-history gaps. The actual trie determines the account's
logical root; physical history independently determines slot lookup.

Missing entries in the period overlay delegate to the pinned prior reader.
`HistoryUnavailable`, corruption and I/O errors propagate unchanged. A missing
account, nil storage root or authenticated slot nonmembership is **not** authority
to convert missing physical history into `Absent`. The executor may return
ordinary zero without a physical read where the reference's nil-root rule says
so, but raw lookup has no such shortcut for an existing account.

An explicit tombstone produced by a successful delete is known. An absent map
entry means only that this period has not produced a row. The finite fresh S4
test's full-history knowledge cannot be exported as an imported-reader capability.
A general fresh-database absence capability, if later needed, must be separately
designed and validated; this slice must not manufacture retention or bootstrap
authority.

## Physical preparation and publication

Keep one staged period owned by the existing `ConcreteStateLifecycle`, bound to
its exact execution marker and durable prior descriptor. Each successful phase
advances a private sequence and returns its actual root plus exact changed
account rows for projection. It does not write a new durable descriptor.

The in-memory physical overlay accumulates:

| Columns | Accumulation rule |
| --- | --- |
| CF1 code, CF2 main-trie nodes, CF4 account-trie nodes | Union all rows actually produced at observer commits. Equal keys require equal bytes. Preserve nodes from earlier intermediate roots. |
| CF3 account values, CF5 slot values | Preserve the latest actual put/tombstone for each physical `logical-hash || period` key. Account deletion does not erase accumulated CF5 entries. Do not emit a row for a logical nonmember delete. |
| Descriptor, provenance, catalog, execution marker | Retain existing lifecycle ordering and validation; update only in the approved final atomic batch, except the existing durable stage/discard operations. |

Prior content lookup must resolve staged nodes and values before durable rows so
the next phase can authenticate and update its current trie. Reuse existing
physical codecs, path authentication, and `IncrementalTrie` logic; extend their
backing read interface rather than duplicating trie encoding in execution.
Apply one phase transactionally in memory: on validation failure expose no
partially advanced view, or poison the builder and require discard/recovery.

Only the latest preparation from the same writer, staged marker and phase sequence
may become the final prepared commit. Failed/replaced preparation and exact
execution discard must invalidate earlier handles. Preserve the existing
discard/restage regression: preparation from marker A cannot be rebound to marker B.
Intermediate content survives successful phase replacement but is cleared on
discard. Uncertain durable writes retain the existing poisoned-handle recovery
contract.

Final commit remains one synchronous atomic batch of prepared contents,
descriptor, app-approved provenance/catalog, and pending-marker deletion. Existing
FinalChain validation and application publication still own authorization and
cross-database recovery. No extra database generation, column family or version-key
format is required by this proposal.

Retaining intermediate CF2/CF4 nodes does not prove that an arbitrary earlier
phase is queryable after commit. Multiple phases share one period suffix, so
their CF3/CF5 values can be overwritten. Do not advertise historical access to an
intermediate root without its corresponding fixed physical view.

## Public batched behavior remains a separate gate

Public `CommitTransaction` leaves updated accounts in `dirties` until period-end
`Commit`. `Account.flush` clears raw dirty writes and copies ordinary dirty values
into `storage_origin`, but does not replace the cached account's storage-root
field with the sink's eventual root. Existing ordinary originals can therefore
survive between transactions. If `storage_origin` is already allocated,
`get_committed_state` can physically read an uncached slot even with a nil account
root; a fresh observer journal would instead take the nil-root zero shortcut.

Public [`PendingBlockState`](../../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/latest_state.go)
at `6c7e5338` queues writes into the period batch and has no local pending-read
map. The local pin adds a pending-write view. A memory sink that immediately
returns submitted writes is not proof of public RocksDB same-block visibility.
The S4 oracle explicitly compares batched and observer modes only for its finite
ordinary transfer/CREATE/CALL corpus; it does not close raw/native visibility.

Keep public batching support an execution-cache and I/O-lifetime gate. Do not
present this observer overlay, or repeated fresh transaction journals, as a
silent replacement for public-mainnet batching. Native caches and raw overlap
need direct pinned cases with the actual relevant I/O visibility model.

## Smallest implementation acceptance

Extend independent Go evidence before claiming general overlay parity:

1. Put then raw delete of a previously missing slot in one settlement; compare
   exact tombstones as well as final roots.
2. Observer phase writes a slot, later phase deletes the account, and a later
   phase recreates it; compare retained physical orphans, nil-root ordinary reads,
   raw reads, and a subsequent nonnil-root orphan read.
3. Repeated ordinary/raw overlap across observer transactions; compare fresh
   SSTORE originals and physical winners. Record batched differences separately.
4. Missing physical history remains unavailable through a prepared view; explicit
   tombstones remain distinguishable. Use existing synthetic read cases, not an
   imported-state blanket absence adapter.
5. Compare all intermediate/final account rows and roots, final complete CF1–CF5
   maps, stale-token rejection, exact discard, atomic lifecycle publication, and
   close/reopen continuation through the existing FinalChain test composition.

These are bounded targeted gates. They do not imply approval for broad replay,
storage differential validation, a process/disk fault campaign, or production
integration.
