# N4 signed parent reward fields and missing-input manifest

This pure diagnostic authenticates the available reward input fields for periods 25,706,901–25,706,948 through the accepted local H anchor. It verifies signed DAG order and certificate bindings, derives ordered transaction fees, and emits specific missing historical inputs. It does not construct reward statistics, calculate weights/rates, compare complete cache rows, or create an owner.

The accepted implementation base is `604664af8ecdee6c25eb1c24c2f42865b5e1724a`. Scope is the qualifier library entry in `src/lib.rs`, new `src/parent_reward_fields.rs`, new `src/bin/parent_cache_fields.rs`, its binary-local `parent_cache_fields/admission.rs`, and this document. No storage module, production route, C++ or supplied database changes. Focused qualifier checks apply; bridge, node and broad differential gates do not apply.

## Contract and fixed evidence

Astra's `parent-cache-offline-field-contract.md` is amended to raw DAG difficulty and fee inputs only, then corrected for cache key/value byte accounting. Final contract SHA is `e4ac4f362f4d20dba7c8675b119d9c5f03c66751cc66ab659d0d8999dd3ecfb7`. Fresh Sol's initial and metadata-correction reviews record READY before dependent implementation/correction. Earlier proposal hashes and failed runs remain retained. Final acceptance is recorded separately after frozen review.

Run records are under `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2`.

| Fixed operational artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `/home/fry/artifacts/light-snapshot-check-20261010/availability.json` | 1088773 | `b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1` |
| Oct 10 run `interval-history-authentication.json` | 2766744 | `3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5` |

Both exact canonical regular paths, sizes and hashes are checked before any parsing. Combined input cap is 3855517 bytes. The adapter checks ten exact source files, each below 1 MiB, before interpretation. Embedded paths are not followed. A changed input or source pin needs a new reviewed manifest. New implementation and binary hashes are frozen separately. Output is an exclusive new file outside both database trees, encoded within 8 MiB. A create-new same-parent temporary is written and synced before atomic exclusive hard-link publication; a cleanup guard removes the temporary on success or error. CLI accepts exactly one output argument.

Reference policy is owner-supplied default mainnet, likely v1.14.1. Producer identity remains unverified. Chain 841, eligibility delay five and frequency 100 are reference assumptions. H=25,706,949, P=25,706,948, E=25,706,944 and H certificate D=25,706,942 remain distinct. No state root is injected.

## Complete implementation units

`parent_cache_fields/admission.rs` is a fixed artifact boundary. `sha` hashes bytes. `outside_db` rejects canonical paths under either original or copied database tree. `evidence` enforces exact path, regular metadata, exact size, bounded growth and SHA. `read_capped` detects overflow with a cap-plus-one read. `Unique` and its Deserialize/Visitor implementation recursively reject duplicate object fields, floats, malformed or trailing JSON. `parse_unique` returns the admitted Value. Nested maps and arrays preserve content; malformed numeric byte arrays fail typed decoding.

The adapter's `pinned` admits both operational byte buffers and all ten frozen source files before returning any parsed input. It records sources in sorted path order. `report` checks the saved interval report shape, decodes raw `IntervalHistoryRows`, admits the full cache metadata, calls the pure kernel, and adds exact input/source/policy/bound/runtime/implementation metadata and digests. It does not trust saved position facts or cryptographic witness flags as a replacement for regeneration. `publish_exclusive` writes and syncs a bounded same-parent temporary, atomically creates the final link without replacement, and cleans its temporary on all exits. `main` rejects unknown modes, checks output exclusion/existence, admits inputs, constructs and validates the report, and publishes only fully encoded and synced bytes. There is no database opener or callback.

`parent_reward_fields.rs` has only pure domain inputs. `CacheExpectedRow` contains period, raw-value byte count and expected SHA, rejecting unknown typed fields. `ParentCacheMetadata` holds all 49 ordered retained metadata rows, including H before parent selection. `ParentRewardFieldReport` holds partial evidence, missing inputs and qualification flags, with no owner/runtime token.

