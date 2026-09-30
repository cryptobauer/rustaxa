# Recovered reward-input diagnostic provenance

Recovered on 2026-09-30 from historical Codex session records, without opening any database, running state execution, or rerunning the diagnostic. This restores the approved diagnostic artifact; it does not qualify producer reward requests or historical prior-state inputs.

Author session: `/home/fry/.codex/sessions/2026/09/13/rollout-2026-09-13T23-58-47-01a09d35-8cd8-77e3-9a37-b72b5ae7745b.jsonl` (one-based physical line numbers).

- Initial source and one manifest dependency: line 1719.
- Source amendments: lines 1799, 1918, 2053, and 2069.
- Formatting operations recorded at lines 1739, 1934, 2062, and 2076; ordinary Rust formatting reproduced during extraction.
- Final report with recorded SHA-256 and byte count: line 2128. Its JSON bytes were extracted directly, preserving whitespace and trailing newline.
- Historical manifest/lock diff: line 1820. The root agent receives this separately; shared dependency files were not edited by the recovery worker.

The restored `experiments/evm_feasibility/snapshot_qualifier/src/bin/reward_inputs.rs` is 26729 bytes with SHA-256 `a43ff666ffde3ecf4ed56b0bada84a317fc4234510172325069bc4c3a50c2a6e`, exactly matching the source hash embedded in the final historical report. It was reconstructed from recorded patch content and formatting, with no redesign or source substitutions. The matched hash establishes byte-exact recovery of the final source.

The restored `recovered_reward_inputs_candidate.json` is 6337 bytes with SHA-256 `a6ea0da0de5072d8c4372fff51204fe1eefb6628e9ebed8459e749538dfcd443`, exactly matching the recorded final report. This is recovered historical output, not newly generated evidence.

Independent reviewer session `/home/fry/.codex/sessions/2026/09/13/rollout-2026-09-13T23-58-58-01a09d35-b71b-7ca0-8d0a-f846763af66a.jsonl`, line 921, also contains the same report; independent recovery review confirmed both hashes.

The diagnostic retains seven application-only exact-key reads, the fixed qualified-copy path, row length/SHA gates, and the distinction between unequal raw RLP bytes and equal decoded typed fields. It reports historical VRF/prior vote inputs, legacy unordered-map ordering, producer configuration, execution, and reward transition/root as unqualified. The source already contains its historical targeted tests. No recovery piece is missing from the final source or report; runtime snapshot access and fresh diagnostic execution remain outside this recovery task. Current dependency integration and targeted validation are handled by the root agent.

The lead restored the manifest and lockfile dependency delta to match the logged
historical diff exactly: 1 manifest insertion and 57 lockfile insertions, with
no unrelated package version changes. Fresh recovery validation passed the two
binary tests, targeted rustfmt, and targeted clippy with warnings denied for the
diagnostic. The dependency's existing unused-import warning remains unchanged.
No database was opened or historical report regenerated for these checks.
