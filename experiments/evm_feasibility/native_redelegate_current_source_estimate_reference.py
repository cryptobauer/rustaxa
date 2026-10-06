#!/usr/bin/env python3
"""Compare redelegate probes from actual pinned DryRunner with unchanged C++ search."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from reference import REVISIONS, ROOT
import native_metadata_estimate_reference as support
import native_redelegate_current_source_reference as observation

HERE = Path(__file__).resolve().parent
INPUT = HERE / 'fixtures/native_redelegate_current_source_simulation/public.json'
FIXTURES = HERE / 'fixtures/native_redelegate_current_source_estimate'

SOURCES = [HERE / name for name in (
    'native_simulation_reference.go', 'native_redelegate_simulation_reference.go',
    'native_redelegate_swap_append_simulation_reference.go',
    'native_redelegate_current_source_simulation_reference.go',
    'native_redelegate_current_source_estimate_reference.go')]


def run_reference(revision, requests):
    with tempfile.TemporaryDirectory(prefix='rustaxa-redelegate-estimate-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        for name, content in observation.SNAPSHOT_HELPERS.items():
            (Path(directory) / name).write_bytes(content)
        command = Path(directory) / 'cmd/redelegate_current_source_estimate_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < len(SOURCES) - 1:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        result = subprocess.run(['go', 'run', '-mod=readonly', './cmd/redelegate_current_source_estimate_reference'], cwd=directory, input=json.dumps(requests).encode(), capture_output=True, check=True)
        return result.stdout, result.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    requests = json.loads(INPUT.read_bytes())['cases']
    if len(requests) != 1 or requests[0]['name'] != 'current_source_signed_h1':
        raise RuntimeError('Signed H1 estimate requires the accepted single request')
    requests += [{**requests[0], 'name':'current_source_minimum', 'gas':101912}, {**requests[0], 'name':'current_source_below_minimum', 'gas':101911}]
    captured = {label: run_reference(revision, requests) for label, revision in REVISIONS.items()}
    outputs = {label: data[0] for label, data in captured.items()}
    stderr = {label: data[1] for label, data in captured.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegate estimate probes differ')
    documents = [json.loads(data) for data in outputs.values()]
    for document in documents:
        if (document['schema'] != 1 or document['state_before'] != document['state_after']
                or [case['name'] for case in document['cases']] != [case['name'] for case in requests]):
            raise RuntimeError('Invalid redelegate estimate corpus')
    # This compiles the unchanged upstream search body. Its callback checks every
    # requested gas against the Go transcript and requires all probes consumed.
    cpp = support.cpp_reference(documents[0])
    outputs['cpp'] = (json.dumps(cpp, sort_keys=True, indent=2) + '\n').encode()
    manifest = {'schema': 1, 'references': REVISIONS,
                'snapshot_helper_sha256':{name:hashlib.sha256(content).hexdigest() for name,content in observation.SNAPSHOT_HELPERS.items()},
                'snapshot_target_sha256':{label:observation.snapshot_targets(rev) for label,rev in REVISIONS.items()},
                'snapshot_harness_sha256':hashlib.sha256(Path(observation.__file__).read_bytes()).hexdigest(),
                'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in SOURCES},
                'support_harness_sha256': hashlib.sha256(Path(support.__file__).read_bytes()).hexdigest(),
                'input_sha256': hashlib.sha256(INPUT.read_bytes()).hexdigest(),
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                'algorithm_sha256': cpp['algorithm_sha256'],
                'stderr_sha256': {label: hashlib.sha256(data).hexdigest() for label, data in stderr.items()},
                'sha256': {label: hashlib.sha256(data).hexdigest() for label, data in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
        for label, data in stderr.items():
            (FIXTURES / f'{label}.stderr.txt').write_bytes(data)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Signed H1 redelegate estimate manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: redelegate estimate changed')
        for label, data in stderr.items():
            if data != (FIXTURES / f'{label}.stderr.txt').read_bytes():
                raise RuntimeError(f'{label}: signed H1 estimate stderr changed')
    print('Both pinned signed H1 redelegate probe corpora match the unchanged C++ search')


if __name__ == '__main__':
    main()
