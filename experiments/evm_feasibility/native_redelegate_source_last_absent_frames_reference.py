#!/usr/bin/env python3
"""Record/reproduce one-transaction source-last absent frame outputs and independent controls."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import os
from reference import REVISIONS, ROOT
HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_redelegate_source_last_absent_frames_reference.go'
SUPPORT = HERE / 'native_validator_info_reference.go'
SEED = HERE / 'fixtures/native_redelegate_source_last_absent/public.json'
FIXTURES = HERE / 'fixtures/native_redelegate_source_last_absent_frames'
def run_reference(revision, control=False):
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
        result = subprocess.run(['go','run','-mod=readonly','./cmd/redelegate_frames'], cwd=directory, capture_output=True, check=False, env={**os.environ, 'CURRENT_SOURCE_FRAME_CONTROL': '1' if control else '0'})
        if result.returncode:
            print(result.stdout.decode(errors='replace'))
            print(result.stderr.decode(errors='replace'))
            raise RuntimeError(f'{revision}: Go execution exit {result.returncode}')
        output = result.stdout
        lines = output.splitlines(keepends=True)
        return lines[-1], b''.join(lines[:-1]), result.stderr
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    outputs = {label:run_reference(revision) for label,revision in REVISIONS.items()}
    controls = {label:run_reference(revision, True) for label,revision in REVISIONS.items()}
    if controls['public'] != controls['local']:
        raise RuntimeError('Pinned controls differ')
    for label in REVISIONS:
        measured = json.loads(outputs[label][0]); control = json.loads(controls[label][0])
        def strip(value):
            if isinstance(value, dict):
                return {key: strip(item) for key,item in value.items() if key not in {'calls','ordered_reads','ordered_raw_writes'}}
            if isinstance(value, list):
                return [strip(item) for item in value]
            return value
        if strip(measured) != control:
            raise RuntimeError(f'{label}: independent control differs')
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegation frame outputs differ')
    for data, diagnostics, stderr in outputs.values():
        document = json.loads(data)
        if document['schema'] != 1 or len(document['cases']) != 7:
            raise RuntimeError('Incomplete redelegation frame corpus')
        expected = {'direct_source_last_absent', 'nested_source_last_absent', 'static_source_last_absent',
                    'parent_revert_source_last_absent', 'two_source_last_absent_calls_parent_revert', 'nested_nonpayable', 'nested_underfunded'}
        if {row['name'] for row in document['cases']} != expected:
            raise RuntimeError('New-destination frame case set changed')
        for row in document['cases']:
            for field,value in [('retained_item_key','00'*19+'33'),('retained_position_key','01000000')]:
                key = row[field]
                if any(row[view].get(key) != value for view in ['prior_raw','after_raw']) or any(w['key'] == key for w in row['ordered_raw_writes']):
                    raise RuntimeError('Retained membership changed')
            if row['prior_raw'][row['source_current_key']] != '':
                raise RuntimeError('Source current node must be absent')
            if len(row['calls']) != (2 if row['two_calls'] else 1):
                raise RuntimeError('Incomplete call vector')
            if any(call['route_staticcall'] != row['static'] for call in row['calls']):
                raise RuntimeError('STATICCALL route differs')
            for field in ['ordered_raw_writes','ordered_reads']:
                if [item for call in row['calls'] for item in call[field]] != row[field]:
                    raise RuntimeError('Per-call vector differs')
            if any(call['setup_reads'] for call in row['calls'][1:]):
                raise RuntimeError('Later setup includes old reads')
            if row['refund_before'] or row['refund_after'] or row['log_count_before']:
                raise RuntimeError('Nonzero initial log/refund')
            if row['cumulative_logs'] != row['logs']:
                raise RuntimeError('Cumulative log suffix differs')
            successful = row['calls'][0]['native_called'] and not row['calls'][0]['native_error']
            if len(row['ordered_raw_writes']) != (17 if successful else 0):
                raise RuntimeError('Native stream differs')
            if row['parent_revert'] and row['logs']:
                raise RuntimeError('Parent logs survive')
            last = {write['key']:write['value'] for write in row['ordered_raw_writes']}
            if any(row['after_raw'][key] != value for key,value in last.items()):
                raise RuntimeError('Dirty LAST witness differs')
            if not successful and row['after_raw'] != row['prior_raw']:
                raise RuntimeError('Failed-only raw changed')
            if successful and [w['value'] for w in row['ordered_raw_writes'] if w['key'] == row['source_current_key']] != ['c28001']:
                raise RuntimeError('Current-source creation differs')
            if row['two_calls'] and (row['calls'][1]['native_error'] != 'Delegation does not exist' or row['calls'][1]['ordered_raw_writes']):
                raise RuntimeError('Second normal failure differs')

    manifest = {'frame_context': {'go_evm_config':'params.TestChainConfig','rust_chain_id':666,'timestamp':0,'gas_price':0,'threshold':100,'vote_step':10,'blocks_per_year':1,'minimum_deposit':100,'maximum_stake':1000000,'seed_scope':'actual source-last genesis initial frozen42-key view; no prefix; full1000->32; dirty raw only','period':1,'sender':'aa','wrapper':'d1','sender_wrapper_balance':1000000,'native_balance':5000,'nonce':0,'direct_target_nonce':0,'log_boundary':'one-Main cumulative logs and exact per-call suffix; no clear/commit; refund zero','transaction_gas':200000,'block_gas':1000000,'native_forks':'fix/Magnolia/Ficus/Cornus0; both Aspen disabled','max_supply':1000000000,'principal':5000,'votes':500}, 'schema':1,'implementation_base':'a11202839','references':REVISIONS,'exporter_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'harness_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'support_sha256':hashlib.sha256(SUPPORT.read_bytes()).hexdigest(),'seed_sha256':hashlib.sha256(SEED.read_bytes()).hexdigest(),'sha256':{label:[hashlib.sha256(data).hexdigest(), hashlib.sha256(diagnostics).hexdigest(), hashlib.sha256(stderr).hexdigest()] for label,(data,diagnostics,stderr) in outputs.items()}}
    manifest['control_sha256'] = {label:[hashlib.sha256(item).hexdigest() for item in data] for label,data in controls.items()}
    outputs.update({label+'.control': data for label,data in controls.items()})
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
