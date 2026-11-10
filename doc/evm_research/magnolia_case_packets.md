# Magnolia case packets

The 20 crypto qualification tests need separate API, C++, roster and rules
bodies. Bulk execution fixtures do not supply these bodies. Keep all packet
bodies, producer outputs and databases under `/home/fry/artifacts`.

Set `RUSTAXA_CRYPTO_PACKET_INDEX` to an external JSON file and set
`RUSTAXA_CRYPTO_PACKET_INDEX_SHA256` to its reviewed SHA-256. Do not also set
`RUSTAXA_CRYPTO_PACKET_MANIFEST`. Each test selects its own case without a
process environment change. The old single-manifest route remains available
for one focused case.

The index has `schema: 1`, `status: "ACCEPTED"` and a `cases` object. Each case
key maps to an object with these fields:

- `fixture_sha256`: decoded execution fixture SHA-256.
- `source_epoch` and `build_identity`: nonempty strings from the reviewed producer.
- `manifest`: an absolute `path`, `bytes` and `sha256` pin for that case manifest.
- `review`: an absolute `path`, `bytes` and `sha256` pin for the independent review.

Recovery adds a `recovery` object to each case entry. Its `primary` and `retry`
objects each have a pinned phase `manifest` and independent `review`. Keep the
ordinary `[9]` packet unchanged. The primary witness must physically reopen after
period 5; the retry witness must physically reopen after periods 4 and 5.

Each phase review binds `case`, `phase`, `ordinary_fixture_sha256`,
`manifest_sha256`, `source_epoch`, `build_identity` and `producer_evidence`.
Phase source and build identities can differ from the ordinary API producer.
The loader checks both phase bodies, exact lifecycle events and schedules,
non-null configuration/genesis/registry/inputs, all 12 complete transaction
arrays, and all 13 roots. Distinct phase hashes are required. A different restart
schedule can change native cleanup writes. Do not copy those expected writes
from Rust or drop the occurrence check.

The review document must have `status: "ACCEPTED"`, `case`, `fixture_sha256`,
`source_epoch`, `build_identity` and `manifest_sha256`. These values must match
the selected entry. Its nonempty `producer_evidence` array contains absolute
`path`, `bytes` and `sha256` pins. All pins are checked before qualification.
A hash authenticates reviewed bytes; an ACCEPTED label alone gives no authority.
Only publish the index hash after independent review of the actual producer
captures, reference revision, source closure, tools, build and case inputs.
Do not turn a draft result into an accepted packet by changing its label alone.

Each selected manifest keeps the existing packet contract: `status: "ACCEPTED"`,
exact case, fixture, source and build identities, plus `streams` pins for `api`,
`cpp`, `roster` and `rules`. Stream filenames must be local basenames. The roster
must bind the fixture and rules hash. A gzip stream has `encoding: "gzip"`.
Its `bytes` and `sha256` fields pin the decoded body; `stored_bytes` and
`stored_sha256` pin the gzip file. Both stored and decoded sizes are limited to
256 MiB. The loader checks the stored pin before bounded decoding, then checks
the decoded pin. Missing encoding keeps the plain JSON contract. The consumer checks all 13 heads, ordered
requests, three estimates and their actual ordered probes, and all required
trace inputs and results. Do not fill expected results from Rust output.

Each qualification still runs the 12-period workload, reopen after period 9,
and recovery split at period 5 with BeforeCommit and AfterCommit faults and
reconciliation. An accepted packet does not count as a passed recovery test. Recovery uses
independent fresh stores and runs before API comparisons. A `recovery_only`
receipt records that scope; API failure still fails the full qualification.

For a normal hook run, use a new retention launch manifest, scratch parent and
ledger. Set `RUN_ARTIFACT_DIR` and `RUSTAXA_ERA_LAUNCH_MANIFEST` to this launch.
Run `.githooks/pre-commit` without bypass flags. Keep complete failed outputs
and databases. A focused pass does not replace whole-hook acceptance. The four
accepted full campaigns retain their original identities unless a changed
selected dependency requires another run.

Native reward comparisons preserve the pinned Go write order. A surviving
validator can retain the reward node loaded before full undelegation decrements
its cursor. Terminal validator deletion keeps its separate cleanup behavior.

Disposable traces also retain VRF input references in caller memory. Memory
writes, pool release and reuse, and heap replacement can change the live raw
value without a second native storage-write event. These traces use one journal
for all prefix and target calls. Pending-period registration accepts owned input.
A retained caller-memory binding in pending mode aborts the attempt with
`InputAliasLifetime`; the caller must discard the journal, native port and
sequence. General nested registration across normal transaction settlement is
not qualified by this route. No live execution route is enabled here.