`hash` computes Keccak-256 and `hx` encodes lowercase hex. `normalized_digest`, `sorted_value` and `ensure_no_floats` recursively sort object keys, preserve arrays, encode compact UTF-8 JSON and reject floating numbers. This is an explicitly specified local format, not a standards claim. `scalar_u64` and `scalar_u16` decode and re-encode exact minimal RLP integers. `order_hash` implements the C++ signed order commitment: zero for no blocks, otherwise Keccak(RLP([[ordered DAG hashes]])). A flat hash list is rejected by comparison.

`DagInputs` keeps each recovered DAG author and original ordered transaction hashes. `dags` bounds the compact three-field bundle, hash dictionary, block count and index lists before expansion. It requires compact/canonical arity, hash widths, canonical in-range indexes and at most ten blocks. It checks compact and projected expanded byte limits with overflow-safe RLP sizing before allocation, then reconstructs canonical eight-field DAG bytes through the existing codec, verifies sender recovery, typed byte-for-byte re-encoding, exact embedded VDF consumption, and canonical u16 raw difficulty. It rejects duplicate hashes, requires the signed nested order commitment, and requires a nonzero signed pivot to occur in the committed list. The sole empty retained encoding is `80`, with zero signed pivot/order. Raw VDF bytes and difficulty remain observed fields; VDF/VRF validity and reward eligibility are unqualified. No minimum-difficulty flag or reward counter is emitted.

`certs` prechecks the five-field optimized bundle, canonical original period/round/step scalars and at most 20 entries. It checks optimized source/proof and projected canonical byte bounds before reconstructing unweighted canonical votes through the existing pure helper. Each vote must have exact three-field shape, canonical embedded sortition, inspection status Valid, signature_valid=true, no embedded weight, cert step three, exact bundle identity and previous signed PBFT hash. Typed canonical re-encoding must preserve all bytes. Duplicate voter or vote hash fails. The complete unique hash set must match the current signed PBFT reward list, with exact cardinality and no duplicates. Wire and signed-list order are retained separately; wire order equality is reported, not assumed. A wire-only reorder remains valid when the signed set matches and changes the wire-order digest. The oldest period retains the missing predecessor body fact. Query Q=c−1 and D=Q−5 use checked arithmetic after decoding c=s−1.

`transactions` decodes the already list-authenticated signed transactions and five-field receipts. It checks chain/signature, count, unique hashes, checked cumulative gas, final stored header gas, and checked U256 gas-price multiplication. It visits committed DAGs and their transactions in original order. First inclusion assigns the fee to that DAG author; later inclusions consume no second fee. Nonfinalized hashes are recorded as ignored and unmatched finalized transactions remain explicitly unassigned. Author totals use checked addition, including zero-fee memberships. These fee facts do not imply a complete validator reward map.

`derive_available_fields` regenerates the entire interval witness before exposing fields. It requires 48 parents, 420457 raw bytes, exact totals of 914 votes, 139 DAGs and 583 transactions. Signed PBFT metadata provides explicitly labeled recovered proposer author; field three is not read as an address. Output periods are ascending; original source positions stay intact within each list. It associates exact retained header hashes/root identities with needed D values and emits symbolic native facts by exact delayed-period/voter/key identity. Missing roots stay null. Values and proofs remain null/unqualified; no physical outcomes are invented.

`derive_parent_reward_fields` attaches expected cache hashes only after independent available-field construction. It checks exactly 49 unique ascending periods 901..949, lowercase SHA shape, bounded row sizes and exact key/value accounting, then excludes H. Raw values total 26614 bytes; 49 eight-byte keys add 392, making the recorded aggregate 27006. Parent values total 26048; 48 keys add 384, making 26432. H value is 566. Output separates these quantities. Expected raw cache values remain missing. Changed expected hashes alter the missing manifest, not available-field calculations or their digest.

`rlp_size` computes checked projected list/string framing before expansion. `validate_field_report` enforces the fixed core/adapter key set, mandatory false authority flags, raw-difficulty-only output, exact periods, missing null facts and lambda fields. Unknown authority/root fields, removed eligibility/counter fields and substituted missing roots fail. The adapter calls it before digests and publication; mutation tests exercise these guards. Generic admission, signature, canonical, identity, arithmetic, count or order errors abort without a success artifact. No fallback value, enlarged mode or partial successful field report bypasses these checks.

