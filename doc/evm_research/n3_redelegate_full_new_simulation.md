# Complete H1 full-source/new-destination simulation

Base: `c9291c2a3`. One actual DryRunner case uses caller d1, source31,
destination32, amount1000, value/price0, gas200000 and supplied nonce2^512.
Caller genesis4000 stakes1000 only on31; a1 genesis2000 stakes1000 each31/32;
supply6000, zero rewards, both validators retained. The H1 seed sets d1 nonce
2^264+5 before the empty public Go commit; effective nonce is2^264+6.

The new [Go exporter](../../experiments/evm_feasibility/native_redelegate_full_new_simulation_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_new_simulation_reference.py)
leave shared owners unchanged. Only the new process assigns d1/a1 to support
address variables before configuration/seed creation. Exact config, actual
d1/a1/fe account presence/encoded bytes, native membership/principal/cursor facts,
complete physical rows and state root are exported. a1's ordinary account is
absent after all its funds enter native custody; its native principal remains.

Actual H1 has79 rows and root
`0759a99daeb9b0f0351a8c5a51cd53aa92bc898feed2f431b5653ef90d31224c`.
Both actual validator block1 reward nodes are absent. The exporter rejects a
present current node rather than altering the seed. Both Go pins independently
repeat the case and leave complete before/after snapshots identical. Success gas
is101912, output empty and one1000 event. Reproduction is byte-exact.

[Tests](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_full_new.rs)
materialize those complete actual rows and use the unchanged CompleteSeedReader.
A separate public semantic owner uses exact d1/a1/31/32 genesis accounts,
principal/order/metadata/config and finalizes empty H1 once. Restart loads head1
without refinalization. Ordinary account/nonce authority comes from the complete
Go physical reader; semantic-owner root/account parity is not claimed.

Eight simulations use two semantic-owner opens, two physical-reader opens and
two fresh requests per open. Each selects H1 via begin_native_simulation, checks
identity, actual current-node absence, caller/receiver/input/price/funding, supplied
nonce preservation and committed nonce+1, success/gas/output/logs. The current-node
guard stays active; successful historical H1 proves its semantic admission, with
no pendingH2 substitution. Public committed validators2000/1000, total3000,
caller exact[31] and a1 exact[31,32] with1000 each are checked before/after.
Concrete rows remain byte-identical after reader/session disposal.

First target passed; an added identity assertion initially used root instead of
state_root and failed compilation. The correction and node assertions pass.
Explicit empty-H1 timestamp was added before final re-record/reproduction; the
first logs remain. Runtime/bridge/storage/C++ owners are unchanged, so accepted
staged ON bridge12/all15 evidence remains applicable.

Actual final record/reproduction, target, EVM check/Clippy/all25 API/package/
serial fast/whitespace pass. Independent Sol review accepted all10 hashes with
no findings (`full-new-simulation-review.md`); final pending gates completed
(`full-new-simulation-gate-closeout.md`).
Records: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`,
prefix `full-new-simulation-`. Sol medium implements; Astra medium accepted the
exact H1 authority/price/current-node contract. No estimate/trace, real network
checkpoint, broader membership/current-node/reward profile, production route,
fallback or broad gate is accepted. N1–N6/Milestone10 remain open.
