#!/usr/bin/env python3
"""Record/reproduce actual dual-pin retained commission claims and controls."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT
import native_v1_custody_reference as custody
import native_redelegate_reward_reference as reward

HERE = Path(__file__).resolve().parent
GO = HERE / 'native_retained_commission_reference.go'
FIXTURES = HERE / 'fixtures/native_retained_commission'
SUPPORT = [HERE / name for name in (
    'native_simulation_reference.go', 'native_v1_custody_reference.go',
    'native_cancel_reward_reference.go')]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(revision, control, keys):
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    with tempfile.TemporaryDirectory(prefix='oracle-', dir=artifact) as directory:
        tree = Path(directory)
        archive = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['rtk', 'proxy', 'tar', '-x', '-C', directory], input=archive, check=True)
        target = tree / custody.TRACE_TARGET
        source = target.read_bytes()
        if digest(source) != custody.TRACE_SHA256 or source.count(custody.TRACE_NEEDLE) != 1:
            raise RuntimeError('Pinned observer target changed')
        if not control:
            source = source.replace(custody.TRACE_NEEDLE, custody.TRACE_INSERTION, 1)
        for op, expression in [('add', 'Add'), ('sub', 'Sub')]:
            needle = ('self.set_balance(new(big.Int).' + expression + '(self.Balance, amount))').encode()
            if source.count(needle) != 1:
                raise RuntimeError('Pinned balance observer shape changed')
            if not control:
                source = source.replace(needle, needle + b'\n\t\tobserveRewardBalance(self.addr,"' + op.encode() + b'",amount,self.Balance)', 1)
        target.write_bytes(source)
        (target.parent / 'retained_raw_observer.go').write_bytes(custody.TRACE_HELPER)
        (target.parent / 'retained_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
        command = tree / 'cmd/retained_commission'
        command.mkdir(parents=True)
        for index, support in enumerate(SUPPORT):
            text = support.read_text()
            if text.count('func main() {') != 1:
                raise RuntimeError('Support main changed')
            (command / support.name).write_text(text.replace('func main() {', f'func unusedSupport{index}Main() {{', 1))
        (command / GO.name).write_bytes(GO.read_bytes())
        result = subprocess.run(['rtk', 'proxy', 'go', 'run', '-mod=readonly', './cmd/retained_commission'], cwd=tree, capture_output=True,
                                env={**os.environ, 'RETAINED_RAW_KEYS': json.dumps(keys), 'RETAINED_CONTROL': '1' if control else '0'})
        if result.returncode:
            os.write(1, result.stdout)
            os.write(2, result.stderr)
            result.check_returncode()
        lines = result.stdout.splitlines(keepends=True)
        if not lines:
            raise RuntimeError('Missing oracle output')
        return lines[-1], b''.join(lines[:-1]), result.stderr


def strip(value):
    if isinstance(value, dict):
        return {k: strip(v) for k, v in value.items() if k not in ('ordered_raw_writes', 'ordered_balance_effects', 'native_observation')}
    if isinstance(value, list):
        return [strip(v) for v in value]
    return value


def validate(d):
    assert d['schema'] == 1
    assert [c['name'] for c in d['cases']] == ['wrong_owner', 'positive', 'zero_repeat']
    assert [c['name'] for c in d['simulation']] == ['positive', 'wrong_owner']
    assert len(d['provenance']) == 3
    forks = d['configuration']['Hardforks']
    assert forks['MagnoliaHf']['BlockNum'] == forks['PhalaenopsisHfBlockNum'] == 0
    assert forks['CornusHf']['DelegationLockingPeriod'] == 3
    assert forks['CactiHf']['BlockNum'] == 2**64 - 1
    for name in ('state_before', 'state_after'):
        s = d[name]
        assert s['stake'] == '0' and s['pending_count'] == 1 and s['seed_rows'] and s['native_raw']
        assert s['queue_count'] == 1 and s['pending'] == {'Amount': 1000, 'Block': 5}
    assert d['state_before']['period'] == 2 and d['state_after']['period'] == 3
    assert d['state_before']['commission'] == '2' and d['state_after']['commission'] == '0'
    for c in d['cases']:
        assert not c['consensus_error'] and c['gas_used'] == 41464 and c['output'] == ''
        if c['name'] == 'wrong_owner':
            assert c['execution_error'] == 'This account is not owner of specified validator'
            assert not c['logs'] and not c['ordered_raw_writes'] and not c['ordered_balance_effects']
        else:
            assert not c['execution_error'] and len(c['logs']) == 1 and len(c['ordered_raw_writes']) == 1
            assert int(c['logs'][0]['data'], 16) == (2 if c['name'] == 'positive' else 0)
    frame = d['frame']
    assert frame['parent_error'] == 'execution reverted' and frame['gas_used'] == 42218
    assert frame['native_observation']['error'] == ''
    assert frame['native_observation']['caller'] == d['owner']
    assert frame['native_observation']['logs'] == d['cases'][1]['logs']
    assert frame['ordered_raw_writes'] == d['cases'][1]['ordered_raw_writes']
    assert frame['ordered_balance_effects'] == d['cases'][1]['ordered_balance_effects']
    assert not frame['logs']
    assert frame['raw_before'] == d['state_before']['native_raw']
    after = dict(frame['raw_before'])
    for write in frame['ordered_raw_writes']:
        after[write['key']] = write['value']
    assert frame['raw_after'] == after == d['state_after']['native_raw']
    wide = d['wide_effects']
    contract = '00' * 19 + 'fe'
    assert wide['parent_error'] == '' and wide['native_observation']['error'] == ''
    assert wide['accounts_before'][contract]['balance'] == str(2**600)
    assert wide['accounts_after'][contract]['balance'] == str(2**600 - 2)
    assert wide['accounts_before'][d['owner']]['balance'] == '-17'
    assert wide['accounts_after'][d['owner']]['balance'] == '-15'
    assert wide['logs'] == d['cases'][1]['logs']
    assert wide['raw_before'] == frame['raw_before'] and wide['raw_after'] == frame['raw_after']
    for account, value in frame['accounts_before'].items():
        expected = dict(value)
        if account == d['delegator']:
            expected['nonce'] = str(int(expected['nonce']) + 1)
        assert frame['accounts_after'][account] == expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    if set(REVISIONS) != {'public', 'local'}:
        raise RuntimeError('Both reference pins required')
    assert REVISIONS == {'public': '6c7e5338b22d5e596cc2365a88d1f94840e1ee1b', 'local': 'bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418'}
    outputs = {label: run(pin, False, []) for label, pin in REVISIONS.items()}
    assert outputs['public'] == outputs['local']
    controls = {}
    for label, pin in REVISIONS.items():
        d = json.loads(outputs[label][0])
        validate(d)
        controls[label] = run(pin, True, d['raw_key_inventory'])
        assert strip(d) == strip(json.loads(controls[label][0]))
    assert controls['public'] == controls['local']
    combined = {**outputs, **{k + '.control': v for k, v in controls.items()}}
    manifest = {
        'schema': 1, 'references': REVISIONS,
        'exporter_sha256': digest(GO.read_bytes()), 'harness_sha256': digest(Path(__file__).read_bytes()),
        'support_sha256': {p.name: digest(p.read_bytes()) for p in SUPPORT + [Path(custody.__file__), Path(reward.__file__), HERE / 'reference.py']},
        'raw_observer_sha256': digest(custody.TRACE_HELPER), 'balance_observer_sha256': digest(reward.BALANCE_HELPER),
        'observer_target_sha256': custody.TRACE_SHA256,
        'go_version': subprocess.check_output(['rtk', 'proxy', 'go', 'version'], text=True).strip(),
        'hook_free_controls_match': True,
        'sha256': {k: [digest(b) for b in v] for k, v in combined.items()},
    }
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    else:
        assert json.loads((FIXTURES / 'manifest.json').read_text()) == manifest
    for label, streams in combined.items():
        for suffix, data in zip(('json', 'stdout.txt', 'stderr.txt'), streams):
            path = FIXTURES / (label + '.' + suffix)
            if args.record:
                path.write_bytes(data)
            else:
                assert path.read_bytes() == data, str(path)
    print('2 pins, 2 hook-free controls, 3 committed claims and 2 repeated historical simulations matched')


if __name__ == '__main__':
    main()
