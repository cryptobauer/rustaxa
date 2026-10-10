# Next N4 run checkpoint — October 10

Branch: `feat/rust/evm-state-db`. Accepted implementation/evidence base before this run:
`1838694c6d9089c497a824a82c120044183553ee`; preparation commit `1256a3e26` was reconciled.
Accepted implementation HEAD `0ce68a6815f6e81f23722b6b2e20493efc902c38`.
Nine reviewed implementation slices are listed below. Nothing was pushed.
No owned implementation, validation, review or process is pending. Existing unrelated workflow, queue, old checkpoint, skill and audit
edits remain in the working tree; preserve them.

This run followed `next_executable_slice_prompt.md` with one saved budget.
Same-session resume must reuse that budget without initialization. The ten-point floor has now been reached; do not resume implementation under
this exhausted budget. A later new run requires explicit authorization and a fresh
baseline. Earlier budgets remain immutable. The old
`n6_restart_checkpoint.md` and branch queue still describe an earlier P0 next
action; this checkpoint and the current prompt supersede that action.

## Accepted work since the old handoff

- `c182cdc4e25e218ef24587bde64dd01eb892d4c0`: exact P0 ordinary-prefix Rust binding.
  Its package, parity, fast/bridge checks and independent review passed. No arbitrary
  prefix admission or production routing was added.
- `99eec2c066a63a125e584c2233cac5b7336dedf0`: reviewed N4 owner restart contract.
- `a45a84ed825789cf1cad3b180e4280d519324c90`: bounded light snapshot availability
  diagnostic. Read-only probes, format, binary Clippy, eight library tests and
  independent review passed. No storage-module or production behavior changed.
- `1838694c6d9089c497a824a82c120044183553ee`: reviewed v1.14.1 mainnet reference basis.
  Seven release/source blobs and all 12 retained DPoS scalars match. No database
  opens or reward execution occurred in that comparison.

Read [snapshot availability](n4_light_snapshot_availability.md),
[configuration basis](n4_v1_14_1_policy_basis.md) and
[owner requirements](n4_owner_input_restart_contract.md), with the first two
updating the last one's input availability and reference-policy basis.

## Fixed window and available inputs

H=25,706,949; P=25,706,948; session eligibility E=25,706,944; certificate
eligibility D=25,706,942. The copy's committed head is H, not P. A historical
reader does not by itself construct the required parent execution owner.

All live DPoS and slashing storage at P/E/D was authenticated and fully inventoried.
Each DPoS trie has 23,278 rows and each slashing trie has 54; the respective tries
match across these identities. Parent vote/stake raw origin rows are present.
The minted-token row is a tombstone, not a supplied zero. Partial inverse decoding
explains 2,065 DPoS rows, leaving 21,213 live rows undecoded. No Rust sidecars exist.
Complete live bytes do not establish logical key catalogs, deleted/all-ever history,
complete delegation membership, reward graph authority or a complete Rust model.

Saved reward cache: 49 decoded rows, periods 25,706,901..25,706,949. Under the
owner-supplied default frequency of 100, excluding H yields the 48-period candidate
parent subset. Its historical runtime/generation authority still needs review.
The default configuration also has chain ID 841, committee 1,000 and delay 5.
H is neither a distribution boundary nor a pillar boundary under that basis.

