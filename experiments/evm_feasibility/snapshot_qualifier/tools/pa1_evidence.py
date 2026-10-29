"""PA1 metadata port: bounded consumed streams and durable finite admission.

This port supplies no profile, command, READY or capture authority. Its caller
must supply the independently frozen ordered closure manifest and semantic
predicates. Existing protocol checks are not replaced by byte authentication.
Only full invocation completion can publish a receipt. All counters are burned
before work, survive failure/restart, and cannot be restored from a receipt.
"""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import stat
import tempfile
import time

import evidence_records as records

POLICY_SHA = '119aee4b697f4d133edbbad0488dbad59b202c1e7bb65595682ab1bb081cfbd4'
LEDGER_SHA = '8b75bcc803170795b9ef9f42993e832fb9e82a6129162f73bc8390b60cb18362'
SETUP_POLICY_SHA = '0013c1c8ef1ed4b61b69cde7a50bdb4c9112347923eb4713cf126e5ae6e8571e'
SETUP_LEDGER_SHA = '8182f960fe702d321caf5c8c6ba168db2feb8b16b7879b328cea51c88ad0ee04'
SETUP_PUBLICATION_SHA = 'f3a82b786b09997f4af634424eabaae862cc32601e059977c149a42d08d37c0b'
SETUP_ENVELOPE = 11423963
EVENT_MAXIMA = (8, 4, 4)
NONBINARY = 31457280
SOURCE = 4194304
BINARY_FILE = 220200960
BINARY_PAIR = 440401920
AGGREGATE = 471859200
LIVE = 5242880
SEMANTIC = 2097152
RECEIPT = 131072
INVOCATIONS = 48
UNITS = 98304
INVOCATION_UNITS = 2048
NONBINARY_LIFETIME = 31980788016+(SETUP_ENVELOPE-1048576)*8*48
BINARY_LIFETIME = 449080998528
SECONDS_LIFETIME = 14400
SECONDS = 300
CHUNK = 65536
COUNTER_IO_RESERVE = 32768
ROUTES = {'archive': 1, 'pre-gate': 1, 'command': 2, 'capture': 3,
          'lead-replay': 1, 'independent-replay': 1, 'reverse': 5, 'seal': 2}


def canonical(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':'),
                       ensure_ascii=True, allow_nan=False)+'\n').encode('ascii')


def digest(value):
    # Manifest binding must not create an uncharged complete JSON string and
    # byte copy alongside the decoded manifest. Callers bound individual scalars.
    result = hashlib.sha256()
    encoder = json.JSONEncoder(sort_keys=True, separators=(',', ':'),
                               ensure_ascii=True, allow_nan=False)
    for piece in encoder.iterencode(value):
        result.update(piece.encode('ascii'))
    result.update(b'\n')
    return result.hexdigest()


def frozen_manifest_digest(unit_ids, unit_wire):
    """Hash the original canonical (IDs, declarations) without expanding it."""
    result = hashlib.sha256(b'[')
    encoder = json.JSONEncoder(sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False)
    for piece in encoder.iterencode(unit_ids):
        result.update(piece.encode('ascii'))
    result.update(b',[')
    for index, raw in enumerate(unit_wire):
        if index:
            result.update(b',')
        if type(raw) is not bytes or not raw.endswith(b'\n'):
            raise ValueError('PA1 canonical unit wire')
        result.update(memoryview(raw)[:-1])
    result.update(b']]\n')
    return result.hexdigest()


def _wire_nodes(raw):
    """Conservative lexical node count before allocating a metadata graph."""
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
        elif byte in (93, 125):
            depth -= 1
            scalar = False
        elif byte in (9, 10, 13, 32, 44, 58):
            scalar = False
        elif not scalar:
            nodes += 1
            scalar = True
        if nodes > 8192 or depth > 9:
            raise ValueError('PA1 metadata nodes/depth')
    return nodes


def integer(value, maximum, label):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError(label)
    return value


def sha(value):
    if (type(value) is not str or len(value) != 64
            or any(c not in '0123456789abcdef' for c in value)):
        raise ValueError('PA1 digest')
    return value


def fixed579_physical_pin(occurrence, pin):
    """Resolve only V5 child64/55's reviewed immutable instruction successor.

    The original logical pin stays in declarations, frames and containing
    records. This exact alias changes only the consumed physical stream path;
    it supplies no verdict, fallback discovery or permission to alter semantics.
    """
    logical = dict(path='/workspaces/rustaxa-evm/doc/evm_research/n4_evidence_record_successor.md',
                   bytes=4446, sha256='00f7c7d32e283acd5fc535c94e634b377f7211897d765ef86e75717ef02cbe4e')
    if type(occurrence) is int and occurrence == 579 and pin == logical:
        return dict(logical, path='/home/fry/artifacts/vrf-contract-preparation-20261017/predecessors/doc/evm_research/n4_evidence_record_successor.md')
    return pin


