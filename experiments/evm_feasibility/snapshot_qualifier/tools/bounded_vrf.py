"""Deterministic fixed-profile command builder and single-use seventh guard.

Building a freeze grants no READY or fact acceptance. Independent review must
approve the exact freeze SHA and invocation. No builder database access occurs.
The guard permits exactly three copy metadata records and one read-only child.
All evidence and source pins are explicit; no discovery or recursive expansion.
"""
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import time

REPO = Path('/workspaces/rustaxa-evm')
SRC = REPO / 'experiments/evm_feasibility/snapshot_qualifier/src'
COPY = REPO / 'local/evm-state-db/snapshot-litenode-copy'
R3 = Path('/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013')
PREPARATION = R3 / 'seventh-voter-preparation.json'
PREPARATION_SHA = '363766ef6de0dad73bc71095172da1bfac6a87f72b4169e9d3ecc501e218ec00'
VOTER = 'c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa'
PROFILE = f'n4_remaining_vrf_voter_{VOTER}_v1'
KEY = 'fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f'
ENV = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8'}
METADATA_CAP = 65536
FREEZE_CAP = 131072
SOURCE_CAP = 1048576
SOURCE_TOTAL = 4194304
FILE_CAP = 268435456
PIN_TOTAL = 471859200
PIN_COUNT = 400
SINK_CAP = 65536
RESULT_CAP = 16384
TIMEOUT = 300
HISTORICAL_MAP = Path('/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/startup-pin-map.json')
HISTORICAL_MAP_SHA = 'bd5beab4dbd66c67d242d410c072540cf753dc3831771aa20f6f190a890a332b'


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate JSON key: {key}')
        result[key] = value
    return result


def outside_database(path):
    if any(part in ('.', '..') for part in str(path).split('/')):
        raise ValueError('lexical path alias prohibited')
    path = Path(path)
    if not path.is_absolute() or path.name == 'AI.md':
        raise ValueError('absolute non-AI path required')
    if path.is_relative_to(COPY) or path.is_relative_to(REPO / 'data'):
        raise ValueError('database path prohibited')
    return path


def regular_path(path):
    path = outside_database(path)
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        if current.is_symlink():
            raise ValueError('symlink path prohibited')
    if path.resolve(strict=True) != path or not path.is_file():
        raise ValueError('canonical regular file required')
    return path


def pin_file(path, expected=None, cap=FILE_CAP):
    path = regular_path(path)
    size = path.stat().st_size
    if size > cap:
        raise ValueError('file cap exceeded')
    digest = hashlib.sha256()
    count = 0
    with path.open('rb') as stream:
        while block := stream.read(min(8192, size - count + 1)):
            count += len(block)
            if count > size:
                raise ValueError('file grew while hashing')
            digest.update(block)
    result = dict(path=str(path), bytes=count, sha256=digest.hexdigest())
    if count != size or (expected is not None and result != expected):
        raise ValueError(f'pin drift: {path}')
    return result


def read_json(path, cap=METADATA_CAP):
    path = regular_path(path)
    with path.open('rb') as stream:
        data = stream.read(cap + 1)
    if len(data) > cap:
        raise ValueError('JSON cap exceeded')
    return json.loads(data, object_pairs_hook=unique_object)


def historical_map(path):
    if Path(path) != HISTORICAL_MAP:
        raise ValueError('historical exception path')
    pin_file(path, dict(path=str(path), bytes=87841, sha256=HISTORICAL_MAP_SHA))
    return read_json(path, 87841)


def encoded(value, cap):
    data = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    if len(data) > cap:
        raise ValueError('encoded metadata cap exceeded')
    return data


def exclusive(path, data):
    path = outside_database(path)
    if path.parent.resolve(strict=True) != path.parent:
        raise ValueError('output parent alias')
    with path.open('xb') as stream:
        stream.write(data)


def expected_source_paths():
    path = R3 / 'sixth-voter/source-freeze.json'
    pin_file(path, dict(path=str(path), bytes=13765,
                       sha256='669add9d030f189bc7a6b68023d1a63aa3ac39186bb12e45984805230cea1962'))
    legacy = read_json(path)
    paths = [row['path'] for row in legacy]
    if len(paths) != 65 or len(set(paths)) != 65:
        raise ValueError('legacy source closure')
    return paths + [str(SRC / name) for name in [
        'vrf_shared_window.rs', 'bin/live_vrf_seventh_voter_window.rs',
        'bin/replay_vrf_seventh_voter_window.rs']]


