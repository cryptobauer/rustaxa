#!/usr/bin/env python3
"""Inventory usage-record coverage in explicit local logs; never print bodies.

This is not a token audit. It does not discover threads, total tokens, reconcile
counters, attribute models or authorize further work. Exit 2 reports incomplete
input, unidentified response records or conflicting duplicates; exit 0 only means
the inventory completed without those issues.
"""

import argparse
from collections import Counter
import json
from pathlib import Path


def inventory(path):
    """Scan one log read-only, retaining usage signatures rather than transcripts."""
    path = Path(path)
    types = Counter()
    events = Counter()
    locations = Counter()
    seen = {}
    conflicts = set()
    session = None
    malformed = unidentified = duplicates = snapshots = last_snapshots = 0
    read_error = None
    try:
        with path.open(encoding='utf-8') as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                except ValueError:
                    malformed += 1
                    continue
                if not isinstance(row, dict):
                    malformed += 1
                    continue
                kind = row.get('type')
                payload = row.get('payload', {})
                if not isinstance(kind, str) or not isinstance(payload, dict):
                    malformed += 1
                    continue
                types[kind] += 1
                if kind == 'session_meta':
                    session = payload.get('id') or payload.get('session_id')
                nested = payload.get('type') if kind == 'event_msg' else None
                if isinstance(nested, str):
                    events[nested] += 1
                if kind == 'token_usage_record' or nested == 'token_usage_record':
                    locations['top_level' if kind == 'token_usage_record' else 'nested_event'] += 1
                    thread = payload.get('thread_id') or payload.get('session_id') or session
                    response = payload.get('response_id')
                    usage = payload.get('usage')
                    if (not isinstance(thread, str) or not thread.strip()
                            or not isinstance(response, str) or not response.strip()
                            or not isinstance(usage, dict)):
                        unidentified += 1
                        continue
                    key = (thread, response)
                    signature = json.dumps(usage, sort_keys=True, separators=(',', ':'))
                    if key in seen:
                        duplicates += 1
                        if signature != seen[key]:
                            conflicts.add(key)
                    else:
                        seen[key] = signature
                if kind == 'token_count' or nested == 'token_count':
                    info = payload.get('info')
                    if isinstance(info, dict):
                        snapshots += isinstance(info.get('total_token_usage'), dict)
                        last_snapshots += isinstance(info.get('last_token_usage'), dict)
    except (OSError, UnicodeError) as exc:
        read_error = type(exc).__name__

    response_records = sum(locations.values())
    if response_records:
        recommendation = 'inspect_per_response_records_and_resolve_gaps'
    elif snapshots:
        recommendation = 'consider_labelled_cumulative_delta_fallback'
    else:
        recommendation = 'no_supported_usage_records_found'
    return {
        'log': str(path),
        'record_types': dict(types),
        'nested_event_types': dict(events),
        'per_response_locations': dict(locations),
        'per_response_records': response_records,
        'unique_response_keys': len(seen),
        'duplicate_response_records': duplicates,
        'conflicting_duplicate_keys': len(conflicts),
        'unidentified_response_records': unidentified,
        'cumulative_snapshots': snapshots,
        'last_usage_snapshots': last_snapshots,
        'malformed_records': malformed,
        'read_error': read_error,
        'scan_complete': read_error is None and malformed == 0,
        'recommendation': recommendation,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--log', type=Path, action='append', required=True,
                        help='Explicit local log; repeat for additional known logs.')
    args = parser.parse_args(argv)
    reports = [inventory(path) for path in args.log]
    print(json.dumps({'logs': reports, 'scope': 'schema inventory only; not a token audit'}, indent=2))
    return 2 if any(not r['scan_complete'] or r['unidentified_response_records']
                    or r['conflicting_duplicate_keys'] for r in reports) else 0


if __name__ == '__main__':
    raise SystemExit(main())
