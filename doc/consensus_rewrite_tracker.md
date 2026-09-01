# Consensus Rewrite Tracker

This file is the live execution queue for consensus rewrite work. `PLAN.md` owns stable architecture and scope,
`doc/consensus_bridge_shim_audit.md` owns the mechanically checked bridge inventory, and
`doc/rewrite_validation_strategy.md` owns reusable validation policy. Completed implementation history belongs in git,
not in this tracker.

## Stable Completed Outcome

The native consensus campaign through `CRW-18`, `CRW-N01`, `CRW-E01`, and `CRW-E02` is complete:

- one native `ConsensusApplication` owns consensus construction, restoration, state, scheduling, finalization,
  publication, network decisions, and public mutation admission;
- `ConsensusNetworkApi` owns consensus packet inspection, routing, peer selection, packet construction, effect identity,
  retry, and cancellation, while C++ retains physical tarcap mechanics;
- `ConsensusQueryApi` owns bounded public consensus reads used by RPC, GraphQL, debug/Test RPC, stats, and the light
  plugin;
- concrete FinalChain execution uses committed StateAPI roots for every Rust-mode finalized period, with fail-closed
  provenance, crash recovery, timestamped backup/full resync, and multi-node root/hash agreement;
- Rust-mode manager, storage, PBFT/vote/pillar/DAG/transaction/proposer, `FinalChain`, and `DbStorage` facades and every
  consensus shim directory are deleted; and
- pure-C++ source selection remains the upstream-compatible reference composition.

Retained C++ operations are external leaves, not unfinished consensus ownership: application process mechanics,
signing, VDF execution, physical tarcap transport, concrete EVM/`state_db`, public-client formatting, administration,
and storage conformance. Their named clients and deletion conditions are recorded only in the live bridge inventory.

## Remaining Consensus Work Queue

No ready, blocked, or active consensus rewrite item exists. A new item requires a demonstrated unclassified production
route, parity regression, or newly authorized ownership boundary; do not recreate completed migration work.

## CRW-18 Closeout Evidence

The checked bridge inventory is 4,965 Rust bridge lines, zero shim lines, 83 CXX functions, 132 CXX carriers, ten
opaque handles, zero shim directories, zero granular flags, zero partial-service factories, zero compatibility
constructor calls, and 17 non-test C++ consumers. Every retained handle, module, export family, and consumer has a named
external client and narrowing or deletion condition in `doc/consensus_bridge_shim_audit.md`.

The 2026-09-01 closeout passed the fast gate (including 1,338 native consensus tests), focused consensus, FinalChain,
storage, storage-conformance differential, startup/smoke, concrete-root E02, isolated pure-C++ FinalChain parity, the
complete two-test Python/five-node full-node suite, and full CTest (10/10). The CTest Go-state leg was reproduced in the
current container with the exact `zlib1g-dev` and `libsnappy-dev` runtime/link libraries declared by `Dockerfile`; the
dependency requirement is also documented in `doc/building.md`.

The repo-wide `check-static` command was run and remains non-green on the pre-existing cppcheck baseline: 20 findings in
unchanged C++ sources, including the known moved-`vote` warning in legacy `pbft_manager.cpp`. This closeout changes no
C++ production source and does not weaken, suppress, or retarget that gate. Inventory self-tests, exact-set guards,
whitespace checks, stale-reference searches, and independent review completed after the documentation contraction.

## Future Changes

Do not add a new consensus rewrite item merely to reduce line counts. Add one only when a named client can migrate, an
external boundary is explicitly authorized to move native, or a correctness/parity gap is demonstrated. Update the
bridge inventory in the same commit whenever a CXX function, carrier, handle, module, consumer, classification, or
deletion condition changes.