## Result and exact remaining inputs

The corrected offline report passed with 48 periods, 914 signed certificates, 139 signed DAG blocks and 583 transactions. Every finalized transaction has a first DAG fee owner. The checked local-chain, DAG order/signature and certificate signature/hash-binding flags are true; independent public finality remains false.

The missing manifest contains:

- 48 distinct D identities, 25,706,894..25,706,941. Exact authenticated root/header sources are present for 901..941. Seven headers/roots and chain-link material at 894..900 are missing from this retained interval.
- 2742 symbolic voter facts, three for each of 914 exact delayed-period/voter pairs: DPoS validator, DPoS VRF and slashing jail record. Source period/vote references are retained. There are also 48 total-vote and 48 jailed-list requirements. No proof/value is supplied. A nonempty jail list needs separately reviewed total correction facts.
- 48 missing lambda selections, requiring selected raw key/value, update period, nearest-at-or-before provenance, policy ID and source classification. All required missing fields are explicitly null. Rate stays null; H lambda is not copied.
- 48 missing raw expected cache values. Hashes/lengths identify the exact expected values but cannot reconstruct their typed contents.
- An unqualified DAG reward-difficulty threshold/predicate/source pin. Raw signed difficulty alone does not establish reward eligibility.

All weight/total/rate/full cache equality, historical cache/key/owner authority, complete semantic snapshot, reward transition, adoption and production flags remain false. This diagnostic establishes specific source dependencies for later narrow contracts; it does not authorize their probes or a historical owner.

`parent-cache-fields-cap-tests.json` is 2257600 bytes, SHA `b54906443d6504bd1ad59d2208791d4cd710ffe2ae4a0731f8306af7b5521a3b`.

| Semantic digest | SHA-256 |
| --- | --- |
| Available period facts | `91f5d4f11dc37d576dc452561ca8ba5a342c5490b47ffb9cb4770ebf407b1a90` |
| Inputs/source/policy | `f1f18e48af3f351391360ac21b6154978606b9edc9a821280df00f5df2f1cbd8` |
| Missing manifest | `eb752ebf1db0fa16c00903b36a6631a4dc72b6a081bbdd50e3d9183ba2ac565d` |
| Full report without its content digest | `180ab1bcee5560cced0c8573fdf21cd6b5fb380e43972f81a7baae6929932af7` |

The file SHA covers serialized bytes plus final newline. Semantic digests exclude mutable timestamps and output paths. The report-content digest omits only its own field. `parent-cache-fields-freeze-final.json` records current source, executable, fixture, report and first/corrected log hashes; capture copies retain exact run source.

## Checks and retained failures

Forty library tests and one adapter test pass. Five new library tests cover the real signed interval, exact absent facts, DAG commitment/pivot/signature/index faults, the nested order hash, noncanonical difficulty, certificate signature status/set/duplicate/period faults, valid wire reorder, receipt cumulative gas, incomplete interval, digest object ordering/no floats, cache key/value accounting and expected-output independence. The adapter test covers duplicate/nested JSON fields, floats, trailing bytes, source size/hash, bounded reads/output, database exclusions and exclusive creation. Existing chain/receipt/RLP fault tests remain unchanged.

Format, check, Clippy with warnings denied, whitespace and corrected offline execution passed. First failures remain recorded: wrong gas newtype getter at compilation, return-type complexity in Clippy, and the adapter metadata total that confused row value bytes with key-plus-value bytes. Astra and fresh Sol verified the precise byte correction against the pinned availability source before the patch. It changed no input pin or authority boundary. The existing consensus unused-import warning remains outside scope. No database, bridge, node, production or broad gate ran.

The first final review required atomic publication, two missing lambda fields, pre-expansion byte caps and explicit output schema rejection. All four were implemented and the affected tests/Clippy/format/offline run passed. The first review, report and freeze remain retained; the corrected review is separate.

Review follow-up: four source and projected-expansion negative cases now assert the specific DAG/certificate byte-cap errors before signature or order work. The affected 41 library and one adapter tests and Clippy pass. Available facts and missing-input digests are unchanged. The implementation pin and report-content digest changed because the core source includes these tests.
