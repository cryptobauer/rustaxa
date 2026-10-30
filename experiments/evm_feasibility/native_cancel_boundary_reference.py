#!/usr/bin/env python3
"""Capture the short-policy Magnolia cancellation boundary corpus."""
import gzip
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import native_integration_reference as base

HERE = Path(__file__).resolve().parent
EXPECTED_EXPORTER = 'e3189439d62dc65d245b89e5eabd1ced2b774409400c3e4540a1f4756a59a43f'


class BoundedSetup:
    """Capture setup and C++ commands with the existing control supervisor."""

    def __init__(self, artifact):
        self.artifact = artifact
        self.index = 0

    def __getattr__(self, name):
        return getattr(subprocess, name)

    def run(self, argv, *, cwd=None, input=None, check=False, timeout=None, **kwargs):
        if kwargs:
            raise ValueError('unsupported setup subprocess options')
        remaining = base.REFERENCE_DEADLINE - time.monotonic()
        if timeout is not None:
            remaining = min(remaining, timeout)
        if remaining <= 0:
            raise subprocess.TimeoutExpired(argv, 0)
        self.index += 1
        directory = Path(tempfile.mkdtemp(prefix=f'boundary-setup-{self.index:02d}-', dir=self.artifact))
        stdin = None
        if input is not None:
            stdin = directory / 'stdin'
            stdin.write_bytes(input)
        sources = []
        for argument in argv:
            path = Path(argument)
            if path.is_file() and (path.name == 'reference.cpp' or path.name == 'reference'):
                raw = path.read_bytes()
                target = directory / ('source-' + base.digest(raw))
                target.write_bytes(raw)
                sources.append({'original': str(path), 'file': target.name, 'sha256': base.digest(raw), 'bytes': len(raw)})
        command = list(argv)
        if input is not None:
            if command[:3] != ['rtk', 'proxy', 'tar']:
                raise ValueError('unsupported setup stdin command')
            command += ['-f', str(stdin)]
        if command[0] != 'rtk':
            command = ['rtk', 'proxy'] + command
        selected_cwd = Path(cwd).resolve() if cwd is not None else Path.cwd()
        (directory / 'inputs.json').write_text(json.dumps({'sources': sources, 'stdin_sha256': base.digest(input) if input is not None else None, 'original_argv': argv}, indent=2) + '\n')
        (directory / 'command.json').write_text(json.dumps({'phase': 'prepared_before_launch', 'argv': command, 'cwd': str(selected_cwd)}, indent=2) + '\n')
        remaining = base.REFERENCE_DEADLINE - time.monotonic()
        if timeout is not None:
            remaining = min(remaining, timeout)
        if remaining <= 0:
            raise subprocess.TimeoutExpired(argv, 0)
        status = base.capture_child(command, selected_cwd, directory,
                                    timeout_seconds=remaining, max_output_bytes=67108864)
        stdout = gzip.decompress((directory / 'stdout.gz').read_bytes())
        stderr = gzip.decompress((directory / 'stderr.gz').read_bytes())
        code = status['child_exit']
        if status['outcome'] == 'timeout':
            raise subprocess.TimeoutExpired(argv, remaining, stdout, stderr)
        if not status['capture_complete'] or code != 0:
            raise subprocess.CalledProcessError(code, argv, stdout, stderr)
        return subprocess.CompletedProcess(argv, code, stdout, stderr)

    def check_output(self, argv, **kwargs):
        return self.run(argv, check=True, **kwargs).stdout


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise RuntimeError('integration exporter transformation target changed')
    return source.replace(old, new, 1)


def derived_source():
    original = base.GO.read_bytes()
    if base.digest(original) != EXPECTED_EXPORTER:
        raise RuntimeError('integration exporter hash changed')
    source = original.decode()
    source = replace_once(source, 'c.Hardforks.CactiHf.BlockNum = 0',
                          'c.Hardforks.CactiHf.BlockNum = types.BlockNumberNIL')
    source = replace_once(source, 'periods, inputs, addresses := integrationInputs(bracket)',
                          'periods, inputs, addresses := integrationBoundaryInputs()')
    source = replace_once(source, '"Soleirolia", "Cacti"}', '"Soleirolia"}')
    source = replace_once(source, '"last_custody": 8, "locking": 2',
                          '"last_custody": 7, "locking": 3')
    start = source.index('\trequests := []integrationAPIRequest{}', source.index('func integrationAPIs('))
    end = source.index('\tfactory := func(p types.BlockNum)', start)
    requests = '''\trequests := []integrationAPIRequest{}
\tfor _, who := range []int{0, 3, 4} {
\t\trequests = append(requests, integrationAPIRequest{name: fmt.Sprintf("cancel_%d", who), from: addressForPrivateKey(integrationKeys[who]), to: *dpos.ContractAddress(), input: integrationPack(false, "cancelUndelegate", integrationValidators[1])})
\t}
'''
    source = source[:start] + requests + source[end:]
    source = replace_once(source, '[]uint64{0, 3, 4, 7, 9, 11}', '[]uint64{3, 4, 5, 6, 7}')
    start = source.index('\t\tif head == 3 {', source.index('func integrationAPIs('))
    end = source.index('\t\tfor _, request := range selectedRequests {', start)
    source = source[:start] + source[end:]
    source = replace_once(source, 'integrationInteractingTraces(latest, cfg, ids, addresses)', '[]map[string]any{}')
    source = replace_once(source, 'return map[string]any{"schema": 1, "execution_mode": mode,', 'return map[string]any{"pending_queue": integrationBoundaryPending(latest), "schema": 1, "execution_mode": mode,')
    return (source + '\n' + (HERE / 'native_cancel_boundary_reference.inc').read_text()).encode()


