#!/usr/bin/env python3
"""Record/reproduce mixed commission and delegator TraceRunner period2 over committed P1 with controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_mixed_claims_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_trace_reference.go','native_redelegate_reward_h1_reference.go']]
FIXTURES=HERE/'fixtures/native_reward_mixed_claims'
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
 assert [c['name'] for c in document['cases']]==['commission_then_delegator','delegator_then_commission']
 for case in document['cases']:
  assert case['prefix']==[] and len(case['targets'])==2 and case['diagnostics']==''
  expected=[(baseline['owner'],'d0eebfe2'),(baseline['caller'],'ef5cfb8c')]
  if case['name']=='delegator_then_commission':expected.reverse()
  for target,(sender,selector) in zip(case['targets'],expected):
   assert target['sender']==sender and target['nonce']==str(2**512)
   assert target['to']=='00'*19+'fe' and target['input']==selector+'0'*62+'31'
   assert target['gas']==200000 and target['gas_price']==target['value']=='0'
  assert len(case['result'])==2 and all(result['structLogs']==[] for result in case['result'])
  assert set(case['observation_witness'])=={'raw','balances','logs'}
  commission_first=case['name']=='commission_then_delegator'
  gases=[41464,61464] if commission_first else [61464,41464]
  assert case['result']==[{'gas':gas,'failed':False,'returnValue':'','structLogs':[]} for gas in gases]
  reward_key='18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c'
  pairs=[('9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e','c28002'),('4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a','c8c68207d064800280'),(reward_key,'c28080' if commission_first else 'c28008'),('9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e','c28001'),('cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1','c48203e802'),('753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3','c583060ae002')]
  pairs=([(reward_key,'c482031880')]+pairs) if commission_first else (pairs+[(reward_key,'c28080')])
  witness=case['observation_witness']
  assert witness['raw']==[{'address':'00'*19+'fe','key':key,'value':value} for key,value in pairs]
  rewards=[(baseline['owner'],'8','1008',4792),(baseline['caller'],'396','3396',4396)] if commission_first else [(baseline['caller'],'396','3396',4404),(baseline['owner'],'8','1008',4396)]
  balances=[]
  for sender,amount,balance,native in rewards:
   balances.extend([{'address':'00'*19+'fe','operation':'sub','amount':amount,'balance':str(native)},{'address':sender,'operation':'add','amount':amount,'balance':balance}])
  assert witness['balances']==balances
  log=lambda sender,amount:{'address':'00'*19+'fe','topics':['f0ec9e0f6add850a1738c5822244e26ffc3d1f14da7537aa240582b25af12ad0' if amount=='8' else '9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7','0'*24+sender,'0'*62+'31'],'data':format(int(amount),'064x')}
  first=log(rewards[0][0],rewards[0][1]);second=log(rewards[1][0],rewards[1][1])
  assert witness['logs']==[first,first,second]

def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()};controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==strip(json.loads(controls[label][0]))
 dry='taraxa/state/state_dry_runner/dry_runner.go'
 manifest={'schema':1,'references':REVISIONS,'profile':'reward_only_P1_mixed_claims_pending2_trace','baseline_sha256':{name:digest((HERE/name).read_bytes()) for name in ['native_redelegate_reward_h1_reference.go','native_redelegate_reward_h1_reference.py','fixtures/native_redelegate_reward_h1/public.json']},'root':json.loads(outputs['public'][0])['state_before']['root'],'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'trace_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+str(TRACE_TARGET)])) for label,revision in REVISIONS.items()},'trace_log_target_sha256':TRACE_SHA,'trace_log_helper_sha256':digest(LOG_HELPER),'dry_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry])) for label,revision in REVISIONS.items()},'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),'raw_observer_sha256':digest(custody.TRACE_HELPER),'control_execution_identical':True,'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
 combined={**outputs,**{label+'.control':data for label,data in controls.items()}}
 if args.record:FIXTURES.mkdir(parents=True,exist_ok=True);(FIXTURES/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
 else:assert json.loads((FIXTURES/'manifest.json').read_text())==manifest
 for label,(data,diagnostics,stderr) in combined.items():
  for suffix,content in [('json',data),('stdout.txt',diagnostics),('stderr.txt',stderr)]:
   path=FIXTURES/(label+'.'+suffix)
   if args.record:path.write_bytes(content)
   else:assert path.read_bytes()==content,str(path)
 print('Both pinned mixed commission and delegator TraceRunner outputs and controls executed and matched')
if __name__=='__main__':main()
