# Independent bounded reward inputs — September 30 continuation

Base: `3605bf909` on `feat/rust/evm-state-db`. These diagnostics use the
qualified independent copy only. They introduce no production routing, state
execution, adoption, complete-snapshot publication or broad replay.

## Independent certificate weights

The [reviewed contract](n4_reward_vote_contract.md) fixes H=25,706,949,
certificate P=25,706,948, requested DPoS Q=25,706,947 and candidate delayed
state D=25,706,942. The existing checkpoint owner authenticated D root
`dc4c6771a9ed227db8179b68783eefc78bb0f6a443c573d71457c8d516f8d041`
while retaining current committed descriptor H.

The [exact execution report](n4_independent_reward_vote_inputs.json) records
59 distinct account/key pairs, each with a raw read and authenticated proof.
The total counter is 534,879 and the jailed-address list is authenticated empty
(`c0`), satisfying the gate before the 19-voter loop. All validator and VRF
members agree with physical bytes. Seventeen jail paths are NonMember with
physical HistoryUnavailable at the exact D identity; two have expired jail
records. These are separate physical and logical observations, not missing-row
inferences. No child enumeration or range inventory occurred.

The consensus-owned narrow point-fact facade reuses the validator decoder and
stake vote kernel without constructing a sparse DposSnapshot. All 19 signatures
and strict VRF checks pass with independently derived vote inputs and no
preverified weight. Every calculated weight matches the retained BlockStats;
the sum is **714**. Expected output enters only after reconstruction. Strict VRF
verification is the diagnostic policy, not a claim about the producer's original
sampling policy. Source fingerprint: `2ef4c85ef12d16e5e574fda92c1c2d51d898cc5a8d9a316d84c176d27397e4de`.

## Independent annualized rate

The [rate report](n4_reward_rate_inputs.json) derives lambda **1500 ms** through
existing `MetadataRepository::period_lambda(H, true)`. With candidate consensus
delay **400 ms**, the existing checked Rust calculator returns **9,275,294**
blocks per year, equal to the retained statistic. The report uses six application
operations: four pair-qualification reads, one predecessor lookup and one output
comparison. Concrete reads are limited to existing descriptor/root qualification.

The repository returns the decoded lambda only; selected update period, selected
raw key and original raw value bytes are not exposed. Canonical re-encoding is
labeled accordingly. This does not establish live dynamic-lambda state or the
producer's effective configuration. Source fingerprint:
`b710ac1379268c460c2e7dfed223b3a9766c9177ee68a1b246ccbdd392454a4e`.

## Retained DPoS configuration metadata

A separate capped CF `8` metadata diagnostic exhausted the retained table within
its bound of 16 records and 64 KiB, using a validity lookahead to detect overflow.
It is explicitly separate from the 59-key proof contract. No other column family
or trie was enumerated. The [exact report](n4_retained_dpos_config.json) contains
one baseline period-zero row, encoded with an empty key, totaling **2,904 bytes**.
Its SHA-256 is `17c5ec372da3122ca291227ec073ef59b1cfef9fed1c93d3be314a9a0c87e9e0`.

Full canonical Go RLP structure was validated: 13 fields, all 20 initial-validator
records and their delegation-map shapes. Numeric update-period ordering selects
baseline zero at Q. Threshold, vote step, maximum stake and delay five all match
the candidate used for independent vote reconstruction. This corroborates those
four scalar values through retained metadata; the metadata is **not authenticated
by the state root**, and does not establish producer identity, PBFT settings or
hardfork configuration. Baseline blocks-per-year 8,523,243 is a different fact
from the independently derived head-period dynamic rate 9,275,294.

Source fingerprint: `4b9b06f02a17711dab913b9cfcfd17f56d894ae192dcc923c08875df90cc86cc`.

## Qualification boundary

The checked-in mainnet policy remains a candidate. Independent vote weights and
rate remove their previous dependence on expected reward output; they do not
qualify all producer settings or the reward transition. Historical typed reward
planner equality and raw byte-order mismatch remain separate observations.
No full native inventory, state execution or reward-root comparison is implied.

The following commands completed successfully on the preserved independent copy:

```sh
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml --bin independent_reward_inputs -- local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/independent-reward-inputs.json
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml --bin reward_rate_inputs -- local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/reward-rate-inputs.json
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml --bin retained_dpos_config -- local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/retained-dpos-config.json
```

Report paths are exclusive-create: use fresh filenames for subsequent runs.
Independent Astra medium review approved all three source fingerprints, exact
report observations and bounded qualification claims. Astra high confirmed the
historical delayed-state contract and full config codec against immutable Go.

## Validation

- 43 isolated qualifier tests passed, including bounded paths/proofs, canonical
  config/caps, reconciliation, factored certificate reconstruction and rate policy.
- Strict all-target qualifier clippy (`--no-deps -- -D warnings`), formatting and
  whitespace checks passed.
- Explicit `.githooks/pre-commit` completed `make rewrite-validate-fast`, including
  workspace format/clippy/tests and structural guards; 1,444 consensus unit tests
  passed. Logs are retained under ignored `local/evm-state-db/reports/`.
- This is Tier 1 offline diagnostic/pure-kernel validation. Storage-library and
  C++ sources are unchanged; no production routing or CMake rebuild is required
  for this scope. No broader differential or replay claim is made.
