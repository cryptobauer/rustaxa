#!/usr/bin/env python3
"""Record/reproduce actual retained P2 default TraceRunner sequences and controls."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from reference import REVISIONS, ROOT
import native_retained_commission_estimate_reference as estimate
import native_retained_commission_reference as retained
import native_v1_custody_reference as custody
import native_redelegate_reward_reference as reward
import native_reward_commission_trace_reference as trace

HERE = Path(__file__).resolve().parent
GO = HERE / 'native_retained_commission_trace_reference.go'
FIXTURES = HERE / 'fixtures/native_retained_commission_trace'
HELPER = b'''package state_dry_runner
import (
 "github.com/Taraxa-project/taraxa-evm/core/vm"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)
var retainedTraceObserver func(string,bool,*state_evm.BlockState,*vm.Transaction,*vm.ExecutionResult,error)
func SetRetainedTraceObserver(f func(string,bool,*state_evm.BlockState,*vm.Transaction,*vm.ExecutionResult,error)){retainedTraceObserver=f}
func observeRetainedTrace(stage string, after bool,s *state_evm.BlockState,t *vm.Transaction,r *vm.ExecutionResult,e error){if retainedTraceObserver!=nil{retainedTraceObserver(stage,after,s,t,r,e)}}
'''
PREFIX = b'\t\tevm.Main(&trx)\n'
TARGET = b'\t\tret, _ := evm.Main(&trx)\n'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(pin, control, baseline):
    with tempfile.TemporaryDirectory(prefix='retained-trace-', dir=os.environ['RUN_ARTIFACT_DIR']) as directory:
        tree = Path(directory)
        archive = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(ROOT / 'submodules/taraxa-evm'), 'archive', pin])
        subprocess.run(['rtk', 'proxy', 'tar', '-x', '-C', directory], input=archive, check=True)
        target = tree / custody.TRACE_TARGET
        source = target.read_bytes()
        assert digest(source) == custody.TRACE_SHA256 and source.count(custody.TRACE_NEEDLE) == 1
        if not control:
            source = source.replace(custody.TRACE_NEEDLE, custody.TRACE_INSERTION, 1)
        for op, expression in [('add', 'Add'), ('sub', 'Sub')]:
            needle = ('self.set_balance(new(big.Int).' + expression + '(self.Balance, amount))').encode()
            assert source.count(needle) == 1
            if not control:
                source = source.replace(needle, needle + b'\n\t\tobserveRewardBalance(self.addr,"' + op.encode() + b'",amount,self.Balance)', 1)
        target.write_bytes(source)
        (target.parent / 'retained_raw_observer.go').write_bytes(custody.TRACE_HELPER)
        (target.parent / 'retained_balance_observer.go').write_bytes(reward.BALANCE_HELPER)
        target = tree / trace.TRACE_TARGET
        original_trace = target.read_bytes()
        assert digest(original_trace) == trace.TRACE_SHA
        assert original_trace.count(PREFIX) == original_trace.count(TARGET) == 1
        changed = original_trace
        if not control:
            changed = changed.replace(PREFIX, b'\t\tobserveRetainedTrace("prefix",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("prefix",true,block_state,&trx,&ret,err)\n', 1)
            changed = changed.replace(TARGET, b'\t\tobserveRetainedTrace("target",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("target",true,block_state,&trx,&ret,err)\n', 1)
        target.write_bytes(changed)
        (target.parent / 'retained_trace_observer.go').write_bytes(HELPER)
        command = tree / 'cmd/retained_trace'
        command.mkdir(parents=True)
        for index, support in enumerate(retained.SUPPORT):
            source = support.read_text()
            assert source.count('func main() {') == 1
            (command / support.name).write_text(source.replace('func main() {', f'func unusedSupport{index}Main() {{', 1))
        (command / retained.GO.name).write_bytes(estimate.seed_source())
        (command / GO.name).write_bytes(GO.read_bytes())
        result = subprocess.run(['rtk', 'proxy', 'go', 'run', '-mod=readonly', './cmd/retained_trace'], cwd=tree,
                                input=json.dumps(baseline).encode(), capture_output=True,
                                env={**os.environ, 'RETAINED_RAW_KEYS': json.dumps(baseline['raw_key_inventory'])})
        if result.returncode:
            os.write(1, result.stdout)
            os.write(2, result.stderr)
            result.check_returncode()
        lines = result.stdout.splitlines(keepends=True)
        assert lines, 'missing oracle output'
        return (lines[-1], b''.join(lines[:-1]), result.stderr), {'account_original': custody.TRACE_SHA256,
                'account_effective': digest((tree / custody.TRACE_TARGET).read_bytes()),
                'trace_original': digest(original_trace), 'trace_effective': digest(changed)}


def strip(value):
    if isinstance(value, dict):
        return {k: strip(v) for k, v in value.items() if k != 'observation_witness'}
    if isinstance(value, list):
        return [strip(v) for v in value]
    return value


def validate(d, baseline):
    assert d['schema'] == 1 and d['period'] == 3 and d['timestamp'] == 1700000003
    assert d['author'] == '00' * 19 + '31' and d['block_gas'] == 1000000
    assert d['repeat_count'] == 2 and d['configuration'] == baseline['configuration']
    assert d['state_before'] == d['state_after'] == baseline['state_before']
    assert [c['name'] for c in d['cases']] == ['positive', 'prefix_then_zero', 'wrong_owner']
    key = baseline['cases'][1]['ordered_raw_writes'][0]['key']
    contract = '00' * 19 + 'fe'
    for c in d['cases']:
        has_prefix = c['name'] == 'prefix_then_zero'
        wrong = c['name'] == 'wrong_owner'
        assert len(c['prefix']) == int(has_prefix) and len(c['targets']) == 1
        assert c['result'] == [{'gas': 41464, 'failed': wrong, 'returnValue': '', 'structLogs': []}]
        requests = c['prefix'] + c['targets']
        calls = c['observation_witness']
        assert len(calls) == len(requests)
        cumulative_logs = []
        for index, (request, call) in enumerate(zip(requests, calls, strict=True)):
            assert request['nonce'] == str(2**512 + index)
            assert request['sender'] == baseline['delegator' if wrong else 'owner']
            assert request['to'] == contract and request['gas'] == 200000
            assert request['gas_price'] == request['value'] == '0'
            assert request['input'] == 'd0eebfe2' + '0' * 62 + '31'
            assert call['request'] == request and call['stage'] == ('prefix' if has_prefix and index == 0 else 'target')
            expected = baseline['cases'][0 if wrong else (2 if index else 1)]
            out = call['output']
            assert out['gas_used'] == expected['gas_used'] and out['return'] == expected['output']
            assert out['execution_error'] == expected['execution_error'] and out['consensus_error'] == expected['consensus_error']
            # TraceRunner keeps one journal: Go's returned Logs include prior calls.
            cumulative_logs += expected['logs']
            assert out['logs'] == cumulative_logs
            assert out['error'] == expected['execution_error']
            assert call['raw'] == expected['ordered_raw_writes']
            assert call['balances'] == expected['ordered_balance_effects']
            before, after = call['before'], call['after']
            assert before['raw'][key] == (baseline['state_before']['native_raw'][key] if index == 0 else expected['ordered_raw_writes'][0]['value'])
            raw = dict(before['raw'])
            for w in call['raw']:
                raw[w['key']] = w['value']
            assert raw == after['raw']
            accounts = json.loads(json.dumps(before['accounts']))
            accounts[request['sender']]['nonce'] = str(int(request['nonce']) + 1)
            for effect in call['balances']:
                a = accounts[effect['address']]
                sign = 1 if effect['operation'] == 'add' else -1
                a['balance'] = str(int(a['balance']) + sign * int(effect['amount']))
                assert a['balance'] == effect['balance']
            assert accounts == after['accounts']
        if has_prefix:
            assert calls[0]['after'] == calls[1]['before']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    args = parser.parse_args()
    assert REVISIONS == {'public': '6c7e5338b22d5e596cc2365a88d1f94840e1ee1b', 'local': 'bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418'}
    accepted = json.loads((estimate.FIXTURES / 'manifest.json').read_bytes())
    for name, sha in accepted['source_sha256'].items():
        assert digest((HERE / name).read_bytes()) == sha, name
    assert digest(estimate.INPUT.read_bytes()) == accepted['input_sha256']
    assert digest(estimate.seed_source()) == accepted['generated_seed_sha256']
    baseline = json.loads(estimate.INPUT.read_bytes())
    outputs, hooks = {}, {}
    evidence = Path(tempfile.mkdtemp(prefix='raw-oracle-', dir=os.environ['RUN_ARTIFACT_DIR']))
    for label, pin in REVISIONS.items():
        for control in (False, True):
            name = label + ('.control' if control else '')
            outputs[name], hooks[name] = run(pin, control, baseline)
            for suffix, data in zip(('json', 'stdout.txt', 'stderr.txt'), outputs[name], strict=True):
                (evidence / (name + '.' + suffix)).write_bytes(data)
    assert outputs['public'] == outputs['local'] and outputs['public.control'] == outputs['local.control']
    for label in REVISIONS:
        d = json.loads(outputs[label][0])
        validate(d, baseline)
        assert strip(d) == strip(json.loads(outputs[label + '.control'][0]))
    sources = retained.SUPPORT + [retained.GO, GO, Path(__file__), Path(estimate.__file__), Path(retained.__file__),
                                 Path(custody.__file__), Path(reward.__file__), Path(trace.__file__), HERE / 'reference.py']
    trace_sequences = sum(json.loads(v[0])['repeat_count'] * len(json.loads(v[0])['cases']) for v in outputs.values())
    actual_calls = sum(json.loads(v[0])['repeat_count'] * sum(len(c['prefix']) + len(c['targets']) for c in json.loads(v[0])['cases']) for v in outputs.values())
    assert trace_sequences > 0 and actual_calls > 0
    manifest = {'schema': 1, 'references': REVISIONS, 'sources': {p.name: digest(p.read_bytes()) for p in sources},
                'input_sha256': digest(estimate.INPUT.read_bytes()), 'accepted_estimate_manifest_sha256': digest((estimate.FIXTURES / 'manifest.json').read_bytes()),
                'generated_seed_sha256': digest(estimate.seed_source()), 'hooks': hooks,
                'helper_sha256': [digest(HELPER), digest(custody.TRACE_HELPER), digest(reward.BALANCE_HELPER)],
                'configuration_sha256': digest(json.dumps(baseline['configuration'], sort_keys=True).encode()),
                'go_version': subprocess.check_output(['rtk', 'proxy', 'go', 'version'], text=True).strip(),
                'hook_free_control_scope': 'ordinary requests/results/configuration/committed identity; live state observations only in instrumented runs',
                'trace_sequences': trace_sequences, 'actual_calls': actual_calls,
                'sha256': {k: [digest(b) for b in v] for k, v in outputs.items()}}
    if args.record:
        FIXTURES.mkdir(parents=True, exist_ok=True)
        (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    else:
        assert json.loads((FIXTURES / 'manifest.json').read_bytes()) == manifest
    for name, streams in outputs.items():
        for suffix, data in zip(('json', 'stdout.txt', 'stderr.txt'), streams, strict=True):
            path = FIXTURES / (name + '.' + suffix)
            if args.record:
                path.write_bytes(data)
            else:
                assert path.read_bytes() == data, str(path)
    print('2 pins; 3 sequences; fresh first/repeated runs; 24 traces/32 calls including hook-free controls; exact default JSON and separate native witnesses matched')


if __name__ == '__main__':
    main()
