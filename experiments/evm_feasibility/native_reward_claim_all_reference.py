#!/usr/bin/env python3
"""Capture one positive noarg claim-all direct call on canonical P1 with uninstrumented controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_claim_all_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_reward_h1_reference.go','native_validator_info_reference.go']]
SEED=HERE/'fixtures/native_redelegate_reward_h1/public.json'
FIXTURES=HERE/'fixtures/native_reward_claim_all'
def digest(data):return hashlib.sha256(data).hexdigest()
def run_reference(revision,control=False):
 with tempfile.TemporaryDirectory(prefix='rustaxa-reward-frames-') as directory:
  tree=Path(directory)
  archive=subprocess.check_output(['rtk','proxy','git','-C',str(ROOT/'submodules/taraxa-evm'),'archive',revision])
  subprocess.run(['rtk','proxy','tar','-x','-C',directory],input=archive,check=True)
  target=tree/custody.TRACE_TARGET; source=target.read_bytes()
  if digest(source)!=custody.TRACE_SHA256:raise RuntimeError('Account observer target changed')
  for operation,expression in [('add','Add'),('sub','Sub')]:
   needle=('self.set_balance(new(big.Int).'+expression+'(self.Balance, amount))').encode()
   if source.count(needle)!=1:raise RuntimeError('Balance observer shape changed')
   if not control:source=source.replace(needle,needle+b'\n\t\tobserveRewardBalance(self.addr,"'+operation.encode()+b'",amount,self.Balance)',1)
  target.write_bytes(source)
  (target.parent/'reward_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
  command=tree/'cmd/reward_frames';command.mkdir(parents=True)
  for index,p in enumerate(SUPPORT):
   source=p.read_text()
   if source.count('func main() {')!=1:raise RuntimeError('Support main changed')
   (command/p.name).write_text(source.replace('func main() {',f'func unusedFrameSupport{index}Main() {{'))
  (command/SOURCE.name).write_bytes(SOURCE.read_bytes())
  result=subprocess.run(['rtk','proxy','go','run','-mod=readonly','./cmd/reward_frames'],cwd=tree,capture_output=True,env={**os.environ,'REWARD_FRAME_CONTROL':'1' if control else '0'})
  if result.returncode:
   sys.stdout.buffer.write(result.stdout);sys.stderr.buffer.write(result.stderr);result.check_returncode()
  lines=result.stdout.splitlines(keepends=True)
  if not lines:raise RuntimeError('Missing output')
  return lines[-1],b''.join(lines[:-1]),result.stderr
OBSERVATIONS={'calls','ordered_raw_writes','ordered_balance_effects'}
def strip(value):
 if isinstance(value,dict):return {k:strip(v) for k,v in value.items() if k not in OBSERVATIONS}
 if isinstance(value,list):return [strip(v) for v in value]
 return value
# Exact selected vectors from the first successful dual-pin capture.
MEASURED = {'accounts': {'00000000000000000000000000000000000000fe': {'balance': '4404', 'nonce': '1'},
              '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000', 'nonce': '0'},
              '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3396', 'nonce': '1'}},
 'accounts_before': {'00000000000000000000000000000000000000fe': {'balance': '4800', 'nonce': '1'},
                     '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000', 'nonce': '0'},
                     '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000', 'nonce': '0'}},
 'after_raw': {'0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe': 'c28001',
               '0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60': 'c28080',
               '1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7': 'c28002',
               '18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c28008',
               '4c7a777396972eccb784d1842f015e33042408e1eab6b5d2d412c5dfbdd22c05': '0000000000000000000000000000000000000031',
               '4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92': '02000000',
               '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064800280',
               '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
               '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': 'c583060ae002',
               '754536fdc3837580b0c148ccad50cac1c2d84dc9a1f8df2790b6f964f4af3180': '02000000',
               '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28001',
               '9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a': 'c8c68207d064800280',
               'a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9': 'c48203e802',
               'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
               'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e802',
               'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
               'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
               'e8595be39863dce96c5e29d86669ffb30f4e4d19ef14907f331391288d7eaab6': '02000000',
               'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
               'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190',
               'fd2a0d75c38edd1628e6f077003f7949d6405ec53bd36fc560972a296914beb0': '0000000000000000000000000000000000000032'},
 'calls': [{'caller': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
            'depth': 0,
            'input': '0b83a727',
            'logs': [{'address': '00000000000000000000000000000000000000fe',
                      'data': '000000000000000000000000000000000000000000000000000000000000018c',
                      'topics': ['9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7',
                                 '00000000000000000000000077952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                                 '0000000000000000000000000000000000000000000000000000000000000031']}],
            'native_called': True,
            'native_error': '',
            'native_output': '',
            'ordered_balance_effects': [{'address': '00000000000000000000000000000000000000fe',
                                         'amount': '396',
                                         'balance': '4404',
                                         'operation': 'sub'},
                                        {'address': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                                         'amount': '396',
                                         'balance': '3396',
                                         'operation': 'add'}],
            'ordered_raw_writes': [{'address': '00000000000000000000000000000000000000fe',
                                    'key': '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e',
                                    'value': 'c28002'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a',
                                    'value': 'c8c68207d064800280'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c',
                                    'value': 'c28008'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e',
                                    'value': 'c28001'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': 'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1',
                                    'value': 'c48203e802'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3',
                                    'value': 'c583060ae002'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe',
                                    'value': 'c28002'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a',
                                    'value': 'c8c68207d064800280'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60',
                                    'value': 'c28080'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe',
                                    'value': 'c28001'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': 'a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9',
                                    'value': 'c48203e802'},
                                   {'address': '00000000000000000000000000000000000000fe',
                                    'key': '1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7',
                                    'value': 'c28002'}],
            'ordered_reads': ['4c7a777396972eccb784d1842f015e33042408e1eab6b5d2d412c5dfbdd22c05',
                              'fd2a0d75c38edd1628e6f077003f7949d6405ec53bd36fc560972a296914beb0',
                              'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1',
                              '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3',
                              '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a',
                              '18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c',
                              '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e',
                              'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5',
                              '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b',
                              'a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9',
                              '1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7',
                              '9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a',
                              '0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60',
                              '0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe',
                              'e8595be39863dce96c5e29d86669ffb30f4e4d19ef14907f331391288d7eaab6',
                              '754536fdc3837580b0c148ccad50cac1c2d84dc9a1f8df2790b6f964f4af3180'],
            'required_gas': 90000,
            'route_staticcall': False,
            'setup_reads': ['f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393',
                            'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795',
                            'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc',
                            'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873',
                            '4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92'],
            'supplied_native_gas': 178728,
            'value': '0'}],
 'code': '',
 'commission': False,
 'consensus_error': '',
 'execution_error': '',
 'from': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
 'gas': 200000,
 'gas_price': '0',
 'index_before': {'count_key': '4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92',
                  'count_value': '02000000',
                  'items': [{'key': '4c7a777396972eccb784d1842f015e33042408e1eab6b5d2d412c5dfbdd22c05',
                             'position': 1,
                             'value': '0000000000000000000000000000000000000031'},
                            {'key': 'fd2a0d75c38edd1628e6f077003f7949d6405ec53bd36fc560972a296914beb0',
                             'position': 2,
                             'value': '0000000000000000000000000000000000000032'}]},
 'input': '0b83a727',
 'logs': [{'address': '00000000000000000000000000000000000000fe',
           'data': '000000000000000000000000000000000000000000000000000000000000018c',
           'topics': ['9310ccfcb8de723f578a9e4282ea9f521f05ae40dc08f3068dfad528a65ee3c7',
                      '00000000000000000000000077952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                      '0000000000000000000000000000000000000000000000000000000000000031']}],
 'name': 'claim_all_positive',
 'nonce': 0,
 'ordered_balance_effects': [{'address': '00000000000000000000000000000000000000fe',
                              'amount': '396',
                              'balance': '4404',
                              'operation': 'sub'},
                             {'address': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                              'amount': '396',
                              'balance': '3396',
                              'operation': 'add'}],
 'ordered_raw_writes': [{'address': '00000000000000000000000000000000000000fe',
                         'key': '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e',
                         'value': 'c28002'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a',
                         'value': 'c8c68207d064800280'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c',
                         'value': 'c28008'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e',
                         'value': 'c28001'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': 'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1',
                         'value': 'c48203e802'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3',
                         'value': 'c583060ae002'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe',
                         'value': 'c28002'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a',
                         'value': 'c8c68207d064800280'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60',
                         'value': 'c28080'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe',
                         'value': 'c28001'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': 'a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9',
                         'value': 'c48203e802'},
                        {'address': '00000000000000000000000000000000000000fe',
                         'key': '1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7',
                         'value': 'c28002'}],
 'parent_error': '',
 'parent_output': '',
 'parent_revert': False,
 'prior_raw': {'0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe': 'c28003',
               '0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60': 'c28080',
               '1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7': '',
               '18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
               '4c7a777396972eccb784d1842f015e33042408e1eab6b5d2d412c5dfbdd22c05': '0000000000000000000000000000000000000031',
               '4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92': '02000000',
               '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
               '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
               '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
               '754536fdc3837580b0c148ccad50cac1c2d84dc9a1f8df2790b6f964f4af3180': '02000000',
               '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
               '9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a': 'c8c68207d064808080',
               'a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9': 'c48203e880',
               'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
               'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
               'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
               'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
               'e8595be39863dce96c5e29d86669ffb30f4e4d19ef14907f331391288d7eaab6': '02000000',
               'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
               'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190',
               'fd2a0d75c38edd1628e6f077003f7949d6405ec53bd36fc560972a296914beb0': '0000000000000000000000000000000000000032'},
 'static': False,
 'to': '00000000000000000000000000000000000000fe',
 'transaction_gas_used': 111272,
 'value': 0}
def validate(document):
 assert document['schema']==1 and document['period']==2 and len(document['cases'])==1
 seed=json.loads(SEED.read_text())
 assert document['state_before']==document['state_after']==seed['state_before']
 assert len(document['state_before']['seed_rows'])==92
 case=document['cases'][0]
 assert case['name']=='claim_all_positive' and case['from']==seed['caller'] and case['to']=='0'*38+'fe'
 assert case['input']=='0b83a727' and case['code']=='' and case['nonce']==0 and case['value']==0
 assert len(case['calls'])==1 and case['calls'][0]['depth']==0
 assert case==MEASURED
 assert case['calls'][0]['required_gas']==90000 and case['transaction_gas_used']==111272
 assert len(case['ordered_raw_writes'])==12 and len(case['logs'])==1 and len(case['prior_raw'])==21
 for key,value in {w['key']:w['value'] for w in case['ordered_raw_writes']}.items():assert case['after_raw'][key]==value
 assert case['index_before']['count_value']=='02000000'
 assert [x['value'] for x in case['index_before']['items']]==['0'*38+'31','0'*38+'32']
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--record',action='store_true');args=parser.parse_args()
 outputs={label:run_reference(revision) for label,revision in REVISIONS.items()}
 controls={label:run_reference(revision,True) for label,revision in REVISIONS.items()}
 assert outputs['public']==outputs['local'] and controls['public']==controls['local']
 for label in REVISIONS:
  validate(json.loads(outputs[label][0]));assert strip(json.loads(outputs[label][0]))==json.loads(controls[label][0])
 manifest={'schema':1,'references':REVISIONS,'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip(),
 'exporter_sha256':digest(SOURCE.read_bytes()),'harness_sha256':digest(Path(__file__).read_bytes()),'support_sha256':{p.name:digest(p.read_bytes()) for p in SUPPORT},'seed_sha256':digest(SEED.read_bytes()),
 'balance_target':str(custody.TRACE_TARGET),'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':digest(reward.BALANCE_HELPER),
 'balance_helper_source_sha256':digest(Path(reward.__file__).read_bytes()),'guard_source_sha256':digest(Path(custody.__file__).read_bytes()),
  'control_execution_identical':True,'frame_context':{'period':2,'committed_seed_period':1,'physical_rows':92,'code_overlay':'none; direct CALL to native contract','go_evm_config':'seedRewardH1 config.EVMChainConfig','rust_chain_id':666,'native_config':'exact canonical seedRewardH1 configuration','transaction_gas':200000,'block_gas':1000000,'gas_price':0,'timestamp':1700000002,'scope':'disposable BlockState EVM.Main frames; fullcanonicalP1 unchanged; no root/publication/production claim'},
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
 print('Both pinned claim-all outputs and controls executed and matched')
if __name__=='__main__':main()
