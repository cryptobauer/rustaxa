# Model and environment validation — 2026-09-30

The current model routes work. The milestone's ephemeral execution environment
needs recovery before snapshot-dependent work resumes. This is setup validation,
not renewed EVM parity or milestone acceptance.

## Verified model routing

At the start of this check, the agent service listed only the root. The six
historical workers mentioned in the conversation were not live. A new Luna was
started and acknowledged before Sol and Astra were allocated.

| Thread | Requested model / reasoning | Observed runtime | Bounded result |
| --- | --- | --- | --- |
| `luna_setup_validation` | `gpt-6-luna`, medium | Same, confirmed from `turn_context` | Read-only stale assignment/checkpoint audit |
| `sol_setup_validation` | `gpt-6.1-sol`, high | Same, confirmed from `turn_context` | Branch, worktree and interrupted-artifact audit |
| `astra_setup_review` | `gpt-6-astra`, high | Same, confirmed from `turn_context` | Independent assignment/ownership review |
| Current root | Existing session | `gpt-6-astra`, medium in latest `turn_context` | Setup coordination and documentation |

Local session IDs for the three startup checks respectively:
`01a0f09b-fa44-76a0-882e-c322388fee76`,
`01a0f09c-577b-7ab1-9e9e-b1e9ef726949`,
`01a0f09c-7ac4-7df0-867d-ac3f300a33e8`.
Runtime metadata, rather than an agent's self-description, establishes these IDs.

All three routes started and completed read-only work without a routing or usage
failure. September 14 usage-limit errors remain historical. No account-wide
quota balance, billing consumption or sustained availability was inferred.
Luna high is a prospective assignment, not exercised by this medium-only check.
No model-equivalence or coding-quality benchmark was run.

## Routing policy

The [current assignment table](08_implementation_plan.md#agent-and-model-assignments)
assigns Sol to implementation, Astra to contracts and independent semantic
review, and Luna to bounded helper work. At the task owner's request, Sol and
Astra now default to medium, including routine review, with high reserved for
ambiguous or high-risk work under the 08 escalation criteria. The startup table
above preserves the actual high-effort checks; it is not rewritten as medium
validation. No medium/high equivalence benchmark has been run. The current
medium-reasoning root coordinates integration; it does not claim to have switched
itself to high reasoning. High-risk decisions go to an explicitly configured
Astra high worker, with a separate reviewer if that worker implements the change.

This is a project-specific choice informed by official guidance:
[GPT-6.1 Sol](https://developers.openai.com/api/docs/models/gpt-6.1-sol)
targets complex coding at lower cost than Astra;
[GPT-6 Astra](https://developers.openai.com/api/docs/models/gpt-6-astra)
targets the most demanding reasoning;
[GPT-6 Luna](https://developers.openai.com/api/docs/models/gpt-6-luna)
targets focused, efficient work. These descriptions do not replace reference
parity tests or establish account access; the actual startup checks above test
routing in this session.

The available tool schema exposes all three IDs. It also still exposes older
models and fixed specialist roles: reviewer/architect/API roles pin 5.6 Sol,
while Rust/code-mapper/C++ specialist roles pin Spark. Use `default` or `worker`
with explicit `model`, `reasoning_effort` and independent context for current
routing. Full-history forks cannot carry model overrides in this interface.
Follow-up tasks do not offer a model-change parameter; do not assume reuse
upgrades a thread. No global configuration was changed. The local default was
observed as Astra medium, consistent with the current root runtime.

Keep one Luna slot reserved and at most two simultaneous implementation writers
with disjoint ownership. Add independent review when needed; do not fill all
seven slots merely because they exist. Shared contracts/manifests/lockfiles and
`PLAN.md` remain lead-owned. Only the lead operates `/build`; database ownership
must be coordinated. Preserve snapshot, test and publication authority rules.

## Current repository and environment

Before these documentation changes, the checkout was clean at
`78dc79054a0b65a000bdd1666fd106b9329c41b6` on `feat/rust/evm-state-db`, matching
the local origin tracking ref. This is a local comparison, not a remote fetch.

Historical `/tmp` worktree directories are absent; their Git entries are marked
prunable. They were not pruned. Surviving refs include:

- `task/n4-native-inverse-coverage` at `e3e586ed3`: committed extension, already
  integrated as `87479197e`.
- `task/n4-reward-inputs` at `5c74ac092`: baseline only, not a saved extractor
  implementation. It has no unique commits relative to the feature branch.

The original `/tmp/snapshot-litenode` and qualified independent copy
`/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy` are absent. No database
was opened. Do not substitute another snapshot without requalification.
The previous `/tmp` diagnostic reports are not available. `/build/CMakeCache.txt`
exists; this check did not rebuild or certify the build tree.

The last conversation recorded a reviewed reward diagnostic with typed equality
but differing serialized validator order. That is historical evidence, not a
currently reproducible artifact: recover the exact source/report from session
records if available, or reconstruct and rerun after input restoration. The
four-key copied-head sender scout has no currently verified completion artifact.

## Next assignments

1. Sol medium by default: recover the interrupted reward extractor and scout source from
   saved evidence; inventory exactly what survives before editing. Keep prior
   raw mismatch and candidate-only qualification explicit.
2. Astra medium by default: independently review reconstructed source and provenance.
   Missing reports or snapshot inputs cannot be replaced by conversation claims.
   Escalate to high if reconstruction leaves provenance or semantic ambiguity.
3. Luna medium: check links, hashes, fixture inventories and handoff completeness.
4. Lead: coordinate restoration/requalification of supplied inputs, then run
   only the applicable bounded checks under the existing repository policy.

No implementation, probe, database mutation or production route was changed
during this setup audit. Existing test results remain historical; documentation
checks and successful model-routing smoke tasks are the validation for this slice.
