# Complete H1 full-source/new-destination simulation

Base: `db08e8798`. One actual DryRunner case uses caller d1, source31,
destination32, amount1000, value/price0, gas200000 and supplied nonce2^512.
Caller genesis5000 stakes1000 on31/33; a1 genesis3000 stakes1000 each31/32/33;
supply8000, zero rewards, all three validators retained. The H1 seed sets d1 nonce
2^264+5 before the empty public Go commit; effective nonce is2^264+6.

The new [Go exporter](../../experiments/evm_feasibility/native_redelegate_swap_append_simulation_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_swap_append_simulation_reference.py)
leave shared owners unchanged. Only the new process assigns d1/a1 to support
address variables before configuration/seed creation. Exact config, actual
d1/a1/fe account presence/encoded bytes, native membership/principal/cursor facts,
complete physical rows and state root are exported. a1's ordinary account is
absent after all its funds enter native custody; its native principal remains.

Actual H1 has99 rows and root
`fbb05f2ff5298016408edf003d39f99bbfbd4bd84d3f917e44ae04d9114f7a97`.
All three actual validator block1 reward nodes are absent. The exporter rejects a
present current node rather than altering the seed. Both Go pins independently
repeat the case and leave complete before/after snapshots identical. Success gas
is101912, output empty and one1000 event. Reproduction is byte-exact.

[Tests](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_swap_append.rs)
materialize those complete actual rows and use the unchanged CompleteSeedReader.
A separate public semantic owner uses exact d1/a1/31/32/33 genesis accounts,
principal/order/metadata/config and finalizes empty H1 once. Restart loads head1
without refinalization. Ordinary account/nonce authority comes from the complete
Go physical reader; semantic-owner root/account parity is not claimed.

Eight simulations use two semantic-owner opens, two physical-reader opens and
two fresh requests per open. Each selects H1 via begin_native_simulation, checks
identity, actual current-node absence, caller/receiver/input/price/funding, supplied
nonce preservation and committed nonce+1, success/gas/output/logs. The current-node
guard stays active; successful historical H1 proves its semantic admission, with
no pendingH2 substitution. Public committed validators2000/1000/2000, total5000,
caller exact[31,33] and a1 exact[31,32,33] with1000 each are checked before/after.
Concrete rows remain byte-identical after reader/session disposal.

Actual record is byte-identical across both pins. Target/reproduction/all28 API/package/
check/Clippy/serialfast/whitespace passed. Independent same-profile Sol medium
accepted all10 frozen hashes with no findings (`swap-append-simulation-review.md`).
Final gates and doc hash are in `swap-append-simulation-closeout.json`.
Runtime/bridge/storage/C++ owners are unchanged; retain staged ONbridge12/all15.
Records: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`,
prefix `swap-append-simulation-`. Sol medium implements; The accepted exact H1 authority/price
contract governs this mechanical topology derivative. No estimate/trace, real network
checkpoint, broader membership/current-node/reward profile, production route,
fallback or broad gate is accepted. N1–N6/Milestone10 remain open.
