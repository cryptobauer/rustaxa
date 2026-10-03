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
SOURCE = HERE / 'native_redelegate_new_destination_reference.go'
SUPPORT = [HERE / 'native_simulation_reference.go', HERE / 'native_v1_custody_reference.go']
FIXTURES = HERE / 'fixtures/native_redelegate_new_destination'
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
    expected = {'new_destination_partial_and_repeat', 'new_destination_below_minimum_and_repeat'}
    if document['schema'] != 1 or document['action_gas'] != 80000 or set(cases) != expected:
        raise RuntimeError('Incomplete new-destination observations')
    for row in cases.values():
        if row['total_delegated'] != '2000' or len(row['attempts']) != 2:
            raise RuntimeError('Principal/repeat observation mismatch')
        for attempt in row['attempts']:
            if (attempt['consensus_error'] or attempt['execution_error'] or attempt['output']
                    or len(attempt['logs']) != 1 or not attempt['ordered_raw_writes']
                    or attempt['balances'] != row['balances_before']):
                raise RuntimeError('New-destination success observation mismatch')
    caller, other = '00' * 19 + 'd1', '00' * 19 + 'a1'
    source, destination = '00' * 19 + '31', '00' * 19 + '32'
    for name, amount in [('new_destination_partial_and_repeat', 300), ('new_destination_below_minimum_and_repeat', 50)]:
        row = cases[name]
        before, after = row['facts_before'], row['facts_after']
        if (before['delegations'][caller + '/' + destination] is not None
                or after['delegations'][caller + '/' + destination]['stake'] != str(2 * amount)
                or after['delegations'][caller + '/' + source]['stake'] != str(1000 - 2 * amount)
                or after['delegations'][other + '/' + destination] != before['delegations'][other + '/' + destination]
                or after['memberships'][caller] != [source, destination]
                or after['memberships'][other] != before['memberships'][other]
                or [attempt['caller_nonce'] for attempt in row['attempts']] != ['1', '2']
                or [(len(a['ordered_reads']), len(a['ordered_raw_writes'])) for a in row['attempts']] != [(15, 14), (0, 10)]):
            raise RuntimeError('Committed new delegation/membership or nonce facts mismatch')
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
