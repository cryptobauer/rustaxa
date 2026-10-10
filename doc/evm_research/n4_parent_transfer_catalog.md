# N4 parent transfer-address delegation catalog

The bounded parent probe found no live delegation-count row for any of the 20
addresses from the accepted 19-transfer window. All 20 physical reads returned
`HistoryUnavailable`. This does not supply twenty zero counts or establish deleted
history. The complete authenticated P inventory contains none of these paths.
No new live row was decoded. N4 execution remains open.

## Inputs and scope

The new experimental `parent_transfer_catalog` binary uses the existing path
and identity guards. It opens only `local/evm-state-db/snapshot-litenode-copy`
read-only and writes a new report outside both data trees. The supplied `data`
tree was not opened. H remains 25,706,949; the selected reader is P=25,706,948,
root `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`.

Address provenance is the exact accepted `n4_replay_preflight.json` SHA-256
`d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437`.
The tool checks H/P/root, all 19 ordered chain-841 call envelopes, one sender,
19 unique recipients and exact union equality with the 20 present prior accounts.
It records source roles and envelope positions. This reuses historical signature
and account evidence; no fresh signature validation is claimed.

The owner-supplied default-mainnet, likely-v1.14.1 policy assumption remains the
reference basis. Actual producer binary and capture identity remain unverified.
The [reconstruction contract](/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/reconstruction-contract.md) limits this
slice to delegation counts, live iterable members, principal and exact referenced
reward nodes. V1/V2, slashing and arbitrary cursor/history searches are excluded.

## Decoder and result

`native_inverse::analyze_address_delegations` reuses the strict existing delegation
codec and observed Go iterable keys, including the pinned prefix alias. It accepts
at most 20 justified addresses, 256 aggregate delegation entries, 1,044 point
calls and 1 MiB accepted point-value bytes. Counts are checked before child reads.
Required indexed children must match live inventory bytes; missing, tombstone,
orphan, unavailable, malformed or inconsistent required data rejects the catalog.
Optional counts and cursors preserve absent, tombstone, unavailable, pruned and
orphan classes. Shared live reward nodes count once in coverage but retain each
reference. Principal and iterable positions remain explicit. Raw observations
record logical key, trie path, exact bytes or digest, length and error class;
references identify source addresses, roles, positions and row families.

The single P DPoS inventory used the reviewed 50,000-node, 50,000-leaf and 32 MiB
ceilings. Its 31,164 nodes, 23,278 leaves, 259,077 value bytes, storage root and
sorted path/value digest match the accepted availability evidence. The catalog
made exactly 20 point calls, accepted zero value bytes, matched zero live rows
and left all 23,278 rows unexplained by this address source. The residual digest
is the unchanged inventory digest. These independent counts are not added to the
existing 2,065 validator/owner matches; their exact path union was not measured.

All semantic/global-delegator/history/owner/adoption/production flags remain false.
No semantic snapshot, historical owner, reward transition or publication occurred.
No full logical-row comparison of every database column was performed, so no
complete database preservation claim is made.

## Validation and evidence

Rustfmt and targeted Clippy with warnings denied passed. Qualifier library tests:
14 passed, including six new catalog tests. New binary provenance test: one passed.
Existing native inverse binary tests: 15 passed. Tests cover ordering, shared cursor
references, principal, strict widths/RLP, missing/reversed/duplicate members,
inventory byte drift, non-live/error classes, exact maximum point count, aggregate
byte bounds and rejection before over-limit child reads. The permitted P probe
exited zero. An existing consensus dependency unused-import warning remains.

This changes experimental diagnostic code only. Storage modules, production
routing and bridge behavior are unchanged; storage bridge, subsystem and broad
rewrite/differential gates are outside this diagnostic's validation scope.
Complete first-run commands, outputs and exits are in `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010`.
Report SHA-256: `4dac8c162b532ae8a5328c40c6045bd7eb7186db844025ae02ded34af4cce7ba`. Independent source/output review is recorded in
`catalog-review.md` there; acceptance requires that review before local commit.

The next source must justify addresses or logical keys beyond this transfer
window. Required history remains specific: the complete delegator key sources,
reward graph/deleted/stale-head authority and pre-fix same-validator operation
history needed for owner completeness. This result does not establish that those
records are absent from the copy. Keep P, E=25,706,944 and D=25,706,942 distinct.
