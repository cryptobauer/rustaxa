#!/usr/bin/env python3
"""Execute both pinned metadata DryRunner oracles without changing source trees."""
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
    'native_metadata_simulation_reference.go')]
FIXTURES = HERE / 'fixtures/native_metadata_simulation'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-metadata-dry-runner-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/metadata_simulation_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < 2:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/metadata_simulation_reference'], cwd=directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    outputs = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Metadata DryRunner pins differ')
    expected = {'replace', 'empty', 'maximum', 'both_too_long', 'missing_validator',
                'malformed', 'nonpayable_malformed', 'insufficient_gas', 'intrinsic_gas'}
    for data in outputs.values():
        document = json.loads(data)
        if (document['schema'] != 1 or not document['repeat_identical']
                or not document['committed_state_unchanged']
                or document['state_before'] != document['state_after']
                or {case['name'] for case in document['cases']} != expected
                or len(document['cases']) != len(expected)):
            raise RuntimeError('Incomplete metadata DryRunner corpus')
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
                raise RuntimeError(f'{label}: metadata DryRunner fixture changed')
    print('Both pinned metadata DryRunner references executed and matched')


if __name__ == '__main__':
    main()
