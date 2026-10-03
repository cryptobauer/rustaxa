#!/usr/bin/env python3
"""Record/reproduce actual pinned redelegation ABI and frame outputs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from reference import REVISIONS, ROOT
HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_redelegate_frames_reference.go'
SUPPORT = HERE / 'native_validator_info_reference.go'
SEED = HERE / 'fixtures/native_redelegate_observation/public.json'
FIXTURES = HERE / 'fixtures/native_redelegate_frames'
def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-redelegate-frames-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/redelegate_frames'
        command.mkdir(parents=True)
        source = SUPPORT.read_text()
        if source.count('func main() {') != 1:
            raise RuntimeError('Support main shape changed')
        (command / SUPPORT.name).write_text(source.replace('func main() {','func unusedInfoMain() {'))
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        (command / 'seed.json').write_bytes(SEED.read_bytes())
        output = subprocess.check_output(['go','run','-mod=readonly','./cmd/redelegate_frames'], cwd=directory)
        lines = output.splitlines(keepends=True)
        return lines[-1], b''.join(lines[:-1])
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    outputs = {label:run_reference(revision) for label,revision in REVISIONS.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegation frame outputs differ')
    for data, diagnostics in outputs.values():
        document = json.loads(data)
        if document['schema'] != 1 or len(document['cases']) != 21:
            raise RuntimeError('Incomplete redelegation frame corpus')
    manifest = {'schema':1,'references':REVISIONS,'exporter_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'harness_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'support_sha256':hashlib.sha256(SUPPORT.read_bytes()).hexdigest(),'seed_sha256':hashlib.sha256(SEED.read_bytes()).hexdigest(),'sha256':{label:[hashlib.sha256(data).hexdigest(), hashlib.sha256(diagnostics).hexdigest()] for label,(data,diagnostics) in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True,exist_ok=True)
        for label,(data,diagnostics) in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
            (FIXTURES / f'{label}.stdout.txt').write_bytes(diagnostics)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Redelegation frame manifest changed')
        for label,(data,diagnostics) in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes() or diagnostics != (FIXTURES / f'{label}.stdout.txt').read_bytes():
                raise RuntimeError(f'{label}: redelegation frame fixture changed')
    print('Both pinned redelegation frame outputs executed and matched')
if __name__ == '__main__':
    main()
