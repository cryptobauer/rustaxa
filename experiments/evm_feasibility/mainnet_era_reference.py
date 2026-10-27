#!/usr/bin/env python3
"""Record/reproduce finite mainnet-policy fixtures in separate immutable files."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

import mixed_period_reference as mixed
import native_integration_reference as capture
from mainnet_era_profile import ROOT, REFERENCES, GENESIS, go_configuration, profile, scenarios
from mainnet_era_compare import comparison_digest, VERSION as COMPARISON_VERSION
from mainnet_era_code_objects import decode_code_objects, code_objects_binding, strict_json_loads

HERE = Path(__file__).resolve().parent
CORPUS = HERE / 'fixtures/mainnet_era'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def record_live_peak(artifact, directory, phase):
    launch = os.environ.get('RUSTAXA_ERA_LAUNCH_MANIFEST')
    if launch is None:
        return
    context = json.loads(Path(launch).read_bytes())
    count = lambda path: sum(p.stat().st_size for p in path.rglob('*') if p.is_file() and not p.is_symlink())
    actual = count(artifact) + count(CORPUS)
    resources = context['resources']
    charged = resources['prior_charged_bytes'] + max(0, actual-resources['baseline_actual_bytes']) + 2*1024*1024
    if charged + (32*1024*1024 if phase == 'before_child' else 0) > 1342177280:
        raise ValueError('oracle live/output peak exceeds amended cap')
    ledger = Path(context['scratch_ledger']).parent / 'oracle-live.jsonl'
    with ledger.open('a') as output:
        output.write(json.dumps({'phase':phase,'directory':str(directory),'charged_live_bytes':charged,'compiler_tree_bytes':count(directory),'output_reserve_bytes':32*1024*1024 if phase=='before_child' else 0})+'\n')
        output.flush(); os.fsync(output.fileno())


def run(pin, label, scenario, schedule, artifact, stop=0, selector=None, frozen=None):
    if selector is None:
        raise ValueError("explicit bounded output selector required")
    directory = Path(tempfile.mkdtemp(prefix=f"era-{label}-{scenario['name']}-", dir=artifact))
    source = ROOT / 'submodules/taraxa-evm'
    actual = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(source), 'rev-parse', pin + '^{commit}'], text=True).strip()
    if actual != pin:
        raise ValueError('Go reference pin differs')
    archive = subprocess.check_output(['rtk', 'proxy', 'git', '-C', str(source), 'archive', pin])
    subprocess.run(['rtk', 'proxy', 'tar', '-x', '-C', str(directory)], input=archive, check=True)
    mixed.apply_raw_trace_patch(directory)
    command = directory / 'cmd/mainnet_era'
    command.mkdir(parents=True)
    for name in [*mixed.SOURCES, 'native_integration_reference.go', 'mainnet_era_reference.go', 'mainnet_era_api.go', 'mainnet_era_seed.go', 'mainnet_era_actions.go', 'mainnet_era_pending.go', 'mainnet_era_code_objects.go']:
        raw = (HERE / name).read_bytes()
        if name in ('mixed_period_reference.go', 'native_integration_reference.go'):
            if raw.count(b'func main() {') != 1:
                raise ValueError('nonunique main entry')
            raw = raw.replace(b'func main() {', b'func unused_' + name.encode().replace(b'.', b'_') + b'() {', 1)
        if name == 'native_integration_reference.go':
            for before, after in [
                (b'GasPrice: big.NewInt(1), Gas: gas', b'GasPrice: big.NewInt(1000000000), Gas: gas'),
                (b'"gas_price": "1", "input": hex.EncodeToString(r.input)', b'"gas_price": "1000000000", "input": hex.EncodeToString(r.input)'),
                (b'return &vm.Block{Number: types.BlockNum(period), BlockInfo: vm.BlockInfo{Author: integrationValidators[1], GasLimit: fullBlockGas, Difficulty: new(big.Int)}}', b'return eraBlock(period)'),
            ]:
                if raw.count(before) != 1:
                    raise ValueError('nonunique mainnet API support adaptation')
                raw = raw.replace(before, after, 1)
        (command / name).write_bytes(raw)
    # Native integration's trace helper is a linked dependency even when the
    # era command does not yet request default API traces.
    import native_retained_commission_trace_reference as tracing
    import native_reward_commission_trace_reference as trace_source
    trace_target = directory / tracing.TRACE_TARGET if hasattr(tracing, 'TRACE_TARGET') else directory / 'taraxa/state/state_dry_runner/trace_runner.go'
    trace_raw = trace_target.read_bytes()
    if digest(trace_raw) != trace_source.TRACE_SHA:
        raise ValueError('trace source pin differs')
    for needle, replacement in [
        (tracing.PREFIX, b'\t\tobserveRetainedTrace("prefix",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("prefix",true,block_state,&trx,&ret,err)\n'),
        (tracing.TARGET, b'\t\tobserveRetainedTrace("target",false,block_state,&trx,nil,nil)\n\t\tret, err := evm.Main(&trx)\n\t\tobserveRetainedTrace("target",true,block_state,&trx,&ret,err)\n'),
    ]:
        if trace_raw.count(needle) != 1:
            raise ValueError('nonunique trace observer insertion')
        trace_raw = trace_raw.replace(needle, replacement, 1)
    trace_target.write_bytes(trace_raw)
    (trace_target.parent / 'native_integration_trace_observer.go').write_bytes(tracing.HELPER)
    def donor_input(value, depth=0):
        if depth > 3:
            raise ValueError('bounded donor depth exceeded')
        result = {'configuration': go_configuration(), **value, 'schedule': schedule_for(value, artifact), 'stop_before_rewards': 0}
        if 'donor' in value:
            result['donor'] = donor_input(value['donor'], depth+1)
        return result
    full_input = {'configuration': go_configuration(), **scenario, 'schedule': schedule, 'stop_before_rewards': stop}
    if 'donor' in scenario:
        full_input['donor'] = donor_input(scenario['donor'])
    input_path = command / 'input.json'
    input_path.write_text(json.dumps(full_input, separators=(',', ':')) + '\n')
    if frozen is not None:
        if frozen['reference_pin'] != pin or frozen['selector'] != selector or frozen['base_archive_sha256'] != digest(archive):
            raise ValueError('frozen producer/reference/selector/archive mismatch')
        expected = {name: value for name, value in frozen['effective_sources'].items() if not name.startswith('harness/')}
        for path in command.glob('*.go'):
            if str(path.relative_to(directory)) not in expected:
                raise ValueError('unbound additional Go producer source')
        for name, value in expected.items():
            path = directory / name
            if not path.is_relative_to(directory) or '..' in Path(name).parts:
                raise ValueError('frozen producer path escape')
            raw = (artifact / 'source-objects' / value['sha256']).read_bytes()
            if len(raw) != value['bytes'] or digest(raw) != value['sha256']:
                raise ValueError('frozen producer source identity')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
    objects = artifact / 'source-objects'
    objects.mkdir(exist_ok=True)
    effective = {}
    paths = [*command.iterdir(), directory / mixed.TRACE_TARGET, directory / mixed.REFUND_TRACE_TARGET,
             trace_target, trace_target.parent / 'native_integration_trace_observer.go']
    paths += list((directory / mixed.TRACE_TARGET).parent.glob('mixed_period_*observer.go'))
    paths += list((directory / mixed.REFUND_TRACE_TARGET).parent.glob('mixed_period_*observer.go'))
    paths += [Path(__file__), HERE / 'mainnet_era_profile.py', HERE / 'mainnet_era_historical_actions.py', HERE / 'mainnet_era_compare.py', HERE / 'mainnet_era_code_objects.py', GENESIS, scheduler_path(artifact)]
    for path in paths:
        raw = path.read_bytes()
        hashed = digest(raw)
        target = objects / hashed
        if not target.exists():
            target.write_bytes(raw)
        elif target.read_bytes() != raw:
            raise ValueError('source object collision')
        name = str(path.relative_to(directory)) if path.is_relative_to(directory) else 'harness/' + path.name
        effective[name] = {'bytes': len(raw), 'sha256': hashed}
    argv = ['rtk', 'proxy', 'go', 'run', '-mod=readonly',
            '-tags=mixed_raw_trace' + (',concrete_observer' if label == 'local' else ''),
            './cmd/mainnet_era', '--input', str(input_path), '--reference', pin, '--output', selector['kind']]
    if selector.get('first') is not None:
        argv.extend(['--first', str(selector['first']), '--end', str(selector['end'])])
    if label == 'local':
        argv.append('--observer')
    if frozen is not None and argv != [arg.replace(frozen['cwd'], str(directory)) for arg in frozen['argv']]:
        raise ValueError('frozen exact argv mismatch')
    prepared = {'reference_pin': pin, 'base_archive_sha256': digest(archive),
                'base_archive_bytes': len(archive), 'effective_sources': effective,
                'argv': argv, 'selector': selector, 'cwd': str(directory), 'phase': 'prepared_before_launch',
                'disposable_compiler_tree': str(directory)}
    if frozen is not None:
        prepared['exact_retained_producer'] = digest(json.dumps(frozen, sort_keys=True).encode())
    (directory / 'command.json').write_text(json.dumps(prepared, indent=2) + '\n')
    record_live_peak(artifact, directory, 'before_child')
    status = capture.capture_child(argv, directory, directory, timeout_seconds=600, max_output_bytes=33554432)
    record_live_peak(artifact, directory, 'child_terminated')
    if not status['capture_complete'] or status['child_exit']:
        raise RuntimeError(f'Go {label}/{scenario["name"]} failed; full capture: {directory}')
    raw = gzip.decompress((directory / 'stdout.gz').read_bytes())
    data = strict_json_loads(raw.splitlines()[-1])
    return data, directory


API_NAMES = ['immediate_delegation', 'delayed_votes', 'delayed_jail', 'delayed_jail_list',
             'private_claim', 'negative_short_selector', 'ordinary_storage_value_log']


def api_names(scenario, stop=0):
    if scenario.get('seed_actions_only'):
        if stop: raise ValueError('seed action prefix cannot be a private reward fork')
        return []
    heads = ([stop] if stop else ([scenario['head'], 25706900, 25706905, 25706906, 25706911, 25707000]
             if scenario['name'] == 'mainnet_106' else scenario.get('api_heads', scenario['periods'])))
    names = [*API_NAMES[:-1], scenario.get('ordinary_api_name', API_NAMES[-1])]
    return [f'{head}/{name}' for head in heads for name in names]


def selectors(scenario, stop=0):
    if stop:
        return [{'kind': 'fork'}, {'kind': 'fork_api', 'first': 0, 'end': len(API_NAMES)}]
    first, end = scenario['first_retained'] + 1, scenario['periods'][-1] + 1
    result = [{'kind': 'base'}]
    result += [{'kind': 'periods', 'first': p, 'end': min(p + 20, end)} for p in range(first, end, 20)]
    result += [{'kind': 'api', 'first': i, 'end': min(i + 7, len(api_names(scenario)))}
               for i in range(0, len(api_names(scenario)), 7)]
    return result


def validate_shard(value, pin, scenario, selector, stop):
    kind = selector.get('kind')
    if kind in ('base', 'fork'):
        if set(selector) != {'kind'} or (kind == 'fork') != bool(stop):
            raise ValueError('inconsistent base/fork selector')
    else:
        first, end = selector.get('first'), selector.get('end')
        domain = list(range(scenario['first_retained'] + 1, scenario['periods'][-1] + 1)) if kind == 'periods' else list(range(len(api_names(scenario, stop))))
        limit = 20 if kind == 'periods' else 7
        if set(selector) != {'kind', 'first', 'end'} or kind not in ('periods', 'api', 'fork_api') or (kind == 'fork_api') != bool(stop) or type(first) is not int or type(end) is not int or not 0 < end - first <= limit or not set(range(first, end)).issubset(domain):
            raise ValueError('unknown/inverted/out-of-range selector')
    if value['schema'] != 3 or value['reference'] != pin or value['scenario'] != scenario['name']:
        raise ValueError('shard source/scenario identity')
    if value['starting_head'] != scenario['head'] or value['stop_before_rewards'] != stop:
        raise ValueError('shard head/fork identity')
    expected = {'kind': selector['kind'], 'first': selector.get('first', -1), 'end': selector.get('end', -1)}
    if value['selector'] != expected:
        raise ValueError('selector receipt differs from requested bounds')
    end = stop if stop else scenario['periods'][-1] + 1
    if [row['period'] for row in value['continuity']] != list(range(scenario['first_retained'] + 1, end)):
        raise ValueError('incomplete prefix/period continuity')
    if value['api_case_names'] != api_names(scenario, stop):
        raise ValueError('incomplete API identity domain')
    if kind in ('base', 'fork') and digest(json.dumps(value['payload']['history_rows'], sort_keys=True, separators=(',', ':')).encode()) != value['history_sha256']:
        raise ValueError('physical history payload identity mismatch')
    if value['code_objects_schema'] != 1 or type(value['code_objects_count']) is not int or value['code_objects_count'] < 0:
        raise ValueError('code object binding schema/count')
    if kind in ('base', 'fork') and code_objects_binding(value['payload']['code_objects']) != {field:value[field] for field in ['code_objects_schema','code_objects_count','code_objects_sha256']}:
        raise ValueError('code object base binding')
    if len(value['api_case_identities']) != len(value['api_case_names']):
        raise ValueError('incomplete API request identities')
    previous = value['seed_identity']['root']
    for row in value['continuity']:
        if row['prior_root'] != previous:
            raise ValueError('broken prior-root continuity')
        previous = row['root']
    if selector['kind'] == 'periods':
        rows = value['payload']['periods']
        if [row['number'] for row in rows] != list(range(selector['first'], selector['end'])):
            raise ValueError('missing/duplicate/out-of-range period')
        roots = {row['period']: row['root'] for row in value['continuity']}
        if any(row['final']['root'] != roots[row['number']] for row in rows):
            raise ValueError('period continuity root mismatch')
        identities = {row['period']: row for row in value['continuity']}
        if any(row['prior_root'] != identities[row['number']]['prior_root'] or [tx['hash'] for tx in row['transactions']] != identities[row['number']]['transaction_hashes'] for row in rows):
            raise ValueError('transaction/prior-root identity mismatch')
    if selector['kind'] in ('api', 'fork_api'):
        rows = value['payload']['api_cases']
        if [row['name'] for row in rows] != api_names(scenario, stop)[selector['first']:selector['end']]:
            raise ValueError('missing/duplicate/out-of-range API case')
        for index, case in enumerate(rows, selector['first']):
            identity = {key: case[key] for key in ['head', 'request', 'trace_period']}
            if digest(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()) != value['api_case_identities'][index]:
                raise ValueError('API head/request/trace identity mismatch')
            if not case['estimates'] or not case['repeat_call'] or not case['call']:
                raise ValueError('incomplete API call/estimation case')
            for name, count in [('trace', 1), ('trace_sequence', 2)]:
                if len(case[name]['calls']) != count or 'result' not in case[name]:
                    raise ValueError('missing actual trace callback sequence')
                stages = ['target'] if count == 1 else ['prefix', 'target']
                if [call['stage'] for call in case[name]['calls']] != stages or any(not all(key in call for key in ['before', 'after', 'result', 'ordered_raw_writes']) for call in case[name]['calls']):
                    raise ValueError('truncated/reordered trace callbacks')


def assemble(shards, pin, scenario, stop=0):
    expected = selectors(scenario, stop)
    if len(shards) != len(expected):
        raise ValueError('missing or extra shard')
    binding_fields = ['reference', 'scenario', 'starting_head', 'input_sha256', 'execution_input_sha256',
                      'configuration_sha256', 'history_sha256', 'observer', 'stop_before_rewards',
                      'seed_identity', 'continuity', 'api_case_names', 'api_case_identities',
                      'code_objects_schema', 'code_objects_count', 'code_objects_sha256']
    baseline = shards[0]
    by_selector = {}
    for shard in shards:
        key = json.dumps(shard['selector'], sort_keys=True)
        if key in by_selector:
            raise ValueError('duplicate/overlapping shard')
        by_selector[key] = shard
        if any(shard[field] != baseline[field] for field in binding_fields):
            raise ValueError('shard configuration/seed/history/continuity disagreement')
    result = None
    periods, cases = [], []
    for selector in expected:
        receipt = {'kind': selector['kind'], 'first': selector.get('first', -1), 'end': selector.get('end', -1)}
        shard = by_selector.get(json.dumps(receipt, sort_keys=True))
        if shard is None:
            raise ValueError('missing/overlapping/out-of-range selector')
        validate_shard(shard, pin, scenario, selector, stop)
        if selector['kind'] in ('base', 'fork'):
            result = dict(shard['payload'])
        periods.extend(shard['payload'].get('periods', []))
        cases.extend(shard['payload'].get('api_cases', []))
    if result is None:
        raise ValueError('missing complete base')
    result['api_cases'] = cases
    if stop:
        result['prefix_period_identities'] = baseline['continuity']
    else:
        result['periods'] = periods
    decode_code_objects(result, expected_binding={field:baseline[field] for field in ['code_objects_schema','code_objects_count','code_objects_sha256']})
    return result


def retain(data, directory=None):
    body = (json.dumps(data, sort_keys=True, separators=(',', ':')) + '\n').encode()
    encoded = gzip.compress(body, mtime=0)
    name = digest(body) + '.json.gz'
    target = CORPUS / name
    if target.exists():
        if target.read_bytes() != encoded:
            raise ValueError('immutable corpus collision')
    else:
        target.write_bytes(encoded)
    return {'file': name, 'sha256': digest(body), 'bytes': len(body), 'gzip_sha256': digest(encoded),
            'gzip_bytes': len(encoded), 'capture': str(directory) if directory else None}


def scheduler_path(artifact):
    path = Path(os.environ.get('RUN_SCHEDULER_MANIFEST', artifact / 'cpp-scheduler/manifest.json')).resolve()
    if not path.is_relative_to(artifact.resolve()):
        raise ValueError('scheduler evidence must belong to this task')
    return path


def schedule_for(scenario, artifact):
    scheduler = json.loads(scheduler_path(artifact).read_bytes())
    # The native owner closes after initializing first_retained. Before Aspen
    # the actual recovered unordered-map order is part of the oracle contract.
    reopen = scenario['first_retained'] if scenario['periods'][-1] < 8118000 else 0
    rows = next(c['rows'] for c in scheduler['cases'] if c['name'] == scenario['name'] and c['reopen'] == reopen)
    selected = {r['period']: r for r in rows}
    for p in range(scenario['first_retained'] + 1, scenario['periods'][0]):
        if p not in selected:
            if p >= 5730000:
                raise ValueError('missing warmup scheduler facts')
            selected[p] = {'period': p, 'order': [p]}
    return [selected[p] for p in sorted(selected)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', action='store_true')
    parser.add_argument('--probe', action='store_true')
    parser.add_argument('--batched-control', action='store_true')
    parser.add_argument('--scenario', choices=[s['name'] for s in scenarios()], required=True)
    args = parser.parse_args()
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    scenario = next(s for s in scenarios() if s['name'] == args.scenario)
    schedule = schedule_for(scenario, artifact)
    CORPUS.mkdir(exist_ok=True)
    if args.batched_control:
        if args.scenario != 'mainnet_106':
            raise ValueError('batched control is qualified only for current mainnet')
        from mainnet_era_estimate_reference import load
        manifest = json.loads((CORPUS / 'mainnet_106.manifest.json').read_bytes())
        public = load(manifest['streams']['public'])
        selector = {'kind': 'periods', 'first': 25706906, 'end': 25706908}
        data, directory = run(REFERENCES['local'], 'local_batched', scenario, schedule, artifact, 0, selector)
        validate_shard(data, REFERENCES['local'], scenario, selector, 0)
        expected = [row for row in public['periods'] if selector['first'] <= row['number'] < selector['end']]
        if data['observer'] or data['payload']['periods'] != expected:
            raise ValueError('local batched control differs from exact public cleanup records')
        result = {'schema': 1, 'scope': 'full chronological local-pin batched execution; selected cleanup-control records only',
                  'stream': retain(data, directory), 'public_stream': manifest['streams']['public'],
                  'selectors': selector, 'periods': [row['number'] for row in expected]}
        raw = (json.dumps(result, sort_keys=True, separators=(',', ':')) + '\n').encode()
        target = artifact / ('batched-control-' + digest(raw) + '.json')
        with target.open('xb') as output: output.write(raw)
        print('Local batched cleanup control matches exact public records:', target)
        return
    if args.probe:
        wanted = [(0, {'kind': 'base'}), (0, {'kind': 'periods', 'first': 25706900, 'end': 25706901}),
                  (0, {'kind': 'periods', 'first': 25707000, 'end': 25707001}),
                  (0, {'kind': 'api', 'first': 35, 'end': 42}),
                  (25706900, {'kind': 'fork'}), (25706900, {'kind': 'fork_api', 'first': 0, 'end': 7}),
                  (25707000, {'kind': 'fork'}), (25707000, {'kind': 'fork_api', 'first': 0, 'end': 7})]
        results = []
        for stop, selector in wanted:
            data, directory = run(REFERENCES['local'], 'local', scenario, schedule, artifact, stop, selector)
            validate_shard(data, REFERENCES['local'], scenario, selector, stop)
            results.append({'stop': stop, 'selector': selector, 'stream': retain(data, directory)})
        target = artifact / ('shard-probe-' + digest(json.dumps(results).encode()) + '.json')
        target.write_text(json.dumps(results, indent=2) + '\n')
        print('Bounded selector probes complete:', target)
        return
    manifest = {'schema': 2, 'references': REFERENCES, 'policy': profile(), 'scenario': scenario,
                'scheduler_manifest_sha256': digest(scheduler_path(artifact).read_bytes()),
                'support': {p.name: digest(p.read_bytes()) for p in [Path(__file__), HERE / 'mainnet_era_profile.py', HERE / 'mainnet_era_historical_actions.py',
                            HERE / 'mainnet_era_reference.go', HERE / 'mainnet_era_api.go', HERE / 'mainnet_era_seed.go', HERE / 'mainnet_era_actions.go', HERE / 'mainnet_era_pending.go', HERE / 'mainnet_era_code_objects.go', HERE / 'mainnet_era_code_objects.py', HERE / 'mainnet_era_scheduler.py', GENESIS]},
                'streams': {}, 'pre_reward_forks': {}, 'shards': {},
                'comparison': {'version': COMPARISON_VERSION, 'support_sha256': digest((HERE / 'mainnet_era_compare.py').read_bytes()), 'streams': {}, 'forks': {}}}
    for label, pin in REFERENCES.items():
        manifest['shards'][label] = {}
        normal = None
        for stop in [0, *([25706900, 25707000] if args.scenario == 'mainnet_106' else [])]:
            shards, pins = [], []
            for selector in selectors(scenario, stop):
                data, directory = run(pin, label, scenario, schedule, artifact, stop, selector)
                validate_shard(data, pin, scenario, selector, stop)
                shards.append(data)
                pins.append({'selector': selector, 'stream': retain(data, directory)})
            value = assemble(shards, pin, scenario, stop)
            stream = retain(value)
            manifest['shards'][label][str(stop)] = pins
            if stop == 0:
                normal = shards[0]['continuity']
                manifest['streams'][label] = stream
                manifest['comparison']['streams'][label] = comparison_digest(value)
            else:
                if shards[0]['continuity'] != [row for row in normal if row['period'] < stop]:
                    raise ValueError('fork prefix differs from uninterrupted execution')
                manifest['pre_reward_forks'].setdefault(label, {})[str(stop)] = stream
                # Forks contain exact prefix identities, authenticated above,
                # and no randomized distribution transcript. Every private
                # account/history/API byte remains exact in this digest.
                manifest['comparison']['forks'].setdefault(label, {})[str(stop)] = stream['sha256']
        checkpoint = artifact / ('shard-progress-' + digest(json.dumps(manifest).encode()) + '.json')
        checkpoint.write_text(json.dumps(manifest, indent=2) + '\n')
    manifest_path = CORPUS / (args.scenario + '.manifest.json')
    if args.record:
        if manifest_path.exists():
            previous = manifest_path.read_bytes()
            history = CORPUS / (digest(previous) + '.manifest.json')
            if not history.exists(): history.write_bytes(previous)
        manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    else:
        previous = json.loads(manifest_path.read_bytes())
        if manifest['support'] != previous['support']:
            raise ValueError('reproduction source mismatch')
        # Raw objects stay separately authenticated. Constrained comparison is
        # required for Go's randomized validator map; never equate raw hashes.
        if manifest['comparison'] != previous['comparison']:
            raise ValueError('validated same-pin comparison/fork identity differs')
    print(args.scenario, 'recorded' if args.record else 'reproduced', 'at both Go pins with complete bounded shards')


if __name__ == '__main__':
    main()
