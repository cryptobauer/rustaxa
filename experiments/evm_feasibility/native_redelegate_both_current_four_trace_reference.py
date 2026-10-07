#!/usr/bin/env python3
"""Execute both pinned redelegate TraceRunner oracles without changing source trees."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT
import native_redelegate_current_source_reference as observation

HERE = Path(__file__).resolve().parent
SOURCES = [HERE / name for name in (
    'native_simulation_reference.go',
    'native_redelegate_simulation_reference.go',
    'native_redelegate_trace_reference.go',
    'native_redelegate_swap_append_simulation_reference.go',
    'native_redelegate_both_current_four_simulation_reference.go',
    'native_redelegate_both_current_four_trace_reference.go')]
FIXTURES = HERE / 'fixtures/native_redelegate_both_current_four_trace'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-redelegate-trace-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        for name, content in observation.SNAPSHOT_HELPERS.items():
            (Path(directory) / name).write_bytes(content)
        command = Path(directory) / 'cmd/redelegate_existing_current_trace_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < len(SOURCES) - 1:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        result = subprocess.run(['go', 'run', '-mod=readonly', './cmd/redelegate_existing_current_trace_reference'], cwd=directory, capture_output=True, check=False)
        if result.returncode:
            print(result.stdout.decode(errors='replace'));print(result.stderr.decode(errors='replace'))
            raise RuntimeError(f'{revision}: Go exit {result.returncode}')
        return result.stdout, result.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    captured = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    outputs = {label: result[0] for label, result in captured.items()}
    stderr = {label: result[1] for label, result in captured.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Redelegate TraceRunner pins differ')
    for data in outputs.values():
        document = json.loads(data)
        if document["schema"] != 1 or document["state_before"] != document["state_after"] or len(document["cases"]) != 1:
            raise RuntimeError("Incomplete redelegate TraceRunner corpus")
        expected = {'partial_prefix_then_full_both_current_four'}
        if {case['name'] for case in document['cases']} != expected:
            raise RuntimeError('Redelegate trace case set changed')
        for case in document['cases']:
            if len(case['prefix']) != 1 or len(case['targets']) != 1:
                raise RuntimeError('Exactly one prefix/target required')
            if int(case['prefix'][0]['nonce']) != 2**512 or int(case['targets'][0]['nonce']) != 2**512+1:
                raise RuntimeError('Original supplied nonces changed')
            if len(case['result']) != len(case['targets']):
                raise RuntimeError(f"{case['name']}: target results missing")
            if any(row['structLogs'] for row in case['result']):
                raise RuntimeError('Direct precompile has unexpected opcode rows')
        witness = document['live_witness']
        # Bind roles to the committed configuration rather than map order.
        seed = json.loads((HERE / 'fixtures/native_redelegate_both_current_four_simulation/public.json').read_bytes())
        caller,owner = seed['caller'],seed['owner']
        if witness['period'] != 2 or len(witness['steps']) != 2:
            raise RuntimeError('Live H2 witness incomplete')
        source,destination = ['00'*19+x for x in ['31','32']]
        if witness['steps'][0]['request'] != document['cases'][0]['prefix'][0] or witness['steps'][1]['request'] != document['cases'][0]['targets'][0]:
            raise RuntimeError('Live/default trace requests differ')
        for index, step in enumerate(witness['steps']):
            facts = step['after']['facts']
            if step['gas_used'] != 101912 or len(step['logs']) != 1 or step['logs'][0]['data'] != format([300,400][index],'064x'):
                raise RuntimeError('Actual live H2 gas/log differs')
            for validator,owner_stake in [(source,'800'),(destination,'1200')]:
                if facts['delegations'][owner+'/'+validator] != {'stake':owner_stake,'last_updated':1}:
                    raise RuntimeError('Live H2 lost retained owner')
            if step['after']['accounts'][owner] != {'nonce':'2','balance':'1000'} or step['after']['accounts'][caller]['nonce'] != str(2**512+index+1):
                raise RuntimeError('Live H2 nonce/owner account differs')
        if witness['seed_root'] != seed['state_before']['root'] or witness['seed_row_count'] != 108:
            raise RuntimeError('Separate live H1 identity differs')
        for index,step in enumerate(witness['steps']):
            for suffix,value in [('3102','c28002'),('3202','c28002' if index==0 else 'c28003')]:
                if step['after']['raw'][witness['node_keys'][suffix]] != {'present':True,'value':value}:
                    raise RuntimeError('Actual H2 new-node witness differs')
        for key in ['b43e743da4ade83344c4edf921144d283b142466c18d45ca4840151c70461fa9','dccd1018a8d3ace84d0ec5bda96921351a6f1dc3974d1ae867905e02b03ec6dc']:
            if any(step['after']['raw'][key] != {'present':True,'value':'c28002'} for step in witness['steps']):
                raise RuntimeError('Live H2 old1 owner node not retained')
    committed = json.loads((HERE / 'fixtures/native_redelegate_both_current_four_simulation/public.json').read_bytes())['state_before']
    if any(json.loads(data)['state_before'] != committed for data in outputs.values()):
        raise RuntimeError('Complete accepted H1 seed changed')
    manifest = {'seed_sha256': hashlib.sha256((HERE / 'fixtures/native_redelegate_both_current_four_simulation/public.json').read_bytes()).hexdigest(), 'seed_root':json.loads(outputs['public'])['state_before']['root'], 'schema': 1, 'implementation_base':'a16acef1e','authority':'same period2 real prefix300->existing32/full400->existing32; defaultJSON only, internal writes remain separate authority', 'references': REVISIONS,
                'snapshot_helper_sha256':{name:hashlib.sha256(content).hexdigest() for name,content in observation.SNAPSHOT_HELPERS.items()},
                'snapshot_target_sha256':{label:observation.snapshot_targets(rev) for label,rev in REVISIONS.items()},
                'snapshot_harness_sha256':hashlib.sha256(Path(observation.__file__).read_bytes()).hexdigest(),
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
            raise RuntimeError('Redelegate TraceRunner manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: redelegate TraceRunner fixture changed')
        for label, data in stderr.items():
            if data != (FIXTURES / f'{label}.stderr.txt').read_bytes():
                raise RuntimeError(f'{label}: trace stderr changed')
    print('Both pinned redelegate TraceRunner references executed and matched')


if __name__ == '__main__':
    main()
