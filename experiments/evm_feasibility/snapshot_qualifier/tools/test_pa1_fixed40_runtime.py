"""Focused fixed40 admission regressions; cloned counters only."""
import hashlib
import json
import os
import tempfile
import unittest
from types import SimpleNamespace
from pathlib import Path
import pa1_evidence as core
import pa1_fixed40 as fixed
import pa1_fixed40_runtime as runtime
import pa1_fixed40_constructors as constructors

ARTIFACTS=Path('/home/fry/artifacts/vrf40-retention-20261018')
OLD=Path('/home/fry/artifacts/vrf40-admission-01a14b66-20261018')
LEGACY=Path('/home/fry/artifacts/vrf-pa1-s3-01a1470a-20261017/fixed40/offline-fit-counters/state.json')

def pin(path):
    raw=path.read_bytes()
    return dict(path=str(path),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())

def put(path,value):
    path.write_bytes(core.canonical(value))
    return pin(path)


def fixture_read_partition(label):
    """Reserve a separate unchanged8MiB fixture packet from the full gate hold.

    Each gate owns three such packets: existing, initial full admission and
    resumed full admission. Their combined25165824 bytes remain inside the
    original509609576-byte full-pass reservation. Old journals never reset.
    """
    from unittest.mock import patch
    if label not in ('fit-initial','fit-resume'):raise ValueError('finite fixture packet')
    original=Path(os.environ['BRIDGE_READ_INVOICE'])
    raw=original.read_bytes()
    if hashlib.sha256(raw).hexdigest()!=os.environ['BRIDGE_READ_INVOICE_SHA']:
        raise ValueError('fixture partition exact original invoice')
    invoice=json.loads(raw)
    if invoice['read_limit']!=8388608:raise ValueError('unchanged fixture read cap')
    path=original.with_name(original.stem+'-'+label+'.json')
    invoice['journal']=str(path.with_suffix('.debits.jsonl'))
    invoice['scope']='NONHISTORICAL '+label+'8MiB subdivision; three packets held inside full509609576 test reservation'
    body=json.dumps(invoice,sort_keys=True,indent=2).encode()+bytes((10,))
    with path.open('xb') as out:out.write(body)
    digest=hashlib.sha256(body).hexdigest()
    def charge(packet):
        with patch.dict(os.environ,{'BRIDGE_READ_INVOICE':str(path),'BRIDGE_READ_INVOICE_SHA':digest}):
            engineering_charge(packet)
    return charge


def engineering_charge(packet):
    """Persist each test-only direct read debit before opening its bodies.

    The exact command invoice is retained before the gate. This fixture uses
    only its own finite subdivision of event107, never runtime admission.
    """
    invoice_path=Path(os.environ['BRIDGE_READ_INVOICE'])
    raw=invoice_path.read_bytes()
    invoice=json.loads(raw)
    expected=os.environ['BRIDGE_READ_INVOICE_SHA']
    if hashlib.sha256(raw).hexdigest()!=expected:
        raise ValueError('test exact read invoice')
    if (invoice['parent']['sha256']!='f1a64c436afe6711280f81e019f994f178651ecdf087dbb46c6696639831d841'
            or invoice['read_limit']!=8388608):
        raise ValueError('test finite engineering read invoice')
    parent=Path(invoice['parent']['path']).read_bytes()
    if (len(parent)!=invoice['parent']['bytes']
            or hashlib.sha256(parent).hexdigest()!=invoice['parent']['sha256']):
        raise ValueError('test exact parent reservation')
    journal=Path(invoice['journal'])
    previous=journal.read_bytes() if journal.exists() else b''
    used=sum(json.loads(row)['bytes'] for row in previous.splitlines())
    amount=sum(p['bytes']+1 for p in packet)+len(raw)+len(parent)+2+len(previous)+1
    if used+amount>invoice['read_limit']:
        raise ValueError('test finite engineering read exhaustion')
    record=dict(packet=packet,bytes=amount,parent=invoice['parent'],invoice_sha=expected)
    fd=os.open(journal,os.O_WRONLY|os.O_APPEND|os.O_CREAT,0o600)
    try:
        body=core.canonical(record)
        if os.write(fd,body)!=len(body):raise ValueError('test incomplete persistent debit')
        os.fsync(fd)
    finally:os.close(fd)


class EnvelopeTests(unittest.TestCase):
    def costs(self):
        consumed=dict(invocations=1,units=8766,seconds=1500,
                      nonbinary=80000000,binary=400058002)
        one=dict(invocations=16,units=12000,seconds=4800,
                 nonbinary=100000000,binary=1000000000)
        return consumed,dict(runtime.LATER_MINIMUM),one

    def test_correction_reads_are_separate_from_rounds(self):
        used,protected,one=self.costs()
        envelope=runtime.derive_runtime_envelope(used,protected,one,225304,(88,0,0))
        self.assertEqual(envelope['development_rounds'],3)
        self.assertEqual(envelope['event_limits'],(93,4,4))
        self.assertEqual(envelope['full_event_read_reservation'],5*32768)
        self.assertEqual(envelope['limits']['nonbinary'],
            runtime.RUNTIME_HARD['nonbinary']-protected['nonbinary']-5*32768)

    def test_read_boundary_reserves_failure_before_admission(self):
        used,protected,one=self.costs()
        used['nonbinary']=runtime.RUNTIME_HARD['nonbinary']-protected['nonbinary']-2*one['nonbinary']
        with self.assertRaisesRegex(ValueError,'capacity'):
            runtime.derive_runtime_envelope(used,protected,one,225304,(88,0,0))

    def test_later_physical_work_keeps_full_reservation(self):
        used,protected,one=self.costs();protected['binary']-=1
        with self.assertRaisesRegex(ValueError,'later37/32'):
            runtime.derive_runtime_envelope(used,protected,one,225304,(88,0,0))

    def test_boolean_resource_is_rejected(self):
        used,protected,one=self.costs();used['units']=True
        with self.assertRaises(ValueError):
            runtime.derive_runtime_envelope(used,protected,one,225304,(88,0,0))

    def test_event_history_cannot_reset(self):
        used,protected,one=self.costs()
        with self.assertRaisesRegex(ValueError,'inherited'):
            runtime.derive_runtime_envelope(used,protected,one,225304,(8,0,0))

    def test_control_exhaustion_does_not_raise_byte_limit(self):
        used,protected,one=self.costs()
        with self.assertRaisesRegex(ValueError,'outcome'):
            runtime.derive_runtime_envelope(used,protected,one,344148,(88,0,0))

    def test_compiler_rejects_an_untyped_gate_bypass(self):
        class Forged:
            caller_charge=0
            def check_compilation(self,*args):pass
        with self.assertRaisesRegex(ValueError,'concrete authenticated'):
            fixed.compile_selected40(consumed_events=(88,0,0),
                                     event_history=fixed.BASELINE_HISTORY,admission=Forged())


