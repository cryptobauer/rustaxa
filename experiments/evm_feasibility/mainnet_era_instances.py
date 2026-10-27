#!/usr/bin/env python3
"""Check shared observations and report exact lifecycle disagreements.

Local transaction observers produce extra intermediate physical trie history.
This check does not compare or normalize physical histories across pins. Every
document is authenticated separately, and same-pin reproduction remains exact
outside the reviewed reward-map segment. Only the local observer field is
excluded from each execution record's public semantic projection. Cleanup
transcripts stay distinct and require actual Rust qualification in each mode.
"""
import argparse
import copy
import json
import os
from pathlib import Path
from mainnet_era_reference import CORPUS
from mainnet_era_estimate_reference import load
from mainnet_era_compare import comparison


def verify(manifest):
    documents = {label: load(pin) for label, pin in manifest['streams'].items()}
    views = {label: comparison(value) for label, value in documents.items()}
    public, local = views['public'], views['local']
    observations = {tx['hash']: tx['observer'] for period in local['periods'] for tx in period['transactions']}
    def project(transactions):
        for tx in transactions:
            observer = tx.pop('observer', None)
            if observer is None or not observer.get('root') or not observer.get('catalog') or observer != observations[tx['hash']]:
                raise ValueError('missing/inconsistent local transaction observation')
    for row in local['pending_reward_facts']:
        project(row.get('transactions', []))
    for period in local['periods']:
        project(period.get('reward_input', {}).get('transactions', []))
        for row in period['distributions']:
            project(row['input'].get('transactions', []))
    for field in ['configuration', 'inputs', 'head', 'profile', 'scenario', 'pending_reward_facts']:
        if public[field] != local[field]:
            raise ValueError('per-pin input disagreement: ' + field)
    for field in ['period', 'root', 'history', 'native_catalog', 'ordered_raw_writes']:
        if public['seed'][field] != local['seed'][field]:
            raise ValueError('per-pin seed disagreement: ' + field)
    if len(public['periods']) != len(local['periods']) or not public['periods']:
        raise ValueError('period coverage disagreement')
    def catalog(left, right, accounts):
        left, right = copy.deepcopy(left), copy.deepcopy(right)
        a = {row['address']: row for row in left.pop('accounts')}
        b = {row['address']: row for row in right.pop('accounts')}
        complete = {row['address']: row for row in accounts}
        if left != right or not a.keys() <= b.keys() or any(b[k] != v for k, v in a.items()):
            raise ValueError('native catalog overlap disagreement')
        if any(complete.get(k) != b[k] for k in b.keys() - a.keys()):
            raise ValueError('observer-only account differs from complete final observation')
    transactions = 0
    differences = []
    for expected, observed in zip(public['periods'], local['periods'], strict=True):
        actual = copy.deepcopy(observed)
        expected = copy.deepcopy(expected)
        if expected.pop('period_catalog_identities') is not None or not isinstance(actual.pop('period_catalog_identities'), dict):
            raise ValueError('per-pin observer mode disagreement')
        project(actual['transactions'])
        transactions += len(actual['transactions'])
        catalog(expected['final'].pop('native_catalog'), actual['final'].pop('native_catalog'), actual['final']['accounts'])
        public_end = expected['ordered_raw_writes'].pop('end_block')
        local_end = actual['ordered_raw_writes'].pop('end_block')
        if public_end != local_end:
            differences.append({'period': expected['number'], 'public': public_end, 'local': local_end})
        if expected != actual:
            raise ValueError('per-pin execution disagreement at ' + str(expected['number']))
    if public['api_cases'] != local['api_cases']:
        raise ValueError('per-pin committed API disagreement')
    count = len(public['api_cases'])
    public_forks = manifest['pre_reward_forks']['public']
    local_forks = manifest['pre_reward_forks']['local']
    if public_forks.keys() != local_forks.keys():
        raise ValueError('fork coverage disagreement')
    for period in public_forks:
        public_fork, local_fork = load(public_forks[period]), load(local_forks[period])
        catalog(public_fork['pre_reward'].pop('native_catalog'), local_fork['pre_reward'].pop('native_catalog'), local_fork['pre_reward']['accounts'])
        for field in ['seed_identity', 'prefix_period_identities', 'pre_reward', 'api_cases']:
            if public_fork[field] != local_fork[field]:
                raise ValueError('per-pin private API/prefix disagreement: ' + period + '/' + field)
        count += len(public_fork['api_cases'])
    if [row['period'] for row in differences] != [25706906, 25706907] or differences[0]['public'] or differences[1]['local'] or differences[0]['local'] != differences[1]['public']:
        raise ValueError('unreviewed lifecycle transcript disagreement')
    artifact = Path(os.environ['RUN_ARTIFACT_DIR'])
    controls = [json.loads(path.read_bytes()) for path in artifact.glob('batched-control-*.json')]
    matching = [value for value in controls if value['public_stream'] == manifest['streams']['public']]
    if not matching:
        raise ValueError('missing source-bound local batched control')
    result = {'schema': 1, 'scope': 'shared observations and source-reviewed distinct lifecycles; Rust per-mode execution remains separately required',
              'streams': manifest['streams'], 'periods': len(public['periods']), 'transactions': transactions,
              'api_cases': count, 'exact_lifecycle_disagreements': differences, 'batched_control': matching[-1]}
    raw = (json.dumps(result, sort_keys=True, separators=(',', ':')) + '\n').encode()
    import hashlib
    target = artifact / ('two-pin-lifecycle-observations-' + hashlib.sha256(raw).hexdigest() + '.json')
    with target.open('xb') as output: output.write(raw)
    print('Shared observations checked:', len(public['periods']), 'periods,', transactions, 'signed transactions,', count,
          'API cases; preserved', len(differences), 'exact lifecycle differences:', target)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenario', required=True)
    args = parser.parse_args()
    verify(json.loads((CORPUS / (args.scenario + '.manifest.json')).read_bytes()))
