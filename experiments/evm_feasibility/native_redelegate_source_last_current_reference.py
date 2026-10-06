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
SOURCE = HERE / 'native_redelegate_source_last_current_reference.go'
SUPPORT = [HERE / 'native_simulation_reference.go', HERE / 'native_v1_custody_reference.go']
FIXTURES = HERE / 'fixtures/native_redelegate_source_last_current'
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
        raise RuntimeError('Incomplete source-current corpus')
    row = document['cases'][0]
    if row['name'] != 'partial_prefix_then_source_last_current_full_new' or len(row['attempts']) != 3:
        raise RuntimeError('Source-current profile changed')
    prefix, target, failure = row['attempts']
    caller, other = '00'*19+'d1','00'*19+'a1'
    source,destination,third = ['00'*19+x for x in ['31','32','33']]
    for index, attempt in enumerate(row['attempts']):
        if attempt['consensus_error'] or attempt['caller_nonce_before'] != str(index) or attempt['caller_nonce'] != str(index+1) or attempt['balances'] != row['balances_before']:
            raise RuntimeError('Actual admission/account transition differs')
    if (prefix['execution_error'] or target['execution_error'] or prefix['output'] or target['output'] or len(prefix['logs']) != 1 or len(target['logs']) != 1 or not prefix['ordered_raw_writes'] or not target['ordered_raw_writes'] or not prefix['ordered_reads']):
        raise RuntimeError('Prefix/target did not succeed')
    if (failure['execution_error'] != 'Delegation does not exist' or failure['output'] or failure['logs'] or failure['ordered_raw_writes'] or failure['facts_before'] != failure['facts_after']):
        raise RuntimeError('Same-direction missing-source failure differs')
    if prefix['facts_after'] != target['facts_before'] or target['facts_after'] != failure['facts_before'] or prefix['raw_after'] != target['raw_before'] or target['raw_after'] != failure['raw_before']:
        raise RuntimeError('Live prefix authority disconnected')
    before, middle, after = row['facts_before'], prefix['facts_after'], row['facts_after']
    if (before['memberships'][caller] != [third,source] or middle['memberships'][caller] != [third,source] or after['memberships'][caller] != [third,destination] or middle['delegations'][caller+'/'+source]['stake'] != '700' or middle['delegations'][caller+'/'+third]['stake'] != '1300' or middle['delegations'][caller+'/'+destination] is not None or after['delegations'][caller+'/'+source] is not None or after['delegations'][caller+'/'+destination]['stake'] != '700' or after['delegations'][caller+'/'+third] != middle['delegations'][caller+'/'+third] or row['source_stake'] != '1000' or row['destination_stake'] != '1700' or row['total_delegated'] != '5000'):
        raise RuntimeError('Exact source-current principal/membership profile differs')
    if before['memberships'][other] != [third,source,destination] or any(before['delegations'][other+'/'+v] != after['delegations'][other+'/'+v] for v in [source,destination,third]):
        raise RuntimeError('Other delegator changed')
    for attempt in (prefix,target):
        reduced = {write['key']:write['value'] for write in attempt['ordered_raw_writes']}
        for key,value in reduced.items():
            expected = {'present': bool(value), 'value':value}
            if attempt['raw_after'].get(key) != expected:
                raise RuntimeError('Actual last write differs from frozen native cache view')
    for write in target['ordered_raw_writes']:
        # Repeated writes are reduced only for final comparison, never execution.
        last = {w['key']:w['value'] for w in target['ordered_raw_writes']}[write['key']]
        if row['final_raw'].get(write['key']) != {'present':bool(last),'value':last}:
            raise RuntimeError('Final committed row differs from target last write')
    if failure['raw_before'] != failure['raw_after']:
        raise RuntimeError('Normal failure changed frozen raw view')
    if any(target['raw_before'][k] != target['raw_after'][k] for k in row['preserved_target_keys']):
        raise RuntimeError('Target changed source old or retained third physical facts')
    key=row['source_current_key']
    if ([w['value'] for w in target['ordered_raw_writes'] if w['key']==key] != ['c28001','c28002'] or target['raw_before'][key] != {'present':True,'value':'c28002'} or target['raw_after'][key] != {'present':True,'value':'c28002'}):
        raise RuntimeError('Exact source-current earlier-copy authority differs')
    key=row['destination_current_key']
    if target['raw_before'][key] != {'present':False,'value':''} or target['raw_after'][key] != {'present':True,'value':'c28002'}:
        raise RuntimeError('Destination current-node absence/creation differs')
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
                raise RuntimeError(f'{label}: full+new control changed')
    print('Both pinned full+new redelegation observations executed and matched')


if __name__ == '__main__':
    main()
