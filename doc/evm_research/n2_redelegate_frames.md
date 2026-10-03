# Redelegation ABI and actual EVM frames

Baseline: `93c4c2917`. This bounded N2 step keeps the accepted authenticated
partial adapter and adds selector-first admission and fixed-word Go ABI decoding.
Gas funding, historical depth and nonpayability precede argument unpack. Quotes
are 80,000 gas, or zero for nonpayable calls. Dirty address high bytes and trailing
bytes are accepted; short arguments report the first missing 32/64/96-byte word.
The existing Rust admission, driver, journal and session owners remain in use.

The [exporter](../../experiments/evm_feasibility/native_redelegate_frames_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_frames_reference.py)
run 21 actual Go cases from both unchanged pins. The manifest records exporter,
harness, support and seed hashes. The seed is the actual first-call read output
from the accepted observation fixture, with explicitly synthetic total-vote and
principal rows (200 and 2,000). The two validators each have 1,000 principal from
`d1`; minimum deposit is 100, maximum stake 1,000,000. Period one activates
Magnolia/Ficus/Cornus at zero. Aspen part two is inactive except for the explicit
zero-amount failure case. The existing copy-only forwarding observer records
actual quotes/funding/depth, errors, logs, ordered raw writes and final accounts
and raw rows. Native diagnostic stdout is preserved separately. No supplied data
or original upstream file is changed.

Actual direct/nested/static partial success consumes 101,912/102,678/102,676
transaction gas. A parent revert keeps all 12 ordered native raw writes and
removes child logs; semantic state remains advanced. A second Rust call after
that revert authenticates the retained overlay and matches the accepted Go
repeat's ten writes. This second frame probe is a composition check of the
settled rollback contract and existing repeated-call oracle, not a new actual Go
repeated-parent-frame capture. Nested nonpayability quotes zero with 82,300 child
funding (including stipend); it restores the child transfer to DPoS and retains
the sender-to-wrapper transfer. Underfunded children do not call native Run and
retain their unspent native action gas. Malformed inputs obey funding/depth/value
precedence. At-fix missing-source and post-Aspen zero failures are also covered.

The consensus test covers all 21 quotes/ABI/native outcomes and ordered effects.
The real EVM integration test checks all 21 transaction results, native funding,
errors, value/static context, logs, dispositions, ordered effects at the port,
final per-key journal writes and raw/account state. Ordered operations are
validated before journal reduction; publication plans intentionally retain only
final values per key. Existing integration tests are unchanged.

First-run commands, full outputs and exit codes use `frames-`/`abi-` prefixes in
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Corrections: Go Aspen field names;
retain native diagnostic stdout; compare ordered effects at their port instead
of the reduced journal write plan; supply the synthetic Aspen maximum in the
Rust test profile. A failed edit script left source unchanged and its subsequent
unchanged test failure is retained. Nineteen targeted consensus tests, all ten
frame integration tests, affected-package clippy, dual-pin reproduction and ON
consensus bridge build12/all 15 tests pass. Serial workspace fast passes.

Independent Astra medium contract review accepted the observed ABI/frame scope
before frame implementation; frozen final implementation review accepted all
13 source/evidence hashes without blocking findings. Requested and confirmed
routes are Sol medium implementation, Luna medium bounded map and Astra medium
contract/final review. There were no routing failures. The final report is
`frames-review.md` in the artifact directory.
Actual historical DryRunner, estimation and direct supported structured traces
remain next. Reward-bearing/new-destination/full/zero successful paths and
historical same-validator success remain excluded. No production route, fallback,
broad campaign, upstream C++ or storage-module change is authorized by this step.
N1–N6 and Milestone 10 remain open.
