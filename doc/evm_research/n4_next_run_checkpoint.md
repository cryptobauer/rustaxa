# N4 VRF checkpoint — October 13, third round

This round accepted 94 new VRF facts: the fifth and sixth fixed 47-row families.
The total is 302. Six complete voters each have all 48 VRF periods closed.
All source commits are local. Nothing was pushed. Production routing is unchanged.

R3 is `/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013`. P5 and P6 are its fifth-voter and sixth-voter directories.
Prior accepted history is in `/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012`
(R2), including its checkpoint, immutable source copies and final acceptance.
The first-period 20 and prior four complete voter families remain accepted.

## Closed dependencies

The offline overlap audit remains closed. The 69 unavailable probes are
undelegation-count tuples, with zero complete tuple overlap with VRF. Shared roots,
FE account or periods do not prove retention and do not authorize retries.

Fifth source commit `b8673ce2af85d866f2b72bb80094154fe0ea7baa` adds the fixed voter
`6bf44c438f6328640b0bc5d7d7766310cf49610f`, prior selection7/proof58.
P5/family-final-review.md accepts all 47 D25706895..25706941 facts.
Bundle: 677739 bytes, SHA256 aef483116caabce7b5080bcbf2a7a67db51cdbcdf2465b1024cced711f8a30a7.
Replay: 59403 bytes, SHA256 9f3124ada4f7c88dd2c8037ec0ac7c7e87907bcea61d683df8746054d9f39fce.

Sixth source commit `42ed7ac9ad9f01d046ef400f6a013791ed22ede6` adds the fixed voter
`a6cec53b4d7920709f05ca1f7ac0d66338d29cb9`, prior selection10/proof61.
P6/family-final-review.md accepts all 47 facts over the same delayed periods.
Bundle: 678585 bytes, SHA256 7b270eb9a7bd023d66a74e7f3cf775e07321ad5361db08ed083756f54cf04d5b.
Replay: 59404 bytes, SHA256 35c05787ece4bb2b38a3cbe2fb3137019cbb34d79fd24c9406b857821d5fe187.

Each family has independent exact-command READY, one guarded capture, a complete
lead replay, an independent byte-identical replay and an old-plan rejection at
Identity/Payload/index0 with PayloadTuples used0. Values are Member/Present,
32 bytes and nonzero as a whole. Outer invocation records retain actual tool exits.
All seven physical VRF guards across the three rounds are consumed. Never rerun them.
Metadata equality is narrower than complete database byte preservation.

All 62 fifth and 65 sixth source copies are preserved in R3/frozen-fifth-source
and R3/frozen-sixth-source, with exact manifests. Historical R2 copies remain intact.
Use archived copies for superseded shared-source hashes. Do not rehash stale workspace
paths as if they represented an earlier source commit.

## Validation and reviewed bounds

Both profiles pass focused6, full qualifier105/111, canonical parity1, CLI19,
guard6, cap-boundary and historical admission controls, fmt/check/clippy and builds.
Sixth review required an additional actual all-bin build; it passed and is frozen.
Each fast gate and actual source commit hook passed 2130 tests with zero failures.
Storage172/bridge4 evidence is reused after exact checks of 52 unchanged R2 pins,
58/61 unchanged parent pins and the Rust-enabled CMake cache. No storage/C++ changed.

Command-freeze admission remains128KiB; normal metadata admission remains64KiB.
One immutable R2 startup map has an exact87841-byte/SHA-before-parse exception,
with growth, short, cap+1 and same-size hash-drift rejection tests. This exception
does not raise other metadata caps. All13 witness ceilings, 47 payloads/98 proofs,
raw7 authentication and full71-prior-proof barrier remain unchanged. Sink64KiB,
result16KiB and300-second timeout remain fixed. No prefix acceptance or retry occurred.
The fifth builder's pre-publication missing-path failure and its correction remain
recorded. A sixth helper's reverse-profile/PATH and closed-pipe wait defects were fixed before
execution. Its actual hanging-child regression confirms deadline termination.
A later offline helper tuple-shape precheck failed before any replay child started;
the frozen helper remained intact and a separately reviewed corrected helper was used.

## Exact remaining blockers

612 VRF rows plus1828 stake/per-voter-jail rows remain:2440 voter rows.
Also open: lambda48 with producer/source/custody/policy/bounded-export provenance;
exact raw expected reward-cache RLP48; producer and configuration authority;
weights, cache equality, owner readiness, public finality and real19-transfer execution.
Public roots do not authenticate auxiliary lambda columns. No default is substituted.
Diagnostic VRF facts do not grant production authority.

Next candidate is `c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa`, seventh of ten
complete voters. Fresh Astra and Sol conditional design records are in R3. A syntax-only guard/test
preparation has independent conditional review; no seventh guard or test ran.
First-D parent41/selection13/proof64; logical key
`fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f`.
Sixth acceptance and immutable65 source copies are now closed dependencies.
Seventh implementation, required gates, final source review/local hook, complete
command freeze and independent READY remain open. No seventh source or read started.

## Budget and handoff

The third authorized round preserves baseline45% and floor35% in R3/quota-budget.json.
R3/final-quota.json and checkpoint.md own the final observed allowance and HEAD.
Previous R2 55/45 and original64/54 budget files remain unchanged. Never reset a saved
budget on resume. At the floor, stop new work/delegation/gates and finish only atomic
in-flight work and minimal closeout. Further work needs new quota authorization.
Quota percentage points do not measure task tokens or billing.

Luna bounded helpers and fresh Astra/Sol profile reviewers ran. Requested models,
startup and results are in R3/routing-round3.json; lead Sol6.1 medium was observed.
Child runtime identity and billing remain unknown; no routing rejection occurred.
All eight unrelated user-file pins are preserved and excluded from owned commits.
