# Journal and trie-sink contract evidence

The additive `journal_reference.py` corpus executes the actual `TransitionState`,
`TrieSink` and trie writers from both pinned Go references in disposable source
exports. A mutex-protected memory adapter stores their physical column rows.
It does not implement rollback or storage mutation policy itself.

Ten cases cover existing accounts, new empty accounts, new nonempty accounts,
frame revert, raw deletion and 41-byte raw values. Each starts with a complete
synthetic input, writes ordinary slot `1` as `0x33`, then writes the same native
raw slot, changes transient state, adds a log/refund, and optionally reverts.
The artifact records the prior root and rows, intermediate observations, final
root and rows, and a fresh reader after the actual sink commits.

Observed in both references:

- Ordinary and raw reads retain distinct dirty-lane visibility. An existing
  account reverts its ordinary value to `0x11` but retains native `0x0044`.
- After trie commit, raw bytes win the overlapping ordinary mutation. Fresh
  ordinary reads decode `0x0044` as 68, raw deletion as zero, and a wide raw
  value without truncation.
- Reverting a new account removes both dirty lanes. An otherwise empty new
  account is removed on transaction flush even without frame revert.
- Transient writes survive frame revert; transaction completion resets
  transient state, logs and refunds. Ordinary logs/refunds revert with a frame.

TrieSink performs asynchronous work. The exporter observes only transaction-local
resets between `CommitTransaction` and `Commit`; it reads physical account/slot
rows only before submission or after `Commit` and `Close`. It deliberately does
not assert cross-transaction cache visibility from a racy intermediate read.

Run `python3 experiments/evm_feasibility/journal_reference.py` to reproduce;
`--record` regenerates this additive corpus after both references agree.
`fixtures/journal_manifest.json` records exporter/artifact SHA-256, reference
commits and Go version. The original research fixtures are unchanged.

Both references executed and agreed, and a fresh reproduction passed. Independent
review found no blocking oracle issues. This is S1/S3 reference evidence, not
evidence that the new Rust journal already passes. It covers one transaction and
one checkpoint per case, ordinary-then-raw write order only, and memory reopen.
Reverse write order, nested checkpoint stacks, cross-transaction caches, RocksDB
reopen, historical replay and production execution remain separate gates.

The additive `--extended` corpus records ten reverse-order, nested-checkpoint and
nil-storage-root cases. Each mutation/checkpoint/revert includes a reference
observation, followed by reset-only transaction facts and a joined TrieSink
reopen. Nested cases revert the inner checkpoint; they do not prove every nested
commit pattern. An existing account with no storage root can still expose a
retained physical row through raw reads while ordinary committed reads return
zero.

The additive `--mutators` corpus records eight no-op and lifecycle cases. It
confirms byte/root preservation for an equal-value ordinary write over a
leading-zero raw value, empty code assignment, reverted new-account creation and
a rejected decreasing nonce. Dirty existing empty accounts are removed,
including a RIPEMD-address zero-balance touch followed by frame rollback. Only
the deliberate nonce-decrease assertion may panic; other panics fail export.
These are real reference account methods and trie writers, not replacement
journal implementations.

Both pinned revisions agree byte-for-byte on the new corpora. The original ten
fixture outputs remain unchanged; their manifest's exporter hash tracks the
expanded source. These fixtures extend the execution-worker regression targets;
they do not establish same-block asynchronous visibility, persisted Rust roots,
RocksDB reopening or complete frame/opcode parity.

```bash
python3 experiments/evm_feasibility/journal_reference.py --extended
python3 experiments/evm_feasibility/journal_reference.py --mutators
```
