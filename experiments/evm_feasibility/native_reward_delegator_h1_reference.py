#!/usr/bin/env python3
"""Record/reproduce canonical reward-only P1 same-height DryRunner with controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_delegator_h1_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_reward_h1_reference.go']]
FIXTURES=HERE/'fixtures/native_reward_delegator_h1'
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
  if source.count(custody.TRACE_NEEDLE)!=1:raise RuntimeError("Raw observer shape changed")
  if not control:source=source.replace(custody.TRACE_NEEDLE,custody.TRACE_INSERTION,1)
  (target.parent/"native_v1_custody_observer.go").write_bytes(custody.TRACE_HELPER)
  target.write_bytes(source);(target.parent/'reward_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
  command=tree/'cmd/reward_h1_raw';command.mkdir(parents=True)
  for index,p in enumerate(SUPPORT):
   source=p.read_text()
   if source.count('func main() {')!=1:raise RuntimeError('Support main changed')
   (command/p.name).write_text(source.replace('func main() {',f'func unusedSupport{index}Main() {{'))
  (command/SOURCE.name).write_bytes(SOURCE.read_bytes())
  result=subprocess.run(['go','run','-mod=readonly','./cmd/reward_h1_raw'],cwd=tree,capture_output=True)
  if result.returncode:sys.stdout.buffer.write(result.stdout);sys.stderr.buffer.write(result.stderr);result.check_returncode()
  lines=result.stdout.splitlines(keepends=True)
  if not lines:raise RuntimeError('Missing output')
  return lines[-1],b''.join(lines[:-1]),result.stderr
def strip(value):
 if isinstance(value,dict):return {k:strip(v) for k,v in value.items() if k not in ('ordered_balance_effects','ordered_raw_writes')}
 if isinstance(value,list):return [strip(v) for v in value]
 return value
def validate(document):
 assert document['schema']==1 and document['state_before']==document['state_after']
 assert document['repeat_identical'] and document['committed_state_unchanged'] and document['state_unchanged_after_each']
 assert document['go_native_period']==document['go_evm_block']==1 and document['reward_minted']=='800'
 baseline=json.loads((HERE/'fixtures/native_redelegate_reward_h1/public.json').read_text())
 assert document['state_before']==baseline['state_before'] and len(document['cases'])==1
 case=document['cases'][0];assert case['name']=='delegator_claim' and case['supplied_nonce']==str(2**512)
 assert case['output']['effective_nonce']=='1' and not case['output']['execution_error'] and not case['output']['consensus_error']
 assert case=={'gas': 200000, 'gas_price': '0', 'input': 'ef5cfb8c0000000000000000000000000000000000000000000000000000000000000031', 'name': 'delegator_claim', 'ordered_balance_effects': [{'address': '00000000000000000000000000000000000000fe', 'amount': '396', 'balance': '4404', 'operation': 'sub'}, {'address': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008', 'amount': '396', 'balance': '3396', 'operation': 'add'}], 'ordered_raw_writes': [{'address': '00000000000000000000000000000000000000fe', 'key': '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e', 'value': 'c28002'}, {'address': '00000000000000000000000000000000000000fe', 'key': '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a', 'value': 'c8c68207d064800180'}, {'address': '00000000000000000000000000000000000000fe', 'key': '18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c', 'value': 'c28008'}, {'address': '00000000000000000000000000000000000000fe', 'key': '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e', 'value': 'c28001'}, {'address': '00000000000000000000000000000000000000fe', 'key': 'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1', 'value': 'c48203e801'}, {'address': '00000000000000000000000000000000000000fe', 'key': 'b43e743da4ade83344c4edf921144d283b142466c18d45ca4840151c70461fa9', 'value': 'c583060ae002'}], 'output': {'consensus_error': '', 'effective_nonce': '1', 'execution_error': '', 'gas_used': 61464, 'logs': [{'address': '00000000000000000000000000000000000000fe', 'data': '000000000000000000000000000000000000000000000000000000000000018c', 'topics': ['9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7', '00000000000000000000000077952ce83ca3cad9f7adcfabeda85bd2f1f52008', '0000000000000000000000000000000000000000000000000000000000000031']}], 'return': ''}, 'supplied_nonce': '13407807929942597099574024998205846127479365820592393377723561443721764030073546976801874298166903427690031858186486050853753882811946569946433649006084096', 'to': '00000000000000000000000000000000000000fe', 'value': '0'}
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()};controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==strip(json.loads(controls[label][0]))
 dry='taraxa/state/state_dry_runner/dry_runner.go'
 manifest={'schema':1,'references':REVISIONS,'profile':'canonicalreward_only_P1_delegator_sameH1_raw','baseline_sha256':{name:digest((HERE/name).read_bytes()) for name in ['native_redelegate_reward_h1_reference.go','native_redelegate_reward_h1_reference.py','fixtures/native_redelegate_reward_h1/public.json']},'root':json.loads(outputs['public'][0])['state_before']['root'],'go_version':subprocess.check_output(['go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'dry_runner_sha256':{label:digest(subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry])) for label,revision in REVISIONS.items()},'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),'raw_observer_sha256':digest(custody.TRACE_HELPER),'control_execution_identical':True,'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
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
