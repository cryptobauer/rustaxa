#!/usr/bin/env python3
"""Capture finite claim failure frames on canonical P1 with uninstrumented controls."""
import argparse,hashlib,json,os,subprocess,tempfile,sys
from pathlib import Path
from reference import REVISIONS,ROOT
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody
HERE=Path(__file__).resolve().parent
SOURCE=HERE/'native_reward_claim_frame_failures_reference.go'
SUPPORT=[HERE/name for name in ['native_simulation_reference.go','native_redelegate_simulation_reference.go','native_redelegate_swap_append_simulation_reference.go','native_redelegate_reward_h1_reference.go','native_validator_info_reference.go']]
SEED=HERE/'fixtures/native_redelegate_reward_h1/public.json'
FIXTURES=HERE/'fixtures/native_reward_claim_frame_failures'
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
# Exact four-case vectors from the first dual-pin measurement.
MEASURED = {'commission_low': {'accounts': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                              'nonce': '1'},
                                 '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000',
                                                                              'nonce': '0'},
                                 '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000',
                                                                              'nonce': '1'}},
                    'accounts_before': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                                     'nonce': '1'},
                                        '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000',
                                                                                     'nonce': '0'},
                                        '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000',
                                                                                     'nonce': '0'}},
                    'after_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                  '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                  '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                  '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                  '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                  '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                  'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                  'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                  'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                  'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                  'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                  'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                    'calls': [{'caller': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                               'depth': 1,
                               'input': 'd0eebfe20000000000000000000000000000000000000000000000000000000000000031',
                               'logs': [],
                               'native_called': False,
                               'native_error': '',
                               'native_output': '',
                               'ordered_balance_effects': [],
                               'ordered_raw_writes': [],
                               'ordered_reads': [],
                               'required_gas': 20000,
                               'route_staticcall': False,
                               'setup_reads': ['f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393',
                                               'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795',
                                               'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc',
                                               'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873'],
                               'supplied_native_gas': 19999,
                               'value': '0'}],
                    'code': '366000600037600060003660003460fe62004e1ff160005260206000f3',
                    'commission': True,
                    'consensus_error': '',
                    'execution_error': '',
                    'from': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                    'gas': 200000,
                    'gas_price': '0',
                    'input': 'd0eebfe20000000000000000000000000000000000000000000000000000000000000031',
                    'logs': [],
                    'nonce': 0,
                    'ordered_balance_effects': [],
                    'ordered_raw_writes': [],
                    'parent_error': '',
                    'parent_output': '0000000000000000000000000000000000000000000000000000000000000000',
                    'parent_revert': False,
                    'prior_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                  '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                  '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                  '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                  '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                  '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                  'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                  'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                  'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                  'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                  'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                  'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                    'static': False,
                    'to': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                    'transaction_gas_used': 22218,
                    'value': 0},
 'commission_nonpayable': {'accounts': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                                     'nonce': '1'},
                                        '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1001',
                                                                                     'nonce': '0'},
                                        '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '2999',
                                                                                     'nonce': '1'}},
                           'accounts_before': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                                            'nonce': '1'},
                                               '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000',
                                                                                            'nonce': '0'},
                                               '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000',
                                                                                            'nonce': '0'}},
                           'after_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                         '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                         '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                         '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                         '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                         '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                         'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                         'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                         'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                         'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                         'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                         'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                           'calls': [{'caller': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                      'depth': 1,
                                      'input': 'd0eebfe20000000000000000000000000000000000000000000000000000000000000031',
                                      'logs': [],
                                      'native_called': True,
                                      'native_error': 'Method is not payable',
                                      'native_output': '',
                                      'ordered_balance_effects': [],
                                      'ordered_raw_writes': [],
                                      'ordered_reads': [],
                                      'required_gas': 0,
                                      'route_staticcall': False,
                                      'setup_reads': ['f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393',
                                                      'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795',
                                                      'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc',
                                                      'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873'],
                                      'supplied_native_gas': 22300,
                                      'value': '1'}],
                           'code': '366000600037600060003660003460fe62004e20f160005260206000f3',
                           'commission': True,
                           'consensus_error': '',
                           'execution_error': '',
                           'from': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                           'gas': 200000,
                           'gas_price': '0',
                           'input': 'd0eebfe20000000000000000000000000000000000000000000000000000000000000031',
                           'logs': [],
                           'nonce': 0,
                           'ordered_balance_effects': [{'address': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                                                        'amount': '1',
                                                        'balance': '2999',
                                                        'operation': 'sub'},
                                                       {'address': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                                        'amount': '1',
                                                        'balance': '1001',
                                                        'operation': 'add'},
                                                       {'address': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                                        'amount': '1',
                                                        'balance': '1000',
                                                        'operation': 'sub'},
                                                       {'address': '00000000000000000000000000000000000000fe',
                                                        'amount': '1',
                                                        'balance': '4801',
                                                        'operation': 'add'}],
                           'ordered_raw_writes': [],
                           'parent_error': '',
                           'parent_output': '0000000000000000000000000000000000000000000000000000000000000000',
                           'parent_revert': False,
                           'prior_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                         '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                         '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                         '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                         '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                         '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                         'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                         'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                         'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                         'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                         'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                         'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                           'static': False,
                           'to': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                           'transaction_gas_used': 28918,
                           'value': 1},
 'delegator_low': {'accounts': {'00000000000000000000000000000000000000fe': {'balance': '4800', 'nonce': '1'},
                                '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000', 'nonce': '1'},
                                '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000',
                                                                             'nonce': '0'}},
                   'accounts_before': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                                    'nonce': '1'},
                                       '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000',
                                                                                    'nonce': '0'},
                                       '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000',
                                                                                    'nonce': '0'}},
                   'after_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                 '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                 '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                 '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                 '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                 '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                 'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                 'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                 'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                 'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                 'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                 'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                   'calls': [{'caller': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                              'depth': 1,
                              'input': 'ef5cfb8c0000000000000000000000000000000000000000000000000000000000000031',
                              'logs': [],
                              'native_called': False,
                              'native_error': '',
                              'native_output': '',
                              'ordered_balance_effects': [],
                              'ordered_raw_writes': [],
                              'ordered_reads': [],
                              'required_gas': 40000,
                              'route_staticcall': False,
                              'setup_reads': ['f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393',
                                              'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795',
                                              'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc',
                                              'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873'],
                              'supplied_native_gas': 39999,
                              'value': '0'}],
                   'code': '366000600037600060003660003460fe62009c3ff160005260206000f3',
                   'commission': False,
                   'consensus_error': '',
                   'execution_error': '',
                   'from': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                   'gas': 200000,
                   'gas_price': '0',
                   'input': 'ef5cfb8c0000000000000000000000000000000000000000000000000000000000000031',
                   'logs': [],
                   'nonce': 0,
                   'ordered_balance_effects': [],
                   'ordered_raw_writes': [],
                   'parent_error': '',
                   'parent_output': '0000000000000000000000000000000000000000000000000000000000000000',
                   'parent_revert': False,
                   'prior_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                 '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                 '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                 '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                 '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                 '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                 'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                 'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                 'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                 'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                 'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                 'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                   'static': False,
                   'to': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                   'transaction_gas_used': 22218,
                   'value': 0},
 'delegator_nonpayable': {'accounts': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                                    'nonce': '1'},
                                       '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '999',
                                                                                    'nonce': '1'},
                                       '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3001',
                                                                                    'nonce': '0'}},
                          'accounts_before': {'00000000000000000000000000000000000000fe': {'balance': '4800',
                                                                                           'nonce': '1'},
                                              '5d5c99edf529335160ff180fa141dd4967fc00d2': {'balance': '1000',
                                                                                           'nonce': '0'},
                                              '77952ce83ca3cad9f7adcfabeda85bd2f1f52008': {'balance': '3000',
                                                                                           'nonce': '0'}},
                          'after_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                        '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                        '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                        '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                        '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                        '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                        'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                        'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                        'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                        'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                        'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                        'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                          'calls': [{'caller': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                                     'depth': 1,
                                     'input': 'ef5cfb8c0000000000000000000000000000000000000000000000000000000000000031',
                                     'logs': [],
                                     'native_called': True,
                                     'native_error': 'Method is not payable',
                                     'native_output': '',
                                     'ordered_balance_effects': [],
                                     'ordered_raw_writes': [],
                                     'ordered_reads': [],
                                     'required_gas': 0,
                                     'route_staticcall': False,
                                     'setup_reads': ['f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393',
                                                     'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795',
                                                     'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc',
                                                     'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873'],
                                     'supplied_native_gas': 42300,
                                     'value': '1'}],
                          'code': '366000600037600060003660003460fe62009c40f160005260206000f3',
                          'commission': False,
                          'consensus_error': '',
                          'execution_error': '',
                          'from': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                          'gas': 200000,
                          'gas_price': '0',
                          'input': 'ef5cfb8c0000000000000000000000000000000000000000000000000000000000000031',
                          'logs': [],
                          'nonce': 0,
                          'ordered_balance_effects': [{'address': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                                       'amount': '1',
                                                       'balance': '999',
                                                       'operation': 'sub'},
                                                      {'address': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                                                       'amount': '1',
                                                       'balance': '3001',
                                                       'operation': 'add'},
                                                      {'address': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                                                       'amount': '1',
                                                       'balance': '3000',
                                                       'operation': 'sub'},
                                                      {'address': '00000000000000000000000000000000000000fe',
                                                       'amount': '1',
                                                       'balance': '4801',
                                                       'operation': 'add'}],
                          'ordered_raw_writes': [],
                          'parent_error': '',
                          'parent_output': '0000000000000000000000000000000000000000000000000000000000000000',
                          'parent_revert': False,
                          'prior_raw': {'18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c': 'c482031808',
                                        '4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a': 'c8c68207d064808080',
                                        '5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b': '01000000',
                                        '5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23': '5d5c99edf529335160ff180fa141dd4967fc00d2',
                                        '753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3': '',
                                        '9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e': 'c28003',
                                        'b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5': '01000000',
                                        'cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1': 'c48203e880',
                                        'd0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873': '0320',
                                        'dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795': '0fa0',
                                        'ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc': '',
                                        'f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393': '0190'},
                          'static': False,
                          'to': '77952ce83ca3cad9f7adcfabeda85bd2f1f52008',
                          'transaction_gas_used': 28918,
                          'value': 1}}
