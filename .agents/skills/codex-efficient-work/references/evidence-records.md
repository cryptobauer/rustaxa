# Evidence records

Read this reference when large artifacts, repeated histories or failure logs
are consuming review context. Keep evidence complete and auditable while making
the active review concise.

- Save the first full failure output once as an immutable artifact. Record its
  path, size, hash, status/exit and the named correction in a short review note.
  Do not recursively copy prior history or full failure bodies into each note.
- Preserve every mandatory input and semantic occurrence. Do not deduplicate
  entries only because their bytes or labels match when each occurrence carries
  meaning. If the active contract requires the full body, preserve it or obtain
  a reviewed successor contract that explicitly accepts a different record.
- Separate proof of command/data authority from provenance and review format.
  Verify required fields and bindings for every input. Reuse source checks or
  gates only for exact matching hashes and covered scope.
- Use machine-readable shape, hash and binding checks before expensive gates.
  These checks find malformed inputs; they do not replace required semantic
  review or acceptance tests.
- Treat path, byte length and digest as one identity. Derive command, manifest and
  permit bindings from the same frozen source of truth; check the actual body
  against the complete tuple. A new path with an old digest is still stale.
- Use `followup_task` to trigger an idle worker. Use `send_message` to steer a
  running worker; do not assume a message starts an idle task.

Keep the concise note linked to the immutable evidence. Do not weaken the
contract, skip a required body, or claim a gate passed from a precheck.

Complete slow source and resource reviews before the final launch-time telemetry
check. Then freeze only necessary freshness-dependent bindings, review the exact
delta and execute promptly within the required freshness window. Do not bypass
changed-pin review or extend freshness to accommodate a long review.

Detailed finite recovery ledgers apply only when the project or user requires them.
For ordinary work, retain focused checks, complete evidence and the requested quota
bound without importing byte/read/time ledgers or approval loops from another repo.
Where ledgers apply, reserve each in-flight operation's complete failure,
review and closeout before admission; do not let a correction spend that reservation.
Keep events monotonic, but do not invent small count-only stops for ordinary
bounded development. Existing contract counters require an authorized reviewed
successor before change. Finite byte/read/parse/allocation/physical limits remain.
A timeout or partial test roster is a failed check; diagnose before retry and bind
results to the exact retained source. Never claim later corrections inherit a pass.

Require the capture helper in each evidence-producing worker/reviewer handoff and
check a real nonzero/stderr demonstration. Bypassing the helper defeats it. Separate
child exit from runner failure;never invent empty stderr/exit0. Overflow is a prefix,
not complete evidence or a bound on lost bytes. stdin must be explicit when the
runner closes it by default;require real expected result counts,not exit0alone.
Preserve irrecoverable uncertainty until an exact owner disposition;never recreate
historical logs,guess a fee or silently use zero. Acceptance logs remain strict.
