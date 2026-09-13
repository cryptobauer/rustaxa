# Snapshot qualification tools

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
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml -- \
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
