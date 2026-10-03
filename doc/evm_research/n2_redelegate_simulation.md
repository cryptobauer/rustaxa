# Actual redelegation historical simulation

Baseline: `13bb0b511`. This test/evidence-only N3 step composes the accepted
partial adapter through existing `simulate_with_native`, real disposable
historical sessions and concrete checkpoint readers. Runtime owners are unchanged.

The exporter and harness `native_redelegate_simulation_reference.{go,py}` run
actual DryRunner.Apply from both unchanged Go pins. The manifest records all
source/configuration/input/output identities. The synthetic committed H=1 seed
has two validators (31/32), each with aa principal 1,000; minimum 100, maximum
1,000,000, zero rewards and active Magnolia/Ficus/Cornus. Empty public Go state
transition commits H=1. Request nonce is 2^512, committed sender nonce is
2^264+5; effective nonce is committed+1, and each immutable request is repeated.
Ten cases cover partial success, missing source/destination, insufficient source,
minimum remainder, same-validator failure, short ABI, nonpayable short ABI,
native out-of-gas and intrinsic consensus failure. Successful/normal failure
gas is 101,912; malformed ABI 101,272, nonpayable 21,272, native out-of-gas
21,912 and intrinsic failure 21,000. Full outputs, logs, errors and exact committed
physical rows/root match on both pins and remain unchanged after all probes.

The Rust test materializes the exact Go trie. CompleteSeedReader authenticates
trie nonmembership only for this complete synthetic seed. The semantic owner
independently establishes H=1 through public Rust finalization, then is destroyed
and reconstructed without re-finalizing. Two physical reader reopen cycles per
owner and two repetitions per ten cases exercise 80 fresh real historical
sessions. Each result, effective nonce, immutable request and concrete identity
matches Go. Exact physical rows remain unchanged; public semantic reads verify
each validator stake and aa delegation remains 1,000, with zero pending rewards.
Total principal alone is not used as the disposal proof.

First commands/full outputs/exit codes have `simulation-` prefixes under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Corrections: require `--all`
for workspace cargo fmt (first unsupported invocation retained); strengthen
semantic disposal assertions after contract review; format the new as_chunks
line wrap (the intermediate fmt-only failure is retained). All 13 historical API
integration tests, affected-package clippy, actual dual-pin reproduction and
serial workspace fast pass. There is no runtime, bridge or storage-module change
in this slice. Previously accepted ON bridge12/all15 remains the runtime gate.
Independent Sol medium review accepted all ten frozen source/evidence hashes
with no blockers; the final formatted fast gate covers the frozen source.
The report is simulation-review.md in the artifact directory.

Routes: Sol medium direct implementation; Luna medium prior API map; Astra
medium bounded historical contract review, recorded in simulation-contract-review.md.
No routing failures. This evidence does not qualify real network history or
production replacement. Estimation and supported direct structured traces follow;
N1–N6 remain open, with unsupported adapter success paths unchanged.
