# N4 VRF checkpoint — October 12, second round

This round has accepted 141 new VRF facts. Together with the prior 67, the accepted
total is 208. The first four complete voters each have all 48 VRF periods closed.
No production routing changed. All commits are local; nothing was pushed.

R2 is `/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012`.
P3 is R2/third-voter; P4 is R2/fourth-voter. The prior run is
`/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011`.
Do not rerun any consumed guard. Each family used one exact READY command,
a complete lead replay, an independent byte-identical replay and a reverse-plan
rejection check. All values are Member/Present, 32 bytes and nonzero as a whole.

## Closed dependencies

The offline overlap audit remains closed: the prior 69 unavailable probes are
undelegation-count requests with zero complete tuple overlap with VRF. Shared FE,
period or roots do not prove retention or grant retry authority.

Source commit `d880f6cf835e2384dae1003bfad5af86a78822d4` adds the fixed second
voter adapter and an additive cross-family storage test. Its 47 accepted facts
belong to `264a30e880d3b53bb2cdeb234e34201ebf9a9965`, D25706895..25706941.
R2/family-final-review.md accepts the complete family. Bundle 649774 bytes/SHA256
6c3e64369bd5243b20b315648f263b4db990d266fb5f0630097dfc5a7ada13b9;
replay 59374 bytes/SHA256
0e3b5e18f00209a7799da50565d90bc273ef226c17cc33c5685578255495db42.

Source commit `6321961cea519cf5e33c08344b5e554217c704c2` adds the fixed third
voter adapter. Its 47 accepted facts belong to
`329d40b5f15cfe4de0e9953d0599d2a39a65fc04` over the same delayed periods.
P3/family-final-review.md is 10734 bytes/SHA256
0d495555d02b9164e34b458c4c9cc06b02752585d6c33aa936992265e771420c.
Bundle 658422 bytes/SHA256
b8d2c96ef4e244f07432319998d30c664a116dbbea08ce1bc470687f02626922;
replay 59403 bytes/SHA256
a7f94b240fcdf7293475ae0ec7972fb0d1fb72609703db7080b339066de6c59a.
Outer capture provenance is in each outer-invocation.json. Neither metadata
checks nor capture success alone substitutes for complete proof replay.

Source commit `9ad13da70eb9b0ebb6838e1fde925dabef36623e` adds the fixed fourth
voter adapter. All47 facts for `575910424ac437467726542781508b2220b71ce8` are
accepted in P4/family-final-review.md:11331 bytes/SHA256
4cccaf89987594a22a3d3ce9bb5b1b79793ffedb7516a2463c3439319adcfa8a.
Bundle653017 bytes/SHA256
5ffd64782835a0db3e3937ea7d5292bc0733887c31e90a90371c3af9d48b9e7a;
replay59410 bytes/SHA256
bd099e106c56454bcf94cb6c2fbadbb5d4f14d0acad9b74786d569af75525815.
One28.05-second capture, complete26.08-second lead replay, byte-identical independent
replay and reverse-plan rejection pass. A recorder precheck failed before any child
start; its provenance remains in the final review. No physical or replay retry ran.

All 53 second-profile and 56 third-profile source copies are preserved under
R2/frozen-second-source and R2/frozen-third-source with exact manifests. Use these
copies for historical shared-source pins after the fourth export changes lib.rs.
The old50-source and new59 fourth-source copies and manifests remain intact. Current source
pins and historical copies have distinct purposes; do not rehash stale workspace
paths as if they still represented the old source commit.

## Validation and next slice

Second profile: focused qualifier5, cross-family storage1, full qualifier87,
storage172 including the doc test, Rust-enabled bridge4 and a12-job bridge build
pass. Third profile: focused qualifier6 and full qualifier93 pass. Both profiles
pass fmt/check/clippy/binary build, canonical parity, fast validation and actual
source commit hooks:2130 passes, zero failures and both rewrite guards. The third
profile reuses exact unchanged storage/bridge evidence after hash/cache checks.
Each guard has six synthetic tests. Third CLI rejects19 invalid cases. Original
failed attempts and corrections remain in their logs; accepted runs are separate.

Fourth profile passes focused6/full qualifier99, CLI19, guard6, cap-boundary tests,
quality/build, canonical parity1, fast2130 and actual hook2130/zero failures.
The unchanged storage172/bridge4 evidence is reused only after source/cache checks.
All314 command pins and59 committed source blobs pass independent READY and
postcapture review. All five physical VRF guards across both rounds are consumed.
No guard may be rerun.

Next voter is `6bf44c438f6328640b0bc5d7d7766310cf49610f`, fifth of ten complete
candidates. R2/fifth-voter-preparation-map.json and .md are unreviewed handoff maps:
47 original rows/root joins, prior parent23/selection7/proof58 and59 verified
source copies. The map's saved status predates fourth acceptance. It grants no
authority. Remaining blockers: fresh Astra Keccak logical-key derivation/full-tuple
overlap/raw-proof authority review, fresh Sol design review, fixed source/profile,
required gates/source review, actual local commit hook, full freeze and separate
exact-command READY before any physical metadata/read. Fourth fact acceptance and
59 source copies are now closed prerequisites. No fifth adapter or physical read
was started. Resume needs new quota authorization; preserve this exhausted budget.

The reviewed command-freeze metadata cap is68KiB for the third guard and128KiB
for the fourth; other JSON admission remains64KiB. DB/storage13 limits, stdout64KiB,
result16KiB and300-second timeout remain unchanged. Preserve the cap amendments
and tests. No bounded family may accept a prefix, reset its budget or retry.

## Budget and exact blockers

The user explicitly authorized another10-point round. Fresh baseline55% remaining,
floor45%, stop observation45% at2026-10-12T20:27:21.531Z:10 points consumed. Preserve
R2/quota-budget.json across resume. The prior round's64%/54% file is unchanged.
Final observed allowance, checkpoint HEAD and pending stages are in R2/checkpoint.md
and the quota closeout record. Quota percentage points are not token or billing data.

706 VRF rows and1828 validator/per-voter-jail rows remain missing:2534 total.
Also open:48 delayed lambda rows with complete source/producer/custody/policy
and bounded-export authority;48 exact expected reward-cache RLP rows; producer
and configuration authority; stake/jail, voter weights, reward-cache equality,
owner readiness, public finality and real19-transfer execution. Public roots do
not authenticate auxiliary lambda columns. Do not infer absent values from
current defaults. Diagnostic VRF facts grant no production authority.

Requested Luna, Astra and Sol6.1 medium workers ran; lead Sol6.1 medium was observed.
Child runtime identities and billing remain unknown. Routing and startup evidence
remain in the artifact records. The unrelated workflow, plan, N6, prompt, skill and
three token-audit changes remain outside the owned commits.
