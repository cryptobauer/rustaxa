#!/usr/bin/env python3
"""Capture actual pinned redelegation reads/writes in disposable archives."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from reference import REVISIONS, ROOT
import native_v1_custody_reference as custody

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_redelegate_reward_reference.go'
SUPPORT = [HERE / 'native_simulation_reference.go', HERE / 'native_v1_custody_reference.go']
FIXTURES = HERE / 'fixtures/native_redelegate_reward'
READ_TARGET = Path('taraxa/state/contracts/storage/evm_state_storage_adapter.go')
READ_SHA256 = '8aaa74d16aa76a0a0a76463f89635ed3b066b84d5d42e986c8ce13e1f447a691'
DIAGNOSTICS = {}
READ_HELPER = b'''package contract_storage

import "github.com/Taraxa-project/taraxa-evm/common"

var redelegateReadObserver func(common.Address, common.Hash, []byte, bool)

func SetRedelegateReadObserver(observer func(common.Address, common.Hash, []byte, bool)) {
    redelegateReadObserver = observer
}

// Archive-only forwarding observer. One read request, unchanged callbacks and
// copied observation after synchronous return, including absent values.
func (self EVMStateStorage) GetAccountStorage(address *common.Address, key *common.Hash, cb func([]byte)) {
    present := false
    var observed []byte
    self.EVMStateFace.GetAccountStorage(address, key, func(value []byte) {
        present = true
        observed = common.CopyBytes(value)
        cb(value)
    })
    if redelegateReadObserver != nil {
        redelegateReadObserver(*address, *key, observed, present)
    }
}
'''

# Archive-only copies: these helpers read no native/backend row and fill no cache.
SNAPSHOT_HELPERS = {
 'taraxa/state/contracts/storage/current_source_snapshot_observer.go': b'''package contract_storage
import "github.com/Taraxa-project/taraxa-evm/common"
func (self *StorageReaderWrapper) CurrentSourceCacheSnapshot() map[common.Hash][]byte {
 out:=make(map[common.Hash][]byte,len(self.cache)); for key,value:=range self.cache { out[key]=common.CopyBytes(value) }; return out
}
''',
 'taraxa/state/contracts/dpos/precompiled/current_source_snapshot_observer.go': b'''package dpos
import "github.com/Taraxa-project/taraxa-evm/common"
func (self *Contract) CurrentSourceCacheSnapshot() map[common.Hash][]byte { return self.storage.CurrentSourceCacheSnapshot() }
''',
 'taraxa/state/state_transition/current_source_snapshot_observer.go': b'''package state_transition
import "github.com/Taraxa-project/taraxa-evm/common"
func (st *StateTransition) CurrentSourceCacheSnapshot() map[common.Hash][]byte { if st.dpos_contract==nil { panic("missing live DPoS") }; return st.dpos_contract.CurrentSourceCacheSnapshot() }
'''
}


BALANCE_HELPER = b'''package state_evm
import "math/big"
import "github.com/Taraxa-project/taraxa-evm/common"
var rewardBalanceObserver func(common.Address,string,*big.Int,*big.Int)
func SetRewardBalanceObserver(observer func(common.Address,string,*big.Int,*big.Int)) {rewardBalanceObserver=observer}
func observeRewardBalance(address common.Address,operation string,amount,balance *big.Int) {if rewardBalanceObserver!=nil {rewardBalanceObserver(address,operation,new(big.Int).Set(amount),new(big.Int).Set(balance))}}
'''

def snapshot_targets(revision):
    paths = ['taraxa/state/contracts/storage/storage.go', 'taraxa/state/state_transition/state_transition.go']
    candidates = subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'ls-tree','-r','--name-only',revision,'taraxa/state/contracts/dpos/precompiled']).decode().splitlines()
    contracts = []
    for path in candidates:
        if path.endswith('.go'):
            content = subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+path])
            if b'type Contract struct {' in content: contracts.append(path)
    if len(contracts) != 1: raise RuntimeError('Live DPoS instance shape changed')
    paths += contracts
    return {path: hashlib.sha256(subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'show',revision+':'+path])).hexdigest() for path in paths}


def run_reference(revision, instrument=True):
    with tempfile.TemporaryDirectory(prefix='rustaxa-redelegate-observation-') as directory:
        tree = Path(directory)
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        target = tree / custody.TRACE_TARGET
        source = target.read_bytes()
        if hashlib.sha256(source).hexdigest() != custody.TRACE_SHA256 or source.count(custody.TRACE_NEEDLE) != 1:
            raise RuntimeError('Raw-write observer target changed')
        if instrument:
            target.write_bytes(source.replace(custody.TRACE_NEEDLE, custody.TRACE_INSERTION, 1))
        (target.parent / 'native_v1_custody_observer.go').write_bytes(custody.TRACE_HELPER)
        target = tree / READ_TARGET
        if hashlib.sha256(target.read_bytes()).hexdigest() != READ_SHA256:
            raise RuntimeError('Raw-read forwarding target changed')
        helper = READ_HELPER if instrument else READ_HELPER.split(b'// Archive-only forwarding observer.')[0]
        (target.parent / 'redelegate_read_observer.go').write_bytes(helper)
        for name, digest in snapshot_targets(revision).items():
            if hashlib.sha256((tree/name).read_bytes()).hexdigest() != digest: raise RuntimeError("Snapshot target changed")
        for name,content in SNAPSHOT_HELPERS.items():
            (tree/name).write_bytes(content)
        # Observe only actual nonzero account arithmetic after the unchanged mutation.
        account_target = tree / custody.TRACE_TARGET
        account_source = account_target.read_bytes()
        for operation, expression in [('add','Add'),('sub','Sub')]:
            needle = ('self.set_balance(new(big.Int).'+expression+'(self.Balance, amount))').encode()
            if account_source.count(needle)!=1: raise RuntimeError('Balance observer target changed')
            if instrument:
                account_source=account_source.replace(needle,needle+b'\n\t\tobserveRewardBalance(self.addr,"'+operation.encode()+b'",amount,self.Balance)',1)
        account_target.write_bytes(account_source)
        (account_target.parent/'reward_balance_observer.go').write_bytes(BALANCE_HELPER)
        command = tree / 'cmd/redelegate_observation'
        command.mkdir(parents=True)
        for index, support in enumerate(SUPPORT):
            source = support.read_text()
            if source.count('func main() {') != 1:
                raise RuntimeError('Support main shape changed')
            (command / support.name).write_text(source.replace('func main() {', f'func unusedSupport{index}Main() {{'))
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        result=subprocess.run(['go','run','-mod=readonly','./cmd/redelegate_observation'],cwd=tree,capture_output=True)
        DIAGNOSTICS[(revision,instrument)]=result.stderr
        if result.returncode:
            sys.stderr.buffer.write(result.stderr)
            result.check_returncode()
        return result.stdout



def validate(data):
    document=json.loads(data)
    assert document['schema']==1 and document['action_gas']==80000 and len(document['cases'])==1
    row=document['cases'][0]
    assert row['name']=='reward_p1_then_partial_p2' and len(row['attempts'])==1
    a=row['attempts'][0]
    assert not a['consensus_error'] and not a['execution_error'] and not a['output']
    assert row['configuration']['yield_percentage']==20
    assert row['reward_minted']=='800'
    caller,other,native=['00'*19+x for x in ['d1','a1','fe']]
    assert row['reward_accounts_before'][caller]['balance']=='3000'
    assert row['reward_accounts_after'][native]['balance']=='4800'
    assert row['reward_accounts_after']==row['reward_committed_accounts']==a['accounts_before']
    assert a['accounts_after']==row['committed_accounts']
    assert a['accounts_after'][caller]=={'balance':'3396','nonce':'1'}
    assert a['accounts_after'][other]==a['accounts_before'][other]
    assert a['accounts_after'][native]['balance']=='4404'
    assert len(a['logs'])==2 and [int(log['data'],16) for log in a['logs']]==[396,300]
    assert row['source_stake']=='1700' and row['destination_stake']=='2300' and row['total_delegated']=='4000'
    source,destination=['00'*19+x for x in ['31','32']]
    cfg=row['configuration']; dpos=cfg['chain_config']['DPOS']; forks=cfg['hardforks']
    assert cfg['chain_id']==666 and cfg['max_supply']=='8000' and cfg['minimum_deposit']=='100' and cfg['maximum_stake']=='1000000'
    assert cfg['genesis_balances']=={'0x'+caller:5000,'0x'+other:3000}
    assert [v['Address'] for v in cfg['initial_validators']]==['0x'+source,'0x'+destination]
    assert all(v['Owner']=='0x'+other and v['Commission']==100 and v['Delegations']=={'0x'+caller:1000,'0x'+other:1000} for v in cfg['initial_validators'])
    assert dpos['BlocksPerYear']==1 and dpos['MaxBlockAuthorReward']==dpos['DagProposersReward']==0 and dpos['DelegationDelay']==1
    assert forks['FixRedelegateBlockNum']==forks['MagnoliaHf']['BlockNum']==forks['FicusHf']['BlockNum']==forks['CornusHf']['BlockNum']==forks['AspenHf']['BlockNumPartOne']==0
    assert forks['AspenHf']['BlockNumPartTwo']==forks['CactiHf']['BlockNum']==2**64-1
    assert [len(row[k]) for k in ['genesis_rows','reward_committed_rows','committed_rows']]==[82,91,113]
    assert a['gas_used']==101912 and len(a['ordered_raw_writes'])==12 and len(a['ordered_reads'])==14
    assert row['reward_end_block_ordered_raw_writes']==[]
    assert [w['value'] for w in row['reward_ordered_raw_writes']]==['c482031808','0320']
    assert a['ordered_balance_effects']==[
        {'address':native,'operation':'sub','amount':'396','balance':'4404'},
        {'address':caller,'operation':'add','amount':'396','balance':'3396'}]
    timeline=a['ordered_effect_timeline']
    assert [e['kind'] for e in timeline]==['raw','raw','balance','balance']+['raw']*10
    assert [e['row'] for e in timeline if e['kind']=='raw']==a['ordered_raw_writes']
    assert [e['row'] for e in timeline if e['kind']=='balance']==a['ordered_balance_effects']
    assert [w['value'] for w in a['ordered_raw_writes']]==['c28002','c28001','c48202bc02','c583060ae002','c8c68206a464800280','c28008','c28002','c28001','c482051402','c28002','c8c68208fc64800280','c28080']
    for name,value in [('source_current_key','c583060ae002'),('destination_current_key','c28002')]:
        assert a['raw_before'][row[name]]=={'present':False,'value':''}
        assert a['raw_after'][row[name]]=={'present':True,'value':value}
    assert a['facts_before']==row['facts_before'] and a['facts_after']==row['facts_after']
    for owner in [caller,other]:
        assert row['facts_before']['memberships'][owner]==row['facts_after']['memberships'][owner]==[source,destination]
    for validator,stake in [(source,'700'),(destination,'1300')]:
        assert row['facts_after']['delegations'][caller+'/'+validator]=={'stake':stake,'last_updated':2}
        assert row['facts_after']['delegations'][other+'/'+validator]==row['facts_before']['delegations'][other+'/'+validator]=={'stake':'1000','last_updated':0}
    for write in a['ordered_raw_writes']:
        last={w['key']:w['value'] for w in a['ordered_raw_writes']}[write['key']]
        assert row['final_raw'][write['key']]=={'present':bool(last),'value':last}
        assert a['raw_after'][write['key']]=={'present':bool(last),'value':last}
    assert row['genesis_rows'] and row['reward_committed_rows'] and row['committed_rows']
    return document


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    outputs = {label: run_reference(revision) for label, revision in REVISIONS.items()}
    controls = {label: run_reference(revision, instrument=False) for label, revision in REVISIONS.items()}
    for data in outputs.values():
        validate(data)
    for label, data in outputs.items():
        observed = json.loads(data)
        control = json.loads(controls[label])
        for document in (observed, control):
            for row in document['cases']:
                row.pop('reward_ordered_raw_writes')
                row.pop('reward_end_block_ordered_raw_writes')
                row.pop('target_end_block_ordered_raw_writes')
                row.pop('final_raw') # Observed key coverage differs; exact complete rows still compare.
                for attempt in row['attempts']:
                    attempt.pop('ordered_balance_effects')
                    attempt.pop('ordered_effect_timeline')
                    attempt.pop('ordered_reads')
                    attempt.pop('ordered_raw_writes')
        if observed != control:
            raise RuntimeError(f'{label}: observers changed execution')
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegation observations differ')
    manifest = {'pending_authority':'frozen live DPoS cache plus immutable committed rewardP1; no pending trie/root claim', 'baseline_period':1, 'snapshot_target_sha256':{label:snapshot_targets(revision) for label,revision in REVISIONS.items()}, 'snapshot_helper_sha256':{p:hashlib.sha256(v).hexdigest() for p,v in SNAPSHOT_HELPERS.items()}, 'schema': 1, 'references': REVISIONS, 'go_version':subprocess.check_output(['go','version'],text=True).strip(), 'balance_observer_sha256':hashlib.sha256(BALANCE_HELPER).hexdigest(), 'diagnostics_sha256':{label+('.observed' if instrument else '.control'):hashlib.sha256(DIAGNOSTICS[(revision,instrument)]).hexdigest() for label,revision in REVISIONS.items() for instrument in (True,False)},
                'exporter_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                'observer_support_harness_sha256': hashlib.sha256(Path(custody.__file__).read_bytes()).hexdigest(),
                'support_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in SUPPORT},
                'read_target': str(READ_TARGET), 'read_target_sha256': READ_SHA256,
                'read_observer_sha256': hashlib.sha256(READ_HELPER).hexdigest(),
                'write_target': str(custody.TRACE_TARGET), 'write_target_sha256': custody.TRACE_SHA256,
                'write_observer_sha256': hashlib.sha256(custody.TRACE_HELPER).hexdigest(),
                'control_sha256': {label: hashlib.sha256(data).hexdigest() for label, data in controls.items()},
                'control_execution_identical': True,
                'sha256': {label: hashlib.sha256(data).hexdigest() for label, data in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
        for label, data in controls.items():
            (FIXTURES / f'{label}.control.json').write_bytes(data)
        for label,revision in REVISIONS.items():
            for instrument in (True,False):
                (FIXTURES/(label+('.observed' if instrument else '.control')+'.stderr')).write_bytes(DIAGNOSTICS[(revision,instrument)])
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Redelegation observation manifest changed')
        for label, data in outputs.items():
            if (FIXTURES / f'{label}.json').read_bytes() != data:
                raise RuntimeError(f'{label}: observations changed')
        for label, data in controls.items():
            if (FIXTURES / f'{label}.control.json').read_bytes() != data:
                raise RuntimeError(f'{label}: full+existing current-node control changed')
    for label,revision in REVISIONS.items():
        for instrument in (True,False):
            assert (FIXTURES/(label+('.observed' if instrument else '.control')+'.stderr')).read_bytes()==DIAGNOSTICS[(revision,instrument)]
    print('Both pinned positive-reward partial redelegation observations executed and matched')


if __name__ == '__main__':
    main()
