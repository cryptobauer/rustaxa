"""Concrete fixed40 runtime closures; no capture or acceptance at import."""
from pathlib import Path
import pa1_evidence as core

OLD_AUX_ROOT=Path('/home/fry/artifacts/vrf40-projection-01a14b66-20261017/saved-v4')
OLD_AUX_NAMES=tuple('shared-profile-'+str(i)+'.json' for i in (*range(8),*range(10,17)))+('native-current-replay-v2.json','first-d-current-replay-v2.json')
OLD_AUX_BYTES=922518

def auxiliary_schedule(completed_visits=5):
    # Every runtime route rereads all retired report bodies, even though they
    # supply no current comparison authority. Preflight and closeout do too.
    routes=tuple(route+':'+str(i) for route,n in core.ROUTES.items() for i in range(n))
    if len(routes)!=16 or len(set(routes))!=16:
        raise ValueError('fixed40 exact sixteen auxiliary reauthentication routes')
    visits=('preflight-lead','preflight-independent',*routes,'closeout')
    if type(completed_visits) is not int or completed_visits<5:
        raise ValueError('fixed40 preserve completed auxiliary visits')
    # One separate admission-review visit is held before any new execution.
    capacity=completed_visits+1+len(visits)
    return dict(completed=completed_visits,admission_review=1,future_visits=visits,
                capacity=capacity,bytes_per_visit=OLD_AUX_BYTES+17,
                total_read_reservation=capacity*(OLD_AUX_BYTES+17))

def auxiliary_units(inventory,first_occurrence):
    if type(inventory) is not list or len(inventory)!=17:
        raise ValueError('fixed40 all seventeen retired auxiliary reports')
    by_name={}
    for pin in inventory:
        if type(pin) is not dict or set(pin)!=set(('path','bytes','sha256')):
            raise ValueError('fixed40 exact retired report pin')
        path=Path(pin['path'])
        if path.parent!=OLD_AUX_ROOT or path.name in by_name:
            raise ValueError('fixed40 retired report identity')
        core.integer(pin['bytes'],65536,'fixed40 retired individual body')
        core.sha(pin['sha256']);by_name[path.name]=pin
    if set(by_name)!=set(OLD_AUX_NAMES) or sum(p['bytes'] for p in inventory)!=OLD_AUX_BYTES:
        raise ValueError('fixed40 complete retired report bytes')
    core.integer(first_occurrence,199984,'fixed40 auxiliary occurrence')
    return tuple(((first_occurrence+i,(p['path'],p['bytes'],p['sha256']),'evidence','bytes'),)
                 for i,name in enumerate(OLD_AUX_NAMES) for p in (by_name[name],))

def reauthenticate_auxiliary(unit):
    if len(unit.declarations)!=1:
        raise ValueError('fixed40 one complete retired report per unit')
    occurrence,pin,kind,mode=unit.declarations[0]
    if kind!='evidence' or mode!='bytes':
        raise ValueError('fixed40 retired report stream contract')
    # Core consumes full regular-file bytes, EOF and hash; no hash-only cache.
    return unit.consume(occurrence,dict(zip(('path','bytes','sha256'),pin)))


# This bridge changes development admission only. It does not manufacture a
# successful preflight, source acceptance, READY, or a physical attempt.
import os
import stat
from contextlib import contextmanager, nullcontext
import hashlib
import fcntl
import secrets
import json
import math
import sys
import weakref
import pa1_fixed40 as fixed
import evidence_records as records
from pa1_development import DevelopmentReservations, CoherentDevelopmentReservations

RUNTIME_HARD = dict(invocations=101, units=205374, seconds=30300,
                    nonbinary=38020585776, binary=449080998528)
LATER_MINIMUM = dict(invocations=32, units=65536, seconds=9600,
                     nonbinary=26826002720, binary=351983904512)
class RetainedDevelopmentReservations(DevelopmentReservations):
    """Only the development retention dimension uses the reviewed64MiB bank.

    All original source/read/unit/time/control limits and exact settlement
    mechanics are inherited. Runtime setup remains independently16MiB.
    """
    NAMES = ('retention', *DevelopmentReservations.NAMES[1:])
    HARD = (67108864, *DevelopmentReservations.HARD[1:])


RESOURCE_INDEX = dict(nonbinary=2, binary=3, units=4, seconds=5)


