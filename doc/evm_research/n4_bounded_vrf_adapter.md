# VRF public API notes

Scope: accepted first20 and first four complete remaining47 voter profiles. These notes describe complete public units from the reviewed source. They introduce no code change or physical execution authority. Existing old96 diagnostic APIs, wire profile, tests and accounting remain unchanged.

## Storage plans and results

`ConcreteVrfPlan` binds the complete first delayed-period family. `new(context: ConcreteDiagnosticPlan, voters: [[u8;20];20])` validates the fixed full context and requires strictly ascending distinct voters. It derives20 immutable tuples at the first delayed identity, FE address ending in fe, and Keccak256(0004 || voter). Invalid context, duplicates or voter order return a typed witness error before any opener. Construction validates shape only; callers must independently authenticate raw votes and roots. Private fields prevent subsequent context or tuple replacement. `context()`, `voters()` and `tuples()` return immutable borrows of the complete fixed data.

`ConcreteVrfVoterPlan` binds one voter across the remaining47 delayed periods. `new(first: ConcreteVrfPlan, voter: [u8;20])` consumes a validated first20 plan, requires that voter in its fixed voter array, preserves its full context and derives47 tuples from delayed[1..] in order with the same FE address/key. An unknown voter returns `ConcreteWitnessError::Request`. No count, period, root, key or profile override exists. The constructor does not authenticate raw authority or execute the prior proof. `context()` and `tuples()` return immutable borrows; `voter()` returns the fixed20-byte identity.

`ConcreteVrfReplay` exposes20 logical results,20 physical provenance results and complete replay usage. `ConcreteVrfVoterReplay` exposes the corresponding47-element arrays and usage. These result values exist only after complete successful family replay. Logical `Member` contains exactly32 bytes; zero is valid stored data but not a usable VRF key. Logical `NonMember` requires the authenticated trie path. Physical Present/Tombstone/HistoryUnavailable alone grants no logical verdict. Unavailable history under these strict profiles is an error and cannot supply a value or nonmembership result.

## Storage capture and replay functions

`capture_vrf_run(application: &Path, state: &Path, plan: &ConcreteVrfPlan, limits: ConcreteWitnessLimits)` captures the first20 RCV1 family. `capture_vrf_voter_run(application: &Path, state: &Path, plan: &ConcreteVrfVoterPlan, limits: ConcreteWitnessLimits)` captures the remaining47 RCV2 family. Both return `Result<ConcreteDiagnosticCapture, ConcreteDiagnosticFailure>`. Success contains the complete canonical bundle and charged usage; it is capture evidence and still requires offline replay. Failure contains the first phase/index/identity/tuple/last-read evidence where available and current usage, with no accepted prefix.

Capture uses the unchanged bounded read-only owners: one application open, then two sequential state opens. Four fixed application rows authenticate genesis/head/parent. Both descriptors bind the fixed head. Constructor identities are [parent,head] and [all48 delayed identities,head], giving51 registrations and50 distinct identities. Only the second state owner supplies payload proofs. Missing nodes, malformed values, Error3, inconsistent rows, descriptor/schema errors or ceiling failures stop the family at its first failure. No writable open, inventory, scan, fallback or retry is part of these APIs.

`replay_vrf_bundle(bytes: &[u8], plan: &ConcreteVrfPlan, limits: ConcreteWitnessLimits)` returns `Result<ConcreteVrfReplay, ConcreteDiagnosticFailure>`. `replay_vrf_voter_bundle(bytes: &[u8], plan: &ConcreteVrfVoterPlan, limits: ConcreteWitnessLimits)` returns `Result<ConcreteVrfVoterReplay, ConcreteDiagnosticFailure>`. These functions have no filesystem, owner or network input. They admit the complete envelope, authenticate the fixed pair/descriptors and all51 constructor proofs, bind each ordered payload tuple, reconstruct logical paths and require full frame/proof consumption. RCV1 has71 proofs and20 payloads; RCV2 has98 proofs and47 payloads. Wrong profile/header/count, roots/keys/identity/order, conflicts, duplicates, unused records, trailing bytes and incomplete proofs return typed first-failure evidence. No partial result escapes.

