# Interrupted-work recovery — 2026-09-30

Recovery restores the logged reward diagnostic and preserves the unfinished
sender-scout patch. No database was opened, snapshot reconstructed, diagnostic
rerun or new semantic implementation invented.

## Later input restoration

After this recovery, the task owner restored the database under `data/`.
The [new intake and next-slice contract](n4_restored_snapshot_next_slice.md)
supersedes the missing-input status below. Only filesystem checks have run;
the restored pair still needs an independent copy and fresh qualification.

## Recovered artifacts

| Item | Recovery result | Evidence |
| --- | --- | --- |
| Final `reward_inputs.rs` | Byte-exact source, 26,729 bytes | SHA-256 `a43ff666ffde3ecf4ed56b0bada84a317fc4234510172325069bc4c3a50c2a6e` matches historical report |
| Reward candidate JSON | Byte-exact historical output, 6,337 bytes | SHA-256 `a6ea0da0de5072d8c4372fff51204fe1eefb6628e9ebed8459e749538dfcd443` matches independent reviewer log |
| Manifest and lockfile | Exact historical dependency diff | 1 manifest insertion and 57 lock insertions; no unrelated package version changes |
| Copied-head sender scout | Exact logged helper patch plus documented terminal newline | No CLI, rejection tests, report or completed probe was recorded |

The reward source is restored at
[`reward_inputs.rs`](../../experiments/evm_feasibility/snapshot_qualifier/src/bin/reward_inputs.rs).
Its [historical report](recovered_reward_inputs_candidate.json) and
[recovery provenance](recovered_reward_inputs_provenance.md) retain the original
qualification limits: typed candidate equality, unequal serialized bytes, and
unqualified producer inputs, prior voting weights, VRF validity, execution and
roots. Recovery does not convert that diagnostic into parity acceptance.

The [sender-scout provenance](recovered_sender_scout.md) links the inert patch
artifact. The integrated inverse binary and compiled helper remain unchanged.
Finishing the scout would be new implementation with its own review and tests.

## Search and verification

Author and reviewer session records supplied source patches, formatting steps,
the final full JSON report and hashes. Independent recovery review checked those
records and the restored bytes. The session paths and physical line numbers are
recorded in the per-artifact provenance documents.

Git fsck found 21 unreachable commits and 29 unreachable blobs. The commits
predate the interrupted September 14 reward/scout work, and no unreachable blob
matched the targeted missing reward/scout content. Surviving worktree indices
did not contain either missing implementation. No Git pruning or garbage
collection was performed.

A bounded filename-only search under `/workspaces`, `/build` and `/home/fry`
(depth 8, excluding Git/build-output dependency trees) found no likely snapshot
database/archive. A shallow check of `/mnt`, `/media` and `/opt` also found none.
This is not proof of permanent loss or an exhaustive external-backup search.
The original and qualified-copy paths remain absent; session logs cannot recover
the complete snapshot databases. Restore supplied inputs and requalify an
independent copy before any database-dependent continuation.

## Fresh validation

The lead ran:

- Locked `cargo test --bin reward_inputs`: 2/2 passed.
- Targeted rustfmt and locked clippy with warnings denied for the diagnostic:
  passed; the existing consensus dependency unused-import warning remains.
- Explicit `.githooks/pre-commit`: passed formatting, clippy, workspace tests
  including 1,441 consensus tests, structural guards and whitespace checks.
- Exact source/report/dependency-diff comparisons and independent fidelity review:
  passed.

The two qualifier commands use
`--manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml`.
No C++/production/storage implementation changed, and no CMake or database gate
was rerun. Historical report recovery and fresh unit validation are separate.

## Team and next boundary

Luna medium resumed first for bounded session mapping and filename discovery.
Two fresh GPT-6.1 Sol medium workers recovered reward and scout artifacts with
disjoint ownership. A fresh GPT-6 Astra medium reviewer independently checked
fidelity. All actual model/effort pairs were confirmed from runtime metadata;
no routing failure occurred. These are recovery results, not a general benchmark
of medium versus high reasoning.

All recoverable source/report content identified in the targeted session records
is now preserved in Git. The remaining obstacle to rerunning the reward probe is
the missing snapshot input; the sender scout also needs its never-completed
implementation. Milestone 10 remains open.
