#!/usr/bin/env python3
"""Record/reproduce sequential delegator TraceRunner period2 over committed P1 with controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_delegator_sequence_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_trace_reference.go','native_redelegate_reward_h1_reference.go']]
FIXTURES=HERE/'fixtures/native_reward_delegator_sequence'
TRACE_TARGET=Path('taraxa/state/state_dry_runner/trace_runner.go')
TRACE_SHA='12fcddf4955e963d91318e5d18725868ecb29f5c32ae32db0fa73b7d99f65efd'
TRACE_NEEDLE=b'\t\tret, _ := evm.Main(&trx)\n'
LOG_HELPER=b'''package state_dry_runner
import "github.com/Taraxa-project/taraxa-evm/core/vm"
var rewardTraceLogObserver func([]vm.LogRecord)
func SetRewardTraceLogObserver(observer func([]vm.LogRecord)){rewardTraceLogObserver=observer}
func observeRewardTraceLogs(logs []vm.LogRecord){if rewardTraceLogObserver!=nil{rewardTraceLogObserver(logs)}}
'''
def digest(data):return hashlib.sha256(data).hexdigest()
def run_reference(revision,control=False):
 with tempfile.TemporaryDirectory(prefix='rustaxa-reward-trace-') as directory:
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
  trace=tree/TRACE_TARGET;data=trace.read_bytes()
  if digest(data)!=TRACE_SHA or data.count(TRACE_NEEDLE)!=1:raise RuntimeError('Trace result log observer target changed')
  if not control:trace.write_bytes(data.replace(TRACE_NEEDLE,TRACE_NEEDLE+b'\t\tobserveRewardTraceLogs(ret.Logs)\n',1))
  (trace.parent/'reward_trace_log_observer.go').write_bytes(LOG_HELPER)
  command=tree/'cmd/reward_trace';command.mkdir(parents=True)
  for index,p in enumerate(SUPPORT):
   source=p.read_text()
   if source.count('func main() {')!=1:raise RuntimeError('Support main changed')
   (command/p.name).write_text(source.replace('func main() {',f'func unusedSupport{index}Main() {{'))
  (command/SOURCE.name).write_bytes(SOURCE.read_bytes())
  result=subprocess.run(['rtk','proxy','go','run','-mod=readonly','./cmd/reward_trace'],cwd=tree,capture_output=True)
  if result.returncode:sys.stdout.buffer.write(result.stdout);sys.stderr.buffer.write(result.stderr);result.check_returncode()
  lines=result.stdout.splitlines(keepends=True)
  if not lines:raise RuntimeError('Missing output')
  return lines[-1],b''.join(lines[:-1]),result.stderr
def strip(value):
 if isinstance(value,dict):return {k:strip(v) for k,v in value.items() if k!='observation_witness'}
 if isinstance(value,list):return [strip(v) for v in value]
 return value
def validate(document):
 assert document['schema']==1 and document['period']==2 and document['state_before']==document['state_after']
 baseline=json.loads((HERE/'fixtures/native_redelegate_reward_h1/public.json').read_text())
 assert document['state_before']==baseline['state_before'] and len(document['cases'])==2
 assert [c['name'] for c in document['cases']]==['delegator_twice','delegator_prefix_then_claim']
 for case in document['cases']:
  requests=case['prefix']+case['targets']
  assert len(requests)==2
  for index,target in enumerate(requests):
   assert target['sender']==baseline['caller'] and target['nonce']==str(2**512+index)
   assert target['input']=='ef5cfb8c'+'0'*62+'31' and target['gas']==200000 and target['gas_price']==target['value']=='0'
  assert len(case['result'])==len(case['targets'])
  assert all(result['structLogs']==[] for result in case['result'])
  assert set(case['observation_witness'])=={'raw','balances','logs'}
  assert case['diagnostics']==''
  assert len(case['prefix'])==(0 if case['name']=='delegator_twice' else 1)
  assert all(target['to']=='00'*19+'fe' for target in requests)
  assert case['result']==[{'gas':61464,'failed':False,'returnValue':'','structLogs':[]}] * len(case['targets'])
  witness=case['observation_witness']
  assert witness['balances']==[{'address':'00'*19+'fe','operation':'sub','amount':'396','balance':'4404'},{'address':baseline['caller'],'operation':'add','amount':'396','balance':'3396'}]
  pairs=[('9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e','c28002'),('4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a','c8c68207d064800280'),('18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c','c28008'),('9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e','c28001'),('cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1','c48203e802'),('753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3','c583060ae002'),('753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3','c583060ae001'),('cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1','c48203e802'),('753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3','c583060ae003')]
  assert witness['raw']==[{'address':'00'*19+'fe','key':key,'value':value} for key,value in pairs]
  log={'address':'00'*19+'fe','topics':['9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7','0'*24+baseline['caller'],'0'*62+'31'],'data':'0'*61+'18c'}
  assert witness['logs']==[log]*len(case['targets'])

def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()};controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==strip(json.loads(controls[label][0]))
 dry='taraxa/state/state_dry_runner/dry_runner.go'
 manifest={'schema':1,'references':REVISIONS,'profile':'reward_only_P1_delegator_sequential_pending2_trace','baseline_sha256':{name:digest((HERE/name).read_bytes()) for name in ['native_redelegate_reward_h1_reference.go','native_redelegate_reward_h1_reference.py','fixtures/native_redelegate_reward_h1/public.json']},'root':json.loads(outputs['public'][0])['state_before']['root'],'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'trace_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+str(TRACE_TARGET)])) for label,revision in REVISIONS.items()},'trace_log_target_sha256':TRACE_SHA,'trace_log_helper_sha256':digest(LOG_HELPER),'dry_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry])) for label,revision in REVISIONS.items()},'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),'raw_observer_sha256':digest(custody.TRACE_HELPER),'control_execution_identical':True,'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
 combined={**outputs,**{label+'.control':data for label,data in controls.items()}}
 if args.record:FIXTURES.mkdir(parents=True,exist_ok=True);(FIXTURES/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
 else:assert json.loads((FIXTURES/'manifest.json').read_text())==manifest
 for label,(data,diagnostics,stderr) in combined.items():
  for suffix,content in [('json',data),('stdout.txt',diagnostics),('stderr.txt',stderr)]:
   path=FIXTURES/(label+'.'+suffix)
   if args.record:path.write_bytes(content)
   else:assert path.read_bytes()==content,str(path)
 print('Both pinned sequential delegator TraceRunner outputs and controls executed and matched')
if __name__=='__main__':main()