def validate_preflight(report):
    pin_file(PREPARATION, dict(path=str(PREPARATION), bytes=60570, sha256=PREPARATION_SHA))
    prepared = read_json(PREPARATION)
    baseline = read_json(R3 / 'sixth-voter/vrf-sixth-voter-window-preflight.json', FREEZE_CAP)
    if report['profile'] != PROFILE or report['status'] != 'authority_inputs_authenticated_no_database_open':
        raise ValueError('fixed profile preflight')
    selected = report['selection']
    if len(selected) != 47 or len(prepared['rows']) != 47:
        raise ValueError('complete fixed selection')
    for i, (actual, expected) in enumerate(zip(selected, prepared['rows'])):
        if actual['parent_manifest_index'] != expected['original_parent_manifest_index'] or actual['source_row'] != expected['source_row']:
            raise ValueError('authenticated row binding')
        tuple_ = actual['tuple']
        if tuple_ != dict(identity=expected['identity'], address_hex=expected['account_hex'],
                          logical_key_hex=KEY, key_preimage_hex=f'0004{VOTER}'):
            raise ValueError('authenticated tuple binding')
        if tuple_['identity']['period'] != 25706895 + i:
            raise ValueError('tuple order')
    authority = report['authority']
    if authority['raw_artifact_reads'] != 7 or authority['raw_artifact_bytes'] != 6785636:
        raise ValueError('raw authority bounds')
    if authority['six_input_owner']['inputs'] != prepared['raw_authority_inputs'][:6]:
        raise ValueError('raw authority pins')
    for key in ['input', 'actual', 'semantic_sha256']:
        if authority['prior_first_d'][key] != baseline['authority']['prior_first_d'][key]:
            raise ValueError('complete prior proof provenance')
    if report['bounds'] != baseline['bounds'] or authority['six_input_owner']['bounds'] != baseline['authority']['six_input_owner']['bounds']:
        raise ValueError('all thirteen ceilings and admission bounds')
    prior = authority['prior_first_d']['selected']
    expected = prepared['prior_first_d']
    if prior != dict(proof_index=64, identity=expected['identity'], voter_hex=VOTER,
                     value_hex=expected['accepted_value']['value_hex'], usable_vrf_key=True):
        raise ValueError('complete prior binding')
    if report['bounds']['payload_tuples_exact'] != 47 or report['bounds']['proofs_exact'] != 98:
        raise ValueError('proof scope')
    if any(report['qualification'].values()):
        raise ValueError('preflight cannot qualify facts')