@contextmanager
def _claim_cold_guard(*, clock, live_bytes=0):
    """Claim the sole migration-prepaid administrative bootstrap.

    Only reviewed migration may create the fixed control. This function reads
    no control, request, state or evidence body and grants no launch authority.
    The returned held descriptors belong to the caller until context close.
    The claim is permanent on every success, failure and interrupted closeout.
    The full control/state/review consumers must run before any physical marker.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 cold claim caller')
    if (live_bytes+fixed.graph_charge((FIT_PROTECTED_COUNTER,clock))+32768
            +2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 cold claim complete caller lifetime')
    started=clock()
    if type(started) not in (int,float) or not math.isfinite(started):
        raise ValueError('fixed40 finite cold guard clock')
    deadline=started+300
    root=Path(FIT_PROTECTED_COUNTER['path']).parent
    if any(parent.is_symlink() for parent in (root,*root.parents)):
        raise ValueError('fixed40 cold guard root symlink')
    directory=lock=claim=None;created=False
    try:
        directory=os.open(root,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        lock=os.open('lock',os.O_RDWR|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=directory)
        root_stat=os.fstat(directory);lock_stat=os.fstat(lock)
        if not stat.S_ISDIR(root_stat.st_mode) or not stat.S_ISREG(lock_stat.st_mode):
            raise ValueError('fixed40 cold guard directory and regular lock')
        # A busy predecessor fails closed; waiting cannot extend the external
        # lifecycle deadline or consume another producer's administrative owner.
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        control=os.stat('guard-bootstrap-control.json',dir_fd=directory,
                        follow_symlinks=False)
        if not stat.S_ISREG(control.st_mode) or not 0<control.st_size<=4096:
            raise ValueError('fixed40 missing bounded migration control')
        if clock()>=deadline:
            raise ValueError('fixed40 external guard deadline before claim')
        identity=((root_stat.st_dev,root_stat.st_ino,stat.S_IFMT(root_stat.st_mode)),
                  (lock_stat.st_dev,lock_stat.st_ino,stat.S_IFMT(lock_stat.st_mode)))
        raw=core.canonical(dict(schema='fixed40-cold-bootstrap-claim-v1',
                                filesystem=identity))
        if len(raw)>4096:raise ValueError('fixed40 administrative claim cap')
        claim=os.open('guard-bootstrap-claim.json',
            os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=directory)
        created=True
        # Persist the blocking name before a write/file-fsync can fail. Even an
        # empty or incomplete claim must prevent another prepaid bootstrap.
        os.fsync(directory)
        offset=0
        while offset<len(raw):
            count=os.write(claim,memoryview(raw)[offset:])
            if count<=0:raise ValueError('fixed40 complete administrative claim write')
            offset+=count
        os.fsync(claim);os.fsync(directory)
        os.close(claim);claim=None
        if clock()>=deadline:
            raise ValueError('fixed40 external guard deadline after claim')
        yield directory,lock,identity,deadline
    finally:
        # Attempt every closer. A failed close never removes the one-use claim.
        primary=sys.exc_info()[1];cleanup=None
        if primary is not None and created:
            for handle in (claim,directory):
                if handle is not None:
                    try:os.fsync(handle)
                    except BaseException as error:
                        if cleanup is None:cleanup=error
        for handle in (claim,lock,directory):
            if handle is not None:
                try:os.close(handle)
                except BaseException as error:
                    if cleanup is None:cleanup=error
        if primary is None and cleanup is not None:raise cleanup


def derive_runtime_envelope(consumed, protected, round_reservation, control_used,
                            inherited_events, *, control_hard=344148):
    """Derive finite routes/events from complete reserved costs, never candidates.

    consumed contains cumulative actual debits including the immutable legacy
    counters. protected holds all future37/32 obligations. round_reservation is
    the independently checked source-backed full sixteen-route cost including
    failure/receipt/control overhead. The current production round is held
    separately before any development round is available.
    """
    for row in (consumed, protected, round_reservation):
        if type(row) is not dict or set(row) != set(RUNTIME_HARD):
            raise ValueError('fixed40 complete runtime resource vector')
        for name, maximum in RUNTIME_HARD.items():
            core.integer(row[name], maximum, 'fixed40 runtime '+name)
    if (type(inherited_events) is not tuple or len(inherited_events) != 3
            or inherited_events[0] < 88 or inherited_events[1:] != (0,0)):
        raise ValueError('fixed40 complete inherited event vector')
    if (round_reservation['invocations'] != 16
            or not round_reservation['seconds']
            or any(not round_reservation[k] for k in RUNTIME_HARD)):
        raise ValueError('fixed40 full sixteen-route reservation')
    for name, floor in LATER_MINIMUM.items():
        if protected[name] < floor:
            raise ValueError('fixed40 preserve full later37/32 reservation')
    core.integer(control_hard,CoherentDevelopmentReservations.HARD[6],
                 'fixed40 finite development control ceiling')
    core.integer(control_used,control_hard,'fixed40 preserved development control')
    limits={name:RUNTIME_HARD[name]-protected[name] for name in RUNTIME_HARD}
    outcomes=(control_hard-control_used)//20483
    if outcomes < 1:
        raise ValueError('fixed40 no reserved failure/control outcome')
    # Correction reads are distinct from production/round reads. Keep the
    # complete finite correction packet unavailable to route capacity.
    route_limits=dict(limits)
    route_limits['nonbinary']-=outcomes*core.COUNTER_IO_RESERVE
    # One full current production round remains held even when development fails.
    rounds=min((route_limits[k]-consumed[k]-round_reservation[k]
                -(core.COUNTER_IO_RESERVE if k=='nonbinary' else 0))//round_reservation[k]
               for k in RUNTIME_HARD)
    if rounds < 1:
        raise ValueError('fixed40 no complete development/recovery/production capacity')
    # Full32768-byte correction IO and20483-byte development controls are
    # reserved for every remaining event. Numeric parser bound is derived.
    core.integer(inherited_events[0],37+67108864//256,'fixed40 finite history parser')
    return dict(limits=route_limits, counter_limits=limits, development_rounds=rounds,
                migration_read_reservation=core.COUNTER_IO_RESERVE,
                event_limits=(inherited_events[0]+outcomes,4,4),
                full_event_control_reservation=outcomes*20483,
                full_event_read_reservation=outcomes*core.COUNTER_IO_RESERVE)


class RuntimeAdmission:
    """Authenticate the concrete fixed40 development-to-runtime handoff.

    Construction is read-only. migrate() can replace only the same schema4
    development root, under the same lock, after exact independent frozen-source
    and complete-preflight acceptance. Preparation can compile a cloned fit with
    the authenticated profile but cannot migrate without that separate review.
    All full source/history/configuration reads are retained in read_trace.
    No profile or receipt substitutes for a required runtime body.
    """
    PROFILE_FIELDS = {'schema','development','authority','history','sources',
                      'events','consumed','protected','round_reservation',
                      'control_used','source_identity','runtime_setup'}
    def __init__(self, profile_pin, *, review_pin=None, read_charge=None, live_bytes=0,
                 _guard_origin=None):
        self.accepted=False
        if read_charge is not None and not callable(read_charge):
            raise ValueError('fixed40 full admission read debit callback')
        core.integer(live_bytes,core.SEMANTIC,'fixed40 outer admission caller')
        # Cold guard reconstruction is the sole physical alias for the original
        # protected body. Capture it in the loader, not a mutable public field.
        origin=None
        if _guard_origin is not None:
            expected=dict(FIT_PROTECTED_COUNTER,
                path=str(Path(FIT_PROTECTED_COUNTER['path']).parent/'guard-origin.json'))
            if (type(_guard_origin) is not tuple
                    or _guard_origin!=fixed._pin_tuple(expected)):
                raise ValueError('fixed40 exact retained guard origin')
            origin=(fixed._pin_tuple(FIT_PROTECTED_COUNTER),_guard_origin)
            # A bound self method here would form a reference cycle, retaining
            # the complete nested admission after its explicit last use.
            owner=weakref.ref(self);base_load=RuntimeAdmission._load
            def origin_load(pin,maximum,**options):
                current=owner()
                if current is None:raise ValueError('fixed40 released origin reader')
                return base_load(current,pin,maximum,_origin=origin,**options)
            self._load=origin_load
            def nested_admission(pin,*,read_charge,live_bytes):
                return RuntimeAdmission(pin,read_charge=read_charge,live_bytes=live_bytes,
                                        _guard_origin=origin[1])
            self._nested_admission=nested_admission
        self._outer_live=live_bytes+(0 if origin is None else fixed.graph_charge(origin)+2048)
        if live_bytes+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 initial nested admission lifetime')
        self._full_read_charge=read_charge
        self._fit_carry_wire=None
        self.read_trace=[]
        self._graphs=[]
        self.profile_pin=dict(profile_pin)
        profile=self._load(profile_pin,32768,decoded=True)
        if type(profile) is not dict:
            raise ValueError('fixed40 exact bridge profile')
        coherent=profile.get('schema')=='fixed40-development-runtime-profile-v2'
        fields=self.PROFILE_FIELDS | ({'development_successor','runtime_prefix'} if coherent else set())
        if (set(profile)!=fields or profile['schema'] not in (
                'fixed40-development-runtime-profile-v1',
                'fixed40-development-runtime-profile-v2')):
            raise ValueError('fixed40 exact bridge profile')
        self._graphs.append(profile)
        controller=CoherentDevelopmentReservations if coherent else RetainedDevelopmentReservations
        if coherent:
            if profile['development_successor']!=controller.PLAN_PIN:
                raise ValueError('fixed40 exact development successor identity')
            plan=self._load(controller.PLAN_PIN,131072,decoded=True)
            controller.check_plan(plan)
            del plan
            for review in controller.REVIEW_PINS:
                self._load(review,32768)
        core.integer(profile['runtime_setup'],16777216,'fixed40 unchanged runtime setup')
        if profile['runtime_setup']<16498255:
            raise ValueError('fixed40 preserve full runtime setup bank')
        sources=profile['sources']
        if (type(sources) is not list or not sources or len(sources)>104
                or len({x['path'] for x in sources})!=len(sources)
                or core.digest(sources)!=profile['source_identity']):
            raise ValueError('fixed40 complete bridge source identity')
        # Cumulative source history never changes the current4MiB closure or
        # individual1MiB source cap. Reject before opening any source body.
        for source in sources:
            fixed._pin_tuple(source)
            core.integer(source['bytes'],records.BODY_CAP,'fixed40 individual current source')
        if sum(source['bytes'] for source in sources)>core.SOURCE:
            raise ValueError('fixed40 current source aggregate')
        # Importing a module supplies no frozen-source authority. Authenticate
        # each complete current source before using this bridge for compilation.
        for source in sources:
            self._load(source,records.BODY_CAP)
        original_sources=self._load(dict(
            path='/home/fry/artifacts/vrf40-admission-01a14b66-20261018/recovery46-rust-source-freeze.json',
            bytes=17908,sha256='be283dccbab3448bb727749b83615ac3326cbfe4ae0cb44772d5a43d6baf48ae'),32768,decoded=True)
        self._graphs.append(original_sources)
        original_tools=self._load(dict(
            path='/home/fry/artifacts/vrf40-admission-01a14b66-20261018/recovery46-tool-source-freeze.json',
            bytes=1214,sha256='9c799141125c01470aeb025a6d84d3360d9e63878054b707e43012b177b57990'),32768,decoded=True)
        self._graphs.append(original_tools)
        new_tools=('pa1_development.py','test_pa1_development.py',
                   'pa1_fixed40_runtime.py','test_pa1_fixed40_runtime.py',
                   'pa1_fixed40_constructors.py')
        expected_paths=tuple(x['path'] for x in original_sources+original_tools)
        expected_paths+=tuple(str(Path(fixed.__file__).parent/name) for name in new_tools)
        # The original roster fixes paths and occurrence order, not the bytes
        # of a new development version. Every current body was authenticated
        # above. Actual gate reuse still requires the exact historical bytes;
        # preparation alone grants no final source or runtime acceptance.
        if (len(original_sources)!=90 or len(original_tools)!=6
                or tuple(x['path'] for x in sources)!=expected_paths):
            raise ValueError('fixed40 exact ordered101 executable source closure')
        self._graphs.pop();self._graphs.pop()
        del original_sources,original_tools,expected_paths
        authority=self._load(profile['authority'],16384,decoded=True)
        self._graphs.append(authority)
        history=self._load(profile['history'],16384,decoded=True)
        self._graphs.append(history)
        if (authority.get('development_only') is not True
                or authority.get('history')!=profile['history']
                or history.get('schema') not in ('fixed40-exact-development-history-v1',
                                                'fixed40-exact-development-history-v2')
                or history.get('breach_event')!=36
                or history.get('effective')!=profile['events']):
            raise ValueError('fixed40 bridge full development authority/history')
        page_pins=controller.history_page_pins(history)
        if page_pins and not coherent:
            raise ValueError('fixed40 paged history requires coherent admission')
        self._history_constructor_pages=page_pins
        event_hashes=[]
        self._graphs.append(event_hashes)
        for rows in controller.history_pages(history,self._load):
            self._graphs.append(rows)
            for row in rows:
                count=row['last']-row['first']+1
                caller=fixed.graph_charge(self._graphs)+fixed.graph_charge(self.read_trace)
                # Include list extension temporary, spare capacity and eventual
                # tuple conversion BEFORE growing the accumulator.
                if caller+128*count+16*(len(event_hashes)+count)+2*core.CHUNK+262144>core.SEMANTIC:
                    raise ValueError('fixed40 history hash accumulator lifetime')
                self._load(row['pin'],327680)
                event_hashes.extend([row['pin']['sha256']]*count)
            self._graphs.pop()
            del rows
        self.event_record_hashes=tuple(event_hashes)
        self._graphs.pop()
        del event_hashes
        self._graphs.append(self.event_record_hashes)
        development=self._load(profile['development'],4096,decoded=True)
        self._graphs.append(development)
        self._check_development(profile,authority,development,controller=controller)
        legacy=records.decode(development['legacy'].encode(),4096)
        consumed=profile['consumed']
        runtime_floor={k:legacy[k] for k in RUNTIME_HARD}
        fit_prefix=False
        if coherent:
            prefix=self._load(profile['runtime_prefix'],16384,decoded=True)
            self._graphs.append(prefix)
            fit_prefix=prefix.get('schema')=='fixed40-runtime-consumed-prefix-v2' if type(prefix) is dict else False
            prefix_fields={'schema','counter','consumed'}|({'fit'} if fit_prefix else set())
            if (type(prefix) is not dict or set(prefix)!=prefix_fields
                    or prefix['schema'] not in ('fixed40-runtime-consumed-prefix-v1',
                                                'fixed40-runtime-consumed-prefix-v2')
                    or type(prefix['consumed']) is not dict
                    or set(prefix['consumed'])!=set(RUNTIME_HARD)):
                raise ValueError('fixed40 exact runtime consumed prefix')
            # The complete original counter body is still required. A pinned
            # prefix adds post-legacy debits and cannot replace that body.
            prior=self._load(prefix['counter'],4096,decoded=True)
            if (core.canonical(prior)!=development['legacy'].encode()
                    or prefix['counter']['sha256']!=fixed.RecoveryCounters.LEGACY_SHA):
                raise ValueError('fixed40 exact runtime prefix legacy counter')
            for name,maximum in RUNTIME_HARD.items():
                core.integer(prefix['consumed'][name],maximum,'fixed40 runtime prefix bound')
                if prefix['consumed'][name]<legacy[name]:
                    raise ValueError('fixed40 runtime prefix legacy rollback')
            if fit_prefix:
                self._fit_carry_wire=self._consume_fit_prefix(prefix,profile,legacy)
            runtime_floor=dict(prefix['consumed'])
            self._graphs.pop()
            del prefix,prior
        for name,index in RESOURCE_INDEX.items():
            engineering=0 if coherent else development['used'][index]
            if consumed[name] < runtime_floor[name]+engineering:
                raise ValueError('fixed40 bridge consumed resource rollback')
        if consumed['invocations']!=runtime_floor['invocations']:
            raise ValueError('fixed40 development is not a protocol invocation')
        if (profile['control_used'] < development['used'][6]
                or profile['events']!=development['events']):
            raise ValueError('fixed40 bridge control/event rollback')
        envelope=derive_runtime_envelope(consumed,profile['protected'],
            profile['round_reservation'],profile['control_used'],tuple(profile['events']),
            control_hard=controller.HARD[6] if coherent else 344148)
        migration_profile={k:profile[k] for k in
            ('development','authority','history','events','consumed')}
        migration_profile['controller']='coherent-v3' if coherent else 'retained-v1'
        self._profile_wire=core.canonical(migration_profile)
        del migration_profile
        self._events=tuple(profile['events'])
        self.envelope=envelope
        self.accepted=False
        if review_pin is not None:
            review=self._load(review_pin,32768,decoded=True)
            required_review={'schema','profile','source_identity','preflight',
                             'frozen_source_accepted','complete_preflight_accepted'}
            if (type(review) is not dict or set(review)!=required_review
                    or review['schema']!='fixed40-runtime-bridge-acceptance-v1'
                    or review['profile']!=profile_pin
                    or review['source_identity']!=profile['source_identity']
                    or review['frozen_source_accepted'] is not True
                    or review['complete_preflight_accepted'] is not True):
                raise ValueError('fixed40 independent frozen/preflight acceptance')
            # Complete accepted preflight remains a full authenticated body.
            self._load(review['preflight'],2097152)
            self.accepted=True
            self.review_sha=review_pin['sha256']
        # Include accepted-review/preflight reads as well as the later migration
        # constructor's authority/history reads. No receipt or review is free.
        extra_reads=sum(x['bytes']+1 for x in self.read_trace)
        extra_reads+=profile['authority']['bytes']+profile['history']['bytes']+2
        extra_reads+=sum(pin['bytes']+1 for pin in page_pins)
        if coherent:
            extra_reads+=controller.PLAN_PIN['bytes']+1
            extra_reads+=sum(x['bytes']+1 for x in controller.REVIEW_PINS)
        engineering_reads=0 if coherent else development['used'][2]
        if consumed['nonbinary'] < runtime_floor['nonbinary']+engineering_reads+extra_reads:
            raise ValueError('fixed40 bridge missing full constructor/read debit')
        if fit_prefix and consumed['units']<runtime_floor['units']+len(self.read_trace):
            raise ValueError('fixed40 fit transfer missing full constructor units')
        if coherent:
            expected=[(dict(controller.PLAN_PIN),*(dict(p) for p in controller.REVIEW_PINS))]
            if page_pins:expected.append(tuple(dict(pin) for pin in page_pins))
            charged=0
            def prepaid_charge(packet):
                nonlocal charged
                # One complete ordered packet sequence for this admission.
                # Recreating a class or retrying a failed constructor cannot
                # mint another plan or page pass.
                if charged>=len(expected) or packet!=expected[charged]:
                    raise ValueError('fixed40 exact prepaid constructor reads')
                charged+=1
            self._migration_read_charge=prepaid_charge
        self.counter_type=self._counter_type()
        self.caller_charge=fixed.graph_charge((self._profile_wire,self._events,
            self.read_trace,self.envelope,self.event_record_hashes,self.profile_pin))
        self.caller_charge+=self._counter_metadata_charge+32768+self._outer_live
        self.caller_charge+=fixed.graph_charge(self._history_constructor_pages)
        self.caller_charge+=fixed.graph_charge(self._fit_carry_wire)
        if self.caller_charge+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 bridge caller lifetime')
        # Release actual authority/history/state graphs after deriving immutable
        # source-bound fields; runtime counters reauthenticate on construction.
        del self._graphs[:]
        self.authority_pin=profile['authority']
        self._outer_live=0

    def seal_read_trace(self,path,*,publication_charge,reuse_pin=None,live_bytes=0):
        """Publish and reauthenticate completed reads before releasing their graph.

        This transfers storage only. Every previous row remains in a complete
        immutable body. The archive authentication is a new charged full read,
        retained in the new live trace. Later whole-preflight review must bind
        every returned archive pin; none is a receipt for a future body read.
        The admission object and its counter closure remain live and charged.
        """
        if not callable(publication_charge):
            raise ValueError('fixed40 trace publication debit required')
        core.integer(live_bytes,core.SEMANTIC,'fixed40 trace caller lifetime')
        full_read_charge=getattr(self,'_full_read_charge',None)
        if not callable(full_read_charge):
            raise ValueError('fixed40 trace full archive read debit required before publication')
        destination=records.canonical_path(path)
        encoder=json.JSONEncoder(sort_keys=True,separators=(',',':'),ensure_ascii=True,allow_nan=False)
        width=1+sum(len(piece) for piece in encoder.iterencode(self.read_trace))
        if (width>1048576 or self.caller_charge+live_bytes+3*width
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 complete trace publication lifetime')
        raw=core.canonical(self.read_trace)
        if len(raw)!=width:raise ValueError('fixed40 exact complete trace width')
        pin=dict(path=str(destination),bytes=width,sha256=hashlib.sha256(raw).hexdigest())
        if reuse_pin is not None:
            fixed._pin_tuple(reuse_pin)
            if reuse_pin!=pin:
                raise ValueError('fixed40 existing trace differs from exact fresh read sequence')
            # Existing bytes retain their original storage charge. They must
            # still pass the new full charged SHA/EOF read below; no receipt
            # substitutes for any current source or authority-body read.
        else:
            publication_charge((dict(pin),))
            fd=os.open(destination,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
            try:
                remaining=memoryview(raw)
                while remaining:
                    count=os.write(fd,remaining)
                    if count<=0:raise ValueError('fixed40 complete trace publication')
                    remaining=remaining[count:]
                os.fsync(fd)
            finally:os.close(fd)
            del remaining
        # The original full hash/EOF reader debits this verification before open.
        self._load(pin,1048576,live_bytes=live_bytes+fixed.graph_charge(raw),read_charge=full_read_charge)
        verified=self.read_trace[-1]
        archives=(*getattr(self,'_sealed_read_trace_pins',()),tuple(fixed._pin_tuple(pin)))
        self._sealed_read_trace_pins=archives
        self.read_trace=[verified]
        del raw
        self.caller_charge=fixed.graph_charge((self._profile_wire,self._events,
            self.read_trace,self.envelope,self.event_record_hashes,self.profile_pin,archives))
        self.caller_charge+=self._counter_metadata_charge+32768+self._outer_live
        self.caller_charge+=fixed.graph_charge(self._history_constructor_pages)
        self.caller_charge+=fixed.graph_charge(self._fit_carry_wire)
        if self.caller_charge+live_bytes+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 sealed trace caller lifetime')
        return pin

    def _load(self,pin,maximum,*,decoded=False,live_bytes=0,read_charge=None,
              completion_receipt=False,_origin=None):
        fixed._pin_tuple(pin)
        if _origin is not None:
            expected=dict(FIT_PROTECTED_COUNTER,
                path=str(Path(FIT_PROTECTED_COUNTER['path']).parent/'guard-origin.json'))
            if _origin!=(fixed._pin_tuple(FIT_PROTECTED_COUNTER),fixed._pin_tuple(expected)):
                raise ValueError('fixed40 exact retained guard origin')
        core.integer(pin['bytes'],maximum,'fixed40 bridge full body bound')
        if (type(completion_receipt) is not bool or (completion_receipt
                and (decoded is not True or maximum!=core.RECEIPT))):
            raise ValueError('fixed40 exact core completion decoding role')
        if (decoded and maximum>32768 and pin!=CoherentDevelopmentReservations.PLAN_PIN
                and not completion_receipt):
            raise ValueError('fixed40 bridge decoded body cap')
        core.integer(live_bytes,core.SEMANTIC,'fixed40 nested authenticated caller')
        initial_caller=live_bytes+getattr(self,'_outer_live',0)+sum(fixed.graph_charge(x) for x in self._graphs)
        initial_caller+=fixed.graph_charge((self.read_trace,pin))
        if initial_caller+(2*pin['bytes'] if decoded else 0)+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 bridge pre-open caller lifetime')
        physical=pin
        if _origin is not None and fixed._pin_tuple(pin)==_origin[0]:
            physical=dict(zip(('path','bytes','sha256'),_origin[1]))
        charge=getattr(self,'_full_read_charge',None) if read_charge is None else read_charge
        if charge is not None:
            if not callable(charge):raise ValueError('fixed40 full body read debit')
            charge((dict(physical),))
        fd=os.open(records.canonical_path(physical['path']),
                   os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
        raw=bytearray() if decoded else None
        size=0;checksum=hashlib.sha256()
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError('fixed40 bridge regular input')
            while size<pin['bytes']:
                chunk=os.read(fd,min(core.CHUNK,pin['bytes']-size))
                if not chunk:raise ValueError('fixed40 bridge truncated input')
                size+=len(chunk);checksum.update(chunk)
                if decoded:raw.extend(chunk)
            if os.read(fd,1) or checksum.hexdigest()!=pin['sha256']:
                raise ValueError('fixed40 bridge full hash/EOF')
        finally:os.close(fd)
        trace=dict(pin=tuple(fixed._pin_tuple(pin)),bytes=size,eof=True,decoded=decoded)
        if physical is not pin:
            trace['physical_pin']=fixed._pin_tuple(physical)
        self.read_trace.append(trace)
        if not decoded:return size
        core.integer(live_bytes,core.SEMANTIC,'fixed40 nested authenticated caller')
        caller=live_bytes+getattr(self,'_outer_live',0)+sum(fixed.graph_charge(x) for x in self._graphs)
        caller+=fixed.graph_charge(self.read_trace)
        peak=caller+2*len(raw)+128*core._wire_nodes(raw)+2*core.CHUNK+262144
        if peak>core.SEMANTIC:
            raise ValueError('fixed40 bridge predecode caller lifetime')
        return records.decode(raw,maximum)

    def _consume_fit_prefix(self,prefix,profile,legacy):
        """Authenticate complete fit closeout, full bodies and reviewed debits.

        The result is immutable migration input. It is not final source/READY
        acceptance. The original snapshot remains mandatory, every direct and
        repeated read is precharged, and the existing full counter reader
        validates the unchanged live clone. No partial counter validator is used.
        """
        if not callable(self._full_read_charge):
            raise ValueError('fixed40 fit transfer requires full admitted reads')
        fit=prefix['fit']
        required={'initial_profile','preflight_review','receipt','matrix','debit_ledger','review'}
        if type(fit) is not dict or set(fit)!=required:
            raise ValueError('fixed40 complete fit transfer controls')
        base_graphs=len(self._graphs)
        def load(pin,maximum,decoded=True):
            value=self._load(pin,maximum,decoded=decoded)
            if decoded:self._graphs.append(value)
            return value
        initial=load(fit['initial_profile'],32768)
        if (initial.get('schema')!='fixed40-development-runtime-profile-v2'
                or initial.get('source_identity')!=profile['source_identity']
                or initial.get('sources')!=profile['sources']
                or initial.get('development_successor')!=profile['development_successor']):
            raise ValueError('fixed40 fit transfer exact frozen source')
        initial_prefix=load(initial['runtime_prefix'],16384)
        if initial_prefix.get('schema')!='fixed40-runtime-consumed-prefix-v1':
            raise ValueError('fixed40 fit transfer cannot recursively import itself')
        receipt=load(fit['receipt'],4096)
        receipt_fields={'schema','mode','profile_sha256','preflight_sha256','counter',
            'live_counter','round','complete_routes','consumed','failures','generation',
            'protected_counter','prepublication_io','receipt_path','receipt_maximum',
            'complete_io_reservation'}
        if (type(receipt) is not dict or set(receipt)!=receipt_fields
                or receipt['schema']!='fixed40-fit-debit-closeout-v1'
                or receipt['mode']!='offline_fit_only_no_runtime_acceptance'
                or receipt['profile_sha256']!=fit['initial_profile']['sha256']
                or receipt['preflight_sha256']!=fit['preflight_review']['sha256']
                or receipt['protected_counter']!=FIT_PROTECTED_COUNTER
                or receipt['complete_routes']!=dict(core.ROUTES)
                or receipt['receipt_maximum']!=4096
                or receipt['complete_io_reservation']!=core.COUNTER_IO_RESERVE):
            raise ValueError('fixed40 exact full fit closeout receipt')
        generation=core.integer(receipt['generation'],200000,'fixed40 fit carry generation')
        if (fit['receipt']['path']!=str(OFFLINE_FIT_ROOT/('fit-debit-closeout-g'+str(generation)+'.json'))
                or receipt['receipt_path']!=fit['receipt']['path']
                or receipt['counter']['path']!=str(OFFLINE_FIT_ROOT/('fit-counter-g'+str(generation)+'.json'))
                or receipt['live_counter']!=str(OFFLINE_FIT_ROOT/'state.json')):
            raise ValueError('fixed40 exact retained closeout version paths')
        snapshot=load(receipt['counter'],4096)
        live_pin=dict(receipt['counter'],path=receipt['live_counter'])
        matrix=load(fit['matrix'],32768)
        review=load(fit['review'],32768)
        bound={'schema','source_identity','initial_profile','preflight_review',
            'receipt','matrix','debit_ledger','consumed','all16_actual_inputs_accepted',
            'all81_constructors_83_prior_accepted','full_binary_schedule_accepted',
            'all_initial_repeated_failed_closeout_debits_accepted'}
        if (type(review) is not dict or set(review)!=bound
                or review['schema']!='fixed40-fit-debit-transfer-review-v1'
                or review['source_identity']!=profile['source_identity']
                or any(review[k]!=fit[k] for k in required-{'review'})
                or review['consumed']!=prefix['consumed']
                or any(review[k] is not True for k in (
                    'all16_actual_inputs_accepted','all81_constructors_83_prior_accepted',
                    'full_binary_schedule_accepted',
                    'all_initial_repeated_failed_closeout_debits_accepted'))):
            raise ValueError('fixed40 independent full fit/debit review')
        # Full ledger authentication remains required; review binds these same
        # complete bytes, including old/failed/closeout operations, not a cache.
        load(fit['debit_ledger'],2097152,False)
        preflight_review=load(fit['preflight_review'],32768)
        matrix_fields={'schema','source_identity','preflight','round','results'}
        routes=tuple((name,index) for name,count in core.ROUTES.items()
                     for index in range(count))
        if (type(matrix) is not dict or set(matrix)!=matrix_fields
                or matrix['schema']!='fixed40-actual-input-offline-fit-matrix-v1'
                or matrix['source_identity']!=profile['source_identity']
                or matrix['preflight']!=preflight_review.get('preflight')
                or matrix['round']!=receipt['round']
                or type(matrix['results']) is not list or len(matrix['results'])!=16):
            raise ValueError('fixed40 exact complete all16 result matrix')
        roster=[]
        self._graphs.append(roster)
        for row,(route,index) in zip(matrix['results'],routes):
            if (type(row) is not dict or set(row)!={'route','index','context','receipt','result'}
                    or (row['route'],row['index'])!=(route,index)
                    or row['context'].get('route')!=route
                    or row['context'].get('phase','').split(':',3)[:3]!=[
                        'development',str(receipt['round']),'0']):
                raise ValueError('fixed40 ordered actual route/source contexts')
            audit=self._load(row['receipt'],core.RECEIPT,decoded=True,completion_receipt=True)
            self._graphs.append(audit)
            if (set(audit)!={'schema','binding','context','generation','unit_verdicts','accepted','authority'}
                    or audit['schema']!='pa1-completion-v1'
                    or audit['binding']!=core.digest(row['context'])
                    or audit['context']!=row['context']
                    or audit['accepted'] is not False or audit['authority'] is not False
                    or type(audit['unit_verdicts']) is not list or not audit['unit_verdicts']
                    or len(set(audit['unit_verdicts']))!=len(audit['unit_verdicts'])):
                raise ValueError('fixed40 complete route audit body')
            core.integer(audit['generation'],generation,'fixed40 route generation')
            core.sha(row['context'].get('manifest'))
            if row['context'].get('source')!=core.digest((profile['sources'][:90],profile['sources'][90:])):
                raise ValueError('fixed40 actual route complete current source')
            if row['result']['bytes']==0:
                raise ValueError('fixed40 missing actual route semantic result')
            self._load(row['result'],131072)
            roster.append(audit['binding'])
            self._graphs.pop()
            del audit
        if len(set(roster))!=16:raise ValueError('fixed40 distinct full route roster')
        # Bound all retained outer graphs before entering either nested reader.
        outer=self._outer_live+fixed.graph_charge((self._graphs,self.read_trace,self.profile_pin))
        if outer+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 fit transfer nested caller lifetime')
        # Nested _load checks the actual wire/node bound before decoding;
        # no serialization of the outer graph is needed to establish it.
        if hasattr(self,'_nested_admission'):
            inner=self._nested_admission(fit['initial_profile'],
                read_charge=self._full_read_charge,live_bytes=outer)
        else:
            inner=RuntimeAdmission(fit['initial_profile'],
                read_charge=self._full_read_charge,live_bytes=outer)
        admitted=OfflineFitAdmission(inner,fit['preflight_review'],read_charge=self._full_read_charge)
        if admitted.caller_charge+128*4097+16*4096+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 complete transfer live counter lifetime')
        context=matrix['results'][0]['context']
        self._full_read_charge((dict(live_pin),))
        counter=admitted.counter_type(OFFLINE_FIT_ROOT,legacy['binding'],legacy['history'],
            namespace='development',round_index=receipt['round'],slot=0,
            context=context,context_roster=tuple(roster))
        self._full_read_charge((dict(live_pin),))
        with counter._locked():
            actual=counter._read()
        if actual!=snapshot or core.canonical(snapshot)!=core.canonical(actual):
            raise ValueError('fixed40 live fit state differs from full snapshot')
        if (snapshot['pending'] is not None
                or snapshot['generation']!=generation
                or snapshot['development'][receipt['round']]!=dict(core.ROUTES)
                or snapshot['failed_development'][receipt['round']]
                or snapshot['namespace_roots'][receipt['round']]!=core.digest(tuple(roster))
                or any(sum(row.values()) for row in snapshot['production'])
                or receipt['consumed']!={k:snapshot[k] for k in RUNTIME_HARD}
                or receipt['failures']!=snapshot['failures']):
            raise ValueError('fixed40 complete preserved fit state/round')
        for k,maximum in RUNTIME_HARD.items():
            core.integer(prefix['consumed'][k],maximum,'fixed40 carried cumulative debit')
            if prefix['consumed'][k]<snapshot[k]:
                raise ValueError('fixed40 final fit debit rollback')
        if prefix['consumed']['invocations']!=snapshot['invocations']:
            raise ValueError('fixed40 fit namespace invocation preservation')
        if any(a<b for a,b in zip(profile['events'],snapshot['failures'])):
            raise ValueError('fixed40 final fit event rollback')
        # Both actual live counter reads must match the authenticated full body.
        for io in counter.io_summary()['versions']:
            if (io['operation']!='read' or io['bytes']!=live_pin['bytes']
                    or io['sha256']!=live_pin['sha256'] or not io['authenticated']):
                raise ValueError('fixed40 complete live counter IO evidence')
        carry=dict(state=snapshot,counter=live_pin,receipt=fit['receipt'],
                   filesystem=[list(row) for row in admitted._root_identity],
                   review=fit['review'],matrix=fit['matrix'],debit_ledger=fit['debit_ledger'])
        wire=core.canonical(carry)
        if len(wire)>16384:raise ValueError('fixed40 complete carry capsule bound')
        addition=fixed.graph_charge((inner.read_trace,counter.io_summary(),wire))
        if outer+addition+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 complete nested read trace transfer lifetime')
        nested=tuple(inner.read_trace)
        del counter,admitted,inner,actual,carry,initial,initial_prefix,receipt,snapshot,matrix,review,preflight_review,roster
        self.read_trace.extend(nested)
        self.read_trace.extend(dict(pin=tuple(fixed._pin_tuple(live_pin)),
            bytes=live_pin['bytes'],eof=True,decoded=True) for _ in range(2))
        del self._graphs[base_graphs:]
        return wire

    @staticmethod
    def _check_development(profile,authority,value,*,controller=RetainedDevelopmentReservations):
        if (type(value) is not dict or set(value)!=DevelopmentReservations.STATE_FIELDS
                or value['schema']!=4 or value['authority']!=profile['authority']['sha256']
                or value['history']!=profile['history']['sha256']
                or value['pending']!=[] or value['closeout']!=65536):
            raise ValueError('fixed40 bridge terminal exact development root')
        if (type(value['legacy']) is not str or len(value['legacy'].encode())!=734
                or hashlib.sha256(value['legacy'].encode()).hexdigest()
                   !=fixed.RecoveryCounters.LEGACY_SHA):
            raise ValueError('fixed40 bridge immutable legacy734')
        if (type(value['caps']) is not list or type(value['used']) is not list
                or len(value['caps'])!=7 or len(value['used'])!=7
                or value['caps']!=authority.get('caps')):
            raise ValueError('fixed40 bridge exact development ledger')
        for i,(used,cap) in enumerate(zip(value['used'],value['caps'])):
            core.integer(cap,controller.HARD[i],'fixed40 development hard cap')
            core.integer(used,cap,'fixed40 preserved development debit')
            if used<max(controller.INHERITED[i],authority['used'][i]):
                raise ValueError('fixed40 development debit rollback')
        DevelopmentReservations._check_capacity(value)
        core.integer(value['generation'],200000,'fixed40 development generation')
        core.integer(value['issued'],16777216//256,'fixed40 development issued')
        core.sha(value['chain'])

    def check_compilation(self,consumed_events,event_history):
        if (consumed_events!=self._events
                or event_history!=fixed.BASELINE_HISTORY):
            raise ValueError('fixed40 exact development compilation binding')
        # No old8/35 threshold; envelope derives all finite remaining outcomes.
        if consumed_events[0]>=self.counter_type.EVENT_LIMITS[0]:
            raise ValueError('fixed40 no full reserved compilation failure outcome')

    def _counter_type(self):
        # Decode an independent private snapshot; no authenticated caller-owned
        # mutable dictionary remains a migration authority.
        caller=getattr(self,'_outer_live',0)+fixed.graph_charge(
            (self._graphs,self.read_trace,self.profile_pin,self.envelope))
        profile=_fit_profile_decode(self._profile_wire,caller)
        controller_name=profile.get('controller','retained-v1')
        if controller_name not in ('coherent-v3','retained-v1'):
            raise ValueError('fixed40 immutable development controller')
        controller_type=(CoherentDevelopmentReservations if controller_name=='coherent-v3'
                         else RetainedDevelopmentReservations)
        # Retain only the fields migration actually needs. The source roster
        # remains in its full immutable profile and is reread by each admission.
        profile={k:profile[k] for k in ('development','authority','history','events','consumed')}
        self._counter_metadata_charge=fixed.graph_charge(profile)
        if controller_type is CoherentDevelopmentReservations:
            self._counter_metadata_charge+=fixed.graph_charge(
                (controller_type.PLAN_PIN,controller_type.REVIEW_PINS))+32768
        envelope=self.envelope
        plan_sha=self.profile_pin['sha256']
        legacy_pin=profile['development']
        floors=dict(profile['consumed'])
        accepted=self.accepted
        review_sha=getattr(self,'review_sha',None)
        expected_event_records=tuple(self.event_record_hashes)
        read_charge=getattr(self,'_migration_read_charge',None)
        full_read_charge=getattr(self,'_full_read_charge',None)
        carry_wire=getattr(self,'_fit_carry_wire',None)
        carry=None
        if carry_wire is not None:
            peak=getattr(self,'_outer_live',0)+fixed.graph_charge((self._graphs,self.read_trace,profile))
            if peak+2*len(carry_wire)+128*core._wire_nodes(carry_wire)+2*core.CHUNK+262144>core.SEMANTIC:
                raise ValueError('fixed40 immutable carry capsule lifetime')
            carry=records.decode(carry_wire,16384)
            self._counter_metadata_charge+=fixed.graph_charge(carry)
        # Freeze the full retained charge; public fields cannot reduce it.
        migration_live=fixed.graph_charge((self._profile_wire,getattr(self,'_events',()),
            self.read_trace,self.envelope,self.event_record_hashes,self.profile_pin))
        migration_live+=self._counter_metadata_charge+32768+getattr(self,'_outer_live',0)
        migration_live+=fixed.graph_charge(getattr(self,'_history_constructor_pages',()))
        migration_live+=fixed.graph_charge(carry_wire)
        class BoundRuntimeCounters(fixed.RecoveryCounters):
            PLAN_SHA=plan_sha
            LIMITS=dict(envelope['counter_limits'])
            EVENT_LIMITS=tuple(envelope['event_limits'])
            DEVELOPMENT_ROUNDS=(len(carry['state']['development']) if carry is not None else envelope['development_rounds'])

            def __init__(self,*args,**kwargs):
                if carry is not None and kwargs.get('namespace')!='production':
                    raise ValueError('fixed40 accepted fit cannot restart development')
                if kwargs.get('namespace')=='production' and kwargs.get('slot')!=0:
                    raise ValueError('fixed40 this round cannot execute37/32')
                super().__init__(*args,**kwargs)

            def _read(self):
                value=super()._read()
                if self.trace_io:self._io_trace[-1]['authenticated']=False
                for key,minimum in floors.items():
                    if value[key]<minimum:
                        raise ValueError('fixed40 inherited complete runtime debit')
                if value['recovery']['inherited']!=profile['events']:
                    raise ValueError('fixed40 inherited exact development events')
                if accepted and (value['recovery']['preflight']!=review_sha
                        or value['recovery']['plan']!=plan_sha):
                    raise ValueError('fixed40 immutable accepted runtime review anchor')
                if accepted and carry is not None:
                    for key in ('development','failed_development','round_failure_events'):
                        if value[key]!=carry['state'][key]:
                            raise ValueError('fixed40 preserved accepted fit vectors')
                    count=len(carry['state']['development'])
                    if value['namespace_roots'][:count]!=carry['state']['namespace_roots'][:count]:
                        raise ValueError('fixed40 preserved accepted fit roots')
                self._check_outcome_hold(value)
                if self.trace_io:self._io_trace[-1]['authenticated']=True
                return value

            def _check_outcome_hold(self,value):
                held=(self.EVENT_LIMITS[0]-value['failures'][0])*core.COUNTER_IO_RESERVE
                if value['nonbinary']+held>self.LIMITS['nonbinary']:
                    raise ValueError('fixed40 reserved correction reads unavailable')

            def _write(self,value):
                # Correction admission both consumes32768 and decreases the
                # remaining hold. Route/unscoped charges cannot spend that hold.
                self._check_outcome_hold(value)
                return super()._write(value)

            @classmethod
            def migrate(cls,directory,*,consumed_events,event_records,accepted_preflight):
                if (not accepted or accepted_preflight!=review_sha
                        or tuple(consumed_events)!=tuple(profile['events'])):
                    raise ValueError('fixed40 bridge requires exact independent acceptance')
                if (type(event_records) is not tuple
                        or event_records!=expected_event_records):
                    raise ValueError('fixed40 bridge complete event records')
                for record in event_records:core.sha(record)
                guard=_require_offline_fit_carry(carry_wire,read_charge=full_read_charge,
                    live_bytes=migration_live)
                with guard as guard_live:
                    retained=migration_live+(guard_live or 0)
                    config_graph=[]
                    class MigrationController(controller_type):
                        # Keep the original full validator and read debit, with
                        # the complete outer migration graph on every parse.
                        @staticmethod
                        def authenticate(pin,maximum,*,decoded=False,live_bytes=0):
                            value=controller_type.authenticate(pin,maximum,decoded=decoded,
                                live_bytes=retained+fixed.graph_charge(config_graph)+live_bytes)
                            if decoded and (pin==profile['authority']
                                    or (controller_type is CoherentDevelopmentReservations
                                        and pin==controller_type.PLAN_PIN)):
                                # authenticate_successor retains the complete
                                # plan while it authenticates every review.
                                # Authority replaces that finished plan pass.
                                config_graph[:]=[value]
                            return value
                        def _read_state_raw(self):
                            raw=super()._read_state_raw()
                            if (retained+fixed.graph_charge(config_graph)+2*len(raw)
                                    +128*core._wire_nodes(raw)+2*core.CHUNK+262144>core.SEMANTIC):
                                raise ValueError('fixed40 migration full state caller lifetime')
                            return raw
                    # Reauthenticate complete pinned development authority and history;
                    # then hold the SAME lock across full state read and replacement.
                    options={}
                    if controller_type is CoherentDevelopmentReservations:
                        options['read_charge']=read_charge
                    controller=MigrationController(directory,
                        profile['authority']['sha256'],authority_pin=profile['authority'],**options)
                    with controller._locked():
                        current=controller._read()
                        actual=controller._io_trace[-1]
                        if (actual['bytes']!=legacy_pin['bytes']
                                or actual['sha256']!=legacy_pin['sha256']
                                or current['pending']!=[]):
                            raise ValueError('fixed40 changed/pending development root')
                        RuntimeAdmission._check_development(profile,controller.config,current,controller=controller_type)
                        old=records.decode(current['legacy'].encode(),4096)
                        value=dict(old)
                        value.update(schema=3,limits=cls.limit_identity(),
                            legacy_raw=current['legacy'],
                            recovery=dict(plan=plan_sha,preflight=accepted_preflight,
                                events=core.digest((profile['history'],event_records,current['chain'])),
                                inherited=list(consumed_events)),
                            development=cls._vectors(cls.DEVELOPMENT_ROUNDS),
                            production=cls._vectors(3),
                            namespace_roots=[None]*(cls.DEVELOPMENT_ROUNDS+3),
                            failed_development=[False]*cls.DEVELOPMENT_ROUNDS,
                            round_failure_events=[None]*cls.DEVELOPMENT_ROUNDS,
                            failures=list(consumed_events),generation=core.integer(max(current['generation'],carry['state']['generation'] if carry is not None else 0)+1,200000,'fixed40 migration generation'))
                        if carry is not None:
                            for key in ('development','production','namespace_roots',
                                        'failed_development','round_failure_events'):
                                value[key]=carry['state'][key]
                            value['recovery']['events']=core.digest((
                                value['recovery']['events'],carry_wire))
                        value.update(floors)
                        value['nonbinary']+=core.COUNTER_IO_RESERVE
                        for name,maximum in cls.LIMITS.items():
                            core.integer(value[name],maximum,'fixed40 migration preserved debit')
                        # Exact4096 write cap and IO guard run BEFORE atomic replacement.
                        if (retained+fixed.graph_charge((config_graph,current,old,value))
                                +3*4096+2*core.CHUNK+262144>core.SEMANTIC):
                            raise ValueError('fixed40 migration replacement caller lifetime')
                        controller._write(value)
                    return controller.io_summary()
        return BoundRuntimeCounters


# This is the sole fit-clone location. It never aliases the original root.
OFFLINE_FIT_ROOT = Path('/home/fry/artifacts/vrf40-development-20261018/fixed40-actual-offline-fit-v1')
FIT_PROTECTED_COUNTER = dict(path=str(fixed.ROOT/'offline-fit-counters/state.json'),
    bytes=734,sha256=fixed.RecoveryCounters.LEGACY_SHA)


def _require_offline_fit_carry(carry_wire=None,*,read_charge=None,live_bytes=0):
    """Keep real migration blocked until the complete consumed fit is frozen.

    The returned context holds the reviewed clone directory/lock descriptors
    through the real-root write. It reauthenticates full live bytes after all
    reviews, so stale closeout or later clone consumption cannot be transferred.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 migration retained caller')
    if live_bytes+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 migration initial carry lifetime')
    exists=OFFLINE_FIT_ROOT.exists() or OFFLINE_FIT_ROOT.is_symlink()
    if not exists:
        if carry_wire is not None:raise ValueError('fixed40 missing consumed fit clone')
        return nullcontext()
    if carry_wire is None or not callable(read_charge):
        raise ValueError('fixed40 authenticated exactly-once offline fit debit carry required')
    if (type(carry_wire) is not bytes or len(carry_wire)>16384
            or live_bytes+2*len(carry_wire)+128*core._wire_nodes(carry_wire)
                +2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 migration carry predecode lifetime')
    carry=records.decode(carry_wire,16384)
    expected=tuple(tuple(row) for row in carry['filesystem'])
    pin=dict(carry['counter'])
    if pin['path']!=str(OFFLINE_FIT_ROOT/'state.json'):
        raise ValueError('fixed40 exact final fit root')
    class Guard(_OfflineFitStateIO):
        _expected_fit_identity=expected
    guard=Guard();guard.directory=OFFLINE_FIT_ROOT
    # Release the second full carry graph before file access or nested work.
    del carry
    guard_live=fixed.graph_charge((pin,expected))+32768
    if live_bytes+guard_live+4097+2*core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 migration held guard lifetime')
    @contextmanager
    def locked():
        read_charge((dict(pin),))
        with guard._locked():
            raw=guard._read_state_raw()
            if len(raw)!=pin['bytes'] or hashlib.sha256(raw).hexdigest()!=pin['sha256']:
                raise ValueError('fixed40 consumed fit changed before migration')
            # Keep the authenticated lock, not a second raw clone body.
            del raw
            yield guard_live
    return locked()


def _offline_fit_directory(directory):
    """Reject every alternate root and symlink before any clone operation."""
    path=Path(directory)
    if str(path)!=str(OFFLINE_FIT_ROOT):
        raise ValueError('fixed40 exact offline fit clone directory')
    for parent in (path,*path.parents):
        if parent.is_symlink():
            raise ValueError('fixed40 offline fit directory symlink')
    if (not path.is_dir() or path.resolve()!=path
            or path.resolve()==Path(FIT_PROTECTED_COUNTER['path']).parent.resolve()):
        raise ValueError('fixed40 offline fit real-root alias')
    return path


def _offline_fit_identity():
    """Bind the reviewed directory and lock inodes as well as the exact path."""
    root=_offline_fit_directory(OFFLINE_FIT_ROOT)
    directory=os.stat(root,follow_symlinks=False)
    lock=os.stat(root/'lock',follow_symlinks=False)
    if not stat.S_ISDIR(directory.st_mode) or not stat.S_ISREG(lock.st_mode):
        raise ValueError('fixed40 fit directory/lock file types')
    return ((directory.st_dev,directory.st_ino,stat.S_IFMT(directory.st_mode)),
            (lock.st_dev,lock.st_ino,stat.S_IFMT(lock.st_mode)))


def _fit_initial_state(profile,review):
    """Keep the initial schema4 body immutable across clone schema3 resumes."""
    snapshot=profile['development'];current=review['clone_state']
    fixed._pin_tuple(snapshot);fixed._pin_tuple(current)
    if (snapshot['path']!=str(OFFLINE_FIT_ROOT/'initial-development-state.json')
            or current['path']!=str(OFFLINE_FIT_ROOT/'state.json')
            or (snapshot['bytes'],snapshot['sha256'])!=(current['bytes'],current['sha256'])):
        raise ValueError('fixed40 exact immutable initial clone state')
    return snapshot


def _fit_profile_decode(wire,caller_charge):
    """Admit the complete wire, parse graph and caller before decoding."""
    core.integer(caller_charge,core.SEMANTIC,'fixed40 fit profile caller')
    if (caller_charge+2*len(wire)+128*core._wire_nodes(wire)
            +2*core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 fit profile predecode lifetime')
    return records.decode(wire,32768)


class _OfflineFitStateIO:
    """Bind only the fixed fit clone's state IO to its reviewed directory FD.

    Existing controllers retain complete parsing, debits, locks and write caps.
    This concrete adapter checks the held directory and lock descriptors and
    performs every state read, temporary write, replace and sync relative to
    the held directory. It never initializes a state or changes a resource cap.
    """

    @staticmethod
    def _fd_identity(fd):
        value=os.fstat(fd)
        return value.st_dev,value.st_ino,stat.S_IFMT(value.st_mode)

    def _held_fit_directory(self):
        fd=getattr(self,'_fit_directory_fd',None)
        if fd is None or self._fd_identity(fd)!=self._expected_fit_identity[0]:
            raise ValueError('fixed40 fit state requires reviewed held directory')
        return fd

    @contextmanager
    def _locked(self):
        _offline_fit_directory(self.directory)
        if getattr(self,'_fit_directory_fd',None) is not None:
            raise ValueError('fixed40 fit nested counter lock')
        scope=getattr(self,'_fit_scope',None)
        if scope is not None:scope()
        directory=os.open(self.directory,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        lock=None
        try:
            if self._fd_identity(directory)!=self._expected_fit_identity[0]:
                raise ValueError('fixed40 fit held directory identity')
            lock=os.open('lock',os.O_RDWR|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=directory)
            if self._fd_identity(lock)!=self._expected_fit_identity[1]:
                raise ValueError('fixed40 fit held lock identity')
            fcntl.flock(lock,fcntl.LOCK_EX)
            self._fit_directory_fd=directory
            if _offline_fit_identity()!=self._expected_fit_identity:
                raise ValueError('fixed40 fit locked pathname identity')
            yield
            if _offline_fit_identity()!=self._expected_fit_identity:
                raise ValueError('fixed40 fit completed pathname identity')
        finally:
            self._fit_directory_fd=None
            if lock is not None:os.close(lock)
            os.close(directory)

    def _read_state_raw(self):
        directory=self._held_fit_directory()
        fd=os.open('state.json',os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,
                   dir_fd=directory)
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError('fixed40 fit regular state')
            return os.read(fd,4097)
        finally:os.close(fd)

    def _replace_state_raw(self,raw):
        directory=self._held_fit_directory()
        if len(raw)>4096:raise ValueError('fixed40 fit full state write cap')
        name='state-fit-'+secrets.token_hex(8)+'.tmp'
        fd=os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,
                   0o600,dir_fd=directory)
        try:
            view=memoryview(raw)
            while view:
                written=os.write(fd,view)
                if written<=0:raise ValueError('fixed40 fit full state write')
                view=view[written:]
            os.fsync(fd)
        finally:os.close(fd)
        try:
            os.replace(name,'state.json',src_dir_fd=directory,dst_dir_fd=directory)
            os.fsync(directory)
        finally:
            try:os.unlink(name,dir_fd=directory)
            except FileNotFoundError:pass


class OfflineFitAdmission:
    """Separate nonphysical capability for the reviewed complete offline fit.

    The caller supplies an already authenticated RuntimeAdmission and an exact
    independent preflight-only review. This mode never sets final acceptance.
    Its fixed clone must already contain the reviewed full schema4 consumed
    state; migration never initializes or overwrites a resumed schema3 root.
    Complete artifact/read-schedule acceptance is required before construction.
    All direct reads require the concrete admitted debit callback before open.
    """

    REVIEW_FIELDS = {'schema','profile','source_identity','preflight','clone_root',
        'clone_state','protected_counter','complete_preflight_accepted',
        'frozen_source_accepted','mode','clone_filesystem'}

    def __init__(self, admission, review_pin, *, read_charge, live_bytes=0):
        if type(admission) is not RuntimeAdmission or admission.accepted:
            raise ValueError('fixed40 separate unaccepted runtime admission required')
        if not callable(read_charge):
            raise ValueError('fixed40 complete fit admission read debit required')
        if not callable(getattr(admission,'_full_read_charge',None)):
            raise ValueError('fixed40 fit requires every original admission read prepaid')
        core.integer(live_bytes,core.SEMANTIC,'fixed40 offline fit caller lifetime')
        if (admission.caller_charge+live_bytes+fixed.graph_charge(
                (admission._profile_wire,review_pin,FIT_PROTECTED_COUNTER))
                +32768+core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 offline fit caller preallocation lifetime')
        self.admission=admission
        self.accepted=False
        self._read_charge=read_charge
        self.review_pin=dict(review_pin)
        self._profile_wire=bytes(admission._profile_wire)
        self.caller_charge=admission.caller_charge+fixed.graph_charge(
            (self._profile_wire,self.review_pin,FIT_PROTECTED_COUNTER))+32768+live_bytes
        private=_fit_profile_decode(self._profile_wire,self.caller_charge)
        if private.get('controller')!='coherent-v3':
            raise ValueError('fixed40 fit requires approved coherent development successor')
        self._graphs=[private]
        def load(pin,cap,decoded=False):
            return admission._load(pin,cap,decoded=decoded,
                live_bytes=self.caller_charge+fixed.graph_charge(self._graphs))
        profile=load(admission.profile_pin,32768,True);self._graphs.append(profile)
        review=load(review_pin,32768,True);self._graphs.append(review)
        trace_archives=tuple(getattr(admission,'_sealed_read_trace_pins',()))
        review_fields=self.REVIEW_FIELDS|({'admission_read_traces'} if trace_archives else set())
        if (type(review) is not dict or set(review)!=review_fields
                or review['schema']!='fixed40-offline-fit-bridge-review-v1'
                or review['profile']!=admission.profile_pin
                or review['source_identity']!=profile['source_identity']
                or review['clone_root']!=str(OFFLINE_FIT_ROOT)
                or review['protected_counter']!=FIT_PROTECTED_COUNTER
                or review['complete_preflight_accepted'] is not True
                or review['frozen_source_accepted'] is not False
                or review['mode']!='offline_fit_clone_only_no_database_no_production'):
            raise ValueError('fixed40 exact independent fit-only review')
        if trace_archives:
            reviewed=review['admission_read_traces']
            if (type(reviewed) is not list or len(reviewed)!=len(trace_archives)
                    or any(fixed._pin_tuple(pin)!=row for pin,row in zip(reviewed,trace_archives))):
                raise ValueError('fixed40 complete sealed admission trace review binding')
            # Iterate the authenticated review list already held in _graphs.
            # Do not create another pin graph across the full archive reads.
            for pin in reviewed:
                load(pin,1048576)
            del reviewed
        directory=_offline_fit_directory(review['clone_root'])
        _fit_initial_state(profile,review)
        # RuntimeAdmission has authenticated the full immutable schema4 body.
        # The live state may now be schema3; only explicit one-time migration
        # requires its initial bytes/hash, and the controller rejects resets.
        self._root_identity=_offline_fit_identity()
        if review['clone_filesystem']!=[list(row) for row in self._root_identity]:
            raise ValueError('fixed40 exact reviewed clone directory/lock identity')
        load(review['preflight'],2097152)
        load(FIT_PROTECTED_COUNTER,4096)
        # Release every decoded local before constructing the retained capsule.
        del private,profile,review,directory,load
        self._graphs.clear()
        self.fit_accepted=True
        self.counter_type=self._counter_type()
        if self.caller_charge+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 complete fit admission caller lifetime')

    def _check_origin(self):
        return self.admission._load(FIT_PROTECTED_COUNTER,4096,
                                   live_bytes=self.caller_charge)

    def _counter_type(self):
        admission=self.admission
        parent=admission.counter_type
        private=_fit_profile_decode(self._profile_wire,self.caller_charge)
        profile={k:private[k] for k in ('development','authority','history','events','consumed')}
        del private
        expected_events=tuple(admission.event_record_hashes)
        expected_root=tuple(tuple(row) for row in self._root_identity)
        self.caller_charge+=fixed.graph_charge((profile,expected_events,expected_root))
        if self.caller_charge+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 fit retained counter capsule lifetime')
        review_sha=self.review_pin['sha256']
        # Capture the authenticated pin, debit callback and bound loader once.
        # Later mutation of the public fit wrapper cannot retarget these reads.
        protected_pin=tuple(FIT_PROTECTED_COUNTER[k] for k in ('path','bytes','sha256'))
        origin_load=admission._load
        origin_charge=getattr(admission,'_full_read_charge',None)
        self.caller_charge+=fixed.graph_charge(protected_pin)
        origin_caller=self.caller_charge
        if origin_caller+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 fit origin capsule lifetime')
        def check_origin(_owner=admission):
            pin=dict(zip(('path','bytes','sha256'),protected_pin))
            # The captured RuntimeAdmission loader debits before opening.
            return origin_load(pin,4096,live_bytes=origin_caller,read_charge=origin_charge)
        read_charge=admission._migration_read_charge

        class OfflineFitCounters(_OfflineFitStateIO,parent):
            """Full existing counters, with a distinct nonphysical identity."""
            _expected_fit_identity=expected_root
            @classmethod
            def limit_identity(cls):
                return core.digest((super().limit_identity(),
                    'fixed40-offline-fit-clone-only-v1',str(OFFLINE_FIT_ROOT),review_sha))

            def __init__(self,directory,*args,live_bytes=0,**kwargs):
                core.integer(live_bytes,core.SEMANTIC,'fixed40 cold counter caller')
                if (live_bytes+origin_caller+fixed.graph_charge((directory,args,kwargs))
                        +128*4097+16*4096+2*core.CHUNK+262144>core.SEMANTIC):
                    raise ValueError('fixed40 cold counter complete caller lifetime')
                self._offline_external_live=live_bytes
                _offline_fit_directory(directory)
                if kwargs.get('namespace')!='development':
                    raise ValueError('fixed40 fit cannot construct production counters')
                super().__init__(directory,*args,**kwargs)

            def _fit_scope(self):
                _offline_fit_directory(self.directory)
                if _offline_fit_identity()!=expected_root:
                    raise ValueError('fixed40 fit directory/lock identity changed')
                if self.namespace!='development' or self.slot!=0:
                    raise ValueError('fixed40 fit cannot operate production counters')

            def _read(self):
                self._fit_scope()
                return super()._read()

            def _write(self,value):
                self._fit_scope()
                return super()._write(value)

            def close_fit_debits(self,*,live_bytes=0,incomplete=False):
                """Publish complete final fit state and a bounded control receipt.

                This is a development closeout, never final fit acceptance.
                It preserves all cumulative costs, requires one complete
                sixteen-route round, rejects production, and retains every
                version. Each attempt first debits the full32768-byte IO hold.
                An independent consumer must still authenticate both complete
                published bodies and the unchanged live state before transfer.
                """
                core.integer(live_bytes,core.SEMANTIC,'fixed40 fit closeout caller')
                live_bytes+=getattr(self,'_offline_external_live',0)
                if type(incomplete) is not bool:
                    raise ValueError('fixed40 explicit incomplete closeout mode')
                # Admit full worst-case4KiB state parse, cold IO, publication
                # buffers and both origin reads before any file operation.
                if (live_bytes+origin_caller+128*4097+16*4096
                        +2*core.CHUNK+262144>core.SEMANTIC):
                    raise ValueError('fixed40 initial closeout lifetime')
                if (not self._io_scope or self._last_io_trace or len(self._io_trace)!=1
                        or self._io_trace[0]['operation']!='read'):
                    raise ValueError('fixed40 fit closeout requires fresh cold IO scope')
                check_origin()
                with self._locked():
                    value=self._read()
                    if value['pending'] is not None:
                        raise ValueError('fixed40 fit closeout cannot change pending work')
                    value['nonbinary']+=core.COUNTER_IO_RESERVE
                    value['generation']=core.integer(value['generation']+1,200000,
                                                      'fixed40 fit closeout generation')
                    # Failed completeness checks retain this attempted IO debit.
                    self._write(value)
                    if (value['pending'] is not None
                            or any(sum(row.values()) for row in value['production'])
                            or (not incomplete and (
                                value['development'][self.round_index]!=dict(core.ROUTES)
                                or value['failed_development'][self.round_index]))):
                        raise ValueError('fixed40 fit closeout requires all16 routes and no production')
                    caller=live_bytes+origin_caller+fixed.graph_charge(
                        (value,self.context_wire,self._io_trace))
                    if caller+8*4096+2*core.CHUNK+262144>core.SEMANTIC:
                        raise ValueError('fixed40 complete fit closeout lifetime')
                    directory=self._held_fit_directory()
                    state_raw=core.canonical(value)
                    if len(state_raw)>4096:raise ValueError('fixed40 full final state cap')
                    suffix=str(value['generation'])
                    state_name='fit-counter-g'+suffix+'.json'
                    receipt_name='fit-debit-closeout-g'+suffix+'.json'
                    def publish(name,raw):
                        if len(raw)>4096:raise ValueError('fixed40 fit closeout record cap')
                        self._io_check(len(raw))
                        fd=os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,
                                   0o600,dir_fd=directory)
                        try:
                            remaining=memoryview(raw)
                            while remaining:
                                count=os.write(fd,remaining)
                                if count<=0:raise ValueError('fixed40 complete closeout write')
                                remaining=remaining[count:]
                            os.fsync(fd)
                        finally:os.close(fd)
                        os.fsync(directory)
                        checksum=hashlib.sha256(raw).hexdigest()
                        self._io_trace.append(dict(operation='write',bytes=len(raw),
                            attempted_bytes=len(raw),sha256=checksum,
                            generation=value['generation'],authenticated=True))
                        return dict(path=str(self.directory/name),bytes=len(raw),sha256=checksum)
                    state_pin=publish(state_name,state_raw)
                    receipt=dict(schema=('fixed40-fit-incomplete-debit-closeout-v1' if incomplete
                                          else 'fixed40-fit-debit-closeout-v1'),
                        mode=('offline_fit_incomplete_no_runtime_acceptance' if incomplete
                              else 'offline_fit_only_no_runtime_acceptance'),
                        profile_sha256=self.PLAN_SHA,preflight_sha256=review_sha,
                        counter=state_pin,live_counter=str(self.directory/'state.json'),
                        round=self.round_index,complete_routes=dict(value['development'][self.round_index]),
                        consumed={k:value[k] for k in RUNTIME_HARD},
                        failures=list(value['failures']),generation=value['generation'],
                        protected_counter=dict(zip(('path','bytes','sha256'),protected_pin)),
                        prepublication_io=self.io_summary(),
                        receipt_path=str(self.directory/receipt_name),
                        receipt_maximum=4096,complete_io_reservation=core.COUNTER_IO_RESERVE)
                    raw=core.canonical(receipt)
                    receipt_pin=publish(receipt_name,raw)
                check_origin()
                result=dict(receipt=receipt_pin,counter=state_pin,
                            complete_io=self.io_summary(),runtime_accepted=False)
                if live_bytes+origin_caller+fixed.graph_charge(result)+2*core.CHUNK+262144>core.SEMANTIC:
                    raise ValueError('fixed40 fit closeout result lifetime')
                return result

            @classmethod
            def migrate(cls,*args,**kwargs):
                raise ValueError('fixed40 fit review cannot authorize real migration')

            @classmethod
            def migrate_fit_clone(cls,directory,*,consumed_events,event_records,
                                  accepted_preflight):
                _offline_fit_directory(directory)
                if _offline_fit_identity()!=expected_root:
                    raise ValueError('fixed40 fit migration directory/lock identity changed')
                if (accepted_preflight!=review_sha
                        or tuple(consumed_events)!=tuple(profile['events'])
                        or type(event_records) is not tuple
                        or event_records!=expected_events):
                    raise ValueError('fixed40 exact fit review/events/complete history')
                check_origin()
                class FitDevelopmentController(_OfflineFitStateIO,CoherentDevelopmentReservations):
                    _expected_fit_identity=expected_root
                controller=FitDevelopmentController(directory,
                    profile['authority']['sha256'],authority_pin=profile['authority'],
                    read_charge=read_charge)
                with controller._locked():
                    if _offline_fit_identity()!=expected_root:
                        raise ValueError('fixed40 fit migration locked identity changed')
                    current=controller._read()
                    actual=controller._io_trace[-1]
                    # Existing schema3 state is never reset by this constructor.
                    if (actual['bytes']!=profile['development']['bytes']
                            or actual['sha256']!=profile['development']['sha256']
                            or current['pending']!=[]):
                        raise ValueError('fixed40 changed or pending fit clone state')
                    RuntimeAdmission._check_development(profile,controller.config,current,
                        controller=CoherentDevelopmentReservations)
                    old=records.decode(current['legacy'].encode(),4096)
                    value=dict(old)
                    value.update(schema=3,limits=cls.limit_identity(),
                        legacy_raw=current['legacy'],
                        recovery=dict(plan=cls.PLAN_SHA,preflight=review_sha,
                            events=core.digest((profile['history'],event_records,current['chain'])),
                            inherited=list(consumed_events)),
                        development=cls._vectors(cls.DEVELOPMENT_ROUNDS),
                        production=cls._vectors(3),
                        namespace_roots=[None]*(cls.DEVELOPMENT_ROUNDS+3),
                        failed_development=[False]*cls.DEVELOPMENT_ROUNDS,
                        round_failure_events=[None]*cls.DEVELOPMENT_ROUNDS,
                        failures=list(consumed_events),
                        generation=core.integer(current['generation']+1,200000,
                                                'fixed40 fit migration generation'))
                    value.update(profile['consumed'])
                    # Both complete origin reads and full counter IO remain
                    # actual debits even though only the clone is modified.
                    value['nonbinary']+=core.COUNTER_IO_RESERVE+2*(protected_pin[1]+1)
                    for name,maximum in cls.LIMITS.items():
                        core.integer(value[name],maximum,'fixed40 preserved fit debit')
                    held=(cls.EVENT_LIMITS[0]-value['failures'][0])*core.COUNTER_IO_RESERVE
                    if value['nonbinary']+held>cls.LIMITS['nonbinary']:
                        raise ValueError('fixed40 fit correction outcomes unavailable')
                    if _offline_fit_identity()!=expected_root:
                        raise ValueError('fixed40 fit migration write identity changed')
                    controller._write(value)
                    if _offline_fit_identity()!=expected_root:
                        raise ValueError('fixed40 fit migration completed identity changed')
                check_origin()
                return controller.io_summary()
        return OfflineFitCounters
