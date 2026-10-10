# Next N4 run checkpoint — October 10

Branch: `feat/rust/evm-state-db`. Accepted implementation/evidence base:
`1838694c6d9089c497a824a82c120044183553ee`. Reconcile any newer preparation-only
commit. Nothing was pushed. No owned implementation, validation, review or process
is pending. Existing unrelated workflow, queue, old checkpoint, skill and audit
edits remain in the working tree; preserve them.

Use `next_executable_slice_prompt.md` for the next explicitly authorized new
10-percentage-point session. Record its fresh quota baseline at startup, not in
this preparation session. Earlier session budgets remain immutable. The old
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

First: Luna maps precise decoder/storage/import seams. Astra settles a bounded
reconstruction contract before completeness or historical-owner changes. The lead
then implements and validates the smallest useful native decoder/catalog family,
with independent review and local commits. Extend coverage from justified retained
indexes/addresses/history without claiming a complete semantic snapshot prematurely.

Proceed to a historical test owner and the same 19-transfer real period only when
complete dependencies and owner/cache authority are reviewed. No input head rewrite,
expected-root injection, production cutover or N4 closure is accepted. Additional
external information is not currently required to begin this work; name an exact
missing record if bounded reconstruction later proves one unavailable.
