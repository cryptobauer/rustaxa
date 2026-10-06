#!/usr/bin/env python3
"""Read current weekly allowance from one local Codex log without changing it.

Exit 0 permits further work, 2 means the configured remaining-allowance floor
has been reached,
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

MIN_REMAINING_PERCENT = 80
DEFAULT_BUDGET_CONSUME_PERCENT = 10
WEEK_SECONDS = 7 * 24 * 60 * 60


def _valid_floor(value):
    return (type(value) in (int, float) and math.isfinite(value)
            and 0 <= value <= 100)


def inspect_log(path, *, now=None, max_age_seconds=300, expected_session=None,
                min_remaining_percent=MIN_REMAINING_PERCENT):
    """Return a quota decision and exit code for a single read-only rollout.

    Require weekly primary telemetry, matching session identity, a fresh UTC
    observation, and an unexpired reset. Reject malformed records, invalid
    percentages and future timestamps. Unknown telemetry never permits a start.
    Cached/reasoning counters and reported credit balances are not consulted.
    """
    valid_floor = _valid_floor(min_remaining_percent)
    now = now or datetime.now(timezone.utc)
    result = {'status': 'unknown', 'session_id': None, 'log': str(path),
              'model': None, 'effort': None,
              'floor_remaining_percent': min_remaining_percent if valid_floor else None}
    latest = None
    window_peaks = {}

    def unknown(reason):
        return {**result, 'reason': reason}, 3

    if not valid_floor:
        return unknown('Minimum remaining percentage must be a finite number from 0 to 100; booleans are invalid.')

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
                        used = primary.get('used_percent')
                        reset = primary.get('resets_at')
                        if (primary.get('window_minutes') == 10080
                                and _valid_floor(used)
                                and type(reset) in (int, float) and math.isfinite(reset)):
                            window_peaks[reset] = max(window_peaks.get(reset, used), used)
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
        if used < window_peaks.get(reset, used):
            return unknown('Cumulative allowance usage decreased within the recorded weekly window.')
    except (KeyError, ValueError, TypeError, AttributeError, OverflowError):
        return unknown('Snapshot fields are missing or invalid.')

    if 100 - used <= min_remaining_percent:
        return {**result, 'status': 'do_not_start',
                'reason': (f'At most {min_remaining_percent:g}% remains; stop new work '
                          'and save a resumable checkpoint.')}, 2
    return {**result, 'status': 'allow_start',
            'reason': (f'More than {min_remaining_percent:g}% remains in this fresh '
                       'recorded observation.')}, 0


def _valid_consume(value):
    return _valid_floor(value)


def _valid_observation(observation, *, now, max_age_seconds=300):
    if (not isinstance(observation, dict)
            or observation.get('status') not in ('allow_start', 'do_not_start')
            or not isinstance(observation.get('session_id'), str)
            or not observation.get('session_id')):
        return False
    used = observation.get('used_percent')
    remaining = observation.get('remaining_percent')
    reset = observation.get('resets_at')
    observed_text = observation.get('observed_utc')
    if (not _valid_floor(used) or not _valid_floor(remaining)
            or type(reset) not in (int, float) or not math.isfinite(reset)
            or not isinstance(observed_text, str)):
        return False
    try:
        observed = datetime.fromisoformat(observed_text.replace('Z', '+00:00'))
        if observed.tzinfo is None:
            return False
        age = (now - observed).total_seconds()
    except (ValueError, TypeError, OverflowError):
        return False
    return (math.isclose(remaining, 100 - used, rel_tol=0, abs_tol=1e-6)
            and -2 <= age <= max_age_seconds and reset > now.timestamp()
            and 0 <= reset - observed.timestamp() <= WEEK_SECONDS + 2)


def _budget_unknown(reason, *, path, floor=None):
    return {'status': 'unknown', 'budget_file': str(path),
            'floor_remaining_percent': floor, 'reason': reason}, 3


def _validate_budget(state, path, *, now):
    """Validate a saved baseline without changing or replacing it."""
    if not isinstance(state, dict) or state.get('schema_version') != 1:
        return None, 'Budget file has an unsupported or malformed schema.'
    session_id = state.get('session_id')
    used = state.get('baseline_used_percent')
    remaining = state.get('baseline_remaining_percent')
    observed_text = state.get('observed_utc')
    reset = state.get('resets_at')
    consume = state.get('consume_percent')
    floor = state.get('floor_remaining_percent')
    if (not isinstance(session_id, str) or not session_id
            or not isinstance(observed_text, str)
            or not _valid_floor(used) or not _valid_floor(remaining)
            or not _valid_consume(consume) or not _valid_floor(floor)
            or type(reset) not in (int, float) or not math.isfinite(reset)):
        return None, 'Budget file contains invalid baseline values.'
    try:
        observed = datetime.fromisoformat(observed_text.replace('Z', '+00:00'))
    except (ValueError, TypeError):
        return None, 'Budget file baseline timestamp is invalid.'
    if observed.tzinfo is None:
        return None, 'Budget file baseline timestamp has no time zone.'
    observed_epoch = observed.timestamp()
    if observed_epoch > now.timestamp() + 2:
        return None, 'Budget file baseline timestamp is in the future.'
    if reset <= observed_epoch or reset - observed_epoch > WEEK_SECONDS + 2:
        return None, 'Budget file reset time does not match a weekly baseline window.'
    expected_remaining = 100 - used
    expected_floor = max(0, expected_remaining - consume)
    if (not math.isclose(remaining, expected_remaining, rel_tol=0, abs_tol=1e-6)
            or not math.isclose(floor, expected_floor, rel_tol=0, abs_tol=1e-6)):
        return None, 'Budget file baseline fields are contradictory.'
    return {'schema_version': 1, 'session_id': session_id,
            'baseline_used_percent': used,
            'baseline_remaining_percent': remaining,
            'observed_utc': observed.isoformat(), 'resets_at': reset,
            'consume_percent': consume, 'floor_remaining_percent': floor}, None


def _budget_decision(state, observation, path, *, initialized=False, now=None):
    """Apply the saved allowance budget to one already validated observation."""
    now = now or datetime.now(timezone.utc)
    floor = state['floor_remaining_percent']
    current_session = observation.get('session_id')
    same_session = current_session == state['session_id']
    used_value = observation.get('used_percent')
    consumed = (used_value - state['baseline_used_percent']
                if same_session and _valid_floor(used_value) else None)
    baseline = {'budget_file': str(path),
                'baseline_session_id': state['session_id'],
                'baseline_used_percent': state['baseline_used_percent'],
                'baseline_remaining_percent': state['baseline_remaining_percent'],
                'baseline_observed_utc': state['observed_utc'],
                'baseline_resets_at': state['resets_at'],
                'consume_percent': state['consume_percent'],
                'consumed_percent': round(consumed, 6) if consumed is not None else None,
                'consumed_percentage_points': round(consumed, 6) if consumed is not None else None,
                'floor_remaining_percent': floor,
                'budget_initialized': initialized}
    if current_session != state['session_id']:
        return {**baseline, 'status': 'unknown',
                'reason': 'Current telemetry does not match the lead baseline session.'}, 3
    if not math.isclose(observation.get('resets_at', float('nan')),
                        state['resets_at'], rel_tol=0, abs_tol=1e-6):
        return {**baseline, 'status': 'unknown',
                'reason': 'Current telemetry uses a different allowance reset window.'}, 3
    current_used = observation['used_percent']
    if current_used < state['baseline_used_percent']:
        return {**baseline, 'status': 'unknown',
                'reason': 'Current allowance usage is below the saved baseline; refusing to rebase.'}, 3
    result = {**observation, **baseline}
    if observation['remaining_percent'] <= floor:
        return {**result, 'status': 'do_not_start',
                'reason': (f'Allowance budget reached its {floor:g}% remaining floor; '
                           'stop new work and save a resumable checkpoint.')}, 2
    points_left = round(observation['remaining_percent'] - floor, 6)
    point_word = 'point' if points_left == 1 else 'points'
    verb = 'remains' if points_left == 1 else 'remain'
    return {**result, 'status': 'allow_start',
            'reason': (f'{points_left:g} percentage {point_word} {verb} in the saved '
                       f'{state["consume_percent"]:g}-point budget.')}, 0


def initialize_budget(path, observation, *, consume_percent=DEFAULT_BUDGET_CONSUME_PERCENT,
                      now=None):
    """Save one fresh current-session observation, refusing to overwrite state."""
    now = now or datetime.now(timezone.utc)
    if not _valid_consume(consume_percent):
        return _budget_unknown('Consume percentage must be a finite number from 0 to 100.',
                               path=path)
    if not _valid_observation(observation, now=now):
        return _budget_unknown('Cannot initialize from unknown or missing current telemetry.',
                               path=path)
    path = Path(path)
    if path.exists():
        return _budget_unknown('Budget file already exists; initialization never overwrites it.',
                               path=path)
    remaining = observation['remaining_percent']
    floor = max(0, remaining - consume_percent)
    state = {'schema_version': 1, 'session_id': observation['session_id'],
             'baseline_used_percent': observation['used_percent'],
             'baseline_remaining_percent': remaining,
             'observed_utc': observation['observed_utc'],
             'resets_at': observation['resets_at'],
             'consume_percent': consume_percent,
             'floor_remaining_percent': floor}
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('x', encoding='utf-8') as stream:
            json.dump(state, stream, sort_keys=True)
            stream.write('\n')
    except FileExistsError:
        return _budget_unknown('Budget file already exists; initialization never overwrites it.',
                               path=path, floor=floor)
    except OSError:
        return _budget_unknown('Budget file could not be created.', path=path, floor=floor)
    return _budget_decision(state, observation, path, initialized=True, now=now)


def inspect_budget(path, observation, *, now=None):
    """Read saved allowance baseline and compare a fresh observation."""
    path = Path(path)
    try:
        with path.open(encoding='utf-8') as stream:
            state = json.load(stream)
    except (OSError, ValueError, TypeError):
        return _budget_unknown('Budget file is missing, unreadable, or malformed.', path=path)
    state, error = _validate_budget(state, path, now=now or datetime.now(timezone.utc))
    if error:
        return _budget_unknown(error, path=path)
    if observation.get('status') == 'unknown' or observation.get('session_id') is None:
        result = {**observation, 'budget_file': str(path),
                  'status': 'unknown',
                  'baseline_session_id': state['session_id'],
                  'baseline_used_percent': state['baseline_used_percent'],
                  'baseline_remaining_percent': state['baseline_remaining_percent'],
                  'baseline_observed_utc': state['observed_utc'],
                  'baseline_resets_at': state['resets_at'],
                  'consume_percent': state['consume_percent'],
                  'consumed_percent': None,
                  'consumed_percentage_points': None,
                  'floor_remaining_percent': state['floor_remaining_percent'],
                  'budget_initialized': False,
                  'reason': (observation.get('reason')
                             or 'Current telemetry is unknown; saved baseline was not changed.')}
        return result, 3
    if not _valid_observation(observation, now=now or datetime.now(timezone.utc)):
        result = {**observation, 'budget_file': str(path),
                  'status': 'unknown',
                  'baseline_session_id': state['session_id'],
                  'baseline_used_percent': state['baseline_used_percent'],
                  'baseline_remaining_percent': state['baseline_remaining_percent'],
                  'baseline_observed_utc': state['observed_utc'],
                  'baseline_resets_at': state['resets_at'],
                  'consume_percent': state['consume_percent'],
                  'consumed_percent': None,
                  'consumed_percentage_points': None,
                  'floor_remaining_percent': state['floor_remaining_percent'],
                  'budget_initialized': False,
                  'reason': 'Current telemetry is unknown or stale; saved baseline was not changed.'}
        return result, 3
    return _budget_decision(state, observation, path, now=now)


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
    parser.add_argument('--min-remaining-percent', type=float, default=None,
                        help='minimum weekly allowance that must remain (default: 80)')
    parser.add_argument('--budget-file', type=Path,
                        help='persistent file for one lead-session allowance budget')
    parser.add_argument('--init-budget', action='store_true',
                        help='create the budget file from a fresh current-session observation')
    parser.add_argument('--consume-percent', type=float, default=None,
                        help='percentage points to budget during initialization (default: 10)')
    args = parser.parse_args()
    budget_mode = args.budget_file is not None
    if args.init_budget and not budget_mode:
        result, code = {'status': 'unknown',
                        'reason': '--init-budget requires --budget-file.'}, 3
        print(json.dumps(result, sort_keys=True))
        return code
    if args.consume_percent is not None and not args.init_budget:
        result, code = {'status': 'unknown',
                        'reason': '--consume-percent is only valid with --init-budget.'}, 3
        print(json.dumps(result, sort_keys=True))
        return code
    if budget_mode and args.min_remaining_percent is not None:
        result, code = {'status': 'unknown', 'budget_file': str(args.budget_file),
                        'reason': '--budget-file cannot be combined with --min-remaining-percent.'}, 3
        print(json.dumps(result, sort_keys=True))
        return code
    floor = (args.min_remaining_percent if args.min_remaining_percent is not None
             else MIN_REMAINING_PERCENT)
    consume = (args.consume_percent if args.consume_percent is not None
               else DEFAULT_BUDGET_CONSUME_PERCENT)
    if not _valid_floor(floor):
        result, code = {'status': 'unknown', 'floor_remaining_percent': None,
                        'reason': 'Minimum remaining percentage must be a finite number from 0 to 100.'}, 3
        print(json.dumps(result, sort_keys=True))
        return code
    if args.init_budget and not _valid_consume(consume):
        result, code = _budget_unknown(
            'Consume percentage must be a finite number from 0 to 100.',
            path=args.budget_file)
        print(json.dumps(result, sort_keys=True))
        return code
    if args.init_budget and args.budget_file.exists():
        result, code = _budget_unknown(
            'Budget file already exists; initialization never overwrites it.',
            path=args.budget_file)
        print(json.dumps(result, sort_keys=True))
        return code
    base = {'floor_remaining_percent': floor}
    current_session = os.environ.get('CODEX_THREAD_ID')
    session = args.session or current_session
    if budget_mode and not session:
        result, code = {'status': 'unknown', 'budget_file': str(args.budget_file),
                        'reason': 'Budget mode requires the current lead session identity.'}, 3
    elif args.session and current_session and args.session != current_session:
        result, code = {**base, 'status': 'unknown', 'reason': 'Requested session does not match CODEX_THREAD_ID.'}, 3
    elif args.log:
        result, code = inspect_log(args.log, expected_session=session,
                                   min_remaining_percent=0 if budget_mode else floor)
    elif not session:
        result, code = {**base, 'status': 'unknown', 'reason': 'Supply --session or --log; CODEX_THREAD_ID is unavailable.'}, 3
    elif not all(c in '0123456789abcdef-' for c in session.lower()) or len(session) != 36:
        result, code = {**base, 'status': 'unknown', 'reason': 'Session UUID is invalid.'}, 3
    else:
        matches = list(args.sessions_dir.glob(f'**/*-{session}.jsonl'))
        if len(matches) != 1:
            result, code = {**base, 'status': 'unknown', 'reason': 'Current session log is missing or ambiguous.'}, 3
        else:
            result, code = inspect_log(matches[0], expected_session=session,
                                       min_remaining_percent=0 if budget_mode else floor)
    if budget_mode:
        if args.init_budget and code == 3:
            result, code = _budget_unknown(result.get('reason', 'Current telemetry is unknown.'),
                                           path=args.budget_file)
        elif args.init_budget:
            result, code = initialize_budget(args.budget_file, result,
                                             consume_percent=consume)
        else:
            result, code = inspect_budget(args.budget_file, result)
    print(json.dumps(result, sort_keys=True))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
