# Restored input and next N4 slice — 2026-09-30

This planned slice is now completed within its bounded scope. See the
[execution evidence](n4_restored_snapshot_evidence.md): the independent pair
qualified, reward observations reproduced, and all four scout keys returned
`history_unavailable`. The intake and plan below remain the pre-execution record.

The task owner restored the database under the repository's `data/` directory.
Treat it as supplied evidence, not a writable node database. This supersedes
the earlier missing-input status, but not the requirement to requalify the pair.

## Current paths and intake evidence

| Role | Path relative to repository root |
| --- | --- |
| Supplied root | `data/` |
| Supplied application DB | `data/db/db` |
| Supplied concrete state DB | `data/db/state_db` |
| Planned independent working root | `local/evm-state-db/snapshot-litenode-copy/` |
| Planned application/state copies | `local/evm-state-db/snapshot-litenode-copy/db/{db,state_db}` |
| Local run outputs | `local/evm-state-db/reports/` |

Only filesystem metadata and the two small `CURRENT` text files were inspected;
RocksDB was not opened, a copy was not created, and no database qualification
or diagnostic was run. Neither supplied DB directory is a symlink.

| DB | CURRENT points to | Referenced file present | CURRENT SHA-256 |
| --- | --- | --- | --- |
| Application | `MANIFEST-32593043` | Yes | `d67ac4c21444515e24261cb981fba930915606078e9052a21fec7a8d586b4cae` |
| State | `MANIFEST-997855` | Yes | `8c0d794b338c7cf4ad381040e48b8c639fff7ac033a962400695f264ecd1e71a` |

These filenames and hashes identify the inspected control files, not all DB
contents, chain identity, paired roots or provenance. The recorded head
25,706,949 and prior qualification are comparison targets, not assertions about
the restored pair. Preserve historical manifests and reports unchanged.

The owner's current `.gitignore` addition excludes `data/`; the existing `local`
rule excludes the planned copy and outputs. Databases must not be staged. Select
only reviewed small evidence reports for Git. Workspace placement avoids the old
temporary-path dependency but is not a Git backup of the ignored database.

## Next implementation slice: safe paths, qualification and sender scout

1. Read `AGENTS.md`, `PLAN.md`, the restart/recovery checkpoints and the current
   model table. Confirm Luna first; use Sol and Astra medium by default, escalating
   only when ambiguity or risk warrants it. Keep a single database operator.
2. Update the affected qualifier tools' path policy for the supplied `data/`
   root and the separate `local/` copy. Existing `reward_inputs` and
   `native_inverse_coverage` still hard-code old `/tmp` roots; do not run them
   against supplied data, bypass guards with symlinks, or simply relax the exact
   working-copy constraint. Preserve canonical-path checks, reject source and
   output overlap/escaping children, read-only opens and create-new outputs.
   Add focused rejection tests for direct source input and symlink escapes.
3. Create an independent copy or COW clone of the DB pair without modifying the
   source. Do not use shared writable inodes or repair/prune the source. Record
   copy provenance and requalify genesis/network, application head, concrete
   descriptor, roots and paired boundary using existing Rust owners. Prefer
   bounded point checks; do not rerun a broad inventory solely because old S0
   tooling offers it. Stop dependent probes if identity differs or is incomplete.
4. Once qualified, reproduce the recovered reward diagnostic's seven point reads.
   Compare observations and raw/typed distinctions against the recovered report;
   changed path/source fingerprints alone are expected metadata changes. Retain
   any actual byte mismatch, candidate-derived weights and unqualified producer
   inputs explicitly. Do not turn typed equality into raw-byte parity.
5. Complete the recovered sender helper as a bounded `--head-sender-scout` mode,
   with CLI, identity/provenance validation, rejection tests and a report. Probe
   only sender `35307b7b24fb1473abb364f0c3dd3082b3730cd5`, derived from the
   recorded 19 head transactions, for delegation count, V1 undelegation count,
   V2 validator count and V2 last-ID cursor. Keep physical unavailable/absent/
   tombstone/pruned outcomes distinct and reject malformed widths. Four successful
   logical reads are the bound; do not enumerate children or rerun the full trie
   inventory. Establish provenance from actual source bytes or explicitly cite
   historical evidence rather than claiming fresh signature validation.
6. Run the narrow qualifier tests and applicable repository checks; obtain
   independent review, commit and push source plus small evidence/provenance.
   Update the restart checkpoint with what actually ran and remaining gaps.

The [recovered scout patch](recovery/head_sender_scout.patch) is an inert artifact
with an obsolete absolute path. Use it as reviewed implementation input, not a
blind patch command. The [recovered reward provenance](recovered_reward_inputs_provenance.md)
distinguishes exact historical recovery from fresh validation.

Exit evidence: guarded persistent paths, a qualified independent pair, an honestly
compared reward diagnostic and one four-key scout report with targeted tests and
independent review. This slice does not authorize wider seed expansion, broad
replay/fault campaigns, existing-head adoption, sparse-snapshot publication,
upstream C++ changes or a production backend switch. Milestone 10 remains open.
