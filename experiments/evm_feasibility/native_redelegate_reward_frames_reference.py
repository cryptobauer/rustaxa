#!/usr/bin/env python3
"""Capture positive-reward partial EVM frame outputs with uninstrumented controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_redelegate_reward_frames_reference.go'
SUPPORT=HERE/'native_validator_info_reference.go'
SEED=HERE/'fixtures/native_redelegate_reward/public.json'
FIXTURES=HERE/'fixtures/native_redelegate_reward_frames'
def digest(data):return hashlib.sha256(data).hexdigest()
def run_reference(revision,control=False):
 with tempfile.TemporaryDirectory(prefix='rustaxa-reward-frames-') as directory:
  tree=Path(directory)
  archive=subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'archive',revision])
  subprocess.run(['tar','-x','-C',directory],input=archive,check=True)
  target=tree/custody.TRACE_TARGET; source=target.read_bytes()
  if digest(source)!=custody.TRACE_SHA256:raise RuntimeError('Account observer target changed')
  for operation,expression in [('add','Add'),('sub','Sub')]:
   needle=('self.set_balance(new(big.Int).'+expression+'(self.Balance, amount))').encode()
   if source.count(needle)!=1:raise RuntimeError('Balance observer shape changed')
   if not control:source=source.replace(needle,needle+b'\n\t\tobserveRewardBalance(self.addr,"'+operation.encode()+b'",amount,self.Balance)',1)
  target.write_bytes(source)
  (target.parent/'reward_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
  command=tree/'cmd/reward_frames';command.mkdir(parents=True)
  source=SUPPORT.read_text()
  if source.count('func main() {')!=1:raise RuntimeError('Support main changed')
  (command/SUPPORT.name).write_text(source.replace('func main() {','func unusedInfoMain() {'))
  (command/SOURCE.name).write_bytes(SOURCE.read_bytes());(command/'seed.json').write_bytes(SEED.read_bytes())
  result=subprocess.run(['go','run','-mod=readonly','./cmd/reward_frames'],cwd=tree,capture_output=True,env={**os.environ,'REWARD_FRAME_CONTROL':'1' if control else '0'})
  if result.returncode:
   sys.stdout.buffer.write(result.stdout);sys.stderr.buffer.write(result.stderr);result.check_returncode()
  lines=result.stdout.splitlines(keepends=True)
  if not lines:raise RuntimeError('Missing output')
  return lines[-1],b''.join(lines[:-1]),result.stderr
OBSERVATIONS={'calls','ordered_reads','ordered_raw_writes','ordered_balance_effects','native_ordered_balance_effects'}
def strip(value):
 if isinstance(value,dict):return {k:strip(v) for k,v in value.items() if k not in OBSERVATIONS}
 if isinstance(value,list):return [strip(v) for v in value]
 return value
def validate(document):
 assert document['schema']==1 and len(document['cases'])==7
 expected={'direct_rewarded_partial','nested_rewarded_partial','static_rewarded_partial','parent_revert_rewarded_partial','two_rewarded_partial_calls_parent_revert','nested_nonpayable','nested_underfunded'}
 assert {c['name'] for c in document['cases']}==expected
 seed=json.loads(SEED.read_text())['cases'][0]
 for c in document['cases']:
  assert c['input']==seed['input'] and c['accounts_before']==seed['reward_committed_accounts']
  assert len(c['calls'])==(2 if c['two_calls'] else 1)
  for field in ['ordered_reads','ordered_raw_writes']:
   assert sum([call[field] for call in c['calls']],[])==c[field]
  assert sum([call['ordered_balance_effects'] for call in c['calls']],[])==c['native_ordered_balance_effects']
  assert not c['refund_before'] and not c['refund_after']
  assert c['log_count_before']==0 and c['cumulative_logs']==c['logs']
  assert all(call['route_staticcall']==c['static'] for call in c['calls'])
  assert len(c['prior_raw'])==52
  assert all(key in c['prior_raw'] for call in c['calls'] for key in call['setup_reads']+call['ordered_reads'])
  for key,value in {w['key']:w['value'] for w in c['ordered_raw_writes']}.items():assert c['after_raw'][key]==value
  call=c['calls'][0];success=call['native_called'] and not call['native_error']
  assert c['transaction_gas_used']=={'direct_rewarded_partial':101912,'nested_rewarded_partial':102678,'static_rewarded_partial':102676,'parent_revert_rewarded_partial':102678,'two_rewarded_partial_calls_parent_revert':183431,'nested_nonpayable':29378,'nested_underfunded':22678}[c['name']]
  if success:
   assert call['ordered_raw_writes']==seed['attempts'][0]['ordered_raw_writes']
   assert call['ordered_balance_effects']==seed['attempts'][0]['ordered_balance_effects']
   assert call['logs']==seed['attempts'][0]['logs']
   assert len(c['ordered_raw_writes'])==12
  else:assert not c['ordered_raw_writes'] and not c['native_ordered_balance_effects'] and not c['logs']
  if c['parent_revert']:
   assert c['parent_error']==c['execution_error']=='execution reverted' and not c['logs']
   for address in c['accounts']:assert c['accounts'][address]['balance']==c['accounts_before'][address]['balance']
  else:
   assert not c['parent_error'] and not c['execution_error']
   if success:assert c['logs']==seed['attempts'][0]['logs']
  if c['two_calls']:
   second=c['calls'][1];assert int(second['input'][-64:],16)==701
   assert second['native_called'] and second['native_error']=='Insufficient delegation'
   assert not second['ordered_raw_writes'] and not second['ordered_balance_effects'] and not second['logs']
  if c['name']=='nested_nonpayable':assert call['native_error']=='Method is not payable' and call['required_gas']==0 and call['supplied_native_gas']==82300
  elif c['name']=='nested_underfunded':assert not call['native_called'] and call['required_gas']==80000 and call['supplied_native_gas']==79999
  else:assert call['required_gas']==80000
 # These expectations come from the first dual-pin measurements retained in run4.
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()}
 controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:
  validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==json.loads(controls[label][0])
 manifest={'schema':1,'references':REVISIONS,'go_version':subprocess.check_output(['go','version'],text=True).strip(),
 'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':digest(SUPPORT.read_bytes()),'seed_sha256':digest(SEED.read_bytes()),
 'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),
 'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),
 'control_execution_identical':True,'frame_context':{'period':2,'committed_seed_period':1,'physical_rows':91,'selected_keys':52,'seed_entries':50,'seed_unique_keys':48,'code_overlay':'existing caller d1 code fields only; actual account nonce/balance unchanged','go_evm_config':'params.TestChainConfig','rust_chain_id':666,'native_config':'exact accepted reward seed configuration.chain_config','transaction_gas':200000,'block_gas':1000000,'gas_price':0,'timestamp':0,'native_gas':80000,'underfunded_native_gas':79999,'two_call_amounts':[300,701],'scope':'leaf EVM.Main frame experiment; no StateTransition lifecycle/reopen/root/production claim'},
 'sha256':{label:[digest(x) for x in data] for label,data in outputs.items()},'control_sha256':{label:[digest(x) for x in data] for label,data in controls.items()}}
 combined={**outputs,**{label+'.control':data for label,data in controls.items()}}
 if args.record:
  FIXTURES.mkdir(parents=True,exist_ok=True)
  (FIXTURES/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
 else:assert json.loads((FIXTURES/'manifest.json').read_text())==manifest
 for label,(data,diagnostics,stderr) in combined.items():
  for suffix,content in [('json',data),('stdout.txt',diagnostics),('stderr.txt',stderr)]:
   path=FIXTURES/(label+'.'+suffix)
   if args.record:path.write_bytes(content)
   else:assert path.read_bytes()==content,str(path)
 print('Both pinned positive-reward frame outputs and controls executed and matched')
if __name__=='__main__':main()
