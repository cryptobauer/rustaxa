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
SOURCE = HERE / 'native_redelegate_swap_append_frames_reference.go'
SUPPORT = HERE / 'native_validator_info_reference.go'
SEED = HERE / 'fixtures/native_redelegate_swap_append/public.json'
FIXTURES = HERE / 'fixtures/native_redelegate_swap_append_frames'
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
        result = subprocess.run(['go','run','-mod=readonly','./cmd/redelegate_frames'], cwd=directory, capture_output=True, check=True)
        output = result.stdout
        lines = output.splitlines(keepends=True)
        return lines[-1], b''.join(lines[:-1]), result.stderr
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    outputs = {label:run_reference(revision) for label,revision in REVISIONS.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegation frame outputs differ')
    for data, diagnostics, stderr in outputs.values():
        document = json.loads(data)
        if document['schema'] != 1 or len(document['cases']) != 7:
            raise RuntimeError('Incomplete redelegation frame corpus')
        expected = {'direct_swap_append', 'nested_swap_append', 'static_swap_append',
                    'parent_revert_swap_append', 'two_swap_append_calls_parent_revert', 'nested_nonpayable', 'nested_underfunded'}
        if {row['name'] for row in document['cases']} != expected:
            raise RuntimeError('New-destination frame case set changed')
        for row in document['cases']:
            if len(row['calls']) != (2 if row['two_calls'] else 1):
                raise RuntimeError('Per-call vector does not cover every frame')
            if [write for call in row['calls'] for write in call['ordered_raw_writes']] != row['ordered_raw_writes']:
                raise RuntimeError('Per-call ordered effects do not cover aggregate writes')
            if [read for call in row['calls'] for read in call['ordered_reads']] != row['ordered_reads']:
                raise RuntimeError('Per-call reads do not cover aggregate execution reads')
            if any(call['setup_reads'] for call in row['calls'][1:]):
                raise RuntimeError('Later quote setup contains previous execution reads')
    manifest = {'frame_context': {'go_evm_config':'params.TestChainConfig','rust_chain_id':666,'timestamp':0,'gas_price':0,'threshold':100,'vote_step':10,'blocks_per_year':1,'minimum_deposit':100,'maximum_stake':1000000,'seed_scope':'first-attempt observed plus seed inspection rows; touched-row only','period':1,'sender':'aa','wrapper':'d1','sender_wrapper_balance':1000000,'native_balance':5000,'nonce':1,'transaction_gas':200000,'block_gas':1000000,'native_forks':'fix/Magnolia/Ficus/Cornus0; both Aspen disabled','max_supply':1000000000,'principal':5000,'votes':500}, 'schema':1,'references':REVISIONS,'exporter_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'harness_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'support_sha256':hashlib.sha256(SUPPORT.read_bytes()).hexdigest(),'seed_sha256':hashlib.sha256(SEED.read_bytes()).hexdigest(),'sha256':{label:[hashlib.sha256(data).hexdigest(), hashlib.sha256(diagnostics).hexdigest(), hashlib.sha256(stderr).hexdigest()] for label,(data,diagnostics,stderr) in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True,exist_ok=True)
        for label,(data,diagnostics,stderr) in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
            (FIXTURES / f'{label}.stdout.txt').write_bytes(diagnostics)
            (FIXTURES / f'{label}.stderr.txt').write_bytes(stderr)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Redelegation frame manifest changed')
        for label,(data,diagnostics,stderr) in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes() or diagnostics != (FIXTURES / f'{label}.stdout.txt').read_bytes() or stderr != (FIXTURES / f'{label}.stderr.txt').read_bytes():
                raise RuntimeError(f'{label}: redelegation frame fixture changed')
    print('Both pinned redelegation frame outputs executed and matched')
if __name__ == '__main__':
    main()
