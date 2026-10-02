#!/usr/bin/env python3
"""Execute both pinned escrow TraceRunner oracles without changing source trees."""
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
    'native_escrow_trace_reference.go')]
FIXTURES = HERE / 'fixtures/native_escrow_trace'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-escrow-trace-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        command = Path(directory) / 'cmd/escrow_trace_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < len(SOURCES) - 1:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/escrow_trace_reference'], cwd=directory)


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
            raise RuntimeError("Incomplete escrow TraceRunner corpus")
        cases = {case['name']: case for case in document['cases']}
        expected = {'single_zero', 'single_value', 'prefix_value_then_value', 'two_values',
                    'low_gas_then_value', 'prefix_failure_then_value', 'stale_nonce_preserved'}
        if set(cases) != expected:
            raise RuntimeError('Escrow trace case set changed')
        for name, case in cases.items():
            if len(case['result']) != len(case['targets']):
                raise RuntimeError(f'{name}: target results missing')
            for index, row in enumerate(case['result']):
                failed = name == 'stale_nonce_preserved' or (name == 'low_gas_then_value' and index == 0)
                gas = 100000 if name == 'stale_nonce_preserved' else 21272 if failed else 22272
                if row['failed'] != failed or row['structLogs'] or row['returnValue'] or row['gas'] != gas:
                    raise RuntimeError(f'{name}: unexpected target admission/result')
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
                raise RuntimeError(f'{label}: escrow TraceRunner fixture changed')
    print('Both pinned escrow TraceRunner references executed and matched')


if __name__ == '__main__':
    main()
