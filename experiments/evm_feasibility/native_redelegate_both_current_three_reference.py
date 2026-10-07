#!/usr/bin/env python3
"""Capture actual pinned redelegation reads/writes in disposable archives."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT
import native_v1_custody_reference as custody

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_redelegate_both_current_three_reference.go'
SUPPORT = [HERE / 'native_simulation_reference.go', HERE / 'native_v1_custody_reference.go']
FIXTURES = HERE / 'fixtures/native_redelegate_both_current_three'
READ_TARGET = Path('taraxa/state/contracts/storage/evm_state_storage_adapter.go')
READ_SHA256 = '8aaa74d16aa76a0a0a76463f89635ed3b066b84d5d42e986c8ce13e1f447a691'
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
        command = tree / 'cmd/redelegate_observation'
        command.mkdir(parents=True)
        for index, support in enumerate(SUPPORT):
            source = support.read_text()
            if source.count('func main() {') != 1:
                raise RuntimeError('Support main shape changed')
            (command / support.name).write_text(source.replace('func main() {', f'func unusedSupport{index}Main() {{'))
        (command / SOURCE.name).write_bytes(SOURCE.read_bytes())
        return subprocess.check_output(['go', 'run', '-mod=readonly', './cmd/redelegate_observation'], cwd=tree)


def validate(data):
    document = json.loads(data)
    if document['schema'] != 1 or document['action_gas'] != 80000 or len(document['cases']) != 1:
        raise RuntimeError('Incomplete both-current count3 corpus')
    row = document['cases'][0]
    if row['name'] != 'caller_owner_prefix_then_full_existing_count_three' or len(row['attempts']) != 4:
        raise RuntimeError('New count3 sequence differs')
    first, second, target, failure = row['attempts']
    caller, owner = '00'*19+'aa', '00'*19+'a1'
    source, destination, third = ['00'*19+x for x in ['31','32','33']]
    cfg = row['configuration']; native = cfg['chain_config']
    if (cfg['chain_id'] != 666 or cfg['max_supply'] != '8000' or cfg['minimum_deposit'] != '100'
        or cfg['maximum_stake'] != '1000000' or cfg['yield_percentage'] != 0
        or cfg['genesis_balances'] != {'0x'+caller:5000,'0x'+owner:3000}
        or [v['Address'] for v in cfg['initial_validators']] != ['0x'+source,'0x'+destination]
        or any(v['Owner'] != '0x'+owner or v['Delegations'] != {'0x'+caller:1000,'0x'+owner:1000} for v in cfg['initial_validators'])
        or native['DPOS']['DelegationLockingPeriod'] != 2
        or native['Hardforks']['CornusHf']['DelegationLockingPeriod'] != 3
        or native['Hardforks']['CactiHf']['DelegationLockingPeriod'] != 7):
        raise RuntimeError('Exact real count3 native configuration differs')
    for index, (attempt, sender, nonce, amount) in enumerate([(first,caller,0,300),(second,owner,0,100),(target,caller,1,700),(failure,caller,2,700)]):
        if (attempt['caller'] != sender or attempt['nonce'] != nonce or attempt['caller_nonce_before'] != str(nonce)
            or attempt['caller_nonce'] != str(nonce+1) or attempt['consensus_error']
            or attempt['balances'] != row['balances_before'] or int(attempt['input'][-64:],16) != amount):
            raise RuntimeError('Real per-account request/admission differs')
        if index < 3 and (attempt['execution_error'] or attempt['output'] or len(attempt['logs']) != 1
            or attempt['logs'][0]['data'] != format(amount,'064x') or not attempt['ordered_raw_writes']):
            raise RuntimeError('Real count3 success missing')
        if index and (row['attempts'][index-1]['raw_after'] != attempt['raw_before']
            or row['attempts'][index-1]['facts_after'] != attempt['facts_before']
            or row['attempts'][index-1]['accounts_after'] != attempt['accounts_before']):
            raise RuntimeError('Live two-prefix continuity differs')
        reduced = {w['key']:w['value'] for w in (attempt['ordered_raw_writes'] or [])}
        if any(attempt['raw_after'].get(k) != {'present':bool(v),'value':v} for k,v in reduced.items()):
            raise RuntimeError('Last serialized write differs from frozen live view')
    if (failure['execution_error'] != 'Delegation does not exist' or failure['output'] or failure['logs']
        or failure['ordered_raw_writes'] or failure['raw_before'] != failure['raw_after']
        or failure['facts_before'] != failure['facts_after']):
        raise RuntimeError('Normal missing-source repeat differs')
    for attempt, gas, writes in [(first,101912,12),(second,101848,10),(target,101912,15),(failure,101912,0)]:
        if attempt['gas_used'] != gas or len(attempt['ordered_raw_writes'] or []) != writes:
            raise RuntimeError('Measured gas or ordered write count differs')
    for field in ['source_old_key','destination_old_key']:
        key = row[field]
        if (first['raw_before'][key] != {'present':True,'value':'c28003'}
            or first['raw_after'][key] != {'present':True,'value':'c28001'}
            or second['raw_after'][key] != {'present':False,'value':''}
            or target['raw_before'][key] != {'present':False,'value':''}):
            raise RuntimeError('Old-node deletion or tombstone authority differs')
    for field, final in [('source_current_key','c28003'),('destination_current_key','c28004')]:
        key = row[field]
        if (first['raw_before'][key] != {'present':False,'value':''}
            or first['raw_after'][key] != {'present':True,'value':'c28002'}
            or second['raw_after'][key] != {'present':True,'value':'c28003'}
            or [w['value'] for w in target['ordered_raw_writes'] if w['key'] == key] != ['c28002',final]
            or target['raw_after'][key] != {'present':True,'value':final}):
            raise RuntimeError('Measured count3 loaded-copy authority differs')
    before, after = target['facts_before'], target['facts_after']
    for sender, amounts in [(caller,(700,1300)),(owner,(900,1100))]:
        if before['memberships'][sender] != [source,destination]: raise RuntimeError('Target member order differs')
        for v, amount in zip([source,destination],amounts):
            if before['delegations'][sender+'/'+v] != {'stake':str(amount),'last_updated':1}:
                raise RuntimeError('Exact current prefix pair differs')
    if (after['memberships'][caller] != [destination] or after['memberships'][owner] != [source,destination]
        or after['delegations'][caller+'/'+source] is not None
        or after['delegations'][caller+'/'+destination] != {'stake':'2000','last_updated':1}
        or any(before['delegations'][owner+'/'+v] != after['delegations'][owner+'/'+v] for v in [source,destination,third])
        or row['source_stake'] != '900' or row['destination_stake'] != '3100' or row['total_delegated'] != '4000'):
        raise RuntimeError('Measured count3 principal/owner/membership differs')
    if any(target['raw_before'][k] != target['raw_after'][k] for k in row['preserved_target_keys']):
        raise RuntimeError('Target changed old node/owner/absent third authority')
    last = {w['key']:w['value'] for w in target['ordered_raw_writes']}
    if any(row['final_raw'][k] != {'present':bool(v),'value':v} for k,v in last.items()):
        raise RuntimeError('Committed target bytes differ')
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
                row.pop('final_raw') # Observed key coverage differs; exact complete rows still compare.
                for attempt in row['attempts']:
                    attempt.pop('ordered_reads')
                    attempt.pop('ordered_raw_writes')
        if observed != control:
            raise RuntimeError(f'{label}: observers changed execution')
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegation observations differ')
    manifest = {'pending_authority':'frozen live DPoS cache plus immutable committed genesis0; no pending trie/root claim', 'baseline_period':0, 'snapshot_target_sha256':{label:snapshot_targets(revision) for label,revision in REVISIONS.items()}, 'snapshot_helper_sha256':{p:hashlib.sha256(v).hexdigest() for p,v in SNAPSHOT_HELPERS.items()}, 'schema': 1, 'references': REVISIONS,
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
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Redelegation observation manifest changed')
        for label, data in outputs.items():
            if (FIXTURES / f'{label}.json').read_bytes() != data:
                raise RuntimeError(f'{label}: observations changed')
        for label, data in controls.items():
            if (FIXTURES / f'{label}.control.json').read_bytes() != data:
                raise RuntimeError(f'{label}: both-current count3 control changed')
    print('Both pinned both-current count3 redelegation observations executed and matched')


if __name__ == '__main__':
    main()
