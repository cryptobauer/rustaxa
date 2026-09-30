# Snapshot qualification tools

The September 30 persistent intake uses supplied `data/` and the exact independent
copy `local/evm-state-db/snapshot-litenode-copy/`. Only the lead database operator
creates/verifies the copy and runs database commands sequentially. Preserve supplied
bytes, use independent inodes or a COW clone, and keep databases/full manifests ignored.

Current bounded commands (run only after copy provenance has been checked):

```bash
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin bounded_qualification -- local/evm-state-db/snapshot-litenode-copy \
  local/evm-state-db/reports/bounded-qualification.json
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin reward_inputs -- local/evm-state-db/snapshot-litenode-copy \
  local/evm-state-db/reports/reward-inputs.json
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin native_inverse_coverage -- --head-sender-scout \
  local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/head-sender-scout.json
```

Stop dependent probes on incomplete/different pair identity. The bounded qualifier
performs four application point checks with Rust repositories, compares historical
genesis/head/header SHA/current and prior roots, then uses `ConcreteCheckpointReaders`
to verify the concrete descriptor and authenticate both roots. It performs no inventory.
Network identity means exact genesis equality; producer revision, exact checkpoint timing,
complete state and adoption remain unqualified.

The reward diagnostic preserves its seven application point reads and candidate-derived
weights, raw-byte comparison and typed comparison as separate facts. The scout first
checks embedded historical `n4_replay_preflight.json` bytes, period and all 19 fixed-sender
entries; this explicitly cites historical signature evidence. It requalifies the pair and
attempts exactly four logical storage reads, rejecting wrong widths before counting a
successful result. Absent, tombstone, history-unavailable and pruned outcomes stay distinct.
No child enumeration, fresh signature validation, live membership authentication,
semantic completeness, publication or production authority is claimed.

The shared guard validates the exact copy and both DB children before any open, rejects
symlink components and source/output overlap, and requires new report files. Opens are
read-only and publication uses exclusive creation. Use fresh output filenames for reruns.
Tests use synthetic filesystem metadata and closures; they do not open supplied databases.

Everything below records the historical S0 workflow. Its `/tmp` paths and broad inventory
commands are obsolete and are not continuation commands. `rustaxa-snapshot-qualifier`,
`head_replay_inputs` and `replay_preflight` retain their obsolete fail-safe path guards;
this slice does not authorize their use or a broad inventory run. The current inverse
binary's inventory mode shares the persistent guard but is also outside this slice.


These tools qualify a preserved pair of Taraxa application and concrete-state databases without opening the supplied
evidence through RocksDB. Use a workspace-local independent copy and keep the copy plus full manifests outside Git.

```bash
mkdir -p .snapshot-work
cp -a --reflink=never /tmp/snapshot-litenode .snapshot-work/snapshot-litenode-copy
python3 experiments/evm_feasibility/snapshot_manifest.py \
  /tmp/snapshot-litenode .snapshot-work/source-content-manifest.jsonl \
  --compare .snapshot-work/snapshot-litenode-copy \
  --compare-manifest .snapshot-work/copy-content-manifest.jsonl \
  >.snapshot-work/content-manifest-summary.json

CARGO_TARGET_DIR=.snapshot-work/cargo-target cargo run --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin rustaxa-snapshot-qualifier -- \
  .snapshot-work/snapshot-litenode-copy .snapshot-work/snapshot_qualification_raw.json
```

Use fresh output paths: the tools refuse existing report, manifest, object and executable files.
The qualifier resolves database child paths and refuses `/tmp/snapshot-litenode` and its descendants.
Report and manifest outputs must remain outside both input trees. It lists and opens every column family read-only,
pins the application head and state descriptor, compares their roots, records observed key extrema, and exports exact
physical fixtures. Extrema do not establish continuity. Root-node presence does not establish trie closure, and
physical version rows are not treated as membership proofs.

The mainnet identity helper reuses the already configured `/build` compile/link inputs without building or modifying
that tree. Its object and executable are written to `.snapshot-work/`:

```bash
python3 experiments/evm_feasibility/snapshot_mainnet_genesis.py \
  --build /build --output .snapshot-work/mainnet_genesis_hash
```

The committed compact result lives in `doc/evm_research/snapshot_qualification.json`; the raw report and full content
manifests remain local evidence.

The bounded head replay-input helper uses the Rust storage repositories to
point-read one fixed period, verifies ordered transaction and receipt roots
against its typed FinalChain header, and opens the prior concrete descriptor.
It does not replay the period or scan retained ranges:

```bash
CARGO_TARGET_DIR=.snapshot-work/cargo-target cargo run --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin head_replay_inputs -- \
  .snapshot-work/snapshot-litenode-copy \
  .snapshot-work/head_replay_inputs_rerun.json

cmp .snapshot-work/head_replay_inputs_rerun.json \
  doc/evm_research/s0_head_replay_inputs.json
```

The generated report separates the complete bounded head bundle from missing
configuration, certificate-vote, and prior-state closure gates. It refuses to
overwrite an output or place one inside either snapshot database.
