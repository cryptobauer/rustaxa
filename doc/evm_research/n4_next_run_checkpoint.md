# N4 next run checkpoint — October 11

Branch `feat/rust/evm-state-db`. Accepted implementation HEAD:
`de81438f478fe34b379d5eb34801934e3a500604`. Five reviewed local slices
completed in this round. Nothing was pushed. No owned implementation, probe,
validation process or final implementation review is pending.

The new user-authorized ten-point round has reached its floor. Reuse its saved
budget without initialization for closeout only; do not start implementation
under this exhausted budget. A later round needs explicit authorization and a
fresh baseline. All earlier budgets remain immutable. This checkpoint supersedes
old P0 next-action text in the branch queue and earlier restart records.

## Accepted work

| Commit | Result |
| --- | --- |
| `67b7ee5de6636b04b03feb15ab56aa6e28cd4093` | Validator/owner seed audit: 290 addresses, 870 candidate tuples, 60 exact prior exclusions; 70 raw-live scalar slots and 800 not-live-only residual tuples. No numeric or physical inference. |
| `604664af8ecdee6c25eb1c24c2f42865b5e1724a` | Offline undelegation family: 1017 tuples, 217 paths/2277 bytes, 45 V1 and two V2 record objects, 800 unresolved tuples. Family acceptance alone did not qualify a union. |
| `55da44664fbd456a9bf959f9ccd68dac5877d0c8` | Parent reward field manifest: 48 periods, 914 certificates, 139 DAG blocks and 583 transactions. Raw difficulty and ordered fee inputs; missing semantic inputs remain explicit. |
| `e2a5931e7f0bd7f1532667b02d04a832065fc0f4` | Pure canonical base row codecs/keys, with a first 4096-byte guard. No physical analyzer behavior or fact format changed. |
| `de81438f478fe34b379d5eb34801934e3a500604` | Exact saved path union: base 2065/35694 bytes, undelegations 217/2277 bytes, actual zero overlap; union 2282/37971 bytes, residual 20996/221106 bytes. All partition digests verified. |

The union result comes from actual membership and typed canonical raw equality,
not addition alone. Raw residual bytes remain present. Full logical/semantic
catalog, global delegators, historical owner/preservation, reward transition,
adoption and production authority remain unqualified. No writable owner, real
one-period transition, source-data mutation or new physical scan/read occurred.

Documents: [seed audit](n4_validator_owner_seed_audit.md),
[offline family](n4_offline_undelegation_catalog.md),
[reward fields](n4_parent_reward_field_manifest.md),
[base codecs](n4_base_codec_preparation.md), [exact union](n4_base_path_union.md).
The [scorecard](../codex_slice_scorecard.md) records corrections and validation.

## Input and authority limits

H=25706949, P=25706948, E=25706944 and H certificate D=25706942 are distinct.
Complete live native inventory at P/E/D is retained and authenticated, but does
not establish all-ever keys, deleted history or a complete model. Slashing still
has 33 opaque live rows; proof-key discovery needs authenticated paired votes
or successful commitDoubleVotingProof calldata with ordered vote hashes.
Non-head reward nodes need justified historical head/cursor references.

Owner directs default mainnet and reports likely v1.14.1. Public EVM revision
6c7e5338b22d5e596cc2365a88d1f94840e1ee1b and local Go oracle revision
bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418 remain distinct. Producer identity
is unverified. Keep that reference assumption in future execution claims.

The candidate parent cache has 48 retained periods 901..948. Available signed
DAG/certificate/fee fields are authenticated, but seven delayed headers/roots at
894..900 remain outside the accepted interval. Also unqualified: 2742 symbolic
native voter facts, 48 total-vote and 48 jailed-list facts, vote weights, lambda
policy/provenance, difficulty reward policy and independently derived cache
values. Missing/tombstone/unavailable observations never supply zero.

Luna's exact subtraction finds 56 distinct retained undelegation child addresses,
all inside the accepted 290 seeds. No new address source is supplied by those
47 child objects. The source-only map grants no completeness or physical status.

## Next executable slice

Fresh Astra medium wrote `delayed-header-contract.md`, SHA
3dfc397f3e1348cb15302c5a4ec995445abf41f8572f9004510395e2aac2e2ff,
in the current run directory. Fresh Sol medium contract review is READY with no finding;
`delayed-header-contract-review.md` records the independent source review. No header source
implementation or probe exists. This is a source plan, not record availability.

The plan revalidates the pinned accepted interval offline, creates a distinct
sealed header-only context from the verified 901 boundary, and reads exactly
seven headers plus seven PeriodData rows at 900..894. Three copy identity guards
make 17 total point reads. No indexes, receipts, state reads or inventory. It
keeps the existing full-list901 endpoint intact and cannot import expected roots.
New returned-value cap 3932200 bytes; header/PeriodData row cap 262144 each.
The contract defines exact canonical/hash-domain checks, failure stops, source
pins, exclusive output and focused tests. Availability of the 14 records is
unknown until that bounded read-only probe.

After a newly authorized allowance round: read the final contract review,
resolve any named finding, implement the fixed verifier/adapter, run focused
checks, then execute one guarded read-only probe through the copy. Replay its
captured bytes offline, freeze and obtain fresh independent final acceptance.
Headers commit roots only; they do not prove native values, weights, cache,
owner readiness or independent public finality. Broader owner and 19-transfer
reward work remains downstream of complete reviewed dependencies.

## Evidence and allowance

Current run: `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2`.
Detailed accepted records: validator-seed-accepted.json,
offline-undelegation-accepted.json, parent-cache-fields-accepted.json,
base-codecs-accepted.json and base-path-union-accepted.json. Each links full
first/corrected gate logs, source/binary/report freezes and independent reviews.

Final reward field report: parent-cache-fields-cap-tests.json,
2257600 bytes, SHA b54906443d6504bd1ad59d2208791d4cd710ffe2ae4a0731f8306af7b5521a3b.
Final union report: base-path-union-guards.json, 2263634 bytes,
SHA 92da8fdde66bce7e9bab9505d5426f2dfcf068a985bec1256c8d231492358c25.
The exact base/raw/catalog/partition and missing-field digests are in those reports.
Prior October10 history and exhausted 53-to43 budget remain preserved in the
prior run and current `previous-n4-next-run-checkpoint.md`.

Lead thread 01a126b2-3ae0-7883-a5e3-b4618ade8cd5 is confirmed Sol medium.
Luna medium started first. Fresh Astra/Sol medium contexts were used for new
profiles. runtime-routing.json records requested and actual models, exact thread
IDs, results and no routing failure. No billing or token claim follows routing.

Baseline 100% at 2026-10-11T11:30:21.257Z; floor 90%; reset 1792322995.
Floor reached at 2026-10-11T13:03:14.058Z: 10 account-wide percentage points used.
Final observation is in quota-final.json. This budget was initialized once and
preserved across resume; it was not reset or transferred. Only the in-flight
source contract review finished after the floor; only minimal checkpoint closeout
continued.

Preserve unrelated initial edits: doc/codex_slice_workflow.md,
n6_remaining_branch_queue.md, n6_restart_checkpoint.md, .agents/ and the three
token_usage_audit files. Final owned documentation closeout is separate from
those changes. No production/storage-module change occurred; storage bridge,
node and broad differential gates did not apply.

Contract source work is complete and READY. No owned implementation, gate, review
or process remains active. The seven-header implementation is deferred because
of the exhausted allowance, not a source authority blocker.