class CompleteStreamTests(unittest.TestCase):
    def trace_admission(self):
        obj=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        obj.read_trace=[dict(pin=('/home/fry/artifacts/NONHISTORICAL-trace-input',17,'a'*64),
            bytes=17,eof=True,decoded=False) for _ in range(120)]
        obj._graphs=[];obj._outer_live=0;obj._profile_wire=b'{}\n';obj._events=(209,0,0)
        obj.envelope={};obj.event_record_hashes=();obj.profile_pin=dict(
            path='/home/fry/artifacts/NONHISTORICAL-trace-profile',bytes=3,sha256='b'*64)
        obj._counter_metadata_charge=0;obj._history_constructor_pages=();obj._fit_carry_wire=None
        obj.caller_charge=fixed.graph_charge((obj.read_trace,obj._profile_wire))+32768
        obj._full_read_charge=lambda packet:None
        return obj

    def test_complete_trace_is_published_verified_and_only_then_released(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-sealed-trace-') as directory:
            obj=self.trace_admission();expected=core.canonical(obj.read_trace);before=obj.caller_charge
            published=[];reads=[];obj._full_read_charge=lambda packet:reads.append(packet)
            path=Path(directory)/'complete-trace.json'
            p=obj.seal_read_trace(str(path),publication_charge=lambda packet:published.append(packet))
            self.assertEqual(path.read_bytes(),expected)
            self.assertEqual(p,pin(path));self.assertEqual(published,[(p,)])
            self.assertEqual(reads,[(p,)])
            self.assertEqual(obj._sealed_read_trace_pins,(fixed._pin_tuple(p),))
            self.assertEqual(obj.read_trace,[dict(pin=fixed._pin_tuple(p),bytes=len(expected),eof=True,decoded=False)])
            self.assertLess(obj.caller_charge,before)

    def test_trace_publication_rejection_preserves_live_rows_and_creates_no_body(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-rejected-trace-') as directory:
            obj=self.trace_admission();rows=obj.read_trace;path=Path(directory)/'trace.json'
            def reject(packet):raise ValueError('publication hold exhausted')
            with self.assertRaisesRegex(ValueError,'hold exhausted'):
                obj.seal_read_trace(str(path),publication_charge=reject)
            self.assertIs(obj.read_trace,rows);self.assertFalse(path.exists())

    def test_existing_trace_is_reused_only_after_exact_fresh_sequence_and_full_read(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-reuse-trace-') as directory:
            obj=self.trace_admission();path=Path(directory)/'trace.json'
            path.write_bytes(core.canonical(obj.read_trace));expected=pin(path)
            published=[];reads=[];obj._full_read_charge=reads.append
            self.assertEqual(obj.seal_read_trace(str(path),publication_charge=published.append,
                reuse_pin=expected),expected)
            self.assertEqual(published,[]);self.assertEqual(reads,[(expected,)])
            changed=self.trace_admission();changed.read_trace[0]=dict(changed.read_trace[0],bytes=0)
            rows=changed.read_trace
            with self.assertRaisesRegex(ValueError,'fresh read sequence'):
                changed.seal_read_trace(str(path),publication_charge=published.append,reuse_pin=expected)
            self.assertIs(changed.read_trace,rows);self.assertEqual(published,[])

    def test_trace_external_caller_is_checked_before_publication_or_release(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-trace-caller-') as directory:
            obj=self.trace_admission();rows=obj.read_trace;path=Path(directory)/'trace.json';calls=[]
            with self.assertRaisesRegex(ValueError,'publication lifetime'):
                obj.seal_read_trace(str(path),publication_charge=calls.append,live_bytes=core.SEMANTIC)
            self.assertEqual(calls,[]);self.assertFalse(path.exists());self.assertIs(obj.read_trace,rows)

    def test_trace_requires_original_full_read_debit_before_publication(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-unpaid-trace-') as directory:
            obj=self.trace_admission();obj._full_read_charge=None;calls=[]
            path=Path(directory)/'trace.json';rows=obj.read_trace
            with self.assertRaisesRegex(ValueError,'before publication'):
                obj.seal_read_trace(str(path),publication_charge=calls.append)
            self.assertEqual(calls,[]);self.assertFalse(path.exists())
            self.assertIs(obj.read_trace,rows)

    def test_trace_verification_uses_bound_original_read_debit(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-bound-trace-') as directory:
            obj=self.trace_admission();reads=[];obj._full_read_charge=reads.append
            def publish(packet):obj._full_read_charge=None
            p=obj.seal_read_trace(str(Path(directory)/'trace.json'),publication_charge=publish)
            self.assertEqual(reads,[(p,)])

    def test_full_hash_eof_and_size_are_enforced(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='stream-fixture-') as directory:
            path=Path(directory)/'body';path.write_bytes(b'full original diagnostic\n')
            obj=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
            obj.read_trace=[];obj._graphs=[]
            p=pin(path)
            self.assertEqual(obj._load(p,1024),len(path.read_bytes()))
            path.write_bytes(path.read_bytes()+b'extra')
            with self.assertRaisesRegex(ValueError,'EOF'):
                obj._load(p,1024)
            path.write_bytes(b'x'*p['bytes'])
            with self.assertRaisesRegex(ValueError,'hash'):
                obj._load(p,1024)

    def test_symlink_cannot_supply_source(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='stream-link-fixture-') as directory:
            target=Path(directory)/'target';target.write_bytes(b'original')
            link=Path(directory)/'link';link.symlink_to(target)
            obj=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
            obj.read_trace=[];obj._graphs=[]
            p=pin(target);p['path']=str(link)
            with self.assertRaisesRegex(ValueError,'symlink path'):obj._load(p,1024)


class ActualSourceBridgeTest(unittest.TestCase):
    CONTROLLER=runtime.RetainedDevelopmentReservations
    PROFILE_SCHEMA='fixed40-development-runtime-profile-v1'
    EFFECTIVE=88
    PAGED_HISTORY=False

    def test_authenticated_source_to_cloned_runtime_transition(self):
        # One actual full101-source pass; other tests do not repeat it. This is
        # a unit fixture acceptance record, never authority for the real root.
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='bridge-fixture-') as directory:
            root=Path(directory);state=root/'state.json'
            raw=LEGACY.read_bytes()
            self.assertEqual(hashlib.sha256(raw).hexdigest(),fixed.RecoveryCounters.LEGACY_SHA)
            state.write_bytes(raw)
            (root/'lock').touch()
            old_record=put(root/'fixture-history-body.json',{'scope':'unit fixture, not historical evidence'})
            history=put(root/'history.json',dict(schema='fixed40-exact-development-history-v1',
                breach_event=36,effective=[self.EFFECTIVE,0,0],
                records=[dict(first=9,last=self.EFFECTIVE,pin=old_record)]))
            if self.PAGED_HISTORY:
                page1=put(root/'history-page1.json',dict(
                    schema='fixed40-exact-development-history-page-v1',
                    records=[dict(first=9,last=37,pin=old_record)]))
                page2=put(root/'history-page2.json',dict(
                    schema='fixed40-exact-development-history-page-v1',
                    records=[dict(first=38,last=self.EFFECTIVE,pin=old_record)]))
                history=put(root/'history-v2.json',dict(
                    schema='fixed40-exact-development-history-v2',
                    admission='NONHISTORICAL complete synthetic clone',
                    breach_event=36,durable=pin(LEGACY),effective=[self.EFFECTIVE,0,0],
                    pages=[dict(first=9,last=37,pin=page1),
                           dict(first=38,last=self.EFFECTIVE,pin=page2)]))
            caps=list(self.CONTROLLER.HARD)
            used=list(self.CONTROLLER.INHERITED)
            authority=put(root/'authority.json',dict(caps=caps,used=used,history=history,
                                                    profiles=[],development_only=True))
            options=({'read_charge':engineering_charge}
                     if self.PROFILE_SCHEMA.endswith('v2') else {})
            self.CONTROLLER.migrate(root,history_pin=history,**options,
                authority=authority['sha256'],authority_pin=authority,caps=tuple(caps),used=tuple(used))
            development=json.loads(state.read_bytes())
            snapshot=root/'development-snapshot.json';snapshot.write_bytes(state.read_bytes())
            source=json.loads((OLD/'recovery46-rust-source-freeze.json').read_bytes())
            tools=json.loads((OLD/'recovery46-tool-source-freeze.json').read_bytes())
            freeze=json.loads(Path(os.environ['BRIDGE_SOURCE_FREEZE']).read_bytes())
            current={row['path']:dict(path=row['path'],bytes=row['bytes'],sha256=row['sha256'])
                     for row in freeze['files']}
            # This fixture exercises current-source admission. Historical gate
            # reuse is tested separately against its immutable original roster.
            source=[current.get(row['path'],row) for row in source]
            tools=[current.get(row['path'],row) for row in tools]
            development_sources=json.loads((OLD/'recovery46-development-source-freeze.json').read_bytes())
            tools+=[current.get(row['path'],row) for row in development_sources]
            for name in ('pa1_fixed40_runtime.py','test_pa1_fixed40_runtime.py','pa1_fixed40_constructors.py'):
                tools.append(current[str(Path(runtime.__file__).parent/name)])
            self.assertEqual(len(source+tools),101)
            # NONHISTORICAL fixture prepayment follows the complete admission
            # closure. Width slots cover the later profile/review fixed point;
            # they do not change a runtime or engineering read ceiling.
            fixture_prepaid=sum(row["bytes"]+1 for row in source+tools)
            fixture_prepaid+=17908+1+1214+1+pin(snapshot)["bytes"]+1
            fixture_prepaid+=2*(authority["bytes"]+1+history["bytes"]+1)
            page_rows=[page1,page2] if self.PAGED_HISTORY else []
            fixture_prepaid+=2*sum(row["bytes"]+1 for row in page_rows)
            fixture_prepaid+=(2 if self.PAGED_HISTORY else 1)*(old_record["bytes"]+1)
            if self.PROFILE_SCHEMA.endswith("v2"):
                fixture_prepaid+=2*(self.CONTROLLER.PLAN_PIN["bytes"]+1+
                    sum(row["bytes"]+1 for row in self.CONTROLLER.REVIEW_PINS))
                fixture_prepaid+=16384+1+len(raw)+1
            fixture_prepaid+=32768+1+32768+1+4096+1
            self.assertLessEqual(fixture_prepaid,8*1024*1024)
            print(json.dumps(dict(scope="NONHISTORICAL-fixture-full-read-prepayment",
                old_fixed_prepaid=3000000,prepaid=fixture_prepaid,
                bound_delta=fixture_prepaid-3000000,partition=8*1024*1024)))
            legacy=json.loads(raw)
            consumed=dict(invocations=legacy['invocations'],
                units=legacy['units']+development['used'][4],
                seconds=legacy['seconds']+development['used'][5],
                binary=legacy['binary']+development['used'][3],
                nonbinary=legacy['nonbinary']+development['used'][2]+fixture_prepaid)
            if self.PROFILE_SCHEMA.endswith('v2'):
                # Engineering history remains fully in its authenticated root;
                # its enlarged read/time budgets never enter runtime working
                # counters. Preserve every actual old runtime debit.
                consumed={k:legacy[k] for k in runtime.RUNTIME_HARD}
                consumed['nonbinary']+=fixture_prepaid
            one=dict(invocations=16,units=12000,seconds=4800,
                     nonbinary=100000000,binary=1000000000)
            profile=dict(schema=self.PROFILE_SCHEMA,
                development=pin(snapshot),authority=authority,history=history,
                sources=source+tools,source_identity=core.digest(source+tools),
                events=[self.EFFECTIVE,0,0],consumed=consumed,protected=dict(runtime.LATER_MINIMUM),
                round_reservation=one,control_used=225304,runtime_setup=16498255)
            if self.PROFILE_SCHEMA.endswith('v2'):
                profile['development_successor']=dict(self.CONTROLLER.PLAN_PIN)
                profile['control_used']=development['used'][6]
                profile['runtime_prefix']=put(root/'runtime-prefix.json',dict(
                    schema='fixed40-runtime-consumed-prefix-v1',counter=pin(LEGACY),
                    consumed={k:legacy[k] for k in runtime.RUNTIME_HARD}))
            if getattr(self,'OFFLINE_FIT_FIXTURE',False):
                self.exercise_restricted_fit(root,state,snapshot,profile,old_record,raw)
                return
            profile_pin=put(root/'profile.json',profile)
            preflight=put(root/'fixture-preflight.json',dict(scope='unit fixture; no real preflight'))
            review=put(root/'fixture-independent-review.json',dict(
                schema='fixed40-runtime-bridge-acceptance-v1',profile=profile_pin,
                source_identity=profile['source_identity'],preflight=preflight,
                frozen_source_accepted=True,complete_preflight_accepted=True))
            if self.PROFILE_SCHEMA.endswith('v2'):
                original_prefix=profile['runtime_prefix']
                prefix=json.loads(Path(original_prefix['path']).read_bytes())
                prefix['consumed']['units']+=1
                profile['runtime_prefix']=put(root/'higher-runtime-prefix.json',prefix)
                with self.assertRaisesRegex(ValueError,'consumed resource rollback'):
                    runtime.RuntimeAdmission(put(root/'rollback-profile.json',profile))
                prefix['counter']=old_record
                profile['runtime_prefix']=put(root/'forged-runtime-prefix.json',prefix)
                with self.assertRaisesRegex(ValueError,'exact runtime prefix legacy counter'):
                    runtime.RuntimeAdmission(put(root/'forged-prefix-profile.json',profile))
                profile['runtime_prefix']=original_prefix
            prep=runtime.RuntimeAdmission(profile_pin,review_pin=review)
            self.assertTrue(prep.accepted)
            if self.PROFILE_SCHEMA.endswith('v2'):
                compiled=fixed.compile_selected40(consumed_events=(self.EFFECTIVE,0,0),
                    event_history=fixed.BASELINE_HISTORY,admission=prep)
                self.assertEqual(len(json.loads(compiled['prior83_wire'])),83)
                self.assertEqual(json.loads(compiled['slot_wire'])['payloads'],40)
                self.assertFalse(compiled['complete_fit'])
                del compiled
                # A caller-owned selection must not replace the immutable
                # controller derived from the authenticated profile schema.
                prep._development_controller=runtime.RetainedDevelopmentReservations
                prep.counter_type=prep._counter_type()
            if self.PROFILE_SCHEMA.endswith('v2'):
                from unittest.mock import patch
                failed=runtime.RuntimeAdmission(profile_pin,review_pin=review)
                original=state.read_bytes()
                state.write_bytes(original+b' ')
                with self.assertRaisesRegex(ValueError,'changed/pending development root'):
                    failed.counter_type.migrate(root,consumed_events=(self.EFFECTIVE,0,0),
                        event_records=(old_record['sha256'],)*(self.EFFECTIVE-8),
                        accepted_preflight=review['sha256'])
                state.write_bytes(original)
                # Recreating the class cannot mint another prepaid read packet.
                failed.counter_type=failed._counter_type()
                with patch.object(self.CONTROLLER,'authenticate') as opened:
                    with self.assertRaisesRegex(ValueError,'exact prepaid constructor reads'):
                        failed.counter_type.migrate(root,consumed_events=(self.EFFECTIVE,0,0),
                            event_records=(old_record['sha256'],)*(self.EFFECTIVE-8),
                            accepted_preflight=review['sha256'])
                    opened.assert_not_called()
            self.assertEqual(prep.counter_type.DEVELOPMENT_ROUNDS,3)
            with self.assertRaisesRegex(ValueError,'independent acceptance'):
                prep.counter_type.migrate(root,consumed_events=(self.EFFECTIVE,0,0),
                    event_records=(old_record['sha256'],)*(self.EFFECTIVE-8),accepted_preflight='0'*64)
            # Changing caller-owned input after authentication cannot change
            # the already-frozen class's migration authority or event floor.
            profile['events'][0]=8
            with self.assertRaisesRegex(ValueError,'complete event records'):
                prep.counter_type.migrate(root,consumed_events=(self.EFFECTIVE,0,0),
                    event_records=('0'*64,)*(self.EFFECTIVE-8),accepted_preflight=review['sha256'])
            prep.counter_type.migrate(root,consumed_events=(self.EFFECTIVE,0,0),
                event_records=(old_record['sha256'],)*(self.EFFECTIVE-8),accepted_preflight=review['sha256'])
            result=json.loads(state.read_bytes())
            self.assertEqual(result['legacy_raw'],raw.decode())
            self.assertEqual(result['failures'],[self.EFFECTIVE,0,0])
            for key in consumed:self.assertGreaterEqual(result[key],consumed[key])
            self.assertEqual(result['nonbinary'],consumed['nonbinary']+32768)
            self.assertEqual(len(result['development']),3)
            self.assertEqual(len(result['production']),3)
            self.assertLessEqual(len(state.read_bytes()),4096)
            context={'route':'archive','phase':'development:0:0:archive'}
            roster=(core.digest(context),)+tuple(core.digest(('fixture',i)) for i in range(15))
            counter=prep.counter_type(root,legacy['binding'],fixed.BASELINE_HISTORY,
                namespace='development',round_index=0,slot=0,context=context,context_roster=roster)
            counter.begin_io_scope()
            admitted=counter.admit(invocation=1,units=1,nonbinary=100,binary=0,seconds=1,
                                   slot=0,route='archive')
            counter.finish(admitted['generation'],failed=True)
            after=json.loads(state.read_bytes())
            self.assertEqual(after['failures'],[self.EFFECTIVE+1,0,0])
            self.assertIsNone(after['pending'])
            self.assertGreater(after['nonbinary'],result['nonbinary'])
            self.assertEqual(LEGACY.read_bytes(),raw)


class ConstructorTests(unittest.TestCase):
    def test_completed_failure_is_not_accepted_as_a_passing_gate(self):
        p=dict(path='/home/fry/artifacts/full-log',bytes=12,sha256='a'*64)
        result=json.loads(constructors.gate_result(command=['false'],cwd='/workspaces/rustaxa-evm',
            exit_code=1,finished_utc='2026-10-18T00:00:00Z',seconds=1,log=p))
        self.assertEqual(result['exit_code'],1)
        self.assertNotIn('accepted',result)

    def test_deadline_is_not_extended_by_constructor(self):
        with self.assertRaises(ValueError):
            constructors.gate_result(command=['test'],cwd='/workspaces/rustaxa-evm',
                exit_code=0,finished_utc='now',seconds=301,
                log=dict(path='/home/fry/artifacts/log',bytes=0,sha256='a'*64))



class CoherentSourceBridgeTest(ActualSourceBridgeTest):
    CONTROLLER=runtime.CoherentDevelopmentReservations
    PROFILE_SCHEMA='fixed40-development-runtime-profile-v2'
    EFFECTIVE=107


class CoherentPolicyTests(unittest.TestCase):
    def test_pinned_successor_preserves_runtime_limits(self):
        controller=runtime.CoherentDevelopmentReservations
        plan=controller.authenticate_successor(read_charge=engineering_charge)
        self.assertGreater(controller.HARD[1],core.SOURCE)
        self.assertGreater(controller.HARD[2],runtime.RUNTIME_HARD['nonbinary'])
        self.assertGreater(controller.HARD[5],runtime.RUNTIME_HARD['seconds'])
        self.assertEqual(core.SOURCE,4194304)
        self.assertEqual(core.SEMANTIC,2097152)
        self.assertEqual(controller.HARD[3],plan['exact_derivation']['physical_binary_upper'])
        self.assertEqual(runtime.RUNTIME_HARD['binary'],449080998528)
        self.assertEqual(controller.INHERITED[1],4187018)
        self.assertEqual(controller.INHERITED[2],38020526127+6422528)

    def test_development_plan_cannot_raise_runtime_source_or_allocation(self):
        controller=runtime.CoherentDevelopmentReservations
        plan=controller.authenticate_successor(read_charge=engineering_charge)
        for key in ('current_source','allocation','framing','binary_lifecycle'):
            changed=json.loads(json.dumps(plan))
            changed['runtime_unchanged'][key]+=1
            with self.assertRaisesRegex(ValueError,'runtime working limits'):
                controller.check_plan(changed)

    def test_new_cumulative_limits_cannot_enter_legacy_profile(self):
        controller=runtime.CoherentDevelopmentReservations
        legacy=json.loads(LEGACY.read_bytes())
        state=dict(schema=4,legacy=core.canonical(legacy).decode(),
            authority='a'*64,history='b'*64,events=[107,0,0],generation=4,
            caps=list(controller.HARD),used=list(controller.INHERITED),
            pending=[],chain='c'*64,closeout=65536,issued=0)
        profile=dict(authority=dict(sha256='a'*64),history=dict(sha256='b'*64))
        authority=dict(caps=list(controller.HARD),used=list(controller.INHERITED))
        with self.assertRaises(ValueError):
            runtime.RuntimeAdmission._check_development(profile,authority,state)
        runtime.RuntimeAdmission._check_development(
            profile,authority,state,controller=controller)
        state['used'][2]-=1
        with self.assertRaisesRegex(ValueError,'rollback'):
            runtime.RuntimeAdmission._check_development(
                profile,authority,state,controller=controller)


class CurrentSourceLimitTests(unittest.TestCase):
    def test_current_caps_reject_before_opening_any_source(self):
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='source-limit-fixture-') as directory:
            root=Path(directory)
            for coherent in (False,True):
                for individual in (False,True):
                    sources=[dict(path=str(root/('missing-'+str(i))),bytes=0,sha256='a'*64)
                             for i in range(101)]
                    if individual:
                        sources[0]['bytes']=1048577
                        message='individual current source'
                    else:
                        for row in sources:row['bytes']=4194304//101+1
                        message='current source aggregate'
                    value=dict.fromkeys(runtime.RuntimeAdmission.PROFILE_FIELDS)
                    value.update(schema='fixed40-development-runtime-profile-v2' if coherent
                                 else 'fixed40-development-runtime-profile-v1',
                        runtime_setup=16498255,sources=sources,source_identity=core.digest(sources))
                    if coherent:
                        value['development_successor']=dict(runtime.CoherentDevelopmentReservations.PLAN_PIN)
                        value['runtime_prefix']=None
                    p=put(root/('profile-'+str(coherent)+'-'+str(individual)+'.json'),value)
                    with self.assertRaisesRegex(ValueError,message):
                        runtime.RuntimeAdmission(p)


class CurrentRustPreparationTests(unittest.TestCase):
    """Exercise the real current101 read stage without creating runtime state."""
    used=0
    opened=[]

    def charge(self,packet):
        amount=sum(row['bytes']+1 for row in packet)
        cls=type(self)
        if cls.used+amount>8388608:
            raise ValueError('current Rust preparation test read reservation')
        cls.used+=amount
        cls.opened.extend(row['path'] for row in packet)
        body=core.canonical(dict(phase='BEFORE_OPEN',packet=packet,
            bytes_plus_EOF=amount,cumulative=cls.used,
            scope='event213 current Rust preparation regressions only'))
        with Path(os.environ['CURRENT_RUST_READ_JOURNAL']).open('ab') as out:
            out.write(body);out.flush();os.fsync(out.fileno())

    def profile(self,corrupt=False):
        manifest=Path(os.environ['BRIDGE_SOURCE_FREEZE'])
        self.charge((dict(path=str(manifest),bytes=manifest.stat().st_size),))
        raw=manifest.read_bytes()
        sources=[{k:row[k] for k in ('path','bytes','sha256')}
                 for row in json.loads(raw)['files']]
        rust=next(row for row in sources if row['path'].endswith('/src/vrf_shared_window.rs'))
        old_path=OLD/'recovery46-rust-source-freeze.json'
        self.charge((dict(path=str(old_path),bytes=old_path.stat().st_size),))
        old=json.loads(old_path.read_bytes())
        self.assertNotEqual(rust,next(row for row in old if row['path']==rust['path']))
        if corrupt:rust['sha256']='f'*64
        root=Path(os.environ['CURRENT_RUST_FIXTURES'])/self._testMethodName
        root.mkdir(parents=True,exist_ok=False)
        absent=dict(path=str(root/'deliberately-unavailable-authority.json'),
                    bytes=1,sha256='a'*64)
        value=dict.fromkeys(runtime.RuntimeAdmission.PROFILE_FIELDS)
        value.update(schema='fixed40-development-runtime-profile-v2',
            development_successor=dict(runtime.CoherentDevelopmentReservations.PLAN_PIN),
            runtime_prefix=None,runtime_setup=16498255,sources=sources,
            source_identity=core.digest(sources),authority=absent)
        path=root/'profile.json';body=core.canonical(value)
        with path.open('xb') as out:
            out.write(body);out.flush();os.fsync(out.fileno())
        return dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest()),absent,rust['path']

    def test_current_rust_reaches_authority_stage_without_runtime_acceptance(self):
        profile,absent,_=self.profile()
        obj=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        with self.assertRaises(FileNotFoundError) as failure:
            obj.__init__(profile,read_charge=self.charge)
        self.assertEqual(failure.exception.filename,absent['path'])
        self.assertFalse(obj.accepted)
        self.assertFalse(Path(absent['path']).exists())

    def test_changed_rust_body_still_requires_its_exact_current_hash(self):
        profile,absent,rust=self.profile(corrupt=True)
        start=len(type(self).opened)
        with self.assertRaises(ValueError):
            runtime.RuntimeAdmission(profile,read_charge=self.charge)
        opened=type(self).opened[start:]
        self.assertIn(rust,opened)
        self.assertNotIn(absent['path'],opened)


class DirectReadReservationTests(unittest.TestCase):
    def test_missing_direct_read_reservation_fails_before_authentication(self):
        with self.assertRaises(TypeError):
            runtime.CoherentDevelopmentReservations.authenticate_successor()

    def test_failed_charge_prevents_any_authentication(self):
        from unittest.mock import patch
        controller=runtime.CoherentDevelopmentReservations
        def exhausted(packet):
            self.assertEqual(packet,(controller.PLAN_PIN,*controller.REVIEW_PINS))
            raise ValueError('prepaid read exhausted')
        with patch.object(controller,'authenticate') as opened:
            with self.assertRaisesRegex(ValueError,'exhausted'):
                controller.authenticate_successor(read_charge=exhausted)
            opened.assert_not_called()

    def test_profile_is_live_before_plan_decode(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='live-profile-fixture-') as directory:
            path=Path(directory)/'profile.json'
            value=dict.fromkeys(runtime.RuntimeAdmission.PROFILE_FIELDS)
            value.update(schema='fixed40-development-runtime-profile-v2',
                development_successor=runtime.CoherentDevelopmentReservations.PLAN_PIN,
                runtime_prefix=None)
            pp=put(path,value)
            original=runtime.RuntimeAdmission._load
            def load(obj,pin,maximum,**kwargs):
                if pin==runtime.CoherentDevelopmentReservations.PLAN_PIN:
                    self.assertEqual(obj._graphs,[value])
                    raise ValueError('live profile confirmed')
                return original(obj,pin,maximum,**kwargs)
            with patch.object(runtime.RuntimeAdmission,'_load',load):
                with self.assertRaisesRegex(ValueError,'live profile confirmed'):
                    runtime.RuntimeAdmission(pp)


class ConcreteRoleConstructorTests(unittest.TestCase):
    def test_full_log_bytes_survive_and_overflow_rejects(self):
        raw=b'\xff'*(constructors.ROLE_MAXIMA[1])
        self.assertIs(constructors.complete_log(1,raw,maxima=constructors.ROLE_MAXIMA),raw)
        with self.assertRaisesRegex(ValueError,'full role-bound log'):
            constructors.complete_log(1,raw+b'x',maxima=constructors.ROLE_MAXIMA)
        with self.assertRaises(ValueError):
            constructors.complete_log(0,b'',maxima=constructors.ROLE_MAXIMA)

    def test_fresh_gate_preserves_source_command_and_failure_evidence(self):
        source=dict(path='/home/fry/artifacts/snapshot',bytes=0,sha256='a'*64)
        command=dict(path='/home/fry/artifacts/command',bytes=0,sha256='b'*64)
        log=dict(path='/home/fry/artifacts/log',bytes=0,sha256='c'*64)
        raw=constructors.validation_result(0,maxima=constructors.ROLE_MAXIMA,
            source_snapshot=source,command_record=command,live_bytes=0,command=['false'],
            cwd='/workspaces/rustaxa-evm',exit_code=17,finished_utc='2026-10-18T00:00:00Z',
            seconds=1,log=log)
        value=json.loads(raw)
        self.assertEqual(value['exit_code'],17)
        self.assertEqual(value['source_snapshot'],source)
        self.assertEqual(value['command_record'],command)
        self.assertEqual(value['cache_status'],'FRESH_EXECUTION')
        self.assertNotIn('accepted',value)

    def test_source_roles_share_the_complete101_closure_and_aggregate_cap(self):
        freeze=json.loads(Path(os.environ['BRIDGE_SOURCE_FREEZE']).read_bytes())
        sources=[{k:p[k] for k in ('path','bytes','sha256')} for p in freeze['files']]
        for role,bounds in ((38,(0,90)),(39,(90,101))):
            raw=constructors.current_source_role(role,sources,
                maxima=constructors.ROLE_MAXIMA,live_bytes=0)
            self.assertEqual(json.loads(raw),sources[slice(*bounds)])
        changed=[dict(p) for p in sources]
        for p in changed[:5]:p['bytes']=1048576
        with self.assertRaisesRegex(ValueError,'source aggregate'):
            constructors.current_source_role(39,changed,maxima=constructors.ROLE_MAXIMA,live_bytes=0)

    def test_all_original64_wrapper_children_are_consumed_with_aliases(self):
        from types import SimpleNamespace
        compiled=fixed.compile_selected40(consumed_events=(4,0,0),
                                          event_history=fixed.BASELINE_HISTORY)
        rows={}
        for wire in compiled['units']:
            for row in json.loads(wire):
                if 524<=row[0]<588:rows[row[0]]=row
        original=constructors.records.authenticate
        class Reader:
            def __init__(self):
                self.invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:4096)
                self.declaration_charge=2048
                self.calls=[]
            def consume(self,occurrence,pin,**kwargs):
                original(dict(role='test-full-original-body',**pin))
                st=Path(pin['path']).stat()
                self.last_file_identity=(st.st_dev,st.st_ino)
                self.calls.append((occurrence,pin))
        offset=524;seen={};total=0
        for role,placement in zip((28,37,49),compiled['context']['reference_placements']):
            count=1+len(placement['references'])
            declarations=tuple(rows[i] for i in range(offset,offset+count))
            findings=(compiled['context']['source28_prefix'] if role==28 else '')+'CURRENT UNIT FIXTURE NO ACCEPTANCE'
            reader=Reader()
            raw=constructors.acceptance_record(role,maxima=constructors.ROLE_MAXIMA,
                compiled=compiled,destination=str(ARTIFACTS/('fixture-role-'+str(role)+'.json')),
                findings=findings,unit=reader,declarations=declarations,identities=seen,live_bytes=0)
            value=json.loads(raw)
            self.assertFalse(value['accepted'])
            self.assertEqual(value['findings'],findings)
            self.assertEqual(len(reader.calls),count)
            self.assertEqual(reader.invocation.operations[1024+role],1)
            total+=sum(p['bytes'] for _,p in reader.calls)
            offset+=count
        self.assertEqual(offset,588)
        self.assertEqual(total,1048091)
        self.assertEqual(constructors.constructed_role_coverage(),tuple(range(81)))


class CandidateConstructorTests(unittest.TestCase):
    def inputs(self):
        slot=fixed.selected_descriptor()['slot']
        raw=Path('/home/fry/artifacts/vrf-run-01a141ba-20261015/luna/offline-full-tuple-map.json').read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(),'43ffbffd48d1d287b2c8c46e25a07456b43a2bcbc099312a6c64cf57b16b56c8')
        mapping=json.loads(raw)
        rows=[row for row in mapping['residual_full_tuples']
              if row['full_tuple']['voter_hex']==slot['voter']]
        report_path=Path('/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-first-d-replay.json')
        report_raw=report_path.read_bytes()
        self.assertEqual(hashlib.sha256(report_raw).hexdigest(),'8bd32c28356e85944a37a900c95e8ae3cb7cbc7f942f27cc2769257b6be36854')
        report=json.loads(report_raw)
        seal_path=Path('/home/fry/artifacts/vrf-round3-01a141ba-20261016/next42/fact-outcome-freeze.json')
        seal_raw=seal_path.read_bytes()
        self.assertEqual(hashlib.sha256(seal_raw).hexdigest(),'eb18e6b3b228a0500a69eb3f511ac320b0a5bf677eb602962c1a22f38a930e1b')
        seal=json.loads(seal_raw)
        review=next(p for p in seal['pins'] if p['path'].endswith('/family-final-review.md'))
        def pin(path,raw):
            return dict(path=str(path),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        return dict(maxima=constructors.ROLE_MAXIMA,slot=slot,mapped_rows=rows,
            prior_report=report,map_pin=pin(Path('/home/fry/artifacts/vrf-run-01a141ba-20261015/luna/offline-full-tuple-map.json'),raw),
            map_acceptance=dict(path='/home/fry/artifacts/vrf-run-01a141ba-20261015/sol/raw-map-check.json',
                bytes=44561,sha256='7af99017b5ab30b981e8fc688a329ec0e5d58f55ebedc90454798a625e289f1e'),
            prior_bundle=dict(path=str(report_path.with_name('vrf-first-d.rcv1')),bytes=417594,
                sha256='6382e8f25cc9ec2225e563111e37b9ad8c8beae4167543cb4d9e915a7d5cbb61'),
            prior_report_pin=pin(report_path,report_raw),accepted_seal=pin(seal_path,seal_raw),
            accepted_review=review,live_bytes=0)

    def test_actual_candidate_keeps40_bindings_and_original_full20_contract(self):
        args=self.inputs()
        raw=constructors.selected40_candidate(**args)
        value=json.loads(raw)
        self.assertLessEqual(len(raw),constructors.ROLE_MAXIMA[42])
        self.assertEqual(len(value['full_tuples']),40)
        self.assertEqual([r['request_index'] for r in value['full_tuples']],
                         [r['request_index'] for r in args['mapped_rows']])
        self.assertEqual(value['prior_context']['selected_prior']['proof_index'],68)
        ref=value['prior_context']['all_20_full_source_bindings_reference']
        self.assertEqual(ref['count'],20)
        self.assertEqual(ref['report_pin'],args['prior_report_pin'])
        self.assertFalse(value['accepted'])

    def prepared(self):
        args=self.inputs()
        raw=constructors.selected40_candidate(**args)
        candidate=json.loads(raw)
        candidate_pin=dict(path='/home/fry/artifacts/fixed40-candidate-test.json',
            bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        raw=constructors.candidate_preparation(maxima=constructors.ROLE_MAXIMA,
            candidate=candidate,candidate_pin=candidate_pin,prior_report=args['prior_report'],live_bytes=0)
        return candidate,candidate_pin,json.loads(raw)

    def test_all40_preparation_rows_and_prior_bind_exactly(self):
        candidate,pin,prepared=self.prepared()
        self.assertEqual(len(prepared['rows']),40)
        self.assertEqual(prepared['prior_first_d']['selection_index'],17)
        result=json.loads(constructors.compare_candidate_preparation(
            maxima=constructors.ROLE_MAXIMA,candidate=candidate,candidate_pin=pin,
            preparation=prepared,live_bytes=0))
        self.assertFalse(result['accepted'])
        self.assertFalse(result['ready'])
        self.assertEqual(result['bundle_proofs'],91)

    def test_preparation_parent_source_and_prior_tamper_reject(self):
        for target in ('parent','source','prior'):
            candidate,pin,prepared=self.prepared()
            if target=='parent':prepared['rows'][39]['original_parent_manifest_index']+=1
            elif target=='source':prepared['rows'][39]['source_row']['source_references']=[]
            else:prepared['prior_first_d']['accepted_value']['proof_index']=60
            with self.assertRaisesRegex(ValueError,'binding'):
                constructors.compare_candidate_preparation(maxima=constructors.ROLE_MAXIMA,
                    candidate=candidate,candidate_pin=pin,preparation=prepared,live_bytes=0)

    def test_parent_reference_and_full_context_cannot_be_omitted(self):
        args=self.inputs()
        args['mapped_rows'][0]['source_references']=[]
        with self.assertRaisesRegex(ValueError,'binding'):
            constructors.selected40_candidate(**args)
        args=self.inputs()
        args['prior_report']['selection'].pop()
        with self.assertRaisesRegex(ValueError,'full20'):
            constructors.selected40_candidate(**args)

    def test_constructor_caller_limit_is_not_raised(self):
        args=self.inputs();args['live_bytes']=core.SEMANTIC
        with self.assertRaisesRegex(ValueError,'caller lifetime'):
            constructors.selected40_candidate(**args)


class FullInstanceConstructorTests(unittest.TestCase):
    """Pure constructor fixtures grant no current source or runtime acceptance."""
    def inputs(self):
        freeze=json.loads(Path(os.environ['BRIDGE_SOURCE_FREEZE']).read_bytes())
        sources=[{k:p[k] for k in ('path','bytes','sha256')} for p in freeze['files']]
        def p(name,size=1):
            return dict(path='/home/fry/artifacts/fixed40-unit-fixture/'+name,
                        bytes=size,sha256='a'*64)
        prefix='/workspaces/rustaxa-evm/local/evm-state-db/snapshot-litenode-copy'
        return dict(maxima=constructors.ROLE_MAXIMA,run='/home/fry/artifacts/fixed40-unit-fixture',
            source_commit='1'*40,sources=sources[:90],tool_sources=sources[90:],
            binaries=[p('live'),p('replay')],evidence=[p('evidence'),p('evidence')],
            preflight=p('preflight'),preparation=p('preparation'),
            copy_metadata=[dict(path=prefix+s,device=1,inode=i+1,mode=16832,mtime_ns=1)
                           for i,s in enumerate(('','/db/db','/db/state_db'))],live_bytes=0)

    def test_spec_preserves_all_fields_and_duplicate_evidence_placements(self):
        args=self.inputs()
        first=constructors.instance_spec(56,**args)
        self.assertEqual(first,constructors.instance_spec(58,**args))
        value=json.loads(first)
        self.assertEqual(set(value),{'schema','run','source_commit','sources','tool_sources',
            'binaries','evidence','preflight','preparation','copy_metadata'})
        self.assertEqual(value['evidence'],[[1,'a'*64],[1,'a'*64]])
        self.assertEqual(len(value['sources']),90)
        self.assertEqual(len(value['tool_sources']),11)
        for field in ('sources','tool_sources','binaries','evidence','copy_metadata'):
            bad=self.inputs();bad[field]=[]
            with self.assertRaises(ValueError):constructors.instance_spec(56,**bad)

    def frozen(self):
        args=self.inputs();spec=constructors.instance_spec(56,**args)
        instance=dict(path=args['run']+'/instance-spec.json',bytes=len(spec),
                      sha256=hashlib.sha256(spec).hexdigest())
        return json.loads(constructors.command_freeze(**args,instance=instance,
            capture_command=[args['binaries'][0]['path'],'--bundle-output',args['run']+'/bundle.rcv6',
                             '--report-output',args['run']+'/capture.json'],embedded_source_bytes=1))

    def test_native_capture_flags_match_sealed_rust_cli(self):
        args=self.inputs()
        spec=constructors.instance_spec(56,**args)
        instance=dict(path=args['run']+'/instance-spec.json',bytes=len(spec),
                      sha256=hashlib.sha256(spec).hexdigest())
        wrong=[args['binaries'][0]['path'],'--bundle',args['run']+'/bundle.rcv6',
               '--report',args['run']+'/capture.json']
        with self.assertRaises(ValueError):
            constructors.command_freeze(**args,instance=instance,
                capture_command=wrong,embedded_source_bytes=1)
        frozen=self.frozen()
        self.assertEqual(frozen['capture_command'][1::2],
                         ['--bundle-output','--report-output'])
        p=args['evidence'][0]
        decision=dict(maxima=args['maxima'],source_commit=args['source_commit'],
            command=['rtk','proxy','guard'],inner_command=frozen['capture_command'],
            freeze=p,exact_commands_pin=p,ready=p,execution_helper=p,live_bytes=0)
        result=json.loads(constructors.capture_execution_decision(**decision))
        self.assertEqual(result['inner_command'],frozen['capture_command'])
        decision['inner_command']=wrong
        with self.assertRaises(ValueError):
            constructors.capture_execution_decision(**decision)

    def test_freeze_retains_exact_original_fields_full_pin_order_and_sums(self):
        value=self.frozen();args=self.inputs()
        self.assertEqual(set(value),{'schema','profile','run','source_commit','cwd',
            'environment','capture_command','pins','pin_count','pinned_file_bytes',
            'source_count','complete_source_bytes','embedded_source_bytes','copy_metadata','accepted'})
        self.assertEqual(value['pins'][:101],args['sources']+args['tool_sources'])
        self.assertEqual(value['pins'][101:103],args['binaries'])
        self.assertEqual(value['pins'][104:106],args['evidence'])
        self.assertEqual(value['pinned_file_bytes'],sum(p['bytes'] for p in value['pins']))
        self.assertEqual(value['pin_count'],len(value['pins']))
        self.assertFalse(value['accepted'])

    def test_exact_commands_retains_all24_unchanged_maxima_and_stays_pending(self):
        freeze=self.frozen();raw=core.canonical(freeze)
        pin=dict(path=freeze['run']+'/freeze.json',bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        outcomes=[dict(cap_bytes=cap,group='outcome24',index=i,
            path=freeze['run']+'/outcome-'+str(i),schema_id='fixture')
            for i,cap in enumerate(constructors.ROLE_MAXIMA[57:])]
        args=dict(maxima=constructors.ROLE_MAXIMA,freeze=freeze,freeze_pin=pin,
            build=['build'],capture=['capture'],lead_replay=['lead'],independent_replay=['independent'],
            outcomes=outcomes,live_bytes=0)
        result=json.loads(constructors.exact_commands(**args))
        self.assertFalse(result['ready']);self.assertFalse(result['captured'])
        self.assertEqual(result['outcome_full_reservation'],2523136)
        self.assertEqual(result['inner_capture'],freeze['capture_command'])
        outcomes[23]['cap_bytes']+=1
        with self.assertRaises(ValueError):constructors.exact_commands(**args)

    def test_caller_failure_precedes_output_allocation(self):
        from unittest.mock import patch
        args=self.inputs();args['live_bytes']=core.SEMANTIC
        with patch.object(constructors.core,'canonical',side_effect=AssertionError('allocated')):
            with self.assertRaisesRegex(ValueError,'caller lifetime'):
                constructors.instance_spec(56,**args)

    def test_candidate_review_reconstructs_complete_actual_inputs(self):
        args=CandidateConstructorTests().inputs()
        raw=constructors.selected40_candidate(**args)
        candidate=json.loads(raw);cp=dict(path='/home/fry/artifacts/candidate-fixture',
            bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        inputs={k:v for k,v in args.items() if k not in ('maxima','live_bytes')}
        review=constructors.candidate_review(maxima=constructors.ROLE_MAXIMA,candidate=candidate,
            candidate_pin=cp,preparation=cp,candidate_inputs=inputs,live_bytes=0)
        value=json.loads(review)
        self.assertEqual(value['candidate_tuples'],40)
        self.assertEqual(value['bundle_proofs'],91)
        self.assertEqual(value['residual_if_later_proven'],69)
        candidate['full_tuples'][39]['request_index']+=1
        with self.assertRaises(ValueError):
            constructors.candidate_review(maxima=constructors.ROLE_MAXIMA,candidate=candidate,
                candidate_pin=cp,preparation=cp,candidate_inputs=inputs,live_bytes=0)

    def native_inputs(self):
        # These are constructor inputs, not a claim that native E946 ran.
        args=CandidateConstructorTests().inputs();slot=args['slot']
        original=json.loads(Path('/home/fry/artifacts/vrf-round3-01a141ba-20261016/next42/preflight.json').read_bytes())
        selection=[]
        for row in slot['rows']:
            t=row['full_tuple']
            selection.append(dict(parent_manifest_index=row['parent_manifest_index'],
                source_row=dict(delayed_period=t['period'],proof=None,qualified=False,
                    semantic_key='fe_vrf_00_04_address',source_references=row['source_references'],
                    value=None,voter_hex=t['voter_hex']),
                tuple=dict(identity=dict(period=t['period'],state_root_hex=t['state_root_hex']),
                    address_hex=t['account_hex'],logical_key_hex=t['logical_key_hex'],
                    key_preimage_hex=t['key_preimage_hex'])))
        prior=args['prior_report'];chosen=prior['selection'][17];value=prior['values'][17]
        original['authority']['prior_first_d']['selected']=dict(
            identity=chosen['tuple']['identity'],proof_index=value['proof_index'],
            usable_vrf_key=value['usable_vrf_key'],value_hex=value['value_hex'],voter_hex=slot['voter'])
        bounds=original['bounds'];bounds['payload_tuples_exact']=40;bounds['proofs_exact']=91
        context={k:original[k] for k in ('head','parent','delayed','genesis_hash_hex','head_header_sha256')}
        inputs=dict(maxima=constructors.ROLE_MAXIMA,selection=selection,authority=original['authority'],
            vrf_sources=original['vrf_sources'],bounds=bounds,slot=slot,context=context,live_bytes=0)
        return inputs,original

    def test_native_selection_digest_matches_retained_rust_report(self):
        original=json.loads(Path('/home/fry/artifacts/vrf-round3-01a141ba-20261016/next42/preflight.json').read_bytes())
        self.assertEqual(constructors._native_selection_digest(original['selection']),
                         original['selection_sha256'])
        self.assertNotEqual(hashlib.sha256(core.canonical(original['selection'])).hexdigest(),
                            original['selection_sha256'])
        inputs,_=self.native_inputs()
        actual=json.loads(constructors.native_window_report(54,**inputs))
        self.assertEqual(actual['selection_sha256'],
                         constructors._native_selection_digest(inputs['selection']))

    def test_native_preflight_constructs_all_actual_tuple_fields_and_retains_authority(self):
        inputs,original=self.native_inputs();selection=inputs['selection']
        raw=constructors.native_window_report(54,**inputs)
        output=json.loads(raw)
        self.assertEqual(output['authority'],original['authority'])
        self.assertEqual(output['selection'],selection)
        self.assertEqual(output['bounds']['proofs_exact'],91)
        self.assertFalse(any(output['qualification'].values()))
        self.assertLessEqual(len(raw),constructors.ROLE_MAXIMA[54])
        selection[39]['tuple']['logical_key_hex']='0'*64
        with self.assertRaisesRegex(ValueError,'tuple binding'):
            constructors.native_window_report(54,**inputs)

    def test_native_capture_and_both_replay_reports_keep_full_values_and_original_base(self):
        inputs,original=self.native_inputs();inputs.pop('context')
        # Explicit unit fixture: this does not assert native proof execution.
        p=dict(path='/home/fry/artifacts/fixed40-unit-fixture/bundle',bytes=1,sha256='a'*64)
        actual=dict(admitted_input_bytes=1,resources=[dict(resource=n,used=0)
                     for n,_ in constructors.NATIVE_RESOURCE_LIMITS])
        capture=json.loads(constructors.native_window_report(69,**inputs,bundle=p,actual=actual))
        self.assertEqual(capture['authority'],original['authority'])
        self.assertEqual(capture['actual'],actual)
        self.assertFalse(any(capture['qualification'].values()))
        values=[dict(logical='member',physical=dict(kind='present',bytes=32,sha256='a'*64),
            proof_index=51+i,usable_vrf_key=True,value_hex='1'*64) for i in range(40)]
        lead=constructors.native_window_report(72,**inputs,bundle=p,actual=actual,values=values)
        self.assertEqual(lead,constructors.native_window_report(75,**inputs,bundle=p,actual=actual,values=values))
        self.assertEqual(json.loads(lead)['values'],values)
        values[39]['proof_index']=90-1
        with self.assertRaisesRegex(ValueError,'value binding'):
            constructors.native_window_report(72,**inputs,bundle=p,actual=actual,values=values)

    def test_native_bundle_authenticates_full_original_stream_without_proof_grant(self):
        raw=b'RCV6'+b'unit fixture, no witness proofs'
        p=dict(path='/home/fry/artifacts/fixed40-unit-fixture/bundle',bytes=len(raw),
               sha256=hashlib.sha256(raw).hexdigest())
        class Reader:
            def __init__(self):self.calls=[]
            def consume(self,*args,**kwargs):self.calls.append((args,kwargs))
        reader=Reader();declaration=(1,(p['path'],p['bytes'],p['sha256']),'evidence','bytes')
        result=constructors.native_bundle(raw,maxima=constructors.ROLE_MAXIMA,
            bundle_pin=p,unit=reader,declaration=declaration,live_bytes=0)
        self.assertIs(result,raw);self.assertEqual(len(reader.calls),1)
        with self.assertRaises(ValueError):
            constructors.native_bundle(raw+b'x',maxima=constructors.ROLE_MAXIMA,
                bundle_pin=p,unit=reader,declaration=declaration,live_bytes=0)

    def test_full_independent_review_and_all_body_reads_stay_pending(self):
        raw=b'Complete independent unit fixture review\nfull retained diagnostic\n'
        p=dict(path='/home/fry/artifacts/fixed40-unit-fixture/review',bytes=len(raw),
               sha256=hashlib.sha256(raw).hexdigest())
        class Reader:
            def __init__(self):
                self.calls=[]
                self.invocation=SimpleNamespace(metadata_charge=lambda:4096)
                self.declaration_charge=2048
            def consume(self,*args,**kwargs):self.calls.append((args,kwargs))
        for role,names in ((60,('exact_commands','command_verification','freeze','execution_helper')),
                           (79,('lead_verification','independent_verification','reverse_outcome','fact_seal_inputs'))):
            evidence={n:dict(p,path=p['path']+'/'+n) for n in names};pins=[p,*evidence.values()]
            declarations=[(i,(q['path'],q['bytes'],q['sha256']),'evidence','bytes') for i,q in enumerate(pins)]
            reader=Reader();output=constructors.independent_review_document(role,
                maxima=constructors.ROLE_MAXIMA,review_raw=raw,review_pin=p,
                evidence=evidence,unit=reader,declarations=declarations,live_bytes=0)
            self.assertEqual(output.count(raw),1);self.assertEqual(len(reader.calls),5)
            self.assertTrue(output.startswith(b'PENDING\n'))
            self.assertNotIn(b'\nREADY\n',output)

    def test_storage_component_retains_every_original_failure_and_scope_field(self):
        original=json.loads(Path('/home/fry/artifacts/vrf-round3-01a141ba-20261016/source/storage-validation-current.json').read_bytes())
        names=('accepted_source_certificate','source_freeze','source_scope_mapping','original_gates',
               'tracked_build_dependency_proof','failure_retention_amendment')
        raw=constructors.storage_validation(maxima=constructors.ROLE_MAXIMA,live_bytes=0,
            **{n:original[n] for n in names})
        output=json.loads(raw)
        for n in names:self.assertEqual(output[n],original[n])
        self.assertFalse(output['ready']);self.assertFalse(output['accepted'])
        self.assertFalse(output['current_artifact_identity_claimed'])

    def test_source_mapping_preserves_original_replay_and_build_proof_bodies(self):
        original=json.loads(Path('/home/fry/artifacts/vrf-round3-01a141ba-20261016/source/source-successor-mapping.json').read_bytes())
        current=self.inputs();base=Path('/home/fry/artifacts/vrf40-admission-01a14b66-20261018')
        old=base/'recovery46-rust-source-freeze.json';raw=old.read_bytes()
        review=base/'native-ten-frozen-acceptance.json';rr=review.read_bytes()
        def pinned(path,body):return dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest())
        names=('cargo_inputs','original_gate_pins','replay_input_proof','tracked_dependency_proof','unresolved')
        # Historical Cargo pins changed before the accepted current Rust90
        # component. Bind all four current inputs from that exact closure.
        cargo=[]
        for old_pin in original['cargo_inputs']:
            current_pin=next(p for p in current['sources']
                if p['path']=='/workspaces/rustaxa-evm/'+old_pin['path'])
            cargo.append(dict(current_pin,path=old_pin['path']))
        original['cargo_inputs']=cargo
        output=json.loads(constructors.source_successor_mapping(maxima=constructors.ROLE_MAXIMA,
            accepted_freeze_raw=raw,accepted_freeze=pinned(old,raw),accepted_review=pinned(review,rr),
            source_commit=current['source_commit'],sources=current['sources'],tool_sources=current['tool_sources'],
            source_freeze=pinned(old,raw),tool_source_freeze=pinned(review,rr),
            historical_sources=json.loads(raw),
            replay_input_source_raw=Path(original['replay_input_proof']['source_freeze']['path']).read_bytes(),
            live_bytes=0,**{n:original[n] for n in names}))
        for n in names:self.assertEqual(output[n],original[n])
        self.assertEqual(output['reusable_source_gate_roles'],[])
        self.assertFalse(output['whole_current_python_gate_reused'])
        self.assertEqual(output['source_list_identity']['count'],90)
        self.assertEqual(output['historical_replay_input_source'],original['replay_input_proof']['source_freeze'])
        self.assertEqual(output['current_gate_state'],'PROSPECTIVE_NOT_EXECUTED_NOT_ACCEPTED')
        self.assertTrue(output['current_source_changed'])

    def test_authority_retains_exact_parent_bytes_and_matches_reviewed_lossless_proposal(self):
        parent=constructors.AUTHORITY_PARENT;raw=Path(parent['path']).read_bytes()
        class Reader:
            def __init__(self):
                from types import SimpleNamespace
                self.calls=[];self.declaration_charge=0
                self.invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:0)
            def consume(self,occurrence,pin,**kwargs):
                constructors.records.authenticate(dict(role='unit-original-full-body',**pin))
                self.calls.append((occurrence,pin))
        mapping=fixed.MAPPING;reader=Reader();prepaid=[]
        def charge_projection(packet):
            self.assertEqual(len(reader.calls),2)
            prepaid.append(packet)
        def declaration(i,p):return (i,(p['path'],p['bytes'],p['sha256']),'evidence','bytes')
        args=dict(maxima=constructors.ROLE_MAXIMA,predecessor_raw=raw,unit=reader,
            parent_declaration=declaration(1,parent),mapping_declaration=declaration(2,mapping),
            projection_read_charge=charge_projection,live_bytes=0)
        output=constructors.successor_contract(**args)
        expected=Path('/home/fry/artifacts/vrf40-development-20261018/authority172-verbatim-parent-and-slot-proposed.json').read_bytes()
        self.assertEqual(output,expected)
        self.assertEqual(output.count(raw),1)
        self.assertEqual(len(reader.calls),2)
        self.assertEqual(prepaid,[(mapping,)])
        value=json.loads(output)
        self.assertFalse(value['accepted']);self.assertFalse(value['ready'])
        self.assertEqual(value['predecessor_contract'],json.loads(raw))
        self.assertLessEqual(len(output),constructors.ROLE_MAXIMA[40])
        args['predecessor_raw']=raw[:-1]+b' '
        with self.assertRaisesRegex(ValueError,'entire exact'):
            constructors.successor_contract(**args)

    def test_authority_projects_before_retaining_parent_in_prescribed_order(self):
        from types import SimpleNamespace
        parent=constructors.AUTHORITY_PARENT;raw=Path(parent['path']).read_bytes();events=[]
        class Reader:
            declaration_charge=0
            invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:0)
            def consume(self,occurrence,pin,**kwargs):
                constructors.records.authenticate(dict(role='unit-original-full-body',**pin))
                events.append(occurrence)
                if kwargs.get('retain'):return Path(pin['path']).read_bytes()
        def declaration(i,p):return (i,fixed._pin_tuple(p),'evidence','bytes')
        def projection(packet):
            self.assertEqual(packet,(fixed.MAPPING,));events.append('projection')
        output=constructors.successor_contract(maxima=constructors.ROLE_MAXIMA,
            predecessor_raw=None,unit=Reader(),parent_declaration=declaration(1,parent),
            mapping_declaration=declaration(2,fixed.MAPPING),projection_read_charge=projection,
            live_bytes=653856)
        self.assertEqual(events,['projection',1,2])
        self.assertEqual(output.count(raw),1)
        expected=Path('/home/fry/artifacts/vrf40-development-20261018/authority172-verbatim-parent-and-slot-proposed.json').read_bytes()
        self.assertEqual(output,expected)

    def test_command_verification_keeps_full_actual_ledgers_without_ready(self):
        value=self.frozen();raw=core.canonical(value)
        pin=dict(path=value['run']+'/freeze',bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        ledger=dict(actual_binary_bytes=2,actual_bytes=5,actual_nonbinary_bytes=3,
            actual_source_bytes=1,charged_nonbinary_bytes=2523139,charged_total_bytes=2523141,
            open_occurrences=1,pending_and_outcome_reserved_bytes=2523136)
        args=dict(maxima=constructors.ROLE_MAXIMA,freeze=pin,exact_commands_pin=pin,
            source_commit=value['source_commit'],rebuilt_freeze_raw=raw,actual_freeze_raw=raw,
            actual_reads=ledger,phase_peaks={'fixture':1},
            history=dict(body_bytes=1,charged_bytes=1,containers=5,members=209),
            copy_metadata=value['copy_metadata'],child_occurrences=64,strict_pending=0,live_bytes=0)
        output=json.loads(constructors.command_verification(**args))
        self.assertEqual(output['actual_reads'],ledger)
        self.assertTrue(output['deterministic_byte_equal']);self.assertFalse(output['ready'])
        args['actual_freeze_raw']=raw+b'x'
        with self.assertRaises(ValueError):constructors.command_verification(**args)

    def test_execution_decision_encodes_all_one_use_inputs_without_execution_grant(self):
        value=self.frozen();pin=value['pins'][-1]
        output=json.loads(constructors.capture_execution_decision(maxima=constructors.ROLE_MAXIMA,
            source_commit=value['source_commit'],command=['guard'],inner_command=value['capture_command'],
            freeze=pin,exact_commands_pin=pin,ready=pin,execution_helper=pin,live_bytes=0))
        self.assertEqual(output['accepted_facts_before'],805)
        self.assertEqual(output['residual_before'],109)
        self.assertTrue(output['no_retry'])
        self.assertEqual(output['status'],'COMPONENT_PENDING_INDEPENDENT_READY_AND_GUARD_ADMISSION')

    def test_lead_requires_full_independent_execution_proof_body(self):
        raw=b'{}\n';names=('capture_report','lead_report','independent_report','independent_verification','bundle')
        pins={n:dict(path='/home/fry/artifacts/fixed40-unit-fixture/'+n,bytes=len(raw),
            sha256=hashlib.sha256(raw).hexdigest()) for n in names}
        class Reader:
            def __init__(self):
                from types import SimpleNamespace
                self.declaration_charge=0
                self.invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:0)
            def consume(self,*args,**kwargs):pass
        declarations=[(i,(pins[n]['path'],pins[n]['bytes'],pins[n]['sha256']),'evidence','bytes')
                      for i,n in enumerate(names[:-1])]
        metadata=self.inputs()['copy_metadata']
        with self.assertRaisesRegex(ValueError,'complete native replay report fields'):
            constructors.lead_verification(maxima=constructors.ROLE_MAXIMA,slot={},capture_raw=raw,
                lead_raw=raw,independent_raw=raw,independent_verification_raw=raw,pins=pins,
                unit=Reader(),declarations=declarations,before=metadata,after=metadata,
                phase_peaks={},live_bytes=0)

    def test_lead_nested_native_builder_keeps_decoded_root_in_caller(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        inputs,_=self.native_inputs();inputs.pop('context')
        bundle=dict(path='/home/fry/artifacts/NONHISTORICAL-format-fixture/bundle',
                    bytes=1,sha256='a'*64)
        actual=dict(admitted_input_bytes=1,resources=[dict(resource=n,used=0)
            for n,_ in constructors.NATIVE_RESOURCE_LIMITS])
        capture=constructors.native_window_report(69,**inputs,bundle=bundle,actual=actual)
        values=[dict(logical='member',physical=dict(kind='present',bytes=32,sha256='a'*64),
            proof_index=51+i,usable_vrf_key=True,value_hex='1'*64) for i in range(40)]
        lead=constructors.native_window_report(72,**inputs,bundle=bundle,actual=actual,values=values)
        bodies=(capture,lead,lead,bytes((123,125,10)))
        names=('capture_report','lead_report','independent_report','independent_verification')
        pins={n:dict(path=bundle['path']+'/'+n,bytes=len(raw),
                    sha256=hashlib.sha256(raw).hexdigest()) for n,raw in zip(names,bodies)}
        pins['bundle']=bundle
        declarations=[(i,(pins[n]['path'],pins[n]['bytes'],pins[n]['sha256']),'evidence','bytes')
                      for i,n in enumerate(names)]
        metadata=self.inputs()['copy_metadata']
        caller=fixed.graph_charge((inputs['slot'],bodies,pins,declarations,metadata,metadata,{}))
        minimum=caller+fixed.graph_charge(json.loads(lead))
        class Reader:
            declaration_charge=0
            invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:0)
            def consume(self,*args,**kwargs):pass
        class Observed(Exception):pass
        def observe(*args,**kwargs):
            self.assertGreaterEqual(kwargs['live_bytes'],minimum)
            raise Observed()
        with patch.object(constructors,'native_window_report',side_effect=observe):
            with self.assertRaises(Observed):
                constructors.lead_verification(maxima=constructors.ROLE_MAXIMA,
                    slot=inputs['slot'],capture_raw=capture,lead_raw=lead,independent_raw=lead,
                    independent_verification_raw=bytes((123,125,10)),pins=pins,unit=Reader(),
                    declarations=declarations,before=metadata,after=metadata,
                    phase_peaks={},live_bytes=0)

    def test_reverse_cannot_substitute_an_old_42_result_or_drop_original_bodies(self):
        with self.assertRaisesRegex(ValueError,'exact complete reverse source bodies'):
            constructors.reverse_outcome_record(maxima=constructors.ROLE_MAXIMA,
                predecessor_raw=b'{}',matrix_raw=b'{}',runs=[],slot={},historical_fbb_report_raw=b'',
                binary_before=[],binary_after=[],current_pins=[],source_commit='1'*40,
                capture_parent={},read_accounting={},semantic_peaks={},source_provenance='',
                unit=None,declarations=[],live_bytes=0)

    def test_reverse_history_preserves22_nonbinary_routes_and_all24_metadata(self):
        import copy
        original=json.loads(Path('/home/fry/artifacts/vrf-round3-01a141ba-20261016/next42/reverse-outcome-record.json').read_bytes())
        boundary=original['historical_boundary']
        rows=constructors.reverse_historical_body_pins(boundary)
        self.assertEqual(rows[4:],boundary['full_nonbinary_routes'])
        self.assertEqual(len(boundary['original_ordered24_pins']),24)
        self.assertFalse(any(p in rows for p in boundary['original_ordered24_pins'][:2]))
        for change in ('fresh','missing','order'):
            bad=copy.deepcopy(boundary)
            if change=='fresh':bad['fresh_historical_binary_validation']=True
            elif change=='missing':bad['full_nonbinary_routes']=bad['full_nonbinary_routes'][:-1]
            else:bad['original_ordered24_pins'][2:4]=reversed(bad['original_ordered24_pins'][2:4])
            with self.assertRaises(ValueError):
                constructors.reverse_historical_body_pins(bad)

    def test_reverse_binary_positions_cannot_be_reclassified_as_evidence(self):
        parent=json.loads(Path(constructors.REVERSE_PREDECESSOR['path']).read_bytes())
        historical=constructors.reverse_historical_body_pins(parent['historical_boundary'])
        expected=[constructors.REVERSE_PREDECESSOR,constructors.REVERSE_MATRIX,*historical,
                  parent['outcomes'][1]['report'],*parent['pins']]
        start=2+len(historical)+1
        declarations=[(i,(p['path'],p['bytes'],p['sha256']),
            'binary' if start<=i<start+2 else 'evidence','bytes')
            for i,p in enumerate(expected)]
        constructors._reverse_declaration_kinds(expected,declarations,start)
        for i in (0,start,start+1,start+2):
            bad=list(declarations);row=list(bad[i])
            row[2]='evidence' if row[2]=='binary' else 'binary';bad[i]=tuple(row)
            with self.assertRaisesRegex(ValueError,'body/binary'):
                constructors._reverse_declaration_kinds(expected,bad,start)

    def test_reverse_complete53_declaration_prefix_reaches_actual_run_validation(self):
        from types import SimpleNamespace
        raw=Path(constructors.REVERSE_PREDECESSOR['path']).read_bytes()
        matrix=Path(constructors.REVERSE_MATRIX['path']).read_bytes()
        parent=json.loads(raw);historical=constructors.reverse_historical_body_pins(parent['historical_boundary'])
        old_report=parent['outcomes'][1]['report'];report_raw=Path(old_report['path']).read_bytes()
        current=parent['pins']
        expected=[constructors.REVERSE_PREDECESSOR,constructors.REVERSE_MATRIX,
                  *historical,old_report,*current]
        start=len(expected)-len(current)
        declarations=[(i,(p['path'],p['bytes'],p['sha256']),
            'binary' if start<=i<start+2 else 'evidence','bytes') for i,p in enumerate(expected)]
        self.assertEqual((len(declarations),start-1,start),(53,28,29))
        class Reader:
            def __init__(self):
                self.calls=[];self.declaration_charge=0
                self.invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:0)
            def consume(self,occurrence,pin,**kwargs):self.calls.append((occurrence,pin,kwargs))
        reader=Reader()
        # Layout-only NONHISTORICAL unit: no file body or binary is consumed by
        # the stub and missing actual run evidence must still reject the record.
        with self.assertRaisesRegex(ValueError,'complete actual reverse run fields'):
            constructors.reverse_outcome_record(maxima=constructors.ROLE_MAXIMA,
                predecessor_raw=raw,matrix_raw=matrix,runs=[{} for _ in range(5)],slot={},
                historical_fbb_report_raw=report_raw,binary_before=[],binary_after=[],
                current_pins=current,source_commit='1'*40,capture_parent=parent['capture_parent'],
                read_accounting={},semantic_peaks={},source_provenance={},unit=reader,
                declarations=declarations,live_bytes=0)
        self.assertEqual(len(reader.calls),53)
        self.assertEqual(sum(c[2]['kind']=='binary' for c in reader.calls),2)

    def test_closed_review_format_preserves_actual_verdict_but_has_no_runtime_authority(self):
        from types import SimpleNamespace
        sources=self.inputs();source_rows=sources['sources']+sources['tool_sources']
        original=json.loads(Path(constructors.AUTHORITY_PARENT['path']).read_bytes())
        # NONHISTORICAL format fixtures. Their nonexistent evidence paths and
        # invalid content hashes cannot pass the real runtime source/proof
        # consumer. No READY or accepted evidence file is created by this test.
        def p(name):return dict(path='/home/fry/artifacts/NONHISTORICAL-format-fixture/'+name,
            bytes=0,sha256='a'*64)
        for role,verdict,names,proof_names in (
            (60,'READY',('exact_commands','command_verification','freeze','execution_helper'),
             ('preflight','candidate_review','candidate','prior_bundle','prior_report','source_review',
              'command_verification','command_closure','normal_hook','gate_equivalence')),
            (79,'ACCEPT_FINAL_FAMILY',('lead_verification','independent_verification','reverse_outcome','fact_seal_inputs'),
             ('bundle','capture_report','lead_report','independent_report','independent_verification',
              'lead_verification','reverse_outcome','fact_seal_inputs'))):
            evidence={n:p(n) for n in names}
            gates=[dict(role=r,result=p('result-'+str(r)),log=p('log-'+str(r))) for r in constructors.VALIDATION_RESULTS]
            proof={n:p(n) for n in proof_names}
            result=dict(schema='fixed40-independent-review-result-v1',role=role,
                profile=constructors.FIXED40_PROFILE,verdict=verdict,evidence=evidence,
                source_freeze=source_rows,required_gates=gates,record_bounds=original['record_bounds'],
                role_maxima=list(constructors.ROLE_MAXIMA),proofs=proof,
                body='NONHISTORICAL FORMAT ONLY; NO RUNTIME AUTHORITY. Full diagnostic retained once.')
            raw=core.canonical(result);rp=dict(p('review'),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
            pins=[rp,*evidence.values(),*source_rows,*[q for g in gates for q in (g['result'],g['log'])],*proof.values()]
            declarations=[(i,(q['path'],q['bytes'],q['sha256']),
                'source' if 5<=i<106 else 'evidence','bytes') for i,q in enumerate(pins)]
            class Reader:
                def __init__(self):
                    self.calls=[];self.declaration_charge=0
                    self.invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:0)
                def consume(self,*args,**kwargs):self.calls.append((args,kwargs))
            reader=Reader()
            output=constructors.independent_review_document(role,maxima=constructors.ROLE_MAXIMA,
                review_raw=raw,review_pin=rp,evidence=evidence,unit=reader,declarations=declarations,live_bytes=0)
            self.assertEqual(output,verdict.encode()+b'\n'+raw)
            self.assertEqual(len(reader.calls),len(pins))
            retained_minimum=fixed.graph_charge((raw,result,evidence))
            self.assertTrue(all(call[1]['live_bytes']>=retained_minimum
                                for call in reader.calls))
            self.assertEqual(result['body'].count('Full diagnostic'),1)
            self.assertNotIn('accepted',result)
            result['required_gates'].pop();bad=core.canonical(result)
            with self.assertRaisesRegex(ValueError,'all14'):
                constructors.independent_review_document(role,maxima=constructors.ROLE_MAXIMA,
                    review_raw=bad,review_pin=dict(rp,bytes=len(bad),sha256=hashlib.sha256(bad).hexdigest()),
                    evidence=evidence,unit=Reader(),declarations=declarations,live_bytes=0)

    def test_independent_renderer_requires_full_actual_control_and_source_inputs(self):
        with self.assertRaisesRegex(ValueError,'complete independent native/control/source pins'):
            constructors.independent_replay_verification(maxima=constructors.ROLE_MAXIMA,
                slot={},review_raw=b'{}',capture_raw=b'{}',lead_raw=b'{}',replay_raw=b'{}',
                command_raw=b'{}',log_raw=b'',source_raw=b'[]',tools_raw=b'[]',freeze_raw=b'{}',
                live_raw=b'{}',pins={},unit=None,declarations=[],native_declarations=[],
                phase_peaks={},live_bytes=0)


class SavedEquivalenceConstructorTests(unittest.TestCase):
    def inputs(self, *, historical_scope=False):
        acceptance_raw=(OLD/'native-ten-frozen-acceptance.json').read_bytes()
        acceptance=json.loads(acceptance_raw)
        freeze_raw=(OLD/'recovery46-rust-source-freeze.json').read_bytes()
        frozen=json.loads(Path(os.environ['BRIDGE_SOURCE_FREEZE']).read_bytes())
        current=[{k:p[k] for k in ('path','bytes','sha256')} for p in frozen['files'][:90]]
        if historical_scope:
            # Exercise the exact old component, without claiming current Rust
            # is unchanged. The separate current-source test must reject reuse.
            current=json.loads(freeze_raw)
        def original_pin(name,key):
            return dict(path=str(ARTIFACTS/name),
                **{k:acceptance[key][k] for k in ('bytes','sha256')})
        return dict(maxima=constructors.ROLE_MAXIMA,acceptance_raw=acceptance_raw,
            old_source_raw=freeze_raw,current_sources=current,
            source_freeze=pin(OLD/'recovery46-rust-source-freeze.json'),
            gate_result_pin=original_pin('NONHISTORICAL-result-fixture','result'),
            gate_log_pin=original_pin('NONHISTORICAL-log-fixture','full_log'),live_bytes=0)

    def test_exact_component_reuse_keeps_ten_and19_without_python_acceptance(self):
        value=json.loads(constructors.saved_equivalence(**self.inputs(historical_scope=True)))
        self.assertEqual(value['matched_tests'],10)
        self.assertEqual(value['reports']['count'],19)
        self.assertFalse(value['whole_current_python_gate_reused'])
        self.assertFalse(value['accepted'])
        self.assertEqual(value['new_facts'],0)

    def test_source_roster_pin_substitution_cannot_reuse_component(self):
        args=self.inputs();args['source_freeze']['sha256']='0'*64
        with self.assertRaisesRegex(ValueError,'source roster pin'):
            constructors.saved_equivalence(**args)

    def test_source_or_gate_drift_cannot_reuse_component(self):
        args=self.inputs(historical_scope=True);args['current_sources'][0]['sha256']='0'*64
        with self.assertRaisesRegex(ValueError,'source scope'):
            constructors.saved_equivalence(**args)
        args=self.inputs(historical_scope=True);args['gate_log_pin']['sha256']='0'*64
        with self.assertRaisesRegex(ValueError,'ten-test/19-report'):
            constructors.saved_equivalence(**args)

    def test_changed_current_rust_cannot_reuse_the_historical_component(self):
        args=self.inputs()
        self.assertNotEqual(args['current_sources'],json.loads(args['old_source_raw']))
        with self.assertRaisesRegex(ValueError,'unchanged complete90 Rust/Cargo source scope'):
            constructors.saved_equivalence(**args)


class ProspectiveEncoderComponentTests(unittest.TestCase):
    def test_gate_model_keeps_all_fields_full_maximum_and_no_execution_claim(self):
        from types import SimpleNamespace
        def p(name):return dict(path='/home/fry/artifacts/NONHISTORICAL-'+name,bytes=17,sha256='a'*64)
        pins=[p('source'),p('planned-command'),p('prospective-log')]
        calls=[]
        unit=SimpleNamespace(invocation=SimpleNamespace(metadata_charge=lambda:0),declaration_charge=0,
            consume=lambda occurrence,pin,**kwargs:calls.append((occurrence,pin,kwargs)))
        declarations=[(i,fixed._pin_tuple(pin),'evidence','bytes') for i,pin in enumerate(pins)]
        raw=constructors.prospective_validation_result(0,maxima=constructors.ROLE_MAXIMA,
            source_snapshot=pins[0],command_record=pins[1],log=pins[2],command=['false'],
            cwd='/workspaces/rustaxa-evm',unit=unit,declarations=declarations,live_bytes=0)
        value=json.loads(raw)
        self.assertLessEqual(len(raw),constructors.ROLE_MAXIMA[0])
        self.assertEqual(raw,core.canonical(value))
        self.assertEqual(set(value),{'command','cwd','exit_code','finished_utc','seconds','log',
            'role','source_snapshot','command_record','cache_status','cache_provenance'})
        self.assertEqual(value['cache_status'],constructors.PROSPECTIVE_MODE)
        self.assertEqual(value['finished_utc'],constructors.PROSPECTIVE_MODE)
        self.assertEqual([row[0] for row in calls],[0,1,2])
        unit.invocation.metadata_charge=lambda:core.SEMANTIC
        with self.assertRaisesRegex(ValueError,'input preallocation'):
            constructors.prospective_validation_result(0,maxima=constructors.ROLE_MAXIMA,
                source_snapshot=pins[0],command_record=pins[1],log=pins[2],command=['false'],
                cwd='/workspaces/rustaxa-evm',unit=unit,declarations=declarations,live_bytes=0)

    def test_native_capacity_model_preserves_fields_and_cannot_qualify_proofs(self):
        # Source-format component only; no native command or physical proof ran.
        inputs,_=FullInstanceConstructorTests().native_inputs()
        value=constructors._native_window_value(54,prospective=True,**inputs)
        self.assertEqual(value['schema'],constructors.PROSPECTIVE_MODE)
        self.assertEqual(value['status'],constructors.PROSPECTIVE_MODE)
        self.assertFalse(any(value['qualification'].values()))
        self.assertEqual(len(value['selection']),40)
        self.assertEqual(value['bounds']['proofs_exact'],91)
        self.assertEqual(value['authority'],inputs['authority'])


    def test_complete_native_producer_consumer_reads_all_inputs_for_every_role(self):
        # NONHISTORICAL source-format fixtures. No executable, DB, witness
        # verification, physical proof, READY or fact acceptance occurs here.
        from types import SimpleNamespace
        args,original_native=FullInstanceConstructorTests().native_inputs()
        del original_native
        args.pop('live_bytes');args.pop('maxima')
        root=Path(os.environ['DEVELOPMENT_TEST_ARTIFACT_ROOT'])/'NONHISTORICAL-native-consumer'
        root.mkdir(parents=True,exist_ok=False)
        source=root/'source.rs';source.write_bytes(b'// NONHISTORICAL source-format fixture\n')
        source_pin=dict(path=str(source),bytes=source.stat().st_size,
            sha256=hashlib.sha256(source.read_bytes()).hexdigest())
        for rows in (args['vrf_sources'],args['authority']['six_input_owner']['sources'],
                     args['authority']['six_input_owner']['aspen_sources']):
            for i in range(len(rows)):rows[i]=dict(source_pin)
        observations=[]
        for group,rows in (('vrf_sources',args['vrf_sources']),
                ('owner_sources',args['authority']['six_input_owner']['sources']),
                ('aspen_sources',args['authority']['six_input_owner']['aspen_sources'])):
            observations.extend(dict(group=group,original=dict(row),canonical=dict(row),
                direct_frozen=True) for row in rows)
        frozen=[dict(source_pin) for _ in range(101)]
        pins=[*args['authority']['six_input_owner']['inputs'],
              args['authority']['prior_first_d']['input'],
              *[row['canonical'] for row in observations]]
        declarations=[(i,fixed._pin_tuple(pin),'evidence' if i<7 else 'source','bytes')
            for i,pin in enumerate(pins)]
        # Separate finite64MiB subdivision of the existing509609576-byte
        # full engineering-pass hold; old8MiB packets remain unchanged.
        native_used=0
        native_journal=root/'before-open-read-debits.jsonl'
        def charge_native(occurrence,pin,kind):
            nonlocal native_used
            amount=pin['bytes']+1
            if native_used+amount>67108864:
                raise ValueError('finite native full-consumer test read packet')
            row=dict(occurrence=occurrence,pin=pin,kind=kind,bytes_plus_EOF=amount,
                cumulative=native_used+amount,read_limit=67108864,
                phase='BEFORE_OPEN',scope='NONHISTORICAL format test; historical data inputs; binary0',
                full_pass_hold=509609576,source=os.environ['BRIDGE_SOURCE_FREEZE'])
            with native_journal.open('ab') as out:
                out.write(core.canonical(row));out.flush();os.fsync(out.fileno())
            native_used+=amount
        class Reader:
            declaration_charge=0
            invocation=SimpleNamespace(operations=bytearray(2048),metadata_charge=lambda:196608)
            def __init__(self):self.calls=[]
            def consume(self,occurrence,pin,**kwargs):
                charge_native(occurrence,pin,kwargs['kind'])
                # Native raw inputs are indivisible original bodies, not
                # reference records subject to the separate1MiB wrapper cap.
                fixed._pin_tuple(pin)
                digest=hashlib.sha256();length=0
                with open(pin['path'],'rb') as stream:
                    while True:
                        chunk=stream.read(core.CHUNK)
                        if not chunk:break
                        length+=len(chunk);digest.update(chunk)
                if length!=pin['bytes'] or digest.hexdigest()!=pin['sha256']:
                    raise ValueError('NONHISTORICAL full input hash/EOF')
                self.calls.append((occurrence,pin,kwargs['kind']))
        capsules=[]
        for role in (54,69,72,75):
            fields=dict(args,context=args['context'] if role==54 else None,
                bundle=None if role==54 else dict(path=str(root/'NOT-A-PROOF-bundle'),
                    bytes=constructors.ROLE_MAXIMA[68],sha256='0'*64),
                source_observations=observations,frozen_sources=frozen)
            encoded={key:core.canonical(fields[key]).decode('ascii')
                for key in constructors.NATIVE_PROSPECTIVE_WIRE_FIELDS}
            body=core.canonical(encoded)
            path=root/('role%d-input-wires.json'%role);path.write_bytes(body)
            capsules.append((role,dict(path=str(path),bytes=len(body),
                sha256=hashlib.sha256(body).hexdigest())))
            del fields,encoded,body
        # Only immutable wires/pins survive in the caller. Every decoded slot,
        # selection, source and authority root is released before construction.
        del args,observations,frozen,pins,group,rows
        for role,capsule in capsules:
            charge_native(100+role,capsule,'evidence')
            body=Path(capsule['path']).read_bytes()
            self.assertEqual(hashlib.sha256(body).hexdigest(),capsule['sha256'])
            encoded=json.loads(body)
            argument_wires={key:value.encode('ascii') for key,value in encoded.items()}
            del body,encoded
            reader=Reader()
            inputs=dict(maxima=constructors.ROLE_MAXIMA,argument_wires=argument_wires,
                unit=reader,declarations=declarations,live_bytes=0)
            # The real role72 consumer exceeded its source-phase allocation
            # with an unused decoded slot. Require strict slot decoding only
            # after the complete70-read pass on both producer and consumer.
            from unittest.mock import patch
            strict_decode=constructors.records.decode
            slot_decode_counts=[]
            def decode_after_sources(wire,maximum):
                if wire==argument_wires['slot']:
                    slot_decode_counts.append(len(reader.calls))
                    self.assertIn(len(reader.calls),(70,140))
                return strict_decode(wire,maximum)
            with patch.object(constructors.records,'decode',side_effect=decode_after_sources):
                raw=constructors.prospective_native_report_from_wires(role,**inputs)
            output=root/('role%d.json'%role);output.write_bytes(raw)
            pin=dict(path=str(output),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
            report=(70,fixed._pin_tuple(pin),'evidence','bytes')
            with patch.object(constructors.records,'decode',side_effect=decode_after_sources):
                value=constructors.consume_prospective_native_report_from_wires(role,raw,
                    report_declaration=report,**inputs)
            self.assertEqual(slot_decode_counts,[70,140])
            self.assertEqual([x[0] for x in reader.calls],list(range(70))*2+[70])
            self.assertEqual(sum(x[1]['bytes'] for x in reader.calls[:7]),6785636)
            self.assertEqual([x[2] for x in reader.calls[7:70]],['source']*63)
            self.assertEqual(raw,core.canonical(value))
            self.assertFalse(any(value['qualification'].values()))
            if role!=54:
                self.assertEqual(value['actual']['resources'],
                    [dict(resource=n,used=v) for n,v in constructors.NATIVE_RESOURCE_LIMITS])
            if role in (72,75):
                self.assertEqual(len(value['values']),40)
                self.assertEqual([x['proof_index'] for x in value['values']],list(range(51,91)))
                self.assertTrue(all(x['logical']=='nonmember' and x['value_hex'] is None
                    and x['physical']['kind']=='successful_seek_without_matching_version'
                    and not x['usable_vrf_key'] for x in value['values']))
                self.assertEqual([x['physical']['identity'] for x in value['values']],
                    [x['tuple']['identity'] for x in value['selection']])
            with self.assertRaisesRegex(ValueError,'output binding'):
                constructors.consume_prospective_native_report_from_wires(role,raw+b' ',
                    report_declaration=report,**inputs)
            del value,raw,inputs,argument_wires,reader
        ledger=[json.loads(row) for row in native_journal.read_bytes().splitlines()]
        self.assertEqual(len(ledger),4*142)
        self.assertEqual(sum(row['bytes_plus_EOF'] for row in ledger),native_used)
        self.assertTrue(all(row['phase']=='BEFORE_OPEN' for row in ledger))
        self.assertLess(native_used,67108864)

    def test_prospective_log_contains_complete_records_without_space_padding(self):
        raw=constructors.prospective_log(1,maxima=constructors.ROLE_MAXIMA,
            command=['false'],live_bytes=0)
        rows=[json.loads(line) for line in raw.splitlines()]
        self.assertGreater(len(rows),1)
        self.assertTrue(all(row['mode']==constructors.PROSPECTIVE_MODE and
            row['executed'] is False for row in rows))
        self.assertEqual(raw,b''.join(core.canonical(row) for row in rows))


class QuotaConstructorTests(unittest.TestCase):
    def inputs(self):
        d=Path('/home/fry/artifacts/vrf40-development-20261018')
        raw=(d/'quota-constructor124.stdout').read_bytes()
        return dict(raw=raw,budget_raw=(ARTIFACTS/'quota-budget.json').read_bytes(),
            maxima=constructors.ROLE_MAXIMA,live_bytes=0,
            now_utc=json.loads(raw)['observed_utc'])

    def test_full_saved_observation_is_retained_unchanged(self):
        args=self.inputs()
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])

    def test_round202_budget_keeps_exact_saved_identity_and_floor(self):
        args=self.inputs();value=json.loads(args['raw'])
        path='/home/fry/artifacts/vrf40-integration-20261018-round202/quota-budget-round202.json'
        args['budget_raw']=Path(path).read_bytes();budget=json.loads(args['budget_raw'])
        value.update(budget_file=path,baseline_observed_utc=budget['observed_utc'],
            baseline_remaining_percent=64,baseline_used_percent=36,
            baseline_session_id=budget['session_id'],baseline_resets_at=budget['resets_at'],
            session_id=budget['session_id'],resets_at=budget['resets_at'],
            floor_remaining_percent=54,remaining_percent=61,used_percent=39,
            consumed_percentage_points=3,consumed_percent=3)
        args['raw']=core.canonical(value)
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])
        value['budget_file']=str(ARTIFACTS/'quota-budget.json');args['raw']=core.canonical(value)
        with self.assertRaisesRegex(ValueError,'baseline/floor'):
            constructors.quota_snapshot(**args)

    def test_round211_budget_keeps_exact_saved_identity_and_floor(self):
        args=self.inputs();value=json.loads(args['raw'])
        path='/home/fry/artifacts/vrf40-integration-20261018-round211/quota-budget-round211.json'
        args['budget_raw']=Path(path).read_bytes();budget=json.loads(args['budget_raw'])
        # This is an explicit nonhistorical boundary fixture, not fresh telemetry.
        value.update(budget_file=path,baseline_observed_utc=budget['observed_utc'],
            observed_utc=budget['observed_utc'],age_seconds=0,
            baseline_remaining_percent=54,baseline_used_percent=46,
            baseline_session_id=budget['session_id'],baseline_resets_at=budget['resets_at'],
            session_id=budget['session_id'],resets_at=budget['resets_at'],
            floor_remaining_percent=44,remaining_percent=54,used_percent=46,
            consumed_percentage_points=0,consumed_percent=0,status='allow_start',
            reason='NONHISTORICAL quota identity boundary fixture')
        args['now_utc']=budget['observed_utc'];args['raw']=core.canonical(value)
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])
        value['budget_file']=str(ARTIFACTS/'quota-budget.json');args['raw']=core.canonical(value)
        with self.assertRaisesRegex(ValueError,'baseline/floor'):
            constructors.quota_snapshot(**args)
        value.update(budget_file=path,remaining_percent=44,used_percent=56,
            consumed_percentage_points=10,consumed_percent=10,status='do_not_start')
        args['raw']=core.canonical(value)
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])
        value['status']='allow_start';args['raw']=core.canonical(value)
        with self.assertRaisesRegex(ValueError,'floor arithmetic'):
            constructors.quota_snapshot(**args)

    def test_source91_budget_preserves_separate_baseline_and_floor(self):
        args=self.inputs();value=json.loads(args['raw'])
        path='/home/fry/artifacts/vrf40-integration-20261018-round211/quota-budget-source91-20261019.json'
        args['budget_raw']=Path(path).read_bytes();budget=json.loads(args['budget_raw'])
        # Boundary fixture only; this is not a fresh allowance observation.
        value.update(budget_file=path,baseline_observed_utc=budget['observed_utc'],
            observed_utc=budget['observed_utc'],age_seconds=0,
            baseline_remaining_percent=44,baseline_used_percent=56,
            baseline_session_id=budget['session_id'],baseline_resets_at=budget['resets_at'],
            session_id=budget['session_id'],resets_at=budget['resets_at'],
            floor_remaining_percent=34,remaining_percent=44,used_percent=56,
            consumed_percentage_points=0,consumed_percent=0,status='allow_start',
            reason='NONHISTORICAL quota identity boundary fixture')
        args['now_utc']=budget['observed_utc'];args['raw']=core.canonical(value)
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])
        value['budget_file']=str(ARTIFACTS/'quota-budget.json')
        args['raw']=core.canonical(value)
        with self.assertRaisesRegex(ValueError,'baseline/floor'):
            constructors.quota_snapshot(**args)
        value.update(budget_file=path,remaining_percent=34,used_percent=66,
            consumed_percentage_points=10,consumed_percent=10,status='do_not_start')
        args['raw']=core.canonical(value)
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])
        value['status']='allow_start';args['raw']=core.canonical(value)
        with self.assertRaisesRegex(ValueError,'floor arithmetic'):
            constructors.quota_snapshot(**args)

    def test_old_observation_cannot_be_relabelled_fresh(self):
        args=self.inputs();args['now_utc']='2026-10-18T18:00:00+00:00'
        with self.assertRaisesRegex(ValueError,'absolute freshness'):
            constructors.quota_snapshot(**args)

    def test_floor_observation_is_retained_without_extending_budget(self):
        args=self.inputs();v=json.loads(args['raw'])
        v.update(remaining_percent=74,used_percent=26,consumed_percentage_points=10,
                 consumed_percent=10,status='do_not_start')
        args['raw']=core.canonical(v)
        self.assertIs(constructors.quota_snapshot(**args),args['raw'])
        v['status']='allow_start';args['raw']=core.canonical(v)
        with self.assertRaisesRegex(ValueError,'floor arithmetic'):
            constructors.quota_snapshot(**args)

    def test_reset_floor_or_arithmetic_substitution_reject(self):
        for key,value in (('budget_initialized',True),('floor_remaining_percent',70),
                          ('consumed_percentage_points',0),('age_seconds',301)):
            args=self.inputs();v=json.loads(args['raw']);v[key]=value
            args['raw']=core.canonical(v)
            with self.assertRaises(ValueError):constructors.quota_snapshot(**args)