All new storage types/functions are exported through `concrete_state::witness`, `concrete_state` and the `rustaxa-storage` crate root. Generic family kernels and wire profiles remain private. `ConcreteWitnessLimits` may lower existing ceilings; it cannot raise them. One monotonic budget charges before work and remains sticky within each complete family. Default ceilings remain: KeyBytes64; RowBytes1048576; RawCopyBytes67108864; CanonicalBytes67108864; EncodedBytes134217728; Operations100000; Visits100000; Records100000; FrameInspections300000; ConstructorIdentities49; Registrations51; DistinctIdentities50; PayloadTuples96. The payload counts20 and47 do not reduce or increase the shared96 ceiling. Old96, prior first20 and remaining47 replay budgets are distinct complete families; no internal reset is permitted.

## Qualifier modules and command entrypoints

Public module `vrf_witness` owns the accepted fixed first20 qualification profile. Its `preflight(output: &Path)`, `capture(bundle_output: &Path, report_output: &Path)` and `replay(bundle_input: &Path, report_output: &Path)` return `anyhow::Result<()>`. Each invocation authenticates the unchanged raw-six owner, original signed vote/root checks, complete voter manifest and fixed first20 selection. Preflight publishes authority/request/source/limit evidence without a DB open. Capture publishes a complete RCV1 bundle plus an unqualified capture report. Replay admits a bounded offline bundle outside DB roots and publishes20 values only after full verification. Its public `PROFILE` constant names the fixed first-period profile. Its19-entry compiled source profile and65536 report cap remain unchanged. Crate-private `selection` and `sources` now permit narrow reuse by the47 module. Crate-private `decode_values<const N>` shares canonical member/nonmember semantics; the existing first20 decoder calls it with20 and retains its prior output behavior.

Public module `vrf_voter_window` owns the fixed remaining47 profile. It exposes the same `preflight`, `capture` and `replay` signatures and public `PROFILE`/`REPORT_CAP` constants. Each invocation calls the raw-six owner once, derives the first20 plan from that same authority, recomputes all48 voter membership and exactly10 eligible candidates, fixes voter138c1b9baea7d93a981ba694f5318f735c997298 and preserves all47 original source rows at the contracted indices. It excludes the first delayed period. It independently admits the fixed417594-byte prior first20 bundle/SHA2566382e8f25cc9ec2225e563111e37b9ad8c8beae4167543cb4d9e915a7d5cbb61 and actually replays it before any opener. Prior index1/proof52 must prove the selected32-byte nonzero Member with matching physical Present bytes. Only then does it consume the first20 plan into the47 plan. Prior errors retain typed debug evidence and prevent the next stage.

The47 report retains prior input pin/usage/semantic commitment and selected first-period value, without the full prior report. It reports raw-six accounting separately and combined7 reads/6785636 bytes. The old owner remains6 reads/6368042 bytes with16 Aspen source reads/465085 bytes. Its compiled source profile is22 entries plus the unchanged25-entry owner profile, each source at most1MiB and combined source bytes at most4MiB. New report cap131072 is checked before capture and at exclusive publication; bundle cap134217728 remains unchanged. Executable cap268435456 and hash buffer8192 remain unchanged.

Both modules reject existing/overlapping output files and require publication outside DB roots. Reports use bounded encoding and exclusive publication. A capture failure publishes only bounded first-failure evidence, values=null and false qualification, then returns an error. A complete capture report remains unqualified. Only complete replay creates all values and sets `vrf_family_qualified=true`. Report or publication failure returns an error; it does not authorize a new attempt. The47 capture retains fixed path validation before authority, but prior replay precedes every opener. Its preflight performs no DB-path inspection. Offline replay performs no DB inspection.