def build(spec_path):
    """Return deterministic bytes after all explicit pins and sealed fields pass.

    The instance spec, freeze and invocation need independent review. This
    function never opens, scans or stats a database path and publishes nothing.
    """
    spec = read_json(spec_path)
    if set(spec) != {'schema', 'run', 'source_commit', 'sources', 'tool_sources', 'binaries', 'preflight', 'evidence', 'copy_metadata'} or spec['schema'] != 1:
        raise ValueError('instance spec schema')
    run = outside_database(spec['run'])
    if not run.is_relative_to(Path('/home/fry/artifacts')) or run.resolve(strict=True) != run:
        raise ValueError('persistent canonical run required')
    sources = spec['sources']
    if [row['path'] for row in sources] != expected_source_paths() or len(sources) != 68:
        raise ValueError('exact successor source closure')
    expected_tools = [str(Path(__file__).resolve()), str(Path(__file__).resolve().with_name('test_bounded_vrf.py'))]
    if [row['path'] for row in spec['tool_sources']] != expected_tools:
        raise ValueError('exact executable tool closure')
    source_total = 0
    for row in sources + spec['tool_sources']:
        pin_file(row['path'], row, SOURCE_CAP)
        source_total += row['bytes']
        if source_total > SOURCE_TOTAL:
            raise ValueError('complete source aggregate cap')
    legacy = read_json(R3 / 'sixth-voter/source-freeze.json')
    changed = {str(SRC / name) for name in ['lib.rs', 'vrf_voter_window.rs', 'vrf_next_voter_window.rs',
                'vrf_third_voter_window.rs', 'vrf_fourth_voter_window.rs', 'vrf_fifth_voter_window.rs', 'vrf_sixth_voter_window.rs']}
    for before, after in zip(legacy, sources):
        if before['path'] not in changed and before != after:
            raise ValueError('undeclared owner source drift')
    binaries = spec['binaries']
    if [row['path'] for row in binaries] != [str(run / f'{kind}-vrf-seventh-voter-window-v1') for kind in ['live', 'replay']]:
        raise ValueError('fixed executable names')
    pin_file(spec['preflight']['path'], spec['preflight'], FREEZE_CAP)
    validate_preflight(read_json(spec['preflight']['path'], FREEZE_CAP))
    report = read_json(spec['preflight']['path'], FREEZE_CAP)
    if report['authority']['six_input_owner']['executable'] != binaries[0]:
        raise ValueError('fresh preflight executable binding')
    source_map = {row['path']: row for row in sources}
    report_sources = report['vrf_sources'] + report['authority']['six_input_owner']['sources']
    if len(report['vrf_sources']) != 22 or len(report['authority']['six_input_owner']['sources']) != 25:
        raise ValueError('embedded provenance counts')
    report_total = 0
    for row in report_sources:
        actual = dict(row, path=str((SRC / row['path']).resolve()))
        if source_map.get(actual['path']) != actual:
            raise ValueError('embedded source successor pin')
        report_total += row['bytes']
        if report_total > SOURCE_TOTAL:
            raise ValueError('report aggregate source cap')
    if len(spec['source_commit']) != 40 or any(c not in '0123456789abcdef' for c in spec['source_commit']):
        raise ValueError('source HEAD shape')
    pins = sources + spec['tool_sources'] + binaries + [spec['preflight']] + spec['evidence'] + [pin_file(spec_path, cap=METADATA_CAP)]
    if len(pins) > PIN_COUNT:
        raise ValueError('command pin count')
    total = 0
    for row in pins:
        pin_file(row['path'], row)
        total += row['bytes']
        if total > PIN_TOTAL:
            raise ValueError('command pin aggregate')
    evidence_paths = {row['path'] for row in spec['evidence']}
    required = [run.parent / name for name in [
        'common-authority-review.md', 'common-authority-amendment.md', 'code-acceptance-review.md', 'validation-summary.json',
        'storage-validation-reuse.json', 'source-commit.json', 'source-commit.log',
        'source-hook-summary.json', 'startup-reconciliation.json', 'instruction-successor-freeze.json']]
    required += [R3 / name for name in ['round3-closeout-freeze.json',
        'frozen-sixth-source-manifest.json', 'sixth-voter/source-freeze.json',
        'sixth-voter/vrf-sixth-voter-window-preflight.json', 'sixth-voter/family-final-review.md',
        'seventh-voter/dependencies-closed.json', 'seventh-voter/builder-draft-dependencies-closed.json']]
    required += [PREPARATION]
    if not {str(path) for path in required}.issubset(evidence_paths):
        raise ValueError('missing mandatory review/history evidence')
    if [row['path'] for row in spec['copy_metadata']] != [str(COPY), str(COPY/'db/db'), str(COPY/'db/state_db')]:
        raise ValueError('fixed copy metadata paths')
    for row in spec['copy_metadata']:
        if set(row) != {'path', 'device', 'inode', 'mode', 'mtime_ns'}:
            raise ValueError('copy metadata shape')
    command = [binaries[0]['path'], '--bundle-output', str(run/'vrf-seventh-voter-window.rcv2'),
               '--report-output', str(run/'vrf-seventh-voter-window-capture.json')]
    return encoded(dict(schema=1, profile=PROFILE, run=str(run), source_commit=spec['source_commit'],
                        cwd=str(REPO), environment=ENV, capture_command=command,
                        pins=pins, pin_count=len(pins), pinned_file_bytes=total,
                        source_count=68, complete_source_bytes=source_total,
                        embedded_source_bytes=report_total,
                        copy_metadata=spec['copy_metadata'], accepted=False), FREEZE_CAP)


def verify_files(freeze):
    if freeze['pin_count'] != len(freeze['pins']) or len(freeze['pins']) > PIN_COUNT:
        raise ValueError('frozen pin count')
    total = 0
    for row in freeze['pins']:
        pin_file(row['path'], row)
        total += row['bytes']
        if total > PIN_TOTAL:
            raise ValueError('frozen total cap')
    if total != freeze['pinned_file_bytes']:
        raise ValueError('frozen total drift')


def copy_metadata(expected):
    if [row['path'] for row in expected] != [str(COPY), str(COPY/'db/db'), str(COPY/'db/state_db')]:
        raise ValueError('copy paths')
    actual = []
    for row in expected:
        path = Path(row['path'])
        if path.is_symlink() or path.resolve() != path:
            raise ValueError('copy alias')
        stat = path.stat()
        item = dict(path=str(path), device=stat.st_dev, inode=stat.st_ino, mode=stat.st_mode, mtime_ns=stat.st_mtime_ns)
        if item != row:
            raise ValueError('copy metadata drift')
        actual.append(item)
    return actual