class HistoricalContextConstructorTests(unittest.TestCase):
    class Reader:
        def __init__(self):
            self.calls=[]
            self.invocation=SimpleNamespace(metadata_charge=lambda:4096)
            self.declaration_charge=2048
        def consume(self,occurrence,pin,**kwargs):
            constructors.records.authenticate(dict(role='full-original-context',**pin))
            self.calls.append((occurrence,pin))

    def args(self,role):
        pin=constructors.HISTORICAL_CONTEXT_PINS[role]
        scope=constructors.HISTORICAL_CONTEXT_SCOPE
        return dict(maxima=constructors.ROLE_MAXIMA,unit=self.Reader(),
            scope_declaration=(1500,fixed._pin_tuple(scope),'evidence','bytes'),
            body_declaration=(1501,fixed._pin_tuple(pin),'evidence','bytes'),live_bytes=0)

    def test_all_five_original_bodies_remain_identical_with_two_full_reads(self):
        total=0
        for role,pin in constructors.HISTORICAL_CONTEXT_PINS.items():
            raw=Path(pin['path']).read_bytes();args=self.args(role)
            self.assertIs(constructors.historical_context_body(role,raw,**args),raw)
            self.assertEqual([p for _,p in args['unit'].calls],
                [constructors.HISTORICAL_CONTEXT_SCOPE,pin])
            total+=len(raw)
        self.assertEqual(total,341615)

    def test_context_body_drift_or_scope_retarget_cannot_reuse(self):
        role=45;pin=constructors.HISTORICAL_CONTEXT_PINS[role]
        raw=Path(pin['path']).read_bytes()
        with self.assertRaisesRegex(ValueError,'body changed'):
            constructors.historical_context_body(role,b' '+raw[1:],**self.args(role))
        args=self.args(role)
        args['scope_declaration']=(1500,fixed._pin_tuple(pin),'evidence','bytes')
        with self.assertRaisesRegex(ValueError,'declarations'):
            constructors.historical_context_body(role,raw,**args)
        self.assertEqual(args['unit'].calls,[])


