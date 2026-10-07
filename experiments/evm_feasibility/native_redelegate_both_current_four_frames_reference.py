#!/usr/bin/env python3
"""Record/reproduce four-transaction both-current count4 frame outputs and independent controls."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import os
from reference import REVISIONS, ROOT
HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_redelegate_both_current_four_frames_reference.go'
SUPPORT = HERE / 'native_validator_info_reference.go'
SEED = HERE / 'fixtures/native_redelegate_both_current_four/public.json'
FIXTURES = HERE / 'fixtures/native_redelegate_both_current_four_frames'
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
        result = subprocess.run(['go','run','-mod=readonly','./cmd/redelegate_frames'], cwd=directory, capture_output=True, check=False, env={**os.environ, 'BOTH_CURRENT_FOUR_FRAME_CONTROL': '1' if control else '0'})
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
        expected = {'direct_both_current_four', 'nested_both_current_four', 'static_both_current_four',
                    'parent_revert_both_current_four', 'two_both_current_four_calls_parent_revert', 'nested_nonpayable', 'nested_underfunded'}
        if {row['name'] for row in document['cases']} != expected:
            raise RuntimeError('New-destination frame case set changed')
        for row in document['cases']:
            seed = json.loads(SEED.read_text())['cases'][0]
            caller, owner = '00'*19+'aa', '00'*19+'a1'
            for index, prefix in enumerate(row['prefixes']):
                if prefix['input'] != seed['attempts'][index]['input'] or prefix['from'] != [caller,owner,owner][index] or prefix['nonce'] != [1,1,2][index]:
                    raise RuntimeError('Real prefix caller/input/nonce changed')
                if prefix['transaction_gas_used'] != [101912,101848,101848][index]:
                    raise RuntimeError('Actual prefix gas changed')
                for key in [seed['source_current_key'],seed['destination_current_key']]:
                    if prefix['after_raw'][key] != ['c28002','c28003','c28004'][index]:
                        raise RuntimeError('Real prefix did not produce both current nodes')
            if row['input'] != seed['attempts'][3]['input'] or row['accounts_before'][owner] != {'nonce':'3','balance':'1000'} or row['accounts'][owner] != row['accounts_before'][owner]:
                raise RuntimeError('Target changes retained owner account or input')
            if any(row['prior_raw'][seed[key]] for key in ['source_old_key','destination_old_key']):
                raise RuntimeError('Owner prefix did not delete both old nodes')
            if row['prefixes'][2]['after_raw'] != row['prior_raw'] or row['prefixes'][2]['accounts'] != row['accounts_before'] or row['prefixes'][0]['after_raw'] != row['prefixes'][1]['prior_raw'] or row['prefixes'][0]['accounts'] != row['prefixes'][1]['accounts_before']:
                raise RuntimeError('Prefix/target boundary differs')
            if row['prefixes'][1]['after_raw'] != row['prefixes'][2]['prior_raw'] or row['prefixes'][1]['accounts'] != row['prefixes'][2]['accounts_before']:
                raise RuntimeError('Owner repeated prefix boundary differs')
            if row['prior_raw'][row['source_current_key']] != 'c28004':
                raise RuntimeError('Source current node not produced by prefix')
            for transaction, count in [(prefix,1) for prefix in row['prefixes']] + [(row, 2 if row['two_calls'] else 1)]:
                if any(call['route_staticcall'] != (row['static'] if transaction is row else False) for call in transaction['calls']):
                    raise RuntimeError('Per-call STATICCALL route differs')
                if len(transaction['calls']) != count:
                    raise RuntimeError('Incomplete per-call vector')
                for field in ['ordered_raw_writes', 'ordered_reads']:
                    if [item for call in transaction['calls'] for item in call[field]] != transaction[field]:
                        raise RuntimeError(f'Per-call {field} differs')
                if any(call['setup_reads'] for call in transaction['calls'][1:]):
                    raise RuntimeError('Later setup contains old reads')
                if transaction['refund_before'] or transaction['refund_after']:
                    raise RuntimeError('Refund makes gas boundary invalid')
                if transaction['cumulative_logs'][transaction['log_count_before']:] != transaction['logs']:
                    raise RuntimeError('Transaction log suffix differs')
            if row['log_count_before'] != 3 or row['cumulative_logs'][:3] != sum([p['logs'] for p in row['prefixes']], []):
                raise RuntimeError('Target loses prefix log')
            if [len(p['ordered_raw_writes']) for p in row['prefixes']] != [12,10,10]:
                raise RuntimeError('Prefix native stream changed')
            successful = row['calls'][0]['native_called'] and not row['calls'][0]['native_error']
            if len(row['ordered_raw_writes']) != (15 if successful else 0):
                raise RuntimeError('Target native stream changed')
            if row['parent_revert'] and (row['logs'] or len(row['cumulative_logs']) != 3):
                raise RuntimeError('Parent log rollback differs')
            prefix_last = {write['key']:write['value'] for prefix in row['prefixes'] for write in prefix['ordered_raw_writes']}
            target_last = {write['key']:write['value'] for write in row['ordered_raw_writes']}
            if any(row['prefixes'][2]['after_raw'][key] != value for key,value in prefix_last.items()):
                raise RuntimeError('Prefix dirty row differs from last actual write')
            if any(row['after_raw'][key] != value for key,value in target_last.items()):
                raise RuntimeError('Target dirty row differs from last actual write')
            if successful:
                source_operations = [write['value'] for write in row['ordered_raw_writes'] if write['key'] == row['source_current_key']]
                if source_operations != ['c28003', 'c28004']:
                    raise RuntimeError('Source node load-copy stream differs')
                destination_key = json.loads(SEED.read_text())['cases'][0]['destination_current_key']
                if (row['prior_raw'][destination_key] != 'c28004' or [w['value'] for w in row['ordered_raw_writes'] if w['key']==destination_key] != ['c28003','c28005']):
                    raise RuntimeError('Destination node loaded increment differs')
            if row['two_calls'] and (row['calls'][1]['native_error'] != 'Delegation does not exist' or row['calls'][1]['ordered_raw_writes']):
                raise RuntimeError('Second target normal failure differs')

    manifest = {'frame_context': {'go_evm_config':'params.TestChainConfig','rust_chain_id':666,'timestamp':0,'gas_price':0,'threshold':100,'vote_step':10,'blocks_per_year':1,'minimum_deposit':100,'maximum_stake':1000000,'seed_scope':'accepted current-source initial actual AA genesis42-key view; actual caller300 then owner100 twice then fullcaller700->existing32; dirty raw only','period':1,'sender':'d1','wrapper':'aa','owner':'a1','owner_balance':1000,'sender_wrapper_balance':1000000,'native_balance':4000,'nonce':1,'direct_target_nonce':2,'prefix_senders':['aa','a1','a1'],'log_boundary':'cumulative Go logs plus exact suffix; no clear/commit; refund zero','transaction_gas':200000,'block_gas':1000000,'native_forks':'fix/Magnolia/Ficus/Cornus0; Aspen1=0 Aspen2 disabled','max_supply':8000,'principal':4000,'votes':400}, 'schema':1,'implementation_base':'a16acef1e','references':REVISIONS,'exporter_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'harness_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'support_sha256':hashlib.sha256(SUPPORT.read_bytes()).hexdigest(),'seed_sha256':hashlib.sha256(SEED.read_bytes()).hexdigest(),'sha256':{label:[hashlib.sha256(data).hexdigest(), hashlib.sha256(diagnostics).hexdigest(), hashlib.sha256(stderr).hexdigest()] for label,(data,diagnostics,stderr) in outputs.items()}}
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
