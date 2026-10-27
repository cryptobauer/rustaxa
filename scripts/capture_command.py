#!/usr/bin/env python3
"""Capture a bounded development command before returning a small summary.

The output directory must be new. Streams are preserved as bytes, and the final
status distinguishes child exit, spawn failure, timeout and incomplete capture.
This runner does not grant task authority or replace resource admission. A limit
failure retains a prefix and is never evidence of a complete command result.
This is not a process sandbox: descendants that detach into a new session are
outside process-group cleanup. Use only development commands that do not detach.
"""

import argparse
from contextlib import ExitStack
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time


def pin(path):
    digest = hashlib.sha256()
    size = 0
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(65536), b''):
            size += len(chunk)
            digest.update(chunk)
    return {'bytes': size, 'sha256': digest.hexdigest()}


def kill_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def capture(directory, command, *, timeout_seconds, max_output_bytes, stdin_file=None):
    """Run argv without shell expansion; return (runner exit, recorded status).

    A successful runner returns the child's nonnegative exit (signals map to
    128+signal). Other codes: 124 timeout, 125 capture/output failure, 127 spawn
    failure. The JSON status is controlling when child codes overlap these codes.
    Standard input is closed unless an explicit retained stdin_file is supplied.
    Up to max_output_bytes combined stdout/stderr bytes are retained. On overflow
    the process group is killed and capture_complete is false. No original lost
    output size is invented. No existing directory or evidence is overwritten.
    """
    if not command or not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
        raise ValueError('command and finite positive timeout required')
    if type(max_output_bytes) is not int or max_output_bytes < 1:
        raise ValueError('positive output bound required')
    directory = Path(directory)
    directory.mkdir(mode=0o700)  # Refuse reuse, including existing symlinks.
    started = time.monotonic()
    deadline = started + timeout_seconds
    status = dict(schema=1, argv=command, cwd=os.getcwd(), launched=False,
                  child_exit=None, capture_complete=False, outcome='pending',
                  timeout_seconds=timeout_seconds, max_output_bytes=max_output_bytes,
                  stdin_file=str(stdin_file) if stdin_file is not None else None)
    process = None
    count = 0
    result = 125
    with (directory / 'stdout').open('xb') as stdout, \
            (directory / 'stderr').open('xb') as stderr, \
            (directory / 'runner.stderr').open('xb') as errors:
        try:
            with selectors.DefaultSelector() as selector, ExitStack() as inputs:
                child_input = (inputs.enter_context(Path(stdin_file).open('rb'))
                               if stdin_file is not None else subprocess.DEVNULL)
                try:
                    process = subprocess.Popen(command, stdin=child_input,
                                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                               start_new_session=True)
                except OSError as error:
                    status['outcome'] = 'spawn_failed'
                    errors.write((type(error).__name__ + ': ' + str(error) + '\n').encode())
                    result = 127
                else:
                    status['launched'] = True
                    selector.register(process.stdout, selectors.EVENT_READ, stdout)
                    selector.register(process.stderr, selectors.EVENT_READ, stderr)
                    while selector.get_map():
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            status['outcome'] = 'timeout'
                            result = 124
                            break
                        for key, _ in selector.select(min(remaining, 0.1)):
                            chunk = os.read(key.fd, min(65536, max_output_bytes - count + 1))
                            if not chunk:
                                selector.unregister(key.fileobj)
                                key.fileobj.close()
                                continue
                            room = max_output_bytes - count
                            key.data.write(chunk[:room])
                            count += min(len(chunk), room)
                            if len(chunk) > room:
                                status['outcome'] = 'output_limit'
                                result = 125
                                break
                        if status['outcome'] != 'pending':
                            break
                    if status['outcome'] == 'pending':
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            status['outcome'] = 'timeout'
                            result = 124
                        else:
                            try:
                                code = process.wait(timeout=remaining)
                            except subprocess.TimeoutExpired:
                                status['outcome'] = 'timeout'
                                result = 124
                            else:
                                if time.monotonic() > deadline:
                                    status['outcome'] = 'timeout'
                                    result = 124
                                else:
                                    status.update(outcome='exited', child_exit=code,
                                                  capture_complete=True)
                                    result = code if code >= 0 else 128 - code
        except Exception as error:
            status['outcome'] = 'capture_error'
            status['capture_complete'] = False
            errors.write((type(error).__name__ + ': ' + str(error) + '\n').encode())
            result = 125
        finally:
            if process is not None:
                # Also terminate descendants holding the streams after parent exit.
                kill_group(process)
                code = process.wait()
                status['child_exit'] = code
                for stream in (process.stdout, process.stderr):
                    if stream is not None and not stream.closed:
                        stream.close()
                with (directory / 'exit').open('x') as exit_file:
                    exit_file.write(str(code) + '\n')
            for stream in (stdout, stderr, errors):
                stream.flush()
                os.fsync(stream.fileno())
    status.update(runner_exit=result, elapsed_seconds=time.monotonic() - started,
                  streams={name: pin(directory / name)
                           for name in ('stdout', 'stderr', 'runner.stderr')})
    with (directory / 'status.json').open('x') as out:
        json.dump(status, out, sort_keys=True, indent=2)
        out.write('\n')
        out.flush()
        os.fsync(out.fileno())
    return result, status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True)
    parser.add_argument('--timeout-seconds', type=float, required=True)
    parser.add_argument('--max-output-bytes', type=int, required=True)
    parser.add_argument('--stdin-file', help='Retained file to feed the child; otherwise stdin is closed')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    code, status = capture(args.output_dir, command,
                           timeout_seconds=args.timeout_seconds,
                           max_output_bytes=args.max_output_bytes,
                           stdin_file=args.stdin_file)
    print(json.dumps({'output_dir': str(Path(args.output_dir).absolute()),
                      'outcome': status['outcome'], 'child_exit': status['child_exit'],
                      'capture_complete': status['capture_complete'], 'runner_exit': code}))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