class RuntimeCommandConstructorTests(unittest.TestCase):
    def args(self,role):
        def p(name):return dict(path='/home/fry/artifacts/NONHISTORICAL-'+name,bytes=17,sha256='a'*64)
        args=dict(maxima=constructors.ROLE_MAXIMA,command=['false'],exit_code=17,
            child_exit=17,seconds=1,log=p('log'),wall_timeout=False,
            output_overflow=False,live_bytes=0)
        if role==66:
            args['before']=[dict(path='/workspaces/rustaxa-evm/local/evm-state-db/snapshot-litenode-copy'+suffix,
                device=1,inode=i,mode=16877,mtime_ns=1)
                for i,suffix in enumerate(('','/db/db','/db/state_db'))]
            args['after']=[dict(row) for row in args['before']]
        else:
            args.update(cwd='/workspaces/rustaxa-evm',environment=dict(LANG='C',LC_ALL='C',PATH='/usr/bin:/bin'))
            if role in (64,73):args.update(freeze=p('freeze'),ready=p('ready'))
            if role in (70,73):args.update(binary=p('binary'),bundle=p('bundle'))
        return args

    def test_all_four_actual_failure_results_remain_unaccepted(self):
        for role in (64,66,70,73):
            result=json.loads(constructors.runtime_command_result(role,**self.args(role)))
            self.assertFalse(result['accepted'])
            self.assertEqual(result['child_exit'],17)
            self.assertEqual(result['exit'],17)
            self.assertEqual(result.get('new_facts',0),0)
            if role==66:self.assertNotIn('schema',result)

    def test_runtime_environment_omission_or_extension_rejects(self):
        for environment in ({},{'LANG':'C','LC_ALL':'C','PATH':''},
                            {'LANG':'C','LC_ALL':'C','PATH':'/bin','EXTRA':'1'}):
            args=self.args(64);args['environment']=environment
            with self.assertRaisesRegex(ValueError,'environment'):
                constructors.runtime_command_result(64,**args)

    def test_copy_metadata_difference_is_preserved_for_rejection(self):
        args=self.args(66);args['after'][2]['mtime_ns']=2
        result=json.loads(constructors.runtime_command_result(66,**args))
        self.assertNotEqual(result['before'],result['after'])
        args['after'].pop()
        with self.assertRaisesRegex(ValueError,'three-path'):
            constructors.runtime_command_result(66,**args)

    def test_runtime_log_deadline_and_caller_limits_are_not_raised(self):
        for key,value in (('seconds',301),('live_bytes',core.SEMANTIC)):
            args=self.args(70);args[key]=value
            with self.assertRaises(ValueError):constructors.runtime_command_result(70,**args)
        args=self.args(70);args['log']['bytes']=65537
        with self.assertRaises(ValueError):constructors.runtime_command_result(70,**args)


class BinaryMappingConstructorTests(unittest.TestCase):
    def args(self):
        def p(name,size=17):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-'+name,bytes=size,sha256='a'*64)
        binaries=[p('live',204551256),p('replay',195506744)]
        return dict(maxima=constructors.ROLE_MAXIMA,binaries=binaries,
            embedded_executable=dict(binaries[0]),preflight=p('preflight'),
            source_freeze=p('source'),live_bytes=0)

    def test_complete_pair_and_embedded_identity_are_preserved_without_acceptance(self):
        args=self.args()
        value=json.loads(constructors.binary_successor_mapping(**args))
        self.assertEqual(value['binaries'],args['binaries'])
        self.assertEqual(value['embedded_executable'],args['binaries'][0])
        self.assertFalse(value['accepted'])

    def test_embedded_substitution_or_binary_limit_increase_rejects(self):
        args=self.args();args['embedded_executable']['sha256']='b'*64
        with self.assertRaisesRegex(ValueError,'executable binding'):
            constructors.binary_successor_mapping(**args)
        args=self.args();args['binaries'][1]['bytes']=core.BINARY_FILE+1
        with self.assertRaisesRegex(ValueError,'selected40 input length'):
            constructors.binary_successor_mapping(**args)


class CommitHookConstructorTests(unittest.TestCase):
    def inputs(self):
        def p(name,raw=b''):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-'+name,
                bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        log=b'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n'
        acceptance=p('acceptance');sources=[p('owned-source')]
        actual=dict(schema=1,command=['rtk','proxy','git','commit','-m','fix(evm): fixture'],
            exit_code=0,hooks_path='.githooks',log=p('hook-log',log),no_push=True,normal_hook=True,
            owned_sources=sources,parent='b'*40,source_acceptance=acceptance,source_commit='c'*40)
        raw=core.canonical(actual)
        return raw,p('actual-commit',raw),log,actual

    def test_actual_commit_and_complete_hook_log_bind_same_commit(self):
        raw,pin,log,actual=self.inputs()
        value=json.loads(constructors.source_commit_record(maxima=constructors.ROLE_MAXIMA,
            actual_raw=raw,actual_pin=pin,source_commit=actual['source_commit'],
            source_acceptance=actual['source_acceptance'],log=actual['log'],
            owned_sources=actual['owned_sources'],live_bytes=0))
        self.assertTrue(value['no_push'])
        summary=json.loads(constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
            actual_raw=raw,actual_pin=pin,full_log=log,live_bytes=0))
        self.assertEqual(summary['source_commit'],value['source_commit'])
        self.assertEqual(summary['matched_tests'],2)
        self.assertFalse(summary['bypassed'])

    def test_many_short_lines_keep_complete_log_and_final_test_summary(self):
        raw,pin,log,actual=self.inputs()
        full=b'\n'*327000+log
        actual['log'].update(bytes=len(full),sha256=hashlib.sha256(full).hexdigest())
        raw=core.canonical(actual)
        pin.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        summary=json.loads(constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
            actual_raw=raw,actual_pin=pin,full_log=full,live_bytes=0))
        self.assertEqual(summary['matched_tests'],2)
        self.assertEqual(summary['log']['bytes'],len(full))

    def test_bool_cannot_replace_commit_schema_or_exit_integer(self):
        for key,value in (('schema',True),('exit_code',False)):
            raw,pin,log,actual=self.inputs();actual[key]=value
            raw=core.canonical(actual)
            pin.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
            with self.assertRaisesRegex(ValueError,'normal-hook local'):
                constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
                    actual_raw=raw,actual_pin=pin,full_log=log,live_bytes=0)

    def test_maximal_digit_capture_workspace_rejects_before_scanning(self):
        from unittest.mock import patch
        raw,pin,log,actual=self.inputs()
        full=b'test result: ok. '+b'0'*326900+b'1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'
        actual['log'].update(bytes=len(full),sha256=hashlib.sha256(full).hexdigest())
        raw=core.canonical(actual)
        pin.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        with patch.object(constructors.re,'compile') as scan:
            with self.assertRaises(ValueError) as stopped:
                constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
                    actual_raw=raw,actual_pin=pin,full_log=full,live_bytes=core.SEMANTIC-1000000)
            scan.assert_not_called()
        self.assertIn('capture/int pre-scan',str(stopped.exception))

    def test_hook_bypass_and_complete_log_substitution_reject(self):
        raw,pin,log,actual=self.inputs()
        actual['command'].append('--no-verify')
        raw=core.canonical(actual);pin.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        with self.assertRaisesRegex(ValueError,'normal-hook local'):
            constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
                actual_raw=raw,actual_pin=pin,full_log=log,live_bytes=0)
        raw,pin,log,actual=self.inputs()
        with self.assertRaisesRegex(ValueError,'complete normal-hook log'):
            constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
                actual_raw=raw,actual_pin=pin,full_log=log+b'x',live_bytes=0)


class ValidationSummaryConstructorTests(unittest.TestCase):
    def args(self):
        def pin(name):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-'+name,bytes=17,sha256='a'*64)
        names=('source_acceptance','source_freeze','tool_source_freeze','source_commit_record',
               'actual_hook','hook_summary','complete_offline_fit_review','authority',
               'bridge_scope','cache_scope')
        return dict(maxima=constructors.ROLE_MAXIMA,source_commit='c'*40,live_bytes=0,
            gates=[dict(index=2*i,result=pin('result-'+str(2*i)),log=pin('log-'+str(2*i+1)))
                   for i in range(14)],**{name:pin(name) for name in names})

    def test_all14_full_bindings_survive_without_physical_readiness(self):
        args=self.args()
        value=json.loads(constructors.validation_summary(**args))
        self.assertEqual(value['gates'],args['gates'])
        self.assertEqual(value['cache_scope'],args['cache_scope'])
        self.assertFalse(value['physical_ready'])
        self.assertFalse(value['captured'])
        self.assertEqual(value['new_facts'],0)

    def test_missing_reordered_or_oversize_gate_cannot_enter_summary(self):
        for kind in ('missing','reordered','oversize','ordinal-renumber'):
            args=self.args()
            if kind=='missing':args['gates'].pop()
            elif kind=='reordered':args['gates'][0],args['gates'][1]=args['gates'][1],args['gates'][0]
            elif kind=='ordinal-renumber':
                for i,gate in enumerate(args['gates']):gate['index']=i
            else:args['gates'][13]['log']['bytes']=327681
            with self.assertRaises(ValueError):constructors.validation_summary(**args)


if __name__=='__main__':unittest.main()

