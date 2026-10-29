"""Sealed selected40 metadata compiler. This module grants no runtime authority.

The historical plan is a framing plan, not a semantic verdict. Compilation
preserves that distinction. Complete fit requires the original containing-body
predicates and exact release/reauthentication edges for every current role.
"""
from collections import Counter
import hashlib
import json
import os
from pathlib import Path

import evidence_records as records
import pa1_evidence as core

BASE = Path('/home/fry/artifacts')
MAPPING = dict(path=str(BASE/'vrf-contract-01a1470a-20261017/obligation-mapping.v5.json'),
               bytes=937012, sha256='1b2474553d064640cea64816a661cef5f9edcfca622ee5faaa692d1cda7e6a23')
PLAN = dict(path=str(BASE/'vrf-pa1-implementation-01a1470a-20261017/unit-plan.json'),
            bytes=154835, sha256='4bed53659cb658ea17b72ccb1b1e2d9b2150a86c4945a2a7590d37bcf7a1c160')
VOTER = 'e94679197f925cb953920594ec7ec4c66fc8763b'
OMISSIONS = (3, 9, 15, 22, 24, 29, 45)
PHASES = ('prepare/validate', 'command/READY', 'capture', 'replay/accept/seal')
PHASE_RANGES = ((0, 57), (57, 63), (63, 70), (70, 81))


class IncompleteFit(ValueError):
    """An exact prerequisite is absent; no partial fit result is returned."""


def _read(pin):
    """Read only fixed metadata, with exact EOF and the consumed-stream hash."""
    core.integer(pin['bytes'], MAPPING['bytes'], 'selected40 metadata allocation')
    path = records.canonical_path(pin['path'])
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        import stat
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError('selected40 regular metadata')
        body = bytearray(pin['bytes'])
        position = 0
        remaining = pin['bytes']
        checksum = hashlib.sha256()
        while remaining:
            block = os.read(fd, min(core.CHUNK, remaining))
            if not block:
                raise ValueError('selected40 truncated metadata')
            checksum.update(block)
            body[position:position+len(block)] = block
            position += len(block)
            remaining -= len(block)
        if os.read(fd, 1) or checksum.hexdigest() != pin['sha256']:
            raise ValueError('selected40 metadata EOF/hash')
        return body
    finally:
        os.close(fd)


def _end(raw, start):
    # The entire immutable document has an exact reviewed hash. Scan boundaries
    # without decoding its 671-entry graph or following embedded paths.
    depth = 0
    quoted = escaped = False
    for pos in range(start, len(raw)):
        byte = raw[pos]
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
        elif byte in (91, 123):
            depth += 1
        elif byte in (93, 125):
            depth -= 1
            if depth == 0:
                return pos + 1
        elif byte == 44 and depth == 0:
            return pos
    raise ValueError('selected40 fixed mapping boundary')


def _field(raw, key):
    """Project a named top-level field from the authenticated fixed V5 body."""
    pos = 1
    while pos < len(raw):
        while raw[pos] in b' \r\n\t,':
            pos += 1
        if raw[pos] == 125:
            break
        stop = raw.index(b'"', pos + 1) + 1
        name = json.loads(raw[pos:stop])
        pos = stop
        while raw[pos] in b' \r\n\t:':
            pos += 1
        stop = _end(raw, pos)
        if name == key:
            return pos, stop
        pos = stop
    raise ValueError('selected40 missing fixed mapping field')


