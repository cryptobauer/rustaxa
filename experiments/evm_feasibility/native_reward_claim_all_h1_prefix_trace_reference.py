#!/usr/bin/env python3
"""Capture literal H1/P0 ordinary-prefix trace before encoding outcome checks."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import native_reward_claim_all_underfunded_trace_reference as producer

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'native_reward_claim_all_h1_prefix_trace_reference.go'
FIXTURES = HERE / 'fixtures/native_reward_claim_all_h1_prefix_trace'

def digest(data):
    return hashlib.sha256(data).hexdigest()

def validate(document, control=False):
    # Encoded after first actual public/local observed/control captures.
    old = json.loads((HERE / 'fixtures/native_reward_claim_all_h1_trace/public.json').read_text())
    assert document['p0_capture_count'] == 1
    assert document['repeat_identical'] and document['source_unchanged_after_each']
    assert document['p0_source_before'] == document['p0_source_after'] == old['p0_source_before']
    assert document['p1_before'] == document['p1_after'] == old['p1_before']
    assert len(document['cases']) == 1
    case, base = document['cases'][0], old['cases'][0]
    caller = document['caller']
    assert caller == old['caller'] and document['owner'] == old['owner']
    prefix = {**base['targets'][0], 'to': caller, 'gas':21000, 'input':''}
    target = {**base['targets'][0], 'nonce':str(2**512+1)}
    assert case['name'] == 'claim_all_H1_P0_ordinary_prefix'
    assert case['prefix'] == [prefix] and case['targets'] == [target]
    assert case['prefix_results'] == [{
        'after_raw':base['prior_raw'], 'caller':caller, 'consensus_error':'',
        'execution_error':'', 'gas':21000, 'logs':0, 'native_balance':'4000',
        'outer_error':'', 'quote_count':0, 'return_value':'', 'run_count':0,
        'sender_balance':'3000', 'sender_nonce':str(2**512+1)}]
    for key in ['diagnostics','execution_period','reader_periods','prior_raw','result','quotes','runs',
                'ordered_account_scalar_reads','ordered_balance_effects']:
        assert case[key] == base[key], key
    expected = json.loads(json.dumps(base['post']))
    expected['supplied_nonce'] = str(2**512+1)
    expected['accounts'][caller]['nonce'] = str(2**512+2)
    assert case['post'] == expected
    if control:
        assert case['ordered_reads'] == case['ordered_raw_writes'] == []
    else:
        assert case['ordered_reads'] == base['ordered_reads']
        assert case['ordered_raw_writes'] == base['ordered_raw_writes']
        after = dict(case['prior_raw'])
        for write in case['ordered_raw_writes']:
            assert write['address'] == target['to'] and write['key'] in after
            after[write['key']] = write['value']
        assert after == case['post']['after_raw']

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    parser.add_argument('--capture', type=Path)
    args = parser.parse_args()
    scratch = Path(os.environ.get('RUSTAXA_P0_PREFIX_TRACE_SCRATCH', '/home/fry/artifacts/claim-all-p0-prefix-trace-scratch'))
    scratch.mkdir(parents=True, exist_ok=True)
    producer.SOURCE = SOURCE
    outputs, details = {}, {}
    for control in [False, True]:
        for label in producer.REVISIONS:
            name = label + ('.control' if control else '')
            outputs[name], details[name] = producer.run_reference(label, control, scratch)
            if args.capture:
                args.capture.mkdir(parents=True, exist_ok=True)
                for suffix, data in zip(['json', 'stdout.txt', 'stderr.txt'], outputs[name], strict=True):
                    (args.capture / (name + '.' + suffix)).write_bytes(data)
                (args.capture / (name + '.instrumentation.json')).write_text(json.dumps(details[name], indent=2, sort_keys=True) + '\n')
    assert outputs['public'] == outputs['local']
    assert outputs['public.control'] == outputs['local.control']
    for label in producer.REVISIONS:
        observed, control = json.loads(outputs[label][0]), json.loads(outputs[label+'.control'][0])
        validate(observed)
        validate(control, True)
        assert producer.inherited.strip(observed) == producer.inherited.strip(control)
    files = {}
    for name, output in outputs.items():
        for suffix, data in zip(['json', 'stdout.txt', 'stderr.txt'], output, strict=True):
            files[name+'.'+suffix] = data
        files[name+'.instrumentation.json'] = (json.dumps(details[name], indent=2, sort_keys=True)+'\n').encode()
    inherited = [Path(producer.__file__), Path(producer.inherited.__file__), Path(producer.boundary.__file__), Path(producer.trace.__file__), Path(producer.boundary.h1.__file__), Path(producer.boundary.custody.__file__), Path(producer.boundary.reward.__file__)]
    accepted = HERE / 'fixtures/native_reward_claim_all_h1_trace'
    manifest = {'schema':1, 'profile':'literal_H1_P0_ordinary_prefix_target200000', 'references':producer.REVISIONS,
                'source_sha256':{p.name:digest(p.read_bytes()) for p in [SOURCE,Path(__file__),*producer.SUPPORT,*inherited]},
                'accepted_empty_P0_sha256':{p.name:digest(p.read_bytes()) for p in [accepted/'public.json',accepted/'manifest.json']},
                'instrumented_sha256':{name:data['instrumented_sha256'] for name,data in details.items()},
                'outputs':{name:digest(data) for name,data in files.items()},
                'repeat_count':2, 'target_cap':200000, 'prefix_cap':21000,
                'transaction_position_authority':'exact ordered composition; not Go concrete transaction metadata',
                'go_version':subprocess.check_output(['rtk','proxy','go','version'],text=True).strip()}
    files['manifest.json']=(json.dumps(manifest,indent=2,sort_keys=True)+'\n').encode()
    if args.record:
        FIXTURES.mkdir(parents=True,exist_ok=True)
    if args.record or not args.capture:
        for name,data in files.items():
            if args.record:
                (FIXTURES/name).write_bytes(data)
            else:
                assert (FIXTURES/name).read_bytes()==data,name
    print('Actual P0 ordinary-prefix pins/repeats/controls matched; sources unchanged')

if __name__ == '__main__':
    main()
