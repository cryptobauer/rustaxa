# Redelegation reference observations

This is reference preparation for N2, not staged Rust redelegation acceptance.
The existing session still rejects `reDelegate(address,address,uint256)`.
The [exporter](../../experiments/evm_feasibility/native_redelegate_observation_reference.go)
executes actual Go `StateTransition` transactions from both pinned archives.
The [harness](../../experiments/evm_feasibility/native_redelegate_observation_reference.py)
records 12 cases and 13 attempts with exact input, nonce, gas, errors, output,
logs, separate ordered storage-read and irreversible-write vectors, and final
source/destination stake and total principal. No supplied data is used.

## Profile and observation boundary

Synthetic period 1 activates Magnolia, Ficus and Cornus at zero; the redelegation
fix is zero. Aspen part two and Cacti remain inactive. Two genesis validators,
`31` and `32`, each have 1,000 principal from caller `d1`, zero rewards and
commission 100. Minimum deposit is 100; the destination-cap case uses maximum
stake 1,500. All other cases use 1,000,000. The genesis balances and supply come
from the reused custody profile with explicit two-validator adjustments.

Read observations are completed synchronous `EVMStateStorage.GetAccountStorage`
requests. They include address, key, copied value and whether the callback ran.
They are not all native API lookups: Go caches native rows in the same block.
Write observations use the existing synchronous copy-only observer before
`Account.SetStateRawIrreversibly`. Both observers exist only in disposable Git
archives. Target source hashes and observer hashes are guarded and recorded in
the fixture manifest. Original upstream files are unchanged. Separate vectors
do not establish read/write interleaving.

Each pin also executes a control with both observer forwarding paths absent.
The full exported execution fields, logs and final stakes match the observed
run after only the two observation vectors are removed. This checks observer
noninterference for this corpus, not arbitrary future execution.

## Results and implementation constraints

| Case | Result | Storage reads | Irreversible writes |
| --- | --- | ---: | ---: |
| First partial 300 | success; source 700, destination 1,300 | 14 | 12 |
| Repeat partial 300 in same block | success; source 400, destination 1,600 | 0 | 10 |
| Destination cap plus insufficient source | destination-cap error first | 4 | 0 |
| Missing source validator | validator error | 1 | 0 |
| Missing destination validator | validator error | 3 | 0 |
| Missing source delegation | delegation error | 5 | 0 |
| Insufficient source or remainder below minimum | insufficient delegation | 5 | 0 |
| Same validator, insufficient native gas, nonpayable | early error | 0 | 0 |
| Zero before Aspen part two | success; principal unchanged | 14 | 12 |
| Full source | success; source 0, destination 2,000 | 16 | 25 |

The successful partial attempts use 101,912 transaction gas and an 80,000 native
action quote. Total delegated principal remains 2,000 in every case. Normal
failures have no native writes or logs. Zero and full-source success are oracle
boundary facts; they are outside the proposed first staged adapter scope.

The first five successful read requests authenticate source validator, source
rewards, destination validator, destination rewards and source delegation, in
that order. Destination-cap validation occurs before the fifth read. Subsequent
reads include current reward nodes, shared old head/cursor nodes, destination
delegation and validator/delegation membership positions. Source writes complete
before destination writes. The first call writes each old shared head/cursor
key twice: count one, then deletion. The repeated call writes current nodes
twice: count one, then count three. Preserve these exact bytes and repetitions;
do not replace them with final-state-only comparison or deduplicated writes.

Independent Astra contract review requires normal preflight error precedence
before successful-transition scope rejection. Zero reward pools alone do not
prove zero payout: authenticate cursor/head/current reward indices and counts.
Use the existing business kernel, one raw trace, separate source/destination
node traces, and no call-level terminal global writes. A narrow account-port
conversion of the two existing kernel parameters is the next implementation
dependency. The complete staged adapter must still settle its authenticated-read
contract explicitly; it cannot claim full Go API or cache parity from these
storage vectors alone.

## Validation and limits

Both pins reproduce byte-identical observations. Both uninstrumented controls
match execution fields. The first harness assertion incorrectly required reads
on the cached second success; the actual output exposed that error and the
corrected harness now checks first/repeated counts explicitly. Complete first,
corrected, control and reproduction logs are retained under
`/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

Workspace fast checks passed. Frozen independent Astra medium review accepted
all six source/fixture/report hashes without findings. Requested and confirmed
routes were Sol medium implementation and Astra medium review; Luna's prior
bounded map identified this dependency. There were no routing failures.
No Rust or C++ implementation changed in this reference slice. Reward-bearing,
new-destination, raw-reader failure/corruption, post-Aspen-zero rejection,
historical same-validator correction, API simulation and rollback coverage
remain future gates. N1–N6 and Milestone 10 remain open.
