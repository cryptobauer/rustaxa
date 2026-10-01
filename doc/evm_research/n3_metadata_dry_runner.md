# Metadata DryRunner and persisted simulation parity

Base: `1fe3282f2`. Objective: compare metadata through the actual pinned Go
DryRunner and the Rust historical simulation API over a complete persisted seed.
No runtime library, C++, storage module or production route changes.

## Contract and scope

The new exporter reuses `native_simulation_reference.go` for exact versioned Go
trie initialization at H=1, snapshot capture and actual `DryRunner.Apply` calls.
It reuses `native_validator_info_reference.go` for dynamic ABI construction.
The harness copies both sources into each disposable pinned archive and renames
only their unused `main` functions. All source and harness hashes are recorded.
The existing fixtures and original tests remain unchanged.

Both public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418` execute the real DryRunner.
The complete synthetic seed contains validator 0x31, owner 0xaa, empty metadata
and a committed 20-unit delegation. Rust reuses the same public FinalChain
finalization fixture for matching semantic history. Its concrete reader uses the
exact physical Go seed rows with fixture-bounded absence authority.

Nine cases cover byte replacement, empty/maximum strings, both length errors,
missing validator, malformed ABI, nonpayable malformed input, insufficient
native funding and intrinsic-gas rejection. Each Go case is applied twice with
the original supplied 512-bit nonce restored, and exact outputs must repeat.
Complete before/after snapshots must match. The Rust test opens the persisted
reader twice and runs two fresh simulations per case: 36 executions. It compares
the effective full-width stored nonce plus one, consensus versus execution
errors, gas, output and typed logs. Caller requests and reader identities remain
unchanged. All persisted physical rows are compared after each reader is dropped.
The matching semantic owner remains fixed across reader reopen; this slice does
not claim semantic-owner reopen or adoption of a real network checkpoint.

## Validation and review

Artifacts: `/home/fry/artifacts/evm-branch-2026-10-01-2233/`,
`metadata-dry-*` logs and exit codes. The first exporter used a gas cap below
intrinsic gas for its intended native-funding case. The first test exposed the
actual Go consensus error. That case remains as a separate intrinsic-gas case;
a 30,000 cap now tests insufficient native funding. No existing test or runtime
behavior was changed.

The complete `native_simulation_reference` target passed all six tests. Both
pinned new fixtures matched and reproduced byte-for-byte. Workspace fast gate
passed. Independent Astra medium review accepted the frozen sources/evidence
without findings. Previous bridge gates are recorded
in earlier reports; no new bridge gate is needed for test-only changes.

Requested and confirmed implementation route: Sol medium. Luna medium checked
the matching input contract and reported no external producer dependency or
routing failure. Requested and confirmed final review route: Astra medium.
Allowance remained 27% after validation/review; no billing is inferred.
N3 remains open for
estimation, tracing and remaining native/API behavior. N1–N6 and Milestone 10
remain open. This synthetic evidence does not certify historical supplied data.
