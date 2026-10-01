# Native metadata gas-estimation composition

Base: `abd7a111d`. Objective: pair actual Go DryRunner probes with the existing
C++ gas-search body and fresh real Rust historical native sessions. Test/oracle
changes only; no runtime, storage-module, C++, RPC or production-route changes.

## Independent references

The new Go exporter reuses the complete native H=1 seed and actual DryRunner
helpers unchanged except for an unused `main` rename in each disposable archive.
It takes the nine requests from the completed metadata DryRunner corpus. Each
candidate gas limit is executed twice, restoring the original 512-bit nonce;
outputs must repeat and the complete committed before/after snapshots must match.
The candidate transcript is not accepted as an estimator oracle by itself.

The Python harness extracts the exact existing search body from `Eth.cpp`, as
in the earlier independent estimator reference. It compiles that unchanged body
with a callback that consumes the actual Go transcript and checks every requested
gas limit. It checks full transcript consumption and records the C++ result or
error. Thus a wrong candidate sequence fails against the original algorithm.
Its search SHA-256 is `7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759`.
Both pinned Go artifacts must match and reproduce together with the C++ output.
The manifest binds both exporters, the request input, harness, search body and
all three outputs. Existing corpora remain unchanged.

There are 24 probes across nine cases. Successful replacement, empty and maximum
metadata each need six probes. Six initial error cases terminate on endpoint
length, missing owner, ABI, payability, native funding or intrinsic gas. Exact C++
estimates are 44,905, 43,913 and 54,081 respectively. This corpus does not claim
midpoint code-failure coverage; the existing independent search corpus covers
that policy separately.

## Rust composition and checks

The new persisted integration test uses `estimate_gas` and, for every callback,
`simulate_with_native` with a new real historical native port. It compares exact
candidate limits, effective nonce, consensus/execution error class and text,
gas, return bytes and typed logs against Go; final result/error and consumed
transcript length against C++. Four searches per case span two physical reader
reopens: 96 real native simulation probes. Requests, selected identity and all
persisted rows remain unchanged. The matching semantic owner stays fixed.
This is synthetic fixture authority, not qualification of supplied network data.

The first targeted test passed. Full logs and exit codes are retained under
`/home/fry/artifacts/evm-branch-2026-10-01-2233/metadata-estimate-*`.
Dual-pin/C++ reproduction and workspace fast checks passed. Independent frozen
Astra medium review accepted the sources and evidence without findings.
Only Rust test and oracle/documentation files change, so earlier bridge gates
remain historical evidence rather than a newly executed requirement.

Requested/confirmed implementation route: Sol medium. Independent final reviewer:
Astra medium, confirmed. There were no routing failures or correction batches.
Allowance was 26% after validation/review; this is not a billing measurement.
N3 still requires tracing, RPC composition and remaining native
coverage; N1–N6 and Milestone 10 remain open.