def _atomic(path, raw):
    """Replace only internal counter state under its held lock; fsync both levels."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.pa1-', delete=False) as out:
            temporary = Path(out.name)
            out.write(raw)
            out.flush()
            os.fsync(out.fileno())
        os.replace(temporary, path)
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


class Counters:
    """Exclusive, restart-safe admission for one fixed run/manifest binding.

    initialize() creates the directory exclusively. A partial initialization or
    missing state is an error on resume; it never creates a replacement budget.
    The caller must freeze this directory identity in its accepted run manifest.
    The filesystem and run-manifest owner remain trusted for counter integrity;
    this is not protection against an administrator rolling back the whole run.
    """
    FIELDS = {'schema', 'binding', 'history', 'limits', 'invocations', 'units', 'nonbinary', 'binary',
              'seconds', 'failures', 'generation', 'slots', 'routes', 'pending'}

    @classmethod
    def initialize(cls, directory, binding, *, consumed_events, event_history, trace_io=False):
        if type(trace_io) is not bool:
            raise ValueError('PA1 counter IO trace mode')
        directory = records.canonical_path(str(directory))
        sha(binding)
        sha(event_history)
        if type(consumed_events) is not tuple or len(consumed_events) != 3:
            raise ValueError('PA1 known consumed event history')
        for slot, count in enumerate(consumed_events):
            integer(count, EVENT_MAXIMA[slot], 'PA1 consumed event count')
        directory.mkdir()  # No exist_ok: restart must use the existing state.
        lock = directory/'lock'
        fd = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        os.close(fd)
        value = dict(schema=2, binding=binding, history=event_history,
                     limits=cls.limit_identity(), invocations=0, units=0,
                     nonbinary=0, binary=0, seconds=0, failures=list(consumed_events), slots=[0, 0, 0], generation=0,
                     routes=[dict.fromkeys(ROUTES, 0) for _ in range(3)], pending=None)
        raw = canonical(value)
        if len(raw) > 4096:
            raise ValueError('PA1 counter state write cap')
        _atomic(directory/'state.json', raw)
        result = cls(directory, binding, event_history, trace_io=trace_io)
        if trace_io:
            result._io_trace.insert(0, dict(operation='write', bytes=len(raw),
                attempted_bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest(),
                generation=0, authenticated=True))
        return result

    @staticmethod
    def limit_identity():
        """Bind durable debits to the approved effective limits and publication.

        Old state cannot acquire a different allowance through a code update.
        Missing or changed identities stop resume; they never initialize state.
        """
        return digest((POLICY_SHA, LEDGER_SHA, SETUP_POLICY_SHA,
                       SETUP_LEDGER_SHA, SETUP_PUBLICATION_SHA,
                       SETUP_ENVELOPE, NONBINARY_LIFETIME, BINARY_LIFETIME,
                       INVOCATIONS, UNITS, SECONDS_LIFETIME, EVENT_MAXIMA, NONBINARY,
                       SOURCE, BINARY_FILE, BINARY_PAIR, AGGREGATE, SEMANTIC,
                       LIVE, RECEIPT, INVOCATION_UNITS, SECONDS, ROUTES, 8, 2048,
                       COUNTER_IO_RESERVE))

    def __init__(self, directory, binding, event_history, *, trace_io=False):
        self.directory = records.canonical_path(str(directory))
        self.binding = sha(binding)
        self.event_history = sha(event_history)
        if type(trace_io) is not bool:
            raise ValueError('PA1 counter IO trace mode')
        self.trace_io = trace_io
        self._io_trace = []
        self._last_io_trace = ()
        self._io_scope = False
        # Validate immediately; no lazy missing-as-zero behavior.
        with self._locked():
            self._read()

    def validate_invocation(self, context, slot):
        """Base profile has no additional namespace beyond its fixed slots."""
        return None

    def _io_check(self, attempted):
        if self.trace_io and (len(self._io_trace) >= 6 or
                sum(row['attempted_bytes'] for row in self._io_trace)+attempted > COUNTER_IO_RESERVE):
            raise ValueError('PA1 counter IO scope overflow')

    def begin_io_scope(self):
        """Keep constructor/initialization operations in the bounded cold scope.

        No trace is discarded here. Complete-fit callers must enable tracing
        before construction and explicitly reserve this scope in framing.
        """
        if not self.trace_io or self._io_scope:
            raise ValueError('PA1 counter IO scope not available')
        self._io_check(0)
        self._io_scope = True

    def io_summary(self):
        rows = self._last_io_trace if self._last_io_trace else tuple(self._io_trace)
        return dict(physical_operations=len(rows),
                    attempted_bytes=sum(row['attempted_bytes'] for row in rows),
                    reserved_bytes=COUNTER_IO_RESERVE, versions=rows)

    def _read_state_raw(self):
        """Read at most4097 full state bytes; subclasses keep all validation.

        The default uses the existing exact canonical pathname. The fixed40
        fit clone overrides only this file operation with its locked dir FD.
        """
        path = records.canonical_path(str(self.directory/'state.json'))
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError('PA1 regular counter state')
            raw = os.read(fd, 4097)
        finally:
            os.close(fd)
        return raw

    def _replace_state_raw(self, raw):
        """Atomically replace full state bytes using the existing write contract."""
        _atomic(self.directory/'state.json', raw)

    def _write(self, value):
        raw = canonical(value)
        if len(raw) > 4096:
            raise ValueError('PA1 counter state write cap')
        self._io_check(len(raw))
        if self.trace_io:
            self._io_trace.append(dict(operation='write', bytes=len(raw), attempted_bytes=len(raw),
                sha256=hashlib.sha256(raw).hexdigest(), generation=value['generation'], authenticated=False))
        self._replace_state_raw(raw)
        if self.trace_io:
            self._io_trace[-1]['authenticated'] = True

    @contextmanager
    def _locked(self):
        path = records.canonical_path(str(self.directory/'lock'))
        fd = os.open(path, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError('PA1 regular counter lock')
            fcntl.flock(fd, fcntl.LOCK_EX)
            yield
        finally:
            os.close(fd)

    def _read(self):
        self._io_check(4097)
        raw = self._read_state_raw()
        if self.trace_io:
            self._io_trace.append(dict(operation='read', bytes=len(raw), attempted_bytes=4097,
                sha256=hashlib.sha256(raw).hexdigest(), generation=None, authenticated=False))
        value = records.decode(raw, 4096)
        if (type(value) is not dict or set(value) != self.FIELDS
                or type(value['schema']) is not int or value['schema'] != 2
                or value['limits'] != self.limit_identity()
                or value['history'] != self.event_history
                or value['binding'] != self.binding):
            raise ValueError('PA1 counter identity')
        for key, cap in [('invocations', INVOCATIONS), ('units', UNITS),
                         ('nonbinary', NONBINARY_LIFETIME), ('binary', BINARY_LIFETIME),
                         ('seconds', SECONDS_LIFETIME), ('generation', 200000)]:
            integer(value[key], cap, 'PA1 counter '+key)
        if type(value['failures']) is not list or len(value['failures']) != 3:
            raise ValueError('PA1 failure slots')
        for slot, count in enumerate(value['failures']):
            integer(count, EVENT_MAXIMA[slot], 'PA1 failure count')
        if type(value['slots']) is not list or len(value['slots']) != 3:
            raise ValueError('PA1 invocation slots')
        for count in value['slots']:
            integer(count, 16, 'PA1 slot invocations')
        if sum(value['slots']) != value['invocations']:
            raise ValueError('PA1 slot accounting')
        if type(value['routes']) is not list or len(value['routes']) != 3:
            raise ValueError('PA1 route counters')
        for slot, routes in enumerate(value['routes']):
            if type(routes) is not dict or set(routes) != set(ROUTES):
                raise ValueError('PA1 route identity')
            for route, cap in ROUTES.items():
                integer(routes[route], cap, 'PA1 route count')
            if sum(routes.values()) != value['slots'][slot]:
                raise ValueError('PA1 route accounting')
        pending = value['pending']
        if pending is not None:
            if type(pending) is not list or len(pending) != 3:
                raise ValueError('PA1 unknown pending invocation')
            integer(pending[0], 2, 'PA1 pending slot')
            if pending[1] not in ROUTES:
                raise ValueError('PA1 pending route')
            integer(pending[2], value['generation'], 'PA1 pending generation')
        if self.trace_io:
            self._io_trace[-1].update(generation=value['generation'], authenticated=True)
        return value

    def finish(self, generation, failed=False):
        if type(failed) is not bool:
            raise ValueError('PA1 completion status')
        if self.trace_io and not self._io_scope:
            raise ValueError('PA1 active counter IO scope required')
        with self._locked():
            value = self._read()
            pending = value['pending']
            if pending is None or pending[2] != generation:
                raise ValueError('PA1 unknown invocation completion')
            if failed:
                slot = pending[0]
                value['failures'][slot] = integer(value['failures'][slot]+1, EVENT_MAXIMA[slot],
                                                   'PA1 failure exhaustion')
            value['pending'] = None
            value['generation'] += 1
            self._write(value)
            if self.trace_io:
                self._last_io_trace = tuple(self._io_trace)
                self._io_trace = []
                self._io_scope = False

    def admit(self, *, invocation=0, units=0, nonbinary=0, binary=0, seconds=0,
              failure_slot=None, slot=None, route=None):
        """Persist the complete worst-case debit before any associated work."""
        limits = dict(invocations=INVOCATIONS, units=UNITS,
                      nonbinary=NONBINARY_LIFETIME, binary=BINARY_LIFETIME,
                      seconds=SECONDS_LIFETIME)
        increments = dict(invocations=invocation, units=units, nonbinary=nonbinary,
                          binary=binary, seconds=seconds)
        if self.trace_io and invocation:
            if not self._io_scope:
                raise ValueError('PA1 counter IO scope required before invocation')
            increments['nonbinary'] += COUNTER_IO_RESERVE
        for key, increment in increments.items():
            integer(increment, limits[key], 'PA1 debit '+key)
        if invocation:
            integer(invocation, 1, 'PA1 single invocation debit')
            integer(slot, 2, 'PA1 invocation slot')
            if route not in ROUTES:
                raise ValueError('PA1 invocation route')
        elif slot is not None or route is not None:
            raise ValueError('PA1 unexpected slot')
        if failure_slot is not None:
            integer(failure_slot, 2, 'PA1 failure slot')
            if invocation:
                raise ValueError('PA1 invocation cannot also consume a failure event')
        with self._locked():
            value = self._read()
            if value['pending'] is not None:
                raise ValueError('PA1 unresolved prior invocation; no restart reset')
            if invocation:
                if value['failures'][slot] >= EVENT_MAXIMA[slot]:
                    raise ValueError('PA1 no reserved failure outcome')
                value['routes'][slot][route] = integer(value['routes'][slot][route]+1, ROUTES[route], 'PA1 route exhaustion')
                value['slots'][slot] = integer(value['slots'][slot]+1, 16, 'PA1 slot exhaustion')
            for key, increment in increments.items():
                value[key] = integer(value[key]+increment, limits[key], 'PA1 lifetime '+key)
            if failure_slot is not None:
                value['failures'][failure_slot] = integer(
                    value['failures'][failure_slot]+1, EVENT_MAXIMA[failure_slot], 'PA1 failure exhaustion')
            value['generation'] = integer(value['generation']+1, 200000, 'PA1 generation')
            if invocation:
                value['pending'] = [slot, route, value['generation']]
            self._write(value)
            return value


class Invocation:
    """One pre-admitted complete manifest; no receipt until all units succeed.

    Ordered unit IDs and their read reservations are supplied by a frozen plan.
    An invocation is never resumed. Restart consumes another admitted invocation
    and keeps all earlier debits. Unknown remaining allowance fails in Counters.
    """
    def __init__(self, counters, context, slot, unit_ids, declarations, nonbinary, binary, clock=time.monotonic,
                 *, serialized=False, live_bytes=0, release_completed_wires=False):
        if (type(release_completed_wires) is not bool
                or (release_completed_wires and serialized is not True)):
            raise ValueError('PA1 completed wire release requires serialized admission')
        self.release_completed_wires=release_completed_wires
        required = {'run', 'manifest', 'source', 'verifier', 'executables', 'phase', 'route',
                    'policy', 'ledger', 'setup_policy', 'setup_ledger', 'setup_publication',
                    'event_history'}
        if (type(context) is not dict or set(context) != required
                or context['run'] != counters.binding or context['policy'] != POLICY_SHA
                or context['ledger'] != LEDGER_SHA
                or context['setup_policy'] != SETUP_POLICY_SHA
                or context['setup_ledger'] != SETUP_LEDGER_SHA
                or context['setup_publication'] != SETUP_PUBLICATION_SHA):
            raise ValueError('PA1 invocation context')
        if context['event_history'] != counters.event_history:
            raise ValueError('PA1 invocation event history')
        for key in ('run', 'manifest', 'source', 'verifier'):
            sha(context[key])
        if type(context['executables']) is not tuple or len(context['executables']) != 2:
            raise ValueError('PA1 invocation executables')
        for value in context['executables']:
            sha(value)
        for key in ('phase', 'route'):
            if type(context[key]) is not str or not context[key] or len(context[key]) > 64:
                raise ValueError('PA1 invocation phase/route')
        counters.validate_invocation(context, slot)
        binding = digest(context)
        if (type(unit_ids) is not tuple or not 0 < len(unit_ids) <= INVOCATION_UNITS
                or any(type(x) is not str or not x or len(x.encode()) > 128 for x in unit_ids)
                or len(set(unit_ids)) != len(unit_ids)):
            raise ValueError('PA1 ordered unit identities')
        if type(serialized) is not bool:
            raise ValueError('PA1 metadata representation')
        if serialized:
            self._initialize_wire(counters, context, binding, slot, unit_ids,
                                  declarations, nonbinary, binary, clock, live_bytes)
            return
        if (type(declarations) is not tuple or len(declarations) != len(unit_ids)
                or any(type(rows) is not tuple or not rows for rows in declarations)):
            raise ValueError('PA1 nonempty exact closures')
        # Two uint16 locations per logical occurrence. This lossless lookup
        # avoids a second heap graph beside the complete immutable declarations.
        identities = bytearray(b'\xff'*8192)
        manifest_charge = 128
        for unit_index, (identity, rows) in enumerate(zip(unit_ids, declarations)):
            manifest_charge += 256+len(identity.encode())+128
            for row_index, row in enumerate(rows):
                if type(row) is not tuple or len(row) != 4:
                    raise ValueError('PA1 frozen declaration')
                occurrence, pin, kind, mode = row
                integer(occurrence, 2047, 'PA1 declaration occurrence')
                if type(pin) is not tuple or len(pin) != 3:
                    raise ValueError('PA1 immutable pin')
                path, size, checksum = pin
                records.canonical_path(path)
                integer(size, BINARY_FILE if kind == 'binary' else NONBINARY, 'PA1 declaration size')
                sha(checksum)
                if (type(kind) is not str or type(mode) is not str or len(mode.encode()) > 512
                        or kind not in ('source', 'evidence', 'binary') or (mode not in ('bytes', 'json') and not mode.startswith(('history:', 'member:', 'slice:')))):
                    raise ValueError('PA1 declared kind/mode')
                manifest_charge += 1280+sum(len(v.encode()) for v in (path, checksum, kind, mode))
                # Store only bounded integer locations. A second pin graph in
                # the lookup table would duplicate the entire manifest lifetime.
                offset = occurrence*4
                prior_unit = int.from_bytes(identities[offset:offset+2], 'little')
                if prior_unit != 65535:
                    prior_row = int.from_bytes(identities[offset+2:offset+4], 'little')
                    prior = declarations[prior_unit][prior_row]
                    if (pin, kind, mode) != prior[1:]:
                        raise ValueError('PA1 conflicting occurrence identity')
                else:
                    integer(row_index, 65534, 'PA1 declaration row position')
                    identities[offset:offset+2] = unit_index.to_bytes(2, 'little')
                    identities[offset+2:offset+4] = row_index.to_bytes(2, 'little')
                if manifest_charge+8320+CHUNK+262144 > SEMANTIC:
                    raise ValueError('PA1 frozen manifest allocation')
        del identities
        if digest((unit_ids, declarations)) != context['manifest']:
            raise ValueError('PA1 exact manifest binding')
        # Retained frozen tuples are the complete manifest, not caller paths or
        # mutable pin dictionaries. Count each repeated semantic occurrence.
        if manifest_charge+CHUNK+262144 > SEMANTIC:
            raise ValueError('PA1 frozen manifest allocation')
        # Count serialized output before producing it. The transfer peak keeps
        # input graph, all completed byte buffers, tuple headers, charge scalars
        # and the largest encoder string/byte pair live at the same time.
        encoder = json.JSONEncoder(sort_keys=True, separators=(',', ':'),
                                   ensure_ascii=True, allow_nan=False)
        wire_total = wire_max = 0
        for rows in declarations:
            width = 1+sum(len(piece) for piece in encoder.iterencode(rows))
            wire_total += width
            wire_max = max(wire_max, width)
            if width > records.MANIFEST_CAP:
                raise ValueError('PA1 unit metadata wire cap')
        transfer_charge = (manifest_charge+wire_total+256*len(unit_ids)
                           +2*wire_max+CHUNK+262144+12288)
        if transfer_charge > SEMANTIC:
            raise ValueError('PA1 manifest transfer allocation')
        integer(nonbinary, NONBINARY_LIFETIME, 'PA1 nonbinary reservation')
        integer(binary, BINARY_LIFETIME, 'PA1 binary reservation')
        state = counters.admit(invocation=1, units=len(unit_ids), nonbinary=nonbinary,
                               binary=binary, seconds=SECONDS, slot=slot, route=context['route'])
        self.counters = counters
        self.caller_charge = 0
        self.metadata_operations = bytearray(len(unit_ids))
        self.consumed_events = tuple(state['failures'])
        self.finalized = False
        self.generation = state['generation']
        self.clock = clock
        self.started = clock()
        self.binding = binding
        self.context = dict(context)
        # Keep only serialized per-unit metadata after admission. Decoding the
        # whole manifest beside a full authority graph would exceed the cap.
        self.declaration_charges = tuple(sum(1280+len(row[1][0].encode())+len(row[1][2])+len(row[2])+len(row[3]) for row in rows) for rows in declarations)
        self.declarations = tuple(canonical(rows) for rows in declarations)
        self.manifest_charge = (128+sum(128+len(raw) for raw in self.declarations)
                                +sum(256+len(x.encode()) for x in unit_ids)
                                +128*(len(self.declaration_charges)+1))
        self.generation = state['generation']
        self.ids = unit_ids
        self.next = 0
        self.nonbinary_remaining = nonbinary
        self.binary_remaining = binary
        self.operations = bytearray(4096)
        self.verdicts = []
        self.verdict_charge = 0
        self.failed = False
        self.closed = False
        self.active = None
        if self.metadata_charge()+CHUNK+262144 > SEMANTIC:
            self.fail()
            raise ValueError('PA1 manifest allocation')

    def _initialize_wire(self, counters, context, binding, slot, ids, wires,
                         nonbinary, binary, clock, live_bytes):
        """Transfer bounded canonical units without a complete decoded graph.

        The caller releases all other references to decoded declarations before
        entry and charges its retained graph. Repeated identities are compared
        as complete tuples; the packed table contains locations, not verdicts.
        """
        integer(live_bytes, SEMANTIC, 'PA1 caller metadata allocation')
        if (type(wires) is not tuple or len(wires) != len(ids)
                or any(type(raw) is not bytes or not 0 < len(raw) <= records.MANIFEST_CAP for raw in wires)):
            raise ValueError('PA1 serialized exact closures')
        base = (128+sum(128+len(raw) for raw in wires)
                +sum(256+len(identity.encode()) for identity in ids)+128*(len(ids)+1))
        fixed = base+12288+24704+128*len(ids)+live_bytes+CHUNK+262144
        if counters.trace_io:
            fixed += COUNTER_IO_RESERVE
        if fixed > SEMANTIC:
            raise ValueError('PA1 serialized manifest allocation')
        shapes = tuple(_wire_nodes(raw)*160+2*len(raw) for raw in wires)
        # Exact byte locations, not cached semantic verdicts. Each complete
        # unit is still decoded and checked canonically below. A repeated row
        # compares the original immutable canonical bytes without reparsing
        # its entire containing unit for every source in a large constructor.
        locations = bytearray(b'\xff'*24576)
        metadata_operations = bytearray(len(ids))
        charges = []
        for unit_index, raw in enumerate(wires):
            if fixed+shapes[unit_index]+2*len(raw) > SEMANTIC:
                raise ValueError('PA1 serialized unit pre-allocation')
            metadata_operations[unit_index] += 1
            if metadata_operations[unit_index] > 7:
                raise ValueError('PA1 metadata parse reserve exhausted')
            rows = records.decode(raw, records.MANIFEST_CAP)
            if type(rows) is not list or not rows or canonical(rows) != raw:
                raise ValueError('PA1 canonical nonempty unit metadata')
            charge = 0
            row_start = 1
            for row_index, row in enumerate(rows):
                if type(row) is not list or len(row) != 4:
                    raise ValueError('PA1 frozen declaration')
                occurrence, pin, kind, mode = row
                integer(occurrence, 2047, 'PA1 declaration occurrence')
                if type(pin) is not list or len(pin) != 3:
                    raise ValueError('PA1 immutable pin')
                path, size, checksum = pin
                records.canonical_path(path)
                integer(size, BINARY_FILE if kind == 'binary' else NONBINARY, 'PA1 declaration size')
                sha(checksum)
                if (type(kind) is not str or type(mode) is not str or len(mode.encode()) > 512
                        or kind not in ('source', 'evidence', 'binary')
                        or (mode not in ('bytes', 'json') and not mode.startswith(('history:', 'member:', 'slice:')))):
                    raise ValueError('PA1 declared kind/mode')
                charge += 1280+sum(len(v.encode()) for v in (path, checksum, kind, mode))
                row_wire=canonical(row)[:-1]
                offset = occurrence*12
                previous = int.from_bytes(locations[offset:offset+2], 'little')
                if previous != 65535:
                    position=int.from_bytes(locations[offset+2:offset+6],'little')
                    length=int.from_bytes(locations[offset+6:offset+10],'little')
                    if (length!=len(row_wire)
                            or memoryview(wires[previous])[position:position+length]!=row_wire):
                        raise ValueError('PA1 conflicting occurrence identity')
                else:
                    integer(row_index, 65534, 'PA1 declaration row position')
                    locations[offset:offset+2]=unit_index.to_bytes(2,'little')
                    locations[offset+2:offset+6]=row_start.to_bytes(4,'little')
                    locations[offset+6:offset+10]=len(row_wire).to_bytes(4,'little')
                row_start+=len(row_wire)+1
                del row_wire
            charges.append(charge)
            del row, rows
        del shapes, locations
        if frozen_manifest_digest(ids, wires) != context['manifest']:
            raise ValueError('PA1 exact manifest binding')
        integer(nonbinary, NONBINARY_LIFETIME, 'PA1 nonbinary reservation')
        integer(binary, BINARY_LIFETIME, 'PA1 binary reservation')
        state = counters.admit(invocation=1, units=len(ids), nonbinary=nonbinary,
                               binary=binary, seconds=SECONDS, slot=slot, route=context['route'])
        self.counters = counters
        self.caller_charge = live_bytes
        self.metadata_operations = metadata_operations
        self.consumed_events = tuple(state['failures'])
        self.finalized = False
        self.generation = state['generation']
        self.clock = clock
        self.started = clock()
        self.binding = binding
        self.context = dict(context)
        self.declaration_charges = tuple(charges)
        self.declarations = wires
        self.manifest_charge = base
        self.ids = ids
        self.next = 0
        self.nonbinary_remaining = nonbinary
        self.binary_remaining = binary
        self.operations = bytearray(4096)
        for index, count in enumerate(metadata_operations):
            self.operations[2048+index] = count
        self.verdicts = []
        self.verdict_charge = 0
        self.failed = False
        self.closed = False
        self.active = None

    def metadata_charge(self):
        # Charge semantic occurrences even when Python objects share strings.
        return (12288+self.manifest_charge+self.verdict_charge+self.caller_charge+len(self.metadata_operations)+128
                +(32768 if self.counters.trace_io else 0))

    def fail(self):
        self.failed = True
        if not self.finalized:
            self.counters.finish(self.generation, failed=True)
            self.finalized = True

    def check(self):
        if self.failed or self.closed:
            raise ValueError('PA1 invocation failed/closed/deadline')
        if self.clock()-self.started >= SECONDS:
            self.fail()
            raise ValueError('PA1 invocation failed/closed/deadline')

    @contextmanager
    def unit(self, identity):
        self.check()
        if self.active or self.next >= len(self.ids) or identity != self.ids[self.next]:
            self.fail()
            raise ValueError('PA1 unit order or overlap')
        raw = self.declarations[self.next]
        # This is a distinct bounded in-memory parse occurrence, charged once
        # at this unit boundary. It never grants a file/body authentication.
        if (self.metadata_charge()+self.declaration_charges[self.next]*2
                +len(raw)*2+CHUNK+262144 > SEMANTIC):
            self.fail()
            raise ValueError('PA1 unit metadata preparse allocation')
        try:
            if self.operations[2048+self.next] >= 8:
                raise ValueError('PA1 ninth metadata parse operation')
            values = records.decode(raw, records.MANIFEST_CAP)
            declarations = tuple((row[0], tuple(row[1]), row[2], row[3]) for row in values)
            unit = Unit(self, declarations)
            unit._charge(len(raw), 'evidence')
            self.operations[2048+self.next] += 1
            self.metadata_operations[self.next] += 1
        except BaseException:
            self.fail()
            raise
        self.active = unit
        del values, declarations, raw
        try:
            yield unit
            self.check()
            if unit.position != len(unit.declarations):
                raise ValueError('PA1 incomplete unit closure')
            self.verdicts.append(identity)
            self.verdict_charge += 128+len(identity.encode())
            self.next += 1
        except BaseException:
            self.fail()
            raise
        finally:
            self.active = None
            # A stale handle must not keep a decoded closure or consumed body
            # alive beside the next unit's graph.
            unit._retained = None
            unit.declarations = ()
            unit.binary_sizes.clear()
        if self.release_completed_wires:
            # Only the completed immutable bytes leave the working set. IDs,
            # verdicts, operation counts, debit totals and the frozen full
            # context binding remain. No later body read is discharged.
            # Replacement tuples contain references to the existing bytes;
            # reserve three headers and twice the pointer-slot storage before
            # allocating them. No second wire encoding is constructed.
            if (self.metadata_charge()+384+32*len(self.declarations)
                    +CHUNK+262144>SEMANTIC):
                self.fail()
                raise ValueError('PA1 completed wire release allocation')
            released=len(self.declarations[self.next-1])
            self.declarations=(b'',)*self.next+self.declarations[self.next:]
            self.manifest_charge-=released

    def complete(self):
        """Build one acyclic audit receipt, never an authority/cache receipt.

        The run binding must cover the exact manifest, sources, verifier, limits,
        executables, phase and route. The frozen context is checked against
        that binding; this prevents a caller substituting metadata at completion.
        """
        self.check()
        if digest(self.context) != self.binding:
            self.fail()
            raise ValueError('PA1 completion binding')
        if self.active or self.next != len(self.ids):
            self.fail()
            raise ValueError('PA1 incomplete invocation')
        # Reserve encoder output and decoded receipt graph before serialization.
        if self.metadata_charge()+RECEIPT*3+262144+CHUNK > SEMANTIC:
            self.fail()
            raise ValueError('PA1 receipt serialization allocation')
        result = dict(schema='pa1-completion-v1', binding=self.binding,
                      context=self.context, generation=self.generation,
                      unit_verdicts=self.verdicts, accepted=False, authority=False)
        encoder = json.JSONEncoder(sort_keys=True, separators=(',', ':'),
                                   ensure_ascii=True, allow_nan=False)
        width = 1
        for piece in encoder.iterencode(result):
            width += len(piece)
            if width > RECEIPT:
                self.fail()
                raise ValueError('PA1 completion receipt overflow before allocation')
        raw = canonical(result)
        if len(raw) > RECEIPT:
            self.fail()
            raise ValueError('PA1 completion receipt overflow')
        self.counters.finish(self.generation)
        self.finalized = True
        self.closed = True
        return raw


class Unit:
    """Authenticate consumed bytes inside one indivisible semantic closure.

    consume() returns only authenticated raw bytes for bounded metadata, or no
    body for streamed binary/log authentication. JSON decoding must use the
    returned bytes. Arbitrary path rereads do not inherit this authentication.
    A caller supplies all original semantic predicates before unit exit.
    """
    def __init__(self, invocation, declarations):
        self.invocation = invocation
        self.declarations = declarations
        self.position = 0
        self.declaration_charge = sum(1280+len(row[1][0].encode())+len(row[1][2])+len(row[2])+len(row[3]) for row in declarations)
        self.nonbinary = self.source = self.binary = 0
        self.binary_sizes = {}
        self._retained = None
        self.last_file_identity = None

    def retained_charge(self):
        return 0 if self._retained is None else 384+2*len(self._retained[1])

    def consume(self, occurrence, pin, kind='evidence', retain=False, live_bytes=0, _decoding=False,
                _history=None, _slice=None):
        owner = self.invocation
        try:
            owner.check()
            if owner.active is not self or type(occurrence) is not int or not 0 <= occurrence < 2048:
                raise ValueError('PA1 occurrence identity')
            if kind not in ('evidence', 'source', 'binary') or type(retain) is not bool:
                raise ValueError('PA1 body kind')
            if type(pin) is not dict or set(pin) != {'path', 'bytes', 'sha256'}:
                raise ValueError('PA1 pin shape')
            mode = ('slice:'+str(_slice[0])+':'+str(_slice[1]) if _slice is not None else
                    'history:'+str(_history[0]) if _history is not None
                    else 'json' if _decoding else 'bytes')
            expected = (occurrence, (pin['path'], pin['bytes'], pin['sha256']), kind, mode)
            if (self.position >= len(self.declarations)
                    or self.declarations[self.position] != expected):
                raise ValueError('PA1 frozen read order/identity/kind/mode')
            size = integer(pin['bytes'], BINARY_FILE if kind == 'binary' else NONBINARY, 'PA1 body size')
            retained_size = size
            if _slice is not None:
                if type(_slice) is not tuple or len(_slice) != 2 or not retain or not _decoding or kind == 'binary':
                    raise ValueError('PA1 exact projection request')
                start = integer(_slice[0], size, 'PA1 projection start')
                retained_size = integer(_slice[1], records.WRAPPER_CAP, 'PA1 projection width')
                if not retained_size or start+retained_size > size or _history is not None:
                    raise ValueError('PA1 projection body bounds')
            sha(pin['sha256'])
            physical = fixed579_physical_pin(occurrence, pin) if mode == 'bytes' and kind == 'evidence' else pin
            path = records.canonical_path(physical['path'])
            # Count logical occurrences across units, not per physical path.
            count = owner.operations[occurrence]+1
            if count > 8:
                raise ValueError('PA1 ninth occurrence operation')
            owner.operations[occurrence] = count
            integer(live_bytes, SEMANTIC, 'PA1 live graph')
            if live_bytes+owner.metadata_charge()+self.declaration_charge+self.retained_charge()+CHUNK+262144 > SEMANTIC:
                raise ValueError('PA1 live allocation')
            if retain and (kind == 'binary' or retained_size > records.WRAPPER_CAP
                           or live_bytes+retained_size*2+owner.metadata_charge()+self.declaration_charge+self.retained_charge()+CHUNK+262144 > SEMANTIC):
                raise ValueError('PA1 retained buffer allocation')
            if kind == 'binary':
                self.binary_sizes[occurrence] = size
                if len(self.binary_sizes) > 2 or sum(self.binary_sizes.values()) > BINARY_PAIR:
                    raise ValueError('PA1 binary pair cap')
            # Debit known body bytes before reading. EOF sentinel is a charged
            # attempted read if nonempty; a changed body never publishes a result.
            self._charge(size, kind)
            # Reserve the full attempted EOF-sentinel byte even on exact EOF.
            # A changed body cannot read one unreserved byte past its full pin.
            self._charge(1, kind)
            before = path.stat(follow_symlinks=False)
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            chunks = [] if retain else None
            h = hashlib.sha256()
            consumed = 0
            try:
                opened = os.fstat(fd)
                if (not stat.S_ISREG(opened.st_mode)
                        or (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino)):
                    raise ValueError('PA1 regular body')
                def take(width):
                    nonlocal consumed
                    owner.check()
                    if type(width) is not int or not 0 <= width <= CHUNK or consumed+width > size:
                        raise ValueError('PA1 bounded stream width')
                    block = os.read(fd, width)
                    if len(block) != width:
                        raise ValueError('PA1 truncated body')
                    h.update(block)
                    consumed += len(block)
                    return block
                if _history is not None:
                    self._history_frames(take, *_history)
                    if consumed != size:
                        raise ValueError('PA1 history exact EOF')
                else:
                    while consumed < size:
                        block_start = consumed
                        block = take(min(CHUNK, size-consumed))
                        if retain:
                            if _slice is None:
                                chunks.append(block)
                            else:
                                low = max(start, block_start)-block_start
                                high = min(start+retained_size, consumed)-block_start
                                if high > low:
                                    chunks.append(block[low:high])
                extra = os.read(fd, 1)
                if extra:
                    raise ValueError('PA1 body trailing bytes')
                if h.hexdigest() != pin['sha256']:
                    raise ValueError('PA1 consumed stream hash')
                self.last_file_identity = (opened.st_dev, opened.st_ino)
            finally:
                os.close(fd)
            owner.check()
            if not _decoding:
                self.position += 1 if _history is None else 1+_history[2]
            raw = b''.join(chunks) if retain else None
            if _slice is not None and len(raw) != retained_size:
                raise ValueError('PA1 projection exact width')
            if retain and not _decoding:
                self._retained = (occurrence, raw)
            return raw
        except BaseException:
            owner.fail()
            raise

    def history(self, occurrence, pin, part_index, first, count, live_bytes=0):
        """Verify one complete N42HIS2 closure and each frozen member occurrence.

        Uses the same consumed stream for container and member authentication.
        Member verdicts remain unpublished until the full container EOF/hash and
        every required frame have passed. External/repeated children are separate
        declared reads. No embedded member path is followed.
        """
        integer(part_index, 65535, 'PA1 history part')
        integer(first, 65535, 'PA1 history first')
        integer(count, 2047, 'PA1 history count')
        if count == 0 or self.position+count >= len(self.declarations):
            self.invocation.fail()
            raise ValueError('PA1 complete history member declarations')
        owner = self.invocation
        owner.check()
        if owner.active is not self or type(occurrence) is not int or not 0 <= occurrence < 2048:
            owner.fail()
            raise ValueError('PA1 history occurrence identity')
        # Header/frame decoding is a parse of the container in addition to its
        # consumed-stream read. The subsequent consume() debits that read.
        if owner.operations[occurrence] >= 7:
            owner.fail()
            raise ValueError('PA1 ninth history operation')
        owner.operations[occurrence] += 1
        return self.consume(occurrence, pin, live_bytes=live_bytes,
                            _history=(part_index, first, count))

    def _history_frames(self, take, part_index, first, count):
        header = take(12)
        if (header[:8] != b'N42HIS2\x00' or int.from_bytes(header[8:10], 'little') != part_index
                or int.from_bytes(header[10:12], 'little') != count):
            raise ValueError('PA1 history header')
        for offset in range(count):
            occurrence, pin, kind, mode = self.declarations[self.position+1+offset]
            if kind != 'evidence' or not mode.startswith('member:'):
                raise ValueError('PA1 frozen history member')
            role = mode[len('member:'):].encode('utf-8')
            path, size, checksum = pin
            location = path.encode('utf-8')
            frame = take(48)
            if (int.from_bytes(frame[:4], 'little') != first+offset
                    or int.from_bytes(frame[4:6], 'little') != len(role)
                    or int.from_bytes(frame[6:8], 'little') != len(location)
                    or int.from_bytes(frame[8:16], 'little') != size
                    or frame[16:] != bytes.fromhex(checksum)):
                raise ValueError('PA1 history frame order/role/pin')
            if take(len(role)) != role or take(len(location)) != location:
                raise ValueError('PA1 history role/path')
            operations = self.invocation.operations[occurrence]+1
            if operations > 8:
                raise ValueError('PA1 ninth member operation')
            self.invocation.operations[occurrence] = operations
            h = hashlib.sha256()
            remaining = size
            while remaining:
                block = take(min(CHUNK, remaining))
                h.update(block)
                remaining -= len(block)
            if h.hexdigest() != checksum:
                raise ValueError('PA1 history member consumed hash')

    def decode(self, occurrence, pin, live_bytes=0):
        """Decode only the retained bytes just authenticated, with preparse admission.

        Read and parse are separately counted invocation-wide. The original
        strict evidence decoder retains its scalar/node/depth/duplicate rules.
        Operational finite-float records need their separately reviewed port.
        """
        owner = self.invocation
        raw = self.consume(occurrence, pin, retain=True, live_bytes=live_bytes, _decoding=True)
        value = self._parse_consumed(occurrence, raw, live_bytes)
        self.position += 1
        return value

    def decode_slice(self, occurrence, pin, start, width, checksum, live_bytes=0):
        """Decode a frozen bounded slice after authenticating its entire body.

        The declaration freezes the whole containing pin and exact slice range.
        Every body byte and EOF is charged and hashed from the consumed stream;
        only the selected bytes are retained. A changed byte outside the slice
        rejects before any result. Slice hash and strict JSON checks are extra
        predicates, not replacements for the complete containing-body checks.
        The read and parse each debit the original occurrence's eight-op cap.
        """
        try:
            sha(checksum)
            raw = self.consume(occurrence, pin, retain=True, live_bytes=live_bytes,
                               _decoding=True, _slice=(start, width))
            if hashlib.sha256(raw).hexdigest() != checksum:
                raise ValueError('PA1 consumed projection hash')
            value = self._parse_consumed(occurrence, raw, live_bytes)
            self.position += 1
            return value
        except BaseException:
            self.invocation.fail()
            raise

    def decode_consumed(self, occurrence, raw, live_bytes=0):
        """Parse the exact retained object returned by this unit's latest read.

        A copied, stale or independently authenticated stream cannot inherit its
        verdict. Parsing debits the same logical occurrence and clears retained
        ownership; caller-owned decoded graphs remain explicit live inputs.
        """
        self._take_consumed(occurrence, raw)
        return self._parse_consumed(occurrence, raw, live_bytes)

    def decode_control_consumed(self, occurrence, raw, role, live_bytes=0):
        """Use the approved finite-float decoder only for the frozen saved gate.

        Protocol/report/manifest bodies retain strict integer-only decoding.
        The selected40 port freezes this one operational unit and its path.
        """
        owner = self.invocation
        if (role != 'selected40-saved-result' or owner.next >= len(owner.ids)
                or owner.ids[owner.next] != 'selected40-semantic:2'
                or occurrence != 684 or self.position != 1
                or len(self.declarations) != 2
                or self.declarations[0][0] != 684
                or self.declarations[0][2:] != ('evidence', 'bytes')
                or self.declarations[1][0] != 685
                or self.declarations[1][2:] != ('evidence', 'bytes')):
            owner.fail()
            raise ValueError('PA1 exact operational role')
        self._take_consumed(occurrence, raw)
        return self._parse_consumed(occurrence, raw, live_bytes, operational=True)

    def _take_consumed(self, occurrence, raw):
        owner = self.invocation
        owner.check()
        if (owner.active is not self or type(occurrence) is not int or self._retained is None
                or self._retained[0] != occurrence or self._retained[1] is not raw):
            owner.fail()
            raise ValueError('PA1 same consumed buffer identity')
        self._retained = None

    def _parse_consumed(self, occurrence, raw, live_bytes, operational=False):
        owner = self.invocation
        try:
            integer(live_bytes, SEMANTIC, 'PA1 live graph')
            count = owner.operations[occurrence]+1
            if count > 8:
                raise ValueError('PA1 ninth occurrence operation')
            owner.operations[occurrence] = count
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
                elif byte in (93, 125):
                    depth -= 1
                    scalar = False
                elif byte in (9, 10, 13, 32, 44, 58):
                    scalar = False
                elif not scalar:
                    nodes += 1
                    scalar = True
                if nodes > 8192 or depth > (10 if operational else 9):
                    raise ValueError('PA1 JSON preparse nodes/depth')
            if (owner.metadata_charge()+self.declaration_charge+self.retained_charge()+live_bytes+len(raw)*2+nodes*(160 if operational else 128)
                    +CHUNK+262144 > SEMANTIC):
                raise ValueError('PA1 JSON preparse allocation')
            if operational:
                if len(raw) > 16384:
                    raise ValueError('PA1 control record cap')
                def invalid(value):
                    raise ValueError('PA1 nonfinite control scalar')
                value = json.loads(raw, object_pairs_hook=records.unique_object, parse_constant=invalid)
                count = charge = 0
                def visit(item, level):
                    nonlocal count, charge
                    count += 1
                    charge += 128
                    if count > 8192 or level > 9:
                        raise ValueError('PA1 control nodes/depth')
                    if isinstance(item, str):
                        charge += len(item.encode())
                    elif isinstance(item, dict):
                        for k, v in item.items():
                            visit(k, level+1)
                            visit(v, level+1)
                    elif isinstance(item, list):
                        for v in item:
                            visit(v, level+1)
                    elif type(item) is float and not math.isfinite(item):
                        raise ValueError('PA1 nonfinite control scalar')
                visit(value, 0)
                if (owner.metadata_charge()+self.declaration_charge+self.retained_charge()
                        +live_bytes+len(raw)*2+charge+CHUNK+262144 > SEMANTIC):
                    raise ValueError('PA1 control retained allocation')
            else:
                value = records.decode(raw, RECEIPT)
            owner.check()
            return value
        except BaseException:
            owner.fail()
            raise

    def _charge(self, size, kind):
        owner = self.invocation
        if kind == 'binary':
            self.binary += size
            owner.binary_remaining -= size
        else:
            self.nonbinary += size
            owner.nonbinary_remaining -= size
            if kind == 'source':
                self.source += size
        if (self.nonbinary > NONBINARY or self.source > SOURCE or self.binary > BINARY_PAIR
                or self.nonbinary+self.binary > AGGREGATE
                or owner.nonbinary_remaining < 0 or owner.binary_remaining < 0):
            raise ValueError('PA1 unit/invocation bytes')


def stage_reservation(maxima, actual, live_indices, output_indices):
    """Admit an exact live closure without discounting any pending obligation.

    maxima is the frozen ordered81-role vector; actual contains only published,
    authenticated records. None reserves the full maximum. Caller may release a
    role only after its exact frozen dependency edges permit that transition.
    Return both live admission and full lifecycle reservation, not a verdict.
    """
    if (type(maxima) is not tuple or len(maxima) != 81 or type(actual) is not tuple
            or len(actual) != 81 or type(live_indices) is not tuple
            or type(output_indices) is not tuple):
        raise ValueError('PA1 stage shape')
    values = []
    for cap, size in zip(maxima, actual):
        integer(cap, 1048576, 'PA1 role maximum')
        if size is not None:
            integer(size, cap, 'PA1 actual role size')
        values.append(cap if size is None else size)
    selected = live_indices+output_indices
    if len(set(selected)) != len(selected):
        raise ValueError('PA1 stage duplicate role')
    for index in selected:
        integer(index, 80, 'PA1 stage index')
    live = sum(values[i] for i in selected)
    if live > LIVE:
        raise ValueError('PA1 live evidence overflow')
    return dict(live_bytes=live, lifecycle_reserved_bytes=sum(values),
                pending_roles=sum(x is None for x in actual), accepted=False)
