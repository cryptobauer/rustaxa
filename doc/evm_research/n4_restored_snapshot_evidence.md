# Restored N4 working pair and bounded probes — 2026-09-30

This slice continues from `5aa3febfee14a3ad59c2b24c15d94a49013fd170` on
`feat/rust/evm-state-db`. It preserves the supplied `data/` tree and uses the
independent `local/evm-state-db/snapshot-litenode-copy/` pair. Milestone 10
remains open; this is neither existing-head adoption nor replay acceptance.

## Copy provenance

The lead was the sole database operator. Before any RocksDB open, the pair was
copied with `cp -a --reflink=never data local/evm-state-db/snapshot-litenode-copy`.
A filesystem check compared every regular file's device/inode identity and size:
1,100,450 files, no shared source/copy inodes, no symlinks and no file with a
link count above one. The supplied tree was never opened with RocksDB.

[Compact copy provenance](n4_restored_copy_provenance.json) records the checks.
Full content manifests were generated with the existing
`experiments/evm_feasibility/snapshot_manifest.py`; the large JSONL files stay in
ignored `local/evm-state-db/reports/`. The source contains 9,826,980,174 logical
bytes, with manifest SHA-256
`d6f7d58c7e9ff8d7d734ea5104fc2bdd593cbfbcecf949f1ccec7a5ec259fccb`, identical
to the historical S0 content digest. A second full-content comparison after the
probes found the same source and copy digests; both trees remain byte-identical
to their pre-open contents. This is byte-content evidence, not proof of
the producer binary or exact checkpoint capture timing.

The owner's existing `data/` ignore addition is included unchanged. The existing `local`
ignore rule covers the copy and full local output. Neither database is staged.

## Guarded tool contract

The qualifier tools use an exact repository-local working-copy constraint,
reject the supplied source and symlink aliases/escaping children, open RocksDB
read-only, and create reports exclusively at new paths outside both trees.
A separate bounded qualification command avoids the original S0 inventory scan.
The scout branches before native inventory enumeration and probes only the fixed
sender's four keys. It cites the recovered historical sender evidence and does
not claim fresh signature validation.

## Fresh qualification and reward diagnostic

[Bounded pair qualification](n4_restored_pair_qualification.json) passed using
four application point reads through Rust repositories and the existing
`ConcreteCheckpointReaders` descriptor/current-and-prior root checks. Genesis
matches the recorded mainnet identity; the head is 25,706,949, state root
`b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`.
The prior identity is 25,706,948, root
`926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`.
This does not establish full trie closure or producer/capture provenance.

The [fresh seven-read reward report](n4_restored_reward_inputs.json) reproduces
all observations and qualifications from the recovered historical output.
The [comparison](n4_restored_reward_comparison.json) finds only `input_copy`
and `tool_source_sha256` changed. Typed distributions still match; serialized
RLP still differs because validator order differs. The retained row SHA is
`18bb350916c3d85a52ec401cb2b898d2696b57e3859d5ff9dda1218a8ed05070`, while
the candidate SHA is `85e2aacceab96cfd4c5fe92773540106761af6f2939da1c70161b40bb99c07a9`.
Vote weights and blocks-per-year remain derived from expected retained output;
this circular comparison does not independently qualify prior weights, VRF inputs,
producer configuration, reward execution or roots. The historical report stays
unchanged.

## Four-key sender scout

The [fresh scout report](n4_head_sender_scout.json) probes only sender
`35307b7b24fb1473abb364f0c3dd3082b3730cd5` at H. The CLI first validates the
embedded historical preflight artifact's SHA, period, count, positions and all
19 sender fields. It explicitly does not repeat signature validation.

| Key family | Fresh physical outcome | Decoded value |
| --- | --- | --- |
| Delegation validator count | `history_unavailable` | Unknown |
| V1 undelegation validator count | `history_unavailable` | Unknown |
| V2 undelegation validator count | `history_unavailable` | Unknown |
| V2 last-ID cursor | `history_unavailable` | Unknown |

There were exactly four attempts and zero successful physical values/absence/
tombstone results. Missing physical history is not promoted to zero, logical
absence or pruning. The scout did not enumerate children, verify membership for
these keys or refresh the full native inventory. The previous 2,282/23,278
coverage and 20,996 unexplained rows remain historical and unchanged.
The scout was repeated once after a final swapped-width rejection test changed
its embedded source fingerprint; each execution attempted the same four keys,
and only the fingerprint differs between the two reports. The committed report
binds the final source; both exclusive local outputs are retained.
This completes the bounded scout; it does not establish that the sender has no
delegations or undelegations. A future separately scoped point-proof slice could
investigate membership/nonmembership for these same keys before seed expansion.

## Validation scope

Tier 1 applies: these are isolated qualification/diagnostic tools, with no
production routing, storage-library, C++ shim or bridge change. The lead explicitly
ran `.githooks/pre-commit`, which passed the workspace fmt/clippy/tests, storage
boundary and bridge inventory guards, and whitespace checks. The log contains
1,887 passing tests across 47 result summaries (including 1,441 consensus tests).
Existing dependency clippy warnings remain; no warning policy was weakened.
No CMake or storage differential gate was required or run for this tooling slice.

The qualifier's locked package tests passed all 28 cases (8 library, 12 inverse
including the scout/provenance tests, 8 other binary tests). The final swapped
count/cursor-width rejection refinement passed the three scout tests again.
Qualifier formatting and `cargo clippy --locked --manifest-path
experiments/evm_feasibility/snapshot_qualifier/Cargo.toml --all-targets --no-deps
-- -D warnings` passed. Focused tests cover source/symlink/output rejection,
identity drift, the fixed sender's independently checked four key hashes,
provenance drift, four-read bounds, malformed widths and distinct physical outcomes.

All three database commands used `cargo run --locked` with the qualifier
manifest, in this order: `bounded_qualification`, `reward_inputs`, then
`native_inverse_coverage -- --head-sender-scout`. Exact arguments are in the
[tool README](../../experiments/evm_feasibility/snapshot_qualifier/README.md).
The final scout output uses `head-sender-scout-final.json` instead of the README's
initial filename; existing outputs were never overwritten by the tools.

## Independent review and routing

Luna (`gpt-6-luna`, medium) started and acknowledged before Sol implementation
(`gpt-6.1-sol`, medium) and independent Astra review (`gpt-6-astra`, medium).
All ran successfully with runtime metadata confirmation; no routing failure or
reasoning escalation occurred. See [the handoff ledger](n6_agent_handoffs.md).

Independent review approved the final source and small reports after correcting
the provenance citation, adding explicit physical-membership/authority limits,
validating the embedded historical artifact, independently checking golden keys,
and rebuilding the scout after its final test-only source hash changed.
Approval covers this bounded evidence slice only. The reviewer noted shallow
new qualification rejection tests as nonblocking because the existing checkpoint
owner supplies descriptor/root validation and its own tests remain unchanged.

## Remaining boundary

Producer configuration and binary identity, independent prior-state reward
weights and VRF inputs, global native/catalog completeness, slashing reconstruction,
existing-head adoption and full-period replay/recovery remain open. No production
routing, original C++ implementation, storage implementation, broad inventory or
broad replay changed in this slice.
