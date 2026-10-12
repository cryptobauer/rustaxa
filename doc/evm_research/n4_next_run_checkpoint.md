# N4 VRF checkpoint — October 12

Branch `feat/rust/evm-state-db`. Startup base `3e070d0c4ba77db1b0952a192129b85cd3bccfe5`.
Accepted source commits: first-period adapter `b707c177a888905f95b2bb2850f98e0874340c91`;
remaining-voter adapter `63166711047b3dcdd2bd282fb3f5e103df8c16b8`.
Local commits only; no push. Current run artifacts (R):
`/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011`.

## Closed dependencies

The earlier 69 unavailable requests are undelegation-count requests at period
25706948. Their full identities and key preimages have zero overlap with the
VRF requests. R/overlap-map.json and overlap-report.md preserve that evidence.
No unavailable request was retried or converted to zero/nonmembership.

Accepted 20 VRF facts for D25706894, covering all recovered voters, then 47
facts for voter `138c1b9baea7d93a981ba694f5318f735c997298` at D25706895..25706941.
Together these close this voter's complete 48-period VRF dependency. Both
captures ran once after independent exact-command READY. Each complete offline
replay matched an independent replay byte for byte. All 67 values are proved
Member, physically Present, exactly 32 bytes and nonzero. Native 96 and the raw
six-input owner retain their APIs, validation and accounting. All 13 witness
ceilings remain unchanged. Prior first 20 source copies and freezes are preserved.

First replay: 32805 bytes, SHA256
`8bd32c28356e85944a37a900c95e8ae3cb7cbc7f942f27cc2769257b6be36854`.
Remaining47 replay: 59391 bytes, SHA256
`b6278f844e627f5830d56803a5555668aa982b56ce2c81fdc9b0cb677b9bc3e6`.
Final reviews: R/vrf-family-final-review.md and
R/vrf-voter-window-family-final-review.md. Outer second-guard exit provenance is
lead-attested in window-outer-invocation.json; the child result, marker, pins
and before/after metadata were independently checked. Metadata is not a DB-byte
immutability proof. Cryptographic roots and complete proofs supply value authority.

## Validation and routing

First slice: 7 focused storage tests, 167 storage package passes, 78 qualifier
passes, canonical native parity, format/check/Clippy/build, fast gate and actual
hook (2125 passes), and 4 Rust-enabled bridge tests. Second slice: 4 new storage
and 4 qualifier tests, 171 storage package passes, 82 qualifier passes, production
format/check/Clippy/build, 6 guard tests, fast gate and actual hook (2129 passes),
and 4 Rust-enabled bridge tests with 12 build jobs. Both gates include both
repository guards. Full logs, first failures and corrections remain in R.
The first second-slice commit attempt ran a passing hook with no staged files;
its exit1 record remains separate from the successful accepted commit record.

Lead observed Sol6.1 medium. Requested workers: Luna medium for bounded maps and
freeze helpers; Astra medium for separate family authority/command/fact reviews;
Sol6.1 medium for separate source reviews; fresh Astra medium for lambda and next
voter preparation. All assigned workers ran; child runtime identities and billing
are unknown. R/routing.json preserves outcomes and no routing failure was observed.

## Quota and exact remaining work

Fresh baseline 64%, floor 54%, reset 1792322995, lead
`01a12d5e-f42f-74f0-86fb-26bc2c433942`. Preserve R/quota-budget.json across resume.
Observation before closeout:56% at 2026-10-12T01:15:11.190Z, 8 points consumed.
Final observation and documentation HEAD are in R/quota-closeout-observation.json
and closeout-head.json. Quota is account-wide, separate from tokens/billing.

- 2675 voter rows remain: 847 VRF plus 1828 validator/per-voter-jail rows.
- 48 lambda selections remain null. R/lambda-next-authority-review.md verifies
  numeric floor-at-or-before and sparse writes, but source/custody, historical
  producer, policy/delay and bounded export/completeness evidence remain missing.
- 48 raw expected cache RLP rows remain missing; size/hash metadata is insufficient.
- Producer/configuration authority remains unverified. Weight/cache/owner/public
  finality/reward transition/19-transfer/production flags remain false.

Next complete voter preparation selects `264a30e880d3b53bb2cdeb234e34201ebf9a9965`,
47 remaining delayed periods, with accepted first-D parent index 8/selection 2/proof 53.
R/vrf-next-voter-window-preparation.json is50138 bytes/SHA256
`68db6aa170beaf8ad90046287df8c865685ec133dbf93f11690ee4d086828888`.
R/vrf-next-voter-window-authority.md is semantic preparation only. Implementation
and physical READY remain false. Settle its implementation contract, confirm fresh
Sol review capacity, then complete fixed adapter/checks/freeze and independent
exact-command READY before any read. Never rerun the consumed two guards.

Resume by checking R/vrf-closeout-freeze.json with verify-vrf-closeout.py after
scope review. Preserve unrelated workflow/N6/skill/audit changes. No source work
is pending. Documentation is a separate local commit. Do not push.
