#!/usr/bin/env python3
"""Read current weekly allowance from one local Codex log without changing it.

Exit 0 permits further work, 2 means the project's 20% floor has been reached,
and 3 means telemetry is missing or unreliable. Check at startup and at work
milestones; this is a decision reader, not a background process interrupter.
This reports account allowance and observed routing, never tokens or billing.
"""

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path

MIN_REMAINING_PERCENT = 20


def inspect_log(path, *, now=None, max_age_seconds=300, expected_session=None):
    """Return a quota decision and exit code for a single read-only rollout.

    Require weekly primary telemetry, matching session identity, a fresh UTC
    observation, and an unexpired reset. Reject malformed records, invalid
    percentages and future timestamps. Unknown telemetry never permits a start.
    Cached/reasoning counters and reported credit balances are not consulted.
    """
    now = now or datetime.now(timezone.utc)
    result = {'status': 'unknown', 'session_id': None, 'log': str(path),
              'model': None, 'effort': None,
              'floor_remaining_percent': MIN_REMAINING_PERCENT}
    latest = None

    def unknown(reason):
        return {**result, 'reason': reason}, 3

    try:
        with Path(path).open(encoding='utf-8') as stream:
            for line in stream:
                row = json.loads(line)
                payload = row.get('payload', {})
                if row.get('type') == 'session_meta':
                    result['session_id'] = payload.get('id') or payload.get('session_id')
                elif row.get('type') == 'turn_context':
                    result['model'] = payload.get('model')
                    result['effort'] = payload.get('effort')
                elif row.get('type') == 'event_msg' and payload.get('type') == 'token_count':
                    primary = (payload.get('rate_limits') or {}).get('primary')
                    if primary is not None:
                        latest = (row.get('timestamp'), primary)
    except (OSError, ValueError, TypeError, AttributeError):
        return unknown('Log is unreadable or contains an incomplete/malformed record; retry after it is complete.')

    if not result['session_id']:
        return unknown('Session identity is missing.')
    if expected_session and result['session_id'] != expected_session:
        return unknown('Log identity does not match the requested current session.')
    if latest is None:
        return unknown('No primary allowance snapshot is recorded.')
    timestamp, primary = latest
    try:
        observed = datetime.fromisoformat(timestamp.replace('Z', '+00:00'))
        if observed.tzinfo is None:
            return unknown('Snapshot has no time zone.')
        age = (now - observed).total_seconds()
        used = primary['used_percent']
        reset = primary['resets_at']
        if (type(used) not in (int, float) or not math.isfinite(used)
                or not 0 <= used <= 100 or type(reset) not in (int, float)
                or not math.isfinite(reset)):
            return unknown('Allowance values are invalid.')
        if primary.get('window_minutes') != 10080:
            return unknown('Primary telemetry does not describe the weekly allowance.')
        result.update({'observed_utc': observed.isoformat(), 'age_seconds': round(age, 1),
                       'used_percent': used, 'remaining_percent': round(100 - used, 6),
                       'resets_at': reset})
        if age < -2:
            return unknown('Snapshot is in the future.')
        if age > max_age_seconds:
            return unknown('Snapshot is stale; use the current session log.')
        if reset <= now.timestamp():
            return unknown('Recorded allowance window has expired.')
    except (KeyError, ValueError, TypeError, AttributeError, OverflowError):
        return unknown('Snapshot fields are missing or invalid.')

    if 100 - used <= MIN_REMAINING_PERCENT:
        return {**result, 'status': 'do_not_start',
                'reason': 'At most 20% remains; stop new work and save a resumable checkpoint.'}, 2
    return {**result, 'status': 'allow_start',
            'reason': 'More than 20% remains in this fresh recorded observation.'}, 0


def main():
    """Resolve an explicit/current-environment thread and print one JSON decision.

    Never guess the current thread from the newest log. An explicit log is useful
    for fixtures and recovery; when CODEX_THREAD_ID exists, it must match too.
    """
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--session', help='current session UUID; defaults to CODEX_THREAD_ID')
    group.add_argument('--log', type=Path, help='explicit current rollout log')
    parser.add_argument('--sessions-dir', type=Path,
                        default=Path.home() / '.codex' / 'sessions')
    args = parser.parse_args()
    current_session = os.environ.get('CODEX_THREAD_ID')
    session = args.session or current_session
    if args.session and current_session and args.session != current_session:
        result, code = {'status': 'unknown', 'reason': 'Requested session does not match CODEX_THREAD_ID.'}, 3
    elif args.log:
        result, code = inspect_log(args.log, expected_session=session)
    elif not session:
        result, code = {'status': 'unknown', 'reason': 'Supply --session or --log; CODEX_THREAD_ID is unavailable.'}, 3
    elif not all(c in '0123456789abcdef-' for c in session.lower()) or len(session) != 36:
        result, code = {'status': 'unknown', 'reason': 'Session UUID is invalid.'}, 3
    else:
        matches = list(args.sessions_dir.glob(f'**/*-{session}.jsonl'))
        if len(matches) != 1:
            result, code = {'status': 'unknown', 'reason': 'Current session log is missing or ambiguous.'}, 3
        else:
            result, code = inspect_log(matches[0], expected_session=session)
    print(json.dumps(result, sort_keys=True))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
