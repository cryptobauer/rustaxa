"""Build and verify bounded evidence references against a reviewed caller roster.

References confer no authority themselves. Every ordered occurrence is checked
and charged, including repeated bodies. No embedded path is followed recursively.
The library returns a complete result only after every body passes; publication
is exclusive and atomic. It never reads a database or grants source/fact acceptance.
"""
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile

SCHEMA = 'n4-evidence-record-v1'
WRAPPER_CAP = 131072
MANIFEST_CAP = 262144
BODY_CAP = 1048576
TOTAL_CAP = 1048576
SUMMARY_CAP = 16384
CHUNK = 65536
NODE_CAP = 8192
DEPTH_CAP = 8
BUFFER_CAP = 2097152
REPO = Path('/workspaces/rustaxa-evm')
LEDGERS = ('execution', 'validation', 'provenance')


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key')
        result[key] = value
    return result


def bounded_shape(value):
    """Reject excess decoded nodes/depth/string storage before body inspection."""
    nodes = 0
    retained = 0

    def visit(item, depth):
        nonlocal nodes, retained
        nodes += 1
        retained += 128
        if nodes > NODE_CAP or depth > DEPTH_CAP:
            raise ValueError('JSON node/depth cap')
        if isinstance(item, str):
            retained += len(item.encode('utf-8'))
        elif isinstance(item, dict):
            for key, child in item.items():
                visit(key, depth + 1)
                visit(child, depth + 1)
        elif isinstance(item, list):
            for child in item:
                visit(child, depth + 1)
        elif item is not None and type(item) not in (bool, int):
            raise ValueError('JSON scalar type')
        if retained > BUFFER_CAP - MANIFEST_CAP - CHUNK:
            raise ValueError('retained data cap')

    visit(value, 0)


def decode(raw, cap=WRAPPER_CAP):
    if len(raw) > cap:
        raise ValueError('input byte cap')
    # Bound parser allocation before json.loads creates the decoded graph.
    # Tokens are counted outside strings; escaped quotes do not end a string.
    nodes = depth = 0
    quoted = escaped = scalar = False
    for byte in raw:
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
            continue
        if byte == 34:
            quoted = True
            scalar = False
            nodes += 1
        elif byte in (91, 123):
            nodes += 1
            depth += 1
            scalar = False
            if depth > DEPTH_CAP + 1:
                raise ValueError('JSON depth cap before parse')
        elif byte in (93, 125):
            depth -= 1
            scalar = False
        elif byte in (9, 10, 13, 32, 44, 58):
            scalar = False
        elif not scalar:
            nodes += 1
            scalar = True
        if nodes > NODE_CAP:
            raise ValueError('JSON node cap before parse')
    value = json.loads(raw, object_pairs_hook=unique_object)
    bounded_shape(value)
    return value


def canonical_path(value):
    """Reject forbidden paths lexically, before touching any filesystem entry."""
    if not isinstance(value, str) or len(value.encode('utf-8')) > 4096:
        raise ValueError('path type/length')
    if not value.startswith('/') or '\\' in value or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ValueError('path syntax')
    if any(p in ('', '.', '..') for p in value.split('/')[1:]):
        raise ValueError('path alias')
    path = Path(value)
    if 'AI.md' in path.parts or path.is_relative_to(REPO/'local/evm-state-db') or path.is_relative_to(REPO/'data'):
        raise ValueError('forbidden path')
    current = Path('/')
    for part in path.parts:
        if part == '/':
            continue
        current /= part
        if current.is_symlink():
            raise ValueError('symlink path')
    return path


def check_row(row):
    if not isinstance(row, dict) or set(row) != {'role', 'path', 'bytes', 'sha256'}:
        raise ValueError('reference shape')
    if not isinstance(row['role'], str) or not row['role'] or len(row['role'].encode('utf-8')) > 256:
        raise ValueError('reference role')
    if type(row['bytes']) is not int or not 0 <= row['bytes'] <= BODY_CAP:
        raise ValueError('reference byte cap')
    sha = row['sha256']
    if not isinstance(sha, str) or len(sha) != 64 or any(c not in '0123456789abcdef' for c in sha):
        raise ValueError('reference digest')
    canonical_path(row['path'])


