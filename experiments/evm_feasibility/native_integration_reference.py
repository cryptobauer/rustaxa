#!/usr/bin/env python3
"""Record/reproduce the signed current-profile integration reference.

The source archives and complete command streams remain in RUN_ARTIFACT_DIR.
The corpus stores immutable bodies once and authenticates them by SHA-256.
No original reference support or old fixture is modified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import gzip
import shutil
import selectors
import signal
import time
import sys

import mixed_period_reference as mixed
import native_retained_commission_trace_reference as tracing
import native_reward_commission_trace_reference as trace_source
import native_metadata_estimate_reference as cpp_support
from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
GO = HERE / 'native_integration_reference.go'
FIXTURES = HERE / 'fixtures/native_integration'
# Leave time for cancellation, gzip verification and status closeout before the
# enclosing 900-second reference-command boundary can terminate this producer.
REFERENCE_DEADLINE = time.monotonic() + 870


def digest(data):
    return hashlib.sha256(data).hexdigest()


def _capture_child_inner(argv, cwd, directory, *, timeout_seconds, max_output_bytes, control_fd):
    """Bound the combined uncompressed streams while the child is running.

    Gzip files retain exactly the captured prefix on interruption. Incomplete
    captures never become oracle data, even if the child exited nonzero first.
    The new process group is killed and reaped on timeout or output overflow.
    """
    if timeout_seconds <= 0 or max_output_bytes < 0:
        raise ValueError('invalid internal capture bound')
    started = time.monotonic()
    streams = {name: {'file': name + '.gz', 'bytes': 0, 'hash': hashlib.sha256()}
               for name in ('stdout', 'stderr')}
    sinks = {}
    selector = selectors.DefaultSelector()
    os.set_blocking(control_fd, False)
    selector.register(control_fd, selectors.EVENT_READ, 'control')
    child = None
    outcome = 'launch_error'
    observed_bytes = 0
    captured_bytes = 0
    error = None
    try:
        for name, stream in streams.items():
            sinks[name] = gzip.GzipFile(filename=str(directory / stream['file']), mode='wb', mtime=0)
        # EOF before launch means the enclosing producer has already gone.
        if selector.select(0):
            raise RuntimeError('producer stopped before internal child launch')
        child = subprocess.Popen(argv, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 process_group=0)
        for name, pipe in (('stdout', child.stdout), ('stderr', child.stderr)):
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, name)
        outcome = 'exited'
        interrupted_at = None
        while any(key.data != 'control' for key in selector.get_map().values()) or child.poll() is None:
            now = time.monotonic()
            if interrupted_at is None and now - started >= timeout_seconds:
                outcome = 'timeout'
                interrupted_at = now
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            # A descendant that escaped the group must not keep capture open.
            if interrupted_at is not None and now - interrupted_at >= 2:
                break
            for key, _ in selector.select(0.05):
                if key.data == 'control':
                    if os.read(control_fd, 1) == b'':
                        selector.unregister(control_fd)
                        os.close(control_fd)
                        if interrupted_at is None:
                            outcome = 'cancelled'
                            interrupted_at = time.monotonic()
                            try:
                                os.killpg(child.pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                    continue
                raw = os.read(key.fileobj.fileno(), 65536)
                if not raw:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                observed_bytes += len(raw)
                keep = raw[:max(0, max_output_bytes - captured_bytes)]
                name = key.data
                sinks[name].write(keep)
                streams[name]['hash'].update(keep)
                streams[name]['bytes'] += len(keep)
                captured_bytes += len(keep)
                if len(keep) != len(raw) and interrupted_at is None:
                    outcome = 'output_limit'
                    interrupted_at = time.monotonic()
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
        # A successful leader can leave descendants after closing its pipes.
        # The supervisor owns this group and terminates it before source cleanup.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=2 if interrupted_at is not None else max(0.001, timeout_seconds - (time.monotonic() - started)))
        except subprocess.TimeoutExpired:
            outcome = 'timeout'
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
    except BaseException as exc:
        error = repr(exc)
        outcome = 'runner_error' if child is not None else 'launch_error'
        if child is not None:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
    finally:
        open_pipes = any(key.data != 'control' for key in selector.get_map().values())
        for key in list(selector.get_map().values()):
            selector.unregister(key.fileobj)
            if key.data == 'control':
                os.close(control_fd)
            else:
                key.fileobj.close()
        selector.close()
        for sink in sinks.values():
            sink.close()
    for stream in streams.values():
        stream['sha256'] = stream.pop('hash').hexdigest()
        path = directory / stream['file']
        stream['gzip_sha256'] = digest(path.read_bytes())
        # Verify losslessness incrementally, including interrupted prefixes.
        check = hashlib.sha256()
        count = 0
        with gzip.open(path, 'rb') as source:
            while block := source.read(65536):
                check.update(block)
                count += len(block)
        assert count == stream['bytes'] and check.hexdigest() == stream['sha256']
    status = {'argv': argv, 'cwd': str(cwd), 'launched': child is not None,
              'child_exit': child.returncode if child else None, 'outcome': outcome,
              'child_pid': child.pid if child else None, 'supervisor_pid': os.getpid(),
              'capture_complete': outcome == 'exited' and not open_pipes,
              'elapsed_seconds': time.monotonic() - started,
              'timeout_seconds': timeout_seconds, 'max_output_bytes': max_output_bytes,
              'observed_bytes': observed_bytes, 'captured_bytes': captured_bytes,
              'streams': streams, 'runner_error': error}
    (directory / 'capture.json').write_text(json.dumps(status, indent=2) + '\n')
    command_path = directory / 'command.json'
    if command_path.exists():
        prepared = json.loads(command_path.read_bytes())
        if prepared.get('phase') == 'prepared_before_launch':
            prepared.update(status)
            prepared['phase'] = 'captured'
            command_path.write_text(json.dumps(prepared, indent=2) + '\n')
            (directory / 'exit').write_text(str(status['child_exit']) + '\n')
            # Only this newly created, pinned compiler tree is disposable.
            # The independent supervisor also performs cleanup after parent death.
            if prepared.get('disposable_compiler_tree') == str(directory):
                cleanup_compiler_tree(directory)
    return status


def cleanup_compiler_tree(directory):
    for path in directory.iterdir():
        if path.name in {'command.json', 'capture.json', 'exit', 'stdout.gz', 'stderr.gz'}:
            continue
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()


def capture_child(argv, cwd, directory, *, timeout_seconds=900, max_output_bytes=67108864):
    """Wait for a bounded supervisor which survives producer-group SIGKILL.

    The supervisor owns Go, gzip files and status. The sole control writer is
    owned here; parent death closes it, so the supervisor cancels and reaps Go
    even when the unchanged outer capture helper kills this whole process group.
    """
    if timeout_seconds <= 0 or max_output_bytes < 0 or max_output_bytes > 67108864:
        raise ValueError('invalid internal capture bound')
    control_read, control_write = os.pipe()
    supervisor = None
    try:
        supervisor = subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), '--capture-supervisor',
             str(control_read), str(Path(directory).resolve()), str(Path(cwd).resolve()),
             str(timeout_seconds), str(max_output_bytes), json.dumps(argv)],
            pass_fds=(control_read,), start_new_session=True,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(control_read)
        control_read = None
        supervisor.wait(timeout=timeout_seconds + 15)
    except BaseException:
        os.close(control_write)
        control_write = None
        if supervisor is not None:
            supervisor.wait(timeout=15)
        raise
    finally:
        if control_read is not None:
            os.close(control_read)
        if control_write is not None:
            os.close(control_write)
    status_path = directory / 'capture.json'
    if supervisor.returncode or not status_path.exists():
        raise RuntimeError(f'internal supervisor failed: exit {supervisor.returncode}, {directory}')
    return json.loads(status_path.read_bytes())


def run(pin, observer, traced, bracket, api=False):
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    directory = Path(tempfile.mkdtemp(prefix='native-oracle-', dir=artifact))
    archive = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', pin])
    subprocess.run(['rtk', 'proxy', 'tar', '-x', '-C', str(directory)], input=archive, check=True)
    if traced:
        mixed.apply_raw_trace_patch(directory)
    trace_target = directory / trace_source.TRACE_TARGET
    original = trace_target.read_bytes()
    assert digest(original) == trace_source.TRACE_SHA
    changed = original
    if traced:
        assert changed.count(tracing.PREFIX) == changed.count(tracing.TARGET) == 1
        changed = changed.replace(tracing.PREFIX, b'\t\tobserveRetainedTrace("prefix",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("prefix",true,block_state,&trx,&ret,err)\n', 1)
        changed = changed.replace(tracing.TARGET, b'\t\tobserveRetainedTrace("target",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("target",true,block_state,&trx,&ret,err)\n', 1)
    trace_target.write_bytes(changed)
    trace_helper = trace_target.parent / 'native_integration_trace_observer.go'
    trace_helper.write_bytes(tracing.HELPER)
    command = directory / 'cmd/native_integration'
    command.mkdir(parents=True)
    derived = {}
    for name in mixed.SOURCES:
        source = (HERE / name).read_bytes()
        if name == 'mixed_period_reference.go':
            assert source.count(b'func main() {') == 1
            assert source.count(b'chainID        = uint64(841)') == 1
            source = source.replace(b'func main() {', b'func unusedMixedMain() {', 1)
            source = source.replace(b'chainID        = uint64(841)', b'chainID        = uint64(666)', 1)
        (command / name).write_bytes(source)
        derived[name] = digest(source)
    (command / GO.name).write_bytes(GO.read_bytes())
    argv = ['rtk', 'proxy', 'go', 'run', '-mod=readonly']
    tags = (['concrete_observer'] if observer else []) + (['mixed_raw_trace'] if traced else [])
    if tags:
        argv.append('-tags=' + ','.join(tags))
    argv.extend(['./cmd/native_integration', '--mode', 'observer' if observer else 'batched'])
    if bracket:
        argv.append('--bracket')
    if api:
        argv.append('--api')
    source_objects = artifact / 'source-objects'
    source_objects.mkdir(exist_ok=True)
    source_files = list(command.iterdir())
    source_files += [directory / mixed.TRACE_TARGET, directory / mixed.REFUND_TRACE_TARGET, trace_target, trace_helper]
    source_files += list((directory / mixed.TRACE_TARGET).parent.glob('mixed_period_*observer.go'))
    source_files += list((directory / mixed.REFUND_TRACE_TARGET).parent.glob('mixed_period_*observer.go'))
    source_pins = {}
    for path in source_files + [Path(__file__)]:
        raw = path.read_bytes()
        hashed = digest(raw)
        destination = source_objects / hashed
        if destination.exists():
            assert destination.read_bytes() == raw
        else:
            destination.write_bytes(raw)
        name = str(path.relative_to(directory)) if path.is_relative_to(directory) else 'harness/native_integration_reference.py'
        source_pins[name] = {'sha256': hashed, 'bytes': len(raw)}
    prepared = {'argv': argv, 'cwd': str(directory), 'reference_pin': pin,
                'base_archive_sha256': digest(archive), 'base_archive_bytes': len(archive),
                'derived_sha256': derived, 'effective_sources': source_pins,
                'phase': 'prepared_before_launch', 'disposable_compiler_tree': str(directory),
                'capture_status_file': 'capture.json'}
    (directory / 'command.json').write_text(json.dumps(prepared, indent=2) + '\n')
    capture = capture_child(argv, directory, directory,
                            timeout_seconds=min(600, max(0.001, REFERENCE_DEADLINE - time.monotonic())))
    metadata = {**prepared, **capture, 'phase': 'captured'}
    (directory / 'command.json').write_text(json.dumps(metadata, indent=2) + '\n')
    (directory / 'exit').write_text(str(capture['child_exit']) + '\n')
    # These are newly created disposable compiler trees, not original data or
    # accepted evidence. Every changed source byte and complete stream is pinned.
    cleanup_compiler_tree(directory)
    if not capture['capture_complete'] or capture['child_exit']:
        raise RuntimeError(f"internal Go capture {capture['outcome']}, exit {capture['child_exit']}: {directory}")
    lines = gzip.decompress((directory / 'stdout.gz').read_bytes()).splitlines()
    data = json.loads(lines[-1])
    return data, directory


def semantic(data):
    """Only cross-mode fields; observed originals retain their actual order."""
    return {
        'configuration': data['configuration'], 'inputs': data['inputs'],
        'genesis_root': data['genesis']['root'], 'registry': data['registry'],
        'periods': [{k: p[k] for k in ('number', 'transactions', 'planner_facts', 'reward_input', 'reward_output', 'final')} for p in data['periods']],
    }


def comparison(data):
    if data.get('kind') == 'historical_api':
        data = json.loads(json.dumps(data))
        for extra in data.get('interacting_traces', []):
            extra['trace'].pop('calls')
            extra['trace'].pop('panic_stack', None)
        for case in data['cases']:
            for probe in [case['call'], *(p for e in case['estimates'] for p in e['probes'])]:
                probe.pop('ordered_raw_writes')
            for name in ('trace', 'trace_sequence'):
                case[name].pop('calls')
                case[name].pop('panic_stack', None)
        return data
    value = semantic(data)
    for period in value['periods']:
        period['transactions'] = [{k: v for k, v in t.items() if k not in ('observer', 'ordered_raw_writes', 'gas_refund')} for t in period['transactions']]
        period['final'] = {k: period['final'][k] for k in ('root', 'committed_root', 'accounts', 'native_storage_by_hashed_path')}
    return value


def validate(data):
    assert data['schema'] == 1
    assert len(data['registry']) == 29
    assert len({(r['contract'], r['selector']) for r in data['registry']}) == 29
    assert [p['number'] for p in data['periods']] == list(range(1, 13))
    assert data['configuration']['chain_id'] == 666
    for period in data['periods']:
        assert period['final']['root'] == period['final']['committed_root']
        assert int(period['reward_output']['minted_reward']) > 0
        assert len(period['transactions']) > 0
        assert all(t['signed_rlp'] and not t['consensus_error'] for t in period['transactions'])
    used = {t['input'][:8] for p in data['periods'] for t in p['transactions'] if t['to'] in ('00' * 19 + 'fe', '00' * 19 + 'ee')}
    assert used == {row['selector'] for row in data['registry']}, (used, data['registry'])


def ordered_comparison(data):
    # Compare only the independently reviewed permutation of disjoint map
    # blocks. Author-before-map, yield/supply and end-block barriers stay exact.
    data = json.loads(json.dumps(data))
    if data.get('kind') == 'historical_api':
        for extra in data.get('interacting_traces', []):
            extra['trace'].pop('panic_stack', None)
        for case in data['cases']:
            for name in ('trace', 'trace_sequence'):
                case[name].pop('panic_stack', None)
        return data
    for period in data['periods']:
        writes = period['ordered_raw_writes']['rewards']
        facts = period['reward_input']['validators']
        if not writes or len(facts) < 2:
            continue
        assert writes[0]['key'] == 'd33e25809fcaa2b6900567812852539da8559dc8b76a7ce3fc5ddd77e8d19a69'
        assert writes[-1]['key'] == 'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc'
        start = 1
        if any(row['key'] == writes[1]['key'] for row in writes[2:-1]):
            start = 2
        middle = writes[start:-1]
        assert len(middle) == len(facts)
        assert {row['key'] for row in middle} == {row['reward_key'] for row in facts}
        assert all(row['address'] == '00' * 19 + 'fe' for row in middle)
        period['ordered_raw_writes']['rewards'] = writes[:start] + sorted(middle, key=lambda row: row['key']) + writes[-1:]
    return data


def body(data):
    return (json.dumps(data, sort_keys=True, separators=(',', ':')) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    assert REVISIONS == {'public': '6c7e5338b22d5e596cc2365a88d1f94840e1ee1b', 'local': 'bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418'}
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    history = artifact / 'reference-manifest-history'
    history.mkdir(exist_ok=True)
    for existing in (FIXTURES / 'manifest.json', artifact / 'reference-record-provenance.json', artifact / 'reference-reproduce-provenance.json'):
        if existing.exists():
            raw = existing.read_bytes()
            target = history / (digest(raw) + '.json')
            if target.exists():
                assert target.read_bytes() == raw
            else:
                target.write_bytes(raw)
    results = {}
    provenance = {}
    for label, pin in REVISIONS.items():
        for observer, traced in ((False, False), (False, True), *(([(True, True)] if label == 'local' else []))):
            name = label + ('.observer' if observer else '.batched') + ('.traced' if traced else '.control')
            for bracket in (False, True):
                instance = name + ('.bracket' if bracket else '')
                data, directory = run(pin, observer, traced, bracket)
                validate(data)
                results[instance] = data
                provenance[instance] = {'directory': str(directory), 'command_sha256': digest((directory / 'command.json').read_bytes()), 'streams': json.loads((directory / 'command.json').read_text())['streams']}
            instance = name + '.api'
            data, directory = run(pin, observer, traced, False, True)
            assert len(data['interacting_traces']) == 38
            assert all('panic' not in row['trace'] for row in data['interacting_traces'])
            assert len(data['cases']) == 6 * 42 + 59
            assert len({row['request']['input'][:8] for row in data['cases']}) == 29
            assert all('panic' not in row[field] for row in data['cases'] for field in ('trace', 'trace_sequence'))
            results[instance] = data
            provenance[instance] = {'directory': str(directory), 'command_sha256': digest((directory / 'command.json').read_bytes()), 'streams': json.loads((directory / 'command.json').read_text())['streams']}
    for bracket in ('', '.bracket'):
        baseline = comparison(results['public.batched.control' + bracket])
        for name, data in results.items():
            if not name.endswith('.api') and name.endswith('.bracket') == bool(bracket):
                assert comparison(data) == baseline, 'required cross-mode canonical root/state/receipt mismatch: ' + name
    for name, data in results.items():
        if name.endswith('.api'):
            assert comparison(data) == comparison(results['public.batched.control.api']), 'historical cross-mode mismatch: ' + name
    cpp = cpp_support.cpp_reference({'cases': [estimate for case in results['local.observer.traced.api']['cases'] for estimate in case['estimates']]})
    results['cpp.api'] = cpp
    provenance['cpp.api'] = {'source': str(cpp_support.CPP_SOURCE.relative_to(ROOT)), 'source_sha256': digest(cpp_support.CPP_SOURCE.read_bytes()), 'input_estimates': 933, 'output_sha256': digest(body(cpp))}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
    saved = json.loads((FIXTURES / 'manifest.json').read_bytes()) if (FIXTURES / 'manifest.json').exists() else None
    support_files = [Path(module.__file__) for module in (mixed, tracing, trace_source, cpp_support)] + [HERE / 'reference.py', cpp_support.CPP_SOURCE]
    source_pins = {}
    objects = artifact / 'source-objects'
    objects.mkdir(exist_ok=True)
    for source in support_files:
        raw = source.read_bytes()
        source_pins[str(source.relative_to(ROOT))] = digest(raw)
        destination = objects / digest(raw)
        if destination.exists():
            assert destination.read_bytes() == raw
        else:
            destination.write_bytes(raw)
    pins = {}
    for name, data in results.items():
        # Full stream is content-addressed; all consumers refer to its digest.
        raw = body(data)
        hashed = digest(raw)
        path = FIXTURES / (hashed + '.json.gz')
        encoded = gzip.compress(raw, mtime=0)
        if args.record:
            # A new capture is still retained in full. Reuse the immutable corpus
            # representative only for identical source/input authority and the
            # complete reviewed partial-order comparison used by reproduction.
            if saved and saved.get('producer_support_sha256') == source_pins and saved['references'] == REVISIONS and saved['exporter_sha256'] == digest(GO.read_bytes()) and saved['support_sha256'] == {n: digest((HERE / n).read_bytes()) for n in mixed.SOURCES} and name in saved['streams']:
                old = saved['streams'][name]
                previous = (FIXTURES / old['file']).read_bytes()
                if old.get('encoding') == 'gzip':
                    previous = gzip.decompress(previous)
                assert digest(previous) == old['sha256'] and len(previous) == old['bytes']
                equal = json.loads(previous) == data if name == 'cpp.api' else ordered_comparison(json.loads(previous)) == ordered_comparison(data)
                if equal:
                    pins[name] = old
                    provenance[name]['reused_corpus_body_sha256'] = old['sha256']
                    provenance[name]['fresh_raw_body_sha256'] = hashed
                    provenance[name]['comparison'] = 'complete reviewed partial order; all other bytes exact'
                    continue
            if path.exists():
                assert gzip.decompress(path.read_bytes()) == raw
            else:
                path.write_bytes(encoded)
        else:
            old = saved['streams'][name]
            previous = (FIXTURES / old['file']).read_bytes()
            if old.get('encoding') == 'gzip':
                previous = gzip.decompress(previous)
            assert digest(previous) == old['sha256'] and len(previous) == old['bytes']
            assert (ordered_comparison(json.loads(previous)) == ordered_comparison(data)) if name != 'cpp.api' else json.loads(previous) == data, name
        pins[name] = {'file': path.name, 'sha256': hashed, 'bytes': len(raw), 'encoding': 'gzip', 'gzip_sha256': digest(encoded), 'gzip_bytes': len(encoded)} if args.record else saved['streams'][name]
    manifest = {'schema': 1, 'references': REVISIONS, 'harness_sha256': digest(Path(__file__).read_bytes()), 'exporter_sha256': digest(GO.read_bytes()), 'support_sha256': {name: digest((HERE / name).read_bytes()) for name in mixed.SOURCES}, 'producer_support_sha256': source_pins, 'streams': pins, 'registry': results['local.observer.traced']['registry'], 'scope': 'signed synthetic current profile; API/recovery acceptance requires Rust consumers'}
    path = FIXTURES / 'manifest.json'
    if args.record:
        path.write_bytes(body(manifest))
    else:
        assert json.loads(path.read_bytes()) == manifest
    immutable_manifest = history / (digest(body(manifest)) + '.json')
    immutable_manifest.write_bytes(body(manifest))
    provenance_bytes = (json.dumps(provenance, indent=2) + '\n').encode()
    (history / (digest(provenance_bytes) + '.json')).write_bytes(provenance_bytes)
    (artifact / ('reference-record-provenance.json' if args.record else 'reference-reproduce-provenance.json')).write_bytes(provenance_bytes)
    print(json.dumps({'streams': len(pins), 'periods': 12, 'selectors': 29, 'record': args.record}))


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--capture-supervisor':
        _, _, control, directory, cwd, timeout, limit, command = sys.argv
        _capture_child_inner(json.loads(command), Path(cwd), Path(directory),
                             timeout_seconds=float(timeout), max_output_bytes=int(limit),
                             control_fd=int(control))
    else:
        main()
