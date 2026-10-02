#!/usr/bin/env python3
"""Run the escrow oracle from both pinned archives without changing them."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
SUPPORT = HERE / "native_validator_info_reference.go"
SOURCE = HERE / 'native_escrow_transfer_reference.go'
FIXTURES = HERE / 'fixtures/native_escrow_transfer'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-validator-info-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/validator_info_reference'
        command.mkdir(parents=True)
        support = SUPPORT.read_text()
        if support.count("func main() {") != 1:
            raise RuntimeError("Shared main shape changed")
        (command / SUPPORT.name).write_text(support.replace("func main() {", "func unusedInfoMain() {"))
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/validator_info_reference'], cwd=directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    if artifacts['public'] != artifacts['local']:
        raise RuntimeError('Pinned escrow references differ')
    for data in artifacts.values():
        document = json.loads(data)
        if document['schema'] != 1 or len(document['cases']) != 12:
            raise RuntimeError('Incomplete escrow corpus')
    manifest = {'schema': 1, 'references': REVISIONS,
                'support_sha256': hashlib.sha256(SUPPORT.read_bytes()).hexdigest(),
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
                raise RuntimeError(f'{label}: escrow fixture changed')
    print('Both pinned escrow references executed and matched')


if __name__ == '__main__':
    main()
