# Fixed45 closure and bounded evidence records

Status: independent final fact ACCEPT grants 45 new facts: 673 accepted,
241 residual and 10 complete voters. Independent seal verification passed.
Source acceptance, exact command READY, one guarded capture, both full replays
and four reverse checks passed.

The source commits are `2b11ca65ea6949b498ed572f118b514e9ca21377`
and `30c7f2c4861ae26667e51756e26faa5a4da0e2d9`. The second is the
exact source HEAD for capture. Both commits ran the actual pre-commit hook:
2146 tests passed in 235.81s and 230.02s.

The fixed45 source uses RCV4 with 51 constructor proofs and 45 payload proofs.
Both replays checked all 96 proofs and the fresh 71-proof prior. They produced
byte-identical reports and all 45 positive values. Full 48-request context and
both delayed omissions remain in the evidence. All 13 limits passed. All four
reverse controls stopped at Admission / Schema, with zero qualified payloads
and all 13 usage counters at zero. The capture marker is permanently consumed.
The three fixed directory metadata tuples match; this does not prove complete
database preservation.

The reviewed successor keeps full immutable evidence once and authenticates
each required body by role, path, byte size and SHA256. Finite lists preserve
order and repeated charges. The command has 376 roles; the outcome has 24,
for 400 total. Its 61 child occurrences charge 966010 bytes. Each verification
also reads the named 45182-byte parse body. Wrappers remain bounded at 131072 bytes. The exact
contract manifest has its separately reviewed 262144-byte bound. No physical
ceiling was increased. Old records, failures, archives and markers remain
unchanged. The verifier rejects missing or changed bodies, wrong roles,
conflicts, aliases, output collisions, cap overflow and partial acceptance.

Validation passed: 73 Python tests; the scope-covered Rust storage 16 focused
and 187 library tests; qualifier 11 focused and 194 package tests; fmt, clippy
and all-bin checks; the 12-job Rust-enabled storage bridge build and four bridge
tests; canonical parity; full native/first-D parity; six invalid-argv checks;
and the 2146-test fast gate and actual source hooks. Original test assertions
remain unchanged. Legacy fixture wiring uses exact archived instruction pins.

Detailed evidence is under
`/home/fry/artifacts/n4-evidence-01a13c53-b487-7dc2-954a-ba9af3ad695d-20261014`
(RUN) and
`/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth`
(P).

- Authority: RUN/astra/contract-manifest-v7-final.json,
  SHA256 `05cb4625567bf6f722a3aee20818fe0824aa8f8e6cbc9bb08b23035e8e722017`.
- Source acceptance: RUN/sol/source-v7-acceptance.md.
- Exact command: P/vrf-residual-fourth-voter-window-command-freeze.json,
  SHA256 `af2b7dd36cb614fc387d42e3dc715fabf12c5f20fb20aa278ef82cb3e38fa49a`.
- READY: P/command-ready-review.md and P/reverse-ready-review.md.
- Full replay reports: P/vrf-residual-fourth-voter-window-replay.json and
  P/independent-replay-report.json, each 57633 bytes,
  SHA256 `60fc9d1488e099aa9b01e245ca1c4a4052f0583ea1cdb116d0ec5913b6d9b914`.
- Reverse outcomes: P/reverse-outcome-record.json, 10388 bytes,
  SHA256 `bb1a7a40de22661f1e86dfc84ed0870975386d0334f0f17f074393904526e9a5`.
- Final fact ACCEPT: P/family-final-review.md, 10140 bytes,
  SHA256 `60c364be63912b5cf7fc3b1db2cef637bd67317fc4680fbf00927b303c6d8082`.
- Measured outcome seal: P/fact-outcome-freeze.json, 7638 bytes,
  SHA256 `15d7a7b44d81b12420130cc110ce9554fe2ccbcfecb5983afe86a4201ea00c81`;
  400 roles and 421993433 charged bytes.
- Independent seal ACCEPT: RUN/sol/final-seal-acceptance.md, 9109 bytes,
  SHA256 `91eba6e32c8332ecc810771174bacc5edf46a5a6430d501a7d97c54ae22d31d8`.
  The two late checker failures remain complete in their reviewed outcome and
  external postseal validation records. The external records do not change
  the sealed 400-role list or the 61 runtime children.
- Timing and routing: RUN/timing-and-routing.json. Requested Luna, Astra and
  Sol models all ran; actual worker model identities were not exposed. No
  routing failure occurred. Active review time and billing are not inferred.
- The separate documentation commit and actual hook are recorded in
  RUN/docs-commit.json and its full RUN/docs-commit.log.

This run started at 73% allowance and has a fixed 63% stop floor. Reuse
RUN/quota-budget.json on same-session resume; do not initialize it again.
No further residual command is READY under this contract. Further work needs
a named residual manifest, the corrected shared route, independent exact
command READY, full replay and final acceptance. Stake, jail, cache, owner,
production and real-replay gaps remain open. No push was made.
