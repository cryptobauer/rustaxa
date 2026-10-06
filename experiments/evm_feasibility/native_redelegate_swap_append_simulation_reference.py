#!/usr/bin/env python3
"""Execute both pinned redelegate DryRunner oracles without changing source trees."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
SOURCES = [HERE / name for name in (
    'native_simulation_reference.go',
    'native_redelegate_simulation_reference.go',
    'native_redelegate_swap_append_simulation_reference.go')]
FIXTURES = HERE / 'fixtures/native_redelegate_swap_append_simulation'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-redelegate-zero_simulation-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/redelegate_zero_simulation_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < len(SOURCES) - 1:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        result = subprocess.run(['go', 'run', '-mod=readonly', './cmd/redelegate_zero_simulation_reference'], cwd=directory, capture_output=True, check=True)
        return result.stdout, result.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    captured = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    outputs = {label: result[0] for label, result in captured.items()}
    stderr = {label: result[1] for label, result in captured.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Redelegate DryRunner pins differ')
    for data in outputs.values():
        document = json.loads(data)
        if (document['schema'] != 1 or document['state_before'] != document['state_after']
                or not document['repeat_identical'] or not document['committed_state_unchanged']
                or len(document['cases']) != 1
                or document['cases'][0]['name'] != 'swap_append'
                or document['state_before']['period'] != 1):
            raise RuntimeError('Incomplete zero DryRunner corpus')
    manifest = {'schema': 1, 'references': REVISIONS,
                'source_sha256': {source.name: hashlib.sha256(source.read_bytes()).hexdigest() for source in SOURCES},
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
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
            raise RuntimeError('Redelegate DryRunner manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: redelegate DryRunner fixture changed')
        for label, data in stderr.items():
            if data != (FIXTURES / f'{label}.stderr.txt').read_bytes():
                raise RuntimeError(f'{label}: zero_simulation stderr changed')
    print('Both pinned redelegate DryRunner references executed and matched')


if __name__ == '__main__':
    main()