class InstructionComparisonConstructorTests(unittest.TestCase):
    def args(self):
        rows=[]
        for i,path in enumerate(constructors.INSTRUCTION_PATHS):
            current=dict(path=path,bytes=100+i,sha256='a'*64)
            authority=dict(current)
            if i:authority['sha256']='b'*64
            previous=dict(path='/home/fry/artifacts/test-only/predecessor-'+str(i),
                          bytes=90+i,sha256='c'*64)
            rows.append(dict(current=current,authority_observation=authority,
                             preparation_predecessor=previous))
        return dict(maxima=constructors.ROLE_MAXIMA,instructions=rows,
            active_user_scope='fixed40 only, saved quota floor74, full fit before gates',
            live_bytes=0)

    def test_complete_six_observations_and_computed_equality(self):
        args=self.args()
        value=json.loads(constructors.instruction_preparation_comparison(**args))
        self.assertEqual(len(value['instructions']),6)
        self.assertEqual([x['exact_authority_pin_equal'] for x in value['instructions']],
                         [True,False,False,False,False,False])
        self.assertFalse(value['accepted']);self.assertFalse(value['ready'])
        for actual,original in zip(value['instructions'],args['instructions']):
            for name,pin in original.items():self.assertEqual(actual[name],pin)

    def test_reject_missing_reordered_forged_fields_and_live_overflow(self):
        for kind in ('missing','order','extra','source_cap','scope','oversized_scope','utf8_scope','caller'):
            args=self.args()
            if kind=='missing':args['instructions'].pop()
            elif kind=='order':args['instructions'].reverse()
            elif kind=='extra':args['instructions'][0]['exact_authority_pin_equal']=True
            elif kind=='source_cap':args['instructions'][0]['current']['bytes']=1048577
            elif kind=='scope':args['active_user_scope']=''
            elif kind=='oversized_scope':args['active_user_scope']='x'*8193
            elif kind=='utf8_scope':args['active_user_scope']='é'*4097
            else:args['live_bytes']=2097152
            with self.assertRaises(ValueError):
                constructors.instruction_preparation_comparison(**args)

class PhysicalAttemptMarkerConstructorTests(unittest.TestCase):
    def args(self):
        return dict(maxima=constructors.ROLE_MAXIMA,
            live_executable=dict(path='/home/fry/artifacts/test-only/live-fixed40',
                                 bytes=200000000,sha256='a'*64),
            bundle_path='/home/fry/artifacts/test-only/fixed40.rcv6',
            report_path='/home/fry/artifacts/test-only/fixed40-capture.json',
            freeze=dict(path='/home/fry/artifacts/test-only/command-freeze.json',
                        bytes=100000,sha256='b'*64),
            source_commit='c'*40,live_bytes=0)

    def test_exact_original_three_fields_and_full_inner_command(self):
        args=self.args()
        value=json.loads(constructors.physical_attempt_marker(**args))
        self.assertEqual(set(value),{'command','freeze_sha256','source_commit'})
        self.assertEqual(value['command'],[args['live_executable']['path'],
            '--bundle-output',args['bundle_path'],'--report-output',args['report_path']])
        self.assertEqual(value['freeze_sha256'],args['freeze']['sha256'])
        self.assertEqual(value['source_commit'],args['source_commit'])
        self.assertFalse(Path(args['bundle_path']).exists())

    def test_reject_collisions_bad_identity_and_live_overflow(self):
        for kind in ('collision','relative','commit','binary','caller'):
            args=self.args()
            if kind=='collision':args['report_path']=args['bundle_path']
            elif kind=='relative':args['bundle_path']='relative.rcv6'
            elif kind=='commit':args['source_commit']='f'*39
            elif kind=='binary':args['live_executable']['bytes']=constructors.core.BINARY_FILE+1
            else:args['live_bytes']=2097152
            with self.assertRaises(ValueError):
                constructors.physical_attempt_marker(**args)

class CurrentCorrectionConstructorTests(unittest.TestCase):
    def args(self):
        body='full failure line\nUTF8 é\n'
        raw=body.encode('utf-8')
        return dict(maxima=constructors.ROLE_MAXIMA,failures=[
            dict(role='test-only-failure',path='/home/fry/artifacts/test-only/failure.log',
                 bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest(),body_utf8=body)],
            live_bytes=0)

    def test_complete_failure_body_retained_without_acceptance(self):
        args=self.args()
        value=json.loads(constructors.current_correction_record(**args))
        self.assertEqual(set(value),{'schema','accepted','failures','findings'})
        self.assertEqual(value['failures'],args['failures'])
        self.assertFalse(value['accepted'])

    def test_reject_missing_changed_duplicated_or_excessive_failure_bodies(self):
        for kind in ('empty','partial','bytes','hash','duplicate','caller'):
            args=self.args();row=args['failures'][0]
            if kind=='empty':args['failures']=None
            elif kind=='partial':row['body_utf8']=row['body_utf8'][:-1]
            elif kind=='bytes':row['bytes']+=1
            elif kind=='hash':row['sha256']='f'*64
            elif kind=='duplicate':args['failures'].append(dict(row))
            else:args['live_bytes']=2097152
            with self.assertRaises(ValueError):
                constructors.current_correction_record(**args)

    def test_exact_escaped_wire_count_and_preallocation_cap(self):
        for text in ('ascii', 'quote"slash'+chr(92), '\n\b\t'+chr(0), 'é中😀',chr(127)):
            expected=len(json.dumps(text,ensure_ascii=True).encode('ascii'))
            self.assertEqual(constructors._correction_string_wire_size(text,10000),expected)
        args=self.args()
        body='x'*constructors.ROLE_MAXIMA[48]
        args['failures'][0].update(body_utf8=body,bytes=len(body),
            sha256=hashlib.sha256(body.encode('ascii')).hexdigest())
        with self.assertRaisesRegex(ValueError,'correction record cap'):
            constructors.current_correction_record(**args)

    def test_json_width_boundaries_all_byte_values_and_unicode_planes(self):
        values=list(range(256))+[0xD7FF,0xD800,0xDFFF,0xE000,0xFFFF,0x10000,0x10FFFF]
        for value in values:
            text=chr(value)
            expected=len(json.dumps(text,ensure_ascii=True).encode('ascii'))
            with self.subTest(codepoint=value):
                self.assertEqual(constructors._correction_string_wire_size(text,expected),expected)
                self.assertGreater(constructors._correction_string_wire_size(text,expected-1),
                                   expected-1)
        self.assertEqual(constructors._correction_string_wire_size(chr(0x7E),8),3)
        self.assertEqual(constructors._correction_string_wire_size(chr(0x7F),8),8)
        self.assertEqual(constructors._correction_string_wire_size(chr(0x80),8),8)

    def test_complete_multiple_failure_rows_with_del_and_exact_role_cap(self):
        args=self.args()
        for i,text in enumerate((chr(0x7F), ''.join(map(chr,range(128)))+'é中😀')):
            raw=text.encode('utf-8')
            args['failures'].append(dict(role='test-only-boundary-'+str(i),
                path='/home/fry/artifacts/test-only/boundary-'+str(i)+'.log',
                bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest(),body_utf8=text))
        raw=constructors.current_correction_record(**args)
        self.assertEqual(json.loads(raw)['failures'],args['failures'])
        maxima=list(args['maxima']);maxima[48]=len(raw);args['maxima']=tuple(maxima)
        # The exact fixed81 role contract rejects altered maxima, even when a
        # shorter output would fit. The existing role cap remains unchanged.
        with self.assertRaises(ValueError):constructors.current_correction_record(**args)

    def test_manifest_proved_empty_failure_list_needs_no_fabricated_body(self):
        args=self.args();args['failures']=[]
        value=json.loads(constructors.current_correction_record(**args))
        self.assertEqual(value['failures'],[])
        self.assertFalse(value['accepted'])

class CoherentPagedHistorySourceBridgeTest(CoherentSourceBridgeTest):
    PAGED_HISTORY=True


class PagedHistoryRepresentationTests(unittest.TestCase):
    def setUp(self):
        import pa1_development as development
        self.controller=development.CoherentDevelopmentReservations
        self.root=Path(tempfile.mkdtemp(dir=ARTIFACTS,prefix='paged-history-NONHISTORICAL-'))
        self.marker=put(self.root/'marker.json',dict(scope='unit fixture only'))
        self.page=put(self.root/'page.json',dict(
            schema='fixed40-exact-development-history-page-v1',
            records=[dict(first=9,last=106,pin=self.marker)]))
        self.history=dict(schema='fixed40-exact-development-history-v2',
            admission='NONHISTORICAL fixture only',breach_event=36,durable=pin(LEGACY),
            effective=[106,0,0],pages=[dict(first=9,last=106,pin=self.page)])

    def test_current_exact_generated_pages_and_every_original_body(self):
        # This is the reviewed exact component, not a runtime admission.
        proposal=json.loads(Path(
            '/home/fry/artifacts/vrf40-development-20261018/history161-v2/proposal.json').read_bytes())
        history=self.controller.authenticate(proposal['root'],16384,decoded=True)
        rows=[]
        for page in self.controller.history_pages(history,self.controller.authenticate):
            for row in page:
                self.controller.authenticate(row['pin'],327680)
                rows.append(row)
        self.assertEqual(len(rows),148)
        self.assertEqual(sum(row['last']-row['first']+1 for row in rows),153)
        self.assertEqual(hashlib.sha256(core.canonical(rows)).hexdigest(),
                         proposal['ordered_rows_sha256'])
        self.assertEqual(proposal['framing_bytes_root_and_pages'],31572)

    def test_corrupted_page_full_hash_and_eof_rejected(self):
        page=Path(self.page['path']);page.write_bytes(page.read_bytes()+b' ')
        with self.assertRaisesRegex(ValueError,'EOF/hash'):
            list(self.controller.history_pages(self.history,self.controller.authenticate))

    def test_root_gaps_caps_identity_and_missing_terminal_rejected_before_page_read(self):
        from unittest.mock import Mock
        for kind in ('gap','cap','durable','terminal','extra'):
            value=json.loads(json.dumps(self.history))
            if kind=='gap':value['pages'][0]['first']=10
            elif kind=='cap':value['pages'][0]['pin']['bytes']=16385
            elif kind=='durable':value['durable']['sha256']='f'*64
            elif kind=='terminal':value['pages'][0]['last']=105
            else:value['records']=[]
            opened=Mock()
            with self.subTest(kind=kind),self.assertRaises(ValueError):
                list(self.controller.history_pages(value,opened))
            opened.assert_not_called()

    def test_page_fields_row_order_and_range_must_match_authenticated_root(self):
        for kind in ('fields','order','range','missing'):
            page=dict(schema='fixed40-exact-development-history-page-v1',
                      records=[dict(first=9,last=106,pin=self.marker)])
            if kind=='fields':page['omitted']=True
            elif kind=='order':page['records'][0]['first']=10
            elif kind=='range':page['records'][0]['last']=105
            else:page['records']=[]
            value=json.loads(json.dumps(self.history))
            value['pages'][0]['pin']=put(self.root/('bad-'+kind+'.json'),page)
            with self.subTest(kind=kind),self.assertRaises(ValueError):
                list(self.controller.history_pages(value,self.controller.authenticate))

    def test_page_lifetime_includes_caller_before_open_and_decoding(self):
        from unittest.mock import patch
        with patch('pa1_development.os.open') as opened:
            with self.assertRaisesRegex(ValueError,'caller lifetime'):
                list(self.controller.history_pages(self.history,self.controller.authenticate,
                    live_bytes=core.SEMANTIC-fixed.graph_charge(self.history)-1))
            opened.assert_not_called()

    def test_runtime_raw_guard_rejects_before_open_for_both_read_modes(self):
        from unittest.mock import patch
        obj=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        obj._graphs=[];obj.read_trace=[]
        for decoded in (False,True):
            with patch('pa1_fixed40_runtime.os.open') as opened:
                with self.assertRaisesRegex(ValueError,'pre-open caller lifetime'):
                    obj._load(self.page,16384,decoded=decoded,live_bytes=core.SEMANTIC-1)
                opened.assert_not_called()

    def test_constructor_requires_precharge_and_authenticates_every_page(self):
        # Base constructor has no authority to omit new page costs.
        import pa1_development as development
        caps=list(self.controller.HARD);used=list(self.controller.INHERITED)
        history=put(self.root/'history.json',self.history)
        authority=put(self.root/'authority.json',dict(caps=caps,used=used,
            history=history,profiles=[],development_only=True))
        (self.root/'state.json').write_bytes(LEGACY.read_bytes())
        (self.root/'lock').touch()
        base_authority=put(self.root/'base-authority.json',dict(
            caps=list(development.DevelopmentReservations.HARD),
            used=list(development.DevelopmentReservations.INHERITED),
            history=history,profiles=[],development_only=True))
        with self.assertRaisesRegex(ValueError,'prepaid full history pages'):
            development.DevelopmentReservations.migrate(self.root,history_pin=history,
                authority=base_authority['sha256'],authority_pin=base_authority,
                caps=tuple(development.DevelopmentReservations.HARD),
                used=tuple(development.DevelopmentReservations.INHERITED))
        charged=[]
        def charge(packet):charged.append(packet);engineering_charge(packet)
        self.controller.migrate(self.root,history_pin=history,
            authority=authority['sha256'],authority_pin=authority,
            caps=tuple(caps),used=tuple(used),read_charge=charge)
        self.assertEqual(charged[-1],(self.page,))
        self.assertEqual(sum(packet==(self.page,) for packet in charged),2)
        before=len(charged)
        self.controller(self.root,authority['sha256'],authority_pin=authority,read_charge=charge)
        self.assertEqual(charged[before+1],(self.page,))
        Path(self.page['path']).write_bytes(b'corrupt')
        with self.assertRaises(ValueError):
            self.controller(self.root,authority['sha256'],authority_pin=authority,read_charge=charge)

    def test_v1_cannot_silently_ignore_pages(self):
        from unittest.mock import Mock
        value=dict(schema='fixed40-exact-development-history-v1',
            breach_event=36,effective=[106,0,0],records=[
                dict(first=9,last=106,pin=self.marker)],pages=self.history['pages'])
        opened=Mock()
        with self.assertRaisesRegex(ValueError,'ambiguous records/pages'):
            list(self.controller.history_pages(value,opened))
        opened.assert_not_called()


class RestrictedOfflineFitTests(unittest.TestCase):
    def test_separate_capability_rejects_missing_admission_or_debit(self):
        with self.assertRaisesRegex(ValueError,'separate unaccepted'):
            runtime.OfflineFitAdmission(object(),{},read_charge=lambda packet:None)
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission.accepted=False
        with self.assertRaisesRegex(ValueError,'read debit'):
            runtime.OfflineFitAdmission(admission,{},read_charge=None)
        admission.accepted=True
        with self.assertRaisesRegex(ValueError,'separate unaccepted'):
            runtime.OfflineFitAdmission(admission,{},read_charge=lambda packet:None)

    def test_exact_clone_path_and_symlink_reject_before_counter_access(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-path-') as raw:
            root=Path(raw);clone=root/'clone';clone.mkdir()
            with patch.object(runtime,'OFFLINE_FIT_ROOT',clone):
                self.assertEqual(runtime._offline_fit_directory(clone),clone)
                with self.assertRaisesRegex(ValueError,'exact offline fit'):
                    runtime._offline_fit_directory(root/'other')
                alias=root/'alias';alias.symlink_to(clone,target_is_directory=True)
                with patch.object(runtime,'OFFLINE_FIT_ROOT',alias):
                    with self.assertRaisesRegex(ValueError,'symlink'):
                        runtime._offline_fit_directory(alias)

    def test_real_migration_cannot_omit_future_clone_debits(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-carry-') as raw:
            clone=Path(raw)/'clone'
            with patch.object(runtime,'OFFLINE_FIT_ROOT',clone):
                runtime._require_offline_fit_carry()
                clone.mkdir()
                with self.assertRaisesRegex(ValueError,'exactly-once'):
                    runtime._require_offline_fit_carry()
                clone.rmdir()
                clone.symlink_to(Path(raw)/'missing',target_is_directory=True)
                with self.assertRaisesRegex(ValueError,'exactly-once'):
                    runtime._require_offline_fit_carry()

    def test_clone_class_denies_all_production_and_real_migration(self):
        from unittest.mock import patch
        class Parent:
            @classmethod
            def limit_identity(cls):return 'base'
            def __init__(self,*args,**kwargs):
                raise AssertionError('parent counter access must not occur')
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission.counter_type=Parent;admission.event_record_hashes=[];admission._load=lambda *a,**k:None
        admission._migration_read_charge=lambda packet:None
        fit=runtime.OfflineFitAdmission.__new__(runtime.OfflineFitAdmission)
        fit.admission=admission;fit.review_pin={'sha256':'1'*64};fit.caller_charge=32768;fit._read_charge=lambda packet:None;fit._root_identity=((1,2,3),(4,5,6))
        fit._profile_wire=core.canonical(dict(development={},authority={},history={},
                                            events=[183,0,0],consumed={}))
        counter=fit._counter_type()
        with patch.object(runtime,'_offline_fit_directory',side_effect=AssertionError('cold IO before guard')):
            with self.assertRaisesRegex(ValueError,'cold counter complete caller lifetime'):
                counter('/NONHISTORICAL/cold',namespace='development',live_bytes=core.SEMANTIC)
        obj=counter.__new__(counter);obj._offline_external_live=core.SEMANTIC
        with self.assertRaisesRegex(ValueError,'initial closeout lifetime'):
            obj.close_fit_debits()
        self.assertNotEqual(counter.limit_identity(),'base')
        with self.assertRaisesRegex(ValueError,'cannot authorize real'):
            counter.migrate(None)
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-scope-') as raw:
            clone=Path(raw)
            with patch.object(runtime,'OFFLINE_FIT_ROOT',clone), patch.object(runtime,'_offline_fit_identity',return_value=fit._root_identity):
                for slot in (0,1,2):
                    with self.assertRaisesRegex(ValueError,'production'):
                        counter(clone,namespace='production',slot=slot)
                obj=counter.__new__(counter);obj.directory=clone
                for namespace,slot in (('production',0),('development',1)):
                    obj.namespace=namespace;obj.slot=slot
                    def locked():
                        with obj._locked():pass
                    for operation in (locked,obj._read,lambda:obj._write({})):
                        with self.assertRaisesRegex(ValueError,'production'):
                            operation()

class CandidateInputProjectionTests(unittest.TestCase):
    def test_projection_reads_full_control_and_rejects_unloaded_source_inputs(self):
        # NONHISTORICAL loader grammar only; no candidate/proof acceptance.
        from types import SimpleNamespace
        pin41=dict(path='/NONHISTORICAL/control41',bytes=2,sha256='0'*64)
        pin42=dict(path='/NONHISTORICAL/control42',bytes=2,sha256='1'*64)
        names=('slot','mapped_rows','prior_report','map_pin','map_acceptance',
            'prior_bundle','prior_report_pin','accepted_seal','accepted_review')
        parameters={name:None for name in names}
        source=dict(schema='fixed40-constructor-arguments-v1',role=42,parameters=parameters,
            byte_inputs={},json_inputs={},wire_inputs={},declaration_bindings={})
        control=dict(schema='fixed40-constructor-arguments-v1',role=41,parameters={},
            byte_inputs={},json_inputs=dict(candidate_inputs=dict(pin=pin42,
                field='parameters',source_role=42)),wire_inputs={},declaration_bindings={})
        calls=[]
        def decode(occurrence,pin,**kwargs):
            calls.append((occurrence,pin))
            return control if occurrence==0 else source
        unit=SimpleNamespace(declarations=[(0,fixed._pin_tuple(pin41),'evidence','json'),
            (1,fixed._pin_tuple(pin42),'evidence','json')],decode=decode)
        kwargs,bindings,position=fixed._fit_load_arguments(unit,dict(role=41,arguments=pin41),0)
        self.assertIs(kwargs['candidate_inputs'],parameters)
        self.assertEqual(calls,[(0,pin41),(1,pin42)])
        self.assertEqual(position,2)
        self.assertEqual(bindings,{})
        with self.assertRaisesRegex(ValueError,'complete role42 parameter projection'):
            fixed._fit_candidate_input_projection(dict(source,json_inputs={'hidden':pin42}),live_bytes=0)


class OfflineFitExternalCallerTests(unittest.TestCase):
    def test_external_request_rejects_before_profile_copy_or_any_read(self):
        admission=object.__new__(runtime.RuntimeAdmission)
        admission.accepted=False
        admission.caller_charge=0
        admission._profile_wire=b'{}\n'
        reads=[]
        admission._full_read_charge=lambda packet:reads.append(packet)
        with self.assertRaisesRegex(ValueError,'caller preallocation lifetime'):
            runtime.OfflineFitAdmission(admission,dict(runtime.FIT_PROTECTED_COUNTER),
                read_charge=lambda packet:reads.append(packet),live_bytes=core.SEMANTIC)
        self.assertEqual(reads,[])


class DeferredOuterGuardModelTests(unittest.TestCase):
    def test_missing_guard_is_nonexecutable_and_actual_command_stays_strict(self):
        # NONHISTORICAL grammar inputs only; no source authentication or launch.
        reference=dict(path='/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/tools/pa1_fixed40.py',
            bytes=1,sha256='0'*64)
        inner=['/NONHISTORICAL/live','--bundle-output','/NONHISTORICAL/bundle',
            '--report-output','/NONHISTORICAL/report']
        guard=constructors.guard_deferred_binding(reference_source=reference,
            inner_argv=inner,template_path='/NONHISTORICAL/template')
        self.assertFalse(guard['executable'])
        self.assertFalse(guard['implementation_present'])
        self.assertEqual(guard['invocation_template']['ready_input_slot']['name'],'ready')
        with self.assertRaisesRegex(ValueError,'closed deferred guard'):
            constructors._checked_guard_deferred(dict(guard,executable=True))
        pin=dict(path='/NONHISTORICAL/input',bytes=0,sha256=hashlib.sha256(b'').hexdigest())
        args=dict(maxima=constructors.ROLE_MAXIMA,live_bytes=0,command=None,
            cwd='/workspaces/rustaxa-evm',environment=dict(constructors.FIXED40_ENVIRONMENT),
            freeze=pin,ready=pin,log=pin)
        raw=constructors.prospective_runtime_command_result(64,guard_deferred=guard,**args)
        value=json.loads(raw)
        self.assertIsNone(value['command'])
        self.assertFalse(value['executed'])
        self.assertFalse(value['guardDeferred']['executable'])
        with self.assertRaisesRegex(ValueError,'terminal runtime result'):
            constructors.runtime_command_result(64,exit_code=0,child_exit=0,seconds=0,
                wall_timeout=False,output_overflow=False,**args)


class OfflineFitDecodeLifetimeTests(unittest.TestCase):
    def test_full_private_profile_guard_precedes_decode(self):
        from unittest.mock import patch
        wire=core.canonical(dict(controller='coherent-v3',payload=['x']*100))
        with patch.object(runtime.records,'decode',side_effect=AssertionError('decode before guard')):
            with self.assertRaisesRegex(ValueError,'predecode lifetime'):
                runtime._fit_profile_decode(wire,core.SEMANTIC-1)
        self.assertEqual(runtime._fit_profile_decode(wire,0)['controller'],'coherent-v3')

class OfflineFitOriginCapsuleTests(unittest.TestCase):
    def test_origin_pin_and_debit_are_snapshots_of_the_reviewed_capability(self):
        from unittest.mock import patch
        class Parent:
            @classmethod
            def limit_identity(cls):return 'base'
        class OriginRead(Exception):pass
        charged=[];loaded=[]
        def loader(pin,cap,**kwargs):
            kwargs['read_charge']((dict(pin),));loaded.append(dict(pin));raise OriginRead()
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission.counter_type=Parent;admission.event_record_hashes=[]
        admission._migration_read_charge=lambda packet:None;admission._load=loader;admission._full_read_charge=charged.append
        fit=runtime.OfflineFitAdmission.__new__(runtime.OfflineFitAdmission)
        fit.admission=admission;fit.review_pin={'sha256':'2'*64}
        fit.caller_charge=32768;fit._read_charge=charged.append;fit._root_identity=((1,2,3),(4,5,6))
        fit._profile_wire=core.canonical(dict(development={},authority={},history={},
            events=[186,0,0],consumed={}))
        counter=fit._counter_type()
        fit._read_charge=lambda packet:self.fail('mutable wrapper debit used')
        admission._load=lambda *a,**k:self.fail('mutable admission loader used')
        admission._full_read_charge=None
        original=dict(runtime.FIT_PROTECTED_COUNTER)
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-origin-') as raw:
            with patch.object(runtime,'OFFLINE_FIT_ROOT',Path(raw)), patch.object(runtime,'_offline_fit_identity',return_value=fit._root_identity):
                with patch.object(runtime,'FIT_PROTECTED_COUNTER',dict(path='/changed',bytes=1,sha256='3'*64)):
                    with self.assertRaises(OriginRead):
                        counter.migrate_fit_clone(raw,consumed_events=(186,0,0),
                            event_records=(),accepted_preflight='2'*64)
        self.assertEqual(charged,[(original,)])
        self.assertEqual(loaded,[original])

class OfflineFitFilesystemIdentityTests(unittest.TestCase):
    def test_directory_and_lock_replacement_change_the_full_identity(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-inodes-') as raw:
            clone=Path(raw)/'clone';clone.mkdir();(clone/'lock').touch()
            with patch.object(runtime,'OFFLINE_FIT_ROOT',clone):
                first=runtime._offline_fit_identity()
                (clone/'lock').rename(clone/'old-lock');(clone/'lock').touch()
                second=runtime._offline_fit_identity()
                self.assertEqual(first[0],second[0])
                self.assertNotEqual(first[1],second[1])
                clone.rename(Path(raw)/'old-clone');clone.mkdir();(clone/'lock').touch()
                third=runtime._offline_fit_identity()
                self.assertNotEqual(second[0],third[0])

class OfflineFitHeldDescriptorTests(unittest.TestCase):
    def test_state_io_stays_in_reviewed_directory_after_path_replacement(self):
        from unittest.mock import patch
        class Fixture(runtime._OfflineFitStateIO):pass
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-fd-') as raw:
            root=Path(raw);clone=root/'clone';clone.mkdir();(clone/'lock').touch()
            (clone/'state.json').write_bytes(b'original')
            with patch.object(runtime,'OFFLINE_FIT_ROOT',clone):
                obj=Fixture();obj.directory=clone
                obj._expected_fit_identity=runtime._offline_fit_identity()
                with self.assertRaisesRegex(ValueError,'completed pathname'):
                    with obj._locked():
                        clone.rename(root/'retained-reviewed')
                        clone.mkdir();(clone/'lock').touch()
                        (clone/'state.json').write_bytes(b'untouched')
                        self.assertEqual(obj._read_state_raw(),b'original')
                        obj._replace_state_raw(b'charged-reviewed-only')
                self.assertEqual((clone/'state.json').read_bytes(),b'untouched')
                self.assertEqual((root/'retained-reviewed/state.json').read_bytes(),b'charged-reviewed-only')
                self.assertIsNone(obj._fit_directory_fd)

    def test_wrong_held_lock_inode_is_rejected_before_yield(self):
        from unittest.mock import patch
        class Fixture(runtime._OfflineFitStateIO):pass
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-lockfd-') as raw:
            clone=Path(raw);(clone/'lock').touch();(clone/'state.json').write_bytes(b'original')
            with patch.object(runtime,'OFFLINE_FIT_ROOT',clone):
                obj=Fixture();obj.directory=clone
                obj._expected_fit_identity=runtime._offline_fit_identity()
                real_open=runtime.os.open
                (clone/'different-lock').touch()
                def swapped(path,*args,**kwargs):
                    if path=='lock':path='different-lock'
                    return real_open(path,*args,**kwargs)
                with patch.object(runtime.os,'open',side_effect=swapped):
                    with self.assertRaisesRegex(ValueError,'held lock identity'):
                        with obj._locked():self.fail('wrong lock yielded')
                with self.assertRaisesRegex(ValueError,'requires reviewed held'):
                    obj._read_state_raw()

class OfflineFitInitialSnapshotTests(unittest.TestCase):
    def test_initial_authority_does_not_follow_mutable_live_counter_path(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-initial-') as raw:
            root=Path(raw)
            initial=root/'initial-development-state.json';initial.write_bytes(b'initial-schema4')
            state=root/'state.json';state.write_bytes(initial.read_bytes())
            profile={'development':pin(initial)};review={'clone_state':pin(state)}
            with patch.object(runtime,'OFFLINE_FIT_ROOT',root):
                self.assertEqual(runtime._fit_initial_state(profile,review),pin(initial))
                state.write_bytes(b'consumed-schema3-never-reset')
                # Review binds the initial copy, not a reset requirement on resume.
                self.assertEqual(runtime._fit_initial_state(profile,review),pin(initial))
                for change in ({'path':str(state)},{'sha256':'9'*64},{'bytes':1}):
                    bad=dict(profile['development']);bad.update(change)
                    with self.assertRaisesRegex(ValueError,'immutable initial'):
                        runtime._fit_initial_state({'development':bad},review)
                self.assertEqual(state.read_bytes(),b'consumed-schema3-never-reset')

class CompleteRestrictedOfflineFitFixture(CoherentPagedHistorySourceBridgeTest):
    """NONHISTORICAL positive/resume regression; no actual preflight authority."""
    OFFLINE_FIT_FIXTURE=True
    EFFECTIVE=190

    def exercise_restricted_fit(self,root,state,snapshot,profile,old_record,original):
        from unittest.mock import patch
        initial=root/'initial-development-state.json'
        snapshot.rename(initial);profile['development']=pin(initial)
        profile_pin=put(root/'offline-fit-profile.json',profile)
        preflight=put(root/'NONHISTORICAL-preflight.json',dict(
            scope='synthetic positive fit capability fixture, not acceptance evidence'))
        with patch.object(runtime,'OFFLINE_FIT_ROOT',root):
            review=put(root/'NONHISTORICAL-fit-review.json',dict(
                schema='fixed40-offline-fit-bridge-review-v1',profile=profile_pin,
                source_identity=profile['source_identity'],preflight=preflight,
                clone_root=str(root),clone_state=pin(state),
                clone_filesystem=[list(row) for row in runtime._offline_fit_identity()],
                protected_counter=dict(runtime.FIT_PROTECTED_COUNTER),
                complete_preflight_accepted=True,frozen_source_accepted=False,
                mode='offline_fit_clone_only_no_database_no_production'))
            admission=runtime.RuntimeAdmission(profile_pin,read_charge=fixture_read_partition('fit-initial'))
            fit=runtime.OfflineFitAdmission(admission,review,read_charge=engineering_charge)
            self.assertFalse(admission.accepted);self.assertFalse(fit.accepted)
            events=(old_record['sha256'],)*(self.EFFECTIVE-8)
            fit.counter_type.migrate_fit_clone(root,consumed_events=(self.EFFECTIVE,0,0),
                event_records=events,accepted_preflight=review['sha256'])
            migrated=state.read_bytes();value=json.loads(migrated)
            self.assertEqual(value['legacy_raw'],original.decode())
            self.assertEqual(value['failures'],[self.EFFECTIVE,0,0])
            self.assertEqual(value['nonbinary'],
                profile['consumed']['nonbinary']+core.COUNTER_IO_RESERVE+2*(len(original)+1))
            contexts=[dict(route=route,phase='development:0:0:'+route,ordinal=i)
                      for route,count in core.ROUTES.items() for i in range(count)]
            self.assertEqual(len(contexts),16)
            context=contexts[0];self.assertEqual(context['route'],'archive')
            roster=tuple(core.digest(item) for item in contexts)
            options=dict(namespace='development',round_index=0,slot=0,
                context=context,context_roster=roster)
            counter=fit.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**options)
            counter.begin_io_scope()
            token=counter.admit(invocation=1,units=1,nonbinary=100,binary=0,
                seconds=1,slot=0,route='archive')
            pending=state.read_bytes()
            blocked=fit.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**options)
            blocked.begin_io_scope()
            with self.assertRaisesRegex(ValueError,'cannot change pending'):
                blocked.close_fit_debits()
            self.assertEqual(state.read_bytes(),pending)
            counter.finish(token['generation'],failed=False)
            consumed=state.read_bytes()
            self.assertGreater(json.loads(consumed)['nonbinary'],value['nonbinary'])
            # Reconstruct from immutable initial bytes while preserving schema3.
            resumed_admission=runtime.RuntimeAdmission(profile_pin,read_charge=fixture_read_partition('fit-resume'))
            resumed=runtime.OfflineFitAdmission(resumed_admission,review,read_charge=engineering_charge)
            resumed.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**options)
            self.assertEqual(state.read_bytes(),consumed)
            with self.assertRaises(ValueError):
                resumed.counter_type.migrate_fit_clone(root,consumed_events=(self.EFFECTIVE,0,0),
                    event_records=events,accepted_preflight=review['sha256'])
            self.assertEqual(state.read_bytes(),consumed)
            incomplete=resumed.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**options)
            incomplete.begin_io_scope()
            with self.assertRaisesRegex(ValueError,'all16 routes'):
                incomplete.close_fit_debits()
            charged=json.loads(state.read_bytes())
            self.assertEqual(charged['nonbinary'],json.loads(consumed)['nonbinary']+32768)
            for item in contexts[1:]:
                current=dict(options);current['context']=item
                route_counter=resumed.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**current)
                route_counter.begin_io_scope()
                token=route_counter.admit(invocation=1,units=1,nonbinary=100,
                    binary=0,seconds=1,slot=0,route=item['route'])
                route_counter.finish(token['generation'],failed=False)
            closing=resumed.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**options)
            with self.assertRaisesRegex(ValueError,'fresh cold IO scope'):
                closing.close_fit_debits()
            closing.begin_io_scope()
            before_guard=state.read_bytes()
            with patch.object(runtime.os,'open') as opened:
                with self.assertRaisesRegex(ValueError,'initial closeout lifetime'):
                    closing.close_fit_debits(live_bytes=core.SEMANTIC-1)
                opened.assert_not_called()
            self.assertEqual(state.read_bytes(),before_guard)
            closed=closing.close_fit_debits()
            snapshot=json.loads(Path(closed['counter']['path']).read_bytes())
            self.assertEqual(snapshot,json.loads(state.read_bytes()))
            self.assertEqual(snapshot['development'][0],dict(core.ROUTES))
            self.assertLessEqual(closed['receipt']['bytes'],4096)
            self.assertLessEqual(closed['complete_io']['attempted_bytes'],32768)
            self.assertFalse(closed['runtime_accepted'])
            first_receipt=Path(closed['receipt']['path']).read_bytes()
            closing_again=resumed.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,**options)
            closing_again.begin_io_scope();newer=closing_again.close_fit_debits()
            self.assertNotEqual(newer['receipt']['path'],closed['receipt']['path'])
            self.assertEqual(Path(closed['receipt']['path']).read_bytes(),first_receipt)
            self.assertGreater(json.loads(state.read_bytes())['nonbinary'],snapshot['nonbinary'])
            with self.assertRaisesRegex(ValueError,'production'):
                resumed.counter_type(root,value['binding'],fixed.BASELINE_HISTORY,
                    namespace='production',round_index=0,slot=0,context=context,context_roster=roster)
            with self.assertRaisesRegex(ValueError,'exactly-once'):
                runtime._require_offline_fit_carry()
            self.assertEqual(LEGACY.read_bytes(),original)