`live_vrf_witness` and `live_vrf_voter_window` accept exactly `--preflight REPORT` or `--bundle-output BUNDLE --report-output REPORT`. `replay_vrf_witness` and `replay_vrf_voter_window` accept exactly `--bundle-input BUNDLE --report-output REPORT`. Each entrypoint calls the corresponding module and returns errors to the process. New frozen binary names are live-vrf-voter-window-v1 and replay-vrf-voter-window-v1. The live command has no authority input/root/voter/count/limit override. Offline replay permits only its bundle evidence path override. Invalid argument count/order/name fails before authority admission.

Qualification remains limited to the complete named VRF fact family. It does not qualify stake, jail, voter weights, reward cache equality, owner readiness, producer verification, public finality, real19-transfer behavior or production routing. A remaining47 failure cannot qualify a prefix. A successful remaining47 replay leaves847 VRF and1828 other missing rows,2675 total. A separate frozen guard and exact-command READY review remain prerequisites for any physical attempt.



## Additional fixed voter modules

`vrf_next_voter_window`, `vrf_third_voter_window`, `vrf_fourth_voter_window`,
`vrf_fifth_voter_window` and `vrf_sixth_voter_window`
export `preflight(output: &Path)`, `capture(bundle_output: &Path, report_output: &Path)`
and `replay(bundle_input: &Path, report_output: &Path)`, each returning `anyhow::Result<()>`,
plus fixed `PROFILE` and131072-byte `REPORT_CAP`. They use the same raw-six owner,
immutable Rust plan, consuming capture/replay kernels, full47-row selector and
exclusive output/error rules documented above. Their fixed bindings are:

| Module | Voter | Complete selector position | First20 index/proof |
| --- | --- | --- | --- |
| vrf_next_voter_window | 264a30e880d3b53bb2cdeb234e34201ebf9a9965 | second | 2/53 |
| vrf_third_voter_window | 329d40b5f15cfe4de0e9953d0599d2a39a65fc04 | third | 3/54 |
| vrf_fourth_voter_window | 575910424ac437467726542781508b2220b71ce8 | fourth | 6/57 |
| vrf_fifth_voter_window | 6bf44c438f6328640b0bc5d7d7766310cf49610f | fifth | 7/58 |
| vrf_sixth_voter_window | a6cec53b4d7920709f05ca1f7ac0d66338d29cb9 | sixth | 10/61 |

Each selector requires exactly ten complete voters and its full fixed lexicographic
prefix. All47 original manifest indices, source references and delayed periods
must match. Each invocation freshly executes all71 prior first20 proofs, then
requires the named prior nonzero32-byte Member/Present equality before any opener.
No previous47 bundle supplies authority. Sources remain22 qualifier plus25 owner,
with1MiB per-file/4MiB combined ceilings. Old96, first20 and new47 have separate
complete budgets; the13 storage ceilings and RCV2 framing are unchanged.

Each module has corresponding `live_vrf_*_voter_window` and
`replay_vrf_*_voter_window` bins with the same exact argument forms. Invalid overrides
fail before admission. Binaries bind a specific voter; wire framing alone does not.
Tests and reverse controls reject another voter's bundle at Identity/Payload/index0,
before PayloadTuples charging. Sixth-source tests cover all five earlier complete-voter bundles.

Second through sixth complete-voter families are independently accepted,
bringing the total to302. The seventh candidate has fresh conditional Astra/Sol
design reviews; implementation and physical gates remain open. Third guard
metadata admission is68KiB; fourth through sixth is128KiB. Normal metadata remains
64KiB, with one reviewed exact87841-byte immutable historical map exception whose
SHA is checked before parsing. These artifact caps do not raise DB, source, report,
sink, result or timeout ceilings. Current acceptance and blockers are in the
checkpoint. Historical counts above describe the first complete voter only.