Owner reports likely v1.14.1 and directs default mainnet configuration. Release
commit `a0e85fe31eb03573cd92c165a5f81035cec9907e` pins public EVM revision
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`. Local oracle pin is the distinct
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. Actual binary/capture identity remains
unverified; use the supplied reference basis without another configuration request.
Initial annual rate 8,523,243 uses delay 700; the independently derived post-Cacti
rate 9,275,294 uses delay 400. Keep those stages distinct.

## Evidence and next action

Detailed accepted records:

- `/home/fry/artifacts/evm-run-01a12614-b9fe-7293-a303-2bdefab52645-20261010`:
  P0 and N4 owner contracts, checks and independent reviews.
- `/home/fry/artifacts/light-snapshot-check-20261010`:
  `availability.json` SHA-256
  `b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1`,
  `config.json`, logs, review and checkpoint.
- `/home/fry/artifacts/n4-v1.14.1-policy-20261010`:
  `comparison.json` SHA-256
  `35b767e1005787c39c859a370c980225be3c2c4625743101aef718fd5ba5e545`,
  review and checkpoint.

## October 10 run results

Run artifacts: `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010`.
Lead UUID `01a126b2-3ae0-7883-a5e3-b4618ade8cd5`, confirmed Sol medium.
Luna medium ran first; fresh Astra medium contracts and independent Sol medium
reviews ran for each new profile. `runtime-routing.json` records exact thread IDs,
confirmed models and outcomes. No routing failure was observed. Historical maps
remain historical evidence; delegation-count unavailability does not imply
unavailability of distinct undelegation slots.

Accepted local implementation commits:

- `a05b3c23b`: 20 H-transfer address hypotheses; every parent delegation count
  returned HistoryUnavailable. No new live partition or numeric count.
- `bcfaa3cbb`: slashing jail catalog. The list and 20 expired live jail entries
  explain 21 of 54 rows; 33 residual rows remain opaque.
- `f3cfda618`: complete raw H ordered lists, signed transactions and receipts
  authenticated against persisted FinalChain and PBFT metadata; 19 transfers.
- `f7b0267ce`: linked P ordered lists and both hash domains authenticated;
  one ordinary transaction, no native call.
- `2d7ef482f`: fixed 48-period predecessor chain, 25,706,901..25,706,948,
  authenticated with 296 fixed reads. All 583 signed envelopes and typed receipts
  authenticate; zero direct DPoS/slashing calls and 23 unique envelope addresses.
- `2733b385d`: exact old-20 subtraction yields three new addresses and 34 role
  occurrences. Three distinct delegation count points are HistoryUnavailable.
- `a569b97d0`: nine independent V1/V2 count and V2 last-ID points for those
  three addresses all returned HistoryUnavailable. No numeric decoding or expansion.
  Source/probe pins match; 35 library and three binary tests, fmt/check/Clippy
  and independent Sol/Astra reviews passed.
- `a3d8ad3`: 60 V1/V2 count and V2 last-ID points for the older 20 H-transfer
  addresses all returned HistoryUnavailable, retaining all 38 provenance records.
  One call-limit test failure was fixed before probing; a review-required test-only
  addition then passed five binary tests/Clippy/format. Runtime code and captured
  bytes remain identical. Independent authority and implementation reviews accepted.
- `0ce68a681`: one authenticated P/DPoS raw inventory capture, closing the raw
  artifact-retention gap. 23,278 rows/259,077 bytes/31,164 nodes retained; pure
  offline digest verification and targeted checks pass. Fresh Sol/Astra accepted.
  One review-required test-only hex correction passed; runtime source unchanged.

The interval has fixed lower bounds. It does not establish the retained-history
boundary, all older native calls, delegation roles or complete historical catalogs.
The C++ ordered-root reference matches H/P raw lists and fixed root vectors.
No upstream C++ source changed. Per-family documents, source captures, freezes,
first-run gate/probe logs and independent closeouts hold exact source/report hashes.
Targeted checks passed; full scope is recorded in each closeout. No production or
storage module changed, so bridge, broad and differential gates did not apply.
No writable historical owner or real one-period reward transition was built.

## Exact next action and limits

The complete live native bytes remain available. Earlier saved inventory reports contain summaries/digests. The accepted raw-retention
slice is: `parent_raw_inventory.rs` plus its document,
`parent-raw-inventory.json` (2,894,865 bytes; SHA-256
`63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056`).
One guarded inventory captured 23,278 rows/259,077 bytes/31,164 nodes and
pure offline verification passed. Format/check/Clippy, 35 library and two binary
tests passed. Independent Sol/Astra final reviews accepted. The exact post-capture diff
contains only added negative hex tests; their affected checks passed.
The raw artifact is digest-verifiable against prior authenticated evidence,
not a standalone Merkle proof. No second inventory is needed for verification. Address hypotheses outside the fixed interval and older reward
node keys have no complete justified catalog. Parent cache generation and runtime
authority also remain unestablished. No absent/tombstone/unavailable result supplies
zero, no root substitutes for semantic reconstruction, and no completeness guard
may be weakened.

Before another useful slice, obtain explicit new-run authorization and record
a fresh session baseline. Do not reset or transfer this exhausted budget. Obtain a fixed
source/key/identity/call/byte contract and fresh review for a changed profile.
Both the three new and older 20 addresses have now been observed for their
V1/V2 count and V2 last-ID keys: 69 HistoryUnavailable outcomes. These cover
only that justified sample and exact P/root; no global absence follows.
Next useful source-only audit: derive the exact accepted validator/record-owner
address union, preserve source positions and subtract previously observed
(P/account/logical-key) tuples. Existing inverse may already cover these keys;
check exact records before proposing any probe. A nonempty unobserved set needs
its own fixed contract and fresh review. Accepted saved raw entries can now support offline analyzer work without
another live inventory.
For slashing proof keys, exact missing inputs are authenticated paired-vote
records (or successful commitDoubleVotingProof calldata); jail logs alone do
not supply ordered vote hashes. Non-head reward nodes need justified historical
cursor/head references, not a block-number sweep.
Larger graph,
cache or owner work needs its own exact authority contract. Proceed to the
historical owner and the 19-transfer real period only after complete dependencies
and cache/owner authority are independently accepted. N4 remains open.

## Run closeout and exact allowance

Lead startup baseline: 53% remaining at 2026-10-10T16:43:22.225Z.
Run floor: 43%; reset window: 1791948695. Floor reached at
2026-10-10T18:12:47.204Z. Final observation: 43% remaining at
2026-10-10T18:13:24.777000+00:00; consumed 10 percentage points.
These are account-wide allowance observations, separate from token or billing claims.
`quota-budget.json` was initialized once and preserved through resume.
No new work/delegation started after the floor; only in-flight review/commit and
checkpoint closeout continued. All useful next work is deferred because of this
budget limit, not a claim that every native source is blocked.

All nine source/probe/check/review freezes and actual model routing are in the
run directory. `progress-checkpoint.md` and `quota-final.json` give final state.
The only remaining working-tree changes are the unrelated initial workflow,
queue, old checkpoint, skill and token-audit edits. Preserve them. No supplied
data mutation, writable owner, one-period reward execution, production route,
standalone Merkle proof or complete logical/semantic snapshot is accepted.