def authenticate(row, identities=None):
    """Stream a complete fixed body, checking pin and descriptor stability."""
    check_row(row)
    path = canonical_path(row['path'])
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError('regular body required')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        initial = os.fstat(fd)
        if (initial.st_dev, initial.st_ino) != (before.st_dev, before.st_ino) or initial.st_size != row['bytes']:
            raise ValueError('body identity/size drift')
        identity = (initial.st_dev, initial.st_ino)
        if identities is not None:
            if identity in identities and identities[identity] != str(path):
                raise ValueError('inode path alias')
            identities[identity] = str(path)
        count = 0
        digest = hashlib.sha256()
        while True:
            block = os.read(fd, min(CHUNK, row['bytes'] - count + 1))
            if not block:
                break
            count += len(block)
            if count > row['bytes']:
                raise ValueError('body grew')
            digest.update(block)
        final = os.fstat(fd)
        after = path.lstat()
        fields = ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')
        if any(getattr(initial, k) != getattr(final, k) or getattr(initial, k) != getattr(after, k) for k in fields):
            raise ValueError('body changed during read')
        if count != row['bytes'] or digest.hexdigest() != row['sha256']:
            raise ValueError('body pin drift')
        return count
    finally:
        os.close(fd)


def verify(record, destination, contract, expected, identities=None):
    """Authenticate the exact reviewed placement, returning no partial result."""
    bounded_shape(record)
    fields = {'schema', 'destination', 'ledger', 'contract', 'references', 'findings', 'accepted'}
    if not isinstance(record, dict) or set(record) != fields:
        raise ValueError('record schema')
    if record['schema'] != SCHEMA or record['destination'] != destination or record['ledger'] != 'validation' or record['accepted'] is not False:
        raise ValueError('record identity')
    canonical_path(destination)
    if record['contract'] != contract or set(contract) != {'path', 'bytes', 'sha256'}:
        raise ValueError('contract identity')
    if not isinstance(record['findings'], str) or len(record['findings'].encode('utf-8')) > SUMMARY_CAP:
        raise ValueError('findings cap')
    if not isinstance(expected, list) or not isinstance(record['references'], list) or record['references'] != expected:
        raise ValueError('ordered reference coverage')
    rows = [dict(role='trusted-contract', **contract), *expected]
    count = len(rows)
    total = 0
    pins = {}
    for row in rows:
        check_row(row)
        total += row['bytes']
        if count > 32 or total > TOTAL_CAP:
            raise ValueError('reference count/total cap')
        pin = (row['bytes'], row['sha256'])
        if row['path'] in pins and pins[row['path']] != pin:
            raise ValueError('conflicting repeated pin')
        pins[row['path']] = pin
    identities = {} if identities is None else identities
    streamed = sum(authenticate(row, identities) for row in rows)
    return dict(reference_count=count, referenced_bytes=total, streamed_bytes=streamed)


def build(destination, contract, findings, expected):
    """Build canonical bytes after the independently supplied roster passes."""
    record = dict(schema=SCHEMA, destination=destination, ledger='validation',
                  contract=contract, references=expected, findings=findings, accepted=False)
    bounded_shape(record)
    raw = (json.dumps(record, indent=2, sort_keys=True) + '\n').encode('utf-8')
    if len(raw) > WRAPPER_CAP:
        raise ValueError('wrapper byte cap')
    verify(decode(raw), destination, contract, expected)
    return raw


def publish(path, raw, destination, contract, expected):
    """Atomically publish a complete verified record, without replacement."""
    path = canonical_path(str(path))
    if path.exists() or path.is_symlink():
        raise FileExistsError(path)
    if any(path == Path(row['path']) for row in [contract, *expected]):
        raise ValueError('input/output collision')
    verify(decode(raw), destination, contract, expected)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.evidence-', delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink()
