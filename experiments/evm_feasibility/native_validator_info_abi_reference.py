#!/usr/bin/env python3
"""Reproduce metadata ABI/Cacti evidence; retain native diagnostic stdout."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_validator_info_abi_reference.go'
SUPPORT = HERE / 'native_validator_info_reference.go'
FIXTURES = HERE / 'fixtures/native_validator_info_abi'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-validator-info-abi-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/validator_info_abi_reference'
        command.mkdir(parents=True)
        support = SUPPORT.read_text()
        if support.count('func main() {') != 1:
            raise RuntimeError('Metadata support main changed')
        (command / SUPPORT.name).write_text(support.replace('func main() {', 'func metadataSupportMain() {', 1))
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        output = subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/validator_info_abi_reference'], cwd=directory)
        lines = output.splitlines(keepends=True)
        document = json.loads(lines[-1])
        if document['schema'] != 1 or len(document['cases']) != 21:
            raise RuntimeError('Incomplete metadata ABI corpus')
        # Native ABI failures print diagnostics before returning an error. Keep
        # those observed bytes as a separate artifact; never change the contract.
        return lines[-1], b''.join(lines[:-1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    artifacts = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    if artifacts['public'] != artifacts['local']:
        raise RuntimeError('Pinned metadata ABI references differ')
    manifest = {'schema': 1, 'references': REVISIONS,
                'source_sha256': {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in (SOURCE, SUPPORT, Path(__file__))},
                'sha256': {label: [hashlib.sha256(data).hexdigest() for data in pair] for label, pair in artifacts.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, (data, diagnostics) in artifacts.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
            (FIXTURES / f'{label}.stdout.txt').write_bytes(diagnostics)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Metadata ABI manifest changed')
        for label, (data, diagnostics) in artifacts.items():
            if data != (FIXTURES / f'{label}.json').read_bytes() or diagnostics != (FIXTURES / f'{label}.stdout.txt').read_bytes():
                raise RuntimeError(f'{label}: metadata ABI evidence changed')
    print('Both pinned metadata ABI/Cacti references executed and matched')


if __name__ == '__main__':
    main()
