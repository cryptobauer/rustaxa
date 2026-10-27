#!/usr/bin/env python3
"""Retain and execute unchanged C++ BlockStats aggregation and Stats scheduling.

Only input/DB/RLP leaves and visibility for observation are substituted. Complete
original headers and .cpp files are retained. Methods are never rewritten. The
observation-only period member follows each actual payload through the scheduler.
Period vectors retain actual C++ unordered/sorted order for the declared input
and DB-reload leaf sequence. Validator-map serialization alone is compared by key.
This script is a reference command; the lead launches it through capture_command.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile

from mainnet_era_profile import ROOT, profile, pending_periods
from native_integration_reference import capture_child

HERE = Path(__file__).resolve().parent
CORPUS = HERE / 'fixtures/mainnet_era'
SUPPORT = HERE / 'mainnet_era_cpp_stats_support.cpp'
PATHS = {
    'BLOCK_HEADER': 'libraries/core_libs/consensus/include/rewards/block_stats.hpp',
    'STATS_HEADER': 'libraries/core_libs/consensus/include/rewards/rewards_stats.hpp',
    'BLOCK_METHODS': 'libraries/core_libs/consensus/src/rewards/block_stats.cpp',
    'STATS_METHODS': 'libraries/core_libs/consensus/src/rewards/rewards_stats.cpp',
    'hardfork_header': 'libraries/config/include/config/hardfork.hpp',
    'hardfork_source': 'libraries/config/src/hardfork.cpp',
    'storage_source': 'libraries/core_libs/storage/src/storage.cpp',
    'period_type_header': 'libraries/common/include/common/types.hpp',
}
FIELDS = ('number', 'block_author', 'blocks_per_year', 'max_votes_weight',
          'total_dag_blocks_count', 'total_votes_weight')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def pin(path):
    raw = path.read_bytes()
    return {'file': str(path), 'bytes': len(raw), 'sha256': sha(raw)}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def strip_directives(text):
    # No body substitutions: only include/pragma lines are removed.
    return ''.join(line for line in text.splitlines(keepends=True)
                   if not line.startswith(('#include', '#pragma once')))


def method(text, signature):
    require(text.count(signature) == 1, 'method extraction guard: ' + signature)
    start = text.index(signature)
    first = text.index('{', start)
    level = 1
    end = first + 1
    while level:
        require(end < len(text), 'unterminated method')
        level += (text[end] == '{') - (text[end] == '}')
        end += 1
    return text[start:end]


def canonical(row):
    require(all(k in row for k in FIELDS), 'incomplete BlockStats fields')
    validators = {}
    for value in row['validators']:
        address = value['validator']
        require(re.fullmatch('[0-9a-f]{40}', address) is not None, 'address width')
        require(address not in validators, 'duplicate validator map key')
        fee = int(value['fees_reward'])
        require(0 <= fee < 2**256, 'uint256 fee width')
        validators[address] = {'dag_blocks_count': int(value['dag_blocks_count']),
                               'vote_weight': int(value['vote_weight']), 'fees_reward': str(fee)}
    # Only validator-map serialization order is ignored. Period vectors remain exact.
    return {**{k: row[k] for k in FIELDS}, 'validators': validators}


def load_document(value):
    name = value['file']
    require(Path(name).name == name, 'corpus stream path')
    stored = (CORPUS / name).read_bytes()
    require(len(stored) == value['gzip_bytes'] and sha(stored) == value['gzip_sha256'],
            'compressed corpus identity')
    raw = gzip.decompress(stored)
    require(len(raw) == value['bytes'] and sha(raw) == value['sha256'], 'inner corpus identity')
    return json.loads(raw)


def coordinate(value, name):
    require(type(value) is int and 0 <= value < 2**64, name + ' coordinate width')
    return value


def consecutive(values, name):
    require(isinstance(values,list) and values, name + ' nonempty ordered identities required')
    for value in values:
        coordinate(value,name)
    require(values == list(range(values[0],values[-1]+1)), name + ' missing/duplicated/reordered identities')
    return values


def prepare(document, policy, scenario):
    require(document['scenario'] == scenario['name'], 'scenario name binding')
    require(document['configuration']['chain_id'] == policy['chain_id'] == 841, 'chain policy')
    configuration=document['configuration']
    require(configuration['committee_size'] == policy['committee_size'], 'committee policy')
    hf=configuration['hardforks']
    require(int(hf['magnolia']['block']) == policy['hardforks']['magnolia_hf']['block_num']
            and int(hf['aspen']['part_one_block']) == policy['hardforks']['aspen_hf']['block_num_part_one']
            and int(hf['cacti']['block']) == policy['hardforks']['cacti_hf']['block_num'], 'actual aggregation activation policy')
    configured_frequency=[(int(row['start_period']),int(row['frequency']))
                          for row in hf['rewards_distribution_frequency']]
    require(configured_frequency == sorted((int(k),int(v)) for k,v in policy['hardforks']['rewards_distribution_frequency'].items()),
            'actual distribution frequency policy')
    require(int(configuration['dpos']['blocks_per_year']) == policy['blocks_per_year_before_cacti'], 'original annualized rate policy')
    head=coordinate(scenario['head'],'head')
    retained=coordinate(scenario['first_retained'],'first retained')
    wanted=consecutive(scenario['periods'],'required suffix')
    require(wanted[0] == head+1 and retained <= head, 'declared suffix/head/seed chronology')
    require(document['head'] == head and document['seed']['period'] == retained, 'actual seed/head identities')
    declared_pending=scenario['pending_periods']
    require(declared_pending == pending_periods(policy,head), 'declared pending interval differs from actual policy')
    # The seed's unfinished interval must also be supplied; a suffix which
    # crosses an earlier distribution cannot invent the missing cache rows.
    require([p for p in declared_pending if p <= retained] == pending_periods(policy,retained),
            'scenario pending inputs do not authenticate the retained seed cache')
    pending=document['pending_reward_facts']
    require(isinstance(pending,list) and [r['number'] for r in pending] == declared_pending,
            'pending fact identities/coverage')
    require(len(set(declared_pending)) == len(declared_pending), 'duplicate pending identities')
    rows={r['number']:dict(r) for r in pending}
    periods=document['periods']
    executed=list(range(retained+1,wanted[-1]+1))
    require(isinstance(periods,list) and [r['number'] for r in periods] == executed,
            'complete warmup and required suffix execution identities')
    require([r['number'] for r in periods if r['seed_prefix'] is False] == wanted,
            'required suffix phase classification')
    for period in periods:
        p=period['number']
        require(period['seed_prefix'] is (p <= head), 'warmup phase classification')
        expected=period['reward_input']
        require(expected['number'] == p, 'reward input identity')
        if p in rows:
            # These are redundant views of the same actual Go fact, not an
            # allowance to replace a pending row with a different warmup row.
            require(rows[p] == expected, 'pending/warmup fact overlap must be exact')
        row=dict(expected)
        txs=period['transactions']
        if 'transactions' in row:
            require(row['transactions'] == txs, 'actual transaction leaf overlap must be exact')
        row['transactions']=txs
        rows[p]=row
        require(period['distribution_order'] == [part['period'] for part in period['distributions']],
                'declared/consumed distribution vector mismatch')
        require(len(set(period['distribution_order'])) == len(period['distribution_order']),
                'duplicate distribution source occurrence')
    ordered=sorted(rows)
    require(ordered and ordered == list(range(ordered[0],wanted[-1]+1)), 'complete finite scheduler interval')
    require(set(ordered) == set(declared_pending).union(executed), 'no fabricated scheduler inputs')
    lines=[]
    cacti=policy['hardforks']['cacti_hf']
    for p in ordered:
        row=rows[p]
        planner=row['planner_facts']
        require(planner['committee_size'] == policy['committee_size'], 'planner committee')
        eligible=int(planner['total_eligible_vote_count'])
        require(0 <= eligible < 2**64, 'eligible-vote width')
        rate=policy['blocks_per_year_before_cacti']
        if p >= cacti['block_num']:
            # The exporter declares synthetic dynamic timing; default1000ms is
            # its existing source-pinned timing, never a shortened locking rule.
            lambda_ms=planner.get('lambda_ms',scenario.get('lambda_ms',1000))
            require(type(lambda_ms) is int and cacti['lambda_min'] <= lambda_ms <= cacti['lambda_max'],
                    'declared timing outside actual mainnet bounds')
            rate=365*24*60*60*1000 // (2*lambda_ms + int(cacti['consensus_delay']))
        require(0 < rate < 2**32 and int(row['blocks_per_year']) == rate, 'actual-period annual-rate derivation')
        txs=row.get('transactions',[])
        dags,votes=planner['dag_blocks'],planner['certificate_votes']
        if p < executed[0]:
            require('transactions' in row or not any(d.get('transaction_hashes') for d in dags),
                    'pending transaction facts missing for nonempty DAG hashes')
        author=row['block_author']
        require(re.fullmatch('[0-9a-f]{40}',author) is not None, 'block author width')
        lines.append(f"{p} {row['blocks_per_year']} {eligible} {author} {len(txs)} {len(dags)} {len(votes)}")
        hashes=set()
        for tx in txs:
            require(re.fullmatch('[0-9a-f]{64}',tx['hash']) is not None and tx['hash'] not in hashes,
                    'transaction hash identity')
            hashes.add(tx['hash'])
            price,used=int(tx['gas_price']),int(tx['gas_used'])
            require(0 <= price < 2**256 and 0 <= used < 2**64, 'transaction arithmetic widths')
            lines.append(f"{tx['hash']} {price} {used}")
        for dag in dags:
            hashes=dag.get('transaction_hashes') or []
            require(re.fullmatch('[0-9a-f]{40}',dag['author']) is not None, 'DAG author')
            require(0 <= int(dag['difficulty']) < 2**16, 'DAG difficulty width')
            require(all(re.fullmatch('[0-9a-f]{64}',h) is not None for h in hashes), 'DAG transaction hash')
            lines.append(f"{dag['author']} {dag['difficulty']} {len(hashes)} " + ' '.join(hashes))
        seen=set()
        for vote in votes:
            require(vote['validator'] not in seen and re.fullmatch('[0-9a-f]{40}',vote['validator']) is not None,
                    'vote identity')
            seen.add(vote['validator'])
            require(0 < int(vote['period']) < 2**64 and 0 < int(vote['weight']) < 2**64, 'vote widths')
            lines.append(f"{vote['period']} {vote['weight']} {vote['validator']}")
    return rows,'\n'.join(lines)+'\n'


def reopen_modes(document,scenario,expected):
    selected=[0,scenario['head']]
    first_distribution=next((row['number'] for row in document['periods']
                             if row['number'] > scenario['head'] and row['distributions']),None)
    if first_distribution is not None:
        selected.append(first_distribution)
    # Both row-model and concrete owner reopen coordinates are explicit claims;
    # neither can be silently kept when outside this scenario's actual rows.
    for owner in [document,scenario]:
        for name in ['go_row_model_reopen_after','concrete_close_reopen_after','cpp_scheduler_seed_reopen_after']:
            value=owner.get(name)
            if value is not None and value != 0:
                selected.append(coordinate(value,name))
    result=[]
    for value in selected:
        require(value == 0 or value in expected, 'declared reopen is not an actual input coordinate')
        if value not in result:
            result.append(value)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenario', default='mainnet_106')
    parser.add_argument('--boost-include', type=Path, help='Existing Boost include directory; defaults to configured /build dependency')
    args = parser.parse_args()
    require(re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_-]*',args.scenario) is not None, 'scenario basename required')
    run = Path(os.environ['RUN_ARTIFACT_DIR']).resolve()
    require(str(run) == '/home/fry/artifacts/mainnet-era-20261026-01a179de', 'same cumulative task directory required')
    directory = Path(tempfile.mkdtemp(prefix='cpp-full-stats-', dir=run))
    policy = profile()
    sources = {key: (ROOT / path).read_bytes() for key, path in PATHS.items()}
    support = SUPPORT.read_text()
    replacements = {key: strip_directives(sources[key].decode()) for key in
                    ['BLOCK_HEADER', 'STATS_HEADER', 'BLOCK_METHODS', 'STATS_METHODS']}
    tag = 'class BlockStats {\n'
    require(replacements['BLOCK_HEADER'].count(tag) == 1, 'BlockStats declaration guard')
    replacements['BLOCK_HEADER'] = replacements['BLOCK_HEADER'].replace(tag, tag +
        ' public:\n  uint64_t observation_period_ = observation_input_period;\n', 1)
    replacements['FREQUENCY_METHOD'] = method(sources['hardfork_source'].decode(),
        'uint32_t HardforksConfig::getRewardsDistributionFrequency(uint64_t block) const')
    replacements['ASPEN_METHOD'] = method(sources['hardfork_header'].decode(),
        'bool isAspenHardforkPartOne(uint64_t block_number) const')
    frequencies = policy['hardforks']['rewards_distribution_frequency']
    replacements['POLICY'] = ('HardforksConfig hf; hf.magnolia_hf.block_num=' + str(policy['hardforks']['magnolia_hf']['block_num']) +
        '; hf.aspen_hf.block_num_part_one=' + str(policy['hardforks']['aspen_hf']['block_num_part_one']) +
        '; hf.rewards_distribution_frequency={' + ','.join('{' + str(int(k)) + ',' + str(int(v)) + '}' for k,v in frequencies.items()) +
        '}; const uint32_t committee=' + str(policy['committee_size']) + ';')
    for key, value in replacements.items():
        marker = '// @' + key + '@'
        require(support.count(marker) == 1, 'support marker ' + key)
        support = support.replace(marker, value)
    require('// @' not in support, 'unfilled source marker')
    effective = directory / 'full_stats.cpp'
    effective.write_text(support)
    originals = directory / 'originals'
    originals.mkdir()
    for key, raw in sources.items():
        (originals / (key + Path(PATHS[key]).suffix)).write_bytes(raw)
    (originals / SUPPORT.name).write_bytes(SUPPORT.read_bytes())
    (originals / Path(__file__).name).write_bytes(Path(__file__).read_bytes())
    manifest_path = CORPUS / (args.scenario + '.manifest.json')
    manifest_raw = manifest_path.read_bytes()
    (directory / 'input-manifest.json').write_bytes(manifest_raw)
    manifest = json.loads(manifest_raw)
    scenario=manifest['scenario']
    require(scenario['name'] == args.scenario, 'manifest scenario binding')
    require(set(manifest['streams']) == {'public','local'}, 'both normal Go streams required')
    documents = [(label, load_document(value)) for label,value in manifest['streams'].items()]
    runs = []
    chunks = []
    for label, document in documents:
        expected, text = prepare(document, policy, scenario)
        for reopen in reopen_modes(document,scenario,expected):
            runs.append((label, reopen, document, expected))
            chunks.append(f'{reopen} {len(expected)}\n' + text)
    input_file = directory / 'input.txt'
    input_file.write_text(str(len(runs)) + '\n' + ''.join(chunks))
    # Reuse the existing configured dependency; system include paths need not
    # contain Boost. This reads CMake metadata but never configures/writes /build.
    boost_include = args.boost_include
    dependency_config = Path('/build/Boost-relwithdebinfo-x86_64-data.cmake')
    dependency_config_pin = None
    if boost_include is None and dependency_config.is_file():
        package = re.search(r'set\(boost_PACKAGE_FOLDER_RELWITHDEBINFO "([^"\n]+)"\)', dependency_config.read_text())
        require(package is not None, 'configured Boost package folder missing')
        boost_include = Path(package.group(1)) / 'include'
        retained_config = originals / dependency_config.name
        retained_config.write_bytes(dependency_config.read_bytes())
        dependency_config_pin = pin(retained_config)
    if boost_include is None:
        boost_include = Path('/usr/include')
    boost_include = boost_include.resolve()
    require((boost_include/'boost/multiprecision/cpp_int.hpp').is_file()
            and (boost_include/'boost/version.hpp').is_file(), 'existing Boost headers required; use --boost-include')
    dependency_file = directory / 'dependencies.d'
    compile_dir = directory / 'compile'
    compile_dir.mkdir()
    compile_status = capture_child(['rtk','proxy','c++','-v','-std=c++20','-O0','-I',str(boost_include),
                                  '-MMD','-MF',str(dependency_file),str(effective),'-o',str(directory/'full_stats')],
                                  ROOT, compile_dir, timeout_seconds=120)
    require(compile_status['capture_complete'] and compile_status['child_exit'] == 0, 'full aggregation C++ compilation failed')
    # Pin every non-system transitive header actually read by this compilation,
    # including Boost's fixed-width arithmetic implementation, not only its version.
    dependency_text = dependency_file.read_text().replace('\\\n', ' ')
    require(':' in dependency_text, 'compiler dependency output missing')
    dependencies = sorted(set(dependency_text.split(':',1)[1].split()))
    dependency_pins = [pin(Path(name)) for name in dependencies]
    wrapper = directory / 'run.py'
    wrapper.write_text('import subprocess\nfrom pathlib import Path\np=Path(__file__).parent\nwith (p/"input.txt").open("rb") as f:\n subprocess.run(["rtk","proxy",str(p/"full_stats")],stdin=f,check=True)\n')
    execute_dir = directory / 'execute'
    execute_dir.mkdir()
    status = capture_child(['rtk','proxy','python3',str(wrapper)], ROOT, execute_dir, timeout_seconds=120)
    require(status['capture_complete'] and status['child_exit'] == 0, 'full aggregation C++ execution failed')
    output = [json.loads(line) for line in gzip.decompress((execute_dir/'stdout.gz').read_bytes()).splitlines()]
    require(len(output) == sum(len(run[3]) for run in runs), 'complete nonzero output count')
    # Retain the actual computed vectors even when a later Go comparison fails.
    # This is an explicitly unaccepted producer record, backed by the complete
    # raw stdout capture, not a normalized schedule or a substituted oracle.
    schedule_cases=[]
    schedule_offset=0
    for index,(label,reopen,document,expected) in enumerate(runs):
        observed=output[schedule_offset:schedule_offset+len(expected)]
        schedule_offset+=len(expected)
        require([(row['run'],row['current']['number']) for row in observed]
                == [(index,p) for p in sorted(expected)], 'schedule producer output identities')
        schedule_cases.append({'pin':label,'reopen':reopen,
                               'rows':[{'period':row['current']['number'],
                                        'order':[value['number'] for value in row['scheduled']]}
                                       for row in observed]})
    observed_schedule=directory/'observed-schedules.json'
    observed_schedule.write_text(json.dumps({
        'schema':1,'status':'OBSERVED_ONLY_NOT_FULL_AGGREGATION_ACCEPTANCE',
        'scenario':scenario,'input_manifest':pin(directory/'input-manifest.json'),
        'input':pin(input_file),'effective_source':pin(effective),'wrapper':pin(wrapper),
        'sources':{PATHS[key]:pin(originals/(key+Path(PATHS[key]).suffix)) for key in sources},
        'helper':pin(originals/Path(__file__).name),'support':pin(originals/SUPPORT.name),
        'compiler_dependencies':dependency_pins,'compile_capture':str(compile_dir),
        'execute_capture':str(execute_dir),'cases':schedule_cases,
        'metadata':gzip.decompress((execute_dir/'stderr.gz').read_bytes()).decode(),
    },indent=2)+'\n')
    cursor = 0
    summaries = []
    for index,(label,reopen,document,expected) in enumerate(runs):
        observed = output[cursor:cursor+len(expected)]
        cursor += len(expected)
        periods = {row['number']: row for row in document['periods']}
        declared_reopen = coordinate(document['cpp_scheduler_seed_reopen_after'], 'declared C++ seed reopen')
        require(declared_reopen == scenario['first_retained'], 'predeclared seed lifecycle binding')
        distributions = 0
        consumed = 0
        for row,p in zip(observed,sorted(expected),strict=True):
            require(row['run'] == index and row['current']['number'] == p, 'exact output order')
            require(canonical(row['current']) == canonical(expected[p]), f'{label}/{reopen}/{p}: current BlockStats mismatch')
            actual = row['scheduled']
            if p in periods:
                target = periods[p]['distributions']
                qualified = p >= policy['hardforks']['aspen_hf']['block_num_part_one'] or reopen == declared_reopen
                if qualified:
                    require([s['number'] for s in actual] == [s['period'] for s in target], 'period vector order mismatch')
                    require([canonical(s) for s in actual] == [canonical(s['input']) for s in target], 'scheduled Go consumption mismatch')
                    require([s['number'] for s in actual] == periods[p]['distribution_order'], 'declared Go vector mismatch')
                    consumed += len(actual)
                else:
                    # These are source controls for a different process lifetime.
                    # Preserve their exact vector in observed-schedules; compare
                    # complete membership/payload, never claim Go consumed it.
                    actual_members = {s['number']: canonical(s) for s in actual}
                    wanted_members = {s['period']: canonical(s['input']) for s in target}
                    require(len(actual_members) == len(actual) and len(wanted_members) == len(target)
                            and actual_members == wanted_members, 'control complete membership/payload mismatch')
            else:
                require(not actual, 'pending facts distributed before first boundary')
            distributions += len(actual)
        summaries.append({'pin':label,'reopen':reopen,'current_rows':len(observed),'distributed_rows':distributions,
                          'go_consumed_rows':consumed,'unconsumed_control_rows':distributions-consumed,
                          'distribution_periods':[row['current']['number'] for row in observed if row['scheduled']]})
    result = {'schema':1, 'scope':'actual-coordinate BlockStats aggregation and exact Stats unordered/sorted scheduling for declared insertion and bytewise-native-key DB reload leaves',
              'scenario':scenario,
              'input_manifest':pin(directory/'input-manifest.json'), 'policy':policy,
              'declared_synthetic_timing':{'default_lambda_ms':scenario.get('lambda_ms',1000),
                                           'per_row_override':'planner_facts.lambda_ms when declared',
                                           'consensus_delay_ms':policy['hardforks']['cacti_hf']['consensus_delay']},
              'sources':{PATHS[key]:pin(originals/(key+Path(PATHS[key]).suffix)) for key in sources},
              'helper':pin(originals/Path(__file__).name), 'support':pin(originals/SUPPORT.name),
              'boost_include':str(boost_include),'dependency_configuration':dependency_config_pin,
              'compiler_dependencies':dependency_pins,'dependency_file':pin(dependency_file),
              'effective_source':pin(effective),'input':pin(input_file),'wrapper':pin(wrapper),
              'compile_capture':str(compile_dir),'execute_capture':str(execute_dir),'cases':summaries,
              'observed_schedules':pin(observed_schedule),
              'comparison':'exact numeric fields and actual pre-Aspen/post-Aspen period vector order; validator serialization map keys only',
              'db_leaf':'source storage.cpp getBlocksRewardsStats inserts bytewise-sorted native-PbftPeriod keys into a fresh unordered_map; typed values preserve its serialized BlockStats fields',
              'observation_extension':'public visibility, disabled RLP declarations, observation_period_ default member; unchanged method bodies',
              'metadata':gzip.decompress((execute_dir/'stderr.gz').read_bytes()).decode()}
    (directory/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
    print('Actual C++ full stats:',len(runs),'runs,',len(output),'current rows;',sum(s['go_consumed_rows'] for s in summaries),'Go-consumed distribution rows;',sum(s['unconsumed_control_rows'] for s in summaries),'unconsumed lifecycle control rows')
    print('Evidence:',directory/'manifest.json')
    print('Exact observed period vectors:',observed_schedule)


if __name__ == '__main__':
    main()
