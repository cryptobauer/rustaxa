At baseline `214c87b58`, harden the synthetic N4 fixture checks. Read
AGENTS.md, the final-review limits in
`doc/evm_research/n4_synthetic_native_transition.md`, and only relevant runner,
manifest, and test ranges. Preserve uncommitted preparation and unrelated work.

Use Sol medium to implement directly; do not spawn a same-model implementation
lead for this serial task. Luna may perform one bounded input map at startup.
Freeze source and evidence before independent Sol-medium review. Use Astra only
for an unresolved semantic or authority issue. Check current quota once at
startup: retain the 25% reserve and do not start at 30% or less. Record missing
telemetry as unknown.

Preserve the Go oracle, source pins, and existing frozen outputs as history.
Generate fresh evidence for the changed harness. Make the runner fail unless `witness.ordered_raw_writes`
is empty; and bind every manifest field that drives either engine's setup or
lifecycle in both Rust and Go. Missing, unknown, or discrepant relevant inputs
must fail closed. Add focused negative tests for manifest drift. Keep observed
outputs and expected values independent; do not weaken tests or inject expected
fields into execution. Preserve the existing positive lifecycle and rejection
coverage.

Capture command, exit status, and complete required logs on the first run.
Avoid status polling and repeated mid-work diff reads. Send one blocker handoff
if blocked, then one final handoff. Run targeted Rust/harness checks and
`make rewrite-validate-fast`; run applicable bridge/storage checks and CMake
with 12 jobs when required. Do not repeat passing checks without a failure,
change, or unresolved concern. Honor standing authorization; ask only where
existing policy requires it.

Do not use `data/` or the historical snapshot copy. Do not add production
routing, adoption, replay, or root derivation. Keep producer qualification and
the real-window gate open. After frozen review
and checks, make a local Conventional Commit; do not push. Report changed
paths, assertions, first-run evidence, review, validation, and limits.
