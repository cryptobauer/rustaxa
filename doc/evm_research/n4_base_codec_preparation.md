# N4 bounded base row codecs

This preparatory diagnostic exposes pure keys and typed canonical row decoders for a later exact P base ledger. No reader or callback is accepted. It does not regenerate the base matched set, re-admit the undelegation report, measure an intersection or union, or establish a complete snapshot or historical owner.

Implementation lives in `snapshot_qualifier/src/native_inverse/base_rows.rs`. The parent module adds only its public module declaration; existing physical analyzer code and serialized facts are unchanged. Astra's `base-codec-preparation-contract.md` and fresh independent Sol contract review are READY. Final source acceptance is recorded separately in the run artifacts.

## Public interface

Every raw decoder checks `MAX_BASE_CODEC_ROW_BYTES = 4096` before parsing, conversion or allocation. The cap applies only to these new helpers. It has not been proved sufficient for every retained P base row. A later oversized row needs a named blocker and reviewed cap change.

- `validator`: typed encoding, arbitrary-width stake, u16 commission, u64 change/head and optional u16 undelegation count. Four-field V1 has None; nested extended V1 has Some, including zero. An exhaustive conversion rejects any internal encoding/count disagreement.
- `info`: exact description/endpoint bytes, permitting empty and leading-zero strings.
- `rewards`: arbitrary-width unsigned delegator/commission rewards.
- `reward_node`: arbitrary-width reward per stake and u32 reference count.
- `delegation`: arbitrary-width stake and u64 graph cursor.
- `scalar`: explicit `ScalarField` and typed `Scalar` output. Field 4 is compact u64; fields 5/6/7 are arbitrary-width compact values; field 8 is canonical RLP u64. Unknown field conversion fails. Empty compact bytes represent zero; RLP zero is 80. A supplied scalar is never evidence of presence or a tombstone default.
- `address`, `vrf`, `index`: exact 20-byte, 32-byte, and four-byte LE values. Empty or wrong widths fail. Index zero is valid bytes; one-based membership remains the caller's duty.

Retained private codecs enforce exact RLP arity/data shape, canonical integers and byte-for-byte re-encoding with no trailing data. They reject fixed numeric overflow. Amounts have no 256-bit restriction within the hard row cap. All decoders return Result; cap, shape, canonicality and width violations return an error. No state is changed.

`ValidatorEncoding`, `Validator`, `Info`, `Rewards`, `RewardNode`, `Delegation`, `ScalarField`, `Scalar` and `ValidatorField` keep explicit domain fields. They have no serde input or physical status. They do not grant ordering, history, enumeration or graph authority.

Pure keys reuse retained constructors: `validator_key` hashes 00/field/address for five explicit payload fields; `reward_node_key` hashes 01/validator/compact-BE-u64 cursor; `delegation_item_key` and `delegation_reverse_key` use 02/01/delegator/02 with LE index or validator address; `delegation_record_key` uses 02/00/validator/delegator; `scalar_key` hashes the supported field byte. `trie_path` applies the second Keccak hash to the logical key. Position bounds and membership remain caller responsibilities; cursor zero has an empty compact suffix. Existing validator seed/count/owner helpers remain available.

## Validation and limits

The affected library tests include independent literal RLP vectors, both validator forms, zero/max fixed integers, arbitrary-width amounts, metadata bytes, all hard-cap paths, exact-cap acceptance, fixed widths, malformed/trailing/nonminimal RLP, nested shape and overflow failures. Literal key preimages check argument order, LE indexes, compact cursor zero/nonzero, the retained discriminator alias and both hash levels; an existing frozen item-key vector is preserved. A required review correction adds independent OpenSSL KECCAK-256 logical-key and trie-path constants for field 8 (preimage 08). The focused exact key test and format checks pass after this test-only addition.

Focused qualifier library tests, cargo check, Clippy with warnings denied for the affected package, format and whitespace checks are required. First and subsequent outputs are retained under `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2`. The existing dependency warning in rustaxa-consensus is outside this change. No executable or database run is required for this pure interface slice; no storage, bridge, node or differential gate applies.

Next action: implement the fixed three-input `base-path-union-contract.md` profile. Rebuild exact base expected-byte equality and residual digests before admitting the 217 undelegation paths and calculating actual path intersections. Counts must not be added arithmetically as union evidence. All completeness, reward execution, adoption and production limits remain in force.
