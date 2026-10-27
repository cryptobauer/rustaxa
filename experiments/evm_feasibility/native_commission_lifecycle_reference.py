#!/usr/bin/env python3
"""Record/reproduce reachable commission lifecycle matrix on both reference pins."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
from reference import REVISIONS, ROOT
import native_retained_commission_reference as retained
import native_v1_custody_reference as custody
import native_redelegate_reward_reference as reward
import native_retained_commission_trace_reference as tracing
import native_reward_commission_trace_reference as trace_source
import native_metadata_estimate_reference as cpp_support
HERE=Path(__file__).resolve().parent
GO=HERE/'native_commission_lifecycle_reference.go'
FIXTURES=HERE/'fixtures/native_commission_lifecycle'
SUPPORT=retained.SUPPORT+[retained.GO]
HOOK_IDENTITIES={}
def digest(data): return hashlib.sha256(data).hexdigest()
def run(revision, control, keys):
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    with tempfile.TemporaryDirectory(prefix='oracle-', dir=artifact) as directory:
        tree = Path(directory)
        archive = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['rtk', 'proxy', 'tar', '-x', '-C', directory], input=archive, check=True)
        target = tree / custody.TRACE_TARGET
        source = target.read_bytes()
        if digest(source) != custody.TRACE_SHA256 or source.count(custody.TRACE_NEEDLE) != 1:
            raise RuntimeError('Pinned observer target changed')
        if not control:
            source = source.replace(custody.TRACE_NEEDLE, custody.TRACE_INSERTION, 1)
        for op, expression in [('add', 'Add'), ('sub', 'Sub')]:
            needle = ('self.set_balance(new(big.Int).' + expression + '(self.Balance, amount))').encode()
            if source.count(needle) != 1:
                raise RuntimeError('Pinned balance observer shape changed')
            if not control:
                source = source.replace(needle, needle + b'\n\t\tobserveRewardBalance(self.addr,"' + op.encode() + b'",amount,self.Balance)', 1)
        target.write_bytes(source)
        (target.parent / 'retained_raw_observer.go').write_bytes(custody.TRACE_HELPER)
        (target.parent / 'retained_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
        target=tree/trace_source.TRACE_TARGET
        trace_original=target.read_bytes()
        assert digest(trace_original)==trace_source.TRACE_SHA
        changed=trace_original
        if not control:
            changed=changed.replace(tracing.PREFIX,b'\t\tobserveRetainedTrace("prefix",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("prefix",true,block_state,&trx,&ret,err)\n',1)
            changed=changed.replace(tracing.TARGET,b'\t\tobserveRetainedTrace("target",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("target",true,block_state,&trx,&ret,err)\n',1)
        target.write_bytes(changed)
        (target.parent/'lifecycle_trace_observer.go').write_bytes(tracing.HELPER)
        HOOK_IDENTITIES[revision+('/control' if control else '/observed')]={'account_original':custody.TRACE_SHA256,'account_effective':digest((tree/custody.TRACE_TARGET).read_bytes()),'trace_original':digest(trace_original),'trace_effective':digest(changed),'dry_runner':digest((tree/'taraxa/state/state_dry_runner/dry_runner.go').read_bytes())}

        command = tree / 'cmd/commission_lifecycle'
        command.mkdir(parents=True)
        for index, support in enumerate(SUPPORT):
            text = support.read_text()
            if text.count('func main() {') != 1:
                raise RuntimeError('Support main changed')
            (command / support.name).write_text(text.replace('func main() {', f'func unusedSupport{index}Main() {{', 1))
        (command / GO.name).write_bytes(GO.read_bytes())
        result = subprocess.run(['rtk', 'proxy', 'go', 'run', '-mod=readonly', './cmd/commission_lifecycle'], cwd=tree, capture_output=True,
                                env={**os.environ, 'RETAINED_RAW_KEYS': json.dumps(keys), 'RETAINED_CONTROL': '1' if control else '0'})
        if len(result.stdout)+len(result.stderr)>67108864:
            raise RuntimeError('Go child exceeded combined 64 MiB output bound')
        if result.returncode:
            os.write(1, result.stdout)
            os.write(2, result.stderr)
            result.check_returncode()
        evidence=Path(tempfile.mkdtemp(prefix='oracle-result-',dir=artifact))
        (evidence/'stdout').write_bytes(result.stdout)
        (evidence/'stderr').write_bytes(result.stderr)
        lines = result.stdout.splitlines(keepends=True)
        if not lines:
            raise RuntimeError('Missing oracle output')
        return lines[-1], b''.join(lines[:-1]), result.stderr


def validate(d):
    assert d['schema']==1 and len(d['scenarios'])==56
    assert len({s['name'] for s in d['scenarios']})==56
    for s in d['scenarios']:
        assert s['configuration']['Hardforks']['PhalaenopsisHfBlockNum']==0
        cases=s['cases']
        assert cases[0]['execution_error']=='This account is not owner of specified validator'
        assert not cases[1]['execution_error'] and not cases[1]['consensus_error']
        assert len(cases[1]['logs'])==1
        assert int(cases[1]['logs'][0]['data'],16)>0
        expected='This account is not owner of specified validator' if s['phase']=='final' else ''
        assert cases[2]['execution_error']==expected
        before=s['state_before']['validators'][0]['validator']
        assert before is not None and before['TotalStake']==0
        expected_count=0 if s['phase']=='final' else 1 if s['phase']=='partial' else 2
        assert before['UndelegationsCount']==expected_count
        if s['phase']=='claim_before_final':
            assert not cases[-2]['execution_error']
            assert cases[-1]['execution_error']=='This account is not owner of specified validator'
        for snap in (s['state_before'],s['state_after']):
            assert snap['seed_rows'] and snap['root'] and snap['native_raw']
        assert s['consumers']['call']['execution_error']==''
        assert s['consumers']['estimates']
        assert s['consumers']['trace']['result'][0]['failed'] is False
        frame=s['frame']
        assert frame['parent_error']=='execution reverted' and not frame['native_observation']['error']
        assert not frame['logs'] and frame['native_observation']['logs']==cases[1]['logs']
        assert frame['ordered_raw_writes']==cases[1]['ordered_raw_writes']
        for case in cases:
            call=case['consumers']['call']
            for field in ('gas_used','consensus_error','execution_error','output','logs','ordered_raw_writes','ordered_balance_effects'):
                assert call[field]==case[field], (s['name'],case['name'],field)
            for sequence in (False,True):
                trace=case['consumers']['trace_sequence' if sequence else 'trace']
                calls=trace['observation_witness']
                assert len(calls)==len(trace['requests'])==1+int(sequence)
                assert calls[0]['result']['ordered_raw_writes']==case['ordered_raw_writes']
                assert calls[0]['result']['execution_error']==case['execution_error']
                assert calls[0]['result']['gas_used']==case['gas_used']
                if sequence: assert calls[0]['after']==calls[1]['before']
    for off,on in zip(d['scenarios'][:28],d['scenarios'][28:],strict=True):
        cfg=json.loads(json.dumps(on['configuration']))
        cfg['Hardforks']['CactiHf']['BlockNum']=2**64-1
        assert cfg==off['configuration']


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
    assert REVISIONS=={'public':'6c7e5338b22d5e596cc2365a88d1f94840e1ee1b','local':'bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418'}
    outputs={label:run(pin,False,[]) for label,pin in REVISIONS.items()}
    assert outputs['public']==outputs['local']
    for output in outputs.values(): validate(json.loads(output[0]))
    keys=json.loads(outputs['public'][0])['raw_key_inventory']
    controls={label+'.control':run(pin,True,keys) for label,pin in REVISIONS.items()}
    for label in REVISIONS:
        assert tracing.strip(retained.strip(json.loads(outputs[label][0])))==tracing.strip(retained.strip(json.loads(controls[label+'.control'][0])))
    combined={**outputs,**controls}
    tempfile.tempdir=os.environ['RUN_ARTIFACT_DIR']
    cases=[]
    for scenario in json.loads(outputs['public'][0])['scenarios']:
        for row in scenario['cases']:
            for cap in row['consumers']['estimates']:
                cases.append({'name':scenario['name']+'/'+row['name']+'/'+str(cap['cap']),'cap':cap['cap'],'probes':[{'gas':p['gas'],'output':p} for p in cap['probes']]})
    cpp=cpp_support.cpp_reference({'cases':cases})
    assert cpp['algorithm_sha256']=='7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759'
    assert all(c['consumed']==len(g['probes']) for c,g in zip(cpp['cases'],cases,strict=True))
    cpp_data=(json.dumps(cpp,indent=2,sort_keys=True)+'\n').encode()

    manifest={'schema':1,'references':REVISIONS,'source_sha256':{p.name:digest(p.read_bytes()) for p in [GO,Path(__file__),*SUPPORT,Path(custody.__file__),Path(reward.__file__),Path(retained.__file__),Path(tracing.__file__),Path(trace_source.__file__),Path(cpp_support.__file__),HERE/'reference.py']},'raw_observer_sha256':digest(custody.TRACE_HELPER),'balance_observer_sha256':digest(reward.BALANCE_HELPER),'observer_target_sha256':custody.TRACE_SHA256,'trace_observer_sha256':digest(tracing.HELPER),'trace_target_sha256':trace_source.TRACE_SHA,'cpp_source_sha256':digest(cpp_support.CPP_SOURCE.read_bytes()),'algorithm_sha256':cpp['algorithm_sha256'],'hook_free_controls_match':True,'hook_identities':HOOK_IDENTITIES,'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),'sha256':{k:[digest(b) for b in v] for k,v in combined.items()},'matrix':[{k:s[k] for k in ('name','queue','phase','topology','cacti','execution_period')} for s in json.loads(outputs['public'][0])['scenarios']]}
    if args.record: FIXTURES.mkdir(parents=True,exist_ok=True)
    for label,streams in combined.items():
        for suffix,data in zip(('json','stdout.txt','stderr.txt'),streams):
            path=FIXTURES/(label+'.'+suffix)
            if args.record:path.write_bytes(data)
            else:assert path.read_bytes()==data,str(path)
    cpp_path=FIXTURES/'cpp.json'
    if args.record:cpp_path.write_bytes(cpp_data)
    else:assert cpp_path.read_bytes()==cpp_data
    manifest['cpp_sha256']=digest(cpp_data)
    path=FIXTURES/'manifest.json'
    if args.record:path.write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
    else:assert json.loads(path.read_text())==manifest
    print('56 lifecycle seeds, 2 pins, 2 hook-free controls; native/call/estimate-probe/trace observations reproduced')
if __name__=='__main__': main()
