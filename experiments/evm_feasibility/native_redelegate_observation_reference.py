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
SOURCE = HERE / 'native_redelegate_observation_reference.go'
SUPPORT = [HERE / 'native_simulation_reference.go', HERE / 'native_v1_custody_reference.go']
FIXTURES = HERE / 'fixtures/native_redelegate_observation'
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
    cases = {row['name']: row for row in document['cases']}
    expected = {'partial_and_repeat', 'destination_cap_before_insufficient_source', 'missing_source',
                'missing_destination', 'missing_source_delegation', 'insufficient_source',
                'remainder_below_minimum', 'same_validator', 'zero_before_aspen_two',
                'full_source', 'insufficient_native_gas', 'nonpayable'}
    if document['schema'] != 1 or document['action_gas'] != 80000 or set(cases) != expected:
        raise RuntimeError('Incomplete redelegation observations')
    for name, row in cases.items():
        if row['total_delegated'] != '2000':
            raise RuntimeError(f'{name}: principal changed')
        for attempt in row['attempts']:
            if attempt['consensus_error'] or attempt['output']:
                raise RuntimeError(f'{name}: unexpected consensus/output')
            if attempt['execution_error'] and (attempt['logs'] or attempt['ordered_raw_writes']):
                raise RuntimeError(f'{name}: failure has effects')
    success = cases['partial_and_repeat']
    if success['source_stake'] != '400' or success['destination_stake'] != '1600' or len(success['attempts']) != 2:
        raise RuntimeError('Repeated partial stake result changed')
    for attempt in success['attempts']:
        if attempt['execution_error'] or len(attempt['logs']) != 1 or not attempt['ordered_raw_writes']:
            raise RuntimeError('Partial success observation incomplete')
    if [(len(a['ordered_reads']), len(a['ordered_raw_writes'])) for a in success['attempts']] != [(14, 12), (0, 10)]:
        raise RuntimeError('First-call/repeated-call storage observation changed')
    failures = {
        'destination_cap_before_insufficient_source': ("Validator's max stake exceeded", 4),
        'missing_source': ('Validator does not exist', 1),
        'missing_destination': ('Validator does not exist', 3),
        'missing_source_delegation': ('Delegation does not exist', 5),
        'insufficient_source': ('Insufficient delegation', 5),
        'remainder_below_minimum': ('Insufficient delegation', 5),
        'same_validator': ('From and to validators are the same', 0),
        'insufficient_native_gas': ('out of gas', 0),
        'nonpayable': ('Method is not payable', 0),
    }
    for name, (error, reads) in failures.items():
        attempt = cases[name]['attempts'][0]
        if attempt['execution_error'] != error or len(attempt['ordered_reads']) != reads:
            raise RuntimeError(f'{name}: failure precedence/read prefix changed')
        if cases[name]['source_stake'] != '1000' or cases[name]['destination_stake'] != '1000':
            raise RuntimeError(f'{name}: failed operation changed stake')
    for name, stakes in {'zero_before_aspen_two': ('1000', '1000'), 'full_source': ('0', '2000')}.items():
        row = cases[name]
        if row['attempts'][0]['execution_error'] or (row['source_stake'], row['destination_stake']) != stakes:
            raise RuntimeError(f'{name}: excluded success behavior changed')
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
                for attempt in row['attempts']:
                    attempt.pop('ordered_reads')
                    attempt.pop('ordered_raw_writes')
        if observed != control:
            raise RuntimeError(f'{label}: observers changed execution')
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned redelegation observations differ')
    manifest = {'schema': 1, 'references': REVISIONS,
                'exporter_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
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
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Redelegation observation manifest changed')
        for label, data in outputs.items():
            if (FIXTURES / f'{label}.json').read_bytes() != data:
                raise RuntimeError(f'{label}: observations changed')
    print('Both pinned redelegation observations executed and matched')


if __name__ == '__main__':
    main()