def selected_descriptor(*, live_bytes=0):
    """Authenticate V5 and project complete roles with bounded caller lifetime.

    Original array byte/node/depth/scalar/retained bounds stay global. Every
    role is fully decoded once; only maxima/identity survive its release.
    """
    core.integer(live_bytes,core.SEMANTIC,'selected40 descriptor caller')
    if live_bytes+MAPPING['bytes']+2*core.CHUNK+262144+16384>core.SEMANTIC:
        raise ValueError('selected40 descriptor initial allocation')
    raw = _read(MAPPING)
    last = len(raw)-1
    while last>=0 and raw[last] in b' \r\n\t':last-=1
    if raw[:1]!=b'{' or last<0 or raw[last]!=125:
        raise ValueError('selected40 fixed mapping document')

    def fragment(start,stop,retained=()):
        view=memoryview(raw)[start:stop]
        charge=(live_bytes+graph_charge((raw,retained))
                +3*len(view)+160*core._wire_nodes(view)+core.CHUNK+262144)
        if charge>core.SEMANTIC:
            raise ValueError('selected40 descriptor fragment allocation')
        return records.decode(bytes(view),records.WRAPPER_CAP)

    start,stop=_field(raw,'future_role_templates')
    if stop-start>records.WRAPPER_CAP or raw[start]!=91 or raw[stop-1]!=93:
        raise ValueError('selected40 original role array byte/shape cap')
    # Preserve the original full-array lexical limits before any row allocation.
    nodes=depth=0;quoted=escaped=scalar=False
    for byte in memoryview(raw)[start:stop]:
        if quoted:
            if escaped:escaped=False
            elif byte==92:escaped=True
            elif byte==34:quoted=False
            continue
        if byte==34:quoted=True;scalar=False;nodes+=1
        elif byte in (91,123):
            nodes+=1;depth+=1;scalar=False
            if depth>records.DEPTH_CAP+1:
                raise ValueError('selected40 role global depth cap')
        elif byte in (93,125):depth-=1;scalar=False
        elif byte in (9,10,13,32,44,58):scalar=False
        elif not scalar:nodes+=1;scalar=True
        if nodes>records.NODE_CAP:
            raise ValueError('selected40 role global node cap')
    maxima=[];identities=[];full_retained=128;pos=start+1
    while True:
        while pos<stop-1 and raw[pos] in b' \r\n\t':pos+=1
        if pos==stop-1:break
        end=_end(raw,pos)
        row=fragment(pos,end,(maxima,identities))
        # Adding the outer array preserves the original decoded depth bound.
        records.bounded_shape([row])
        full_retained+=graph_charge(row)
        if full_retained>records.BUFFER_CAP-records.MANIFEST_CAP-records.CHUNK:
            raise ValueError('selected40 role global retained cap')
        if type(row) is not dict:
            raise ValueError('selected40 complete role object')
        maxima.append(row['required_maximum'])
        identities.append((row['group'],row['index']))
        del row
        pos=end
        while pos<stop-1 and raw[pos] in b' \r\n\t':pos+=1
        if pos==stop-1:break
        if raw[pos]!=44:raise ValueError('selected40 role array separator')
        pos+=1
        peek=pos
        while peek<stop-1 and raw[peek] in b' \r\n\t':peek+=1
        if peek==stop-1:raise ValueError('selected40 role array trailing comma')
    maxima=tuple(maxima);identities=tuple(identities)
    expected=tuple(('current54',i) for i in range(54))+tuple(
        ('precommand3',i) for i in range(3))+tuple(('outcome24',i) for i in range(24))
    if (identities!=expected or len(maxima)!=81
            or sum(maxima[:57])!=8469410 or sum(maxima[57:])!=2523136):
        raise ValueError('selected40 original role order/maxima')
    del expected
    a, b = _field(raw, 'source_rosters')
    source_rosters = fragment(a,b,(maxima,identities))
    start, _ = _field(raw, 'sequence_slots')
    start += 1
    while raw[start] in b' \r\n\t':
        start += 1
    stop=_end(raw,start)
    # Preserve the full authenticated slot bytes, but release the much larger
    # original mapping before its decoded graph is allocated. The copy itself
    # has a separate guard while both complete bodies coexist.
    retained=(maxima,identities,source_rosters)
    if (live_bytes+graph_charge((raw,retained))+(stop-start)+128
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 slot transfer allocation')
    slot_raw=bytes(memoryview(raw)[start:stop])
    del raw
    if (live_bytes+graph_charge(retained)+3*len(slot_raw)
            +160*core._wire_nodes(slot_raw)+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 slot decode allocation')
    slot=records.decode(slot_raw,records.WRAPPER_CAP)
    del slot_raw,retained
    if (slot['voter'] != VOTER or slot['payloads'] != 40 or slot['full_proofs'] != 91
            or (slot['prior_one'], slot['prior_zero'], slot['prior_proof']) != (18, 17, 68)
            or len(slot['rows']) != 40):
        raise ValueError('selected40 fixed shape/prior')
    periods = tuple(row['full_tuple']['period'] - 25706894 for row in slot['rows'])
    if periods != tuple(i for i in range(1, 48) if i not in OMISSIONS):
        raise ValueError('selected40 fixed omissions/order')
    for row in slot['rows']:
        value = row['full_tuple']
        if (value['voter_hex'] != VOTER
                or value['account_hex'] != '00000000000000000000000000000000000000fe'
                or value['key_preimage_hex'] != '0004'+VOTER
                or value['logical_key_hex'] != '8b1ace38daea48c3738d2841802efffca8b51e3c108416b1625370dec2f5a4d4'
                or not row['source_references']
                or any(ref['period'] != value['period'] + 7 for ref in row['source_references'])):
            raise ValueError('selected40 exact tuple/source shape')
    return dict(slot=slot, maxima=maxima, identities=identities, source_rosters=source_rosters,
                accepted=False, ready=False, complete_fit=False)


def historical_plan():
    """Load the exact accepted preparation; preserve order and repeated roles."""
    raw = _read(PLAN)
    value = records.decode(raw, records.MANIFEST_CAP)
    del raw
    if (value['schema'] != 'pa1-frozen-historical-units-v1'
            or value['mapping_sha256'] != MAPPING['sha256']
            or value['unit_count'] != 380 or value['historical_occurrences'] != 671
            or value['new_physical_part_occurrences'] != 11):
        raise ValueError('selected40 historical plan shape')
    # JSON arrays and immutable tuples have the same canonical binding. Do not
    # duplicate the complete declaration graph beside the decoded plan.
    ids = value['unit_ids']
    declarations = value['declarations']
    if len(ids) != 380 or len(set(ids)) != 380 or len(declarations) != 380:
        raise ValueError('selected40 historical units')
    if core.digest((ids, declarations)) != value['frozen_manifest_sha256']:
        raise ValueError('selected40 frozen historical manifest')
    counts = Counter(row[0] for unit in declarations for row in unit)
    if set(counts) != set(range(682)) or any(count != 1 for count in counts.values()):
        raise ValueError('selected40 all671 plus11 physical occurrences')
    return value


def historical_plan_wires(*, live_bytes=0):
    """Project the complete authenticated plan with one decoded unit at a time.

    The original global byte, node, depth, scalar, duplicate-key and retained
    data limits apply across the whole document. Every original top-level field
    is parsed; declarations retain their exact canonical order and manifest
    binding. The returned wires replace the full decoded declaration graph.
    ``live_bytes`` charges the real caller state; allocation excess rejects
    before parsing or encoding the next fragment. This grants no fit authority.
    """
    core.integer(live_bytes, core.SEMANTIC, 'selected40 projection caller')
    initial_charge = live_bytes+2*PLAN['bytes']+core.CHUNK+262144+16384+1024
    if initial_charge > core.SEMANTIC:
        raise ValueError('selected40 projected plan startup allocation')
    raw = _read(PLAN)
    if len(raw) > records.MANIFEST_CAP:
        raise ValueError('selected40 plan byte cap')
    nodes = depth = 0
    quoted = escaped = scalar = False
    for byte in raw:
        if quoted:
            if escaped: escaped = False
            elif byte == 92: escaped = True
            elif byte == 34: quoted = False
            continue
        if byte == 34:
            nodes += 1; quoted = True; scalar = False
            if depth > records.DEPTH_CAP:
                raise ValueError('selected40 plan scalar depth')
        elif byte in (91, 123):
            nodes += 1; depth += 1; scalar = False
            if depth > records.DEPTH_CAP+1:
                raise ValueError('selected40 plan depth')
        elif byte in (93, 125):
            depth -= 1; scalar = False
        elif byte in (9, 10, 13, 32, 44, 58):
            scalar = False
        elif not scalar:
            nodes += 1; scalar = True
            if depth > records.DEPTH_CAP:
                raise ValueError('selected40 plan scalar depth')
        if nodes > records.NODE_CAP:
            raise ValueError('selected40 plan global node cap')
    fields = {}
    pos = 1
    if raw[:1] != b'{':
        raise ValueError('selected40 plan object')
    while True:
        while pos < len(raw) and raw[pos] in b' \r\n\t': pos += 1
        if pos >= len(raw): raise ValueError('selected40 plan object EOF')
        if raw[pos] == 125:
            pos += 1
            break
        if raw[pos] != 34: raise ValueError('selected40 plan field key')
        end = pos+1
        escaped = False
        while end < len(raw):
            byte = raw[end]
            if escaped: escaped = False
            elif byte == 92: escaped = True
            elif byte == 34: break
            end += 1
        if end-pos > 64 or len(fields) >= 12:
            raise ValueError('selected40 plan field table cap')
        key = records.decode(raw[pos:end+1], records.MANIFEST_CAP)
        if key in fields: raise ValueError('selected40 duplicate plan key')
        pos = end+1
        while pos < len(raw) and raw[pos] in b' \r\n\t': pos += 1
        if pos >= len(raw) or raw[pos] != 58: raise ValueError('selected40 plan colon')
        pos += 1
        while pos < len(raw) and raw[pos] in b' \r\n\t': pos += 1
        stop = _end(raw, pos)
        fields[key] = (pos, stop)
        pos = stop
        while pos < len(raw) and raw[pos] in b' \r\n\t': pos += 1
        if pos < len(raw) and raw[pos] == 125: continue
        if pos >= len(raw) or raw[pos] != 44: raise ValueError('selected40 plan separator')
        pos += 1
        after_comma = pos
        while after_comma < len(raw) and raw[after_comma] in b' \r\n\t': after_comma += 1
        if after_comma >= len(raw) or raw[after_comma] == 125:
            raise ValueError('selected40 plan trailing comma')
    if raw[pos:].strip(): raise ValueError('selected40 plan trailing bytes')
    expected = {'declarations', 'frozen_manifest_sha256', 'future_role_caps', 'historical_occurrences',
                'history_parts', 'logical_aliases', 'mapping_sha256', 'new_physical_part_occurrences',
                'schema', 'scope', 'unit_count', 'unit_ids'}
    if set(fields) != expected: raise ValueError('selected40 complete plan fields')
    metadata = {}
    wires = []
    seen = bytearray(682)
    peak = initial_charge

    def reserve(fragment, decoded=None):
        nonlocal peak
        # Source body, field table and all completed outputs stay live. The
        # next fragment accounts for UTF8/parser storage before decode, and
        # complete decoded/encoder storage before its canonical wire is built.
        shape = core._wire_nodes(fragment)*160+2*len(fragment)
        if decoded is not None:
            shape = max(shape, graph_charge(decoded)+3*len(fragment))
        total = (live_bytes+graph_charge((raw, fields, metadata, wires, seen))
                 +shape+core.CHUNK+262144)
        peak = max(peak, total)
        if total > core.SEMANTIC:
            raise ValueError('selected40 projected plan allocation')

    for key, (start, stop) in fields.items():
        if key == 'declarations': continue
        fragment = memoryview(raw)[start:stop]
        reserve(fragment)
        fragment = bytes(fragment)
        value = records.decode(fragment, records.MANIFEST_CAP)
        reserve(fragment, value)
        metadata[key] = value
        del value, fragment
    if (metadata['schema'] != 'pa1-frozen-historical-units-v1'
            or metadata['mapping_sha256'] != MAPPING['sha256']
            or metadata['unit_count'] != 380 or metadata['historical_occurrences'] != 671
            or metadata['new_physical_part_occurrences'] != 11):
        raise ValueError('selected40 historical projected plan shape')
    ids = metadata['unit_ids']
    if type(ids) is not list or len(ids) != 380 or len(set(ids)) != 380:
        raise ValueError('selected40 projected ordered units')
    # This cumulative charge reproduces bounded_shape's full-document retained
    # storage check; splitting the document never resets its global allowance.
    full_shape = graph_charge(metadata)+graph_charge('declarations')+128
    start, stop = fields['declarations']
    if raw[start] != 91 or raw[stop-1] != 93:
        raise ValueError('selected40 projected declarations array')
    pos = start+1
    scope_pins = set()
    scope_path = str(BASE/'vrf-round3-01a141ba-20261016/source/storage-validation-current.json')
    while pos < stop-1:
        while pos < stop-1 and raw[pos] in b' \r\n\t': pos += 1
        if pos == stop-1: break
        end = _end(raw, pos)
        fragment = memoryview(raw)[pos:end]
        reserve(fragment)
        fragment = bytes(fragment)
        rows = records.decode(fragment, records.MANIFEST_CAP)
        reserve(fragment, rows)
        if type(rows) is not list or not rows:
            raise ValueError('selected40 projected nonempty unit')
        full_shape += graph_charge(rows)
        if full_shape > records.BUFFER_CAP-records.MANIFEST_CAP-records.CHUNK:
            raise ValueError('selected40 projected global retained cap')
        for row in rows:
            if type(row) is not list or len(row) != 4:
                raise ValueError('selected40 projected declaration row')
            occurrence = row[0]
            core.integer(occurrence, 681, 'selected40 projected occurrence')
            if seen[occurrence]: raise ValueError('selected40 duplicate projected occurrence')
            seen[occurrence] = 1
            if row[1][0] == scope_path: scope_pins.add(tuple(row[1]))
        wire = core.canonical(rows)
        if len(wire) > records.MANIFEST_CAP:
            raise ValueError('selected40 projected unit wire cap')
        wires.append(wire)
        del rows, row, wire, fragment
        pos = end
        while pos < stop-1 and raw[pos] in b' \r\n\t': pos += 1
        if pos == stop-1: break
        if raw[pos] != 44: raise ValueError('selected40 projected unit separator')
        pos += 1
        look = pos
        while look < stop-1 and raw[look] in b' \r\n\t': look += 1
        if look == stop-1: raise ValueError('selected40 projected trailing unit comma')
    if len(wires) != 380 or not all(seen) or len(scope_pins) != 1:
        raise ValueError('selected40 projected complete occurrences/scope')
    if core.frozen_manifest_digest(ids, tuple(wires)) != metadata['frozen_manifest_sha256']:
        raise ValueError('selected40 projected frozen manifest')
    transfer = (live_bytes+graph_charge((raw, fields, metadata, wires, seen))
                +graph_charge((ids, wires, metadata['logical_aliases'], metadata['future_role_caps']))
                +core.CHUNK+262144)
    peak = max(peak, transfer)
    if transfer > core.SEMANTIC:
        raise ValueError('selected40 projected plan transfer allocation')
    return dict(unit_ids=tuple(ids), units=tuple(wires), logical_aliases=tuple(metadata['logical_aliases']),
                future_role_caps=tuple(metadata['future_role_caps']), scope_pin=_pin(next(iter(scope_pins))),
                allocation=dict(peak=peak, caller=live_bytes, original_full_graph=full_shape,
                                source_body_bytes=len(raw), global_nodes=nodes))


def all_phase_reservations(maxima, actual, stages):
    """Check complete ordered stage packing without implicit releases.

    A stage declares its phase, all live prerequisites and output roles. This
    function proves arithmetic only; it cannot authenticate a release claim.
    Unknown roles keep full lifecycle maxima even after their decoded release.
    """
    if (type(stages) is not tuple or not stages
            or type(actual) is not tuple or len(actual) != 81):
        raise ValueError('selected40 complete stages')
    results = []
    covered = []
    for stage in stages:
        if type(stage) is not tuple or len(stage) != 3:
            raise ValueError('selected40 stage shape')
        phase, live, outputs = stage
        if phase not in PHASES or type(outputs) is not tuple or not outputs:
            raise ValueError('selected40 stage phase/outputs')
        bounds = PHASE_RANGES[PHASES.index(phase)]
        if any(type(i) is not int or not bounds[0] <= i < bounds[1] for i in outputs):
            raise ValueError('selected40 phase role')
        covered.extend(outputs)
        result = core.stage_reservation(maxima, actual, live, outputs)
        results.append(dict(phase=phase, **result))
    if covered != list(range(81)):
        raise ValueError('selected40 complete ordered current81 coverage')
    return tuple(results)


BASELINE_EVENTS = (4, 0, 0)
BASELINE_HISTORY = '3eebc13514a5fbe0569914407d17d9f45606f8baca161a1ba9ab81168261c0e0'
ROOT = BASE/'vrf-pa1-s3-01a1470a-20261017/fixed40'
SAVED_REPORT_ROOT = BASE/'vrf40-admission-01a14b66-20261018/saved-v5'
REPO = Path('/workspaces/rustaxa-evm')
TOOLS = REPO/'experiments/evm_feasibility/snapshot_qualifier/tools'
SAVED_COMMAND = ('rtk', 'proxy', 'cargo', 'test', '--offline', '--manifest-path',
                 'experiments/evm_feasibility/snapshot_qualifier/Cargo.toml',
                 '--lib', 'saved', '--', '--nocapture')
SAVED_TESTS = ('saved_report_reuse_requires_identical_complete_bytes',
               'six_saved_windows_have_complete_equivalent_reports',
               'first_residual_saved_bundle_report_is_completely_equal',
               'second_residual_saved_bundle_report_is_completely_equal',
               'third_residual_saved_bundle_report_is_completely_equal',
               'fourth_residual_saved_bundle_report_is_completely_equal',
               'fifth_residual_saved_bundle_report_is_completely_equal',
               'sixth_residual_saved_bundle_report_is_completely_equal',
               'seventh_residual_saved_bundle_report_is_completely_equal',
               'native_and_first_d_saved_reports_are_completely_equal')
# These are the original finite source/executable positions, not arbitrary
# provenance fields. Full logical, physical, proof and other metadata must equal.
NATIVE_FIELDS = ('/source_profile/compiled/0', '/source_profile/compiled/10', '/source_profile/compiled/11', '/source_profile/compiled/12', '/source_profile/executable', '/source_profile/compiled/8')
FIRST_FIELDS = ('/authority/executable', '/authority/sources/2', '/authority/sources/16', '/authority/sources/17', '/authority/sources/18', '/vrf_sources/0', '/vrf_sources/3', '/vrf_sources/4', '/vrf_sources/5', '/authority/sources/14')
BEFORE = (
    dict(path=str(BASE/'vrf-run-01a141ba-20261015/fbb45/compatibility/native-current.json'),
         bytes=10945, sha256='4c4c045bb090749c8c83acab2fd851999473dfa1b2820ab496cc1fbc0730ae06'),
    dict(path=str(BASE/'vrf-run-01a141ba-20261015/fbb45/compatibility/first-d-current.json'),
         bytes=32813, sha256='b1c09f0c1bacfad76764f40df66b71b8a9f5e8c97eac79c4151f6b87dc68b765'))
PERSISTENT = (28, 30, 32, 37, 38, 39, 40, 41, 48, 49, 53, 54, 55, 56)
DEFERRED = ('source acceptance with every current gate and original historical child',
            'future commit and normal hook', 'exact command READY and launch binding',
            'one-use physical capture and native13 ceilings',
            'fresh71 plus new91 proofs and exact EOF at both replays',
            'five reverse controls, independent full-tuple/fact acceptance and final seal',
            'copy metadata observation and complete database preservation')


def require_live40_budget(consumed_events, event_history):
    """Preserve inherited four events and bind the exact accepted S3 baseline."""
    if type(consumed_events) is not tuple or len(consumed_events) != 3:
        raise ValueError('selected40 exact consumed-event counts')
    for count, minimum, maximum in zip(consumed_events, BASELINE_EVENTS, core.EVENT_MAXIMA):
        core.integer(count, maximum, 'selected40 consumed events')
        if count < minimum:
            raise ValueError('selected40 inherited events cannot reset')
    if event_history != BASELINE_HISTORY:
        raise ValueError('selected40 inherited event-history binding')
    if consumed_events[0] >= core.EVENT_MAXIMA[0]:
        raise IncompleteFit('selected40 failure/correction events exhausted; no reserved outcome')


def sealed_stages(maxima, actual):
    """Compile the finite PA1 current81 schedule, including receipt overlap.

    Source/gate roles0..27 stay live through source28 acceptance. Roles29..56
    stay beside source28 until code/gates37 and authority40 publication. Only
    immutable published validation logs, completed projections and inherited
    parent bodies then leave decoded memory; every later route rereads their
    exact original occurrences. Persistent authority, current failures, full
    tuple map and precommand inputs remain live through final acceptance.
    No body is erased, and the schedule never grants a pending publication.
    """
    stages = (
        (PHASES[0], (), tuple(range(29))),
        (PHASES[0], (28,), tuple(range(29, 57))),
        (PHASES[1], PERSISTENT, tuple(range(57, 63))),
        (PHASES[2], PERSISTENT+tuple(range(57, 63)), tuple(range(63, 70))),
        (PHASES[3], PERSISTENT+tuple(range(57, 70)), tuple(range(70, 81))))
    reservations = all_phase_reservations(maxima, actual, stages)
    for row in reservations:
        row['receipt_reserved_bytes'] = core.RECEIPT
        if row['live_bytes']+core.RECEIPT > core.LIVE:
            raise ValueError('selected40 live closure plus receipt overflow')
    releases = (dict(after=28, roles=tuple(range(28)),
                     prerequisite='source28 acceptance and exact current gate bindings'),
                dict(after=56, roles=tuple(i for i in range(29, 57) if i not in PERSISTENT),
                     prerequisite='published code/gates37, authority40 and exact source/commit bindings'))
    routes = tuple((route, count) for route, count in core.ROUTES.items())
    return dict(stages=stages, reservations=reservations, releases=releases,
                reauthentication_routes=routes, retained_roles=tuple(range(81)),
                accepted=False, conditional_on_publication=True)


def reference_context(pin):
    """Project exact accepted placement/predicate descriptors; never expand671."""
    raw = _read(pin)
    result = {}
    for key in ('reference_placements', 'historical_equivalence_parse', 'correction_record48'):
        start, stop = _field(raw, key)
        result[key] = records.decode(raw[start:stop], records.WRAPPER_CAP)
    del raw
    placements = result['reference_placements']
    if len(placements) != 3 or sum(1+len(row['references']) for row in placements) != 64:
        raise ValueError('selected40 complete64 original record children')
    if sum(row['common_contract']['bytes']+sum(p['bytes'] for p in row['references'])
           for row in placements) != 1048091:
        raise ValueError('selected40 unchanged complete child-byte reservation')
    return result


def wrapper_specimen(placement, destination, prefix=''):
    """Build the reviewed fit-only max-findings body; no actual pin or verdict."""
    value = dict(schema=records.SCHEMA, destination=destination, ledger='validation',
                 contract=placement['common_contract'], references=placement['references'],
                 findings=(prefix+'FIT_ONLY_SELECTED40_NO_AUTHORITY').ljust(16384, 'x'), accepted=False)
    if len(value['findings'].encode()) != 16384:
        raise ValueError('selected40 complete required findings prefix cap')
    raw = core.canonical(value)
    if len(raw) > 65536:
        raise ValueError('selected40 original wrapper encoding reserve')
    return records.decode(raw, records.WRAPPER_CAP)


def verify_historical_projection(value, descriptor):
    """Keep every original45182 parse predicate and complete output/projection."""
    if (set(value) != set(descriptor['exact_keys']) or type(value['schema']) is not int
            or value['schema'] != 1 or type(value['matched_tests']) is not int
            or value['matched_tests'] != 11 or type(value['new_facts']) is not int
            or value['new_facts'] != 0 or any(value[k] is not True for k in
                ('first_second_third_and_ten_complete_equal', 'only_reviewed_provenance_changes_allowed'))):
        raise ValueError('selected40 historical projection schema/facts')
    pairs = value['native_and_first_d_complete_equality']
    if (type(pairs) is not list or [r.get('profile') for r in pairs] != ['native-current', 'first-d-current']
            or any(r.get('complete_equal_except_exact_pin_mappings') is not True for r in pairs)):
        raise ValueError('selected40 historical native/firstD predicates')
    output = value['full_gate_output']
    if type(output) is not str:
        raise ValueError('selected40 historical complete output type')
    body = output.encode('utf-8')
    if len(body) != descriptor['output_bytes'] or hashlib.sha256(body).hexdigest() != descriptor['output_sha256']:
        raise ValueError('selected40 historical complete output hash')
    projected = {k:v for k,v in value.items() if k != 'full_gate_output'}
    # Original projection omits the newline used by PA1's own canonical format.
    if hashlib.sha256(core.canonical(projected)[:-1]).hexdigest() != descriptor['projection_canonical_sha256']:
        raise ValueError('selected40 historical full projection hash')


def _location(value, field):
    parts = field.lstrip('/').split('/')
    for key in parts[:-1]:
        value = value[int(key)] if isinstance(value, list) else value[key]
    return value, int(parts[-1]) if isinstance(value, list) else parts[-1]


def verify_current_pair(before, after, fields, source_pins, executable):
    """Compare entire authenticated reports after only exact finite pin mappings."""
    if fields not in (NATIVE_FIELDS, FIRST_FIELDS):
        raise ValueError('selected40 finite comparator fields')
    if len({row['path'] for row in source_pins}) != len(source_pins):
        raise ValueError('selected40 duplicate source freeze paths')
    for field in fields:
        old, key = _location(before, field)
        new, new_key = _location(after, field)
        if field.endswith('/executable'):
            expected = executable
        else:
            original = old[key]
            path = str((REPO/'experiments/evm_feasibility/snapshot_qualifier/src'/original['path']).resolve())
            source = next((row for row in source_pins if row['path'] == path), None)
            if source is None or new[new_key]['path'] != original['path']:
                raise ValueError('selected40 finite source mapping identity')
            expected = dict(source, path=original['path'])
        if new[new_key] != expected:
            raise ValueError('selected40 finite source/executable mapping')
        old[key] = expected
    if core.digest(before) != core.digest(after):
        raise ValueError('selected40 complete current report inequality')


def verify_saved_gate(value, log_pin, output):
    """Require the exact focused saved-report command and all nine test results."""
    if (value.get('command') != list(SAVED_COMMAND) or type(value.get('exit_code')) is not int
            or value['exit_code'] != 0 or value.get('log') != log_pin):
        raise ValueError('selected40 exact saved comparator command/output')
    lines = output.splitlines()
    expected = {('test vrf_shared_window::tests::'+name+' ... ok').encode() for name in SAVED_TESTS}
    if any(sum(line == target for line in lines) != 1 for target in expected):
        raise ValueError('selected40 nonzero exact nine saved comparisons')


# Exact object spans in the immutable full V5 mapping. Every byte is still
# streamed, hashed and checked through EOF before any projection is trusted.
PRIOR83_RANGES = ((755939, 757060), (757066, 758187), (758193, 759293), (759299, 760414), (760420, 761536), (761542, 762659), (762665, 763778), (763784, 764899), (764905, 766026), (766032, 767143), (767149, 768254), (768260, 769393), (769399, 770513), (770519, 771639), (771645, 772746), (772752, 773857), (773863, 774975), (774981, 776092), (776098, 777199), (777205, 778326), (778332, 779437), (779443, 780545), (780551, 781664), (781670, 782788), (782794, 783914), (783920, 785005), (785011, 786100), (786106, 787200), (787206, 788297), (788303, 789410), (789416, 790493), (790499, 791584), (791590, 792675), (792681, 793767), (793773, 794860), (794866, 795956), (795962, 797048), (797054, 798135), (798141, 799257), (799263, 800386), (800392, 801517), (801523, 802582), (802588, 803651), (803657, 804721), (804727, 805848), (805854, 806982), (806988, 808118), (808124, 809246), (809252, 810381), (810387, 811518), (811524, 812649), (812655, 813785), (813791, 814923), (814929, 816053), (816059, 817188), (817194, 818325), (818331, 819455), (819461, 820590), (820596, 821727), (821733, 822864), (822870, 824003), (824009, 825139), (825145, 826277), (826283, 827412), (827418, 828549), (828555, 829684), (829690, 830821), (830827, 831952), (831958, 833085), (833091, 834215), (834221, 835347), (835353, 836477), (836483, 837609), (837615, 838739), (838745, 839871), (839877, 841001), (841007, 842133), (842139, 843263), (843269, 844395), (844401, 845525), (845531, 846657), (846663, 847857), (847863, 849059))
def prior83_bindings(wires, *, live_bytes=0):
    """Bind all prior81 sources and both historical binaries to original units.

    Fixed byte spans are valid only under MAPPING's immutable full-stream hash.
    Projection does not authenticate a body or discharge an original read.
    One bounded unit and one original mapping row are decoded at a time.
    """
    import stat
    core.integer(live_bytes,core.SEMANTIC,'selected40 prior83 caller')
    fragment_max=sum(b-a for a,b in PRIOR83_RANGES)
    initial=(live_bytes+fragment_max+128*(len(PRIOR83_RANGES)+4)
             +2*core.CHUNK+262144)
    if initial>core.SEMANTIC:
        raise ValueError('selected40 prior83 projection allocation')
    fd=os.open(records.canonical_path(MAPPING['path']),
               os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    fragments=[bytearray() for _ in PRIOR83_RANGES]
    size=0;checksum=hashlib.sha256()
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError('selected40 prior83 regular mapping')
        while size<MAPPING['bytes']:
            block=os.read(fd,min(core.CHUNK,MAPPING['bytes']-size))
            if not block:raise ValueError('selected40 prior83 truncated mapping')
            start=size;size+=len(block);checksum.update(block)
            for index,(a,b) in enumerate(PRIOR83_RANGES):
                left=max(start,a);right=min(size,b)
                if left<right:fragments[index].extend(block[left-start:right-start])
            if (live_bytes+graph_charge(fragments)+2*core.CHUNK+262144
                    >core.SEMANTIC):
                raise ValueError('selected40 prior83 projection allocation')
        if os.read(fd,1) or checksum.hexdigest()!=MAPPING['sha256']:
            raise ValueError('selected40 prior83 full mapping EOF/hash')
    finally:os.close(fd)
    declarations={}
    for raw in wires:
        if (live_bytes+graph_charge((fragments,declarations))
                +2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144
                >core.SEMANTIC):
            raise ValueError('selected40 prior83 unit allocation')
        rows=records.decode(raw,records.MANIFEST_CAP)
        for row in rows:
            if 588<=row[0]<=670:
                if row[0] in declarations:raise ValueError('selected40 duplicate prior83')
                declarations[row[0]]=row
        del rows
    if set(declarations)!=set(range(588,671)):
        raise ValueError('selected40 complete prior83 unit bindings')
    bindings=[]
    for index,fragment in enumerate(fragments):
        if len(fragment)!=PRIOR83_RANGES[index][1]-PRIOR83_RANGES[index][0]:
            raise ValueError('selected40 complete prior83 mapping span')
        if (live_bytes+graph_charge((fragments,declarations,bindings))
                +2*len(fragment)+160*core._wire_nodes(fragment)
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 prior83 binding allocation')
        value=records.decode(fragment,16384)
        occurrence=index+588
        declaration=declarations[occurrence]
        kind='source' if index<81 else 'binary'
        expected_class='source81' if index<81 else 'historical-fixed42-binary2'
        if (value['occurrence']!=occurrence
                or value['predecessor_class']!=expected_class
                or value['predecessor_index']!=(index if index<81 else index-81)
                or declaration!=[occurrence,list(_pin_tuple(value['pin'])),kind,'bytes']
                or value['read_charge_bytes']!=value['pin']['bytes']
                or value['source_path_following'] is not False
                or value['multiplicity']!=1):
            raise ValueError('selected40 exact prior83 original binding')
        bindings.append(dict(occurrence=occurrence,pin=value['pin'],kind=kind,mode='bytes',
            predecessor_class=expected_class,predecessor_index=value['predecessor_index'],
            parse_rule=value['parse_rule'],authority_ref=value['authority_ref']))
        fragments[index]=bytearray()
        del value
    return tuple(bindings)


RETAINED_CHILD_ALIAS_PIN = {'path': '/home/fry/artifacts/vrf40-alias577-20261024-round154/retained-child-source-aliases-v2.json', 'bytes': 2922, 'sha256': 'ecc8e9a676993feb7b719b1d3b135ed3922d076426086fc7fbf0974d22463cd5'}
RETAINED_CHILD_ALIAS_REVIEW = {'path': '/home/fry/artifacts/vrf40-alias577-20261024-round154/sol-four-aliases-preapplication-review.json', 'bytes': 2734, 'sha256': '797ed80f7d6ab413b7a7754f846545749f526b2c24206937c251ddeff81b422d'}

def retained_child_wires(plan, *, live_bytes):
    """Apply only the four independently reviewed exact physical aliases.

    Original PLAN bytes/manifest remain unchanged and fully authenticated.
    Current successor wires preserve every logical occurrence, original full
    body hash/length/kind/mode, semantic predicate and read/parse charge.
    """
    core.integer(live_bytes,core.SEMANTIC,'selected40 retained child caller')
    if live_bytes+graph_charge(plan)+3*32768+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 retained child metadata allocation')
    raw=_read(RETAINED_CHILD_ALIAS_REVIEW)
    a,b=_field(raw,'verdict')
    verdict=records.decode(raw[a:b],32768)
    if verdict.get('aliases')!='ACCEPT_FOUR_EXACT_HISTORICAL_PHYSICAL_ALIASES_PREAPPLICATION_ONLY':
        raise ValueError('selected40 exact retained child independent review')
    del raw,verdict
    value=records.decode(_read(RETAINED_CHILD_ALIAS_PIN),16384)
    if (value.get('schema')!='fixed40-exact-retained-child-aliases-v1'
            or value.get('original_plan')!=PLAN
            or value.get('source_reconstruction') is not False
            or value.get('all_original_bytes_and_charges_preserved') is not True
            or tuple(x['occurrence'] for x in value['rows'])!=(571,577,578,579)):
        raise ValueError('selected40 exact retained child alias proposal')
    targets={row['occurrence']:row for row in value['rows']}
    if set(targets)&{row['occurrence'] for row in plan['logical_aliases']}:
        raise ValueError('selected40 duplicate retained child alias')
    if (live_bytes+graph_charge((plan,value))+graph_charge(plan['units'])
            +2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 retained child wire transfer allocation')
    wires=list(plan['units']);aliases=list(plan['logical_aliases']);seen=set()
    for index,wire in enumerate(wires):
        # Canonical unit bytes have exact immutable structure. Only candidate
        # units are decoded; no full declaration graph is reintroduced.
        if not any(('['+str(n)+',').encode() in wire for n in targets):continue
        if (live_bytes+graph_charge((plan,value,wires,aliases))
                +3*len(wire)+160*core._wire_nodes(wire)
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 retained child unit allocation')
        rows=records.decode(wire,records.MANIFEST_CAP)
        changed=False
        for row in rows:
            if row[0] not in targets:continue
            replacement=targets[row[0]]
            logical=replacement['logical'];physical=replacement['physical']
            if (row[0] in seen or row!=[row[0],list(_pin_tuple(logical)),
                    replacement['kind'],replacement['mode']]
                    or physical['bytes']!=logical['bytes']
                    or physical['sha256']!=logical['sha256']):
                raise ValueError('selected40 unchanged original retained child binding')
            row[1]=list(_pin_tuple(physical))
            aliases.append(dict(occurrence=row[0],logical=logical,physical=physical))
            seen.add(row[0]);changed=True
        if changed:wires[index]=core.canonical(rows)
        del rows
    if seen!=set(targets):
        raise ValueError('selected40 missing retained child occurrence')
    plan['units']=tuple(wires);plan['logical_aliases']=tuple(aliases)
    return plan


def _compiled_profile_binding(admission, *, live_bytes):
    """Project authenticated profile controls without retaining its decoded graph."""
    if admission is None:return None
    # The private wire is deliberately migration-only. The original profile
    # remains a full authenticated input, including a new charged read here.
    pin=admission.profile_pin
    if admission._graphs or admission._outer_live!=0:
        raise ValueError('selected40 completed profile admission required')
    # _load owns these two retained graphs itself. All other admission and
    # external roots remain in the caller. Its128-node decoder reserve plus
    # this32-byte reserve per input byte covers the existing160-node bound.
    trace_before=graph_charge(admission.read_trace)
    load_owned=trace_before+graph_charge(pin)
    if live_bytes<load_owned:
        raise ValueError('selected40 complete profile caller ownership')
    decoder_reserve=32*(pin['bytes']+1)
    if (live_bytes+2*pin['bytes']+decoder_reserve+2*core.CHUNK+262144
            >core.SEMANTIC):
        raise ValueError('selected40 authenticated full profile predecode lifetime')
    try:
        profile=admission._load(pin,32768,decoded=True,
            live_bytes=live_bytes-load_owned+decoder_reserve)
    finally:
        # A completed full read remains live even if its decoder later fails.
        trace_growth=max(0,graph_charge(admission.read_trace)-trace_before)
        admission.caller_charge+=trace_growth
        live_bytes+=trace_growth
    coherent=profile.get('schema')=='fixed40-development-runtime-profile-v2' if type(profile) is dict else False
    fields=admission.PROFILE_FIELDS|({'development_successor','runtime_prefix'} if coherent else set())
    if (type(profile) is not dict or set(profile)!=fields
            or profile['schema'] not in ('fixed40-development-runtime-profile-v1',
                'fixed40-development-runtime-profile-v2')):
        raise ValueError('selected40 exact authenticated original profile')
    del fields
    if live_bytes+2*graph_charge(profile)+graph_charge(pin)+2048+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 authenticated compact profile allocation')
    result=dict(profile=dict(admission.profile_pin))
    for name in ('authority','history','events','source_identity'):
        result[name]=profile[name]
    if live_bytes+graph_charge((profile,result))+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 authenticated compact profile lifetime')
    del profile,name
    _pin_tuple(result['authority']);_pin_tuple(result['history'])
    core.sha(result['source_identity'])
    return result


def compile_selected40(*, consumed_events, event_history, live_bytes=0, admission=None):
    """Compile all81 full reservations and the exact actual-input execution plan.

    The returned plan is preparation, not successful fit or authority. Running
    the semantic port below requires actual comparator/source inputs and durable
    counters. It never treats pending production predicates as successful.
    """
    if admission is None:
        require_live40_budget(consumed_events, event_history)
    else:
        from pa1_fixed40_runtime import RuntimeAdmission
        if type(admission) is not RuntimeAdmission:
            raise ValueError('selected40 concrete authenticated runtime admission')
        admission.check_compilation(consumed_events, event_history)
        live_bytes += admission.caller_charge
    profile_caller_before=admission.caller_charge if admission is not None else 0
    profile_binding=_compiled_profile_binding(admission,live_bytes=live_bytes)
    if admission is not None:
        live_bytes+=admission.caller_charge-profile_caller_before
    live_bytes+=graph_charge(profile_binding)
    # Project the large V5 source before any historical wire graph exists.
    descriptor=selected_descriptor(live_bytes=live_bytes)
    maxima=descriptor['maxima'];sources=descriptor['source_rosters']
    if (live_bytes+graph_charge(descriptor)+3*records.WRAPPER_CAP
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 descriptor transfer allocation')
    slot_wire=core.canonical(descriptor['slot'])
    del descriptor
    descriptor_charge=graph_charge((maxima,sources,slot_wire))
    projection_caller=live_bytes+descriptor_charge
    plan=historical_plan_wires(live_bytes=projection_caller)
    prior=prior83_bindings(plan['units'],
        live_bytes=projection_caller+graph_charge(plan))
    if (projection_caller+graph_charge((plan,prior))+3*records.WRAPPER_CAP
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 prior83 serialization allocation')
    prior_wire=core.canonical(prior)
    if len(prior_wire)>records.WRAPPER_CAP:
        raise ValueError('selected40 complete prior83 wire cap')
    del prior
    plan=retained_child_wires(plan,
        live_bytes=projection_caller+graph_charge(prior_wire))
    setup_wire=tuple(core.canonical(rows) for rows in setup_units())
    schedule=sealed_stages(maxima,(None,)*81)
    context = reference_context(sources['accepted_roles'])
    if tuple(plan['future_role_caps']) != maxima:
        raise ValueError('selected40 role reservation drift')
    # Retain serialized unit closures only. Core performs bounded per-unit
    # parsing and keeps all body operations inside one indivisible unit.
    scope_pin = plan['scope_pin']
    scope_raw = _read(scope_pin)
    a, b = _field(scope_raw, 'failure_retention_amendment')
    amendment = records.decode(scope_raw[a:b], records.WRAPPER_CAP)
    del scope_raw
    prefix = amendment['source28_findings_prefix']
    body = prefix['body_utf8'].encode()
    if len(body) != prefix['bytes'] or hashlib.sha256(body).hexdigest() != prefix['sha256']:
        raise ValueError('selected40 complete source28 findings prefix')
    context['source28_prefix'] = prefix['body_utf8']
    context['scope36_pin'] = scope_pin
    context['retention_amendment'] = amendment
    units = plan['units']
    ids = plan['unit_ids']
    aliases = plan['logical_aliases']
    projection_allocation = plan['allocation']
    del plan
    return dict(schema='selected40-compiled-semantic-plan-v1', unit_ids=ids, units=units,
                aliases=aliases, context=context, sources=sources, maxima=maxima, schedule=schedule,
                slot_wire=slot_wire,profile_binding=profile_binding,
                setup_unit_wire=setup_wire,
                projection_allocation=projection_allocation, prior83_wire=prior_wire,
                prior83_mapping_read=dict(pin=MAPPING,bytes=MAPPING['bytes'],eof=True),
                consumed_events=consumed_events, inherited_event_history=event_history,
                deferred_production_checks=DEFERRED, accepted=False, ready=False,
                complete_fit=False, runtime_admission=admission is not None,
                bridge_caller_charge=0 if admission is None else admission.caller_charge)


SHARED_BEFORE = ({'path': '/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-voter-window-replay.json', 'bytes': 59391, 'sha256': 'b6278f844e627f5830d56803a5555668aa982b56ce2c81fdc9b0cb677b9bc3e6', 'adapter': 'vrf_voter_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/vrf-next-voter-window-replay.json', 'bytes': 59374, 'sha256': '0e3b5e18f00209a7799da50565d90bc273ef226c17cc33c5685578255495db42', 'adapter': 'vrf_next_voter_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/third-voter/vrf-third-voter-window-replay.json', 'bytes': 59403, 'sha256': 'a7f94b240fcdf7293475ae0ec7972fb0d1fb72609703db7080b339066de6c59a', 'adapter': 'vrf_third_voter_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/fourth-voter/vrf-fourth-voter-window-replay.json', 'bytes': 59410, 'sha256': 'bd099e106c56454bcf94cb6c2fbadbb5d4f14d0acad9b74786d569af75525815', 'adapter': 'vrf_fourth_voter_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/fifth-voter/vrf-fifth-voter-window-replay.json', 'bytes': 59403, 'sha256': '9f3124ada4f7c88dd2c8037ec0ac7c7e87907bcea61d683df8746054d9f39fce', 'adapter': 'vrf_fifth_voter_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/sixth-voter/vrf-sixth-voter-window-replay.json', 'bytes': 59404, 'sha256': '35c05787ece4bb2b38a3cbe2fb3137019cbb34d79fd24c9406b857821d5fe187', 'adapter': 'vrf_sixth_voter_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/seventh-voter/vrf-seventh-voter-window-replay.json', 'bytes': 59467, 'sha256': 'f551c202f8b929616294395d0748f8e269b53352748a8411fd7671a9a37a8c5e', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/eighth-voter/vrf-eighth-voter-window-replay.json', 'bytes': 59470, 'sha256': '59280657393129d87ead097ecb1ce967f24e6ce5974ec8f3f926aed328f7631d', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/ninth-voter/vrf-ninth-voter-window-replay.json', 'bytes': 59464, 'sha256': '3833e18901c4c633e48962f607248a5850c2ce24e759eca94e37955f2c435d5d', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/tenth-voter/vrf-tenth-voter-window-replay.json', 'bytes': 59464, 'sha256': '760aab8712c5047c4efd9a1ee3c7ab518d2dd1af468a4f975e65cfa3e2ff9d90', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/vrf-residual-first-voter-window-replay.json', 'bytes': 58552, 'sha256': 'c6e2c514ef2d923441d3b121cd69e33be5e51c5cab9792e3ae015ddc4daeb1bb', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/vrf-residual-second-voter-window-replay.json', 'bytes': 58554, 'sha256': '61d9eee9508222092c3d5595e7b7ec8c7f9b6150dfbc15f23c4bd05cbe15152a', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/vrf-residual-third-voter-window-replay.json', 'bytes': 58552, 'sha256': '609bae5bfce834365e89423c10913fa921876192b905b978c0f757976c72ca44', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/vrf-residual-fourth-voter-window-replay.json', 'bytes': 57633, 'sha256': '60fc9d1488e099aa9b01e245ca1c4a4052f0583ea1cdb116d0ec5913b6d9b914', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/vrf-residual-20261014-current/a8a45/independent-replay-report.json', 'bytes': 57499, 'sha256': 'e3b766f19941f296632c7e8f860e3fc8f92f10741ed2abef8c0ce0445a63ada8', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/vrf-round2-01a141ba-20261016/fbb45/fbb45-replay.json', 'bytes': 57495, 'sha256': 'b906f78a4b45abfc2d269c5197f2ac710d63ec29c6a0f4cdbcb3dc14d520b69b', 'adapter': 'vrf_shared_window.rs'}, {'path': '/home/fry/artifacts/vrf-round3-01a141ba-20261016/next42/next42-replay.json', 'bytes': 54733, 'sha256': '2a0a86f5fb1b2f2e67637a4750a5df156270c5b28937edb663fc85a64cd8efa4', 'adapter': 'vrf_shared_window.rs'})
STORAGE_PATHS = frozenset('../../../../rust/crates/rustaxa-storage/src/'+suffix for suffix in
    ('lib.rs', 'concrete_state/mod.rs', 'concrete_state/witness.rs',
     'concrete_state/witness_live.rs', 'concrete_state/witness_bundle.rs'))
INPUT_FIELDS = {'source_freeze', 'tool_freeze', 'saved_result', 'saved_log',
                'native_comparison', 'test_executable', 'shared_reports', 'native_reports'}


def _pin_tuple(pin):
    if type(pin) is not dict or set(pin) != {'path', 'bytes', 'sha256'}:
        raise ValueError('selected40 frozen input pin')
    records.canonical_path(pin['path'])
    core.sha(pin['sha256'])
    core.integer(pin['bytes'], core.BINARY_FILE, 'selected40 input length')
    return pin['path'], pin['bytes'], pin['sha256']


def _executable_value(pin):
    # Original comparator semantics compare a frozen native executable value;
    # no report path is followed and no third PA1 binary body is introduced.
    if type(pin) is not dict or set(pin) != {'path', 'bytes', 'sha256'}:
        raise ValueError('selected40 executable metadata value')
    if (type(pin['path']) is not str or not pin['path'].startswith('/')
            or len(pin['path'].encode()) > 4096):
        raise ValueError('selected40 executable metadata path')
    core.integer(pin['bytes'], 268435456, 'selected40 native executable metadata cap')
    core.sha(pin['sha256'])


def _pin(row):
    return dict(zip(('path', 'bytes', 'sha256'), row))


def _expected_sources(compiled):
    raw = _read(compiled['sources']['accepted_parent'])
    start, stop = _field(raw, 'pins')
    # Do not decode the complete376-pin graph beside the historical plan.
    pins = records.decode(raw[start:stop], records.WRAPPER_CAP)
    if len(pins) != 376:
        raise ValueError('selected40 original parent inventory count')
    paths = [row['path'] for row in pins[:90]]
    del pins
    del raw
    source = tuple(paths[:88])+tuple(str(REPO/'experiments/evm_feasibility/snapshot_qualifier/src/bin'/name)
                                     for name in ('live_vrf_residual_e946.rs', 'replay_vrf_residual_e946.rs'))
    tools = tuple(paths[88:])+tuple(str(TOOLS/name) for name in
                                  ('pa1_evidence.py', 'test_pa1_evidence.py', 'pa1_fixed40.py', 'test_pa1_fixed40.py'))
    if compiled.get('runtime_admission'):
        tools += tuple(str(TOOLS/name) for name in
                       ('pa1_development.py', 'test_pa1_development.py',
                        'pa1_fixed40_runtime.py', 'test_pa1_fixed40_runtime.py',
                        'pa1_fixed40_constructors.py'))
    return source, tools


def _projection_body(raw, pin, decoder=None):
    if len(raw) != pin['bytes'] or hashlib.sha256(raw).hexdigest() != pin['sha256']:
        raise ValueError('selected40 retained full report pin')
    return records.decode(raw, records.WRAPPER_CAP) if decoder is None else decoder(raw)


NATIVE_GENERATOR_LOGICAL = dict(
    path=str(TOOLS/'pa1_fixed40.py'), bytes=92662,
    sha256='65dec81540b09cc897cc32a3e9d83507e8bacbe6b7eccd3ec1ea886b17f29127')
NATIVE_GENERATOR_RETAINED = dict(NATIVE_GENERATOR_LOGICAL,
    path=str(BASE/'vrf40-admission-01a14b66-20261018/recovery46-source-final/pa1_fixed40.py'))


def _native_payload(value, source_pins, tool_pins, executable, unit, occurrence, live, native_reports,
                    generator_source=None):
    """Compare complete pinned reports with one decoded report graph at a time.

    Both original UTF-8 bodies stay in the authenticated containing record. The
    before graph receives only the exact source/executable substitutions, then
    its canonical digest survives release. The after graph must match that full
    digest and every declared mapping. Parse operations and the surviving digest
    remain charged; malformed bodies, drift or unapproved mappings reject.
    """
    if (type(value) is not dict or set(value) != {'schema', 'tool_source', 'pairs'}
            or type(value['schema']) is not int or value['schema'] != 1
            or (value['tool_source'] not in tool_pins
                and not (generator_source == NATIVE_GENERATOR_LOGICAL
                         and value['tool_source'] == NATIVE_GENERATOR_LOGICAL))
            or value['tool_source']['path'] != str(TOOLS/'pa1_fixed40.py')
            or type(value['pairs']) is not list or len(value['pairs']) != 2):
        raise ValueError('selected40 actual native/firstD comparator schema')
    if len({row['path'] for row in source_pins}) != len(source_pins):
        raise ValueError('selected40 duplicate source freeze paths')
    for i, (pair, fields) in enumerate(zip(value['pairs'], (NATIVE_FIELDS, FIRST_FIELDS))):
        if (set(pair) != {'profile', 'before', 'after', 'before_body_utf8', 'after_body_utf8', 'mappings'}
                or pair['profile'] != ('native-current', 'first-d-current')[i]
                or pair['before'] != BEFORE[i]
                or pair['after'] != native_reports[i]
                or [row.get('field') for row in pair['mappings']] != list(fields)):
            raise ValueError('selected40 exact complete native comparison identities')
        before = _projection_body(pair['before_body_utf8'].encode(), pair['before'],
                                  lambda raw:_inline_decode(unit, occurrence, raw, live))
        for mapping in pair['mappings']:
            a, ak = _location(before, mapping['field'])
            if (set(mapping) != {'field', 'predecessor', 'successor'}
                    or mapping['predecessor'] != a[ak]):
                raise ValueError('selected40 complete native mapping body')
            if mapping['field'].endswith('/executable'):
                expected = executable
            else:
                original = a[ak]
                path = str((REPO/'experiments/evm_feasibility/snapshot_qualifier/src'/original['path']).resolve())
                source = next((row for row in source_pins if row['path'] == path), None)
                if source is None:
                    raise ValueError('selected40 finite source mapping identity')
                expected = dict(source, path=original['path'])
                del original, source, path
            if mapping['successor'] != expected:
                raise ValueError('selected40 finite source/executable mapping')
            a[ak] = expected
        before_hash = core.digest(before)
        del before, a, ak, expected
        # No before graph is captured by the decoder or retained beside after.
        after = _projection_body(pair['after_body_utf8'].encode(), pair['after'],
                                 lambda raw:_inline_decode(unit, occurrence, raw,
                                                           live+graph_charge(before_hash)))
        for mapping in pair['mappings']:
            b, bk = _location(after, mapping['field'])
            if mapping['successor'] != b[bk]:
                raise ValueError('selected40 complete native mapping body')
        if before_hash != core.digest(after):
            raise ValueError('selected40 complete current report inequality')
        del after, b, bk, before_hash


HISTORICAL_TEN19_ACCEPTANCE = dict(
    path=str(BASE/'vrf40-admission-01a14b66-20261018/native-ten-frozen-acceptance.json'),
    bytes=2821, sha256='1bc1edd4924538f692faa892d4cd8badba547db7b2ce773530c5eb8cb340e07f')
HISTORICAL_TEN19_SOURCES = dict(
    path=str(BASE/'vrf40-admission-01a14b66-20261018/recovery46-rust-source-freeze.json'),
    bytes=17908, sha256='be283dccbab3448bb727749b83615ac3326cbfe4ae0cb44772d5a43d6baf48ae')
HISTORICAL_TEN19_WRAPPER = dict(
    path=str(BASE/'vrf40-admission-01a14b66-20261018/saved-ten-input-v5.json'),
    bytes=1503, sha256='b1b94bc353e45a27521acc3a03f0eda8fc4b6fd1e3bcfb4a9ad2903c807a400c')


def historical_gate_scope(scope_pin, inputs, *, live_bytes=0):
    """Bind historical comparisons without lending their verdict to current Rust.

    This preparation reads the complete, pinned historical component and roster.
    Every control and every original source body is also declared for a charged
    reread on each fit route. An alias changes only the physical retained path;
    the logical identity, order, length and digest remain historical.
    """
    _pin_tuple(scope_pin)
    core.integer(live_bytes, core.SEMANTIC, 'selected40 historical scope caller')
    if (scope_pin['bytes'] > 32768
            or live_bytes+scope_pin['bytes']+2*core.CHUNK+262144 > core.SEMANTIC):
        raise ValueError('selected40 historical scope pre-read allocation')
    raw = _read(scope_pin)
    if (len(raw) > 32768 or live_bytes+3*len(raw)+160*core._wire_nodes(raw)
            +core.CHUNK+262144 > core.SEMANTIC):
        raise ValueError('selected40 historical scope predecode allocation')
    scope = records.decode(raw, 32768)
    fields = {'schema','acceptance','source_freeze','physical_sources',
              'current_source_freeze','current_tool_freeze','current_gate_state'}
    if (type(scope) is not dict or set(scope) != fields
            or scope['schema'] != 'fixed40-offline-historical-gate-scope-v1'
            or scope['acceptance'] != HISTORICAL_TEN19_ACCEPTANCE
            or scope['source_freeze'] != HISTORICAL_TEN19_SOURCES
            or scope['current_source_freeze'] != inputs['source_freeze']
            or scope['current_tool_freeze'] != inputs['tool_freeze']
            or scope['current_gate_state'] != 'PROSPECTIVE_NOT_EXECUTED_NOT_ACCEPTED'):
        raise ValueError('selected40 separate historical/current gate scopes')
    source_raw = bytes(_read(HISTORICAL_TEN19_SOURCES))
    acceptance_raw = bytes(_read(HISTORICAL_TEN19_ACCEPTANCE))
    caller = live_bytes+graph_charge((raw,scope,source_raw,acceptance_raw))
    if (caller+3*(len(source_raw)+len(acceptance_raw))
            +160*(core._wire_nodes(source_raw)+core._wire_nodes(acceptance_raw))
            +core.CHUNK+262144 > core.SEMANTIC):
        raise ValueError('selected40 historical full component predecode')
    sources = records.decode(source_raw, records.WRAPPER_CAP)
    # This one exact operational acceptance contains a finite elapsed-time
    # scalar. Protocol reports continue to use the strict integer decoder.
    acceptance = json.loads(acceptance_raw, object_pairs_hook=records.unique_object)
    if (type(sources) is not list or len(sources) != 90
            or type(scope['physical_sources']) is not list
            or len(scope['physical_sources']) != 90
            or acceptance.get('accepted') is not True
            or acceptance['reports']['count'] != 19
            or set(acceptance['success_roster']) != set(SAVED_TESTS)):
        raise ValueError('selected40 complete accepted historical comparison scope')
    for logical, physical in zip(sources, scope['physical_sources']):
        _pin_tuple(logical); _pin_tuple(physical)
        if (physical['bytes'] != logical['bytes'] or physical['sha256'] != logical['sha256']
                or physical['path'] != logical['path']
                and not physical['path'].startswith(str(BASE)+'/')):
            raise ValueError('selected40 exact historical source-body alias')
    if inputs['saved_result'] != HISTORICAL_TEN19_WRAPPER:
        raise ValueError('selected40 exact historical gate wrapper')
    result = json.loads(_read(inputs['saved_result']), object_pairs_hook=records.unique_object)
    execution = result['execution_record']; _pin_tuple(execution)
    if (any(execution[k] != acceptance['result'][k] for k in ('bytes','sha256'))
            or any(inputs['saved_log'][k] != acceptance['full_log'][k] for k in ('bytes','sha256'))
            or result['log'] != inputs['saved_log']):
        raise ValueError('selected40 accepted historical gate result/log identity')
    executable = result['test_executable']
    _executable_value(executable)
    if (result['source_freeze'] != HISTORICAL_TEN19_SOURCES
            or inputs['test_executable'] != executable
            or executable != acceptance['executable']):
        raise ValueError('selected40 historical executable/source binding')
    return dict(pin=scope_pin, source_freeze=HISTORICAL_TEN19_SOURCES,
                acceptance=HISTORICAL_TEN19_ACCEPTANCE, sources=sources,
                physical_sources=scope['physical_sources'], executable=executable,
                execution_record=execution,
                current_gate_state=scope['current_gate_state'])


def semantic_units(compiled, inputs, *, offline_scope=None, include_historical=False,
                   live_bytes=None):
    """Freeze the exact extra semantic/read closures before durable admission.

    Inputs are owner-frozen current records, not profile/limit overrides. Native
    whole reports and source hashes remain actual prerequisites. Future source
    acceptance, all-package gates and runtime effects stay pending at full caps.
    """
    if live_bytes is None:live_bytes=compiled.get('bridge_caller_charge',0)
    core.integer(live_bytes,core.SEMANTIC,'selected40 semantic preparation caller')
    if live_bytes+graph_charge((compiled,inputs,offline_scope))+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 semantic preparation entry lifetime')
    if type(inputs) is not dict or set(inputs) != INPUT_FIELDS:
        raise IncompleteFit('actual source/tool freezes, exact saved gate/log, full native comparison '
                            'and current test executable are required')
    for key, value in inputs.items():
        if key == 'test_executable':
            _executable_value(value)
        elif key not in ('shared_reports', 'native_reports'):
            _pin_tuple(value)
    if type(inputs['shared_reports']) is not tuple or len(inputs['shared_reports']) != 17:
        raise IncompleteFit('all17 retained full current shared reports are required')
    for index, pin in enumerate(inputs['shared_reports']):
        _pin_tuple(pin)
        if (pin['path'] not in tuple(str(root/('shared-profile-'+str(index)+'.json'))
                                    for root in (ROOT, SAVED_REPORT_ROOT))
                or pin['bytes'] > 65536):
            raise ValueError('selected40 exact bounded retained shared report')
    if type(inputs['native_reports']) is not tuple or len(inputs['native_reports']) != 2:
        raise IncompleteFit('both retained full current native/firstD reports are required')
    for index, pin in enumerate(inputs['native_reports']):
        _pin_tuple(pin)
        stem = ('native-current-replay', 'first-d-current-replay')[index]
        if (pin['path'] not in tuple(str(root/(stem+suffix+'.json'))
                                    for root in (ROOT, SAVED_REPORT_ROOT) for suffix in ('', '-v2'))
                or pin['bytes'] > 65536):
            raise ValueError('selected40 explicit native report version')
    if sum(pin['bytes'] for pin in inputs['shared_reports']+inputs['native_reports']) > 1327104:
        raise ValueError('selected40 full auxiliary outputs cap')
    expected_sources, expected_tools = _expected_sources(compiled)
    def source_roster(pin,retained=()):
        owner=live_bytes+graph_charge((compiled,inputs,offline_scope,retained))
        if owner+3*pin['bytes']+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('selected40 source roster preread lifetime')
        raw=_read(pin)
        if owner+2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('selected40 source roster predecode lifetime')
        return records.decode(raw,records.WRAPPER_CAP)
    sources=source_roster(inputs['source_freeze'])
    tools=source_roster(inputs['tool_freeze'],sources)
    if (type(sources) is not list or type(tools) is not list
            or tuple(row['path'] for row in sources) != expected_sources
            or tuple(row['path'] for row in tools) != expected_tools):
        raise ValueError('selected40 complete ordered current source closure')
    if sum(row['bytes'] for row in sources+tools) > core.SOURCE:
        raise ValueError('selected40 complete current source/tool aggregate')
    for row in sources+tools:
        if row['bytes'] > records.BODY_CAP:
            raise ValueError('selected40 individual current source cap')
    if type(include_historical) is not bool:
        raise ValueError('selected40 explicit historical-context return')
    historical = (None if offline_scope is None else historical_gate_scope(
        offline_scope, inputs, live_bytes=live_bytes+graph_charge((compiled,inputs,sources,tools))))
    extra = []
    instructions = []
    occurrence = 682
    for name in ('source_freeze', 'tool_freeze'):
        extra.append(((occurrence, _pin_tuple(inputs[name]), 'evidence', 'json'),))
        instructions.append(('input', name))
        occurrence += 1
    extra.append(((occurrence, _pin_tuple(inputs['saved_result']), 'evidence', 'bytes'),
                  (occurrence+1, _pin_tuple(inputs['saved_log']), 'evidence', 'bytes')))
    instructions.append(('gate', None)); occurrence += 2
    if compiled.get('runtime_admission'):
        # Preserve the accepted constructor's original source identity. Its
        # complete old source is reauthenticated before the unchanged full-body
        # comparison; never relabel the101751-byte input as newly generated.
        extra.append(((occurrence, _pin_tuple(NATIVE_GENERATOR_RETAINED), 'source', 'bytes'),))
        instructions.append(('native_generator_source', None)); occurrence += 1
    extra.append(((occurrence, _pin_tuple(inputs['native_comparison']), 'evidence', 'json'),))
    instructions.append(('input', 'native_comparison')); occurrence += 1
    # The original fixed42 authority is a semantic input, not a hash-only cache.
    seal = dict(path=str(BASE/'vrf-round3-01a141ba-20261016/next42/fact-outcome-freeze.json'),
                bytes=6818, sha256='eb18e6b3b228a0500a69eb3f511ac320b0a5bf677eb602962c1a22f38a930e1b')
    extra.append(((occurrence, _pin_tuple(seal), 'evidence', 'json'),))
    instructions.append(('seal', None)); occurrence += 1
    parse = compiled['context']['historical_equivalence_parse']
    extra.append(((occurrence, _pin_tuple(parse['pin']), 'evidence', 'json'),))
    instructions.append(('historical_projection', None)); occurrence += 1
    for index, row in enumerate(sources+tools):
        extra.append(((occurrence, _pin_tuple(row), 'source', 'bytes'),))
        instructions.append(('source', index)); occurrence += 1
    for index, after in enumerate(inputs['shared_reports']):
        old = {k:SHARED_BEFORE[index][k] for k in ('path', 'bytes', 'sha256')}
        extra.append(((occurrence, _pin_tuple(old), 'evidence', 'json'),
                      (occurrence+1, _pin_tuple(after), 'evidence', 'json')))
        instructions.append(('shared_pair', index)); occurrence += 2
    extra.append(((occurrence, _pin_tuple(compiled['context']['scope36_pin']), 'evidence', 'json'),))
    instructions.append(('scope36', None)); occurrence += 1
    for pin in inputs['native_reports']:
        extra.append(((occurrence, _pin_tuple(pin), 'evidence', 'bytes'),))
        instructions.append(('native_body', None)); occurrence += 1
    # Exact logical64 child slots are retained separately from current gates.
    original = {}
    for raw in compiled['units']:
        for row in records.decode(raw, records.MANIFEST_CAP):
            if 524 <= row[0] < 588:
                original[row[0]] = (row[0], tuple(row[1]), row[2], row[3])
    offset = 524
    for role, placement in zip((28, 37, 49), compiled['context']['reference_placements']):
        rows = []
        expected = [dict(role='trusted-contract', **placement['common_contract']), *placement['references']]
        for row in expected:
            declaration = original[offset]
            logical = _pin_tuple({k:row[k] for k in ('path', 'bytes', 'sha256')})
            physical = declaration[1]
            if physical != logical:
                accepted = [a for a in compiled['aliases']
                            if a['occurrence'] == offset and _pin_tuple(a['logical']) == logical
                            and _pin_tuple(a['physical']) == physical]
                if len(accepted) != 1:
                    raise ValueError('selected40 exact accepted original child alias')
            rows.append(declaration); offset += 1
        extra.append(tuple(rows)); instructions.append(('record', role))
    if offset != 588:
        raise ValueError('selected40 all64 child semantic closure')
    # Reauthenticate the exact alias amendment and its focused review on every
    # prescribed route; source binding is not a body or control-read waiver.
    for pin in (RETAINED_CHILD_ALIAS_PIN,RETAINED_CHILD_ALIAS_REVIEW):
        extra.append(((occurrence,_pin_tuple(pin),'evidence','bytes'),))
        instructions.append(('retained_child_control',None));occurrence+=1
    if historical is not None:
        for pin in (historical['pin'], historical['source_freeze'], historical['acceptance'],
                    historical['execution_record']):
            extra.append(((occurrence,_pin_tuple(pin),'evidence','bytes'),))
            instructions.append(('historical_gate_control',None));occurrence+=1
        for logical, physical in zip(historical['sources'], historical['physical_sources']):
            extra.append(((occurrence,_pin_tuple(physical),'source','bytes'),))
            instructions.append(('historical_gate_source',_pin_tuple(logical)));occurrence+=1
    result = tuple(extra), tuple(instructions), sources, tools
    return (*result, historical) if include_historical else result


def verify_record_unit(unit, placement, role, declarations, prefix='', live_bytes=0, identities=None):
    """Run the original record verifier against the same core-consumed streams."""
    extra_charge = graph_charge(placement)+16384+3*65536
    if (unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes+extra_charge
            +core.CHUNK+262144 > core.SEMANTIC):
        raise ValueError('selected40 wrapper pre-allocation')
    operation = 1024+role
    if unit.invocation.operations[operation] >= 8:
        raise ValueError('selected40 ninth generated wrapper parse')
    unit.invocation.operations[operation] += 1
    value = wrapper_specimen(placement, str(ROOT/('current54-'+str(role)+'.json')), prefix)
    rows = [dict(role='trusted-contract', **placement['common_contract']), *placement['references']]
    identities = {} if identities is None else identities
    position = 0
    original = records.authenticate
    def consume(row, identities=None):
        nonlocal position
        if position >= len(rows) or row != rows[position]:
            raise ValueError('selected40 exact ordered child semantics')
        occurrence, pin, kind, mode = declarations[position]
        if mode != 'bytes':
            raise ValueError('selected40 original child stream mode')
        unit.consume(occurrence, _pin(pin), kind=kind, live_bytes=live_bytes+graph_charge(value))
        identity = unit.last_file_identity
        location = pin[0]
        if identity in identities and identities[identity] != location:
            raise ValueError('selected40 original record inode/path alias')
        identities[identity] = location
        position += 1
        return row['bytes']
    records.authenticate = consume
    try:
        result = records.verify(value, value['destination'], placement['common_contract'], placement['references'])
    finally:
        records.authenticate = original
    if position != len(rows):
        raise ValueError('selected40 complete child semantics')
    return result


def graph_charge(value):
    """Charge each live semantic occurrence, including shared strings/children."""
    if isinstance(value, str):
        return 128+len(value.encode())
    if isinstance(value, (bytes, bytearray)):
        return 128+len(value)
    if isinstance(value, dict):
        return 128+sum(graph_charge(k)+graph_charge(v) for k,v in value.items())
    if isinstance(value, (tuple, list)):
        return 128+sum(graph_charge(v) for v in value)
    return 128


def _inline_decode(unit, occurrence, raw, live):
    """Count nested report parses from the authenticated comparator body."""
    owner = unit.invocation
    count = owner.operations[occurrence]+1
    if count > 8:
        raise ValueError('selected40 ninth nested report operation')
    owner.operations[occurrence] = count
    nodes = 0
    quoted = escaped = scalar = False
    for byte in raw:
        if quoted:
            if escaped: escaped = False
            elif byte == 92: escaped = True
            elif byte == 34: quoted = False
        elif byte == 34:
            quoted = True; scalar = False; nodes += 1
        elif byte in (91, 123):
            nodes += 1; scalar = False
        elif byte in (9, 10, 13, 32, 44, 58, 93, 125):
            scalar = False
        elif not scalar:
            nodes += 1; scalar = True
        if nodes > records.NODE_CAP:
            raise ValueError('selected40 inline report preparse nodes')
    if (owner.metadata_charge()+unit.declaration_charge+live+len(raw)*2+nodes*128
            +core.CHUNK+262144 > core.SEMANTIC):
        raise ValueError('selected40 inline report preparse allocation')
    return records.decode(raw, records.WRAPPER_CAP)


def verify_shared_pair(before, after, index, sources, executable):
    """Preserve all17 complete old reports with only the existing exact mappings."""
    before['authority']['six_input_owner']['executable'] = executable
    if after['authority']['six_input_owner']['executable'] != executable:
        raise ValueError('selected40 current shared executable binding')
    for old, new in ((before['vrf_sources'], after['vrf_sources']),
                     (before['authority']['six_input_owner']['sources'], after['authority']['six_input_owner']['sources'])):
        if len(old) != len(new):
            raise ValueError('selected40 complete shared source roster')
        for position, (a, b) in enumerate(zip(old, new)):
            path = a['path']
            successor = path
            allowed = path in STORAGE_PATHS
            if path == '../Cargo.toml':
                if old is not before['authority']['six_input_owner']['sources'] or position != 14:
                    raise ValueError('selected40 exact Cargo provenance position')
                allowed = True
            if index < 10 and path == 'lib.rs' and index != 6:
                allowed = True
            if path == SHARED_BEFORE[index]['adapter']:
                successor = 'vrf_shared_window.rs'; allowed = True
            if allowed:
                absolute = str((REPO/'experiments/evm_feasibility/snapshot_qualifier/src'/successor).resolve())
                source = next((row for row in sources if row['path'] == absolute), None)
                if source is None:
                    raise ValueError('selected40 shared source freeze entry')
                expected = dict(source, path=successor)
                if b != expected:
                    raise ValueError('selected40 exact shared source successor')
                old[position] = expected
            elif a != b:
                raise ValueError('selected40 unchanged shared source required')
    if core.digest(before) != core.digest(after):
        raise ValueError('selected40 complete shared report inequality')


def framing_routes():
    """Return every prescribed logical route, in its original order."""
    return tuple(route+':'+str(i) for route, count in core.ROUTES.items()
                 for i in range(count))


ARCHIVE_CONTEXT_FIELDS=frozenset(('reference_placements','historical_equivalence_parse',
    'correction_record48','source28_prefix','scope36_pin','retention_amendment'))


def archive_context_wires(compiled,instructions,sources,tools,*,live_bytes):
    """Encode complete archive predicates for ownership transfer, not acceptance.

    Every field and instruction remains present. The caller must release its
    decoded inputs after transferring these immutable wires, and must still
    execute every prescribed full-body read on each route. Later phases can
    release the entire completed archive context instead of retaining it beside
    unrelated future constructors. This function grants no fit authority.
    """
    core.integer(live_bytes,core.SEMANTIC,'selected40 archive transfer caller')
    if (type(compiled) is not dict or set(compiled['context'])!=ARCHIVE_CONTEXT_FIELDS
            or type(instructions) not in (list,tuple)
            or type(sources) is not list or len(sources)!=90
            or type(tools) is not list or len(tools)!=11):
        raise ValueError('selected40 complete archive transfer inputs')
    inputs=(compiled,instructions,sources,tools)
    if live_bytes+graph_charge(inputs)+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 archive transfer initial allocation')
    context={};opcodes=[];completed=[]
    def encode(value):
        encoder=json.JSONEncoder(sort_keys=True,separators=(',',':'),ensure_ascii=True,allow_nan=False)
        width=1+sum(len(piece) for piece in encoder.iterencode(value))
        if width>records.WRAPPER_CAP:
            raise ValueError('selected40 complete archive field wire cap')
        if (live_bytes+graph_charge((inputs,context,opcodes,completed))+2*width
                +core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 archive field encoding preallocation')
        raw=core.canonical(value)
        if len(raw)!=width:raise ValueError('selected40 archive exact field width')
        if (live_bytes+graph_charge((inputs,context,opcodes,completed))+2*width
                +128*core._wire_nodes(raw)+core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 archive field validation preallocation')
        # Canonical JSON normalizes the compiler's immutable tuple columns.
        # Validate every complete field with the original decoder constraints.
        decoded=records.decode(raw,records.WRAPPER_CAP)
        del decoded
        return raw
    for key in sorted(ARCHIVE_CONTEXT_FIELDS):context[key]=encode(compiled['context'][key])
    for row in instructions:
        if type(row) not in (tuple,list) or len(row)!=2:
            raise ValueError('selected40 complete archive instruction')
        opcodes.append(encode(row))
    for value in (sources,tools,compiled['schedule']):completed.append(encode(value))
    if (live_bytes+graph_charge((inputs,context,opcodes,completed))
            +128*(len(opcodes)+8)+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 archive transfer result preallocation')
    return dict(schema='fixed40-complete-archive-context-wires-v1',context=context,
        instructions=tuple(opcodes),sources=completed[0],tools=completed[1],schedule=completed[2])


def archive_context_value(storage,field,*,live_bytes,index=None):
    """Decode only the active archive predicate with its complete caller live.

    live_bytes includes the full owner, storage, external graphs and active unit
    declarations. Unit readers independently add owner metadata, so their caller
    argument must instead contain only external growth. No source read is waived.
    """
    core.integer(live_bytes,core.SEMANTIC,'selected40 active archive field caller')
    if (type(storage) is not dict or set(storage)!=
            {'schema','context','instructions','sources','tools','schedule'}
            or storage['schema']!='fixed40-complete-archive-context-wires-v1'
            or set(storage['context'])!=ARCHIVE_CONTEXT_FIELDS
            or type(storage['instructions']) is not tuple
            or live_bytes<graph_charge(storage)):
        raise ValueError('selected40 complete retained archive storage')
    if field=='instruction':
        core.integer(index,len(storage['instructions'])-1,'selected40 archive instruction index')
        raw=storage['instructions'][index]
    elif index is not None:
        raise ValueError('selected40 archive field has no instruction index')
    elif field in ARCHIVE_CONTEXT_FIELDS:raw=storage['context'][field]
    elif field in ('sources','tools','schedule'):raw=storage[field]
    else:raise ValueError('selected40 closed archive field')
    if (type(raw) is not bytes or len(raw)>records.WRAPPER_CAP
            or live_bytes+2*len(raw)+128*core._wire_nodes(raw)
                +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 active archive field predecode allocation')
    return records.decode(raw,records.WRAPPER_CAP)


def framing_wire_pool(route_manifests, *, live_bytes):
    """Intern complete identical wires while preserving every logical occurrence.

    This is a storage encoding only. Every expanded route keeps the original
    manifest digest and every body/parse/EOF operation. The input is consumed
    one route at a time; callers must include the producer's retained graphs in
    live_bytes. A complete preflight must also include all non-wire components.
    """
    core.integer(live_bytes, core.SEMANTIC, 'selected40 framing caller')
    pool, routes, keys = [], [], {}
    expected = framing_routes()
    for position, row in enumerate(route_manifests):
        if position >= len(expected) or type(row) is not tuple or len(row) != 3:
            raise ValueError('selected40 exact framing route')
        route, ids, wires = row
        if (route != expected[position] or type(ids) is not tuple
                or type(wires) is not tuple or len(ids) != len(wires)
                or not 1 <= len(ids) <= core.INVOCATION_UNITS):
            raise ValueError('selected40 ordered complete route units')
        # The original invocation accounts for each logical occurrence even
        # when the immutable byte objects happen to share storage.
        current = graph_charge((row, pool, routes, keys))
        if live_bytes+current+2*core.CHUNK+262144 > core.SEMANTIC:
            raise ValueError('selected40 framing pool caller allocation')
        references = []
        for identity, raw in zip(ids, wires):
            if (type(identity) is not str or not identity
                    or len(identity.encode()) > 1024 or type(raw) is not bytes
                    or not raw or len(raw) > records.MANIFEST_CAP):
                raise ValueError('selected40 complete bounded unit wire')
            key = (len(raw), hashlib.sha256(raw).hexdigest())
            candidates = keys.get(key, ())
            index = next((i for i in candidates if pool[i] == raw), None)
            growth=512+len(identity.encode())
            if index is None:
                growth+=1024+len(raw)+128*len(candidates)
            if (live_bytes+graph_charge((row,pool,routes,keys,references))+growth
                    +2*core.CHUNK+262144 > core.SEMANTIC):
                raise ValueError('selected40 framing pregrowth allocation')
            if index is None:
                # A hash match alone never substitutes for full byte equality.
                index = len(pool)
                pool.append(raw)
                keys[key] = (*candidates, index)
            references.append((identity, index))
            if (live_bytes+graph_charge((row,pool,routes,keys,references))
                    +2*core.CHUNK+262144 > core.SEMANTIC):
                raise ValueError('selected40 framing growth allocation')
        wire_max=4096+sum(12*len(identity.encode())+80
                          for identity,index in references)
        if (live_bytes+graph_charge((row,pool,routes,keys,references))
                +2*wire_max+2*core.CHUNK+262144 > core.SEMANTIC):
            raise ValueError('selected40 framing index serialization allocation')
        wire = core.canonical((route, references,
                               core.frozen_manifest_digest(ids,wires)))
        if len(wire) > records.MANIFEST_CAP:
            raise ValueError('selected40 complete route index cap')
        routes.append(wire)
        del references, wire, row, ids, wires
    if len(routes) != len(expected):
        raise ValueError('selected40 all sixteen framing routes')
    if (live_bytes+graph_charge((pool,routes,keys))
            +128*(len(pool)+len(routes))+512+2*core.CHUNK+262144
                >core.SEMANTIC):
        raise ValueError('selected40 framing result preallocation')
    result = dict(schema='fixed40-lossless-unit-wire-storage-v1',
                  wires=tuple(pool), routes=tuple(routes),
                  complete_preflight=False)
    if (live_bytes+graph_charge((result,pool,routes,keys))
            +2*core.CHUNK+262144 > core.SEMANTIC):
        raise ValueError('selected40 framing result allocation')
    return result


def expand_framing_route(storage, position, *, live_bytes):
    """Restore exact ordered wires and authenticate their full route digest.

    Retained pool, index wires, decoded index and expanded reference tuple all
    count beside the caller before returning. This grants no semantic authority.
    """
    core.integer(position,15,'selected40 framing route index')
    core.integer(live_bytes,core.SEMANTIC,'selected40 framing expansion caller')
    if (type(storage) is not dict or set(storage) !=
            {'schema','wires','routes','complete_preflight'}
            or storage['schema'] != 'fixed40-lossless-unit-wire-storage-v1'
            or storage['complete_preflight'] is not False
            or type(storage['wires']) is not tuple
            or type(storage['routes']) is not tuple or len(storage['routes'])!=16):
        raise ValueError('selected40 exact framing storage')
    raw=storage['routes'][position]
    caller=live_bytes+graph_charge(storage)
    if (type(raw) is not bytes or len(raw)>records.MANIFEST_CAP
            or caller+2*len(raw)+128*core._wire_nodes(raw)+2*core.CHUNK+262144
                >core.SEMANTIC):
        raise ValueError('selected40 framing index predecode allocation')
    value=records.decode(raw,records.MANIFEST_CAP)
    if (type(value) is not list or len(value)!=3
            or value[0]!=framing_routes()[position]
            or type(value[1]) is not list
            or not 1<=len(value[1])<=core.INVOCATION_UNITS):
        raise ValueError('selected40 exact expanded route')
    ids,wires=[],[]
    for entry in value[1]:
        if (type(entry) is not list or len(entry)!=2
                or type(entry[0]) is not str or not entry[0]
                or len(entry[0].encode())>1024 or type(entry[1]) is not int
                or not 0<=entry[1]<len(storage['wires'])):
            raise ValueError('selected40 framing reference identity')
        wire=storage['wires'][entry[1]]
        if type(wire) is not bytes or not wire or len(wire)>records.MANIFEST_CAP:
            raise ValueError('selected40 framing referenced wire')
        if (caller+graph_charge((value,ids,wires))+len(wire)
                +len(entry[0].encode())+768+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 expanded reference preallocation')
        ids.append(entry[0]);wires.append(wire)
        if (caller+graph_charge((value,ids,wires))+2*core.CHUNK+262144
                >core.SEMANTIC):
            raise ValueError('selected40 expanded route allocation')
    if (caller+graph_charge((value,ids,wires))*2+2*core.CHUNK+262144
            >core.SEMANTIC):
        raise ValueError('selected40 expanded tuple allocation')
    ids,wires=tuple(ids),tuple(wires)
    if core.frozen_manifest_digest(ids,wires)!=value[2]:
        raise ValueError('selected40 full expanded manifest digest')
    return ids,wires


COMPLETE_FIT_SECTIONS = (
    'header','authority','history','current_sources','historical_scope','prior83',
    'future81','unit_pool','routes','controls','receipts','recovery',
    'future_maxima','binary_schedule','lifetimes')


def pack_complete_fit_units(ids,wires,*,live_bytes):
    """Losslessly index full pins while retaining every occurrence and unit.

    Inner unit wires remain individual canonical JSON strings. Thus their
    original per-record node/depth limits apply independently; the containing
    record never grows a recursively decoded all-unit declaration graph.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 indexed packing caller')
    if (type(ids) is not tuple or type(wires) is not tuple or len(ids)!=len(wires)
            or any(type(raw) is not bytes or not 0<len(raw)<=records.MANIFEST_CAP for raw in wires)):
        raise ValueError('fixed40 complete ordered unit pool input')
    pins=[];modes=[];units=[];index={};kinds=('source','evidence','binary')
    for identity,raw in zip(ids,wires):
        if (live_bytes+graph_charge((ids,wires,pins,modes,units,index))
                +2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete pool packing lifetime')
        rows=records.decode(raw,records.MANIFEST_CAP);compact=[]
        if core.canonical(rows)!=raw:raise ValueError('fixed40 exact original unit wire')
        for occurrence,pin,kind,mode in rows:
            pin_width,_=_framing_json_width(pin)
            growth=graph_charge((pin,mode))+2048+2*(pin_width+1)
            if (live_bytes+graph_charge((ids,wires,pins,modes,units,index,rows,compact))
                    +growth+2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('fixed40 complete pool index pregrowth lifetime')
            key=core.digest(pin)
            position=index.get(key)
            if position is None:
                position=len(pins);pins.append(pin);index[key]=position
            elif pins[position]!=pin:raise ValueError('fixed40 full pin index collision')
            if mode not in modes:modes.append(mode)
            compact.append([occurrence,position,kinds.index(kind),modes.index(mode)])
        inner=[identity,compact]
        width,_=_framing_json_width(inner)
        if (live_bytes+graph_charge((ids,wires,pins,modes,units,index,rows,inner))
                +2*(width+1)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete compact unit encoding lifetime')
        units.append(core.canonical(inner).decode('ascii'))
        del rows,compact,inner
    value=dict(schema='fixed40-lossless-complete-unit-pool-v1',pins=pins,
        kinds=list(kinds),modes=modes,units=units)
    records.bounded_shape(value)
    if _framing_json_width(['unit_pool',value])[0]+1>records.MANIFEST_CAP:
        raise ValueError('fixed40 complete indexed pool record cap')
    return value


def expand_complete_fit_units(pool,*,live_bytes):
    """Expand every indexed field and preserve canonical original wire bytes."""
    core.integer(live_bytes,core.SEMANTIC,'fixed40 indexed expansion caller')
    if (type(pool) is not dict or set(pool)!={'schema','pins','kinds','modes','units'}
            or pool['schema']!='fixed40-lossless-complete-unit-pool-v1'
            or pool['kinds']!=['source','evidence','binary']
            or any(type(pool[key]) is not list for key in ('pins','modes','units'))
            or not 0<len(pool['units'])<=core.INVOCATION_UNITS):
        raise ValueError('fixed40 complete indexed pool schema')
    records.bounded_shape(pool)
    for pin in pool['pins']:_pin_tuple(_pin(pin))
    ids=[];wires=[]
    for text in pool['units']:
        if type(text) is not str or not text.isascii():raise ValueError('fixed40 indexed unit encoding')
        if (live_bytes+graph_charge((pool,ids,wires))+3*len(text)
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete pool expansion predecode lifetime')
        raw=text.encode('ascii')
        if (live_bytes+graph_charge((pool,ids,wires,raw))+2*len(raw)
                +160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete indexed unit parse lifetime')
        value=records.decode(raw,records.MANIFEST_CAP)
        if (type(value) is not list or len(value)!=2 or core.canonical(value)!=raw
                or type(value[0]) is not str or type(value[1]) is not list or not value[1]):
            raise ValueError('fixed40 complete indexed unit fields')
        rows=[]
        for row in value[1]:
            if type(row) is not list or len(row)!=4 or any(type(n) is not int for n in row):
                raise ValueError('fixed40 indexed occurrence shape')
            occurrence,pin_index,kind_index,mode_index=row
            core.integer(occurrence,2047,'fixed40 indexed occurrence')
            core.integer(pin_index,len(pool['pins'])-1,'fixed40 indexed pin')
            core.integer(kind_index,2,'fixed40 indexed kind')
            core.integer(mode_index,len(pool['modes'])-1,'fixed40 indexed mode')
            growth=graph_charge(pool['pins'][pin_index])+graph_charge(pool['kinds'][kind_index])
            growth+=graph_charge(pool['modes'][mode_index])+768
            if (live_bytes+graph_charge((pool,ids,wires,value,rows,raw))
                    +growth+2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('fixed40 complete expanded row pregrowth lifetime')
            rows.append([occurrence,pool['pins'][pin_index],pool['kinds'][kind_index],pool['modes'][mode_index]])
        width,_=_framing_json_width(rows)
        if (width+1>records.MANIFEST_CAP
                or live_bytes+graph_charge((pool,ids,wires,value,rows,raw))
                    +2*(width+1)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 expanded unit encoding lifetime')
        ids.append(value[0]);wires.append(core.canonical(rows))
        del raw,value,rows
    if (live_bytes+graph_charge((pool,ids,wires))+sum(graph_charge(identity)+256 for identity in ids)
            +128*(len(ids)+len(wires))+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 complete pool tuple/set pregrowth lifetime')
    if len(set(ids))!=len(ids):raise ValueError('fixed40 complete unique unit IDs')
    return tuple(ids),tuple(wires)


def _framing_json_width(value):
    """Measure the complete ensure_ascii graph before allocating an encoding."""
    nodes=0;largest=4
    def width(item,depth):
        nonlocal nodes,largest
        nodes+=1
        if depth>8 or nodes>8192:raise ValueError('selected40 framing JSON shape')
        if type(item) is str:
            n=2
            for char in item:
                code=ord(char)
                n+=(2 if char in '\"\\\b\f\n\r\t' else
                    1 if 32<=code<127 else 6 if code<=65535 else 12)
                if n>records.MANIFEST_CAP:raise ValueError('selected40 framing string width')
            largest=max(largest,n+4);return n
        if item is None:return 4
        if type(item) is bool:return 4 if item else 5
        if type(item) is int:
            if not -(1<<63)<=item<(1<<64):raise ValueError('selected40 framing integer')
            return len(str(item))
        if type(item) in (list,tuple):
            return 2+max(0,len(item)-1)+sum(width(x,depth+1) for x in item)
        if type(item) is dict:
            if any(type(k) is not str for k in item):raise ValueError('selected40 framing object keys')
            return 2+max(0,len(item)-1)+sum(width(k,depth+1)+1+width(v,depth+1) for k,v in item.items())
        raise ValueError('selected40 framing JSON type')
    return width(value,0),largest


def write_complete_fit_representation(destination, sections, *, live_bytes, owner_charge=None):
    """Write one complete framed representation, with bounded encoding scratch.

    Sections are supplied lazily in the closed fixed40 order. Large immutable
    bodies remain their original authenticated inputs; this representation
    contains their complete occurrence declarations and executable bindings.
    A descriptor can name unresolved future identities but cannot assert that
    their producer, proof or acceptance has run. Independent review remains a
    separate prerequisite for any binary-reading gate or fit execution.
    """
    core.integer(live_bytes,core.SEMANTIC,'selected40 representation caller')
    if owner_charge is not None and not callable(owner_charge):
        raise ValueError('selected40 representation section owner required')
    path=records.canonical_path(str(destination))
    checksum=hashlib.sha256();total=0;rows=[]
    # Include the supplied section owner before advancing it. A lazy producer
    # must include its external retained roots in live_bytes/owner_charge.
    static=graph_charge((str(path),sections))+16384
    def caller_carry():
        carry=0 if owner_charge is None else owner_charge()
        core.integer(carry,core.SEMANTIC,'selected40 representation section carry')
        return carry
    if live_bytes+static+caller_carry()+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 representation entry allocation lifetime')
    iterator=iter(sections);fd=None
    try:
        count=0
        while True:
            if (live_bytes+static+caller_carry()+graph_charge(rows)
                    +2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('selected40 representation producer allocation lifetime')
            try:name,value=next(iterator)
            except StopIteration:break
            if count>=len(COMPLETE_FIT_SECTIONS) or name!=COMPLETE_FIT_SECTIONS[count]:
                raise ValueError('selected40 complete ordered representation sections')
            current=graph_charge((value,rows))
            encoder=json.JSONEncoder(sort_keys=True,separators=(',',':'),ensure_ascii=True,allow_nan=False)
            # No complete second encoding is retained beside the input graph.
            width,largest=_framing_json_width([name,value]);length=1+width
            if length>records.MANIFEST_CAP or total+length>core.SEMANTIC:
                raise ValueError(('selected40 complete framing limit',name,total+length,core.SEMANTIC))
            carry=caller_carry()
            if live_bytes+static+carry+current+2*largest+core.CHUNK+262144>core.SEMANTIC:
                raise ValueError('selected40 representation complete encoding lifetime')
            # First-section encoding, dynamic owner and complete caller guards
            # have passed before any destination can be created.
            if fd is None:
                fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
            start=total
            for piece in encoder.iterencode([name,value]):
                encoded=piece.encode('ascii')
                for offset in range(0,len(encoded),core.CHUNK):
                    block=encoded[offset:offset+core.CHUNK]
                    checksum.update(block);total+=len(block)
                    view=memoryview(block)
                    while view:
                        wrote=os.write(fd,view)
                        if wrote<=0:raise ValueError('selected40 complete representation write')
                        view=view[wrote:]
            checksum.update(b'\n');total+=1
            if os.write(fd,b'\n')!=1 or total-start!=length:
                raise ValueError('selected40 exact representation section width')
            rows.append(dict(section=name,offset=start,bytes=length));count+=1
            del value
        if count!=len(COMPLETE_FIT_SECTIONS):
            raise ValueError('selected40 representation omitted section')
        os.fsync(fd)
    finally:
        if fd is not None:os.close(fd)
    return dict(pin=dict(path=str(path),bytes=total,sha256=checksum.hexdigest()),
                sections=rows,complete_fit=False,accepted=False)


COMPLETE_CONTROL_FIELDS={
    'authority':{'schema','state','profile','authority','history','development_only'},
    'history':{'schema','state','profile','history','effective','event_history'},
    'current_sources':{'schema','state','profile','source_identity','runtime_source_identity','source_freeze','tool_freeze','sources','tools'},
    'historical_scope':{'schema','state','scope','current_source_freeze','current_tool_freeze','historical_acceptance','historical_source_freeze'},
    'controls':{'schema','state','profile','inputs','matrix_storage','future_program','future_arguments'},
    'receipts':{'schema','state','routes','receipt_maximum','result_maximum','completion_producer','completion_consumer','accepted'},
    'recovery':{'schema','state','routes','maximum_rounds','terminal_failure','source_change_after_success','reset','refund'},
    'binary_schedule':{'schema','state','routes','manifest','units','declarations','metadata_bytes','kind_reads','mode_counts','future_outputs'},
    'lifetimes':{'schema','state','semantic_maximum','chunk','failure_publication_carry','unit_pool_manifest','future_program','consumer_guards','accepted'},
}


def iter_complete_fit_control_sections(compiled, inputs, sources, tools, offline_scope,
                                  future, ids, wires, routes, matrix_storage, *, live_bytes,
                                  _retained=None,_owner_observer=None):
    """Derive closed prospective controls from the actual complete program.

    These records describe full pending obligations. They are not future
    completion receipts, gate results, READY or permission to launch. Every
    original body and every constructor consumer remains mandatory on each
    route. The caller owns all arguments throughout this bounded projection.
    """
    core.integer(live_bytes,core.SEMANTIC,'selected40 complete controls caller')
    if _retained is not None and not callable(_retained):
        raise ValueError('selected40 complete controls retained owner')
    if _owner_observer is not None and not callable(_owner_observer):
        raise ValueError('selected40 complete controls owner observer')
    def carry():
        value=0 if _retained is None else _retained()
        core.integer(value,core.SEMANTIC,'selected40 complete controls retained carry')
        return value
    if live_bytes+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 complete controls entry lifetime')
    binding=compiled.get('profile_binding')
    if (type(binding)is not dict or set(binding)!={'profile','authority','history','events','source_identity'}
            or binding['events']!=list(compiled['consumed_events'])
            or binding['source_identity']!=core.digest(sources+tools)):
        raise ValueError('selected40 actual authenticated profile/control source binding')
    if (len(sources)!=90 or len(tools)!=11 or len(future)!=81 or len(ids)!=len(wires)
            or len(routes)!=16 or [r['route'] for r in routes]!=list(framing_routes())):
        raise ValueError('selected40 complete controls actual program dimensions')
    if (type(matrix_storage)is not dict or set(matrix_storage)!=
            {'schema','activation','profile','contexts_digest','directory','ledger','round','phase'}
            or matrix_storage['schema']!='fixed40-matrix-storage-binding-v1'
            or matrix_storage['activation']!=MATRIX_STORAGE_ACTIVATION
            or matrix_storage['profile']!=binding['profile']
            or type(matrix_storage['round'])is not int or matrix_storage['round'] not in (0,1)
            or matrix_storage['phase']!=('normal' if matrix_storage['round']==0 else 'recovery')):
        raise ValueError('selected40 closed activated matrix storage control')
    if live_bytes+carry()+graph_charge(binding)+3*records.WRAPPER_CAP+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 complete control projection preallocation')
    state='NONHISTORICAL_PROSPECTIVE_FIT_ONLY'
    manifest=core.frozen_manifest_digest(ids,wires)
    program=core.digest(future)
    route_bindings=[dict(route=r['route'],manifest=r['manifest']) for r in routes]
    if sorted(row['role'] for row in future)!=list(range(81)):
        raise ValueError('selected40 complete controls all81 roles')
    for row in future:
        role=row['role']
        if row['maximum']!=compiled['maxima'][role]:
            raise ValueError('selected40 complete controls future producer order')
        _pin_tuple(row['arguments'])
    kind_reads={name:dict(occurrences=0,bytes_plus_EOF=0) for name in ('source','evidence','binary')}
    modes={};declarations=metadata=0
    for identity,raw in zip(ids,wires):
        if (live_bytes+carry()+graph_charge((binding,route_bindings,kind_reads,modes))
                +2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 complete read schedule predecode lifetime')
        rows=records.decode(raw,records.MANIFEST_CAP);metadata+=len(raw)
        for occurrence,pin,kind,mode in rows:
            if kind not in kind_reads:raise ValueError('selected40 closed declared read kind')
            declarations+=1;modes[mode]=modes.get(mode,0)+1
            kind_reads[kind]['occurrences']+=1
            if not mode.startswith('member:'):kind_reads[kind]['bytes_plus_EOF']+=pin[1]+1
        del rows
    # The same concrete producer is used by the writer and by each consumer;
    # no caller-selected expected digest or success boolean is an authority.
    producers=(
        ('authority',lambda:dict(profile=binding['profile'],authority=binding['authority'],history=binding['history'],development_only=True)),
        ('history',lambda:dict(profile=binding['profile'],history=binding['history'],effective=binding['events'],event_history=compiled['inherited_event_history'])),
        ('current_sources',lambda:dict(profile=binding['profile'],source_identity=binding['source_identity'],runtime_source_identity=core.digest((sources,tools)),source_freeze=inputs['source_freeze'],tool_freeze=inputs['tool_freeze'],sources=sources,tools=tools)),
        ('historical_scope',lambda:dict(scope=offline_scope,current_source_freeze=inputs['source_freeze'],current_tool_freeze=inputs['tool_freeze'],historical_acceptance=HISTORICAL_TEN19_ACCEPTANCE,historical_source_freeze=HISTORICAL_TEN19_SOURCES)),
        ('controls',lambda:dict(profile=binding['profile'],inputs=inputs,matrix_storage=matrix_storage,future_program=program,future_arguments=[dict(role=r['role'],constructor=r['constructor'],arguments=r['arguments']) for r in future])),
        ('receipts',lambda:dict(routes=route_bindings,receipt_maximum=core.RECEIPT,result_maximum=core.RECEIPT,completion_producer='pa1_evidence.Invocation.complete',completion_consumer='pa1_fixed40_runtime.RuntimeAdmission._consume_fit_prefix',accepted=False)),
        ('recovery',lambda:dict(routes=route_bindings,maximum_rounds=2,terminal_failure='STOP_REMAINING_ROUTES',source_change_after_success=False,reset=False,refund=False)),
        ('binary_schedule',lambda:dict(routes=route_bindings,manifest=manifest,units=len(ids),declarations=declarations,metadata_bytes=metadata,kind_reads=kind_reads,mode_counts=modes,future_outputs=[dict(role=r['role'],output=r['output'],maximum=r['maximum']) for r in future])),
        ('lifetimes',lambda:dict(semantic_maximum=core.SEMANTIC,chunk=core.CHUNK,failure_publication_carry=MATRIX_FAILURE_PUBLICATION_CARRY,unit_pool_manifest=manifest,future_program=program,consumer_guards=['framing_partition','_fit_check_program','_fit_execute_future_unit','run_selected40_fit','run_selected40_matrix'],accepted=False)))
    def checked_section(name,produce):
        owner_metadata=graph_charge((binding,route_bindings,kind_reads,modes,producers))+32768
        if live_bytes+carry()+owner_metadata+3*records.WRAPPER_CAP+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('selected40 complete control section pregrowth lifetime')
        row=produce()
        row.update(schema='fixed40-complete-'+name.replace('_','-')+'-v1',state=state)
        if set(row)!=COMPLETE_CONTROL_FIELDS[name]:raise ValueError('selected40 closed control producer fields')
        required=live_bytes+carry()+owner_metadata+graph_charge(row)+3*records.WRAPPER_CAP+core.CHUNK+262144
        if (required>core.SEMANTIC):
            raise ValueError(('selected40 complete controls retained result lifetime',name,
                dict(caller=live_bytes,carry=carry(),metadata=owner_metadata,row=graph_charge(row),
                     required=required,limit=core.SEMANTIC)))
        if _owner_observer is not None:_owner_observer(owner_metadata)
        return row
    for name,produce in producers:
        # The checked frame returns before yield. No generator local retains
        # the yielded section while the consumer advances another dataflow.
        yield name,checked_section(name,produce)
    if _owner_observer is not None:_owner_observer(0)


def complete_fit_control_sections(*args,live_bytes):
    """Retain the original dictionary API, charging every collected section."""
    value={}
    for name,row in iter_complete_fit_control_sections(*args,live_bytes=live_bytes,
            _retained=lambda:graph_charge(value)):
        value[name]=row
        del row
    return value


def verify_complete_fit_control_receipts(partition, expected, *, live_bytes,owner_charge=None):
    """Compare every full parsed control with its source-derived producer."""
    core.integer(live_bytes,core.SEMANTIC,'selected40 complete controls comparison caller')
    if live_bytes+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('selected40 complete controls comparison entry lifetime')
    if owner_charge is not None and not callable(owner_charge):
        raise ValueError('selected40 complete control comparison owner')
    collected=type(expected) is dict
    if collected and set(expected)!=set(COMPLETE_CONTROL_FIELDS):
        raise ValueError('selected40 all closed controls required')
    seen=set()
    for name,value in (expected.items() if collected else expected):
        if name in seen or name not in COMPLETE_CONTROL_FIELDS:
            raise ValueError('selected40 unique closed control comparison')
        seen.add(name)
        if set(value)!=COMPLETE_CONTROL_FIELDS[name]:raise ValueError('selected40 complete expected control fields')
        width,_=_framing_json_width(value)
        carry=0 if owner_charge is None else owner_charge()
        core.integer(carry,core.SEMANTIC,'selected40 complete comparison owner carry')
        if live_bytes+carry+graph_charge((partition,expected if collected else value,seen))+3*width+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('selected40 complete control comparison lifetime')
        raw=core.canonical(value)
        if partition['control_receipts'][name]!=dict(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest()):
            raise ValueError('selected40 full closed control semantic mismatch: '+name)
        del raw,value
    if seen!=set(COMPLETE_CONTROL_FIELDS):raise ValueError('selected40 all closed controls required')
    partition['control_semantics_checked']=True


def framing_partition(compiled, extra, *, representation=None, read_charge=None,
                      live_bytes=0,archive_manifest=None):
    """Authenticate and check the exact complete artifact, one section at a time.

    This validates the generated bytes, including all16 manifests, rather than
    extrapolating a repeated archive. Full future execution and same-byte review
    are still required; this function does not manufacture either acceptance.
    """
    if representation is None or not callable(read_charge):
        raise IncompleteFit('complete source-backed representation pin and prepaid full framing read required')
    _pin_tuple(representation)
    core.integer(live_bytes,core.SEMANTIC,'selected40 framing full caller')
    if live_bytes<compiled.get('bridge_caller_charge',0):
        raise ValueError('selected40 framing inherited caller and carry required')
    if not 0<representation['bytes']<=core.SEMANTIC:
        raise ValueError('selected40 complete framing byte cap')
    read_charge((dict(representation),))
    checksum=hashlib.sha256();total=0;position=0;pending=bytearray()
    if archive_manifest is None:
        extra_width=sum(_framing_json_width(rows)[0]+128 for rows in extra)
        if (live_bytes+graph_charge((compiled,extra))+extra_width
                +256*(len(compiled['units'])+len(compiled['setup_unit_wire'])+len(extra))
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 framing archive manifest pre-allocation')
        archive_ids,archive_wires,_=invocation_wire_manifest(compiled,extra,live_bytes=live_bytes)
    else:
        if (type(archive_manifest) is not tuple or len(archive_manifest)!=2
                or any(type(part) is not tuple for part in archive_manifest)):
            raise ValueError('selected40 transferred archive manifest')
        archive_ids,archive_wires=archive_manifest
    archive_digest=core.frozen_manifest_digest(archive_ids,archive_wires)
    if archive_manifest is not None and compiled.get('archive_manifest_binding')!=archive_digest:
        raise ValueError('selected40 transferred archive exact frozen binding')
    retained=dict(archive_manifest=archive_digest,control_receipts={},control_semantics_checked=False)
    def section(raw):
        nonlocal position
        if (position>=len(COMPLETE_FIT_SECTIONS) or len(raw)>records.MANIFEST_CAP
                or live_bytes+graph_charge((compiled,extra,retained,pending,archive_ids,archive_wires))
                    +2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 complete framing section predecode lifetime')
        pair=records.decode(raw,records.MANIFEST_CAP)
        if (type(pair) is not list or len(pair)!=2
                or pair[0]!=COMPLETE_FIT_SECTIONS[position]):
            raise ValueError('selected40 complete framing section order')
        name,value=pair;position+=1
        if name=='header':
            if (type(value) is not dict or set(value)!={'schema','archive_manifest','accepted','ready','new_facts'} or value.get('schema')!='fixed40-complete-fit-representation-v1'
                    or value.get('archive_manifest')!=archive_digest
                    or value.get('accepted') is not False or value.get('ready') is not False
                    or value.get('new_facts')!=0):
                raise ValueError('selected40 exact complete framing header')
            retained['header']=value
        elif name=='prior83':
            if core.canonical(value)!=compiled['prior83_wire']:
                raise ValueError('selected40 complete83 exact binding equality')
        elif name=='future81':
            if (type(value) is not list or len(value)!=81
                    or sorted(row['role'] for row in value)!=list(range(81))):
                raise ValueError('selected40 complete81 executable input bindings')
            for row in value:
                if (set(row)!={'role','constructor','arguments','units','output','maximum','dependencies'}
                        or row['maximum']!=compiled['maxima'][row['role']]
                        or type(row['constructor']) is not str
                        or type(row['units']) is not list or len(row['units'])!=3
                        or type(row['dependencies']) is not list
                        or any(type(n) is not int or not 0<=n<81 for n in row['dependencies'])):
                    raise ValueError('selected40 complete future executable step schema')
                _pin_tuple(row['arguments'])
            retained['future81_wire']=bytes(raw)
        elif name=='unit_pool':
            if (type(value) is not dict
                    or value.get('schema')!='fixed40-lossless-complete-unit-pool-v1'):
                raise ValueError('selected40 complete unit pool')
            retained['unit_pool_wire']=bytes(raw)
        elif name=='routes':
            if (type(value) is not list or len(value)!=16
                    or [row['route'] for row in value]!=list(framing_routes())
                    or any(set(row)!={'route','manifest','archive_manifest','future_roles'}
                        or row['archive_manifest']!=archive_digest
                        or row['future_roles']!=list(range(81)) for row in value)):
                raise ValueError('selected40 complete sixteen-route manifest bindings')
            retained['routes']=[{k:row[k] for k in ('route','manifest','archive_manifest')} for row in value]
        elif name=='future_maxima':
            if value!=list(compiled['maxima']):
                raise ValueError('selected40 unchanged full future maxima')
        elif name in COMPLETE_CONTROL_FIELDS:
            if (type(value)is not dict or set(value)!=COMPLETE_CONTROL_FIELDS[name]
                    or value['schema']!='fixed40-complete-'+name.replace('_','-')+'-v1'
                    or value['state']!='NONHISTORICAL_PROSPECTIVE_FIT_ONLY'):
                raise ValueError('selected40 closed complete framing control: '+name)
            width,_=_framing_json_width(value)
            if (live_bytes+graph_charge((compiled,extra,retained,pending,archive_ids,archive_wires,value))
                    +3*width+2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('selected40 complete control canonical lifetime')
            canonical=core.canonical(value)
            retained['control_receipts'][name]=dict(bytes=len(canonical),sha256=hashlib.sha256(canonical).hexdigest())
            del canonical
        elif value is None:
            raise ValueError('selected40 omitted required framing body')
    fd=os.open(records.canonical_path(representation['path']),os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    try:
        import stat
        if not stat.S_ISREG(os.fstat(fd).st_mode):raise ValueError('selected40 regular complete representation')
        while total<representation['bytes']:
            if (live_bytes+graph_charge((compiled,extra,retained,pending,archive_ids,archive_wires))
                    +records.MANIFEST_CAP+2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('selected40 complete framing stream pregrowth lifetime')
            block=os.read(fd,min(core.CHUNK,representation['bytes']-total))
            if not block:raise ValueError('selected40 truncated complete representation')
            checksum.update(block);total+=len(block)
            # Keep at most one bounded section plus a single stream chunk.
            pending.extend(block)
            while b'\n' in pending:
                end=pending.index(10)+1
                raw=bytes(pending[:end]);del pending[:end]
                section(raw);del raw
            if len(pending)>records.MANIFEST_CAP:
                raise ValueError('selected40 individual framing section cap')
        if (os.read(fd,1) or pending or position!=len(COMPLETE_FIT_SECTIONS)
                or checksum.hexdigest()!=representation['sha256']):
            raise ValueError('selected40 complete framing hash/EOF/section count')
    finally:
        os.close(fd)
    return dict(schema='fixed40-complete-framing-partition-v1',representation=dict(representation),
                actual_framing_bytes=total,framing_limit=core.SEMANTIC,
                exact_complete_bytes_authenticated=True,accepted=False,**retained)


S3 = BASE/'vrf-pa1-s3-01a1470a-20261017'
S3_MANIFEST = dict(path=str(S3/'setup-manifest-v1.json'), bytes=53116,
                   sha256='8182f960fe702d321caf5c8c6ba168db2feb8b16b7879b328cea51c88ad0ee04')
S3_PUBLICATION = dict(path=str(S3/'setup-publication-v1.json'), bytes=3432,
                      sha256='f3a82b786b09997f4af634424eabaae862cc32601e059977c149a42d08d37c0b')


def verify_setup_manifest(value):
    """Validate every finite row; pending bodies keep their full reservation."""
    columns = ['role', 'version', 'stage', 'producer', 'path', 'bytes', 'sha256',
               'status', 'maximum_bytes', 'prerequisites']
    if value.get('columns') != columns or len(value.get('entries', ())) != 175:
        raise ValueError('selected40 exact S3 catalogue dimensions')
    existing = []
    for index, row in enumerate(value['entries']):
        if type(row) is not list or len(row) != 10:
            raise ValueError('selected40 S3 row shape')
        role, version, stage, producer, path, size, checksum, status, maximum, prerequisites = row
        if not all(type(v) is str and v for v in (role, version, producer, path)):
            raise ValueError('selected40 S3 row identity')
        records.canonical_path(path)
        if Path(path).name == 'AI.md':
            raise ValueError('selected40 forbidden input')
        core.integer(maximum, 16777216, 'S3 finite maximum')
        if maximum == 0 or type(prerequisites) is not list or any(
                type(n) is not int or not 0 <= n < index for n in prerequisites):
            raise ValueError('selected40 S3 acyclic prerequisites')
        if status == 'EXISTING':
            pin = dict(path=path, bytes=size, sha256=checksum)
            _pin_tuple(pin)
            if size > maximum:
                raise ValueError('selected40 S3 existing maximum')
            existing.append(pin)
        elif status != 'PENDING' or size is not None or checksum is not None:
            raise ValueError('selected40 S3 pending full reservation')
    mapping = value['event_mapping']
    if (len(existing) != 61 or mapping['ceilings'] != [8,4,4]
            or mapping['consumed'] != [4,0,0] or mapping['consumed_history'] != BASELINE_HISTORY
            or mapping['new_events'] != [5,6,7,8]
            or mapping['original_v5_finite_failures'] != 2031616
            or mapping['incremental_distinct_growth'] != 0):
        raise ValueError('selected40 S3 event mapping')
    ledger = value['ledger']
    # The exact pinned manifest supplies the full accounting proof. These
    # effective values must also match the independently published authority.
    if ledger['selected_setup_envelope'] != 11423963 or ledger['nonbinary_lifecycle_ceiling'] != 35964936624:
        raise ValueError('selected40 effective S3 ledger')
    return tuple(existing)


def setup_units():
    """Compile serialized pre-archive units, releasing the175-row graph first.

    Preparation freezes identities only. Every body is read again under the
    admitted Invocation; a compiler read grants no semantic discharge.
    """
    manifest = records.decode(_read(S3_MANIFEST), records.WRAPPER_CAP)
    existing = verify_setup_manifest(manifest)
    publication = records.decode(_read(S3_PUBLICATION), records.WRAPPER_CAP)
    pins = [S3_MANIFEST, S3_PUBLICATION]
    for pin in publication['inputs'] + list(publication['predecessor'].values()):
        if pin not in pins:
            pins.append(pin)
    # Preserve distinct catalogue containment even when a root publication
    # names the same physical owner body. No semantic occurrence is deduped.
    pins.extend(existing)
    del existing, manifest, publication
    return tuple(((1500+i, _pin_tuple(pin), 'evidence',
                    'json' if i < 5 else 'bytes'),) for i, pin in enumerate(pins))


def invocation_wire_manifest(compiled, extra, *, live_bytes=None):
    """Transfer frozen unit wires without constructing a complete pin graph."""
    setup = compiled['setup_unit_wire']
    if live_bytes is None:live_bytes=compiled.get('bridge_caller_charge',0)
    core.integer(live_bytes,core.SEMANTIC,'selected40 invocation wire caller')
    count=len(setup)+len(compiled['unit_ids'])+len(extra)
    width=sum(_framing_json_width(rows)[0]+1 for rows in extra)
    if (live_bytes+graph_charge((compiled,extra))+32*count
            +256*(len(setup)+len(extra))+3*width+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 invocation wire preallocation lifetime')
    ids = (tuple('selected40-setup:'+str(i) for i in range(len(setup))) +
           compiled['unit_ids'] + tuple('selected40-semantic:'+str(i) for i in range(len(extra))))
    return ids, setup+compiled['units']+tuple(core.canonical(rows) for rows in extra), len(setup)


def invocation_manifest(compiled, extra):
    """Freeze one logical invocation with the setup authority first."""
    setup = tuple(tuple((r[0], tuple(r[1]), r[2], r[3]) for r in
                  records.decode(raw, records.MANIFEST_CAP)) for raw in compiled['setup_unit_wire'])
    archive = tuple(tuple((r[0], tuple(r[1]), r[2], r[3]) for r in
                    records.decode(raw, records.MANIFEST_CAP)) for raw in compiled['units'])
    ids = (tuple('selected40-setup:'+str(i) for i in range(len(setup))) +
           compiled['unit_ids'] + tuple('selected40-semantic:'+str(i) for i in range(len(extra))))
    return ids, setup+archive+extra, len(setup)


def setup_role49_binding(inputs, compiled):
    """Add S3 duty inside original findings; preserve original child references."""
    binding = dict(schema='selected40-S3-role49-fit-only-v1', manifest=S3_MANIFEST,
                   publication=S3_PUBLICATION, effective_setup=11423963,
                   effective_nonbinary_lifecycle=35964936624, event_maxima=(8,4,4),
                   source_freeze=inputs['source_freeze'], tool_freeze=inputs['tool_freeze'],
                   exact_saved_gate=inputs['saved_result'], exact_saved_log=inputs['saved_log'],
                   command=SAVED_COMMAND, all_phase_prerequisites=compiled['schedule'],
                   original_reference_count=7, authority=False)
    prefix = core.canonical(binding).decode()
    if len(prefix.encode())+len('FIT_ONLY_SELECTED40_NO_AUTHORITY') > 16384:
        raise ValueError('selected40 combined role49 S3 findings overflow')
    return prefix


def verify_setup_unit(unit, index, live):
    occurrence, pin, kind, mode = unit.declarations[0]
    if index >= 5:
        unit.consume(occurrence, _pin(pin), kind=kind)
        return
    value = unit.decode(occurrence, _pin(pin), live_bytes=live)
    if index == 0:
        verify_setup_manifest(value)
    elif index == 1:
        if (value['schema'] != 'pa1-s3-authenticated-publication-v1'
                or value['accounting_accepted'] is not True
                or value['effective_setup'] != 11423963
                or value['effective_nonbinary_lifecycle'] != 35964936624
                or value['binary_lifecycle'] != 449080998528
                or value['event_maxima'] != [8,4,4] or value['consumed'] != [4,0,0]
                or value['consumed_history'] != BASELINE_HISTORY
                or value['inputs'][1] != S3_MANIFEST):
            raise ValueError('selected40 S3 publication authority')
    elif index == 2:
        if (value['schema'] != 'pa1-s3-owner-approval-v1'
                or value['event_maxima'] != [8,4,4] or value['consumed_events'] != [4,0,0]
                or value['history_sha256'] != BASELINE_HISTORY
                or value['setup_maximum'] != 16777216
                or value['nonbinary_lifecycle_maximum'] != 38020585776):
            raise ValueError('selected40 S3 owner authority')
    elif index == 3:
        if value['status'] != 'PASS_S3_EVENT_SETUP_LIFECYCLE_CONFORMANCE':
            raise ValueError('selected40 S3 independent conformance')
    elif index == 4:
        if (value['verdict'] != 'ACCEPT_EXACT_S3_ACCOUNTING_PREREQUISITE'
                or value['accepted'] is not True
                or any(value[k] is not False for k in
                       ('source_accepted','complete_executable_fit','runtime_authority','ready'))):
            raise ValueError('selected40 S3 independent accounting')
    # Index5 is the fully pinned predecessor checkpoint. It is authenticated as
    # an opaque body below; no historical operational float becomes authority.


def _fit_constructor_name(role):
    """Return the one allowed offline constructor for an original role."""
    import pa1_fixed40_constructors as constructors
    if role in constructors.VALIDATION_RESULTS:
        return 'prospective_validation_result'
    if role in constructors.LOG_ROLES:
        return 'prospective_log'
    if role in (28, 37, 49):
        return 'acceptance_record'
    if role in (45, 50, 51, 52, 53):
        return 'historical_context_body'
    if role in (54, 69, 72, 75):
        return 'prospective_native_report_from_wires'
    if role in (64, 66, 70, 73):
        return 'prospective_runtime_command_result'
    if role in (76, 77, 78):
        return 'prospective_verification_record'
    return {29: 'prospective_validation_summary', 30: 'prospective_source_commit_record',
        32: 'prospective_normal_hook_summary', 33: 'source_successor_mapping',
        34: 'binary_successor_mapping', 35: 'instruction_preparation_comparison',
        36: 'storage_validation', 38: 'current_source_role', 39: 'current_source_role',
        40: 'successor_contract', 41: 'candidate_review', 42: 'selected40_candidate',
        43: 'candidate_pins', 44: 'compare_candidate_preparation',
        46: 'prospective_saved_equivalence', 48: 'current_correction_record',
        55: 'candidate_preparation', 56: 'instance_spec', 57: 'command_freeze',
        58: 'instance_spec', 59: 'prospective_exact_commands', 60: 'independent_review_document',
        61: 'prospective_command_verification', 62: 'prospective_capture_execution_decision',
        63: 'physical_attempt_marker', 68: 'prospective_native_bundle',
        79: 'independent_review_document', 80: 'quota_snapshot'}[role]

def _fit_check_program(rows, pool, archive_ids, archive_wires, maxima, *, live_bytes=0):
    """Check exact executable order, declarations and output identities.

    Each role owns three consecutive units: argument authentication, construction,
    and independent output reauthentication/reconstruction. Occurrences remain
    global core identities, including across the archive/future boundary.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 complete program caller')
    if (live_bytes+graph_charge((rows,pool,archive_ids,archive_wires))
            +2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 complete program entry lifetime')
    if (type(rows) is not list or len(rows) != 81
            or type(pool) is not tuple or len(pool) != 243
            or sorted(row['role'] for row in rows) != list(range(81))):
        raise ValueError('fixed40 exact81 program/243 future units')
    ids = []; completed = set(); outputs = set()
    def before_decode(raw,retained=()):
        if (live_bytes+graph_charge((rows,pool,archive_ids,archive_wires,ids,completed,outputs,retained))
                +2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete program unit predecode lifetime')
    for index, row in enumerate(rows):
        role = row['role']
        if (type(role) is not int or set(row) != {
                'role','constructor','arguments','units','output','maximum','dependencies'}
                or row['constructor'] != _fit_constructor_name(role)
                or row['maximum'] != maxima[role]
                or row['units'] != list(range(index*3, index*3+3))
                or type(row['dependencies']) is not list
                or len(set(row['dependencies'])) != len(row['dependencies'])
                or any(type(n) is not int or n not in completed for n in row['dependencies'])):
            raise ValueError('fixed40 closed constructor/topological binding')
        _pin_tuple(row['arguments']); _pin_tuple(row['output'])
        if not 0 < row['output']['bytes'] <= row['maximum']:
            raise ValueError('fixed40 complete output body/maximum')
        if row['output']['path'] in outputs:
            raise ValueError('fixed40 distinct future output paths')
        outputs.add(row['output']['path'])
        for offset, position in enumerate(row['units']):
            raw = pool[position]
            if type(raw) is not bytes or not raw.isascii():
                raise ValueError('fixed40 canonical future unit wire')
            if (live_bytes+graph_charge((rows,pool,archive_ids,archive_wires,ids,completed,outputs))
                    +len(raw)+2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('fixed40 complete program wire pre-allocation')
            before_decode(raw)
            declarations = records.decode(raw, records.MANIFEST_CAP)
            if not declarations or core.canonical(declarations) != raw:
                raise ValueError('fixed40 nonempty canonical future declarations')
            if offset == 0:
                if tuple(declarations[0][1]) != _pin_tuple(row['arguments']):
                    raise ValueError('fixed40 complete argument control first')
            elif tuple(declarations[0][1]) != _pin_tuple(row['arguments']):
                raise ValueError('fixed40 repeated constructor control first')
            if offset == 2:
                last = declarations[-1]
                if (tuple(last[1]) != _pin_tuple(row['output'])
                        or last[2:] != ['evidence', 'bytes']):
                    raise ValueError('fixed40 consumer complete output last')
                producer_raw=pool[position-1]
                if (live_bytes+graph_charge((rows,pool,archive_ids,archive_wires,ids,
                        completed,outputs,declarations,raw,producer_raw))
                        +2*len(producer_raw)+4096
                        +2*core.CHUNK+262144>core.SEMANTIC):
                    raise ValueError('fixed40 consumer prefix predecode lifetime')
                # The preceding immutable producer wire was fully decoded and
                # proved canonical in offset1. This consumer is also fully
                # decoded and canonical above. Compare their complete encoded
                # arrays directly, keeping only the small final output row.
                # Actual Invocation/Unit parsing and every full body read keep
                # their original operation and byte charges.
                prefix_width=len(producer_raw)-2
                tail=b','+core.canonical(last)[:-1]+b']\n'
                if (not producer_raw.endswith(b']\n') or prefix_width<1
                        or raw[:prefix_width]!=producer_raw[:-2]
                        or raw[prefix_width:]!=tail):
                    raise ValueError('fixed40 producer/consumer identical full input schedule')
                del producer_raw,last,prefix_width,tail
            if offset == 1:
                argument_raw=pool[position-1]
                before_decode(argument_raw,declarations)
                argument_rows=records.decode(argument_raw,records.MANIFEST_CAP)
                if declarations[:len(argument_rows)] != argument_rows:
                    raise ValueError('fixed40 complete repeated argument input prefix')
                del argument_rows,argument_raw
            ids.append('selected40-future:'+str(role)+':'+str(offset))
            del declarations,raw
        completed.add(role)
    if len(archive_ids)+len(ids) > core.INVOCATION_UNITS:
        raise ValueError('fixed40 full invocation unit cap')
    # Check all occurrence identities and operation reservations before counter IO.
    # Only a packed location/count table stays live. The admitted core repeats
    # exact tuple equality before its debit; a second complete pin graph here
    # would retain every historical path beside all future constructor wires.
    uses=bytearray(2048)
    # Check every archive/future occurrence before allocating the combined
    # return tuples. Both immutable input pieces remain owned throughout this
    # scan; no concatenated second program sits beside its decoded unit.
    for piece in (archive_wires,pool):
        for raw in piece:
            before_decode(raw,uses)
            decoded=records.decode(raw, records.MANIFEST_CAP)
            for occurrence, pin, kind, mode in decoded:
                core.integer(occurrence, 2047, 'fixed40 full occurrence')
                uses[occurrence]+=2 if mode=='json' else 1
                # Keep every full read and JSON parse in the original cap.
                if uses[occurrence] > 8:
                    raise ValueError('fixed40 full occurrence operation reservation')
            if decoded:del occurrence,pin,kind,mode
            del decoded
    del piece,raw,uses,before_decode
    completed.clear();outputs.clear()
    if (live_bytes+graph_charge((rows,pool,archive_ids,archive_wires,ids,completed,outputs))
            +32*(len(archive_ids)+len(ids))+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 complete program return tuple lifetime')
    all_ids = archive_ids+tuple(ids); all_wires = archive_wires+pool
    return all_ids, all_wires

def assemble_complete_fit_representation(destination,owned,*,publication_charge,live_bytes=0):
    """Consume the concrete assembler roots and publish all15 full sections.

    owned transfers the named graphs to this function. Other retained caller
    roots and callback state must remain in live_bytes. Full control bodies,
    native identities and all81 outputs must already be authenticated by the
    admitted producer. Publication is prepaid before exclusive file creation.
    This creates a prospective artifact; independent same-byte review and all16
    actual executions remain mandatory. It grants no acceptance or capture.
    """
    names={'compiled','inputs','sources','tools','historical_scope','future',
           'future_pool','archive_ids','archive_wires','matrix_storage'}
    if type(owned) is not dict or set(owned)!=names or not callable(publication_charge):
        raise ValueError('fixed40 complete assembler ownership and publication')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 complete assembler caller')
    if live_bytes+graph_charge(owned)+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 complete assembler entry lifetime')
    compiled=owned.pop('compiled');inputs=owned.pop('inputs')
    sources=owned.pop('sources');tools=owned.pop('tools')
    scope=owned.pop('historical_scope');future=owned.pop('future')
    future_pool=owned.pop('future_pool');archive_ids=owned.pop('archive_ids')
    archive_wires=owned.pop('archive_wires');matrix_storage=owned.pop('matrix_storage')
    archive_digest=core.frozen_manifest_digest(archive_ids,archive_wires)
    retained=live_bytes+graph_charge((compiled,inputs,sources,tools,scope,matrix_storage))
    ids,wires=_fit_check_program(future,future_pool,archive_ids,archive_wires,
                                compiled['maxima'],live_bytes=retained)
    del archive_ids,archive_wires,future_pool
    manifest=core.frozen_manifest_digest(ids,wires)
    routes=[dict(route=route,manifest=manifest,archive_manifest=archive_digest,
                 future_roles=list(range(81))) for route in framing_routes()]
    retained=live_bytes+graph_charge((compiled,inputs,sources,tools,scope,future,
                                     matrix_storage,routes))
    pool=pack_complete_fit_units(ids,wires,live_bytes=retained)
    retained+=graph_charge((ids,wires,pool))+32768
    projection=[0]
    def observe(value):projection[0]=value
    def prior_body():
        raw=compiled['prior83_wire']
        if (retained+2*len(raw)+160*core._wire_nodes(raw)
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete assembler prior83 lifetime')
        prior=records.decode(raw,records.MANIFEST_CAP)
        if type(prior) is not list or len(prior)!=83:
            raise ValueError('fixed40 complete assembler all83 bindings')
        return prior
    def sections():
        controls=iter_complete_fit_control_sections(compiled,inputs,sources,tools,scope,
            future,ids,wires,routes,matrix_storage,live_bytes=retained,_owner_observer=observe)
        for name in COMPLETE_FIT_SECTIONS:
            if name in COMPLETE_CONTROL_FIELDS:yield next(controls)
            elif name=='header':
                yield name,dict(schema='fixed40-complete-fit-representation-v1',
                    archive_manifest=archive_digest,accepted=False,ready=False,new_facts=0)
            elif name=='prior83':yield name,prior_body()
            elif name=='future81':yield name,future
            elif name=='unit_pool':yield name,pool
            elif name=='routes':yield name,routes
            elif name=='future_maxima':yield name,list(compiled['maxima'])
            else:raise ValueError('fixed40 complete assembler closed section')
        try:next(controls)
        except StopIteration:pass
        else:raise ValueError('fixed40 complete assembler excess control')
    maximum=0
    for name,value in sections():
        width,largest=_framing_json_width([name,value])
        if (retained+projection[0]+graph_charge(value)+2*largest
                +core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete assembler section encoding lifetime')
        if width+1>records.MANIFEST_CAP:
            raise ValueError('fixed40 complete assembler individual section cap')
        maximum+=width+1
        del value
    if maximum>core.SEMANTIC:
        raise ValueError('fixed40 complete assembler total framing cap')
    publication_charge((dict(path=str(records.canonical_path(str(destination))),
                             bytes=maximum,kind='complete15section'),))
    result=write_complete_fit_representation(destination,
        sections(),live_bytes=retained,owner_charge=lambda:projection[0])
    if result['pin']['bytes']!=maximum:
        raise ValueError('fixed40 complete assembler exact encoded width')
    return result


def complete_fit_role_order(cases, *, live_bytes=0):
    """Order all81 concrete dataflows without changing their role identities.

    Dependencies name roles; unit positions are assigned after sorting. No
    input body is read. Missing roles, duplicate edges and cycles fail closed.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 complete order caller')
    if (type(cases) not in (list,tuple) or len(cases)!=81
            or live_bytes+graph_charge(cases)+65536+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 complete81 dataflow descriptors')
    by_role={}
    for index,case in enumerate(cases):
        if type(case) is not dict or type(case.get('role')) is not int:
            raise ValueError('fixed40 exact dataflow role')
        role=case['role'];dependencies=case.get('dependencies')
        if not 0<=role<81 or role in by_role:
            raise ValueError('fixed40 unique complete dataflow role')
        if (type(dependencies) is not list
                or any(type(x) is not int or not 0<=x<81 or x==role for x in dependencies)
                or len(set(dependencies))!=len(dependencies)):
            raise ValueError('fixed40 exact dataflow dependencies')
        by_role[role]=index
    ordered=[];done=set()
    while len(ordered)<81:
        ready=[role for role in range(81) if role not in done
               and all(parent in done for parent in cases[by_role[role]]['dependencies'])]
        if not ready:raise ValueError('fixed40 complete dataflow dependency cycle')
        role=ready[0];ordered.append(role);done.add(role)
    return tuple(ordered)


def _fit_require_resolved_arguments(value):
    """Reject development promises while retaining ordinary semantic kinds."""
    helpers={'generated-constructor-output','unresolved-current-executable-identity',
        'prospective-normal-commit-control','prospective-review-body',
        'complete-model-fact-seal-input-body','prospective-complete-fit-review',
        'fresh-execution-utc','fresh-same-budget-quota-observation','same-saved-budget',
        'derived-complete-program-read-ceilings','derived-complete-program-phase-lifetimes',
        'current214-full-history-read-accounting','compiled-reference-placement-full-children'}
    def check(node):
        if type(node) is dict:
            if node.get('kind') in helpers:
                raise ValueError('fixed40 unresolved complete argument helper: '+node['kind'])
            for child in node.values():check(child)
        elif type(node) in (tuple,list):
            for child in node:check(child)
    check(value)


def build_complete_fit_program(cases,archive_ids,archive_wires,maxima,source_pins,
                               *,read_control,live_bytes=0):
    """Build the existing243 future units from concrete, fully read controls.

    Closed cases contain role, arguments, output, constructor_rows and
    dependencies. Constructor rows are ordered (pin,kind,mode) triples, or an
    authenticated pin to those complete rows. Original four-field archive
    declarations preserve their exact occurrence/alias binding. Repeats remain distinct
    occurrences. One control and its constructor rows are decoded at a time.
    read_control must debit before open; bytes/hash are checked again here.
    No constructor, native resolution, publication or runtime admission occurs.
    """
    if not callable(read_control):raise ValueError('fixed40 full control reader required')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 complete program caller')
    roots=(cases,archive_ids,archive_wires,maxima,source_pins)
    occupied=bytearray(2048);source_pairs={}
    def guard(extra=(),scratch=32768):
        if (live_bytes+graph_charge((roots,occupied,source_pairs,extra))+scratch
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete program retained lifetime')
    guard()
    order=complete_fit_role_order(cases,live_bytes=live_bytes+graph_charge(
        (archive_ids,archive_wires,maxima,source_pins)))
    fields={'role','arguments','output','constructor_rows','dependencies'}
    by_role={case['role']:index for index,case in enumerate(cases)}
    if any(set(case)!=fields for case in cases):
        raise ValueError('fixed40 closed complete dataflow descriptor')
    if type(maxima) not in (list,tuple) or len(maxima)!=81:
        raise ValueError('fixed40 complete81 maxima')
    known_sources=set()
    for pin in source_pins:known_sources.add(_pin_tuple(pin))
    occurrence=0
    for raw in archive_wires:
        guard((order,known_sources),2*len(raw)+160*core._wire_nodes(raw))
        rows=records.decode(raw,records.MANIFEST_CAP)
        for row in rows:
            core.integer(row[0],2047,'fixed40 original global occurrence')
            occupied[row[0]]=1
        del rows
    future=[];pool=[]
    for index,role in enumerate(order):
        case=cases[by_role[role]]
        _pin_tuple(case['arguments']);_pin_tuple(case['output'])
        if not 0<case['arguments']['bytes']<=records.WRAPPER_CAP:
            raise ValueError('fixed40 complete argument body cap')
        guard((order,known_sources,future,pool),2*case['arguments']['bytes'])
        raw=read_control(case['arguments'])
        if (type(raw) is not bytes or len(raw)!=case['arguments']['bytes']
                or hashlib.sha256(raw).hexdigest()!=case['arguments']['sha256']):
            raise ValueError('fixed40 complete argument body authentication')
        guard((order,known_sources,future,pool,raw),2*len(raw)+160*core._wire_nodes(raw))
        control=records.decode(raw,records.WRAPPER_CAP);del raw
        required={'schema','role','parameters','byte_inputs','json_inputs',
                  'wire_inputs','declaration_bindings'}
        if (type(control) is not dict or set(control)!=required
                or control['schema']!='fixed40-constructor-arguments-v1'
                or type(control['role']) is not int or control['role']!=role
                or any(type(control[name]) is not dict for name in required-{'schema','role'})):
            raise ValueError('fixed40 complete constructor argument schema')
        if {'role','maxima','live_bytes','unit','compiled','identities',
                'projection_read_charge'}.intersection(control['parameters']):
            raise ValueError('fixed40 complete argument reserved parameter')
        _fit_require_resolved_arguments(control)
        prefix=[];row_roots=();shared_in_role={}
        def append(target,pin,kind,mode,*,constructor=False):
            nonlocal occurrence
            physical=_pin_tuple(pin)
            if kind not in ('source','evidence','binary') or mode not in ('bytes','json'):
                raise ValueError('fixed40 complete future read declaration')
            guard((order,known_sources,future,pool,control,row_roots,prefix,target,shared_in_role),
                  graph_charge(pin)+4096)
            # Original IDs never change. Each new constructor source closure
            # has one full read in its producer and one in its consumer. The
            # two named pairs can share the exact same source declaration:
            # four reads, with every byte/EOF debit and the eight-operation
            # bound preserved. Prefix reads keep their separate identities.
            pair=(29 if role in (29,30) else 60 if role in (60,79) else None)
            key=(pair,physical)
            shared=(constructor and pair is not None and kind=='source'
                    and mode=='bytes' and physical in known_sources)
            if shared:
                if key in shared_in_role:
                    raise ValueError('fixed40 duplicate source in paired closure')
                shared_in_role[key]=None
                previous=source_pairs.pop(key,None)
                if previous is not None:
                    original,other_role=previous
                    if other_role==role:
                        raise ValueError('fixed40 repeated paired source role')
                    target.append([original,list(physical),kind,mode])
                    return
            while occurrence<2048 and occupied[occurrence]:occurrence+=1
            core.integer(occurrence,2047,'fixed40 complete future occurrence')
            selected=occurrence;occupied[selected]=1;occurrence+=1
            target.append([selected,list(physical),kind,mode])
            if shared:source_pairs[key]=(selected,role)
        def input_row(pin,mode):
            append(prefix,pin,'source' if _pin_tuple(pin) in known_sources else 'evidence',mode)
        input_row(case['arguments'],'json');names=set()
        for category in ('byte_inputs','json_inputs','wire_inputs'):
            for name,pin in control[category].items():
                if (name in names or name in control['parameters'] or name in
                        {'role','maxima','live_bytes','unit','compiled','identities','projection_read_charge'}):
                    raise ValueError('fixed40 duplicate complete argument')
                names.add(name)
                if category=='wire_inputs':
                    if type(pin) is not dict:raise ValueError('fixed40 complete wire argument')
                    for source in pin.values():input_row(source,'bytes')
                elif category=='json_inputs' and set(pin)=={'pin','field','source_role'}:
                    if (role!=41 or name!='candidate_inputs' or pin['field']!='parameters'
                            or pin['source_role']!=42):
                        raise ValueError('fixed40 exact candidate42 argument projection')
                    input_row(pin['pin'],'json')
                else:input_row(pin,'json' if category=='json_inputs' else 'bytes')
        constructor_rows=case['constructor_rows']
        if type(constructor_rows) is dict:
            _pin_tuple(constructor_rows)
            if not 0<constructor_rows['bytes']<=records.MANIFEST_CAP:
                raise ValueError('fixed40 complete constructor rows body cap')
            guard((order,known_sources,future,pool,control),2*constructor_rows['bytes'])
            raw=read_control(constructor_rows)
            if (type(raw) is not bytes or len(raw)!=constructor_rows['bytes']
                    or hashlib.sha256(raw).hexdigest()!=constructor_rows['sha256']):
                raise ValueError('fixed40 complete constructor rows authentication')
            guard((order,known_sources,future,pool,control,raw),
                  2*len(raw)+160*core._wire_nodes(raw))
            constructor_rows=records.decode(raw,records.MANIFEST_CAP);del raw
            row_roots=constructor_rows
        if type(constructor_rows) not in (list,tuple):
            raise ValueError('fixed40 complete ordered constructor rows')
        for binding in control['declaration_bindings'].values():
            indexes=[binding] if type(binding) is int else binding
            if (type(indexes) is not list or not indexes
                    or any(type(i) is not int or not 0<=i<len(constructor_rows) for i in indexes)):
                raise ValueError('fixed40 complete constructor declaration binding')
        producer=list(prefix)
        for declaration in constructor_rows:
            if len(declaration)==3:
                pin,kind,mode=declaration;append(producer,pin,kind,mode,constructor=True)
            elif len(declaration)==4:
                original,physical,kind,mode=declaration
                core.integer(original,2047,'fixed40 original child occurrence')
                if (type(physical) not in (tuple,list) or len(physical)!=3
                        or kind not in ('source','evidence') or mode!='bytes'):
                    raise ValueError('fixed40 original full child declaration')
                pin=dict(zip(('path','bytes','sha256'),physical));_pin_tuple(pin)
                expected=[original,list(_pin_tuple(pin)),kind,mode];found=False
                for archive_raw in archive_wires:
                    guard((order,known_sources,future,pool,control,row_roots,prefix,producer),
                          2*len(archive_raw)+160*core._wire_nodes(archive_raw))
                    archive_rows=records.decode(archive_raw,records.MANIFEST_CAP)
                    for old in archive_rows:
                        if old[0]==original:
                            if old!=expected:
                                raise ValueError('fixed40 original child declaration equality')
                            found=True
                    del archive_rows
                if not found:raise ValueError('fixed40 original child occurrence absent')
                guard((order,known_sources,future,pool,control,row_roots,prefix,producer),
                      graph_charge(expected)+2048)
                producer.append(expected)
            else:raise ValueError('fixed40 complete constructor declaration width')
        # All complete constructor declarations are now in producer. Release
        # their decoded source pin dictionaries before serializing. Publish
        # the producer wire first, then extend the SAME owned list with the
        # output declaration. No second decoded101-source closure stays live.
        constructor_rows=None;row_roots=();declaration=None;shared_in_role.clear()
        for rows in (prefix,producer):
            width,_=_framing_json_width(rows)
            guard((order,known_sources,future,pool,control,prefix,producer),3*(width+1))
            pool.append(core.canonical(rows))
        prefix=()
        append(producer,case['output'],'evidence','bytes')
        width,_=_framing_json_width(producer)
        guard((order,known_sources,future,pool,control,producer),3*(width+1))
        pool.append(core.canonical(producer))
        future.append(dict(role=role,constructor=_fit_constructor_name(role),
            arguments=case['arguments'],units=list(range(index*3,index*3+3)),
            output=case['output'],maximum=maxima[role],dependencies=case['dependencies']))
        del control,prefix,producer,input_row,append,names,constructor_rows,rows,row_roots,shared_in_role
    del by_role,known_sources,case,order
    guard((future,pool),128*len(pool));pool=tuple(pool)
    _fit_check_program(future,pool,archive_ids,archive_wires,maxima,
        live_bytes=live_bytes+graph_charge((cases,source_pins)))
    return future,pool


def _fit_candidate_input_projection(value, *, live_bytes):
    """Reuse role42's complete decoded parameters for role41, without nesting.

    The entire immutable control is authenticated and parsed before this call.
    The closed source control has no secondary input maps to leave unresolved.
    Returning its parameters keeps the actual graph; no body read is waived.
    """
    if live_bytes+4096+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 candidate input projection caller lifetime')
    if (type(value) is not dict or set(value)!={
            'schema','role','parameters','byte_inputs','json_inputs','wire_inputs','declaration_bindings'}
            or value['schema']!='fixed40-constructor-arguments-v1' or value['role']!=42
            or any(value[name]!={} for name in
                ('byte_inputs','json_inputs','wire_inputs','declaration_bindings'))
            or type(value['parameters']) is not dict
            or set(value['parameters'])!={'slot','mapped_rows','prior_report','map_pin',
                'map_acceptance','prior_bundle','prior_report_pin','accepted_seal','accepted_review'}):
        raise ValueError('fixed40 complete role42 parameter projection')
    return value['parameters']


def _fit_load_arguments(unit, step, live_bytes, *, exact=True):
    """Load the complete closed argument control and its explicit body inputs."""
    rows = unit.declarations
    first = rows[0]
    if tuple(first[1]) != _pin_tuple(step['arguments']):
        raise ValueError('fixed40 exact argument control occurrence')
    control = unit.decode(first[0], step['arguments'], live_bytes=live_bytes)
    if (type(control) is not dict or set(control) != {
            'schema','role','parameters','byte_inputs','json_inputs','wire_inputs','declaration_bindings'}
            or control['schema'] != 'fixed40-constructor-arguments-v1'
            or control['role'] != step['role']
            or any(type(control[k]) is not dict for k in (
                'parameters','byte_inputs','json_inputs','wire_inputs','declaration_bindings'))):
        raise ValueError('fixed40 complete argument schema')
    kwargs = dict(control['parameters']); position = 1
    forbidden = {'role','maxima','live_bytes','unit','compiled','identities','projection_read_charge'}
    if forbidden & set(kwargs):
        raise ValueError('fixed40 caller cannot replace runtime constructor bindings')
    def body(pin, decoded):
        nonlocal position
        if position >= len(rows): raise ValueError('fixed40 missing argument body declaration')
        declaration = rows[position]; position += 1
        if tuple(declaration[1]) != _pin_tuple(pin):
            raise ValueError('fixed40 ordered argument body identity')
        current = live_bytes+graph_charge((control, kwargs))
        if decoded:
            return unit.decode(declaration[0], pin, live_bytes=current)
        return unit.consume(declaration[0], pin, kind=declaration[2], retain=True, live_bytes=current)
    for kind in ('byte_inputs','json_inputs','wire_inputs'):
        for name, pin in control[kind].items():
            if name in kwargs or name in forbidden:
                raise ValueError('fixed40 distinct argument bindings')
            if kind == 'wire_inputs':
                if type(pin) is not dict: raise ValueError('fixed40 native wire input mapping')
                kwargs[name] = {}
                for field, value in pin.items():
                    kwargs[name][field] = body(value, False)
            elif kind=='json_inputs' and type(pin) is dict and set(pin)=={'pin','field','source_role'}:
                if (step['role']!=41 or name!='candidate_inputs'
                        or pin['field']!='parameters' or pin['source_role']!=42):
                    raise ValueError('fixed40 closed candidate input projection binding')
                source_record=body(pin['pin'],True)
                kwargs[name]=_fit_candidate_input_projection(source_record,
                    live_bytes=live_bytes+graph_charge((control,kwargs,source_record)))
                del source_record
            else:
                kwargs[name] = body(pin, kind == 'json_inputs')
    if exact and position != len(rows): raise ValueError('fixed40 complete argument input closure')
    bindings = control['declaration_bindings']
    return kwargs, bindings, position

def _fit_construct(step, kwargs, bindings, unit, maxima, compiled, identities, live_bytes, *, consumer, input_start, read_charge):
    """Dispatch only reviewed constructor names, with actual admitted unit handles."""
    import pa1_fixed40_constructors as constructors
    role = step['role']; name = _fit_constructor_name(role)
    if name != step['constructor']: raise ValueError('fixed40 constructor substitution')
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    selection_growth=256+128*(len(kwargs)+len(bindings))+sum(
        128+128*len(value) for value in bindings.values() if type(value) is list)
    if (owner+live_bytes+graph_charge((kwargs,bindings))+selection_growth
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 constructor full caller/binding pre-allocation')
    arguments = dict(kwargs)
    declarations = unit.declarations[input_start:-1] if consumer else unit.declarations[input_start:]
    for key, selection in bindings.items():
        if key in arguments or key in {'unit','maxima','live_bytes','role','compiled','identities','projection_read_charge'}:
            raise ValueError('fixed40 constructor declaration substitution')
        if type(selection) is int:
            if not 0 <= selection < len(declarations):
                raise ValueError('fixed40 declaration index outside constructor input closure')
            arguments[key] = declarations[selection]
        elif type(selection) is list and all(type(n) is int and 0 <= n < len(declarations) for n in selection):
            arguments[key] = tuple(declarations[n] for n in selection)
        else: raise ValueError('fixed40 bounded declaration binding')
    unit_names = {'prospective_validation_summary','acceptance_record','historical_context_body','prospective_validation_result',
        'prospective_source_commit_record','prospective_normal_hook_summary',
        'prospective_saved_equivalence','prospective_native_report_from_wires',
        'prospective_native_bundle','prospective_verification_record',
        'independent_review_document','successor_contract',
        'prospective_command_verification','prospective_capture_execution_decision'}
    if name in unit_names: arguments['unit'] = unit
    if name == 'acceptance_record':
        arguments.update(compiled=compiled, identities=identities)
    if name == 'successor_contract':
        if not callable(read_charge):raise ValueError('fixed40 full mapping projection debit callback')
        arguments['projection_read_charge']=read_charge
    role_names = {'acceptance_record','historical_context_body','prospective_validation_result',
        'prospective_log','current_source_role','instance_spec','independent_review_document',
        'prospective_runtime_command_result','prospective_verification_record',
        'prospective_native_report_from_wires'}
    # The argument container stays live beside constructor-owned roots. Count
    # its shell here; each constructor charges its complete argument values.
    caller = live_bytes+graph_charge(bindings)+512+128*(len(arguments)+len(kwargs))
    if name not in unit_names:caller+=owner
    function = getattr(constructors, name)
    if name == 'historical_context_body':
        raw = arguments.pop('raw')
        output = function(role, raw, maxima=maxima, live_bytes=caller, **arguments)
    else:
        output = function(*((role,) if name in role_names else ()),
            maxima=maxima, live_bytes=caller, **arguments)
    if (type(output) is not bytes or len(output) != step['output']['bytes']
            or len(output) > step['maximum']
            or hashlib.sha256(output).hexdigest() != step['output']['sha256']):
        raise ValueError('fixed40 exact complete constructed output')
    if name == 'binary_successor_mapping':
        # The constructor renders metadata only. Each real producer AND
        # consumer unit must authenticate both complete current binary bodies.
        # Historical archive binaries remain their original independent reads.
        binaries=arguments['binaries']
        if (len(declarations)!=2 or any(kind!='binary' or mode!='bytes'
                or tuple(physical)!=_pin_tuple(pin)
                for pin,(_,physical,kind,mode) in zip(binaries,declarations))):
            raise ValueError('fixed40 current binary mapping full body declarations')
        binary_caller=caller+graph_charge((kwargs,arguments,bindings,output,declarations))
        if binary_caller+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 current binary mapping complete caller lifetime')
        for pin,(occurrence,physical,kind,mode) in zip(binaries,declarations):
            unit.consume(occurrence,pin,kind='binary',live_bytes=binary_caller)
    return output

def _fit_execute_future_unit(index, step, state, unit, *, maxima, compiled, identities, live_bytes,read_charge=None):
    """Execute one of the three mandatory units; retain only one role at a time."""
    offset = index % 3
    if offset == 0:
        if state: raise ValueError('fixed40 unreleased prior constructor arguments')
        kwargs, bindings, position = _fit_load_arguments(unit, step, live_bytes)
        # No input graph crosses the unit boundary. Both following units read
        # and parse the entire ordered argument prefix again before rebuilding.
        del kwargs,bindings,position
        state.update(role=step['role'])
        return
    if state.get('role') != step['role']:
        raise ValueError('fixed40 future argument lifetime/order')
    kwargs,bindings,position=_fit_load_arguments(unit,step,live_bytes+graph_charge(state),exact=False)
    raw = _fit_construct(step, kwargs, bindings, unit, maxima,
        compiled, identities, live_bytes, consumer=offset == 2,input_start=position,read_charge=read_charge)
    if offset == 2:
        occurrence, physical, kind, mode = unit.declarations[-1]
        actual = unit.consume(occurrence, _pin(physical), kind=kind,
            retain=True, live_bytes=live_bytes+graph_charge((state,kwargs,bindings,raw)))
        if actual != raw: raise ValueError('fixed40 complete future output reconstruction')
        del actual, raw
        state.clear()



# Three bounded encoder/write chunks, fixed metadata and the unchanged safety
# margin are prepaid by every admitted fit unit. No output/body cap changes.
MATRIX_FAILURE_PUBLICATION_CARRY=3*core.CHUNK+32768+262144


def _matrix_failure_publication(error,route,path,*,retained_fit_peak):
    """Write the complete canonical terminal diagnostic with the error live.

    retained_fit_peak includes the entire admitted fit and its outer caller,
    including decoder traceback frames, JSONDecodeError.doc and exception causes.
    It is the prepaid ceiling, not a Boolean or a second allocation bank.
    The original error is rethrown unchanged if publication cannot complete.
    """
    fd=None;directory=None;complete=False;count=0;digest=hashlib.sha256()
    publication_error=None;expected=None
    try:
        if (type(retained_fit_peak) is not int or retained_fit_peak<0
                or retained_fit_peak+MATRIX_FAILURE_PUBLICATION_CARRY>core.SEMANTIC):
            raise ValueError('fixed40 prepaid terminal failure peak')
        if route not in framing_routes():raise ValueError('fixed40 terminal failure route')
        path=str(records.canonical_path(str(path)))
        name=type(error).__name__
        if len(name)>128:raise ValueError('fixed40 full exception type bound')
        # These built-in single-string exceptions return their existing argument
        # from __str__; no new message copy is made while the decoder is live.
        if (type(error).__str__ is BaseException.__str__ and len(error.args)==1
                and type(error.args[0]) is str):
            message=error.args[0]
        elif type(error).__str__ is OSError.__str__:
            # Bound native OSError formatting BEFORE calling it. All path and
            # message fields remain complete; no slicing or omitted arguments.
            for item in (error.filename,error.filename2,error.strerror):
                if item is not None and (type(item) not in (str,bytes) or len(item)>4096):
                    raise ValueError('fixed40 full OS exception field bound')
            message=str(error)
        else:
            raise ValueError('fixed40 unsupported full exception formatter')
        def width(text):
            result=2
            for char in text:
                code=ord(char)
                result+=(2 if code in (34,92,8,12,10,13,9) else
                         6 if code<32 or 127<=code<=65535 else
                         12 if code>65535 else 1)
            return result
        # Match core.canonical exactly while measuring before any large encode.
        expected=(len(b'{"accepted":false,"exception":')+width(name)
            +len(b',"message":')+width(message)+len(b',"route":')+width(route)
            +len(b',"schema":')+width('fixed40-offline-route-failure-v1')+2)
        if expected>core.RECEIPT:raise ValueError('fixed40 full terminal diagnostic record cap')
        directory=os.open(str(Path(path).parent),os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        def put(block):
            nonlocal count
            if len(block)>core.CHUNK:raise ValueError('fixed40 terminal write chunk')
            view=memoryview(block)
            while view:
                wrote=os.write(fd,view)
                if wrote<=0:raise ValueError('fixed40 terminal diagnostic write')
                digest.update(view[:wrote]);count+=wrote;view=view[wrote:]
        def string(text):
            put(b'"')
            # <=12 ASCII bytes per input character, with separate substring,
            # escaped text and byte copies all inside three prepaid chunks.
            for offset in range(0,len(text),2048):
                escaped=json.encoder.encode_basestring_ascii(text[offset:offset+2048])
                put(escaped[1:-1].encode('ascii'))
            put(b'"')
        put(b'{"accepted":false,"exception":');string(name)
        put(b',"message":');string(message)
        put(b',"route":');string(route)
        put(b',"schema":');string('fixed40-offline-route-failure-v1');put(bytes((125,10)))
        if count!=expected:raise ValueError('fixed40 complete terminal canonical width')
        os.fsync(fd);os.fsync(directory);complete=True
    except BaseException as failure:
        publication_error=type(failure).__name__
    finally:
        for handle in (fd,directory):
            if handle is not None:
                try:os.close(handle)
                except BaseException as failure:
                    complete=False
                    if publication_error is None:publication_error=type(failure).__name__
        try:
            error.__dict__['fixed40_matrix_failure_publication']=dict(
                complete=complete,error_type=publication_error,written=count,expected=expected,
                retained_fit_peak=retained_fit_peak)
        except BaseException:pass
    if not complete:raise error
    return dict(path=path,bytes=count,sha256=digest.hexdigest())


def run_selected40_matrix(fit_admission, inputs, contexts, *, offline_scope,
                          representation, read_charge, output_directory,round_index=0,
                          trace_publication_charge=None, output_publication_charge=None):
    """Run the unchanged sixteen offline routes with complete live ownership.

    The output callback must bind the existing proper-write ledger. No storage,
    native, production or extra-attempt authority is created by this function.
    """
    import pa1_fixed40_runtime as runtime
    if (type(fit_admission) is not runtime.OfflineFitAdmission
            or fit_admission.accepted or not fit_admission.fit_accepted
            or not callable(read_charge)):
        raise ValueError('fixed40 reviewed nonphysical matrix admission required')
    if type(contexts) is not tuple or len(contexts)!=16:
        raise ValueError('fixed40 complete frozen matrix contexts')
    core.integer(round_index,fit_admission.counter_type.DEVELOPMENT_ROUNDS-1,'fixed40 matrix round')
    routes=framing_routes();roster=tuple(core.digest(context) for context in contexts)
    if len(set(roster))!=16:raise ValueError('fixed40 unique complete matrix contexts')
    for context,route in zip(contexts,routes):
        name,ordinal=route.rsplit(':',1)
        if context['route']!=name or context['phase']!='development:'+str(round_index)+':0:'+ordinal:
            raise ValueError('fixed40 matrix context order/namespace')
    directory=Path(records.canonical_path(output_directory))
    if not directory.is_dir() or directory.is_symlink():
        raise ValueError('fixed40 reserved matrix output directory')
    rows=[];success_rows=[];events=();source_identity=None
    counters=cold=compiled=raw=completion=closeout=diagnostic=create_route_counter=None
    def matrix_live(value=None):
        return fit_admission.caller_charge+graph_charge((inputs,contexts,offline_scope,
            representation,read_charge,trace_publication_charge,output_publication_charge,
            output_directory,roster,rows,success_rows,events,source_identity,
            compiled,raw,completion,closeout,diagnostic,create_route_counter,value,
            None if counters is None else counters.__dict__,
            None if cold is None else cold.__dict__))
    if type(output_publication_charge)is not _MatrixOutputOwner:
        raise ValueError('fixed40 complete matrix output storage reservation required')
    output_publication_charge.reserve(
        matrix_output_reservation(output_directory,live_bytes=matrix_live()),live_bytes=matrix_live())
    review=fit_admission.admission._load(fit_admission.review_pin,32768,
        decoded=True,live_bytes=matrix_live())
    if review['preflight']!=representation:
        raise ValueError('fixed40 matrix same-byte reviewed representation')
    del review
    profile=runtime._fit_profile_decode(fit_admission._profile_wire,matrix_live())
    events=tuple(profile['events']);source_identity=profile['source_identity'];del profile
    admission=fit_admission.admission
    def publish(name,value):
        if matrix_live(value)+3*core.RECEIPT+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 full matrix publication caller lifetime')
        body=value if type(value) is bytes else core.canonical(value)
        if len(body)>core.RECEIPT:raise ValueError('fixed40 matrix record cap')
        path=directory/name
        fd=os.open(str(path),os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        try:
            with os.fdopen(fd,'wb',closefd=False) as output:
                output.write(body);output.flush();os.fsync(fd)
        finally:os.close(fd)
        return dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest())
    failed=False
    for index,(route,context) in enumerate(zip(routes,contexts)):
        if failed:
            rows.append(dict(route=route,executed=False,completed=False,
                status='NOT_EXECUTED_AFTER_TERMINAL_ROUND_FAILURE'))
            continue
        def create_route_counter():
            nonlocal counters
            if counters is not None:
                raise ValueError('fixed40 route counter constructed once after controls')
            counters=fit_admission.counter_type(runtime.OFFLINE_FIT_ROOT,
                context['run'],context['event_history'],namespace='development',
                round_index=round_index,slot=0,context=context,context_roster=roster,
                trace_io=True,live_bytes=matrix_live())
            compiled['bridge_caller_charge']=matrix_live()+MATRIX_FAILURE_PUBLICATION_CARRY
            return counters
        try:
            compiled=compile_selected40(consumed_events=events,
                event_history=context['event_history'],admission=admission)
            if not callable(trace_publication_charge):
                raise ValueError('fixed40 complete route trace storage reservation required')
            admission.seal_read_trace(str(directory/('route-'+str(index)+'-admission-trace.json')),
                publication_charge=trace_publication_charge,live_bytes=matrix_live()+32768)
            # Every admitted Unit guard now owns both the complete outer roots
            # and its terminal-publication carry before any unit input opens.
            compiled['bridge_caller_charge']=matrix_live()+MATRIX_FAILURE_PUBLICATION_CARRY
            compiled['matrix_storage_binding']=output_publication_charge.binding
            raw,completion=run_selected40_fit(compiled,inputs,counters,context,
                offline_scope=offline_scope,representation=representation,
                read_charge=read_charge,return_completion=True,
                failure_publication_carry=MATRIX_FAILURE_PUBLICATION_CARRY,
                counter_factory=create_route_counter)
            receipt_pin=publish('route-'+str(index)+'-receipt.json',completion)
            result=publish('route-'+str(index)+'-result.json',raw)
            name,local=route.rsplit(':',1)
            success_rows.append(dict(route=name,index=int(local),context=context,
                receipt=receipt_pin,result=result))
            rows.append(dict(route=route,executed=True,completed=True,status='PASS',
                result=result,receipt=receipt_pin))
        except BaseException as error:
            # Pre-admission/compilation/publication failures have no proved
            # terminal Unit carry. Preserve their actual outer-command failure;
            # never invent an admitted terminal round or serialize it unguarded.
            bound=getattr(error,'fixed40_matrix_retained_fit_peak',None)
            if bound!=core.SEMANTIC-MATRIX_FAILURE_PUBLICATION_CARRY:raise
            if matrix_live()>bound:raise
            failure_pin=_matrix_failure_publication(error,route,
                directory/('route-'+str(index)+'-failure.json'),retained_fit_peak=bound)
            rows.append(dict(route=route,executed=True,completed=False,status='FAIL',diagnostic=failure_pin))
            failed=True
        # No earlier Counter/fit graph survives merely for closeout. The actual
        # bodies, full traceback during publication and every debit remain owned.
        counters=compiled=raw=completion=create_route_counter=None
    final_context=contexts[sum(row['executed'] for row in rows)-1]
    cold=fit_admission.counter_type(runtime.OFFLINE_FIT_ROOT,
        final_context['run'],final_context['event_history'],namespace='development',
        round_index=round_index,slot=0,context=final_context,context_roster=roster,
        trace_io=True,live_bytes=matrix_live())
    cold.begin_io_scope()
    closeout=cold.close_fit_debits(incomplete=failed,live_bytes=matrix_live())
    diagnostic=dict(schema='fixed40-actual-input-fit-matrix-diagnostic-v1',representation=representation,
        routes=rows,executed=sum(row['executed'] for row in rows),
        completed=sum(row['completed'] for row in rows),complete_fit=not failed,
        accepted=False,ready=False,new_facts=0,closeout=closeout)
    if failed:return publish('matrix-diagnostics.json',diagnostic),diagnostic
    matrix=actual_fit_matrix(source_identity,representation,round_index,success_rows,
        live_bytes=matrix_live())
    return publish('matrix.json',matrix),matrix


def run_selected40_fit(compiled, inputs, counters, context, *, offline_scope=None,
                       representation=None, read_charge=None, return_completion=False,
                       failure_publication_carry=0,counter_factory=None):
    """Execute all archive bindings and future producers/consumers in one admission.

    Every later route must run this full port again. No receipt substitutes for
    required bodies. No DB operation, binary launch or counter initialization is
    available. Caller must publish the returned bounded result exclusively.
    """
    if failure_publication_carry not in (0,MATRIX_FAILURE_PUBLICATION_CARRY):
        raise ValueError('fixed40 exact terminal-publication carry')
    if compiled.get('bridge_caller_charge',0)<failure_publication_carry:
        raise ValueError('fixed40 terminal carry before invocation')
    preparation_owner=compiled.get('bridge_caller_charge',0)
    if (preparation_owner+graph_charge((compiled,inputs,context,offline_scope,
            representation,read_charge,counter_factory))+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 pure preparation entry lifetime')
    extra, instructions, sources, tools, historical = semantic_units(
        compiled, inputs, offline_scope=offline_scope, include_historical=True,
        live_bytes=preparation_owner+graph_charge((context,representation,read_charge,counter_factory)))
    comparison_sources = sources if historical is None else historical['sources']
    ids, wires, setup_count = invocation_wire_manifest(compiled, extra,
        live_bytes=preparation_owner+graph_charge((inputs,sources,tools,instructions,historical)))
    # Reject missing prerequisites before changing a reused compiler object.
    if representation is None or not callable(read_charge):
        framing_partition(compiled,extra,representation=representation,read_charge=read_charge)
    archive_count=len(compiled['unit_ids'])
    compiled['archive_manifest_binding']=core.frozen_manifest_digest(ids,wires)
    del compiled['units'],compiled['setup_unit_wire'],compiled['unit_ids'],compiled['sources']
    del extra
    partition = framing_partition(compiled, (), representation=representation,
        read_charge=read_charge, archive_manifest=(ids,wires),
        live_bytes=preparation_owner+graph_charge((inputs,sources,tools,instructions,historical)))
    def program_section(name,retained=()):
        raw=partition.pop(name)
        if (preparation_owner+graph_charge((compiled,inputs,sources,tools,instructions,historical,
                         partition,ids,wires,raw,retained))
                +2*len(raw)+160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 complete program predecode lifetime')
        return records.decode(raw,records.MANIFEST_CAP)[1]
    future=program_section('future81_wire')
    pool=program_section('unit_pool_wire',future)
    full_pool_ids,full_pool_wires=expand_complete_fit_units(pool,
        live_bytes=preparation_owner+graph_charge((compiled,inputs,sources,tools,instructions,historical,
                                partition,ids,wires,future)))
    if (full_pool_ids[:len(ids)]!=ids or full_pool_wires[:len(wires)]!=wires):
        raise ValueError('selected40 complete archive unit pool equality')
    future_pool=full_pool_wires[len(wires):]
    expected_future_ids=full_pool_ids[len(ids):]
    del pool,full_pool_ids,full_pool_wires
    program_caller=preparation_owner+graph_charge((compiled,inputs,sources,tools,instructions,historical,partition))
    ids, wires = _fit_check_program(future, future_pool, ids, wires, compiled['maxima'],live_bytes=program_caller)
    if ids[-len(expected_future_ids):]!=expected_future_ids:
        raise ValueError('selected40 exact complete future unit identities')
    del future_pool,expected_future_ids
    control_caller=(compiled.get('bridge_caller_charge',0)
        +graph_charge((compiled,inputs,sources,tools,instructions,historical,
                      partition,ids,wires,future,offline_scope,counter_factory)))
    control_projection=[0]
    expected_controls=iter_complete_fit_control_sections(compiled,inputs,sources,tools,
        offline_scope,future,ids,wires,partition['routes'],
        compiled.get('matrix_storage_binding'),live_bytes=control_caller+1024,
        _owner_observer=lambda value:control_projection.__setitem__(0,value))
    verify_complete_fit_control_receipts(partition,expected_controls,live_bytes=control_caller+1024,
        owner_charge=lambda:control_projection[0])
    del expected_controls,control_caller,control_projection
    if not partition['control_semantics_checked']:
        raise ValueError('selected40 all control semantics before counter IO')
    route_name = context['route']+':'+context['phase'].rsplit(':',1)[-1]
    route_rows = [row for row in partition['routes'] if row['route'] == route_name]
    if len(route_rows) != 1 or route_rows[0]['manifest'] != core.frozen_manifest_digest(ids,wires):
        raise ValueError('selected40 exact full route manifest')
    del route_rows
    future_width=sum(_framing_json_width(row)[0]+129 for row in future)
    if (preparation_owner+graph_charge((compiled,inputs,sources,tools,instructions,historical,partition,ids,wires,future))
            +future_width+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('selected40 future context ownership transfer lifetime')
    future_wires=tuple(core.canonical(row) for row in future)
    del future
    # Setup prefix shares this invocation and admission; gate unit identities stay fixed.
    if context['manifest'] != core.frozen_manifest_digest(ids, wires):
        raise ValueError('selected40 exact invocation manifest')
    if context['source'] != core.digest((sources, tools)):
        raise ValueError('selected40 exact current source/verifier closure')
    nonbinary = binary = 0
    for raw in wires:
        if (preparation_owner+graph_charge((compiled,inputs,sources,tools,instructions,
                historical,partition,ids,wires,future_wires))+2*len(raw)
                +160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('selected40 full declared read schedule predecode lifetime')
        rows = records.decode(raw, records.MANIFEST_CAP)
        for occurrence, pin, kind, mode in rows:
            if kind == 'binary':
                binary += pin[1]+1
            elif not mode.startswith('member:'):
                nonbinary += pin[1]+1
        nonbinary += len(raw)
        if rows:del occurrence,pin,kind,mode
        del rows
    nonbinary += 32768  # Cold counter IO, precharged by the caller's framing scope.
    # Transfer the existing wire objects before the core validates/retains them.
    # The compiler does not retain a second logical graph beside this tuple.
    # All aliases/source-roster predicates have passed in semantic_units. The
    # final wires own those bindings; retained duplicate compiler indexes are
    # not needed while the invocation authenticates their complete bodies.
    if counters is None:
        if not callable(counter_factory):
            raise ValueError('selected40 counter factory after complete controls required')
        counters=counter_factory()
        counter_factory=None
    elif counter_factory is not None:
        raise ValueError('selected40 existing counter cannot also have a factory')
    caller_live = (graph_charge((compiled, inputs, sources, tools, instructions, partition, historical, future_wires))
                   + compiled.get('bridge_caller_charge', 0))
    counters.begin_io_scope()
    invocation = core.Invocation(counters, context, 0, ids, wires, nonbinary, binary,
                                 serialized=True, live_bytes=caller_live,release_completed_wires=True)
    del wires
    if tuple(invocation.consumed_events) != compiled['consumed_events']:
        invocation.fail()
        raise ValueError('selected40 durable consumed-event vector differs from compiler')
    values = {}; future_completed = []
    future_start = setup_count+archive_count+len(instructions)
    child_count = child_bytes = 0
    identities = {}
    def execute_unit(index,unit):
        nonlocal child_count,child_bytes
        rows = unit.declarations
        actual_live = graph_charge((compiled, inputs, sources, tools, values, instructions, identities, partition, historical, future_wires, future_completed))
        live = max(0, actual_live-invocation.caller_charge)
        if invocation.metadata_charge()+unit.declaration_charge+live+core.CHUNK+262144 > core.SEMANTIC:
            raise ValueError('selected40 complete live metadata allocation')
        if index < setup_count:
            verify_setup_unit(unit, index, live)
            return
        if index >= future_start:
            future_index = index-future_start
            step_raw=future_wires[future_index//3]
            if (invocation.metadata_charge()+unit.declaration_charge+live
                    +2*len(step_raw)+160*core._wire_nodes(step_raw)
                    +2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('selected40 active future step predecode lifetime')
            step=records.decode(step_raw,records.MANIFEST_CAP)
            # Constructor values account for their own full graphs. The
            # live state container, bindings and all other callers stay
            # charged here; the native owned-wire fix stays unchanged.
            _fit_execute_future_unit(future_index,step,values,unit,
                maxima=compiled['maxima'],compiled=compiled,identities=identities,
                live_bytes=live+graph_charge((step,step_raw)),read_charge=read_charge)
            if future_index%3 == 2: future_completed.append(step['role'])
            del step,step_raw
            return
        archive_index = index-setup_count
        if archive_index < archive_count:
            first = rows[0]
            if first[3].startswith('history:'):
                part = int(first[3].split(':')[1])
                unit.history(first[0], _pin(first[1]), part, rows[1][0], len(rows)-1)
            else:
                for occurrence, pin, kind, mode in rows:
                    unit.consume(occurrence, _pin(pin), kind=kind)
            return
        action, argument = instructions[archive_index-archive_count]
        occurrence, pin, kind, mode = rows[0]
        if action == 'input':
            value = unit.decode(occurrence, _pin(pin), live_bytes=live)
            if argument == 'source_freeze' and value != sources:
                raise ValueError('selected40 reauthenticated source freeze')
            if argument == 'tool_freeze' and value != tools:
                raise ValueError('selected40 reauthenticated tool freeze')
            if argument == 'native_comparison':
                _native_payload(value, comparison_sources, tools, inputs['test_executable'],
                                unit, occurrence, live+graph_charge(value), inputs['native_reports'],
                                NATIVE_GENERATOR_LOGICAL if compiled.get('runtime_admission') else None)
            del value
        elif action == 'gate':
            gate_raw = unit.consume(rows[0][0], _pin(rows[0][1]), retain=True, live_bytes=live)
            gate = unit.decode_control_consumed(rows[0][0], gate_raw, role='selected40-saved-result', live_bytes=live)
            del gate_raw
            output = unit.consume(rows[1][0], _pin(rows[1][1]), retain=True,
                                  live_bytes=live+graph_charge(gate))
            gate_source = inputs['source_freeze'] if historical is None else historical['source_freeze']
            if gate.get('source_freeze') != gate_source or gate.get('test_executable') != inputs['test_executable']:
                raise ValueError('selected40 current comparison execution/source binding')
            verify_saved_gate(gate, inputs['saved_log'], output)
            del gate, output
        elif action == 'historical_projection':
            verify_historical_projection(unit.decode(occurrence, _pin(pin), live_bytes=live), compiled['context']['historical_equivalence_parse'])
        elif action == 'seal':
            seal = unit.decode(occurrence, _pin(pin), live_bytes=live)
            exact = dict(schema=1, full_proofs=93, fresh_prior_proofs=71, constructor_proofs=51,
                         payload_proofs=42, full_context_requests=48, accepted_facts_total=805,
                         residual_requests=109, new_facts=42, independent_acceptance=True,
                         guard_permanently_consumed=True, lead_independent_reports_byte_identical=True)
            if any(type(seal.get(k)) is not type(v) or seal[k] != v for k,v in exact.items()):
                raise ValueError('selected40 original accepted fixed42 semantic authority')
        elif action == 'shared_pair':
            before = unit.decode(rows[0][0], _pin(rows[0][1]), live_bytes=live)
            after = unit.decode(rows[1][0], _pin(rows[1][1]), live_bytes=live+graph_charge(before))
            verify_shared_pair(before, after, argument, comparison_sources, inputs['test_executable'])
            del before, after
        elif action == 'scope36':
            scope = unit.decode(occurrence, _pin(pin), live_bytes=live)
            if scope['failure_retention_amendment'] != compiled['context']['retention_amendment']:
                raise ValueError('selected40 exact original retention scope')
            del scope
        elif action == 'record':
            placement = compiled['context']['reference_placements'][(28, 37, 49).index(argument)]
            prefix = (compiled['context']['source28_prefix'] if argument == 28 else
                      setup_role49_binding(inputs, compiled) if argument == 49 else '')
            result = verify_record_unit(unit, placement, argument, rows, prefix, live, identities)
            child_count += result['reference_count']; child_bytes += result['referenced_bytes']
        else:
            unit.consume(occurrence, _pin(pin), kind=kind)
    try:
        for index, identity in enumerate(ids):
            if index==future_start:
                # All archive/source/gate/child predicates have now run. Only
                # these reviewed wrapper fields are needed by future helpers.
                # Drop the actual roots before reducing their allocation owner.
                bridge=compiled.get('bridge_caller_charge',0)
                kept=dict(maxima=compiled['maxima'],schedule=compiled['schedule'],
                    aliases=compiled['aliases'],bridge_caller_charge=bridge,
                    context={name:compiled['context'][name] for name in
                        ('reference_placements','source28_prefix')})
                compiled.clear();compiled.update(kept);del kept
                sources=tools=instructions=historical=comparison_sources=()
                invocation.caller_charge=bridge+graph_charge((compiled,inputs,sources,tools,
                    values,instructions,identities,partition,historical,future_wires,future_completed))
            with invocation.unit(identity) as unit:
                execute_unit(index,unit)
        if (values or child_count != 64 or child_bytes != 1048091
                or sorted(future_completed) != list(range(81))):
            raise ValueError('selected40 complete semantic closure')
        complete_actual = graph_charge((compiled, inputs, sources, tools, values, instructions, identities, partition, historical, future_wires, future_completed))
        complete_live = max(0, complete_actual-invocation.caller_charge)
        if invocation.metadata_charge()+complete_live+core.RECEIPT*3+262144+core.CHUNK > core.SEMANTIC:
            raise ValueError('selected40 completion receipt pre-allocation')
        receipt = invocation.complete()
        counter_io = counters.io_summary()
        if (counter_io['physical_operations'] > 6 or counter_io['attempted_bytes'] > 32768
                or counter_io['reserved_bytes'] != 32768
                or not all(row['authenticated'] for row in counter_io['versions'])):
            raise ValueError('selected40 complete authenticated counter IO scope')
    except BaseException as error:
        try:invocation.fail()
        except BaseException as cleanup:
            try:error.__dict__['fixed40_fit_cleanup_error']=type(cleanup).__name__
            except BaseException:pass
            raise error
        if failure_publication_carry:
            # All Unit/constructor guards included this reserve through their
            # immutable invocation caller charge. The bound includes E once.
            error.__dict__['fixed40_matrix_retained_fit_peak']=(
                core.SEMANTIC-failure_publication_carry)
        raise
    result = dict(schema='selected40-complete-offline-fit-v1', complete_fit=True,
                  component_pass=True,
                  accepted=False, ready=False, new_facts=0,
                  inherited_events=BASELINE_EVENTS, inherited_event_history=BASELINE_HISTORY,
                  full_current81_reserved_bytes=10992546, schedule=compiled['schedule'],
                  setup_authority=dict(finite_rows=175, existing_bodies=61, pending_rows=114,
                                       reauthenticated_units=setup_count, same_invocation=True,
                                       publication=S3_PUBLICATION, effective_setup=11423963,
                                       effective_nonbinary_lifecycle=35964936624),
                  semantic_child_count=child_count, semantic_child_bytes=child_bytes,
                  framing_partition=partition, counter_io=counter_io, full_current_shared_reports=17,
                  invocation_receipt=json.loads(receipt), deferred_production_checks=DEFERRED,
                  future_producers_completed=future_completed,
                  future_consumers_completed=future_completed,
                  prior_bindings_completed=83)
    result_actual = graph_charge((compiled, inputs, sources, tools, values, instructions, identities, partition, result, historical, future_wires, future_completed))
    result_growth = max(0, result_actual-invocation.caller_charge)
    if invocation.metadata_charge()+result_growth+len(receipt)+128+core.RECEIPT*3 > core.SEMANTIC:
        raise ValueError('selected40 result serialization pre-allocation')
    raw = core.canonical(result)
    if len(raw) > core.RECEIPT:
        raise ValueError('selected40 full fit serialization cap')
    return (raw,receipt) if return_completion else raw


def matrix_output_reservation(output_directory, *, live_bytes=0):
    """Closed full-size output hold; it allocates no storage or authority."""
    core.integer(live_bytes,core.SEMANTIC,"fixed40 matrix reservation caller")
    if live_bytes+graph_charge(output_directory)+32768+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError("fixed40 complete matrix reservation lifetime")
    directory=str(records.canonical_path(output_directory))
    per_attempt=dict(receipts=16,results=16,terminal_failure=1,matrix_or_diagnostic=1)
    phases=('normal','failure','recovery','closeout')
    return dict(schema='fixed40-complete-matrix-output-reservation-v1',
        directory=directory,record_maximum=core.RECEIPT,per_attempt=per_attempt,
        per_attempt_bytes=sum(per_attempt.values())*core.RECEIPT,
        phases=list(phases),total_bytes=len(phases)*sum(per_attempt.values())*core.RECEIPT,
        exclusive=True,overwrite=False,read_bank=False,accepted=False)


def actual_fit_matrix(source_identity,preflight,round_index,results, *, live_bytes=0):
    """Close the actual matrix fields; genuine receipt bodies remain mandatory."""
    core.integer(live_bytes,core.SEMANTIC,"fixed40 actual matrix caller")
    if live_bytes+graph_charge((source_identity,preflight,results))+32768+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError("fixed40 actual matrix full caller lifetime")
    core.sha(source_identity);_pin_tuple(preflight)
    core.integer(round_index,1,'fixed40 actual matrix round')
    if type(results) is not list or len(results)!=16:
        raise ValueError('fixed40 actual complete matrix requires16 results')
    bindings=[]
    for row,route in zip(results,framing_routes()):
        name,index=route.rsplit(':',1)
        if (type(row) is not dict or set(row)!={'route','index','context','receipt','result'}
                or row['route']!=name or type(row['index']) is not int
                or row['index']!=int(index) or type(row['context']) is not dict
                or row['context'].get('route')!=name
                or row['context'].get('phase')!='development:'+str(round_index)+':0:'+index):
            raise ValueError('fixed40 exact actual matrix context order')
        _pin_tuple(row['receipt']);_pin_tuple(row['result'])
        if not 0<row['receipt']['bytes']<=core.RECEIPT or not 0<row['result']['bytes']<=core.RECEIPT:
            raise ValueError('fixed40 full actual matrix record bounds')
        bindings.append(core.digest(row['context']))
    if len(set(bindings))!=16:
        raise ValueError('fixed40 actual matrix unique contexts')
    return dict(schema='fixed40-actual-input-offline-fit-matrix-v1',
        source_identity=source_identity,preflight=preflight,round=round_index,results=results)


class RecoveryCounters(core.Counters):
    """Fixed40 additive admission; never initialize or replace a missing root.

    Migration is a separate, externally reviewed action. Preparation and tests
    must use cloned fixture roots. Production callers must authenticate the
    complete preflight and frozen source before calling migrate(). Each object
    binds one exact invocation context and one finite namespace/round/slot;
    all global charges include the immutable legacy state. No receipt credits
    any resource. A pending crash blocks further admission until reviewed.
    """
    PLAN_SHA = '40aabe009d6418dc7c0c58b317a1375df166730a4a330c01895f163003ff717a'
    LEGACY_SHA = '3e8e3c9fee95ff32b8b4efdbe20424f9f6ced376c3dd0e16efa162a6705b1f47'
    LIMITS = dict(invocations=81, units=164414, seconds=24300,
                  nonbinary=37405828546, binary=392625375570)
    EVENT_LIMITS = (35, 4, 4)
    DEVELOPMENT_ROUNDS = 2
    EXTRA = {'legacy_raw', 'recovery', 'development', 'production', 'namespace_roots', 'failed_development', 'round_failure_events'}

    @classmethod
    def initialize(cls, *args, **kwargs):
        raise ValueError('fixed40 recovery must migrate its existing root')

    @classmethod
    def limit_identity(cls):
        return core.digest((cls.PLAN_SHA, cls.LIMITS, cls.EVENT_LIMITS,
                            core.Counters.limit_identity(), core.ROUTES))

    def __init__(self, directory, binding, event_history, *, namespace, round_index,
                 slot, context, context_roster, trace_io=True):
        if trace_io is not True:
            raise ValueError('fixed40 recovery requires complete IO tracing')
        if namespace not in ('development', 'production'):
            raise ValueError('fixed40 recovery namespace')
        core.integer(round_index, self.DEVELOPMENT_ROUNDS-1 if namespace == 'development' else 0, 'fixed40 round')
        core.integer(slot, 0 if namespace == 'development' else 2, 'fixed40 namespace slot')
        self.namespace = namespace
        self.round_index = round_index
        self.slot = slot
        # A full immutable canonical context binds phase, route, exact manifest,
        # source, verifier, executable pair and all original authority digests.
        self.context_wire = core.canonical(context)
        self.context_binding = core.digest(context)
        if type(context_roster) is not tuple or len(context_roster) != 16 or len(set(context_roster)) != 16:
            raise ValueError('fixed40 exact sixteen-context roster')
        for value in context_roster:
            core.sha(value)
        if self.context_binding not in context_roster:
            raise ValueError('fixed40 context outside frozen roster')
        self.round_binding = core.digest(context_roster)
        super().__init__(directory, binding, event_history, trace_io=trace_io)

    def validate_invocation(self, context, slot):
        if (slot != self.slot or core.canonical(context) != self.context_wire
                or context['phase'].split(':', 3)[:3] != [self.namespace, str(self.round_index), str(self.slot)]):
            raise ValueError('fixed40 exact namespace/context binding')

    @staticmethod
    def _vectors(count):
        return [dict.fromkeys(core.ROUTES, 0) for _ in range(count)]

    @classmethod
    def migrate(cls, directory, *, consumed_events, event_records, accepted_preflight):
        """Atomically carry exact old bytes and append reviewed recovery identity.

        accepted_preflight is the authenticated independent review digest supplied
        by the caller after its exact source/artifact validation, not a Boolean.
        Event record hashes cover every consumed marker9..consumed, in order.
        This method supplies no acceptance or physical authority of its own.
        """
        core.sha(accepted_preflight)
        if type(consumed_events) is not tuple or len(consumed_events) != 3:
            raise ValueError('fixed40 effective event vector')
        for i, n in enumerate(consumed_events):
            core.integer(n, cls.EVENT_LIMITS[i], 'fixed40 event ceiling')
        if consumed_events[0] < 21 or consumed_events[1:] != (0, 0):
            raise ValueError('fixed40 recovery event floor')
        if type(event_records) is not tuple or len(event_records) != consumed_events[0]-8:
            raise ValueError('fixed40 complete monotonic event history')
        for value in event_records:
            core.sha(value)
        # Base construction authenticates the unchanged exact old profile. The
        # lock is the SAME legacy lock; no new counter directory is created.
        base = core.Counters(directory,
            '200328ad6e5a5153a31a67cd05b0be30da41b060f45712568c7548cf3e6c6a4f',
            BASELINE_HISTORY, trace_io=True)
        with base._locked():
            value = base._read()
            consumed = base._io_trace[-1]
            if consumed['bytes'] != 734 or consumed['sha256'] != cls.LEGACY_SHA or not consumed['authenticated']:
                raise ValueError('fixed40 exact consumed legacy stream')
            raw = core.canonical(value)
            if (len(raw) != 734 or hashlib.sha256(raw).hexdigest() != cls.LEGACY_SHA
                    or value['pending'] is not None or value['failures'] != [8, 0, 0]):
                raise ValueError('fixed40 exact legacy migration input')
            legacy_raw = raw.decode('ascii')
            value.update(schema=3, limits=cls.limit_identity(),
                legacy_raw=legacy_raw,
                recovery=dict(plan=cls.PLAN_SHA, preflight=accepted_preflight,
                              events=core.digest(event_records), inherited=list(consumed_events)),
                development=cls._vectors(cls.DEVELOPMENT_ROUNDS), production=cls._vectors(3),
                namespace_roots=[None]*(cls.DEVELOPMENT_ROUNDS+3),
                failed_development=[False]*cls.DEVELOPMENT_ROUNDS,
                round_failure_events=[None]*cls.DEVELOPMENT_ROUNDS,
                failures=list(consumed_events), generation=core.integer(value['generation']+1,200000,'fixed40 migration generation'))
            # Preserve every old charge and add the separately reserved migration debit.
            value['nonbinary'] += core.COUNTER_IO_RESERVE
            base._write(value)
        return base.io_summary()

    def _read(self):
        # Same full regular-file/O_NOFOLLOW/read4097/write4096 contract as base;
        # only this fixed successor's schema/limits and additive vectors differ.
        import os
        import stat
        self._io_check(4097)
        raw = self._read_state_raw()
        if self.trace_io:
            self._io_trace.append(dict(operation='read', bytes=len(raw), attempted_bytes=4097,
                sha256=hashlib.sha256(raw).hexdigest(), generation=None, authenticated=False))
        value = records.decode(raw, 4096)
        if (type(value) is not dict or set(value) != self.FIELDS | self.EXTRA
                or type(value['schema']) is not int or value['schema'] != 3
                or value['binding'] != self.binding or value['history'] != self.event_history
                or value['limits'] != self.limit_identity()):
            raise ValueError('fixed40 recovery state identity')
        legacy = value['legacy_raw']
        if (type(legacy) is not str or len(legacy.encode()) != 734
                or hashlib.sha256(legacy.encode()).hexdigest() != self.LEGACY_SHA):
            raise ValueError('fixed40 immutable legacy raw')
        old = records.decode(legacy.encode(), 4096)
        if value['slots'] != old['slots'] or value['routes'] != old['routes']:
            raise ValueError('fixed40 immutable legacy route vectors')
        recovery = value['recovery']
        if (type(recovery) is not dict or set(recovery) != {'plan','preflight','events','inherited'}
                or recovery['plan'] != self.PLAN_SHA):
            raise ValueError('fixed40 recovery authority identity')
        core.sha(recovery['preflight']); core.sha(recovery['events'])
        for name, count in [('development', self.DEVELOPMENT_ROUNDS), ('production', 3)]:
            vectors = value[name]
            if type(vectors) is not list or len(vectors) != count:
                raise ValueError('fixed40 namespace vectors')
            for routes in vectors:
                if type(routes) is not dict or set(routes) != set(core.ROUTES):
                    raise ValueError('fixed40 namespace route identity')
                for route, maximum in core.ROUTES.items():
                    core.integer(routes[route], maximum, 'fixed40 route maximum')
                if sum(routes.values()) > 16:
                    raise ValueError('fixed40 namespace slot maximum')
        roots = value['namespace_roots']
        failed = value['failed_development']
        if type(roots) is not list or len(roots) != self.DEVELOPMENT_ROUNDS+3 or type(failed) is not list or len(failed) != self.DEVELOPMENT_ROUNDS or any(type(x) is not bool for x in failed):
            raise ValueError('fixed40 frozen round identities')
        vectors = value['development'] + value['production']
        for root, routes in zip(roots, vectors):
            if root is not None:
                core.sha(root)
            if (root is None) != (sum(routes.values()) == 0):
                raise ValueError('fixed40 frozen round binding accounting')
        failures = value['round_failure_events']
        if type(failures) is not list or len(failures) != self.DEVELOPMENT_ROUNDS:
            raise ValueError('fixed40 failed round event identities')
        for flag, event in zip(failed, failures):
            if flag:
                core.integer(event, self.EVENT_LIMITS[0], 'fixed40 round failure event')
            elif event is not None:
                raise ValueError('fixed40 unfailed round event')
        inherited = recovery['inherited']
        if type(inherited) is not list or len(inherited) != 3 or type(value['failures']) is not list or len(value['failures']) != 3:
            raise ValueError('fixed40 recovery event dimensions')
        for i, maximum in enumerate(self.EVENT_LIMITS):
            core.integer(inherited[i], maximum, 'fixed40 inherited events')
            core.integer(value['failures'][i], maximum, 'fixed40 consumed events')
            if value['failures'][i] < inherited[i] or inherited[i] < old['failures'][i]:
                raise ValueError('fixed40 event rollback')
        for key, maximum in self.LIMITS.items():
            core.integer(value[key], maximum, 'fixed40 cumulative '+key)
            if value[key] < old[key]:
                raise ValueError('fixed40 resource rollback')
        count = sum(sum(v.values()) for name in ('development','production') for v in value[name])
        if value['invocations'] != old['invocations']+count:
            raise ValueError('fixed40 cumulative namespace accounting')
        core.integer(value['generation'], 200000, 'fixed40 generation')
        if value['generation'] <= old['generation']:
            raise ValueError('fixed40 generation rollback')
        pending = value['pending']
        if pending is not None:
            if type(pending) is not list or len(pending) != 6:
                raise ValueError('fixed40 pending shape')
            namespace, round_index, slot, route, generation, binding = pending
            if namespace not in ('development','production') or route not in core.ROUTES:
                raise ValueError('fixed40 pending namespace/route')
            core.integer(round_index, self.DEVELOPMENT_ROUNDS-1 if namespace == 'development' else 0, 'fixed40 pending round')
            core.integer(slot, 0 if namespace == 'development' else 2, 'fixed40 pending slot')
            core.integer(generation, value['generation'], 'fixed40 pending generation')
            core.sha(binding)
        if self.trace_io:
            self._io_trace[-1].update(generation=value['generation'], authenticated=True)
        return value

    def admit(self, *, invocation=0, units=0, nonbinary=0, binary=0, seconds=0,
              failure_slot=None, slot=None, route=None, event_record=None):
        increments = dict(invocations=invocation, units=units, nonbinary=nonbinary,
                          binary=binary, seconds=seconds)
        if invocation:
            if invocation != 1 or type(invocation) is not int or not self._io_scope:
                raise ValueError('fixed40 single scoped invocation')
            if slot != self.slot or route not in core.ROUTES:
                raise ValueError('fixed40 invocation namespace binding')
            if json.loads(self.context_wire)['route'] != route:
                raise ValueError('fixed40 invocation exact route')
            core.integer(units, 2048, 'fixed40 invocation units')
            core.integer(seconds, 300, 'fixed40 invocation seconds')
            increments['nonbinary'] += core.COUNTER_IO_RESERVE
        elif slot is not None or route is not None or any((units, binary, seconds)):
            raise ValueError('fixed40 unexpected unscoped debit')
        if failure_slot is not None:
            core.integer(failure_slot, 2, 'fixed40 event slot')
            if invocation:
                raise ValueError('fixed40 distinct correction event')
            core.sha(event_record)
        elif event_record is not None:
            raise ValueError('fixed40 unexpected event record')
        for key, n in increments.items():
            core.integer(n, self.LIMITS[key], 'fixed40 debit '+key)
        with self._locked():
            value = self._read()
            if value['pending'] is not None:
                raise ValueError('fixed40 unresolved pending invocation')
            if invocation:
                if value['failures'][self.slot] >= self.EVENT_LIMITS[self.slot]:
                    raise ValueError('fixed40 no reserved failure outcome')
                index = self.round_index if self.namespace == 'development' else self.slot
                if self.namespace == 'development' and value['failed_development'][index]:
                    raise ValueError('fixed40 failed development round is terminal')
                root_index = index if self.namespace == 'development' else index+self.DEVELOPMENT_ROUNDS
                previous = value['namespace_roots'][root_index]
                if self.namespace == 'development' and index > 0 and previous is None:
                    failure_event = value['round_failure_events'][index-1]
                    if (not value['failed_development'][index-1] or failure_event is None
                            or value['failures'][0] <= failure_event
                            or self.round_binding == value['namespace_roots'][index-1]):
                        raise ValueError('fixed40 next round requires terminal failure and named correction')
                if previous is not None and previous != self.round_binding:
                    raise ValueError('fixed40 frozen round/source context changed')
                value['namespace_roots'][root_index] = self.round_binding
                routes = value[self.namespace][index]
                routes[route] = core.integer(routes[route]+1, core.ROUTES[route], 'fixed40 route exhaustion')
                if sum(routes.values()) > 16:
                    raise ValueError('fixed40 round exhaustion')
            for key, n in increments.items():
                value[key] = core.integer(value[key]+n, self.LIMITS[key], 'fixed40 lifetime '+key)
            if failure_slot is not None:
                value['failures'][failure_slot] = core.integer(value['failures'][failure_slot]+1,
                    self.EVENT_LIMITS[failure_slot], 'fixed40 event exhaustion')
                value['recovery']['events'] = core.digest((value['recovery']['events'],
                    failure_slot,value['failures'][failure_slot],event_record))
                value['nonbinary'] = core.integer(value['nonbinary']+core.COUNTER_IO_RESERVE,
                    self.LIMITS['nonbinary'],'fixed40 correction scope debit')
            value['generation'] = core.integer(value['generation']+1,200000,'fixed40 generation')
            if invocation:
                value['pending'] = [self.namespace,self.round_index,self.slot,route,value['generation'],self.context_binding]
            self._write(value)
            return value

    def finish(self, generation, failed=False):
        if type(failed) is not bool or not self._io_scope:
            raise ValueError('fixed40 scoped completion')
        with self._locked():
            value = self._read()
            pending = value['pending']
            expected = [self.namespace,self.round_index,self.slot,
                        json.loads(self.context_wire)['route'],generation,self.context_binding]
            if pending != expected:
                raise ValueError('fixed40 exact pending completion')
            if failed:
                if self.namespace == 'development':
                    value['failed_development'][self.round_index] = True
                value['failures'][self.slot] = core.integer(value['failures'][self.slot]+1,
                    self.EVENT_LIMITS[self.slot], 'fixed40 failure exhaustion')
                if self.namespace == 'development':
                    value['round_failure_events'][self.round_index] = value['failures'][self.slot]
                value['recovery']['events'] = core.digest((value['recovery']['events'],
                    pending,value['failures'], 'failed'))
            value['pending'] = None
            value['generation'] = core.integer(value['generation']+1,200000,'fixed40 generation')
            self._write(value)
            if self.trace_io:
                self._last_io_trace = tuple(self._io_trace)
                self._io_trace = []
                self._io_scope = False


MATRIX_STORAGE_ACTIVATION = dict(
    path='/home/fry/artifacts/vrf40-source132-prospective-20261021/matrix-storage-activation.json',
    bytes=6543,sha256='9a09aa960421e1b29c4b3bc200d7e33b1aecefd06b0e302bdfe482c420343887')


class _MatrixOutputOwner:
    """One durable write hold, never an Invocation or physical capability."""
    def __init__(self,binding,reservation,fd):
        self.binding=binding;self.reservation=reservation;self.fd=fd;self.paid=False

    def reserve(self,packet,*,live_bytes):
        core.integer(live_bytes,core.SEMANTIC,'fixed40 full matrix write caller')
        if live_bytes+graph_charge(packet)+3*32768+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 write debit preallocation lifetime')
        if self.paid or packet!=self.reservation:
            raise ValueError('fixed40 exact single matrix write reservation')
        raw=core.canonical(dict(schema='fixed40-matrix-write-debit-v1',column='write',
            binding=self.binding,reservation=packet,bytes=packet['total_bytes'],
            scope='BEFORE_CREATE and BEFORE_COUNTER_IO; capacity only; no read credit'))
        if len(raw)>32768:raise ValueError('fixed40 complete write debit control cap')
        view=memoryview(raw)
        while view:
            n=os.write(self.fd,view)
            if n<=0:raise ValueError('fixed40 durable matrix write debit')
            view=view[n:]
        os.fsync(self.fd);self.paid=True


def _matrix_cli_write_owner(request,directory,ledger,round_index,fd,*,live_bytes,read_charge):
    """Authenticate the fixed capacity owner and its reviewed framing binding.

    The caller still needs the exact OfflineFitAdmission and all complete
    program checks. This function grants no native read, route or attempt.
    Every body is fully authenticated; other framing semantics remain required
    by the complete fit consumer on each route.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 CLI write owner caller')
    context_width=_framing_json_width(request['contexts'])[0]
    if (not callable(read_charge) or live_bytes+graph_charge(request)+2*context_width
            +3*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 owner context digest preallocation')
    expected=dict(schema='fixed40-matrix-storage-binding-v1',activation=MATRIX_STORAGE_ACTIVATION,
        profile=request['profile'],contexts_digest=core.digest(request['contexts']),
        directory=str(records.canonical_path(directory)),ledger=str(records.canonical_path(ledger)),
        round=round_index,phase='normal' if round_index==0 else 'recovery')
    if type(round_index)is not int or round_index not in (0,1) or request['matrix_storage']!=expected:
        raise ValueError('fixed40 exact owner/profile/context/phase/round/directory binding')
    reservation=matrix_output_reservation(directory,live_bytes=live_bytes+graph_charge(expected))
    def checked(pin,cap):
        _pin_tuple(pin)
        if pin['bytes']>cap or (live_bytes+graph_charge((request,expected,reservation))
                +2*pin['bytes']+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 matrix owner full-body lifetime')
        return _read(pin)
    raw=checked(MATRIX_STORAGE_ACTIVATION,32768)
    if (live_bytes+graph_charge((request,expected,reservation))+2*len(raw)
            +160*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 matrix activation predecode lifetime')
    activation=records.decode(raw,32768);del raw
    fields={'schema','state','proposal','certificate','independent_review','inventory','review_metrics',
        'complete_accepted_proposal','existing_correction_result_owner','fit_round_rights',
        'future_native_preflights','global_storage_ceiling','matrix_total','max_fit_rounds',
        'native_allocation_after','native_allocation_before','native_authentication',
        'native_margin_after','native_required','new_native_margin_owner',
        'no_reset_refund_or_cap_increase','physical_authority','publication_authority','read_bank'}
    if (type(activation)is not dict or set(activation)!=fields
            or activation['schema']!='fixed40-matrix-storage-activation-v1'
            or activation['state']!='ACTIVE_CAPACITY_ONLY'
            or any(activation[k]!=v for k,v in dict(global_storage_ceiling=3167859430,
                native_allocation_before=1073741824,native_allocation_after=1058013184,
                native_required=1002608722,native_margin_after=55404462,
                matrix_total=17825792,existing_correction_result_owner=2097152,
                new_native_margin_owner=15728640,max_fit_rounds=2).items())
            or activation['physical_authority'] is not False or activation['read_bank'] is not False
            or activation['no_reset_refund_or_cap_increase'] is not True):
        raise ValueError('fixed40 activated disjoint matrix/native capacity')
    del fields
    # Authenticate every complete evidence body. The accepted proposal remains
    # lossless, including all phase owners and preserved future native holds.
    for name in ('proposal','certificate','independent_review','inventory','review_metrics'):
        source=activation[name]
        if type(source)is not dict or set(source)!={'path','bytes','sha256','eof'} or source['eof']!=1:
            raise ValueError('fixed40 complete matrix owner evidence pin')
        pin={k:source[k] for k in ('path','bytes','sha256')}
        if (live_bytes+graph_charge((request,expected,reservation,activation))
                +2*pin['bytes']+2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 owner evidence retained caller')
        body=checked(pin,131072)
        if name=='proposal':
            if (live_bytes+graph_charge((request,expected,reservation,activation))+2*len(body)
                    +160*core._wire_nodes(body)+2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('fixed40 full proposal predecode lifetime')
            proposal=records.decode(body,131072)
            if proposal!=activation['complete_accepted_proposal']:
                raise ValueError('fixed40 complete accepted capacity proposal')
            del proposal
        del body,pin,source
    del activation,name
    representation=request['representation'];_pin_tuple(representation)
    if representation['bytes']>core.SEMANTIC:raise ValueError('fixed40 complete representation cap')
    # _read is prepaid by the CLI; streaming below charges its complete packet
    # once without retaining a second full artifact body.
    read_charge((representation,));checksum=hashlib.sha256();pending=bytearray();total=position=0;matched=False
    sourcefd=os.open(records.canonical_path(representation['path']),os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    try:
        import stat
        if not stat.S_ISREG(os.fstat(sourcefd).st_mode):raise ValueError('fixed40 regular owner framing')
        while total<representation['bytes']:
            chunk=os.read(sourcefd,min(core.CHUNK,representation['bytes']-total))
            if not chunk:raise ValueError('fixed40 complete owner framing EOF')
            total+=len(chunk);checksum.update(chunk)
            if (live_bytes+graph_charge((request,expected,reservation,pending))+len(chunk)
                    +2*core.CHUNK+262144>core.SEMANTIC):
                raise ValueError('fixed40 owner framing pregrowth lifetime')
            pending.extend(chunk);del chunk
            while b'\n' in pending:
                end=pending.index(10)+1;line=bytes(pending[:end]);del pending[:end]
                if (len(line)>records.MANIFEST_CAP or position>=len(COMPLETE_FIT_SECTIONS)
                        or live_bytes+graph_charge((request,expected,reservation,pending))
                        +2*len(line)+160*core._wire_nodes(line)+2*core.CHUNK+262144>core.SEMANTIC):
                    raise ValueError('fixed40 owner framing section lifetime')
                pair=records.decode(line,records.MANIFEST_CAP)
                if type(pair)is not list or len(pair)!=2 or pair[0]!=COMPLETE_FIT_SECTIONS[position]:
                    raise ValueError('fixed40 complete owner section order')
                if pair[0]=='controls':
                    if type(pair[1])is not dict or pair[1].get('matrix_storage')!=expected:
                        raise ValueError('fixed40 same-byte representation matrix owner')
                    matched=True
                position+=1;del pair,line
            if len(pending)>records.MANIFEST_CAP:raise ValueError('fixed40 owner section maximum')
        if os.read(sourcefd,1) or pending or position!=len(COMPLETE_FIT_SECTIONS) or checksum.hexdigest()!=representation['sha256'] or not matched:
            raise ValueError('fixed40 full owner representation hash/EOF')
    finally:os.close(sourcefd)
    del pending,representation,checksum
    return _MatrixOutputOwner(expected,reservation,fd)


def _matrix_cli_summary(matrix,pin,*,live_bytes):
    """Report actual successful rows; terminal diagnostics stay distinct."""
    if (type(matrix)is dict and set(matrix)=={'schema','source_identity','preflight','round','results'}
            and matrix['schema']=='fixed40-actual-input-offline-fit-matrix-v1'):
        checked=actual_fit_matrix(matrix['source_identity'],matrix['preflight'],matrix['round'],matrix['results'],
                                 live_bytes=live_bytes+graph_charge((matrix,pin)))
        return dict(matrix=pin,executed=len(checked['results']),completed=len(checked['results']),
                    complete_fit=True,accepted=False,new_facts=0)
    return dict(terminal_diagnostic=pin,complete_fit=False,accepted=False,new_facts=0)


def complete_fit_main(argv=None):
    """Execute an exact independently reviewed request; no clone initialization.

    The request and its read ceiling must be reserved before this command is
    launched. The exclusive before-open ledger records every direct metadata
    read. Binary streams remain exclusively inside admitted core invocations.
    """
    import argparse
    import pa1_fixed40_runtime as runtime
    parser=argparse.ArgumentParser(description='fixed40 complete actual-input offline fit')
    parser.add_argument('--complete-fit-request',required=True)
    parser.add_argument('--request-bytes',required=True,type=int)
    parser.add_argument('--request-sha256',required=True)
    parser.add_argument('--read-ceiling',required=True,type=int)
    parser.add_argument('--read-ledger',required=True)
    parser.add_argument('--output-directory',required=True)
    parser.add_argument('--round-index',type=int,default=0)
    args=parser.parse_args(argv)
    core.integer(args.request_bytes,32768,'fixed40 command request cap')
    core.integer(args.read_ceiling,2038438304,'fixed40 admitted engineering read ceiling')
    if args.request_bytes==0 or args.read_ceiling==0:
        raise ValueError('fixed40 positive request/read reservation')
    request_pin=dict(path=str(records.canonical_path(args.complete_fit_request)),
        bytes=args.request_bytes,sha256=args.request_sha256)
    _pin_tuple(request_pin)
    ledger=str(records.canonical_path(args.read_ledger))
    fd=os.open(ledger,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    total=0
    def charge(packet):
        nonlocal total
        amount=0
        for pin in packet:
            _pin_tuple(pin);amount+=pin['bytes']+1
        if total+amount>args.read_ceiling:
            raise ValueError('fixed40 complete command direct-read reservation exhausted')
        total+=amount
        raw=core.canonical(dict(packet=packet,bytes_plus_EOF=amount,cumulative=total,
            ceiling=args.read_ceiling,scope='BEFORE_OPEN; no consumed-charge refund'))
        view=memoryview(raw)
        while view:
            wrote=os.write(fd,view)
            if wrote<=0:raise ValueError('fixed40 direct-read debit journal failure')
            view=view[wrote:]
        os.fsync(fd)
    original=_read
    def prepaid(pin):
        charge((pin,));return original(pin)
    # The fixed compiler's direct metadata helper is restored even on failure.
    global_read=globals()
    global_read['_read']=prepaid
    try:
        request=records.decode(prepaid(request_pin),32768)
        if (type(request) is not dict or set(request)!={
                'schema','profile','review','inputs','contexts','historical_scope','representation',
                'admission_trace','trace_storage_reservation','matrix_storage'}
                or request['schema']!='fixed40-complete-fit-request-v2'):
            raise ValueError('fixed40 exact complete command request schema')
        trace_reservation=request['trace_storage_reservation']
        core.integer(trace_reservation,1048576,'fixed40 complete finite trace storage')
        trace_storage=0
        def publish_trace(packet):
            nonlocal trace_storage
            amount=0
            for pin in packet:
                _pin_tuple(pin);amount+=pin['bytes']
            if trace_storage+amount>trace_reservation:
                raise ValueError('fixed40 complete trace storage reservation exhausted')
            trace_storage+=amount
            raw=core.canonical(dict(publication_packet=packet,reserved_bytes=amount,
                cumulative_publication_bytes=trace_storage,ceiling=trace_reservation,
                scope='BEFORE_CREATE; complete immutable trace; no read credit'))
            view=memoryview(raw)
            while view:
                count=os.write(fd,view)
                if count<=0:raise ValueError('fixed40 trace publication debit journal failure')
                view=view[count:]
            os.fsync(fd)
        # Finalize request-owned containers before computing their carried owner.
        inputs=request['inputs']
        inputs['shared_reports']=tuple(inputs['shared_reports'])
        inputs['native_reports']=tuple(inputs['native_reports'])
        cli_control_live=graph_charge((request,request_pin,args.__dict__,trace_reservation))+32768
        matrix_owner=_matrix_cli_write_owner(request,args.output_directory,ledger,args.round_index,
            fd,live_bytes=cli_control_live,read_charge=charge)
        external_owner=graph_charge((request,matrix_owner.__dict__,request_pin,args.__dict__,trace_reservation))+32768
        admission=runtime.RuntimeAdmission(request['profile'],read_charge=charge,
            live_bytes=external_owner)
        _pin_tuple(request['admission_trace'])
        admission.seal_read_trace(request['admission_trace']['path'],
            publication_charge=publish_trace,reuse_pin=request['admission_trace'],
            # RuntimeAdmission carries the complete CLI owner in _outer_live.
            live_bytes=0)
        fit=runtime.OfflineFitAdmission(admission,request['review'],read_charge=charge,
            # Inherit admission.caller_charge; no new external owner yet.
            live_bytes=0)
        contexts=[]
        for context in request['contexts']:
            context=dict(context);context['executables']=tuple(context['executables']);contexts.append(context)
        pin,matrix=run_selected40_matrix(fit,inputs,tuple(contexts),
            offline_scope=request['historical_scope'],representation=request['representation'],
            read_charge=charge,output_directory=args.output_directory,round_index=args.round_index,
            trace_publication_charge=publish_trace,output_publication_charge=matrix_owner)
        # request/inputs, owner and control metadata remain in the inherited
        # charge. Only these converted contexts were allocated afterwards.
        summary=_matrix_cli_summary(matrix,pin,
            live_bytes=fit.caller_charge+graph_charge(contexts))
        summary.update(direct_read_bytes_plus_EOF=total,trace_publication_bytes=trace_storage,
            matrix_write_reservation_bytes=matrix_owner.reservation['total_bytes'] if matrix_owner.paid else 0)
        print(json.dumps(summary,sort_keys=True))
        return 0 if summary['complete_fit'] else 1
    finally:
        global_read['_read']=original
        os.close(fd)





def capture_operation_reservation(value, maxima, *, live_bytes):
    """Check the closed IO contract for the sole, still guarded capture.

    This validates costs and locations, not physical permission. A production
    adapter must bind this complete record through its frozen argument pin,
    verify source/READY/full fit and storage ownership, and prepay the operation
    before Invocation admission. Input reads and output writes stay distinct.
    The native failure ceiling is deliberately larger than accepted role68.
    """
    import pa1_fixed40_constructors as constructors
    core.integer(live_bytes,core.SEMANTIC,"fixed40 capture reservation caller")
    if (live_bytes+graph_charge(value)+4*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError("fixed40 capture reservation lifetime")
    fields={"schema","profile","route","index","global_index","phase",
        "constructor_role","unit","source_identity","native_inputs",
        "outputs","marker","storage_owner","io"}
    if (type(value) is not dict or set(value)!=fields
            or value["schema"]!="fixed40-production-capture-operation-v1"
            or value["profile"]!="ResidualE946"
            or value["route"]!="capture" or type(value["index"]) is not int
            or value["index"]!=0 or type(value["global_index"]) is not int
            or value["global_index"]!=4 or value["phase"]!="production:0:0:0"
            or type(value["constructor_role"]) is not int
            or value["constructor_role"]!=63
            or type(value["unit"]) is not str or not value["unit"]
            or len(value["unit"].encode())>1024):
        raise ValueError("fixed40 sole capture route and construction-unit reservation")
    core.sha(value["source_identity"])
    if tuple(maxima)!=tuple(constructors.ROLE_MAXIMA):
        raise ValueError("fixed40 unchanged capture role envelopes")
    inputs=value["native_inputs"]
    if type(inputs) is not list or len(inputs)!=7:
        raise ValueError("fixed40 full seven native input reservations")
    for pin in inputs:_pin_tuple(pin)
    if sum(pin["bytes"] for pin in inputs)!=6785636:
        raise ValueError("fixed40 exact native raw input bytes")
    outputs=value["outputs"]
    if type(outputs) is not list or len(outputs)!=6:
        raise ValueError("fixed40 all six capture output roles")
    paths=[];native_written=0
    for role,row in zip(range(64,70),outputs):
        producer=(67109268 if role==68 else 131072 if role==69 else maxima[role])
        if (type(row) is not dict or set(row)!={"role","path","accepted_maximum","producer_maximum"}
                or type(row["role"]) is not int or row["role"]!=role
                or row["accepted_maximum"]!=maxima[role]
                or row["producer_maximum"]!=producer):
            raise ValueError("fixed40 accepted envelope versus full failed producer ceiling")
        paths.append(str(records.canonical_path(row["path"])))
        native_written+=producer
    marker=value["marker"]
    if (type(marker) is not dict or set(marker)!={"path","maximum","exclusive","fsync"}
            or marker["maximum"]!=maxima[63] or marker["exclusive"] is not True
            or marker["fsync"] is not True):
        raise ValueError("fixed40 one durable exclusive capture attempt")
    paths.append(str(records.canonical_path(marker["path"])))
    if len(set(paths))!=len(paths) or any(pin["path"] in paths for pin in inputs):
        raise ValueError("fixed40 distinct input, marker and output paths")
    _pin_tuple(value["storage_owner"])
    # stdout/stderr are each accepted up to64KiB. Failure retains another
    # read chunk plus a bounded drained pipe for each stream; no quiet truncation.
    expected=dict(native_reads=sum(pin["bytes"]+1 for pin in inputs),
        native_output_writes=native_written,pipe_reads=393218,
        native_output_rereads=67109268+131072+2,
        marker_reads=maxima[63]+1,marker_writes=maxima[63],
        stdout_success=65536,stderr_success=65536,
        stdout_failure=196608,stderr_failure=196608,
        result_publication=maxima[64],closeout_bytes=32768,
        invocation_seconds=300,kill_reap_seconds=5)
    if value["io"]!=expected:
        raise ValueError("fixed40 complete read/write/failure/closeout cost equality")
    return dict(read_bytes=expected["native_reads"]+expected["native_output_rereads"]
            +expected["marker_reads"]+expected["pipe_reads"],
        write_bytes=expected["native_output_writes"]+expected["marker_writes"]
            +expected["stdout_failure"]+expected["stderr_failure"]
            +expected["result_publication"]+expected["closeout_bytes"],
        seconds=300,kill_reap_seconds=5,
        accepted=False,physical_permission=False)


def _supervise_capture_streams(argv, *, cwd, environment, stdout_path,
        stderr_path, child_deadline, closeout_deadline, debit, live_bytes):
    """Bound one child and retain actual streams/exit; this grants no authority.

    Only the future source/READY/Invocation adapter may call this helper after
    all proof and reservation checks. There is deliberately no public CLI or
    automatic native launch here. debit records every stream reservation before
    its open/write; it is not a Unit input-read declaration or a second bank.
    """
    import selectors,subprocess,signal,time,math
    core.integer(live_bytes,core.SEMANTIC,"fixed40 capture supervisor caller")
    if (type(argv) is not list or not argv or any(type(x) is not str for x in argv)
            or cwd!=str(ROOT) or environment!={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"}
            or not callable(debit) or type(child_deadline) not in (int,float)
            or type(closeout_deadline) not in (int,float)
            or child_deadline<=time.monotonic()
            or not math.isfinite(child_deadline) or not math.isfinite(closeout_deadline)
            or closeout_deadline-child_deadline!=5
            or closeout_deadline-time.monotonic()>300):
        raise ValueError("fixed40 exact bounded capture supervision inputs")
    if (live_bytes+graph_charge((argv,cwd,environment,stdout_path,stderr_path))
            +8*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError("fixed40 capture supervisor complete stream lifetime")
    paths=(str(records.canonical_path(stdout_path)),str(records.canonical_path(stderr_path)))
    if paths[0]==paths[1]:raise ValueError("fixed40 distinct capture streams")
    debit(dict(operation="reserve_stream_io",paths=list(paths),
               per_stream=196608,read_total=393218,write_total=393216,phase="BEFORE_OPEN"))
    fds=[];process=None;selector=selectors.DefaultSelector();sizes=[0,0]
    digests=[hashlib.sha256(),hashlib.sha256()]
    read_calls=[0,0];write_calls=[0,0];eof=[False,False]
    failed=None;started=time.monotonic();complete=False
    primary_error=None;observed=None;pipes=[];exit_code=None
    group_signal_attempts=0;group_signal_deliveries=0
    def kill():
        nonlocal group_signal_attempts,group_signal_deliveries
        # The leader can exit while descendants still own pipe writers.
        if process is not None:
            group_signal_attempts+=1
            try:
                os.killpg(process.pid,signal.SIGKILL);group_signal_deliveries+=1
            except ProcessLookupError:pass
    try:
        for path in paths:
            fds.append(os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600))
        debit(dict(operation="child_launch",argv=argv,cwd=cwd,environment=environment,
                   deadline=child_deadline,closeout_deadline=closeout_deadline,phase="BEFORE_LAUNCH"))
        process=subprocess.Popen(argv,cwd=cwd,env=environment,stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
        pipes=[process.stdout,process.stderr]
        for index,stream in enumerate(pipes):
            os.set_blocking(stream.fileno(),False);selector.register(stream,selectors.EVENT_READ,index)
        while selector.get_map():
            now=time.monotonic()
            if now>=child_deadline and failed is None:failed="timeout";kill()
            if now>=closeout_deadline:failed="closeout_timeout";kill();break
            for key,_ in selector.select(min(0.05,closeout_deadline-now)):
                index=key.data
                block=os.read(key.fileobj.fileno(),65536);read_calls[index]+=1
                if not block:
                    eof[index]=True;selector.unregister(key.fileobj);key.fileobj.close();continue
                if sizes[index]+len(block)>196608:
                    failed="failure_stream_limit";kill();raise ValueError("fixed40 full failure stream reservation exhausted")
                view=memoryview(block)
                while view:
                    n=os.write(fds[index],view);write_calls[index]+=1
                    if n<=0:raise ValueError("fixed40 actual stream write failure")
                    view=view[n:]
                sizes[index]+=len(block);digests[index].update(block)
                if sizes[index]>65536 and failed is None:failed="output_limit";kill()
        # EOF can precede child exit. Execution never borrows closeout time.
        remaining=max(0.0,child_deadline-time.monotonic())
        try:
            if remaining==0 and process.poll() is None:
                raise subprocess.TimeoutExpired(argv,0)
            exit_code=process.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            if failed is None:failed="timeout"
            kill()
            try:exit_code=process.wait(timeout=max(0.001,closeout_deadline-time.monotonic()))
            except subprocess.TimeoutExpired:
                raise ValueError("fixed40 leader reap unconfirmed within closeout")
        complete=not selector.get_map() and all(eof) and exit_code is not None
        if exit_code not in (None,0) and failed is None:failed="nonzero_exit"
        for fd in fds:
            debit(dict(operation="stream_fsync",phase="BEFORE_FSYNC"));os.fsync(fd)
        observed=dict(schema="fixed40-observed-child-streams-v1",launched=True,
            exit_code=exit_code,capture_complete=complete,reason=failed,
            stdout=dict(path=paths[0],bytes=sizes[0],sha256=digests[0].hexdigest()),
            stderr=dict(path=paths[1],bytes=sizes[1],sha256=digests[1].hexdigest()),
            cleanup=dict(leader_reaped=exit_code is not None,pipe_writers_closed=all(eof),
                group_signal_attempts=group_signal_attempts,
                group_signal_deliveries=group_signal_deliveries,
                all_descendants_reaped=False),
            stream_io=dict(reserved_reads=393218,reserved_writes=393216,
                read_bytes=sum(sizes),write_bytes=sum(sizes),read_calls=read_calls,
                write_calls=write_calls,eof=eof),
            elapsed_seconds=time.monotonic()-started,accepted=False,
            physical_permission=False)
        return observed
    except BaseException as error:
        primary_error=error
        raise
    finally:
        # Every closer runs independently. Keep the primary error and append a
        # finite cleanup record; a later close failure must not replace it.
        cleanup_errors=[];first_cleanup_error=None;cleanup_reaped=False
        def attempt(operation,action):
            nonlocal first_cleanup_error
            try:return action()
            except BaseException as error:
                if first_cleanup_error is None:first_cleanup_error=error
                cleanup_errors.append(dict(operation=operation,
                    error_type=type(error).__name__.encode("ascii","backslashreplace")[:128].decode("ascii")))
                return None
        attempt("process_group_kill",kill)
        def reap():
            nonlocal cleanup_reaped,exit_code
            if process is None:return
            code=process.poll()
            if code is None:
                code=process.wait(timeout=max(0.001,closeout_deadline-time.monotonic()))
            cleanup_reaped=type(code) is int
            if cleanup_reaped:exit_code=code
            if not cleanup_reaped:raise ValueError("fixed40 cleanup leader reap unconfirmed")
        attempt("leader_reap",reap)
        for index,stream in enumerate(pipes):
            attempt("pipe_close:"+str(index),stream.close)
        attempt("selector_close",selector.close)
        for index,fd in enumerate(fds):
            attempt("sink_close:"+str(index),lambda fd=fd:os.close(fd))
        if observed is None:
            observed=dict(schema="fixed40-observed-child-streams-v1",
                launched=process is not None,exit_code=exit_code,
                capture_complete=all(eof) and type(exit_code) is int,
                reason=failed or "supervisor_exception",
                stdout=dict(path=paths[0],bytes=sizes[0],sha256=digests[0].hexdigest()),
                stderr=dict(path=paths[1],bytes=sizes[1],sha256=digests[1].hexdigest()),
                cleanup=dict(pipe_writers_closed=all(eof),all_descendants_reaped=False),
                stream_io=dict(reserved_reads=393218,reserved_writes=393216,
                    read_bytes=sum(sizes),write_bytes=sum(sizes),read_calls=read_calls,
                    write_calls=write_calls,eof=eof),
                elapsed_seconds=time.monotonic()-started,accepted=False,
                physical_permission=False)
        observed["cleanup"].update(leader_reaped=cleanup_reaped,
            pipe_close_attempts=len(pipes),sink_close_attempts=len(fds),
            selector_close_attempted=True,cleanup_errors=cleanup_errors,
            cleanup_complete=not cleanup_errors,
            group_signal_attempts=group_signal_attempts,
            group_signal_deliveries=group_signal_deliveries)
        # The adapter can publish this actual outcome without invoking a failed
        # debit callback again. A primary exception retains its exact identity.
        target=primary_error if primary_error is not None else first_cleanup_error
        if target is not None:
            if len(core.canonical(observed))>16384:
                raise AssertionError("fixed40 finite exception outcome record")
            try:target.__dict__["fixed40_observed_outcome"]=observed
            except BaseException:pass
        if cleanup_errors:
            note="fixed40 bounded cleanup errors: "+json.dumps(cleanup_errors,
                sort_keys=True,separators=(",",":"))
            if len(note.encode())>4096:
                raise AssertionError("fixed40 finite cleanup error record")
            target=primary_error if primary_error is not None else first_cleanup_error
            # Annotation failure must not replace the original exception.
            try:target.__dict__["fixed40_cleanup_errors"]=tuple(cleanup_errors)
            except BaseException:pass
            try:BaseException.add_note(target,note)
            except BaseException:pass
            if primary_error is None:raise first_cleanup_error


def _supervise_capture_with_outcome(argv, *, cwd, environment, stdout_path,
        stderr_path, outcome_path, child_deadline, closeout_deadline, debit,
        live_bytes):
    """Publish actual child outcomes after cleanup without a second callback.

    The caller must already own all stream and publication costs. This private
    component grants no READY, attempt, Invocation or native execution authority.
    Publication is prepaid before any child launch; a failed supervision debit
    cannot disable its failure-result path. UNKNOWN means unavailable evidence.
    """
    core.integer(live_bytes,core.SEMANTIC,"fixed40 outcome publisher caller")
    path=str(records.canonical_path(outcome_path))
    if path in (str(stdout_path),str(stderr_path)) or not callable(debit):
        raise ValueError("fixed40 distinct actual outcome path")
    owned=live_bytes+graph_charge((argv,cwd,environment,stdout_path,stderr_path,
        outcome_path,child_deadline,closeout_deadline,debit))+32768
    # The supervisor traceback can retain its full input arguments, eight
    # chunk buffers and fixed local/cleanup metadata. Prepay this before launch,
    # including repeated argument graphs; no traceback or alias discount.
    supervisor_frame_reserve=8*core.CHUNK+2*graph_charge((argv,cwd,environment,
        stdout_path,stderr_path,child_deadline,closeout_deadline,debit))+65536
    if owned+supervisor_frame_reserve+3*16384+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError("fixed40 actual outcome publication lifetime")
    # Proper write/metadata operations, never fictitious Unit input reads.
    debit(dict(operation="reserve_outcome_publication",phase="BEFORE_LAUNCH",
        path=path,maximum_bytes=16384,exclusive_create=True,write_bytes=16384,
        file_fsyncs=3,directory_fsyncs=3,close_attempts=6,
        stream_reopens=2,stream_directory_opens=2))
    primary=None;observed=None;publication_error=None;pin=None;fd=None;directory=None
    try:
        try:
            observed=_supervise_capture_streams(argv,cwd=cwd,environment=environment,
                stdout_path=stdout_path,stderr_path=stderr_path,
                child_deadline=child_deadline,closeout_deadline=closeout_deadline,
                debit=debit,live_bytes=owned)
        except BaseException as error:
            primary=error
            observed=getattr(error,"fixed40_observed_outcome",None)
        record=dict(schema="fixed40-durable-child-outcome-v1",observed=observed,
            outcome_availability="ACTUAL" if observed is not None else "UNKNOWN",
            supervisor_error=None if primary is None else
                type(primary).__name__.encode("ascii","backslashreplace")[:128].decode("ascii"),
            accepted=False,physical_permission=False)
        # A callback exception can retain the supervisor frame and its buffers.
        # Carry its complete eight-chunk reserve through failure publication.
        if owned+supervisor_frame_reserve+graph_charge((observed,record))+3*16384+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError("fixed40 full outcome record publication lifetime")
        raw=core.canonical(record)
        if len(raw)>16384:raise ValueError("fixed40 complete actual outcome record cap")
        # No callback occurs from this point: its complete cost is already held.
        # A debit failure may have occurred BEFORE the supervisor fsynced a
        # stream. Reopen/fsync actual existing sinks without reading their body.
        # Each local close is attempted even if fsync failed.
        for stream_path in ((stdout_path,stderr_path)
                if observed is not None and observed["launched"] else ()):
            stream_fd=None;parent_fd=None;sync_error=None
            try:
                stream_fd=os.open(str(stream_path),os.O_RDONLY|os.O_NOFOLLOW)
                parent_fd=os.open(str(Path(stream_path).parent),os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
                os.fsync(stream_fd);os.fsync(parent_fd)
            except BaseException as error:sync_error=error
            finally:
                for handle in (stream_fd,parent_fd):
                    if handle is not None:
                        try:os.close(handle)
                        except BaseException as error:
                            if sync_error is None:sync_error=error
            if sync_error is not None:raise sync_error
        directory=os.open(str(Path(path).parent),os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        offset=0
        while offset<len(raw):
            count=os.write(fd,memoryview(raw)[offset:])
            if count<=0:raise ValueError("fixed40 actual outcome write failed")
            offset+=count
        os.fsync(fd);os.fsync(directory)
        pin=dict(path=path,bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
    except BaseException as error:
        publication_error=error
    finally:
        # One close failure cannot skip another close or replace the primary.
        for handle in (fd,directory):
            if handle is not None:
                try:os.close(handle)
                except BaseException as error:
                    if publication_error is None:publication_error=error
    if primary is not None:
        try:
            primary.__dict__["fixed40_outcome_publication"]=dict(pin=pin,
                complete=pin is not None and publication_error is None,
                error_type=None if publication_error is None else
                    type(publication_error).__name__[:128])
        except BaseException:pass
        raise primary
    if publication_error is not None:raise publication_error
    return dict(observed=observed,outcome=pin,publication_complete=True,
        accepted=False,physical_permission=False)


if __name__=="__main__":
    raise SystemExit(complete_fit_main())