def main():
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    bounded = BoundedSetup(artifact)
    base.subprocess = bounded
    base.cpp_support.subprocess = bounded
    fixtures = artifact / 'cancel-boundary-fixtures'
    effective = derived_source()
    derived = artifact / ('cancel-boundary-exporter-' + base.digest(effective) + '.go')
    if derived.exists() and derived.read_bytes() != effective:
        raise RuntimeError('derived source collision')
    derived.write_bytes(effective)
    base.GO = derived
    results, provenance = {}, {}
    for label, revision in base.REVISIONS.items():
        modes = [(False, False), (False, True)]
        if label == 'local':
            modes.append((True, True))
        for observer, traced in modes:
            name = label + ('.observer' if observer else '.batched') + ('.traced' if traced else '.control')
            for api in (False, True):
                key = name + ('.api' if api else '')
                data, directory = base.run(revision, observer, traced, False, api)
                if api:
                    cases = data['cases']
                    if len(cases) != 15 or {case['head'] for case in cases} != {3, 4, 5, 6, 7}:
                        raise RuntimeError('boundary API case set changed')
                    if len({(case['head'], case['request']['sender']) for case in cases}) != 15:
                        raise RuntimeError('boundary API callers not distinct')
                else:
                    rows = data['pending_queue']
                    if len(rows) != 3 or any(row['block'] != 6 or not row['raw'] for row in rows):
                        raise RuntimeError('committed queue maturity changed')
                results[key] = data
                provenance[key] = {'directory': str(directory), 'command_sha256': base.digest((directory / 'command.json').read_bytes())}
    for api in (False, True):
        suffix = '.api' if api else ''
        expected = base.comparison(results['public.batched.control' + suffix])
        for name, data in results.items():
            if name.endswith('.api') == api and base.comparison(data) != expected:
                raise RuntimeError('reference mismatch: ' + name)
    for suffix in ('', '.api'):
        if base.ordered_comparison(results['public.batched.traced' + suffix]) != base.ordered_comparison(results['local.batched.traced' + suffix]):
            raise RuntimeError('public/local traced effects differ')
    if results['public.batched.traced']['pending_queue'] != results['local.batched.traced']['pending_queue']:
        raise RuntimeError('public/local committed queues differ')
    cpp = base.cpp_support.cpp_reference({'cases': [estimate for case in results['local.observer.traced.api']['cases'] for estimate in case['estimates']]})
    results['cpp.api'] = cpp
    pins = {}
    fixtures.mkdir(parents=True, exist_ok=True)
    for name, data in results.items():
        raw = base.body(data)
        encoded = gzip.compress(raw, mtime=0)
        path = fixtures / (base.digest(raw) + '.json.gz')
        if path.exists() and path.read_bytes() != encoded:
            raise RuntimeError('fixture collision')
        path.write_bytes(encoded)
        pins[name] = {'file': path.name, 'sha256': base.digest(raw), 'bytes': len(raw), 'encoding': 'gzip', 'gzip_sha256': base.digest(encoded), 'gzip_bytes': len(encoded)}
    manifest = {'schema': 1, 'scope': 'short-policy Magnolia component; contiguous periods', 'references': base.REVISIONS,
                'base_exporter_sha256': EXPECTED_EXPORTER, 'effective_exporter_sha256': base.digest(effective),
                'producer_sha256': base.digest(Path(__file__).read_bytes()), 'input_sha256': base.digest((HERE / 'native_cancel_boundary_reference.inc').read_bytes()),
                'registry': results['local.observer.traced']['registry'], 'streams': pins}
    (fixtures / 'manifest.json').write_bytes(base.body(manifest))
    (artifact / 'cancel-boundary-provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')


if __name__ == '__main__':
    main()
