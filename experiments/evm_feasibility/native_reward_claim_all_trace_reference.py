#!/usr/bin/env python3
"""Record/reproduce direct claim-all TraceRunner period2 over committed P1 with controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_claim_all_trace_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_trace_reference.go','native_redelegate_reward_h1_reference.go']]
FIXTURES=HERE/'fixtures/native_reward_claim_all_trace'
TRACE_TARGET=Path('taraxa/state/state_dry_runner/trace_runner.go')
TRACE_SHA='12fcddf4955e963d91318e5d18725868ecb29f5c32ae32db0fa73b7d99f65efd'
TRACE_NEEDLE=b'\t\tret, _ := evm.Main(&trx)\n'
PREFIX_NEEDLE=b'\t\tevm.Main(&trx)\n'
PREFIX_INSERTION=b'\t\tprefixRet,prefixError:=evm.Main(&trx)\n\t\tobserveRewardTracePrefix(prefixRet,prefixError,block_state,&trx)\n'
LOG_HELPER=b'''package state_dry_runner
import ("github.com/Taraxa-project/taraxa-evm/core/vm";"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm")
var rewardTraceLogObserver func([]vm.LogRecord)
func SetRewardTraceLogObserver(observer func([]vm.LogRecord)){rewardTraceLogObserver=observer}
func observeRewardTraceLogs(logs []vm.LogRecord){if rewardTraceLogObserver!=nil{rewardTraceLogObserver(logs)}}
var rewardTracePrefixObserver func(vm.ExecutionResult,error,*state_evm.BlockState,*vm.Transaction)
func SetRewardTracePrefixObserver(observer func(vm.ExecutionResult,error,*state_evm.BlockState,*vm.Transaction)){rewardTracePrefixObserver=observer}
func observeRewardTracePrefix(ret vm.ExecutionResult,err error,state *state_evm.BlockState,trx *vm.Transaction){if rewardTracePrefixObserver!=nil{rewardTracePrefixObserver(ret,err,state,trx)}}
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
  if data.count(PREFIX_NEEDLE)!=1:raise RuntimeError('Prefix copy observer target changed')
  data=data.replace(PREFIX_NEEDLE,PREFIX_INSERTION,1)
  if not control:data=data.replace(TRACE_NEEDLE,TRACE_NEEDLE+b'\t\tobserveRewardTraceLogs(ret.Logs)\n',1)
  trace.write_bytes(data)
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
 direct=json.loads((HERE/'fixtures/native_reward_claim_all/public.json').read_text())['cases'][0]
 assert [c['name'] for c in document['cases']]==['claim_all_empty_prefix','claim_all_ordinary_prefix']
 for index,case in enumerate(document['cases']):
  assert len(case['prefix'])==index and len(case['targets'])==1
  target=case['targets'][0]
  assert target['sender']==baseline['caller'] and target['to']=='00'*19+'fe' and target['nonce']==str(2**512+index)
  assert target['input']=='0b83a727' and target['gas']==200000 and target['gas_price']==target['value']=='0'
  assert case['result']==[{'gas':111272,'failed':False,'returnValue':'','structLogs':[]}]
  assert case['diagnostics']==''
  assert case['prefix_results']==([] if index==0 else [{'gas':21000,'consensus_error':'','execution_error':'','outer_error':'','return_value':'','logs':0,'sender_nonce':str(2**512+1),'sender_balance':'3000','native_balance':'4800'}])
  if index:
   prefix=case['prefix'][0]
   assert prefix['sender']==prefix['to']==baseline['caller'] and prefix['nonce']==str(2**512)
   assert prefix['input']=='' and prefix['gas']==21000 and prefix['gas_price']==prefix['value']=='0'
  assert set(case['observation_witness'])=={'raw','balances','logs'}
  witness=case['observation_witness']
  assert witness['raw']==direct['ordered_raw_writes'] and len(witness['raw'])==12
  assert witness['balances']==direct['ordered_balance_effects'] and witness['logs']==direct['logs']

def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()};controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==strip(json.loads(controls[label][0]))
 dry='taraxa/state/state_dry_runner/dry_runner.go'
 manifest={'schema':1,'references':REVISIONS,'profile':'reward_only_P1_positive_claim_all_pending2_trace','baseline_sha256':{name:digest((HERE/name).read_bytes()) for name in ['native_redelegate_reward_h1_reference.go','native_redelegate_reward_h1_reference.py','fixtures/native_redelegate_reward_h1/public.json','native_reward_claim_all_reference.go','native_reward_claim_all_reference.py','fixtures/native_reward_claim_all/public.json']},'root':json.loads(outputs['public'][0])['state_before']['root'],'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'trace_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+str(TRACE_TARGET)])) for label,revision in REVISIONS.items()},'trace_log_target_sha256':TRACE_SHA,'trace_log_helper_sha256':digest(LOG_HELPER),'prefix_needle_sha256':digest(PREFIX_NEEDLE),'prefix_insertion_sha256':digest(PREFIX_INSERTION),'dry_runner_sha256':{label:digest(subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+dry])) for label,revision in REVISIONS.items()},'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),'raw_observer_sha256':digest(custody.TRACE_HELPER),'control_execution_identical':True,'prefix_copy_observer_in_both_runs':True,'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
 combined={**outputs,**{label+'.control':data for label,data in controls.items()}}
 if args.record:FIXTURES.mkdir(parents=True,exist_ok=True);(FIXTURES/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
 else:assert json.loads((FIXTURES/'manifest.json').read_text())==manifest
 for label,(data,diagnostics,stderr) in combined.items():
  for suffix,content in [('json',data),('stdout.txt',diagnostics),('stderr.txt',stderr)]:
   path=FIXTURES/(label+'.'+suffix)
   if args.record:path.write_bytes(content)
   else:assert path.read_bytes()==content,str(path)
 print('Both pinned direct claim-all TraceRunner outputs and controls executed and matched')
if __name__=='__main__':main()
