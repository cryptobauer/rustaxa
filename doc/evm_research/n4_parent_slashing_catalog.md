# N4 parent slashing jail catalog

The bounded P probe decoded 21 of 54 authenticated live slashing rows: the empty
jailed list and 20 persistent jail-block rows. The remaining 33 paths retain
unknown logical-key meaning. All 20 decoded jail blocks precede P. An empty
current list does not erase the historical jail-block map. No complete slashing
snapshot or N4 execution is accepted.

## Source and bounds

The experimental `parent_slashing_catalog` binary uses existing path guards and
read-only paired qualification for `local/evm-state-db/snapshot-litenode-copy`.
It keeps committed H=25,706,949 and selects P=25,706,948 at root
`926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`.
The supplied `data` tree was not opened. Output is a new file outside both trees.

The exact accepted availability report supplies 196 P validator addresses, SHA-256
`b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1`.
The tool reads at most 2 MiB plus one overflow byte, checks the hash before parsing,
then checks unique P/account selection, exact root, successful inverse evidence,
196 distinct twenty-byte addresses and source positions 1..196. These are current
validator seeds, not every validator ever jailed.

A fresh Astra context approved the [contract](/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/slashing-authority-review.md)
before implementation. The single EE inventory has ceilings of 256 nodes,
128 leaves and 4,096 accepted value bytes. The fixed profile requires the live
jailed list to be canonical `c0`; any drift rejects the profile. Only 197 raw
point calls and 1 MiB accepted point-value bytes are admitted. No transaction,
receipt, proof-hash, period or bloom scan occurs. Source layout agreement was
checked at both the public Go revision and the distinct local oracle revision;
this does not claim complete Go implementation parity.

## Decoder contract and observed result

`native_slashing::analyze_slashing_jails` accepts justified unique addresses and
valid distinct source positions. It preserves source order and expired blocks.
The list uses direct leftpad32(02); jail rows use Keccak(00 || validator), then
Keccak(logical key) for their trie path. Live integers must decode as canonical,
fully consumed RLP uint64 and match exact inventory bytes. A live zero can be
decoded as zero; absent, tombstone, unavailable, pruned or orphan rows never
supply zero. Unexpected backend/corruption errors fail closed. Non-live optional
history errors remain partial diagnostics. Non-live/error results at live paths
reject the catalog, as do malformed bytes, duplicates and exceeded caps.

The authenticated inventory matches all accepted pins: 71 nodes, 54 live rows,
128 value bytes, storage root
`d688bfddb847c241b44f78c048b41d7e423f138bc5ee8fe6c24e6f047b051a7a`
and sorted path/value digest
`8309d0e7fc00c05fdec78037aee0f09dd24586d73ea4aa6457f5b3c8cd9ff5ef`.
Actual calls: 197. Accepted point-value bytes: 91.
Validator-row classifications: {'live_present': 20, 'history_unavailable': 176}. The 21 unique matched rows contain
91 value bytes; the 33 residual rows contain
37 bytes. Their exact union is the complete inventory.
Per-seed records preserve source position, key, path, raw result/value and optional
jail block. Residual raw path/value rows are retained without guessed proof meaning.

The owner-supplied default-mainnet, likely-v1.14.1 reference assumption remains
explicit; actual producer identity is unverified. This reads exact P state and
makes no delayed-eligibility or policy decision. All all-ever/proof-key/semantic
completeness, owner, adoption and production flags remain false. No writable
owner, cleanup, head rewrite, reward execution or publication occurs. No complete
logical-row comparison across all database columns was made.

## Validation and next dependency

Rustfmt, affected library/binary check and Clippy with warnings denied passed.
Qualifier library tests: 19 passed; new binary provenance test: one passed.
The first five targeted slashing tests also passed. Tests check direct versus
hashed key layout, canonical integer/list data, live byte mismatch, missing/error
classes, expired-row retention, source order, exact partition, provenance drift,
seed/byte caps and exact 197-call maximum. The bounded P probe exited zero.
An existing consensus dependency unused-import warning remains. Only experimental
diagnostic code changed; storage production and bridge/subsystem routes are
unchanged, so broad rewrite/differential gates do not apply to this slice.

Report SHA-256: `4b958530c6c40559c52b35cc339872676bab4171875838f494f98daa4ba42c02`. Complete commands, first outputs, exits and frozen hashes
are under `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010`. Final independent decoder/output review is `slashing-review.md`;
acceptance requires that review before local commit.

The precise source needed to attribute proof keys is successful
`commitDoubleVotingProof` paired-vote calldata or an equivalent authenticated
paired-vote record. Jailed logs omit the two vote hashes. This slice did not
establish whether those older records remain in the copy. Unknown residual bytes
must not be called proof markers solely because their values resemble true.
Deleted-validator jail addresses remain outside the current-validator seed set.
Complete reward graph and pre-fix same-validator history remain separate owner
requirements. No full snapshot request or historical owner construction follows.
