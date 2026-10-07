#!/usr/bin/env python3
"""Record/reproduce canonical reward-only P1 same-height DryRunner with controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_redelegate_reward_h1_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go']]
FIXTURES=HERE/'fixtures/native_redelegate_reward_h1'
def digest(data):return hashlib.sha256(data).hexdigest()
def run_reference(revision,control=False):
 with tempfile.TemporaryDirectory(prefix='rustaxa-reward-h1-') as directory:
  tree=Path(directory);archive=subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'archive',revision]);subprocess.run(['tar','-x','-C',directory],input=archive,check=True)
  target=tree/custody.TRACE_TARGET;source=target.read_bytes()
  if digest(source)!=custody.TRACE_SHA256:raise RuntimeError('Account observer target changed')
  for operation,expression in [('add','Add'),('sub','Sub')]:
   needle=('self.set_balance(new(big.Int).'+expression+'(self.Balance, amount))').encode()
   if source.count(needle)!=1:raise RuntimeError('Balance observer shape changed')
   if not control:source=source.replace(needle,needle+b'\n\t\tobserveRewardBalance(self.addr,"'+operation.encode()+b'",amount,self.Balance)',1)
  target.write_bytes(source);(target.parent/'reward_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
  command=tree/'cmd/reward_h1';command.mkdir(parents=True)
  for index,p in enumerate(SUPPORT):
   source=p.read_text()
   if source.count('func main() {')!=1:raise RuntimeError('Support main changed')
   (command/p.name).write_text(source.replace('func main() {',f'func unusedSupport{index}Main() {{'))
  (command/SOURCE.name).write_bytes(SOURCE.read_bytes())
  result=subprocess.run(['go','run','-mod=readonly','./cmd/reward_h1'],cwd=tree,capture_output=True)
  if result.returncode:sys.stdout.buffer.write(result.stdout);sys.stderr.buffer.write(result.stderr);result.check_returncode()
  lines=result.stdout.splitlines(keepends=True)
  if not lines:raise RuntimeError('Missing output')
  return lines[-1],b''.join(lines[:-1]),result.stderr
def strip(value):
 if isinstance(value,dict):return {k:strip(v) for k,v in value.items() if k!='ordered_balance_effects'}
 if isinstance(value,list):return [strip(v) for v in value]
 return value
def validate(document):
 assert document['schema']==1 and document['state_before']==document['state_after']
 assert document['repeat_identical'] and document['committed_state_unchanged'] and document['state_unchanged_after_each']
 assert document['go_native_period']==document['go_evm_block']==1 and document['reward_minted']=='800'
 assert len(document['cases'])==10
 assert document['caller']=='77952ce83ca3cad9f7adcfabeda85bd2f1f52008' and document['owner']=='5d5c99edf529335160ff180fa141dd4967fc00d2'
 state=document['state_before'];assert state['period']==1 and state['seed_rows']
 cfg=state['configuration'];assert cfg['DPOS']['YieldPercentage']==20 and cfg['DPOS']['BlocksPerYear']==1
 accounts={row['address']:row for row in state['accounts']};assert accounts[document['caller']]['nonce']=='0' and accounts[document['caller']]['balance']=='3000'
 assert accounts[document['owner']]['nonce']=='0' and accounts[document['owner']]['balance']=='1000'
 assert accounts['00'*19+'fe']['nonce']=='1' and accounts['00'*19+'fe']['balance']=='4800'
 for case in document['cases']:assert case['supplied_nonce']==str(2**512) and case['output']['effective_nonce']=='1'
 assert len({c['name'] for c in document['cases']})==10
 assert state['root']=='0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b' and len(state['seed_rows'])==92
 expected=[('partial',101912,'',''),('missing_source',101912,'Validator does not exist',''),('missing_destination',101912,'Validator does not exist',''),('insufficient_source',101912,'Insufficient delegation',''),('remainder_below_minimum',101912,'Insufficient delegation',''),('same_validator',101912,'From and to validators are the same',''),('malformed',101272,'abi: cannot marshal in to go type: length insufficient 0 require 32',''),('nonpayable_malformed',21272,'Method is not payable',''),('insufficient_native_gas',21912,'out of gas',''),('intrinsic_gas',21000,'','intrinsic gas too low')]
 assert [(c['name'],c['output']['gas_used'],c['output']['execution_error'],c['output']['consensus_error']) for c in document['cases']]==expected
 native='00'*19+'fe';caller=document['caller']
 assert document['cases'][0]['ordered_balance_effects']==[{'address':native,'operation':'sub','amount':'396','balance':'4404'},{'address':caller,'operation':'add','amount':'396','balance':'3396'}]
 assert document['cases'][7]['ordered_balance_effects']==[{'address':caller,'operation':'sub','amount':'1','balance':'2999'},{'address':native,'operation':'add','amount':'1','balance':'4801'}]
 for i,c in enumerate(document['cases']):
  assert c['output']['return']==''
  if i!=0:assert c['output']['logs']==[]
  if i not in (0,7):assert c['ordered_balance_effects']==[]
 logs=document['cases'][0]['output']['logs'];assert len(logs)==2
 assert logs[0]['data']==f'{396:064x}' and logs[1]['data']==f'{300:064x}'
 assert [l['topics'][0] for l in logs]==['9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7','12e144c27d0bad08abc77c66a640b5cf15a03a93f6582f40de6932b033a5fa5e']
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()};controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==strip(json.loads(controls[label][0]))
 dry='taraxa/state/state_dry_runner/dry_runner.go'
 manifest={'schema':1,'references':REVISIONS,'profile':'canonicalreward_only_P1_sameH1','root':json.loads(outputs['public'][0])['state_before']['root'],'go_version':subprocess.check_output(['go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'dry_runner_sha256':{label:digest(subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry])) for label,revision in REVISIONS.items()},'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),'control_execution_identical':True,'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
 combined={**outputs,**{label+'.control':data for label,data in controls.items()}}
 if args.record:FIXTURES.mkdir(parents=True,exist_ok=True);(FIXTURES/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
 else:assert json.loads((FIXTURES/'manifest.json').read_text())==manifest
 for label,(data,diagnostics,stderr) in combined.items():
  for suffix,content in [('json',data),('stdout.txt',diagnostics),('stderr.txt',stderr)]:
   path=FIXTURES/(label+'.'+suffix)
   if args.record:path.write_bytes(content)
   else:assert path.read_bytes()==content,str(path)
 print('Both pinned canonical reward-only H1 outputs and controls executed and matched')
if __name__=='__main__':main()