def validate(document):
 assert document['schema']==1 and document['period']==2 and len(document['cases'])==4
 seed=json.loads(SEED.read_text())
 assert document['state_before']==document['state_after']==seed['state_before']
 assert len(document['state_before']['seed_rows'])==92
 assert [c['name'] for c in document['cases']]==['commission_low','commission_nonpayable','delegator_low','delegator_nonpayable']
 for case in document['cases']:
  assert len(case['calls'])==1 and case['nonce']==0 and case['gas']==200000 and case['gas_price']=='0'
  assert case['value']==(1 if case['name'].endswith('nonpayable') else 0)
  call=case['calls'][0]
  assert call['caller']==case['to'] and call['depth']==1 and call['input']==case['input']
  assert len(case['prior_raw'])==12
  assert {k:v for k,v in case.items() if k!='name'}==MEASURED[case['name']],case['name']
  assert case['after_raw']==case['prior_raw'] and case['ordered_raw_writes']==[]
  assert call['ordered_raw_writes']==call['ordered_reads']==call['logs']==call['ordered_balance_effects']==[]
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
  'control_execution_identical':True,'frame_context':{'period':2,'committed_seed_period':1,'physical_rows':92,'code_overlay':'existing nativecaller code only; outer sender is the other existing account','go_evm_config':'seedRewardH1 config.EVMChainConfig','rust_chain_id':666,'native_config':'exact canonical seedRewardH1 configuration','transaction_gas':200000,'block_gas':1000000,'gas_price':0,'outer_value_cases':[0,1],'timestamp':1700000002,'scope':'disposable BlockState EVM.Main frames; fullcanonicalP1 unchanged; no root/publication/production claim'},
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
 print('Both pinned claim-failure frame outputs and controls executed and matched')
if __name__=='__main__':main()