def run_bounded(command, env, log, timeout_seconds=TIMEOUT):
    """Bound child output and the full deadline, including wait after pipe EOF."""
    written = 0
    overflow = False
    timed_out = False
    deadline = time.monotonic() + timeout_seconds
    with Path(log).open('xb') as output:
        process = subprocess.Popen(command, cwd=REPO, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    timed_out = True
                    break
                data = os.read(process.stdout.fileno(), min(8192, SINK_CAP-written+1))
                if not data:
                    break
                capacity = SINK_CAP - written
                output.write(data[:capacity])
                written += min(len(data), capacity)
                if len(data) > capacity:
                    overflow = True
                    break
        finally:
            selector.close()
            process.stdout.close()
            if overflow or timed_out:
                process.kill()
            try:
                child_exit = process.wait(timeout=max(0.001, deadline-time.monotonic()))
            except subprocess.TimeoutExpired:
                timed_out = True
                process.kill()
                child_exit = process.wait()
    return dict(exit=2 if overflow or timed_out else child_exit, child_exit=child_exit,
                output_overflow=overflow, wall_timeout=timed_out, log_bytes=written)


def capture(freeze_path, expected_sha):
    freeze_path = regular_path(freeze_path)
    with freeze_path.open('rb') as stream:
        frozen_bytes = stream.read(FREEZE_CAP + 1)
    if len(frozen_bytes) > FREEZE_CAP or hashlib.sha256(frozen_bytes).hexdigest() != expected_sha:
        raise ValueError('reviewed freeze SHA/cap drift')
    # Parse precisely the bounded bytes whose SHA was authenticated above.
    freeze = json.loads(frozen_bytes, object_pairs_hook=unique_object)
    expected_fields = {'schema', 'profile', 'run', 'source_commit', 'cwd', 'environment',
                       'capture_command', 'pins', 'pin_count', 'pinned_file_bytes', 'source_count',
                       'complete_source_bytes', 'embedded_source_bytes', 'copy_metadata', 'accepted'}
    if not isinstance(freeze, dict) or set(freeze) != expected_fields or freeze['accepted'] is not False:
        raise ValueError('trusted freeze schema')
    run = outside_database(freeze['run'])
    if freeze.get('schema') != 1 or freeze.get('profile') != PROFILE or run != freeze_path.parent \
            or not run.is_relative_to(Path('/home/fry/artifacts')) or run.resolve(strict=True) != run:
        raise ValueError('trusted freeze/run binding')
    # Trusted admission ends here. All later failures permanently consume this
    # attempt, including offline pin, HEAD, output and metadata failures.
    exclusive(run/'vrf-seventh-voter-window-attempt.json', encoded(
        dict(command=freeze['capture_command'], source_commit=freeze['source_commit'],
             freeze_sha256=expected_sha), RESULT_CAP))
    verify_files(freeze)
    spec_pin = freeze['pins'][-1]
    if build(Path(spec_pin['path'])) != frozen_bytes:
        raise ValueError('freeze not deterministic builder output')
    if freeze['profile'] != PROFILE or freeze['cwd'] != str(REPO) or freeze['environment'] != ENV or Path.cwd() != REPO:
        raise ValueError('exact invocation binding')
    head = subprocess.check_output(['/usr/bin/git', 'rev-parse', 'HEAD'], cwd=REPO, env=ENV).decode().strip()
    if head != freeze['source_commit']:
        raise ValueError('source HEAD drift')
    outputs = ['vrf-seventh-voter-window.rcv2', 'vrf-seventh-voter-window-capture.json',
               'vrf-seventh-voter-window-live.log', 'vrf-seventh-voter-window-live-result.json']
    if any(os.path.lexists(run/name) for name in outputs):
        raise ValueError('single-use output collision')
    before = copy_metadata(freeze['copy_metadata'])
    start = time.monotonic()
    result = run_bounded(freeze['capture_command'], ENV, run/outputs[2])
    record = dict(command=freeze['capture_command'], **result, seconds=round(time.monotonic()-start, 2), before=before, accepted=False)
    try:
        record['after'] = copy_metadata(freeze['copy_metadata'])
        verify_files(freeze)
    finally:
        exclusive(run/outputs[3], encoded(record, RESULT_CAP))
    print(json.dumps(record))
    return result['exit']


def main():
    if len(sys.argv) == 4 and sys.argv[1] == 'build':
        exclusive(Path(sys.argv[3]), build(Path(sys.argv[2])))
        return 0
    if len(sys.argv) == 4 and sys.argv[1] == 'capture':
        return capture(Path(sys.argv[2]), sys.argv[3])
    raise ValueError('usage: bounded_vrf.py build SPEC FREEZE | capture FREEZE REVIEWED_SHA256')


if __name__ == '__main__':
    sys.exit(main())
