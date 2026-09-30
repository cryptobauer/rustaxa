# Recovered unfinished head sender scout

Recovery preserves the exact logged helper patch as
[`recovery/head_sender_scout.patch`](recovery/head_sender_scout.patch).
The executable and its compiled helper remain at the integrated baseline.
This is a partial source recovery, not a runnable scout mode or a scout result.

## Provenance

Historical author session:
`/home/fry/.codex/sessions/2026/09/13/rollout-2026-09-13T23-58-26-01a09d35-3bb8-7592-a598-bda6d53dedf2.jsonl`.
SHA-256: `63a9c1b3be236acc9f6aa7908ac4d5eca709967d8c287e4f4f2439d6697d4719`.

Historical reviewer session:
`/home/fry/.codex/sessions/2026/09/13/rollout-2026-09-13T23-58-14-01a09d35-0cf1-7212-aa91-09cce354e2ab.jsonl`.
SHA-256: `3c417cd0d7b9c59d9de9d16a5f8c5d4046d03f19548542daa80823a6bb70d04f`.

Line numbers below count physical JSONL records, starting at one. Author line
1511 records clean baseline `e3e586ed3b49fcaf5b71d46d3ec80e0935748264`, whose
committed implementation was already integrated as `87479197e`. Author line
1522 (2026-09-14 01:40:32.265 UTC) contains the subsequent helper patch;
line 1525 records its successful application. Line 1550 records only
`seeded_undelegations.rs` modified, while the printed executable still contains
the inventory enumeration path and no scout mode.

The artifact is the JSON-decoded `const patch` string from line 1522, plus one
terminal newline. Its original historical absolute path is preserved; it has
not been retargeted for direct application to this checkout. SHA-256:
`99f79d37acf48d8042643013a0152166373ea1adc30b408b62052984a2b7cfa9`.

## Recovered implementation and missing work

The patch adds `SeededAddressScoutObservation`, `scout_seeded_address`, and a
delegation-validator prefix helper. A successful helper call performs four
reads: delegation validator count, V1 undelegation validator count, V2
undelegation validator count, and V2 last ID. Present counts require exactly
four little-endian bytes; a present last ID requires exactly eight. Each
observation retains its logical key, physical result, exact present bytes, and
decoded value. Absent, tombstone, history-unavailable, and pruned results remain
distinct. Other reader errors and invalid widths fail; failure may stop before
all four reads. The helper does not enumerate children or authenticate live
inventory membership. Its caller must provide the fixed identity and account.

Author line 1471 identifies candidate
`35307b7b24fb1473abb364f0c3dd3082b3730cd5` as the common sender of 19
authenticated head transactions, absent from the previous 290 seeds. This is
historical evidence, not a fresh corpus or database verification.

Author line 1487 proposes `--head-sender-scout`, canonical-copy and read-only
guards, exact FinalChain head/header identity checks, validation of the embedded
signed-call corpus, an early return before inventory enumeration, and a report
denying catalog, snapshot, adoption, publication, and routing authority. Line
1546 says the mode and rejection tests are being finished. Neither executable
wiring, corpus validation, identity/report implementation, nor rejection tests
are present in a subsequent logged patch. Reviewer line 745 announces a future
review of the scout and saved report; it is not an acceptance or result.

No saved four-query report or actual scout observations were recovered. The
proposed command is historical planning only and has not been executed during
recovery. No database or full inventory was read, and no probe was resumed.

## Recovery validation

The extracted artifact was checked against the exact logged patch string. The
temporarily applied helper addition was removed in favor of the inert artifact;
the helper has no remaining diff from HEAD. No manifest or lockfile changes are
part of this recovery. Runnable source remains the integrated baseline, so
scout-specific compilation, tests, and execution readiness are unproven. No
new CLI or tests have been invented to complete the historical plan.
