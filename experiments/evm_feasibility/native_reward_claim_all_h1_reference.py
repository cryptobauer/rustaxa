#!/usr/bin/env python3
"""Record/reproduce canonical positive noarg claim-all same-height DryRunner with controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_claim_all_h1_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_reward_h1_reference.go']]
FIXTURES=HERE/'fixtures/native_reward_claim_all_h1'
DRY_TARGET=Path('taraxa/state/state_dry_runner/dry_runner.go')
DRY_SHA='abc2b1ace3669007190f901fea2da144bcc00167468d02939b847c7fdac506ed'
DRY_NEEDLE=b'\tret, err := evm.Main(trx)\n'
POST_HELPER=b'''package state_dry_runner
import "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
var rewardH1PostObserver func(*state_evm.BlockState)
func SetRewardH1PostObserver(observer func(*state_evm.BlockState)){rewardH1PostObserver=observer}
func observeRewardH1Post(state *state_evm.BlockState){if rewardH1PostObserver!=nil{rewardH1PostObserver(state)}}
'''
VM_TARGET=Path('core/vm/evm.go')
VM_SHA={'public':'4389d46cc6fef3159c0da9fb3f548d9515aa555a2af85124824625c2c3f3e121','local':'9d63e0f58b3c2b32987f0979b88a30cccd294787eb7f5c8d45cc2ca1cbe5882f'}
VM_NEEDLE=b'precompiled.RequiredGas(frame, self)'
QUOTE_HELPER=b'''package vm
var rewardH1QuoteObserver func(CallFrame,uint64)
func SetRewardH1QuoteObserver(observer func(CallFrame,uint64)){rewardH1QuoteObserver=observer}
func observeRewardH1QuoteRequired(frame CallFrame,required uint64)uint64{if rewardH1QuoteObserver!=nil{rewardH1QuoteObserver(frame,required)};return required}
'''
def digest(data):return hashlib.sha256(data).hexdigest()
def run_reference(revision,control=False):
 with tempfile.TemporaryDirectory(prefix='rustaxa-reward-h1-') as directory:
  tree=Path(directory);archive=subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'archive',revision]);subprocess.run(['rtk','proxy','tar','-x','-C',directory],input=archive,check=True)
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
  for path,sha,needle,insertion,helper,name in [
   (DRY_TARGET,DRY_SHA,DRY_NEEDLE,DRY_NEEDLE+b'\tobserveRewardH1Post(block_state)\n',POST_HELPER,'reward_h1_post_observer.go'),
   (VM_TARGET,VM_SHA[next(label for label,rev in REVISIONS.items() if rev==revision)],VM_NEEDLE,b'observeRewardH1QuoteRequired(frame, '+VM_NEEDLE+b')',QUOTE_HELPER,'reward_h1_quote_observer.go')]:
   target=tree/path;data=target.read_bytes()
   if digest(data)!=sha or data.count(needle)!=1:raise RuntimeError('Copy observer target changed: '+str(path))
   target.write_bytes(data.replace(needle,insertion,1));(target.parent/name).write_bytes(helper)
  command=tree/'cmd/reward_h1_raw';command.mkdir(parents=True)
  for index,p in enumerate(SUPPORT):
   source=p.read_text()
   if source.count('func main() {')!=1:raise RuntimeError('Support main changed')
   (command/p.name).write_text(source.replace('func main() {',f'func unusedSupport{index}Main() {{'))
  (command/SOURCE.name).write_bytes(SOURCE.read_bytes())
  result=subprocess.run(['rtk','proxy','go','run','-mod=readonly','./cmd/reward_h1_raw'],cwd=tree,capture_output=True)
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
 scalar=json.loads((HERE/'fixtures/native_reward_delegator_h1/public.json').read_text())['cases'][0]
 assert document['state_before']==baseline['state_before'] and len(document['cases'])==1
 c=document['cases'][0];assert c['name']=='claim_all_positive' and c['supplied_nonce']==str(2**512)
 assert c['input']=='0b83a727' and c['to']=='00'*19+'fe' and c['gas']==200000 and c['gas_price']==c['value']=='0'
 assert c['output']['effective_nonce']=='1' and not c['output']['execution_error'] and not c['output']['consensus_error']
 assert c['output']['gas_used']==111272 and c['output']['return']=='' and c['output']['logs']==scalar['output']['logs']
 assert c['quotes']==[{'caller':document['caller'],'input':'0b83a727','required_gas':90000,'supplied_native_gas':178728}]
 assert c['ordered_balance_effects']==scalar['ordered_balance_effects']
 assert len(c['ordered_raw_writes'])==12 and c['ordered_raw_writes'][:6]==scalar['ordered_raw_writes']
 expected=dict(c['prior_raw']);assert len(expected)==23 and set(expected)==set(c['after_raw'])
 for w in c['ordered_raw_writes']:
  assert w['address']==c['to'] and w['key'] in expected
  expected[w['key']]=w['value']
 assert expected==c['after_raw']
 assert c['accounts']=={document['caller']:{'balance':'3396','nonce':'2'},document['owner']:{'balance':'1000','nonce':'0'},c['to']:{'balance':'4404','nonce':'1'}}
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()};controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==strip(json.loads(controls[label][0]))
 dry='taraxa/state/state_dry_runner/dry_runner.go'
 manifest={'schema':1,'references':REVISIONS,'profile':'canonicalreward_only_P1_positive_noarg_claim_all_sameH1_raw','baseline_sha256':{name:digest((HERE/name).read_bytes()) for name in ['native_redelegate_reward_h1_reference.go','native_redelegate_reward_h1_reference.py','fixtures/native_redelegate_reward_h1/public.json','fixtures/native_reward_delegator_h1/public.json']},'root':json.loads(outputs['public'][0])['state_before']['root'],'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'dry_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry])) for label,revision in REVISIONS.items()},'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),'raw_observer_sha256':digest(custody.TRACE_HELPER),'control_execution_identical':True,'copy_quote_and_post_state_in_both_runs':True,'copy_observers':{'dry_target':str(DRY_TARGET),'dry_sha256':DRY_SHA,'dry_needle_sha256':digest(DRY_NEEDLE),'dry_helper_sha256':digest(POST_HELPER),'vm_target':str(VM_TARGET),'vm_sha256':VM_SHA,'vm_needle_sha256':digest(VM_NEEDLE),'vm_helper_sha256':digest(QUOTE_HELPER)},'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
 combined={**outputs,**{label+'.control':data for label,data in controls.items()}}
 if args.record:FIXTURES.mkdir(parents=True,exist_ok=True);(FIXTURES/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
 else:assert json.loads((FIXTURES/'manifest.json').read_text())==manifest
 for label,(data,diagnostics,stderr) in combined.items():
  for suffix,content in [('json',data),('stdout.txt',diagnostics),('stderr.txt',stderr)]:
   path=FIXTURES/(label+'.'+suffix)
   if args.record:path.write_bytes(content)
   else:assert path.read_bytes()==content,str(path)
 print('Both pinned canonical positive noarg claim-all H1 outputs and controls executed and matched')
if __name__=='__main__':main()
