# Command capture repair and exact legacy loss disposition

Latest checkpoint: /home/fry/artifacts/vrf40-integration-20261018-round211/checkpoint-210-final.json.
Source83mapping reviewed;real54/69complete,72allocation+46659bytes,75notrun.
Effective210/durable8;0/16completefits and805/109accepted/residual remain.

## Cause and installed repair

Reviewer commands bypassed persistent stderr capture. Original calls2/6lost raw
streams;literal empty source.stderr and literal0source.exit are INVALID process
evidence. The lost8161-byte diagnostic report has a known retained charge. Missing
raw streams remain unknown,never zero,token-derived bounds or reconstructed text.
All original files and rejection records remain immutable;no failed review qualifies.

`scripts/capture_command.py` now captures subprocess stdout/stderr as distinct raw
files before returning a small tool summary,records the REAL child exit,keeps
runner errors separate,refuses output-directory reuse,and enforces explicit wall
and combined output bounds. Its status.json binds actual stream sizes/hashes.
Spawn failure produces no child exit file. Timeout/overflow never claims complete
capture;overflow retains a bounded prefix,not the full original or its byte ceiling.
Runner failure/missing status is not a pass. Capture completion is not test/fit
acceptance. Exact source snapshots and operation admission remain separate duties.

Use an admitted unique persistent output directory;parent must already exist:

```sh
rtk proxy python3 scripts/capture_command.py --output-dir /persistent/run/unique-op --timeout-seconds 30 --max-output-bytes 1048576 -- rtk proxy python3 -m unittest discover -s scripts/tests -p test_capture_command.py -v
```

Replace example bounds with admitted bounds. Runner exits124timeout,125capture/
overflow,127spawn failure;JSONoutcome disambiguates child codes with those values.
stdin is closed by default. Run retained script files as arguments;for explicit
input use --stdin-file /persistent/script-or-input BEFORE --. A here-document on
the wrapper alone does not reach the child. Check nonzero expected test/result
counts rather than interpreting exit0alone as success.
The stdin path in status.json is not an authenticated input-stream identity.
Retain and pin input scripts separately under the task's input rules;do not use
the runner alone as proof of which mutable input bytes a child consumed.

This is no process sandbox. Ordinary process-group descendants are killed on
failure;deliberately detached sessions are not contained. Use non-daemonizing
development commands only. Production/native capture paths are unchanged.

Every lead/worker/reviewer handoff must name this runner,the exact admitted bounds,
a new persistent directory,and required launched/outcome/capture_complete/child_exit
plus stream paths/hashes. Verify a tiny nonzero/stderr command through EACH active
worker's actual execution path before substantive diagnostics. No direct shell
fallback after failure,no literal status/stderr fabrication. Bounded previews of
already retained files are allowed;every new diagnostic producer uses capture.

Final regression evidence: /home/fry/artifacts/capture-runner-fix-tests-20261018-v3.
Eleven tests cover binary/nonzero streams,spawn and input failure,timeout,overflow/
exact boundary,reuse refusal,closed-stream living child,inherited child streams,
post-wait deadline race and explicit retained stdin. Independent Luna review found
the deadline edge case,which was fixed and tested. Final stdin delta review is
recorded separately when returned;no EVM/runtime acceptance is implied.

## Narrow owner-dispatch exception

The user continuation explicitly accepts ONLY original diagnostic calls2/6missing
streams and the lost8161-byte administrative report as documented legacy losses.
Unknown byte charge staysUNKNOWN. Keep known bytes and all holds;no refund/reset,
retrospective admission or invented fixed fee. Continuing development is permitted
under this exact exception;do not claim past aggregate resource totals are fully
finite/proved. Future accounting remains strict;record the exception explicitly.
Scope confirmation must exclude literal files/failed reviews and establish accepted
source83/current real-input bodies are intact. Any actual acceptance-evidence loss
remains blocking. Do not repeat searches for irrecoverable bytes indefinitely.

## Direct next work

Reuse SAME54baseline/44floor budget,last52historical. Scope confirmation/capture-path
checks,then role72ordering/lifetime correction:defer unused slot strict decode until
after all70input reads,preserving wire/caller/semantic charges. Freeze source84,
exact-input regression/independent review,then75and SAMEv7packet. Complete all81/
all83fullartifact/16fits/gates/sourceacceptance/hook/READY/onecapture/replays/seal.
Do not start another evidence framework or claim capture-tool tests fix EVM code.