class FullAdmissionReadPrechargeTests(unittest.TestCase):
    def test_failed_precharge_prevents_profile_open(self):
        from unittest.mock import patch
        def deny(packet):raise ValueError('NONHISTORICAL precharge denied')
        fake=dict(path='/unopened-profile',bytes=10,sha256='4'*64)
        with patch.object(runtime.os,'open') as opened:
            with self.assertRaisesRegex(ValueError,'precharge denied'):
                runtime.RuntimeAdmission(fake,read_charge=deny)
            opened.assert_not_called()

    def test_old_component_admission_cannot_grant_fit_read_authority(self):
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission.accepted=False
        with self.assertRaisesRegex(ValueError,'every original admission read prepaid'):
            runtime.OfflineFitAdmission(admission,{},read_charge=lambda packet:None)

class FitDebitTransferBoundaryTests(unittest.TestCase):
    def test_nested_admission_caller_rejects_before_profile_open(self):
        from unittest.mock import patch
        with patch.object(runtime.os,'open') as opened:
            with self.assertRaisesRegex(ValueError,'initial nested admission lifetime'):
                runtime.RuntimeAdmission(dict(path='/unopened',bytes=2,sha256='0'*64),
                    live_bytes=core.SEMANTIC)
            opened.assert_not_called()

    def test_fit_transfer_requires_complete_admitted_reads_before_controls(self):
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission._full_read_charge=None
        with self.assertRaisesRegex(ValueError,'full admitted reads'):
            admission._consume_fit_prefix({'fit':{}},{},{})

    def test_fit_transfer_rejects_missing_controls_before_loading(self):
        from unittest.mock import patch
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission._full_read_charge=lambda packet:None
        with patch.object(admission,'_load') as opened:
            with self.assertRaisesRegex(ValueError,'complete fit transfer controls'):
                admission._consume_fit_prefix({'fit':{}},{},{})
            opened.assert_not_called()

    def test_final_migration_guard_authenticates_current_full_locked_bytes(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='NONHISTORICAL-fit-transfer-lock-') as raw:
            root=Path(raw);(root/'lock').touch()
            state=root/'state.json';state.write_bytes(b'complete-fixture-state')
            with patch.object(runtime,'OFFLINE_FIT_ROOT',root):
                captured=pin(state);charges=[]
                wire=core.canonical(dict(counter=captured,
                    filesystem=[list(row) for row in runtime._offline_fit_identity()]))
                with runtime._require_offline_fit_carry(wire,read_charge=charges.append):
                    self.assertEqual(charges,[(captured,)])
                state.write_bytes(b'later-consumed-fixture-state')
                with self.assertRaisesRegex(ValueError,'changed before migration'):
                    with runtime._require_offline_fit_carry(wire,read_charge=charges.append):
                        self.fail('stale fit snapshot entered real migration')
                self.assertEqual(len(charges),2)

    def test_final_carry_outer_guard_precedes_decode_and_file_open(self):
        from unittest.mock import patch
        with patch.object(runtime.records,'decode') as decoded, patch.object(runtime.os,'open') as opened:
            with self.assertRaisesRegex(ValueError,'initial carry lifetime'):
                runtime._require_offline_fit_carry(b'{}',read_charge=lambda packet:None,
                    live_bytes=core.SEMANTIC-1)
            decoded.assert_not_called();opened.assert_not_called()

    def test_final_carry_second_graph_guard_precedes_decode(self):
        from unittest.mock import patch
        raw=core.canonical(dict(nodes=list(range(1500))))
        with patch.object(runtime.OFFLINE_FIT_ROOT.__class__,'exists',return_value=True), patch.object(runtime.records,'decode') as decoded, patch.object(runtime.os,'open') as opened:
            with self.assertRaisesRegex(ValueError,'carry predecode lifetime'):
                runtime._require_offline_fit_carry(raw,read_charge=lambda packet:None,
                    live_bytes=core.SEMANTIC-2*core.CHUNK-262144-100)
            decoded.assert_not_called();opened.assert_not_called()

    def test_actual_descriptor_fits_full_history_admission_caller(self):
        # Actual complete mapping, with the measured207-history caller charge.
        # This is a projection regression, never full offline-fit acceptance.
        descriptor=fixed.selected_descriptor(live_bytes=653874)
        self.assertEqual(len(descriptor['slot']['rows']),40)
        self.assertEqual(len(descriptor['maxima']),81)
        self.assertFalse(descriptor['complete_fit'])

    def test_full_core_audit_preserves_original131072_limit(self):
        # NONHISTORICAL whole v1 audit, larger than unrelated4KiB closeout.
        with tempfile.TemporaryDirectory(dir=ARTIFACTS,prefix='core-audit-boundary-') as directory:
            value=dict(schema='pa1-completion-v1',binding='a'*64,context={},
                generation=200000,unit_verdicts=['selected40-semantic:'+str(i) for i in range(582)],
                accepted=False,authority=False)
            audit=put(Path(directory)/'NONHISTORICAL-core-audit.json',value)
            self.assertGreater(audit['bytes'],4096)
            self.assertEqual(core.RECEIPT,131072)
            admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
            admission._graphs=[];admission.read_trace=[];admission._full_read_charge=engineering_charge
            self.assertEqual(admission._load(audit,core.RECEIPT,decoded=True,
                completion_receipt=True),value)
            with self.assertRaisesRegex(ValueError,'decoded body cap'):
                admission._load(audit,core.RECEIPT,decoded=True)

    def test_core_audit_role_cannot_change_decoder_bound(self):
        from unittest.mock import patch
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission._graphs=[];admission.read_trace=[];admission._full_read_charge=None
        missing=dict(path=str(ARTIFACTS/'NONHISTORICAL-NOT-OPENED'),bytes=1,sha256='a'*64)
        with patch.object(runtime.os,'open') as opened:
            for maximum,decoded in ((32768,True),(core.RECEIPT,False)):
                with self.assertRaisesRegex(ValueError,'exact core completion decoding role'):
                    admission._load(missing,maximum,decoded=decoded,completion_receipt=True)
            opened.assert_not_called()


