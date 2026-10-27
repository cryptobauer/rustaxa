#!/usr/bin/env python3
"""Measure retained P2 DryRunner gas boundaries on both pins and check C++ search."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT
import native_metadata_estimate_reference as cpp_support
import native_retained_commission_reference as retained
import native_v1_custody_reference as custody
import native_redelegate_reward_reference as reward

HERE = Path(__file__).resolve().parent
GO = HERE / 'native_retained_commission_estimate_reference.go'
INPUT = retained.FIXTURES / 'public.json'
FIXTURES = HERE / 'fixtures/native_retained_commission_estimate'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def seed_source():
    # Reuse the accepted producer's exact genesis/reward/undelegate sequence.
    # Keep its snapshot helpers; omit all later P3 claims and frame experiments.
    source = retained.GO.read_text()
    prefix, main = source.split('func main() {', 1)
    setup = main.split('\tbefore := retainedSnapshot(seed)', 1)[0]
    start = setup.index('\tstate_evm.SetNativeV1CustodyRawWriteObserver(')
    end = setup.index('\tcfg := cancelRewardConfig()', start)
    setup = (setup[:start] + '\tstate_evm.SetNativeV1CustodyRawWriteObserver(nil)\n'
             + '\tstate_evm.SetRewardBalanceObserver(nil)\n' + setup[end:])
    return (prefix.replace('\n\t"reflect"', '').replace('\n\t"sort"', '').replace('\n\t"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"', '') + 'func estimateRetainedSeed() nativeSimulationSeed {'
            + setup + '\t_ = provenance\n\treturn seed\n}\n').encode()


def run_reference(revision, baseline):
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    with tempfile.TemporaryDirectory(prefix='retained-estimate-', dir=artifact) as directory:
        tree = Path(directory)
        archive = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['rtk', 'proxy', 'tar', '-x', '-C', directory], input=archive, check=True)
        target = tree / custody.TRACE_TARGET
        if digest(target.read_bytes()) != custody.TRACE_SHA256:
            raise RuntimeError('Observer stub target changed')
        # Definitions only: account.go is unchanged, and both hooks are nil.
        (target.parent / 'retained_raw_observer.go').write_bytes(custody.TRACE_HELPER)
        (target.parent / 'retained_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
        command = tree / 'cmd/retained_estimate'
        command.mkdir(parents=True)
        for index, support in enumerate(retained.SUPPORT):
            content = support.read_text()
            if content.count('func main() {') != 1:
                raise RuntimeError('Support main changed')
            (command / support.name).write_text(content.replace('func main() {', f'func unusedSupport{index}Main() {{', 1))
        (command / retained.GO.name).write_bytes(seed_source())
        (command / GO.name).write_bytes(GO.read_bytes())
        result = subprocess.run(['rtk', 'proxy', 'go', 'run', '-mod=readonly', './cmd/retained_estimate'], cwd=tree,
                                input=json.dumps(baseline).encode(), capture_output=True,
                                env={**os.environ, 'RETAINED_RAW_KEYS': json.dumps(baseline['raw_key_inventory'])})
        if result.returncode:
            os.write(1, result.stdout)
            os.write(2, result.stderr)
            result.check_returncode()
        return result.stdout, result.stderr


def validate(document, baseline):
    assert document['schema'] == 1 and document['repeat_count'] == 2
    assert document['state_before'] == document['state_after'] == baseline['state_before']
    assert document['configuration'] == baseline['configuration']
    cases = document['cases']
    assert [c['name'] for c in cases] == ['positive', 'minimum', 'below_minimum', 'wrong_owner']
    assert [c['cap'] for c in cases] == [200000, document['minimum'], document['minimum'] - 1, 200000]
    # Verify the actual binary boundary measurement and both adjacent outcomes.
    lo, hi = 0, 200000
    boundary = document['boundary_probes']
    assert boundary[0] == cases[0]['probes'][0]
    for p in boundary[1:]:
        assert p['gas'] == lo + (hi - lo) // 2
        if not p['output']['execution_error'] and not p['output']['consensus_error']:
            hi = p['gas']
        else:
            lo = p['gas']
    assert hi - lo == 1 and hi == document['minimum']
    assert cases[1]['probes'][0]['output'] == cases[0]['probes'][0]['output'] == baseline['simulation'][0]['output']
    assert cases[2]['probes'][0]['output']['execution_error'] == 'out of gas'
    assert cases[3]['probes'][0] == baseline['simulation'][1]
    for c in cases:
        assert c['probes']
        for p in c['probes']:
            request = baseline['simulation'][int(c['name'] == 'wrong_owner')]
            for field in ('to', 'input', 'supplied_nonce', 'gas_price', 'value'):
                assert p[field] == request[field]
            if not p['output']['execution_error'] and not p['output']['consensus_error']:
                assert p['output'] == baseline['simulation'][0]['output']
                assert int(p['output']['logs'][0]['data'], 16) == 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    assert REVISIONS == {'public': '6c7e5338b22d5e596cc2365a88d1f94840e1ee1b', 'local': 'bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418'}
    baseline = json.loads(INPUT.read_bytes())
    captured = {label: run_reference(pin, baseline) for label, pin in REVISIONS.items()}
    assert captured['public'] == captured['local']
    document = json.loads(captured['public'][0])
    validate(document, baseline)
    # Put even the unchanged C++ wrapper's disposable files under this run.
    tempfile.tempdir = os.environ['RUN_ARTIFACT_DIR']
    cpp = cpp_support.cpp_reference(document)
    assert cpp['algorithm_sha256'] == '7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759'
    assert all(c['consumed'] == len(g['probes']) for c, g in zip(cpp['cases'], document['cases'], strict=True))
    outputs = {label + '.' + suffix: data for label, streams in captured.items()
               for suffix, data in zip(('json', 'stderr.txt'), streams, strict=True)}
    outputs['cpp.json'] = (json.dumps(cpp, sort_keys=True, indent=2) + '\n').encode()
    sources = retained.SUPPORT + [retained.GO, GO, Path(__file__), Path(retained.__file__),
                                 Path(custody.__file__), Path(reward.__file__), Path(cpp_support.__file__), HERE / 'reference.py']
    manifest = {'schema': 1, 'references': REVISIONS,
                'source_sha256': {p.name: digest(p.read_bytes()) for p in sources},
                'input_sha256': digest(INPUT.read_bytes()), 'generated_seed_sha256': digest(seed_source()),
                'cpp_source_sha256': digest(cpp_support.CPP_SOURCE.read_bytes()),
                'algorithm_sha256': cpp['algorithm_sha256'],
                'stub_target_sha256': custody.TRACE_SHA256,
                'stub_sha256': [digest(custody.TRACE_HELPER), digest(reward.BALANCE_HELPER)],
                'observers': 'nil; target account.go unchanged; no inserted calls',
                'dry_runner_sha256': {label: digest(subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'show', pin + ':taraxa/state/state_dry_runner/dry_runner.go'])) for label, pin in REVISIONS.items()},
                'go_version': subprocess.check_output(['rtk', 'proxy', 'go', 'version'], text=True).strip(),
                'sha256': {name: digest(data) for name, data in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    else:
        assert json.loads((FIXTURES / 'manifest.json').read_text()) == manifest
    for name, data in outputs.items():
        if args.record:
            (FIXTURES / name).write_bytes(data)
        else:
            assert (FIXTURES / name).read_bytes() == data, name
    print('2 pins; 4 cases; minimum', document['minimum'], '; boundary probes', len(document['boundary_probes']),
          '; search probes', [len(c['probes']) for c in document['cases']], '; every probe repeated fresh twice; C++ fully consumed')


if __name__ == '__main__':
    main()
