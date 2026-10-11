# Exact saved P base and undelegation path union

This offline diagnostic rebuilds the existing base path set from typed facts, verifies the accepted undelegation catalog against the same raw rows, and measures actual path intersections, unions and complements. It opens no database. This path evidence does not create a complete logical catalog, a DposSnapshot, a historical owner or reward execution authority.

## Contract and admission

Astra's `base-path-union-contract.md` and fresh Sol's contract review are READY. The pure base codecs were independently accepted at `e2a5931e7f0bd7f1532667b02d04a832065fc0f4`. Implementation consists of `src/bin/base_path_union.rs` and its binary-local `base_path_union/ledger.rs`; it reuses the accepted parent-cache admission module and bounded base row codecs. Final independent review is recorded after the frozen checks and output.

CLI accepts exactly `base_path_union OUTPUT`. Three fixed whole-file pins are admitted before duplicate-key rejecting JSON parsing: availability (1088773 bytes, b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1), parent raw inventory (2894865 bytes, 63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056), and accepted offline undelegations (1117732 bytes, cf63dd21b22d249b4013bfcc9d94b1ea89ee1354982e320d5f194ee922157d53). Exact paths are in the report and contract. Per-file cap is 4 MiB; aggregate cap is 8 MiB. Canonical regular paths are outside both database trees. Cap-plus-one reads reject growth. No embedded input path is followed.

P=25706948, FE, P header/state/storage identities and the raw inventory digest are fixed. H/P/genesis pair fields agree across the availability and raw reports. The selected P dependency and FE account are unique, authenticated, fully inventoried and partially inverted. Raw rows are strictly ordered unique paths with exactly 23278 rows and 259077 value bytes. Canonical hex lengths and remaining aggregate bytes are checked before decoding. The length-framed raw digest is recomputed.

## Base reconstruction

`Raw::admit` retains each saved row index and bytes in an ordered map. `Base::visit` derives Keccak(preimage) and then Keccak(logical key), counts each in-memory lookup, and requires a live raw row for a recorded live fact. Pure bounded codecs decode canonical raw bytes; full typed values must equal saved facts. Their exact re-encoding checks establish canonical byte equality without an additional encoder. Payloads over 4096 bytes fail before decode; the real base succeeds within this cap.

`rebuild` replays count, validators by stored position, item/reverse/payload/current reward head, five global fields, then the sorted validator/owner address union and each delegation by stored position. It verifies 196 unique validators, 290 exact role-bearing candidates, 108 child records, scalar field names, all numeric widths, representation/count agreement and candidate count/child consistency. It does not sort stored map positions by address.

Saved non-live outcomes require no live raw membership and null numeric fields. They remain saved evidence, not newly observed physical outcomes. Live reward references use explicit new/repeated status: a repeated reference must reach the earlier reward family with identical preimage/key. Ordinary duplicates, new/repeated disagreement and collisions fail. First attribution reproduces every saved family counter. All reward provenance edges remain attached to one distinct row.

Base matched rows are 2065 with 35694 bytes. Its complement must reproduce 21213 rows, 223383 bytes, both saved residual digests and the first 16 residual paths. Equal totals alone cannot pass. Full base ledger entries include identity, source selector, family/provenance, preimage/key/path, canonical equality, raw index and expected/raw lengths and hashes.

## Accepted family and exact partition

`union_report` verifies all 1017 catalog tuples against the accepted catalog version/digest. It requires exact period/account/raw-artifact links, both hash levels, raw row index/bytes/hex/hash, supported status and the exact sorted 217 matched paths with 2277 bytes. The 800 unresolved tuples have no live membership or decoded/raw values. Unknown or invalid statuses fail. Existing accepted catalog semantics are reused by its whole-file pin; no new key discovery or decoding authority is added. The 70 scalar matches are already inside the 217 rows; V2 last IDs remain cursors.

The ordered raw map is visited once for eight partition digests. Exact B/U overlap requires identical key/preimage and the same raw reference. Both provenance sets are retained, including explicit accepted tuple references. Counts and bytes come from actual membership; inclusion-exclusion is a secondary check. No family-name disjointness assumption or arithmetic-only union claim is used.

Measured initial result: intersection empty; union 2282 rows/37971 bytes; residual 20996 rows/221106 bytes. Exact output pin and partition digests are recorded in the final run freeze after any review corrections. All residual bytes are retained live bytes, not missing input. Their semantics remain unexplained.

Base derivations/lookups are capped at 2296, U links at 1017, total 3313. Raw admission and base-residual loops each visit 23278 rows, separately from the one 23278-row union partition loop. Report counters distinguish in-memory work from zero physical opens, points and inventories. Output is serialized through a 16 MiB bounded writer, synced in a new same-parent temporary, then atomically published with exclusive hard-link creation; overwrite and partial report publication fail.

`SetDigest` hashes ascending path bytes and length-framed path/value entries for each selected set. Private parsers reject missing/wrong field types, noncanonical hex and integer overflow. Every violation returns an error without a success report. No state owner is changed.

## Validation and remaining limits

The affected library's 47 tests pass. Five binary tests cover typed equality, shared reward provenance/status errors, admitted real reconstruction, identity/order/role/value/residual/catalog/raw-link changes, duplicate raw paths, a synthetic nonempty overlap with both provenance sets, duplicate JSON, cap-plus-one growth and exclusive publication, exact lookup/distinct-byte/repeated-byte boundaries and the output writer exact cap/overflow. These fixed-profile tests require the three pinned artifact files; they fail rather than skip if unavailable. Existing codec malformed/width/cap vectors remain covered by library tests.

Format/check/Clippy, bounded offline execution and whitespace checks are captured in the run directory. The initial construction draft passed compile/tests, then Clippy rejected an unused import/publisher, argument count and modulo style; those findings were corrected before execution. The existing rustaxa-consensus dependency warning is outside this change. No storage module, C++ or production route changed; bridge/node/differential gates do not apply.

`base_union_verified` is the only new qualification. Complete semantics, global delegators, logical catalog, owner/database preservation, reward transition, adoption and production routing all remain false. Source and binary pins are frozen with exact inputs/output. Parent reward fields still lack earlier delayed roots, native voter facts, weights, totals, lambda provenance and independently derived cache rows. The producer remains unverified under the owner-supplied default mainnet/likely v1.14.1 reference basis.

Final review requested explicit guard tests for the 2296 lookup, 35694 distinct bytes, 1 MiB repeated bytes and 16 MiB output limits. These tests were added without changing operational bounds. Earlier review findings and report pins are retained; final acceptance is separate.