class ProspectiveRemainingControlTests(unittest.TestCase):
    """NONHISTORICAL field/read/lifetime tests; no complete-fit claim."""
    def fixture(self):
        from types import SimpleNamespace
        def p(name,body=b''):
            return dict(path='/home/fry/artifacts/NONHISTORICAL-control212-'+name,
                bytes=len(body),sha256=hashlib.sha256(body).hexdigest())
        log=constructors.prospective_log(31,maxima=constructors.ROLE_MAXIMA,
            command=['rtk','proxy','git','commit','-m','fix(evm): specimen'],live_bytes=0)
        sources=[p('source-%d'%i) for i in range(101)]
        value=dict(schema=constructors.PROSPECTIVE_MODE,
            command=['rtk','proxy','git','commit','-m','fix(evm): specimen'],
            exit_code=2147483647,hooks_path='.githooks',log=p('log',log),
            no_push=True,normal_hook=True,owned_sources=sources,parent='b'*40,
            source_acceptance=p('acceptance'),source_commit='c'*40)
        raw=core.canonical(value);planned=p('planned-control',raw);calls=[]
        unit=SimpleNamespace(invocation=SimpleNamespace(metadata_charge=lambda:0),
            declaration_charge=0,consume=lambda occurrence,pin,**kw:calls.append((occurrence,pin,kw)))
        return value,raw,planned,log,unit,calls

    def declarations(self,pins,kinds=None):
        kinds=kinds or ['evidence']*len(pins)
        return [(i,fixed._pin_tuple(p),kind,'bytes') for i,(p,kind) in enumerate(zip(pins,kinds))]

    def test_commit_model_reads_all101_sources_and_actual_consumer_rejects(self):
        value,raw,planned,log,unit,calls=self.fixture()
        pins=[planned,value['source_acceptance'],value['log'],*value['owned_sources']]
        args=dict(maxima=constructors.ROLE_MAXIMA,planned_raw=raw,planned_pin=planned,
            source_commit=value['source_commit'],source_acceptance=value['source_acceptance'],
            log=value['log'],owned_sources=value['owned_sources'],unit=unit,
            declarations=self.declarations(pins,['evidence']*3+['source']*101),live_bytes=0)
        model=json.loads(constructors.prospective_source_commit_record(**args))
        self.assertEqual(model['schema'],constructors.PROSPECTIVE_MODE)
        self.assertEqual([x[0] for x in calls],list(range(104)))
        with self.assertRaisesRegex(ValueError,'normal-hook local commit'):
            constructors.source_commit_record(maxima=constructors.ROLE_MAXIMA,
                actual_raw=raw,actual_pin=planned,source_commit=value['source_commit'],
                source_acceptance=value['source_acceptance'],log=value['log'],
                owned_sources=value['owned_sources'],live_bytes=0)
        calls.clear();args['live_bytes']=core.SEMANTIC
        with self.assertRaisesRegex(ValueError,'predecode lifetime'):
            constructors.prospective_source_commit_record(**args)
        self.assertEqual(calls,[])

    def test_hook_reads_complete_log_and_cannot_invent_passing_tests(self):
        value,raw,planned,log,unit,calls=self.fixture()
        args=dict(maxima=constructors.ROLE_MAXIMA,planned_raw=raw,planned_pin=planned,
            full_log=log,unit=unit,declarations=self.declarations([planned,value['log']]),live_bytes=0)
        model=json.loads(constructors.prospective_normal_hook_summary(**args))
        self.assertEqual(model['schema'],constructors.PROSPECTIVE_MODE)
        self.assertEqual([x[0] for x in calls],[0,1])
        self.assertNotEqual(model['exit_code'],0)
        with self.assertRaisesRegex(ValueError,'normal-hook local commit'):
            constructors.normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
                actual_raw=raw,actual_pin=planned,full_log=log,live_bytes=0)
        bad=log+b'test result: ok. 1 passed; 0 failed;\n'
        # Keep within the original log cap, bind both complete changed bodies.
        first=log.partition(b'\n')[0]+b'\n'
        bad=first+b'test result: ok. 1 passed; 0 failed;\n'
        value['log'].update(bytes=len(bad),sha256=hashlib.sha256(bad).hexdigest())
        raw=core.canonical(value);planned.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        args.update(planned_raw=raw,planned_pin=planned,full_log=bad,
            declarations=self.declarations([planned,value['log']]))
        with self.assertRaisesRegex(ValueError,'cannot claim passing tests'):
            constructors.prospective_normal_hook_summary(**args)

    def test_verification_keeps_all_ledger_fields_and_full_freeze_equality(self):
        value,raw,planned,log,unit,calls=self.fixture()
        freeze=dict(planned);exact=value['source_acceptance']
        ledger=dict(actual_binary_bytes=0,actual_bytes=31,actual_nonbinary_bytes=31,
            actual_source_bytes=17,charged_nonbinary_bytes=41,charged_total_bytes=41,
            open_occurrences=2,pending_and_outcome_reserved_bytes=sum(constructors.ROLE_MAXIMA[57:]))
        copies=RuntimeCommandConstructorTests().args(66)['before']
        args=dict(maxima=constructors.ROLE_MAXIMA,freeze=freeze,exact_commands_pin=exact,
            source_commit='c'*40,rebuilt_freeze_raw=raw,actual_freeze_raw=raw,
            model_reads=ledger,phase_peaks={'closed-model':1234},
            history=dict(body_bytes=1,charged_bytes=2,containers=3,members=204),
            copy_metadata=copies,child_occurrences=64,strict_pending=1,unit=unit,
            declarations=self.declarations([freeze,exact]),live_bytes=0)
        model=json.loads(constructors.prospective_command_verification(**args))
        self.assertEqual(model['actual_reads'],ledger)
        self.assertEqual(model['schema'],constructors.PROSPECTIVE_MODE)
        self.assertFalse(model['ready']);self.assertEqual([x[0] for x in calls],[0,1])
        args['rebuilt_freeze_raw']=raw+b' '
        with self.assertRaisesRegex(ValueError,'full freeze comparison'):
            constructors.prospective_command_verification(**args)
        args['rebuilt_freeze_raw']=raw;args['model_reads']=dict(ledger)
        del args['model_reads']['actual_source_bytes']
        with self.assertRaisesRegex(ValueError,'complete verification ledger'):
            constructors.prospective_command_verification(**args)

    def test_decision_reads_ready_and_helper_without_granting_authority(self):
        value,raw,planned,log,unit,calls=self.fixture()
        pins=[planned,value['source_acceptance'],value['log'],value['owned_sources'][0]]
        args=dict(maxima=constructors.ROLE_MAXIMA,source_commit='c'*40,
            command=['rtk','proxy','false'],
            inner_command=['false','--bundle-output','/home/fry/artifacts/NONHISTORICAL-bundle',
                '--report-output','/home/fry/artifacts/NONHISTORICAL-report'],
            freeze=pins[0],exact_commands_pin=pins[1],ready=pins[2],execution_helper=pins[3],
            unit=unit,declarations=self.declarations(pins,['evidence']*3+['source']),live_bytes=0)
        model=json.loads(constructors.prospective_capture_execution_decision(**args))
        self.assertEqual(model['schema'],constructors.PROSPECTIVE_MODE)
        self.assertEqual(model['status'],constructors.PROSPECTIVE_MODE)
        self.assertEqual(model['new_facts'],0);self.assertTrue(model['no_retry'])
        self.assertEqual([x[0] for x in calls],[0,1,2,3])
        args['inner_command'][1]='--wrong'
        with self.assertRaisesRegex(ValueError,'native argv'):
            constructors.prospective_capture_execution_decision(**args)

    def test_commit_node_workspace_is_checked_before_decode(self):
        from unittest.mock import patch
        value,raw,planned,log,unit,calls=self.fixture()
        raw=core.canonical([0]*6000)
        planned.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        pins=[planned,value['source_acceptance'],value['log'],*value['owned_sources']]
        declarations=self.declarations(pins,['evidence']*3+['source']*101)
        inputs=(raw,planned,value['source_commit'],value['source_acceptance'],
            value['log'],value['owned_sources'],declarations)
        external=(core.SEMANTIC-2*fixed.graph_charge(inputs)
            -3*constructors.ROLE_MAXIMA[30]-core.CHUNK-262144-10)
        self.assertGreaterEqual(external,0)
        with patch.object(constructors.records,'decode') as decode:
            with self.assertRaisesRegex(ValueError,'commit predecode lifetime'):
                constructors.prospective_source_commit_record(maxima=constructors.ROLE_MAXIMA,
                    planned_raw=raw,planned_pin=planned,source_commit=value['source_commit'],
                    source_acceptance=value['source_acceptance'],log=value['log'],
                    owned_sources=value['owned_sources'],unit=unit,declarations=declarations,
                    live_bytes=external)
            decode.assert_not_called()
        self.assertEqual(calls,[])

    def test_hook_header_graph_is_reserved_before_header_decode(self):
        from unittest.mock import patch
        value,raw,planned,log,unit,calls=self.fixture()
        log=core.canonical(dict(mode=constructors.PROSPECTIVE_MODE,executed=False,values=[0]*6000))
        value['log'].update(bytes=len(log),sha256=hashlib.sha256(log).hexdigest())
        raw=core.canonical(value);planned.update(bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
        strict=constructors.records.decode
        with patch.object(constructors.records,'decode',wraps=strict) as decode:
            with self.assertRaisesRegex(ValueError,'header predecode lifetime'):
                constructors.prospective_normal_hook_summary(maxima=constructors.ROLE_MAXIMA,
                    planned_raw=raw,planned_pin=planned,full_log=log,unit=unit,
                    declarations=self.declarations([planned,value['log']]),live_bytes=800000)
            self.assertEqual(decode.call_count,1)
        self.assertEqual(calls,[])


class NativeFutureValueDomainTests(unittest.TestCase):
    def test_unavailable_history_full_identity_exceeds_member_by_two_bytes(self):
        # NONHISTORICAL width-domain regression from the pinned Rust variants.
        selection=[dict(tuple=dict(identity=dict(period=25706894+i,
            state_root_hex='a'*64))) for i in range(40)]
        values=constructors._prospective_replay_values(selection)
        member=[dict(logical='member',physical=dict(kind='present',bytes=32,sha256='0'*64),
            proof_index=51+i,usable_vrf_key=False,value_hex='0'*64) for i in range(40)]
        self.assertEqual(len(core.canonical(values))-len(core.canonical(member)),80)
        self.assertEqual([v['physical']['identity'] for v in values],
            [s['tuple']['identity'] for s in selection])
        self.assertTrue(all(v['logical']=='nonmember' and v['value_hex'] is None
            and v['usable_vrf_key'] is False for v in values))
        with self.assertRaisesRegex(ValueError,'full40 value domains'):
            constructors._prospective_replay_values(selection[:-1])
        selection[0]['tuple']['identity']['state_root_hex']='a'*65
        with self.assertRaisesRegex(ValueError,'physical history identity'):
            constructors._prospective_replay_values(selection)


class NativeRole68StopModelTests(unittest.TestCase):
    def base(self):
        # NONHISTORICAL transform fixture; full native wire constructors still
        # perform all original authority/selection/source reads and predicates.
        return dict(schema=constructors.PROSPECTIVE_MODE,status=constructors.PROSPECTIVE_MODE,
            qualification={'vrf_family_qualified':False},bundle={'unchanged':'not evidence'},
            actual=dict(admitted_input_bytes=1048576,resources=[
                dict(resource=n,used=v) for n,v in constructors.NATIVE_RESOURCE_LIMITS]),
            values=[{'unexecuted':True}],authority={'preserved':'full existing input graph'})

    def test_capture_stop_keeps_authority_usage_and_full_observed_limit(self):
        value=self.base();authority=value['authority'];resources=value['actual']['resources']
        result=constructors._prospective_role68_stopped(69,value,67109268)
        self.assertIs(result['authority'],authority)
        self.assertIs(result['actual']['resources'],resources)
        self.assertNotIn('bundle',result);self.assertIsNone(result['values'])
        self.assertEqual(result['failure'],dict(phase='Fixed40RoleEnvelope',
            reason='bundle exceeds fixed40 role68 envelope',observed_bytes=67109268,limit_bytes=1048576))
        self.assertEqual(result['schema'],constructors.PROSPECTIVE_MODE)
        self.assertFalse(any(result['qualification'].values()))
        for observed in (1048576,67109269,True):
            with self.assertRaisesRegex(ValueError,'stopped domain'):
                constructors._prospective_role68_stopped(69,self.base(),observed)

    def test_replay_stop_is_before_any_engine_input_and_covers_u64_metadata(self):
        value=self.base();authority=value['authority']
        result=constructors._prospective_role68_stopped(75,value,18446744073709551615)
        self.assertIs(result['authority'],authority)
        self.assertEqual(result['actual']['admitted_input_bytes'],0)
        self.assertTrue(all(r['used']==0 for r in result['actual']['resources']))
        self.assertNotIn('bundle',result);self.assertIsNone(result['values'])
        self.assertEqual(result['failure']['observed_bytes'],18446744073709551615)
        with self.assertRaisesRegex(ValueError,'stopped domain'):
            constructors._prospective_role68_stopped(75,self.base(),18446744073709551616)


class DeferredCompleteFitReviewTests(unittest.TestCase):
    def test_complete_closed_obligation_reads_and_rejects_authority_or_growth(self):
        # NONHISTORICAL format/binding fixture. These are not gate results,
        # a runtime profile acceptance, reviewer evidence or physical proof.
        root=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-role29"
        root.mkdir(parents=True,exist_ok=False)
        journal=(root/"before-open-read-debits.jsonl").open("x")
        total=0;published=0
        def read_file(p,expected=None):
            nonlocal total
            size=p.stat().st_size;total+=size+1
            if total>8388608:raise ValueError("role29 unchanged8MiB fixture subdivision")
            row=dict(path=str(p),bytes=size,bytes_plus_EOF=size+1,
                     cumulative=total,phase="BEFORE_OPEN",binary=0)
            if expected is not None:row["sha256"]=expected["sha256"]
            journal.write(json.dumps(row,sort_keys=True)+"\n")
            journal.flush();os.fsync(journal.fileno())
            body=p.read_bytes()
            if expected is not None:
                self.assertEqual(len(body),expected["bytes"])
                self.assertEqual(hashlib.sha256(body).hexdigest(),expected["sha256"])
            return body
        def publish(name,value):
            nonlocal published
            body=core.canonical(value);published+=len(body)
            if published>131072:raise ValueError("role29 complete fixture body storage")
            path=root/name
            with path.open("xb") as stream:stream.write(body)
            return dict(path=str(path),bytes=len(body),sha256=hashlib.sha256(body).hexdigest()),body
        try:
            index=json.loads(read_file(Path(os.environ["BRIDGE_SOURCE_FREEZE"])))
            sources=[{k:row[k] for k in ("path","bytes","sha256")} for row in index["files"]]
            self.assertEqual(len(sources),101)
            self.assertLessEqual(sum(row["bytes"] for row in sources),core.SOURCE)
            profile_original=Path("/home/fry/artifacts/vrf40-role72-recovery-20261019/compiler115-placement-v3/runtime-profile-PREPARATION.json")
            original_pin=dict(path=str(profile_original),bytes=21152,
                sha256="bfdcdee340c15c850f964dde70f87aac276c7b5a006d01087cc8980a1ade83fc")
            profile=json.loads(read_file(profile_original,original_pin))
            # Preserve the complete profile field graph. Only this format
            # fixture source roster/identity changes; RuntimeAdmission is not
            # invoked and this modified profile cannot supply acceptance.
            profile["sources"]=sources;profile["source_identity"]=core.digest(sources)
            profile_pin,profile_raw=publish("profile.NONHISTORICAL.json",profile)
            evidence,_=publish("unexecuted.NONHISTORICAL.json",
                dict(mode=constructors.PROSPECTIVE_MODE,executed=False,accepted=False))
            gates=[dict(index=role,result=dict(evidence),log=dict(evidence))
                   for role in constructors.VALIDATION_RESULTS]
            refs={name:dict(evidence) for name in ("source_acceptance","source_freeze",
                "tool_source_freeze","source_commit_record","actual_hook","hook_summary",
                "authority","bridge_scope","cache_scope")}
            known={name:"a"*64 for name in ("source","verifier","policy","ledger",
                "setup_policy","setup_ledger","setup_publication","event_history")}
            known.update(run="NONHISTORICAL-role29-format",
                source=core.digest((sources[:90],sources[90:])))
            context_fields=sorted({"run","manifest","source","verifier","executables",
                "phase","route","policy","ledger","setup_policy","setup_ledger",
                "setup_publication","event_history"})
            routes=list(fixed.framing_routes())
            obligation=dict(schema="fixed40-deferred-complete-fit-review-v1",
                mode=constructors.PROSPECTIVE_MODE,history="NONHISTORICAL",status="NOT_EXECUTED",
                accepted=False,all16_accepted=False,production_authority=False,
                current_profile=profile_pin,source_identity=profile["source_identity"],
                sources=sources,gates=gates,bindings=refs,
                context_derivation=dict(schema="fixed40-context-derivation-v1",round_index=0,
                    routes=routes,context_fields=context_fields,known=known,
                    manifest_slots=[dict(route=route,global_index=i,
                        local_ordinal=int(route.rsplit(":",1)[1]),
                        kind="exact-frozen-manifest-digest",state="RESOLVE_FROM_FULL_PROGRAM")
                        for i,route in enumerate(routes)],
                    executable_slots=[dict(path=str(root/("unexecuted-executable-"+str(i))),
                        bytes=1,kind="OWNED_FULL_SHA256_RESOLUTION",
                        owner="accepted-bootstrap-schedule") for i in range(2)],
                    phase_format="development:{round}:0:{local_ordinal}"),
                descriptor_template=evidence,schedule_template=evidence,
                future_review_body_maximum=131072,actual_review_required=True)
            obligation_pin,obligation_raw=publish("deferred.NONHISTORICAL.json",obligation)
            class Reader:
                def __init__(self,pins):
                    self.declarations=tuple((i,fixed._pin_tuple(pin),
                        "source" if i>=len(pins)-101 else "evidence","bytes")
                        for i,pin in enumerate(pins))
                    self.declaration_charge=fixed.graph_charge(self.declarations)
                    self.invocation=SimpleNamespace(metadata_charge=lambda:1024)
                    self.used=[]
                def consume(inner,occurrence,pin,*,kind,live_bytes,**options):
                    row=inner.declarations[occurrence]
                    self.assertEqual(row[1],fixed._pin_tuple(pin));self.assertEqual(row[2],kind)
                    self.assertNotIn(occurrence,inner.used)
                    inner.used.append(occurrence)
                    body=read_file(Path(pin["path"]),pin)
                    return body if options.get("retain") else None
            complete_refs=dict(refs,complete_offline_fit_review=obligation_pin)
            # Match the constructor exact insertion order, including repeated
            # obligation/evidence occurrences; no deduplication read credit.
            ordered_refs={k:complete_refs[k] for k in ("source_acceptance","source_freeze",
                "tool_source_freeze","source_commit_record","actual_hook","hook_summary",
                "complete_offline_fit_review","authority","bridge_scope","cache_scope")}
            pins=(obligation_pin,profile_pin,*ordered_refs.values(),
                *(pin for row in gates for pin in (row["result"],row["log"])),
                evidence,evidence,*sources)
            unit=Reader(pins)
            args=dict(maxima=constructors.ROLE_MAXIMA,gates=gates,source_commit="1"*40,
                **complete_refs,deferred_review_raw=obligation_raw,
                current_profile_raw=profile_raw,source_bindings=sources,
                review_body_maximum=131072,unit=unit,declarations=unit.declarations,live_bytes=0)
            # NONHISTORICAL allocation/order regression. Grow the explicit
            # caller until the previous simultaneous decode budget fails;
            # both full documents and every143 occurrence remain mandatory.
            from unittest.mock import patch
            import sys
            input_names=("gates","source_commit","source_acceptance","source_freeze",
                "tool_source_freeze","source_commit_record","actual_hook","hook_summary",
                "complete_offline_fit_review","authority","bridge_scope","cache_scope",
                "deferred_review_raw","current_profile_raw","source_bindings")
            inputs=tuple(args[k] for k in input_names)+(unit.declarations,)
            previous_required=(1024+unit.declaration_charge+2*fixed.graph_charge(inputs)
                +3*(len(profile_raw)+len(obligation_raw))
                +160*(core._wire_nodes(profile_raw)+core._wire_nodes(obligation_raw))
                +3*constructors.ROLE_MAXIMA[29]+core.CHUNK+262144)
            observer=32768+fixed.graph_charge((profile_raw,obligation_raw))
            args["live_bytes"]=max(observer,core.SEMANTIC-previous_required+1)
            self.assertGreater(previous_required+args["live_bytes"],core.SEMANTIC)
            import evidence_records as records
            decoded=[]
            original_decode=records.decode
            def decode_in_order(raw,*positional,**keywords):
                if raw is profile_raw:decoded.append("profile")
                elif raw is obligation_raw:
                    frame=sys._getframe(1)
                    self.assertNotIn("profile",frame.f_locals)
                    binding=frame.f_locals["profile_binding"]
                    self.assertEqual(binding,(len(profile_raw),
                        hashlib.sha256(profile_raw).hexdigest(),profile["source_identity"]))
                    decoded.append("review")
                    del frame,binding
                return original_decode(raw,*positional,**keywords)
            with patch.object(records,"decode",new=decode_in_order):
                result=json.loads(constructors.prospective_validation_summary(**args))
            self.assertEqual(decoded,["profile","review"])
            self.assertEqual(result["schema"],constructors.PROSPECTIVE_MODE)
            self.assertFalse(result["all16_accepted"]);self.assertFalse(result["production_authority"])
            self.assertTrue(result["actual_review_required"])
            self.assertEqual(unit.used,list(range(len(pins))))
            self.assertNotEqual(known["source"],profile["source_identity"])
            bad=json.loads(obligation_raw)
            bad["context_derivation"]["known"]["source"]=profile["source_identity"]
            raw=core.canonical(bad)
            changed=dict(args,deferred_review_raw=raw,
                complete_offline_fit_review=dict(obligation_pin,bytes=len(raw),
                    sha256=hashlib.sha256(raw).hexdigest()))
            before=len(unit.used)
            with self.assertRaisesRegex(ValueError,"sixteen-context derivation"):
                constructors.prospective_validation_summary(**changed)
            self.assertEqual(len(unit.used),before)
            for key in ("accepted","all16_accepted","production_authority"):
                bad=dict(obligation);bad[key]=True;raw=core.canonical(bad)
                changed=dict(args,deferred_review_raw=raw,
                    complete_offline_fit_review=dict(obligation_pin,bytes=len(raw),
                        sha256=hashlib.sha256(raw).hexdigest()))
                before=len(unit.used)
                with self.assertRaisesRegex(ValueError,"nonhistorical"):
                    constructors.prospective_validation_summary(**changed)
                self.assertEqual(len(unit.used),before)
            before=len(unit.used)
            with self.assertRaisesRegex(ValueError,"predecode lifetime"):
                constructors.prospective_validation_summary(**dict(args,live_bytes=core.SEMANTIC))
            self.assertEqual(len(unit.used),before)
            self.assertEqual(fixed._fit_constructor_name(29),"prospective_validation_summary")
            self.assertLessEqual(len(obligation_raw),65536)
        finally:
            journal.close()


class CaptureIOComponentTests(unittest.TestCase):
    def test_failure_producer_cost_is_separate_from_accepted_model_envelope(self):
        maxima=constructors.ROLE_MAXIMA
        # NONHISTORICAL cost-format fixtures, never file/proof evidence.
        raw=[dict(path="/NONHISTORICAL/raw-"+str(i),bytes=1,sha256="a"*64) for i in range(7)]
        raw[-1]["bytes"]=6785630
        outputs=[dict(role=role,path="/NONHISTORICAL/output-"+str(role),
            accepted_maximum=maxima[role],producer_maximum=(67109268 if role==68
                else 131072 if role==69 else maxima[role])) for role in range(64,70)]
        io=dict(native_reads=6785643,
            native_output_writes=sum(x["producer_maximum"] for x in outputs),
            pipe_reads=393218,native_output_rereads=67109268+131072+2,
            marker_reads=maxima[63]+1,marker_writes=maxima[63],
            stdout_success=65536,stderr_success=65536,
            stdout_failure=196608,stderr_failure=196608,
            result_publication=maxima[64],closeout_bytes=32768,
            invocation_seconds=300,kill_reap_seconds=5)
        value=dict(schema="fixed40-production-capture-operation-v1",profile="ResidualE946",
            route="capture",index=0,global_index=4,phase="production:0:0:0",
            constructor_role=63,unit="NONHISTORICAL-real-program-unit-is-required-later",
            source_identity="a"*64,native_inputs=raw,outputs=outputs,
            marker=dict(path="/NONHISTORICAL/one-attempt",maximum=maxima[63],
                exclusive=True,fsync=True),
            storage_owner=dict(path="/NONHISTORICAL/not-a-storage-grant",bytes=1,sha256="b"*64),io=io)
        result=fixed.capture_operation_reservation(value,maxima,live_bytes=0)
        self.assertGreater(result["write_bytes"],67109268)
        self.assertFalse(result["physical_permission"]);self.assertFalse(result["accepted"])
        self.assertEqual(maxima[68],1048576)
        old=outputs[4]["producer_maximum"];outputs[4]["producer_maximum"]=maxima[68]
        with self.assertRaisesRegex(ValueError,"full failed producer ceiling"):
            fixed.capture_operation_reservation(value,maxima,live_bytes=0)
        outputs[4]["producer_maximum"]=old
        value["phase"]="production:0:0:4"
        with self.assertRaisesRegex(ValueError,"sole capture route"):
            fixed.capture_operation_reservation(value,maxima,live_bytes=0)

    def test_actual_synthetic_child_stderr_and_exit_are_retained_without_native_launch(self):
        import sys,time
        root=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-capture-streams"
        root.mkdir(parents=True,exist_ok=False)
        journal=(root/"before-operation-debits.jsonl").open("x")
        def debit(row):
            wire=json.dumps(row,sort_keys=True)+"\n"
            if journal.tell()+len(wire.encode())>32768:raise ValueError("bounded fixture operation journal")
            journal.write(wire);journal.flush();os.fsync(journal.fileno())
        try:
            now=time.monotonic()
            result=fixed._supervise_capture_streams([
                sys.executable,"-c","import sys;print(\"probe\");print(\"probe-stderr\",file=sys.stderr);sys.exit(7)"],
                cwd=str(fixed.ROOT),environment={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"},
                stdout_path=str(root/"stdout"),stderr_path=str(root/"stderr"),
                child_deadline=now+2,closeout_deadline=now+7,debit=debit,live_bytes=0)
            self.assertTrue(result["launched"]);self.assertTrue(result["capture_complete"])
            self.assertEqual(result["exit_code"],7);self.assertEqual(result["reason"],"nonzero_exit")
            self.assertEqual(result["stdout"]["bytes"],6);self.assertEqual(result["stderr"]["bytes"],13)
            self.assertEqual(result["stderr"]["sha256"],hashlib.sha256(b"probe-stderr\n").hexdigest())
            self.assertFalse(result["physical_permission"]);self.assertFalse(result["accepted"])
            body=core.canonical(result)
            self.assertLess(len(body),16384)
            debit(dict(operation="fixture_result_write",bytes=len(body),phase="BEFORE_CREATE"))
            with (root/"result.NONHISTORICAL.json").open("xb") as stream:stream.write(body)
        finally:journal.close()


    def test_synthetic_overflow_and_deadline_kill_reap_keep_actual_failure_streams(self):
        import sys,time
        base=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-capture-failures"
        base.mkdir(parents=True,exist_ok=False)
        cases=(("overflow","import os,time;os.write(1,b\"x\"*70000);time.sleep(10)",
                2,"output_limit",70000),
               ("timeout","import time;time.sleep(10)",0.1,"timeout",0),
               ("exited-leader","import subprocess,sys;subprocess.Popen([sys.executable,\"-c\",\"import time;time.sleep(10)\"])",0.2,"timeout",0))
        for name,script,seconds,reason,size in cases:
            root=base/name;root.mkdir()
            with (root/"before-operation-debits.jsonl").open("x") as journal:
                def debit(row):
                    wire=json.dumps(row,sort_keys=True)+"\n"
                    if journal.tell()+len(wire.encode())>32768:raise ValueError("bounded synthetic failure journal")
                    journal.write(wire);journal.flush();os.fsync(journal.fileno())
                now=time.monotonic()
                observed=fixed._supervise_capture_streams([sys.executable,"-c",script],
                    cwd=str(fixed.ROOT),environment={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"},
                    stdout_path=str(root/"stdout"),stderr_path=str(root/"stderr"),
                    child_deadline=now+seconds,closeout_deadline=now+seconds+5,
                    debit=debit,live_bytes=0)
                self.assertTrue(observed["capture_complete"])
                if name=="exited-leader":self.assertEqual(observed["exit_code"],0)
                else:self.assertLess(observed["exit_code"],0)
                self.assertTrue(observed["cleanup"]["leader_reaped"])
                self.assertTrue(observed["cleanup"]["pipe_writers_closed"])
                self.assertGreaterEqual(observed["cleanup"]["group_signal_deliveries"],1)
                self.assertFalse(observed["cleanup"]["all_descendants_reaped"])
                self.assertEqual(observed["reason"],reason)
                self.assertEqual(observed["stdout"]["bytes"],size)
                self.assertEqual(observed["stderr"]["bytes"],0)
                self.assertFalse(observed["physical_permission"])
                self.assertLess(observed["elapsed_seconds"],seconds+5)
                body=core.canonical(observed)
                debit(dict(operation="fixture_result_write",bytes=len(body),phase="BEFORE_CREATE"))
                with (root/"result.NONHISTORICAL.json").open("xb") as out:out.write(body)

    def test_closed_pipes_do_not_extend_child_execution_deadline(self):
        import sys,time
        root=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-closed-pipes-deadline"
        root.mkdir(parents=True,exist_ok=False)
        with (root/"before-operation-debits.jsonl").open("x") as journal:
            def debit(row):
                raw=json.dumps(row,sort_keys=True)+"\n"
                self.assertLess(journal.tell()+len(raw.encode()),32768)
                journal.write(raw);journal.flush();os.fsync(journal.fileno())
            deadline=time.monotonic()+0.2
            result=fixed._supervise_capture_streams([sys.executable,"-c",
                "import os,time;os.close(1);os.close(2);time.sleep(2)"],
                cwd=str(fixed.ROOT),environment={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"},
                stdout_path=str(root/"stdout"),stderr_path=str(root/"stderr"),
                child_deadline=deadline,closeout_deadline=deadline+5,debit=debit,live_bytes=0)
            self.assertEqual(result["reason"],"timeout")
            self.assertLess(result["exit_code"],0)
            self.assertEqual(result["stream_io"]["eof"],[True,True])
            self.assertTrue(result["cleanup"]["leader_reaped"])
            self.assertLess(result["elapsed_seconds"],1.5)
            body=core.canonical(result)
            debit(dict(operation="fixture_result_write",bytes=len(body),phase="BEFORE_CREATE"))
            with (root/"result.NONHISTORICAL.json").open("xb") as out:out.write(body)

    def test_cleanup_errors_attempt_every_closer_and_preserve_primary_failure(self):
        import sys,time,subprocess,selectors
        from contextlib import ExitStack
        from unittest.mock import patch
        base=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-cleanup-exceptions"
        base.mkdir(parents=True,exist_ok=False)
        real_popen=subprocess.Popen;real_selector=selectors.DefaultSelector
        real_open=os.open;real_close=os.close
        for case in ("final-reap","callback","close-error","outer-except-close"):
            root=base/case;root.mkdir()
            primary=RuntimeError("NONHISTORICAL primary "+case)
            close_failure=OSError("NONHISTORICAL injected close error after real close")
            opened=[];closed=[];children=[];selector_objects=[]
            close_injected=[False]
            journal=(root/"before-operation-debits.jsonl").open("x")
            def debit(row):
                wire=json.dumps(row,sort_keys=True)+"\n"
                if journal.tell()+len(wire.encode())>32768:raise ValueError("finite exception fixture journal")
                journal.write(wire);journal.flush();os.fsync(journal.fileno())
                if case not in ("final-reap","outer-except-close") and row["operation"]=="stream_fsync":raise primary
            def opening(*args,**kwargs):
                fd=real_open(*args,**kwargs)
                if str(args[0]) in (str(root/"stdout"),str(root/"stderr")):opened.append(fd)
                return fd
            def closing(fd):
                real_close(fd)
                if fd in opened:
                    closed.append(fd)
                    if case in ("close-error","outer-except-close") and not close_injected[0]:
                        close_injected[0]=True
                        raise close_failure
            def spawning(*args,**kwargs):
                child=real_popen(*args,**kwargs)
                old_wait=child.wait
                children.append((child,old_wait))
                if case=="final-reap":
                    child.poll=lambda:None
                    def waiting(*a,**k):
                        raise subprocess.TimeoutExpired("NONHISTORICAL wait",k.get("timeout"))
                    child.wait=waiting
                return child
            def selecting():
                selector=real_selector();selector_objects.append(selector)
                if case=="final-reap":
                    def select(*args,**kwargs):raise primary
                    selector.select=select
                return selector
            now=time.monotonic();deadline=now+1;closeout=deadline+5
            error=None
            try:
                with ExitStack() as stack:
                    stack.enter_context(patch.object(os,"open",side_effect=opening))
                    stack.enter_context(patch.object(os,"close",side_effect=closing))
                    stack.enter_context(patch.object(subprocess,"Popen",side_effect=spawning))
                    stack.enter_context(patch.object(selectors,"DefaultSelector",side_effect=selecting))
                    try:
                        try:raise LookupError("NONHISTORICAL caller handled error")
                        except LookupError:
                            fixed._supervise_capture_streams([sys.executable,"-c",
                            "import time;time.sleep(10)" if case=="final-reap" else "pass"],
                            cwd=str(fixed.ROOT),
                            environment={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"},
                            stdout_path=str(root/"stdout"),stderr_path=str(root/"stderr"),
                            child_deadline=deadline,closeout_deadline=closeout,
                            debit=debit,live_bytes=0)
                    except BaseException as caught:error=caught
                self.assertIs(error,close_failure if case=="outer-except-close" else primary)
                self.assertEqual(len(opened),2)
                self.assertEqual(closed,opened)
                for fd in opened:
                    with self.assertRaises(OSError):os.fstat(fd)
                self.assertTrue(all(child.stdout.closed and child.stderr.closed for child,_ in children))
                self.assertTrue(all(selector.get_map() is None for selector in selector_objects))
                outcome=error.fixed40_observed_outcome
                self.assertTrue(outcome["launched"])
                self.assertFalse(outcome["physical_permission"])
                cleanup=outcome["cleanup"]
                self.assertGreaterEqual(cleanup["group_signal_attempts"],1)
                self.assertIs(type(cleanup["group_signal_deliveries"]),int)
                self.assertEqual(cleanup["pipe_close_attempts"],2)
                self.assertEqual(cleanup["sink_close_attempts"],2)
                self.assertTrue(cleanup["selector_close_attempted"])
                self.assertFalse(cleanup["all_descendants_reaped"])
                self.assertEqual(cleanup["leader_reaped"],case!="final-reap")
                self.assertEqual(outcome["exit_code"],None if case=="final-reap" else 0)
                self.assertEqual(outcome["stream_io"]["eof"],
                    [False,False] if case=="final-reap" else [True,True])
                self.assertEqual(cleanup["cleanup_complete"],case=="callback")
                notes=getattr(error,"__notes__",[])
                if case=="final-reap":self.assertTrue(any("TimeoutExpired" in note for note in notes))
                if case in ("close-error","outer-except-close"):
                    self.assertTrue(close_injected[0])
                    self.assertTrue(any("sink_close:0" in note and "OSError" in note for note in notes))
                record=dict(schema="NONHISTORICAL-cleanup-exception-v1",case=case,
                    primary_preserved=True,outer_handled_error_not_used=True,
                    primary_type=type(error).__name__,
                    cleanup_notes=notes,observed_outcome=outcome,opened=opened,closed=closed,
                    all_local_pipes_closed=True,selector_closed=True,
                    actual_guard_implemented=False,physical_permission=False,
                    all_descendants_reaped=False)
                body=core.canonical(record);self.assertLess(len(body),8192)
                debit(dict(operation="fixture_result_write",bytes=len(body),phase="BEFORE_CREATE"))
                with (root/"failure.NONHISTORICAL.json").open("xb") as out:out.write(body)
            finally:
                # A mocked wait cannot certify reap. The fixture independently
                # reaps its real synthetic child within the original closeout.
                for child,old_wait in children:
                    old_wait(timeout=max(0.001,closeout-time.monotonic()))
                journal.close()



class CaptureOutcomePublicationTests(unittest.TestCase):
    def test_actual_exit_and_callback_failure_have_durable_outcomes(self):
        import time,sys
        from unittest.mock import patch
        root=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-outcome-publication"
        root.mkdir()
        for failure in (False,True):
            target=root/("callback" if failure else "nonzero");target.mkdir()
            calls=[];primary=RuntimeError("NONHISTORICAL-debit-failure")
            cause=LookupError("NONHISTORICAL-original-cause");primary.__cause__=cause
            frames=[];canonical=core.canonical
            def observe_record(value):
                if type(value) is dict and value.get("schema")=="fixed40-durable-child-outcome-v1" and failure:
                    caller=sys._getframe(1)
                    self.assertIs(caller.f_locals["primary"],primary)
                    traceback=primary.__traceback__;found=None
                    while traceback is not None:
                        if traceback.tb_frame.f_code is fixed._supervise_capture_streams.__code__:
                            found=traceback.tb_frame;break
                        traceback=traceback.tb_next
                    self.assertIsNotNone(found)
                    self.assertEqual(len(found.f_locals["pipes"]),2)
                    self.assertEqual(len(found.f_locals["fds"]),2)
                    self.assertIn("block",found.f_locals)
                    self.assertIn("view",found.f_locals)
                    self.assertGreaterEqual(caller.f_locals["supervisor_frame_reserve"],8*core.CHUNK+65536)
                    frames.append(tuple(sorted(found.f_locals)))
                    del traceback,found,caller
                return canonical(value)
            def debit(event):
                calls.append(event["operation"])
                if failure and event["operation"]=="stream_fsync":raise primary
            now=time.monotonic()
            arguments=dict(cwd=str(fixed.ROOT),environment={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"},
                stdout_path=str(target/"stdout"),stderr_path=str(target/"stderr"),
                outcome_path=str(target/"outcome.json"),child_deadline=now+2,
                closeout_deadline=now+7,debit=debit,live_bytes=32768)
            command=[sys.executable,"-c","import sys;print(123);print(456,file=sys.stderr);sys.exit(7)"]
            if failure:
                with patch.object(core,"canonical",new=observe_record):
                    with self.assertRaises(RuntimeError) as raised:
                        fixed._supervise_capture_with_outcome(command,**arguments)
                self.assertIs(raised.exception,primary)
                self.assertIs(raised.exception.__cause__,cause)
                self.assertEqual(len(frames),1)
                publication=primary.fixed40_outcome_publication
                self.assertTrue(publication["complete"])
                self.assertIsNone(publication["error_type"])
                self.assertEqual(calls[-1],"stream_fsync")
                pin=publication["pin"]
            else:
                result=fixed._supervise_capture_with_outcome(command,**arguments)
                self.assertTrue(result["publication_complete"])
                self.assertFalse(result["physical_permission"])
                pin=result["outcome"]
            raw=Path(pin["path"]).read_bytes()
            self.assertEqual(len(raw),pin["bytes"])
            self.assertEqual(hashlib.sha256(raw).hexdigest(),pin["sha256"])
            record=json.loads(raw);observed=record["observed"]
            self.assertEqual(record["outcome_availability"],"ACTUAL")
            self.assertEqual(observed["exit_code"],7)
            self.assertTrue(observed["capture_complete"])
            self.assertTrue(observed["cleanup"]["leader_reaped"])
            self.assertEqual((target/"stdout").read_bytes(),b"123\n")
            self.assertEqual((target/"stderr").read_bytes(),b"456\n")
            self.assertEqual(calls[0],"reserve_outcome_publication")

    def test_retained_frame_crossing_limit_rejects_before_launch(self):
        import sys,time
        root=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-carry-rejection"
        root.mkdir();calls=[];now=time.monotonic()
        with self.assertRaisesRegex(ValueError,"outcome publication lifetime"):
            fixed._supervise_capture_with_outcome([sys.executable,"-c","raise SystemExit(99)"],
                cwd=str(fixed.ROOT),environment={"PATH":"/usr/bin:/bin","LANG":"C","LC_ALL":"C"},
                stdout_path=str(root/"stdout"),stderr_path=str(root/"stderr"),
                outcome_path=str(root/"outcome.json"),child_deadline=now+2,
                closeout_deadline=now+7,debit=calls.append,live_bytes=1200000)
        self.assertEqual(calls,[])
        self.assertEqual(list(root.iterdir()),[])

    def test_direct_cli_defines_capture_helpers_before_entry(self):
        import subprocess,sys
        root=Path(os.environ["DEVELOPMENT_TEST_ARTIFACT_ROOT"])/"NONHISTORICAL-direct-cli"
        root.mkdir()
        command=["rtk","proxy",sys.executable,"/workspaces/rustaxa-evm/scripts/capture_command.py",
            "--output-dir",str(root/"capture"),"--timeout-seconds","10",
            "--max-output-bytes","65536","--","rtk","proxy",sys.executable,
            str(Path(fixed.__file__)),"--help"]
        # Both runner streams exist exclusively BEFORE the subprocess starts;
        # the existing capture helper likewise owns child sinks before launch.
        with (root/"runner.stdout").open("xb") as stdout,(root/"runner.stderr").open("xb") as stderr:
            result=subprocess.run(command,cwd=str(fixed.ROOT),stdout=stdout,stderr=stderr,timeout=15)
        (root/"runner.exit").write_text(str(result.returncode)+"\n")
        self.assertEqual(result.returncode,0)
        output=(root/"capture/stdout").read_bytes()
        self.assertLessEqual(len(output)+(root/"capture/stderr").stat().st_size,65536)
        self.assertIn(b"usage",output.lower())



class ActualMatrixWireContractTests(unittest.TestCase):
    def test_closed_runtime_wire_and_complete_output_hold(self):
        # NONHISTORICAL format inputs only: no Invocation or native execution.
        import copy
        pin=dict(path="/NONHISTORICAL/matrix-proof",bytes=1,sha256="a"*64)
        rows=[]
        for i,route in enumerate(fixed.framing_routes()):
            name,index=route.rsplit(':',1)
            context=dict(route=name,phase="development:0:0:"+index,manifest=hex(i)[2:].zfill(64))
            rows.append(dict(route=name,index=int(index),context=context,receipt=pin,result=pin))
        matrix=fixed.actual_fit_matrix("b"*64,pin,0,rows)
        self.assertEqual(set(matrix),{"schema","source_identity","preflight","round","results"})
        self.assertEqual(matrix["schema"],"fixed40-actual-input-offline-fit-matrix-v1")
        self.assertEqual(len(matrix["results"]),16)
        with self.assertRaisesRegex(ValueError,"requires16"):
            fixed.actual_fit_matrix("b"*64,pin,0,rows[:-1])
        bad=copy.deepcopy(rows);bad[0]["extra"]=True
        with self.assertRaisesRegex(ValueError,"context order"):
            fixed.actual_fit_matrix("b"*64,pin,0,bad)
        bad=copy.deepcopy(rows);bad[0]["index"]=1
        with self.assertRaisesRegex(ValueError,"context order"):
            fixed.actual_fit_matrix("b"*64,pin,0,bad)
        hold=fixed.matrix_output_reservation("/NONHISTORICAL/matrix")
        self.assertEqual(hold["per_attempt_bytes"],34*core.RECEIPT)
        self.assertEqual(hold["total_bytes"],4*34*core.RECEIPT)
        self.assertFalse(hold["read_bank"])
        self.assertFalse(hold["accepted"])



class MatrixCliOwnerTests(unittest.TestCase):
    """NONHISTORICAL controller tests; no fit Invocation or native execution."""
    def test_real_capacity_body_requires_full_artifact_owner_and_fsync(self):
        from unittest.mock import patch
        import tempfile
        root=Path(os.environ['DEVELOPMENT_TEST_ARTIFACT_ROOT'])
        with tempfile.TemporaryDirectory(dir=root,prefix='NONHISTORICAL-owner-') as name:
            directory=Path(name);output=str(directory/'matrix');ledger=str(directory/'debits.jsonl')
            profile=dict(path='/NONHISTORICAL/profile',bytes=1,sha256='a'*64)
            contexts=[dict(NONHISTORICAL=i) for i in range(16)]
            binding=dict(schema='fixed40-matrix-storage-binding-v1',activation=fixed.MATRIX_STORAGE_ACTIVATION,
                profile=profile,contexts_digest=core.digest(contexts),directory=output,ledger=ledger,round=0,phase='normal')
            raw=b''.join(core.canonical([section,{'matrix_storage':binding} if section=='controls'
                else {'NONHISTORICAL':True}]) for section in fixed.COMPLETE_FIT_SECTIONS)
            path=directory/'framing.jsonl';path.write_bytes(raw)
            representation=dict(path=str(path),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())
            request=dict(profile=profile,contexts=contexts,matrix_storage=binding,representation=representation)
            calls=[];total=0
            def charge(packet):
                calls.extend(packet)
                with (root/'NONHISTORICAL-owner-proof-reads.jsonl').open('ab') as f:
                    f.write((json.dumps(dict(scope='BEFORE_OPEN full framing',packet=packet))+'\n').encode());f.flush();os.fsync(f.fileno())
            def full(pin):
                nonlocal total
                total+=pin['bytes']+1
                if total>1048576:raise ValueError('bounded real capacity proof reads')
                with (root/'NONHISTORICAL-owner-proof-reads.jsonl').open('ab') as f:
                    f.write((json.dumps(dict(pin=pin,bytes_plus_EOF=pin['bytes']+1,cumulative=total))+'\n').encode());f.flush();os.fsync(f.fileno())
                with Path(pin['path']).open('rb') as f:body=f.read(pin['bytes']+1)
                if len(body)!=pin['bytes'] or hashlib.sha256(body).hexdigest()!=pin['sha256']:raise ValueError('full proof pin')
                return body
            fd=os.open(ledger,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
            try:
                with patch.object(fixed,'_read',side_effect=full):
                    owner=fixed._matrix_cli_write_owner(request,output,ledger,0,fd,live_bytes=0,read_charge=charge)
                self.assertFalse(owner.paid)
                self.assertEqual(calls,[representation])
                with patch.object(fixed.os,'fsync',wraps=os.fsync) as sync:
                    owner.reserve(fixed.matrix_output_reservation(output),live_bytes=0)
                    self.assertGreaterEqual(sync.call_count,1)
                self.assertTrue(owner.paid)
                with self.assertRaisesRegex(ValueError,'single matrix'):
                    owner.reserve(owner.reservation,live_bytes=0)
            finally:os.close(fd)
            debit=json.loads(Path(ledger).read_bytes())
            self.assertEqual(debit['column'],'write');self.assertEqual(debit['bytes'],17825792)
            self.assertEqual(debit['binding'],binding)
            # A different request cannot rebind the same authenticated artifact.
            request=dict(request);request['matrix_storage']=dict(binding,phase='recovery')
            with self.assertRaisesRegex(ValueError,'exact owner'):
                fixed._matrix_cli_write_owner(request,output,ledger,0,-1,live_bytes=0,read_charge=charge)

    def test_owner_write_guard_rejects_before_io(self):
        from unittest.mock import patch
        owner=fixed._MatrixOutputOwner({},fixed.matrix_output_reservation('/NONHISTORICAL/output'),-1)
        with patch.object(fixed.os,'write',side_effect=AssertionError('write before guard')):
            with self.assertRaisesRegex(ValueError,'preallocation'):
                owner.reserve(owner.reservation,live_bytes=core.SEMANTIC)

    def test_closed_matrix_summary_and_terminal_diagnostic(self):
        pin=dict(path='/NONHISTORICAL/proof',bytes=1,sha256='a'*64);rows=[]
        for route in fixed.framing_routes():
            name,index=route.rsplit(':',1)
            rows.append(dict(route=name,index=int(index),context=dict(route=name,
                phase='development:0:0:'+index,manifest='b'*64),receipt=pin,result=pin))
        matrix=fixed.actual_fit_matrix('c'*64,pin,0,rows)
        summary=fixed._matrix_cli_summary(matrix,pin,live_bytes=0)
        self.assertEqual((summary['executed'],summary['completed']),(16,16))
        self.assertTrue(summary['complete_fit']);self.assertFalse(summary['accepted'])
        failed=fixed._matrix_cli_summary(dict(schema='NONHISTORICAL-terminal'),pin,live_bytes=0)
        self.assertFalse(failed['complete_fit']);self.assertNotIn('completed',failed)


class CompleteFramingOrderTests(unittest.TestCase):
    def test_real_retained_owner_and_carry_reject_before_decode_or_canonical(self):
        from unittest.mock import patch
        args=CompleteFramingControlTests().specimen()
        expected=fixed.complete_fit_control_sections(*args,live_bytes=0)
        outer=dict(NONHISTORICAL_retained_payload='x'*450000)
        actual_owner=fixed.graph_charge((outer,args,expected))+fixed.MATRIX_FAILURE_PUBLICATION_CARRY
        self.assertLessEqual(actual_owner,core.SEMANTIC)
        with patch.object(core,'canonical',side_effect=AssertionError('canonical before owner guard')) as canonical,patch.object(fixed.records,'decode',side_effect=AssertionError('decode before owner guard')) as decode:
            with self.assertRaises(ValueError):
                fixed.complete_fit_control_sections(*args,live_bytes=actual_owner)
            with self.assertRaises(ValueError):
                fixed.verify_complete_fit_control_receipts({},expected,live_bytes=actual_owner)
            self.assertEqual(len(outer['NONHISTORICAL_retained_payload']),450000)
            canonical.assert_not_called();decode.assert_not_called()

    def test_actual_fit_control_failure_precedes_counter_factory(self):
        from unittest.mock import patch
        args=CompleteFramingControlTests().specimen();compiled=args[0]
        compiled.update(units=(),setup_unit_wire=(),unit_ids=(),sources={},bridge_caller_charge=0,matrix_storage_binding=args[9])
        partition=dict(future81_wire=core.canonical(['future81',args[5]]),
            unit_pool_wire=core.canonical(['unit_pool',dict(schema='fixed40-lossless-complete-unit-pool-v1')]),routes=args[8],
            control_receipts={name:dict(bytes=1,sha256='0'*64) for name in fixed.COMPLETE_CONTROL_FIELDS},control_semantics_checked=False)
        calls=[]
        def factory():
            calls.append('counter');raise AssertionError('counter before full controls')
        with patch.object(fixed,'semantic_units',return_value=((),(),args[2],args[3],None)),patch.object(fixed,'invocation_wire_manifest',return_value=((),(),0)),patch.object(fixed,'framing_partition',return_value=partition),patch.object(fixed,'expand_complete_fit_units',return_value=(args[6],args[7])),patch.object(fixed,'_fit_check_program',return_value=(args[6],args[7])):
            with self.assertRaisesRegex(ValueError,'full closed control semantic mismatch'):
                fixed.run_selected40_fit(compiled,args[1],None,{},offline_scope=args[4],representation=args[4],read_charge=lambda p:None,counter_factory=factory)
        self.assertEqual(calls,[])

    def test_real_fit_entry_retains_outer_owner_and_rejects_before_any_preparation(self):
        from unittest.mock import patch
        args=CompleteFramingControlTests().specimen();compiled=args[0]
        outer=dict(NONHISTORICAL_actual_cli_root='q'*1300000)
        owner=fixed.graph_charge(outer)+fixed.MATRIX_FAILURE_PUBLICATION_CARRY
        self.assertLessEqual(owner,core.SEMANTIC)
        compiled['bridge_caller_charge']=owner
        inputs={name:args[4] for name in fixed.INPUT_FIELDS}
        inputs['shared_reports']=tuple(args[4] for _ in range(17))
        inputs['native_reports']=(args[4],args[4])
        calls=[]
        def factory():calls.append('counter');raise AssertionError('counter before owner guard')
        with patch.object(fixed,'_read',side_effect=AssertionError('body open before owner guard')) as read,patch.object(fixed.records,'decode',side_effect=AssertionError('decode before owner guard')) as decode,patch.object(core,'canonical',side_effect=AssertionError('canonical before owner guard')) as canonical:
            with self.assertRaisesRegex(ValueError,'pure preparation entry lifetime'):
                fixed.run_selected40_fit(compiled,inputs,None,{},representation=args[4],read_charge=lambda p:None,counter_factory=factory,failure_publication_carry=fixed.MATRIX_FAILURE_PUBLICATION_CARRY)
            self.assertEqual(len(outer['NONHISTORICAL_actual_cli_root']),1300000)
            self.assertEqual(calls,[]);read.assert_not_called();decode.assert_not_called();canonical.assert_not_called()


class CompleteFramingControlTests(unittest.TestCase):
    """NONHISTORICAL format/binding tests; no gate, proof or route is executed."""
    def specimen(self):
        pin=dict(path='/NONHISTORICAL/control',bytes=1,sha256='a'*64)
        sources=[dict(path='/NONHISTORICAL/source'+str(i),bytes=1,sha256='b'*64) for i in range(90)]
        tools=[dict(path='/NONHISTORICAL/tool'+str(i),bytes=1,sha256='c'*64) for i in range(11)]
        compiled=dict(profile_binding=dict(profile=pin,authority=pin,history=pin,
            events=[214,0,0],source_identity=core.digest(sources+tools)),
            consumed_events=(214,0,0),inherited_event_history='d'*64,maxima=(4096,)*81)
        inputs=dict(source_freeze=pin,tool_freeze=pin)
        future=[dict(role=i,constructor='NONHISTORICAL',arguments=pin,
            units=['a','b','c'],output=pin,maximum=4096,dependencies=[]) for i in range(81)]
        ids=('NONHISTORICAL-unit',);wires=(core.canonical([[0,['/NONHISTORICAL/body',7,'e'*64],'evidence','bytes']]),)
        manifest=core.frozen_manifest_digest(ids,wires)
        routes=[dict(route=r,manifest=manifest) for r in fixed.framing_routes()]
        storage=dict(schema='fixed40-matrix-storage-binding-v1',activation=fixed.MATRIX_STORAGE_ACTIVATION,
            profile=pin,contexts_digest='f'*64,directory='/NONHISTORICAL/output',ledger='/NONHISTORICAL/ledger',round=0,phase='normal')
        return compiled,inputs,sources,tools,pin,future,ids,wires,routes,storage

    def test_actual_producer_binds_full_program_and_declared_reads(self):
        args=self.specimen();value=fixed.complete_fit_control_sections(*args,live_bytes=0)
        self.assertEqual(set(value),set(fixed.COMPLETE_CONTROL_FIELDS))
        self.assertEqual(value['binary_schedule']['kind_reads']['evidence'],dict(occurrences=1,bytes_plus_EOF=8))
        self.assertEqual(value['binary_schedule']['future_outputs'][80]['maximum'],4096)
        self.assertEqual(len(value['controls']['future_arguments']),81)
        receipts={name:dict(bytes=len(core.canonical(row)),sha256=hashlib.sha256(core.canonical(row)).hexdigest()) for name,row in value.items()}
        partition=dict(control_receipts=receipts,control_semantics_checked=False)
        fixed.verify_complete_fit_control_receipts(partition,value,live_bytes=0)
        self.assertTrue(partition['control_semantics_checked'])
        core.canonical(partition)
        value['history']['effective']=[215,0,0]
        with self.assertRaisesRegex(ValueError,'semantic mismatch'):
            fixed.verify_complete_fit_control_receipts(partition,value,live_bytes=0)

    def test_profile_projection_preserves_full_authenticated_raw_owner(self):
        from types import SimpleNamespace
        args=self.specimen();binding=args[0]['profile_binding']
        raw=core.canonical({k:binding[k] for k in ('authority','history','events','source_identity')})
        admission=SimpleNamespace(_profile_wire=raw,profile_pin=binding['profile'])
        result=fixed._compiled_profile_binding(admission,live_bytes=fixed.graph_charge(admission.__dict__))
        self.assertEqual(result,binding);self.assertIs(admission._profile_wire,raw)
        with self.assertRaises(ValueError):
            fixed._compiled_profile_binding(admission,live_bytes=core.SEMANTIC)

    def test_complete_reader_consumes_controls_and_returns_json_safe_receipts(self):
        args=self.specimen();compiled=args[0];value=fixed.complete_fit_control_sections(*args,live_bytes=0)
        root=Path(os.environ['DEVELOPMENT_TEST_ARTIFACT_ROOT'])/'NONHISTORICAL-closed-framing';root.mkdir()
        archive=core.frozen_manifest_digest((),());compiled.update(archive_manifest_binding=archive,prior83_wire=core.canonical([]))
        full_routes=[dict(**r,archive_manifest=archive,future_roles=list(range(81))) for r in args[8]]
        sections=dict(header=dict(schema='fixed40-complete-fit-representation-v1',archive_manifest=archive,accepted=False,ready=False,new_facts=0),
            prior83=[],future81=args[5],unit_pool=dict(schema='fixed40-lossless-complete-unit-pool-v1'),routes=full_routes,future_maxima=list(compiled['maxima']),**value)
        emitted=fixed.write_complete_fit_representation(root/'complete.jsonl',((name,sections[name]) for name in fixed.COMPLETE_FIT_SECTIONS),live_bytes=0)
        charges=[]
        def charge(packet):
            charges.append(packet)
            debit=core.canonical(dict(packet=packet,bytes_plus_EOF=sum(p['bytes']+1 for p in packet),scope='NONHISTORICAL test BEFORE_OPEN'))
            with (root/'before-open.jsonl').open('ab') as out:out.write(debit);out.flush();os.fsync(out.fileno())
        partition=fixed.framing_partition(compiled,(),representation=emitted['pin'],read_charge=charge,archive_manifest=((),()),live_bytes=0)
        self.assertEqual(charges,[(emitted['pin'],)])
        self.assertFalse(partition['control_semantics_checked'])
        fixed.verify_complete_fit_control_receipts(partition,value,live_bytes=0)
        del partition['future81_wire'],partition['unit_pool_wire']
        self.assertFalse(any(k.endswith('_wire') for k in partition))
        self.assertNotIn('future_roles',partition['routes'][0]);core.canonical(partition)


class MatrixCliInheritedOwnerTests(unittest.TestCase):
    """Actual constructor/callsite guards; no Invocation or physical proof."""
    def test_actual_fit_constructor_inherits_outer_and_rejects_real_growth(self):
        from unittest.mock import patch
        outer={'payload':'x'*800000};base='y'*262144
        admission=runtime.RuntimeAdmission.__new__(runtime.RuntimeAdmission)
        admission.accepted=False;admission._outer_live=fixed.graph_charge(outer)
        admission.caller_charge=admission._outer_live+fixed.graph_charge(base)
        admission._profile_wire=b'{}\n';admission._full_read_charge=lambda packet:None
        review=dict(path='/NONHISTORICAL/review',bytes=1,sha256='a'*64)
        observed=[]
        class ReachedDecode(Exception):pass
        def observe(raw,live_bytes):
            self.assertTrue(len(outer['payload'])==800000 and len(base)==262144)
            observed.append(live_bytes);raise ReachedDecode()
        with patch.object(runtime,'_fit_profile_decode',side_effect=observe):
            with self.assertRaises(ReachedDecode):
                runtime.OfflineFitAdmission(admission,review,read_charge=lambda p:None,live_bytes=0)
            self.assertEqual(len(observed),1)
            self.assertGreater(observed[0],admission.caller_charge)
            # The same caller a second time is the old bug, not free metadata.
            with self.assertRaisesRegex(ValueError,'preallocation lifetime'):
                runtime.OfflineFitAdmission(admission,review,read_charge=lambda p:None,
                    live_bytes=admission._outer_live)
        outer['payload']+='z'*(16*core.CHUNK)
        admission._outer_live=fixed.graph_charge(outer)
        admission.caller_charge=admission._outer_live+fixed.graph_charge(base)
        with patch.object(runtime,'_fit_profile_decode',side_effect=AssertionError('decode before guard')):
            with self.assertRaisesRegex(ValueError,'preallocation lifetime'):
                runtime.OfflineFitAdmission(admission,review,read_charge=lambda p:None,live_bytes=0)

    def test_actual_cli_passes_one_outer_owner_to_seal_fit_and_summary(self):
        from unittest.mock import patch
        import io,contextlib,tempfile
        observed={};root=Path(os.environ['DEVELOPMENT_TEST_ARTIFACT_ROOT'])
        with tempfile.TemporaryDirectory(dir=root,prefix='NONHISTORICAL-cli-inherit-') as name:
            root=Path(name);pin=dict(path='/NONHISTORICAL/pin',bytes=1,sha256='a'*64)
            contexts=[];rows=[]
            for route in fixed.framing_routes():
                role,index=route.rsplit(':',1);context=dict(route=role,
                    phase='development:0:0:'+index,manifest='b'*64,executables=[])
                contexts.append(context);rows.append(dict(route=role,index=int(index),context=context,receipt=pin,result=pin))
            request=dict(schema='fixed40-complete-fit-request-v2',profile=pin,review=pin,
                inputs=dict(shared_reports=[],native_reports=[],NONHISTORICAL_padding='x'*12000),
                contexts=contexts,historical_scope={},representation=pin,admission_trace=pin,
                trace_storage_reservation=1,matrix_storage={})
            raw=core.canonical(request);path=root/'request.json';path.write_bytes(raw)
            class Admission:
                def __init__(self,profile,*,read_charge,live_bytes):
                    observed['outer']=live_bytes;self.caller_charge=live_bytes+4096
                def seal_read_trace(self,*args,**kwargs):observed['seal']=kwargs['live_bytes']
            class Fit:
                def __init__(self,admission,review,*,read_charge,live_bytes):
                    observed['fit']=live_bytes;self.caller_charge=admission.caller_charge+8192+live_bytes
                    observed['fit_charge']=self.caller_charge
            def owner(request,directory,ledger,round_index,fd,**kwargs):
                return fixed._MatrixOutputOwner({},fixed.matrix_output_reservation(directory),fd)
            matrix=fixed.actual_fit_matrix('c'*64,pin,0,rows)
            actual_summary=fixed._matrix_cli_summary
            def summary(value,proof,*,live_bytes):
                observed['summary']=live_bytes;return actual_summary(value,proof,live_bytes=live_bytes)
            argv=['--complete-fit-request',str(path),'--request-bytes',str(len(raw)),
                '--request-sha256',hashlib.sha256(raw).hexdigest(),'--read-ceiling','65536',
                '--read-ledger',str(root/'reads.jsonl'),'--output-directory',str(root/'matrix')]
            with patch.object(fixed,'_read',return_value=raw),patch.object(fixed,'_matrix_cli_write_owner',side_effect=owner),patch.object(runtime,'RuntimeAdmission',Admission),patch.object(runtime,'OfflineFitAdmission',Fit),patch.object(fixed,'run_selected40_matrix',return_value=(pin,matrix)),patch.object(fixed,'_matrix_cli_summary',side_effect=summary),contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(fixed.complete_fit_main(argv),0)
            self.assertEqual(observed['seal'],0);self.assertEqual(observed['fit'],0)
            self.assertGreater(observed['summary'],observed['fit_charge'])
            self.assertLess(observed['summary']-observed['fit_charge'],observed['outer'])


class MatrixFailureLifetimeTests(unittest.TestCase):
    def test_actual_decoder_frame_and_cause_survive_complete_publication(self):
        from unittest.mock import patch
        root=Path(os.environ['DEVELOPMENT_TEST_ARTIFACT_ROOT'])/'NONHISTORICAL-matrix-failure'
        root.mkdir();cause=RuntimeError('NONHISTORICAL-cause');observed=[]
        def fail():
            document=' '*32768+'{'
            retained_buffer=bytearray(core.CHUNK)
            try:json.loads(document)
            except json.JSONDecodeError as primary:raise primary from cause
        try:fail()
        except json.JSONDecodeError as error:
            primary=error;original_tb=error.__traceback__;original_open=os.open
            def check_open(*args,**kwargs):
                traceback=primary.__traceback__;frame=None
                while traceback is not None:
                    if traceback.tb_frame.f_code is fail.__code__:frame=traceback.tb_frame;break
                    traceback=traceback.tb_next
                self.assertIsNotNone(frame)
                self.assertIs(primary.doc,frame.f_locals['document'])
                self.assertEqual(len(frame.f_locals['retained_buffer']),core.CHUNK)
                self.assertIs(primary.__cause__,cause);observed.append(True)
                del traceback,frame
                return original_open(*args,**kwargs)
            bound=core.SEMANTIC-fixed.MATRIX_FAILURE_PUBLICATION_CARRY
            with patch.object(os,'open',new=check_open):
                pin=fixed._matrix_failure_publication(error,fixed.framing_routes()[0],
                    root/'failure.json',retained_fit_peak=bound)
            self.assertIs(error.__traceback__,original_tb)
            self.assertIs(error.__cause__,cause)
            self.assertGreaterEqual(len(observed),2)
            raw=Path(pin['path']).read_bytes()
            expected=dict(schema='fixed40-offline-route-failure-v1',
                route=fixed.framing_routes()[0],exception='JSONDecodeError',message=str(error),accepted=False)
            self.assertEqual(raw,core.canonical(expected))
            self.assertEqual(hashlib.sha256(raw).hexdigest(),pin['sha256'])
            self.assertTrue(error.fixed40_matrix_failure_publication['complete'])

    def test_long_complete_message_and_prepaid_peak_rejection(self):
        root=Path(os.environ['DEVELOPMENT_TEST_ARTIFACT_ROOT'])/'NONHISTORICAL-matrix-message'
        root.mkdir();message='x'*10000+(chr(10)+chr(0x2603))*2000
        try:raise ValueError(message)
        except ValueError as error:
            pin=fixed._matrix_failure_publication(error,fixed.framing_routes()[0],root/'full.json',
                retained_fit_peak=core.SEMANTIC-fixed.MATRIX_FAILURE_PUBLICATION_CARRY)
            self.assertEqual(Path(pin['path']).read_bytes(),core.canonical(dict(
                schema='fixed40-offline-route-failure-v1',route=fixed.framing_routes()[0],
                exception='ValueError',message=message,accepted=False)))
        class NoFormat(RuntimeError):
            def __str__(self):raise AssertionError('format before prelaunch carry')
        primary=NoFormat('NONHISTORICAL')
        with self.assertRaises(NoFormat) as raised:
            fixed._matrix_failure_publication(primary,fixed.framing_routes()[0],root/'absent.json',
                retained_fit_peak=core.SEMANTIC)
        self.assertIs(raised.exception,primary)
        self.assertFalse((root/'absent.json').exists())
