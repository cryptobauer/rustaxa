#!/usr/bin/env python3
"""Compare delegator_claim probes from actual pinned DryRunner with unchanged C++ search."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from reference import REVISIONS, ROOT
import native_metadata_estimate_reference as support
import native_redelegate_reward_reference as reward
import native_v1_custody_reference as custody

HERE = Path(__file__).resolve().parent
INPUT = HERE / 'fixtures/native_reward_delegator_h1/public.json'
FIXTURES = HERE / 'fixtures/native_reward_delegator_h1_estimate'

SOURCES = [HERE / name for name in (
    'native_simulation_reference.go', 'native_redelegate_simulation_reference.go',
    'native_redelegate_swap_append_simulation_reference.go',
    'native_redelegate_reward_h1_reference.go',
    'native_reward_delegator_h1_estimate_reference.go')]


def run_reference(revision, requests):
    with tempfile.TemporaryDirectory(prefix='rustaxa-delegator_claim-estimate-') as directory:
        archive = subprocess.check_output(['git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', revision])
        subprocess.run(['tar', '-x', '-C', directory], input=archive, check=True)
        target=Path(directory)/custody.TRACE_TARGET
        if hashlib.sha256(target.read_bytes()).hexdigest()!=custody.TRACE_SHA256:raise RuntimeError('Account helper target changed')
        (target.parent/'reward_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
        command = Path(directory) / 'cmd/delegator_claim_existing_current_estimate_reference'
        command.mkdir(parents=True)
        for index, source in enumerate(SOURCES):
            content = source.read_text()
            if index < len(SOURCES) - 1:
                if content.count('func main() {') != 1:
                    raise RuntimeError('Shared exporter main shape changed')
                content = content.replace('func main() {', f'func unusedSharedMain{index}() {{')
            (command / source.name).write_text(content)
        result = subprocess.run(['go', 'run', '-mod=readonly', './cmd/delegator_claim_existing_current_estimate_reference'], cwd=directory, input=json.dumps(requests).encode(), capture_output=True, check=True)
        return result.stdout, result.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    baseline=json.loads(INPUT.read_bytes())
    requests=baseline['cases'].copy()
    assert requests[0]['name']=='delegator_claim'
    requests += [{**requests[0], 'name':'delegator_h1_minimum', 'gas':61464}, {**requests[0], 'name':'delegator_h1_below_minimum', 'gas':61463}]
    captured = {label: run_reference(revision, requests) for label, revision in REVISIONS.items()}
    outputs = {label: data[0] for label, data in captured.items()}
    stderr = {label: data[1] for label, data in captured.items()}
    if outputs['public'] != outputs['local']:
        raise RuntimeError('Pinned delegator_claim estimate probes differ')
    documents = [json.loads(data) for data in outputs.values()]
    for document in documents:
        if (document['schema'] != 1 or document['state_before'] != document['state_after']
                or [case['name'] for case in document['cases']] != [case['name'] for case in requests]):
            raise RuntimeError('Invalid delegator_claim estimate corpus')
    assert documents[0]['state_before']==baseline['state_before']
    first={k:v for k,v in baseline['cases'][0].items() if k not in ('ordered_balance_effects','ordered_raw_writes')}
    assert documents[0]['cases'][0]['probes'][0]==first
    assert len(documents[0]['cases'])==3
    assert documents[0]['cases'][1]['probes'][0]['output']==first['output']
    assert documents[0]['cases'][2]['probes'][0]['output']['execution_error']=='out of gas'
    assert documents[0]['cases'][2]['probes'][0]['output']['gas_used']==21464
    # This compiles the unchanged upstream search body. Its callback checks every
    # requested gas against the Go transcript and requires all probes consumed.
    cpp = support.cpp_reference(documents[0])
    assert cpp['algorithm_sha256']=='7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759'
    assert cpp['cases'][0]=={'name':'delegator_claim','result':63628,'consumed':7}
    assert [p['gas'] for p in documents[0]['cases'][0]['probes']]==[200000,130732,96098,78781,70122,65793,63628]
    assert [len(c['probes']) for c in documents[0]['cases']]==[7,1,1]
    assert cpp['cases'][1]=={'name':'delegator_h1_minimum','result':61464,'consumed':1}
    assert cpp['cases'][2]=={'name':'delegator_h1_below_minimum','error':'out of gas','consumed':1}
    outputs['cpp'] = (json.dumps(cpp, sort_keys=True, indent=2) + '\n').encode()
    manifest = {'schema': 1, 'references': REVISIONS,
                'go_version':subprocess.check_output(['go','version'],text=True).strip(),
                'balance_target_sha256':custody.TRACE_SHA256,'balance_helper_sha256':hashlib.sha256(reward.BALANCE_HELPER).hexdigest(),
                'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in SOURCES},
                'balance_harness_sha256':hashlib.sha256(Path(reward.__file__).read_bytes()).hexdigest(),'guard_harness_sha256':hashlib.sha256(Path(custody.__file__).read_bytes()).hexdigest(),'cpp_source_sha256':hashlib.sha256(support.CPP_SOURCE.read_bytes()).hexdigest(),'dry_runner_sha256':{label:hashlib.sha256(subprocess.check_output(['git','-C',str(ROOT/'submodules/taraxa-evm'),'show',rev+':taraxa/state/state_dry_runner/dry_runner.go'])).hexdigest() for label,rev in REVISIONS.items()},
                'support_harness_sha256': hashlib.sha256(Path(support.__file__).read_bytes()).hexdigest(),
                'input_sha256': hashlib.sha256(INPUT.read_bytes()).hexdigest(),
                'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                'algorithm_sha256': cpp['algorithm_sha256'],
                'stderr_sha256': {label: hashlib.sha256(data).hexdigest() for label, data in stderr.items()},
                'sha256': {label: hashlib.sha256(data).hexdigest() for label, data in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        for label, data in outputs.items():
            (FIXTURES / f'{label}.json').write_bytes(data)
        for label, data in stderr.items():
            (FIXTURES / f'{label}.stderr.txt').write_bytes(data)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    else:
        if json.loads((FIXTURES / 'manifest.json').read_text()) != manifest:
            raise RuntimeError('Signed H1 delegator_claim estimate manifest changed')
        for label, data in outputs.items():
            if data != (FIXTURES / f'{label}.json').read_bytes():
                raise RuntimeError(f'{label}: delegator_claim estimate changed')
        for label, data in stderr.items():
            if data != (FIXTURES / f'{label}.stderr.txt').read_bytes():
                raise RuntimeError(f'{label}: signed H1 estimate stderr changed')
    print('Both pinned reward-only H1 delegator_claim probe corpora match the unchanged C++ search')


if __name__ == '__main__':
    main()
