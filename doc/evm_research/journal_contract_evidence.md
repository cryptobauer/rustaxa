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
