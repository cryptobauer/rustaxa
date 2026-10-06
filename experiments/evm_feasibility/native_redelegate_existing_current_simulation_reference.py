#!/usr/bin/env python3
"""Execute both pinned redelegate DryRunner oracles without changing source trees."""
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
    'native_redelegate_swap_append_simulation_reference.go',
    'native_redelegate_existing_current_simulation_reference.go')]
FIXTURES = HERE / 'fixtures/native_redelegate_existing_current_simulation'


def run_reference(revision):
    with tempfile.TemporaryDirectory(prefix='rustaxa-redelegate-existing-current-simulation-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        for name, content in observation.SNAPSHOT_HELPERS.items():
            (Path(directory) / name).write_bytes(content)
        command = Path(directory) / 'cmd/redelegate_existing_current_simulation_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < len(SOURCES) - 1:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        result = subprocess.run(['go', 'run', '-mod=readonly', './cmd/redelegate_existing_current_simulation_reference'], cwd=directory, capture_output=True, check=False)
        if result.returncode:
            print(result.stdout.decode(errors='replace')); print(result.stderr.decode(errors='replace'))
            raise RuntimeError(f'{revision}: actual Go exit {result.returncode}')
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
                or document['cases'][0]['name'] != 'existing_current_signed_h1'
                or document['state_before']['period'] != 1):
            raise RuntimeError('Incomplete signed H1 DryRunner corpus')
        cfg = document['state_before']['configuration']
        if (cfg['DPOS']['DelegationLockingPeriod'] != 2 or cfg['Hardforks']['CornusHf']['DelegationLockingPeriod'] != 3 or cfg['Hardforks']['CactiHf']['DelegationLockingPeriod'] != 7):
            raise RuntimeError('Explicit native locking configuration differs')
        caller = document['caller']; source,destination,third = ['00'*19+x for x in ['31','32','33']]
        prefix = document['prefix']; state = document['state_before']; facts = state['native_facts']; raw = state['native_raw']
        if prefix['after_prefix'] != prefix['after_end_block'] or prefix['after_end_block'] != prefix['after_commit']:
            raise RuntimeError('Committed prefix authority differs')
        if state['native_facts'] != prefix['after_commit']['facts'] or state['native_raw'] != prefix['after_commit']['raw']:
            raise RuntimeError('Committed H1 graph facts disconnected')
        if facts['memberships'][caller] != [source,destination] or facts['delegations'][caller+'/'+source] != {'stake':'700','last_updated':1} or facts['delegations'][caller+'/'+destination] != {'stake':'1300','last_updated':1} or facts['delegations'][caller+'/'+third] is not None:
            raise RuntimeError('Actual H1 caller shape differs')
        for validator,stake,head in [(source,'1700',1),(destination,'2300',1)]:
            if facts['validators'][validator] != {'stake':stake,'head':head,'rewards':'0','commission_rewards':'0'}:
                raise RuntimeError('H1 validator/reward/head facts differ')
            if facts['delegations']['00'*19+'a1/'+validator] != {'stake':'1000','last_updated':0}:
                raise RuntimeError('Other delegator changed')
        source_key = 'b43e743da4ade83344c4edf921144d283b142466c18d45ca4840151c70461fa9'
        destination_key = 'dccd1018a8d3ace84d0ec5bda96921351a6f1dc3974d1ae867905e02b03ec6dc'
        if raw[source_key] != {'present':True,'value':'c28002'} or raw[destination_key] != {'present':True,'value':'c28002'}:
            raise RuntimeError('Current-source H1 precondition differs')
        accounts = {row['address']:row for row in state['accounts']}
        if accounts[caller]['nonce'] != '1' or accounts[caller]['balance'] != '3000' or accounts['00'*19+'fe']['balance'] != '4000' or not accounts['00'*19+'a1']['exists'] or accounts['00'*19+'a1']['balance'] != '1000' or accounts['00'*19+'a1']['nonce'] != '0':
            raise RuntimeError('Post-debit committed accounts differ')
        signed = prefix['signed_transaction']; output = document['cases'][0]['output']
        if (prefix['receipt']['gas_used'] != 101912 or output['gas_used'] != 101912 or output['logs'][0]['data'] != format(700,'064x') or prefix['receipt']['logs'][0]['data'] != format(300,'064x') or facts['validators'][third] is not None or facts['memberships']['00'*19+'a1'] != [source,destination]):
            raise RuntimeError('Exact H1 events, gas, owner order or absent-third authority differs')
        if signed['sender'] != caller or not signed['signature_valid'] or signed['nonce'] != '0' or signed['chain_id'] != 666 or output['effective_nonce'] != '2' or output['consensus_error'] or output['execution_error'] or output['return'] or len(output['logs']) != 1:
            raise RuntimeError('Signed prefix/same-height DryRunner admission differs')

    dry_file = 'taraxa/state/state_dry_runner/dry_runner.go'
    dry_hashes = {label: hashlib.sha256(subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry_file])).hexdigest() for label,revision in REVISIONS.items()}
    manifest = {'dry_runner_file_sha256':dry_hashes, 'schema': 1, 'implementation_base':'072fc39fb','profile':'existing_current_signed_h1','snapshot_target_sha256':{label:observation.snapshot_targets(revision) for label,revision in REVISIONS.items()},'snapshot_helper_sha256':{name:hashlib.sha256(content).hexdigest() for name,content in observation.SNAPSHOT_HELPERS.items()},'snapshot_harness_sha256':hashlib.sha256(Path(observation.__file__).read_bytes()).hexdigest(),'root':json.loads(outputs['public'])['state_before']['root'], 'references': REVISIONS,
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
                raise RuntimeError(f'{label}: signed H1 simulation stderr changed')
    print('Both pinned redelegate DryRunner references executed and matched')


if __name__ == '__main__':
    main()
