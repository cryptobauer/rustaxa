#!/usr/bin/env python3
"""Execute both pinned metadata TraceRunner oracles without changing source trees."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
SOURCES = [HERE / name for name in (
    'native_simulation_reference.go', 'native_validator_info_reference.go',
    'native_metadata_trace_reference.go')]
FIXTURES = HERE / 'fixtures/native_metadata_trace'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-metadata-trace-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/metadata_trace_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < 2:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/metadata_trace_reference'], cwd=directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    outputs = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Metadata DryRunner pins differ')
    for data in outputs.values():
        document = json.loads(data)
        if document["schema"] != 1 or document["state_before"] != document["state_after"] or len(document["cases"]) != 7:
            raise RuntimeError("Incomplete metadata TraceRunner corpus")
        cases = {case['name']: case for case in document['cases']}
        prefix_query = cases['prefix_then_query']['result'][0]
        target_query = cases['two_updates_then_query']['result'][-1]
        if (prefix_query['failed'] or '616c706861' not in prefix_query['returnValue']
                or target_query['failed'] or '62657461' not in target_query['returnValue']
                or cases['abi_error_then_valid']['result'][-1]['failed']
                or cases['low_gas_then_valid']['result'][-1]['failed']
                or not cases['stale_nonce_preserved']['result'][0]['failed']):
            raise RuntimeError('Metadata trace did not exercise its declared state/failure paths')
    manifest = {'schema': 1, 'references': REVISIONS,
                'source_sha256': {source.name: hashlib.sha256(source.read_bytes()).hexdigest() for source in SOURCES},
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                'sha256': {label: hashlib.sha256(data).hexdigest() for label, data in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Metadata DryRunner manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: metadata TraceRunner fixture changed')
    print('Both pinned metadata TraceRunner references executed and matched')


if __name__ == '__main__':
    main()
