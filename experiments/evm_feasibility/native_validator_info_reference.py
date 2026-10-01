#!/usr/bin/env python3
"""Run the metadata oracle from both pinned archives without changing them."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_validator_info_reference.go'
FIXTURES = HERE / 'fixtures/native_validator_info'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-validator-info-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/validator_info_reference'
        command.mkdir(parents=True)
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/validator_info_reference'], cwd=directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    if artifacts['public'] != artifacts['local']:
        raise RuntimeError('Pinned metadata references differ')
    for data in artifacts.values():
        document = json.loads(data)
        if document['schema'] != 1 or len(document['cases']) != 14:
            raise RuntimeError('Incomplete metadata corpus')
    manifest = {'schema': 1, 'references': REVISIONS,
                'exporter_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                'sha256': {label: hashlib.sha256(data).hexdigest() for label, data in artifacts.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in artifacts.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Metadata manifest changed')
        for label, data in artifacts.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: metadata fixture changed')
    print('Both pinned metadata references executed and matched')


if __name__ == '__main__':
    main()
